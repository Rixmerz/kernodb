//! Write-ahead log, format v1 (`docs/file-format/v1.md` §6): one file per
//! memtable (`NNNNNN.wal`), never appended to after a restart.
//!
//! ```text
//! header (96 bytes, common header: file number, db_id, base_seq)
//! frame* (32-byte head: payload_len, crc, first_seq, synced_len, count; payload = commits)
//! ```
//!
//! A frame holds one group of commits (group commit, one `write_at` and at
//! most one `fsync` per group; a single commit when groups are disabled);
//! the frame, canonical commit and replay rules (WAL-C1..C4, WAL-R1..R6) live
//! in [`crate::format::wal`].
//!
//! **Preallocated and recycled files.** A file that grows makes every
//! `fdatasync` commit its new size (and extent) to the file system's journal.
//! A WAL can instead start as `header ‖ zeros` of the expected size, already
//! written and synced, so that appending a frame overwrites blocks that exist
//! and the sync only flushes data. The zeros are the padding that v1.md §6.1
//! and WAL-R4 already allow: replay stops at the first invalid frame and
//! accepts the rest as padding only if it is all zeros, so no format change is
//! needed. A WAL that was flushed is *recycled* the same way, after its bytes
//! are overwritten with zeros: its stale frames would not replay (their CRC
//! covers another file number) but they would not be padding either, and a
//! rotated WAL that ends in non-zero bytes is a torn log in the middle of the
//! chain (REC-10a). The spare is a file apart (`SPARE_WAL`) that recovery
//! ignores and deletes: only a complete one is ever renamed into place.
//!
//! **Torn tail or media damage?** Every frame records `synced_len`, the
//! prefix of the file the writer knew durable when it wrote the frame. An
//! invalid frame at `P` that is followed by a valid frame claiming that `P`
//! was durable was damaged after it was synced, and replay reports
//! corruption instead of dropping it as a torn tail (which would silently
//! lose acknowledged commits). In `Buffered` mode `synced_len` stays behind
//! the writes, so a power loss may cut the log at any frame boundary.

use std::io;
use std::path::Path;
use std::sync::Arc;

use kerno_core::vfs::{File, OpenMode, Vfs};
use kerno_core::{Error, Result};

use crate::files::{FileSource, read_error};
use crate::format::features::{SLOT_LEN, WRITE_INCOMPAT, frame_has_slot};
use crate::format::header::{HEADER_LEN, Header};
use crate::format::wal::{CanonicalCommit, WalScan, WalScanParams, frame_len, reseal_frame, scan_wal};

pub fn wal_name(number: u64) -> String {
    format!("{number:06}.wal")
}

/// The file of a [`WalWriter::read_only`] log: nothing can be written to it.
struct ReadOnlyLog;

impl File for ReadOnlyLog {
    fn read_at(&self, _: &mut [u8], _: u64) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn write_at(&self, _: &[u8], _: u64) -> io::Result<()> {
        Err(io::ErrorKind::ReadOnlyFilesystem.into())
    }

    fn len(&self) -> io::Result<u64> {
        Ok(0)
    }

    fn set_len(&self, _: u64) -> io::Result<()> {
        Err(io::ErrorKind::ReadOnlyFilesystem.into())
    }

    fn sync_data(&self) -> io::Result<()> {
        Err(io::ErrorKind::ReadOnlyFilesystem.into())
    }
}

/// The spare WAL: a file of zeros ready to become the next log (see the
/// module documentation). Not a canonical name, so recovery never replays it.
pub const SPARE_WAL: &str = "WAL.spare";

/// Zeros are written, and synced, in pieces of this size, so that a spare
/// does not leave the file system a burst of dirty pages that the next
/// commit's `fdatasync` would have to wait behind.
const ZERO_CHUNK: usize = 1 << 20;

pub struct WalWriter {
    file: Arc<dyn File>,
    number: u64,
    /// Bytes written, the header included.
    offset: u64,
    /// The prefix of the file known durable (W-11): what each frame records.
    synced_len: u64,
}

impl WalWriter {
    /// Creates the log `number`: `CreateNew`, the header (whose first commit
    /// carries sequence `base_seq`) and `sync_dir` (W-1). The header itself is
    /// not synced: it becomes durable with the first `sync`, and no frame is
    /// acknowledged before that.
    ///
    /// With `preallocate` above the header's size the file is first extended
    /// with zeros to that many bytes and synced, so that the syncs of its
    /// frames need not commit its size (see the module documentation); that
    /// costs the caller the write of the zeros.
    pub fn create(
        vfs: &dyn Vfs,
        dir: &Path,
        number: u64,
        db_id: [u8; 16],
        base_seq: u64,
        preallocate: u64,
    ) -> Result<Self> {
        let file = vfs.open(&dir.join(wal_name(number)), OpenMode::CreateNew)?;
        if preallocate > HEADER_LEN as u64 {
            write_zeros(&*file, 0, preallocate, &|| false)?;
        }
        file.write_at(&Header::wal(db_id, number, base_seq).encode(), 0)?;
        vfs.sync_dir(dir)?;
        Ok(WalWriter { file, number, offset: HEADER_LEN as u64, synced_len: 0 })
    }

    /// Makes the spare WAL (see [`prepare_spare`]) the log `number`: renames
    /// it, writes the header over its zeros and `sync_dir` (W-1). Same
    /// durability as [`create`](Self::create): the header is not synced.
    pub fn create_from_spare(vfs: &dyn Vfs, dir: &Path, number: u64, db_id: [u8; 16], base_seq: u64) -> Result<Self> {
        let path = dir.join(wal_name(number));
        vfs.rename(&dir.join(SPARE_WAL), &path)?;
        let file = vfs.open(&path, OpenMode::Existing)?;
        file.write_at(&Header::wal(db_id, number, base_seq).encode(), 0)?;
        vfs.sync_dir(dir)?;
        Ok(WalWriter { file, number, offset: HEADER_LEN as u64, synced_len: 0 })
    }

    /// The placeholder log of a read-only handle: every entry point that
    /// writes rejects the call before it gets here, and a write that did get
    /// here would fail without touching the disk.
    pub fn read_only(number: u64) -> Self {
        WalWriter { file: Arc::new(ReadOnlyLog), number, offset: HEADER_LEN as u64, synced_len: 0 }
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    /// Bytes written, the header included: where the next frame goes, and
    /// the end of everything that is not padding.
    pub fn written(&self) -> u64 {
        self.offset
    }

    /// Appends one frame holding `commits`, the first of which has sequence
    /// `first_seq` (a single `write_at`), and returns its length. The
    /// sequence must follow the last one of the previous frame, and
    /// `commits` must be 1..=`MAX_FRAME_COMMITS` commits.
    pub fn append_group<B: AsRef<[u8]>>(
        &mut self,
        first_seq: u64,
        commits: &[&CanonicalCommit<B>],
    ) -> io::Result<usize> {
        let slot = frame_has_slot(WRITE_INCOMPAT);
        let payload_len: usize = commits.iter().map(|c| c.encoded_len()).sum();
        let mut frame = Vec::with_capacity(frame_len(payload_len as u32, slot) as usize);
        frame.extend_from_slice(&(payload_len as u32).to_le_bytes());
        frame.extend_from_slice(&[0; 4]); // crc, set below
        frame.extend_from_slice(&first_seq.to_le_bytes());
        frame.extend_from_slice(&self.synced_len.to_le_bytes());
        frame.extend_from_slice(&(commits.len() as u32).to_le_bytes());
        frame.extend_from_slice(&[0; 4]); // reserved
        for commit in commits {
            commit.encode_into(&mut frame);
        }
        if slot {
            frame.resize(frame.len() + SLOT_LEN, 0);
        }
        reseal_frame(&mut frame, self.number);
        self.file.write_at(&frame, self.offset)?;
        self.offset += frame.len() as u64;
        Ok(frame.len())
    }

    /// Whether a frame was written since the last successful `sync` (a log
    /// holding only its header has nothing to make durable: the header
    /// becomes durable with the first frame's `fsync`, W-1).
    pub fn needs_sync(&self) -> bool {
        self.offset > self.synced_len.max(HEADER_LEN as u64)
    }

    /// `sync_data`; on success everything written so far is durable. After a
    /// failure the caller poisons the handle and writes nothing more (W-11).
    pub fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()?;
        self.synced_len = self.offset;
        Ok(())
    }
}

/// Writes zeros over `[from, to)` of `file`, syncing after each
/// [`ZERO_CHUNK`], and stops early (returning `Ok(false)`) once `abort` says
/// so. `Ok(true)` means the whole range is zero and durable.
fn write_zeros(file: &dyn File, from: u64, to: u64, abort: &dyn Fn() -> bool) -> io::Result<bool> {
    let zeros = vec![0u8; ZERO_CHUNK];
    let mut at = from;
    while at < to {
        if abort() {
            return Ok(false);
        }
        let n = ((to - at) as usize).min(ZERO_CHUNK);
        file.write_at(&zeros[..n], at)?;
        file.sync_data()?;
        at += n as u64;
    }
    Ok(true)
}

/// Prepares the spare WAL ([`SPARE_WAL`]) of `size` bytes of zeros, durable,
/// ready for [`WalWriter::create_from_spare`]. With `retired`, the number of
/// a WAL nothing needs any more and the bytes of it that were written
/// ([`WalWriter::written`]; 0 if not known), that file is recycled (renamed
/// and its written bytes overwritten, so no extent is freed and allocated
/// again); otherwise a new file is created. Returns `Ok(false)`, with no
/// spare left behind, if `abort` ended the work. On an error the half-made
/// spare may remain: the caller removes it.
///
/// Only the written prefix of a recycled file is rewritten: the writer of a
/// preallocated log only ever overwrites zeros from the front, so the rest
/// of the file is zero already.
pub fn prepare_spare(
    vfs: &dyn Vfs,
    dir: &Path,
    retired: Option<(u64, u64)>,
    size: u64,
    abort: &dyn Fn() -> bool,
) -> io::Result<bool> {
    let path = dir.join(SPARE_WAL);
    let mut stale = 0;
    let file = match retired {
        Some((number, written)) => {
            vfs.rename(&dir.join(wal_name(number)), &path)?;
            let file = vfs.open(&path, OpenMode::Existing)?;
            stale = if written == 0 { file.len()? } else { written.min(file.len()?) };
            file
        }
        None => {
            match vfs.remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            vfs.open(&path, OpenMode::CreateNew)?
        }
    };
    // A file shorter than `size` (a log that grew, because no spare was ready
    // when it was created) is extended with zeros; one that is longer (a
    // group that ran past the expected end) keeps its length.
    let len = file.len()?;
    if !write_zeros(&*file, 0, stale, abort)? || !write_zeros(&*file, len, size, abort)? {
        vfs.remove_file(&path)?;
        return Ok(false);
    }
    vfs.sync_dir(dir)?;
    Ok(true)
}

/// Replays the log `number`, which must start at sequence `expected_seq`
/// (WAL-R1), calling `apply(seq, commit)` for every commit in order. Returns
/// the scan: the header state, the commits and the length of the valid
/// prefix, and how the file ends. Which logs may end torn or have a broken
/// header is the caller's decision (REC-10).
///
/// The commits are provisional until the caller has checked the whole chain
/// of live logs: `apply` may build in-memory state, and recovery writes
/// (S-2) only from a second replay, after the chain was classified. An error
/// from `apply` stops the replay and is returned as it is.
pub fn replay(
    vfs: &dyn Vfs,
    dir: &Path,
    number: u64,
    db_id: [u8; 16],
    expected_seq: u64,
    mut apply: impl FnMut(u64, &CanonicalCommit<Vec<u8>>) -> Result<()>,
) -> Result<WalScan> {
    let name = wal_name(number);
    let file = vfs.open(&dir.join(&name), OpenMode::Existing)?;
    let mut src = FileSource::new(&*file)?;
    let params = WalScanParams { file_number: number, db_id, expected_seq };
    let mut has_ranges = false;
    let mut failed = None;
    let scanned = scan_wal(&mut src, &params, |_, frame| {
        for (i, commit) in frame.commits.iter().enumerate() {
            if !commit.ranges.is_empty() {
                // The caller's apply may not accept ranges; the log is refused below.
                has_ranges = true;
                continue;
            }
            if let Err(e) = apply(frame.first_seq + i as u64, commit) {
                failed = Some(e);
                return Err(io::Error::other("replay stopped by its caller"));
            }
        }
        Ok(())
    });
    if let Some(e) = failed {
        return Err(e);
    }
    let scan = scanned.map_err(|e| read_error(&name, e))?;
    if has_ranges {
        // Applying such a log without its ranges would bring deleted keys
        // back: refuse it instead.
        return Err(Error::Incompatible(format!("{name}: range deletions are not supported yet")));
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::End;
    use crate::format::wal::{CommitOp, WalHeaderState, canonicalize, seal_frame};
    use kerno_core::WriteBatch;
    use kerno_testkit::vfs::{CrashKind, SimVfs};

    const DB: [u8; 16] = [9; 16];

    fn batch(i: u64) -> WriteBatch {
        let mut b = WriteBatch::new();
        b.put(format!("k{i}"), vec![i as u8; (i as usize * 97) % 1500]);
        if i.is_multiple_of(3) {
            b.delete(format!("k{}", i - 1));
        }
        b
    }

    fn canonical(b: &WriteBatch) -> CanonicalCommit<&[u8]> {
        canonicalize(b.ops().iter().map(|op| match op {
            kerno_core::Op::Put { key, value } => CommitOp::Put { key, value },
            kerno_core::Op::Delete { key } => CommitOp::Delete { key },
            kerno_core::Op::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
        }))
    }

    type Commits = Vec<(u64, CanonicalCommit<Vec<u8>>)>;

    fn collect(vfs: &dyn Vfs, first: u64) -> Result<(WalScan, Commits)> {
        let mut got = Vec::new();
        let scan = replay(vfs, Path::new("/w"), 1, DB, first, |s, c| {
            got.push((s, c.clone()));
            Ok(())
        })?;
        Ok((scan, got))
    }

    fn setup(sim: &SimVfs, base_seq: u64) -> (Arc<dyn Vfs>, WalWriter) {
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, base_seq, 0).unwrap();
        (vfs, w)
    }

    fn append(w: &mut WalWriter, seq: u64) {
        w.append_group(seq, &[&canonical(&batch(seq))]).unwrap();
    }

    #[test]
    fn roundtrip() {
        let sim = SimVfs::new(1);
        let (vfs, mut w) = setup(&sim, 11);
        for i in 1..=20 {
            append(&mut w, 10 + i);
        }
        let (scan, got) = collect(&*vfs, 11).unwrap();
        assert_eq!((scan.frames, scan.commits, scan.next_seq, scan.end), (20, 20, 31, End::Clean));
        assert!(matches!(scan.header, WalHeaderState::Valid(h) if h.per_type == 11 && h.file_number == 1));
        for (i, (s, c)) in got.iter().enumerate() {
            assert_eq!(*s, 11 + i as u64);
            assert_eq!(c, &canonical(&batch(10 + i as u64 + 1)).to_owned_commit());
        }
        // The log starts at base_seq: any other expectation is corruption, not a torn tail.
        assert!(matches!(collect(&*vfs, 12), Err(Error::Corruption(m)) if m.contains("WAL-R1")));
    }

    #[test]
    fn creation_writes_the_header_and_syncs_only_the_directory() {
        use kerno_testkit::vfs::CountingVfs;
        let (vfs, io) = CountingVfs::wrap(SimVfs::new(1).as_vfs());
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let before = io.snapshot();
        let w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, 5, 0).unwrap();
        let after = io.snapshot();
        assert_eq!(after.file_syncs, before.file_syncs, "the header is not synced (W-1)");
        assert_eq!(after.dir_syncs, before.dir_syncs + 1);
        assert_eq!(w.offset, HEADER_LEN as u64);
        assert_eq!(w.number(), 1);
    }

    #[test]
    fn frames_record_the_synced_prefix() {
        let sim = SimVfs::new(1);
        let (vfs, mut w) = setup(&sim, 1);
        append(&mut w, 1);
        w.sync().unwrap();
        let after_first = w.offset;
        append(&mut w, 2);
        append(&mut w, 3); // not synced: still records the same prefix
        let file = vfs.open(Path::new("/w/000001.wal"), OpenMode::Existing).unwrap();
        let mut synced = Vec::new();
        let mut src = FileSource::new(&*file).unwrap();
        scan_wal(&mut src, &WalScanParams { file_number: 1, db_id: DB, expected_seq: 1 }, |off, f| {
            synced.push((off, f.synced_len));
            Ok(())
        })
        .unwrap();
        assert_eq!(synced.len(), 3);
        assert_eq!(synced[0], (HEADER_LEN as u64, 0));
        assert_eq!(synced[1].1, after_first);
        assert_eq!(synced[2].1, after_first);
    }

    /// Unsynced appends + power loss: replay yields a prefix of the
    /// commits and reports the rest as a torn tail.
    #[test]
    fn torn_tail_yields_a_prefix() {
        let mut torn_seen = 0;
        for seed in 0..200 {
            let sim = SimVfs::new(seed);
            let (vfs, mut w) = setup(&sim, 1);
            for i in 1..=5 {
                append(&mut w, i);
            }
            w.sync().unwrap();
            for i in 6..=12 {
                append(&mut w, i);
            }
            let after = sim.crash_now(CrashKind::PowerLoss);
            drop(vfs);
            let (scan, got) = collect(&*after.as_vfs(), 1).unwrap();
            assert!(scan.next_seq > 5, "synced commits lost");
            assert_eq!(got.len() as u64, scan.next_seq - 1);
            for (s, c) in &got {
                assert_eq!(c, &canonical(&batch(*s)).to_owned_commit());
            }
            if scan.end == End::Torn {
                torn_seen += 1;
            }
        }
        assert!(torn_seen > 0);
    }

    /// One frame holds a whole group: `count` commits, the first sequence of
    /// the group, one `synced_len`, and replay hands out the commits in order.
    #[test]
    fn group_frame_holds_consecutive_commits() {
        let sim = SimVfs::new(1);
        let (vfs, mut w) = setup(&sim, 1);
        append(&mut w, 1);
        w.sync().unwrap();
        let synced = w.offset;
        let batches: Vec<WriteBatch> = (2..=5).map(batch).collect();
        let commits: Vec<_> = batches.iter().map(canonical).collect();
        let len = w.append_group(2, &commits.iter().collect::<Vec<_>>()).unwrap();
        assert_eq!(w.offset, synced + len as u64);
        let (scan, got) = collect(&*vfs, 1).unwrap();
        assert_eq!((scan.frames, scan.commits, scan.next_seq, scan.end), (2, 5, 6, End::Clean));
        for (i, (s, c)) in got.iter().enumerate() {
            assert_eq!(*s, 1 + i as u64);
            assert_eq!(c, &canonical(&batch(1 + i as u64)).to_owned_commit());
        }
        let file = vfs.open(Path::new("/w/000001.wal"), OpenMode::Existing).unwrap();
        let mut frames = Vec::new();
        let mut src = FileSource::new(&*file).unwrap();
        scan_wal(&mut src, &WalScanParams { file_number: 1, db_id: DB, expected_seq: 1 }, |off, f| {
            frames.push((off, f.first_seq, f.synced_len, f.commits.len()));
            Ok(())
        })
        .unwrap();
        assert_eq!(frames, [(HEADER_LEN as u64, 1, 0, 1), (synced, 2, synced, 4)]);
    }

    #[test]
    fn needs_sync_follows_unsynced_frames() {
        let sim = SimVfs::new(1);
        let (_vfs, mut w) = setup(&sim, 1);
        assert!(!w.needs_sync(), "a header alone has nothing to make durable");
        append(&mut w, 1);
        assert!(w.needs_sync());
        w.sync().unwrap();
        assert!(!w.needs_sync());
        append(&mut w, 2);
        assert!(w.needs_sync());
    }

    #[test]
    fn garbage_tail_after_valid_frames() {
        let sim = SimVfs::new(1);
        let (vfs, mut w) = setup(&sim, 1);
        append(&mut w, 1);
        let good = w.offset;
        let mut frame = seal_frame(
            1,
            2,
            0,
            1,
            &{
                let mut p = Vec::new();
                canonical(&batch(2)).encode_into(&mut p);
                p
            },
            false,
        );
        let n = frame.len();
        frame[n - 1] ^= 0x40;
        w.file.write_at(&frame, w.offset).unwrap();
        w.offset += n as u64;
        append(&mut w, 3);
        let (scan, got) = collect(&*vfs, 1).unwrap();
        assert_eq!((scan.next_seq, scan.valid_len, scan.end, got.len()), (2, good, End::Torn, 1));
        // A truncated head.
        let f = vfs.open(Path::new("/w/000001.wal"), OpenMode::Existing).unwrap();
        f.set_len(good + 7).unwrap();
        let (scan, _) = collect(&*vfs, 1).unwrap();
        assert_eq!((scan.next_seq, scan.end), (2, End::Torn));
    }

    /// In a Sync-mode log, an invalid frame followed by a valid one cannot be
    /// a torn write: every frame was durable before the next was written.
    /// Damage anywhere (body or length field) is corruption.
    #[test]
    fn damage_before_a_synced_frame_is_corruption() {
        for damaged_byte in [0usize, 2, 9, 20, 40] {
            let sim = SimVfs::new(1);
            let (vfs, mut w) = setup(&sim, 1);
            let mut offsets = Vec::new();
            for i in 1..=5 {
                offsets.push(w.offset);
                append(&mut w, i);
                w.sync().unwrap();
            }
            sim.corrupt_byte(Path::new("/w/000001.wal"), offsets[2] + damaged_byte as u64, 0x10);
            assert!(
                matches!(collect(&*vfs, 1), Err(Error::Corruption(_))),
                "damage at byte {damaged_byte} of frame 3 was taken for a torn tail"
            );
        }
        // Damage to the *last* synced frame is indistinguishable from a torn
        // write of the commit in flight: accepted as a torn tail.
        let sim = SimVfs::new(2);
        let (vfs, mut w) = setup(&sim, 1);
        for i in 1..=3 {
            append(&mut w, i);
            w.sync().unwrap();
        }
        sim.corrupt_byte(Path::new("/w/000001.wal"), w.offset - 1, 0x10);
        let (scan, _) = collect(&*vfs, 1).unwrap();
        assert_eq!((scan.next_seq, scan.end), (3, End::Torn));
    }

    #[test]
    fn range_deletions_are_refused_not_dropped() {
        let sim = SimVfs::new(1);
        let (vfs, mut w) = setup(&sim, 1);
        let mut b = WriteBatch::new();
        b.put("a", "1").delete_range("b", "c");
        w.append_group(1, &[&canonical(&b)]).unwrap();
        assert!(matches!(collect(&*vfs, 1), Err(Error::Incompatible(m)) if m.contains("range deletions")));
        // The callback never sees a commit with ranges.
        let r = replay(&*vfs, Path::new("/w"), 1, DB, 1, |_, c| {
            assert!(c.ranges.is_empty());
            Ok(())
        });
        assert!(matches!(r, Err(Error::Incompatible(_))));
    }

    // ---- preallocated and recycled logs ----

    const PREALLOC: u64 = 3 * ZERO_CHUNK as u64 / 2;

    fn file_len(vfs: &dyn Vfs, name: &str) -> u64 {
        vfs.open(&Path::new("/w").join(name), OpenMode::Existing).unwrap().len().unwrap()
    }

    fn setup_prealloc(sim: &SimVfs, base_seq: u64) -> (Arc<dyn Vfs>, WalWriter) {
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, base_seq, PREALLOC).unwrap();
        (vfs, w)
    }

    /// A preallocated log replays like any other and ends as zero padding
    /// (WAL-R4); appending and syncing never changes its length.
    #[test]
    fn preallocated_log_replays_and_keeps_its_length() {
        let sim = SimVfs::new(1);
        let (vfs, mut w) = setup_prealloc(&sim, 1);
        assert_eq!(file_len(&*vfs, "000001.wal"), PREALLOC);
        for i in 1..=20 {
            append(&mut w, i);
            w.sync().unwrap();
        }
        assert_eq!(file_len(&*vfs, "000001.wal"), PREALLOC, "the frames overwrite zeros");
        let (scan, got) = collect(&*vfs, 1).unwrap();
        assert_eq!((scan.frames, scan.commits, scan.next_seq, scan.end), (20, 20, 21, End::ZeroPadded));
        assert_eq!(scan.valid_len, w.offset);
        for (s, c) in &got {
            assert_eq!(c, &canonical(&batch(*s)).to_owned_commit());
        }
        // Nothing but the header: no frames, padding from the first byte.
        let (vfs, _) = setup_prealloc(&SimVfs::new(2), 7);
        let (scan, _) = collect(&*vfs, 7).unwrap();
        assert_eq!((scan.frames, scan.next_seq, scan.end), (0, 7, End::ZeroPadded));
    }

    /// A log that grew past its preallocation is still a valid log.
    #[test]
    fn log_may_outgrow_its_preallocation() {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, 1, 4096).unwrap();
        for i in 1..=30 {
            append(&mut w, i);
        }
        assert!(file_len(&*vfs, "000001.wal") > 4096);
        let (scan, _) = collect(&*vfs, 1).unwrap();
        assert_eq!((scan.commits, scan.end), (30, End::Clean));
    }

    /// A power loss with unsynced frames in a preallocated log leaves a
    /// prefix of the commits, whether the cut falls inside a frame (torn) or
    /// between frames (padding), and never loses a synced one.
    #[test]
    fn torn_tail_in_a_preallocated_log_yields_a_prefix() {
        let (mut torn_seen, mut padded_seen) = (0, 0);
        for seed in 0..200 {
            let sim = SimVfs::new(seed);
            let (vfs, mut w) = setup_prealloc(&sim, 1);
            for i in 1..=5 {
                append(&mut w, i);
            }
            w.sync().unwrap();
            for i in 6..=12 {
                append(&mut w, i);
            }
            let after = sim.crash_now(CrashKind::PowerLoss);
            drop(vfs);
            let (scan, got) = collect(&*after.as_vfs(), 1).unwrap();
            assert!(scan.next_seq > 5, "synced commits lost");
            assert_eq!(got.len() as u64, scan.next_seq - 1);
            for (s, c) in &got {
                assert_eq!(c, &canonical(&batch(*s)).to_owned_commit());
            }
            match scan.end {
                End::Torn => torn_seen += 1,
                End::ZeroPadded => padded_seen += 1,
                End::Clean => panic!("a preallocated log cannot end exactly at its last frame"),
            }
        }
        assert!(torn_seen > 0 && padded_seen > 0, "torn {torn_seen}, padded {padded_seen}");
    }

    /// Damage before a synced frame is corruption in a preallocated log too:
    /// the zeros after the frames do not make it look like a torn write.
    #[test]
    fn damage_before_a_synced_frame_is_corruption_when_preallocated() {
        for damaged_byte in [0usize, 2, 9, 20, 40] {
            let sim = SimVfs::new(1);
            let (vfs, mut w) = setup_prealloc(&sim, 1);
            let mut offsets = Vec::new();
            for i in 1..=5 {
                offsets.push(w.offset);
                append(&mut w, i);
                w.sync().unwrap();
            }
            sim.corrupt_byte(Path::new("/w/000001.wal"), offsets[2] + damaged_byte as u64, 0x10);
            assert!(matches!(collect(&*vfs, 1), Err(Error::Corruption(_))), "byte {damaged_byte} of frame 3");
        }
    }

    /// Frames of a recycled file's earlier life never replay, and the old
    /// tail is gone: the recycled log ends as padding, not as a torn log in
    /// the middle of the chain.
    #[test]
    fn recycled_log_does_not_replay_stale_frames() {
        let old_log = || {
            let (vfs, mut old) = setup(&SimVfs::new(1), 1);
            for i in 1..=40 {
                append(&mut old, i);
            }
            old.sync().unwrap();
            (vfs, old.offset)
        };
        let scan_as_log_2 = |vfs: &dyn Vfs| {
            let file = vfs.open(Path::new("/w/000002.wal"), OpenMode::Existing).unwrap();
            let mut src = FileSource::new(&*file).unwrap();
            let params = WalScanParams { file_number: 2, db_id: DB, expected_seq: 1 };
            let mut commits = 0;
            let scan = scan_wal(&mut src, &params, |_, f| {
                commits += f.commits.len();
                Ok(())
            })
            .unwrap();
            (commits, scan)
        };

        // Renamed and overwritten without zeroing: the stale frames carry
        // another file number, so they do not replay, but they are not padding
        // either: this log would be a torn log wherever it sits in the chain.
        let (vfs, _) = old_log();
        vfs.rename(Path::new("/w/000001.wal"), Path::new("/w/000002.wal")).unwrap();
        let file = vfs.open(Path::new("/w/000002.wal"), OpenMode::Existing).unwrap();
        file.write_at(&Header::wal(DB, 2, 1).encode(), 0).unwrap();
        let mut w = WalWriter { file, number: 2, offset: HEADER_LEN as u64, synced_len: 0 };
        for i in 1..=3 {
            append(&mut w, i);
        }
        let (commits, scan) = scan_as_log_2(&*vfs);
        assert_eq!((commits, scan.end), (3, End::Torn), "stale bytes are not padding");

        // Recycled through the spare: zeros, so padding.
        let (vfs, old_len) = old_log();
        assert!(prepare_spare(&*vfs, Path::new("/w"), Some((1, old_len)), PREALLOC, &|| false).unwrap());
        assert!(!vfs.exists(Path::new("/w/000001.wal")).unwrap());
        assert_eq!(file_len(&*vfs, SPARE_WAL), PREALLOC.max(old_len));
        let mut w = WalWriter::create_from_spare(&*vfs, Path::new("/w"), 2, DB, 1).unwrap();
        for i in 1..=3 {
            append(&mut w, i);
        }
        w.sync().unwrap();
        let (commits, scan) = scan_as_log_2(&*vfs);
        assert_eq!((commits, scan.next_seq, scan.end), (3, 4, End::ZeroPadded));
        assert_eq!(scan.valid_len, w.offset);
    }

    /// A recycled log is overwritten only up to the bytes that were written,
    /// whether it was preallocated (the rest is zero already), grew (the rest
    /// is added), or the length written is not known.
    #[test]
    fn recycling_rewrites_only_the_written_prefix() {
        use kerno_testkit::vfs::CountingVfs;
        for (prealloc, known) in [(PREALLOC, true), (0, true), (PREALLOC, false)] {
            let (vfs, io) = CountingVfs::wrap(SimVfs::new(5).as_vfs());
            vfs.create_dir_all(Path::new("/w")).unwrap();
            let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, 1, prealloc).unwrap();
            for i in 1..=30 {
                append(&mut w, i);
            }
            w.sync().unwrap();
            let (written, len) = (w.written(), file_len(&*vfs, "000001.wal"));
            let before = io.snapshot();
            let used = if known { written } else { 0 };
            assert!(prepare_spare(&*vfs, Path::new("/w"), Some((1, used)), PREALLOC, &|| false).unwrap());
            let rewritten = io.snapshot().bytes_written - before.bytes_written;
            let expected = if known { written + PREALLOC.saturating_sub(len) } else { len.max(PREALLOC) };
            assert_eq!(rewritten, expected, "preallocated {prealloc}, known {known}");
            assert_eq!(file_len(&*vfs, SPARE_WAL), len.max(PREALLOC));
            let f = vfs.open(&Path::new("/w").join(SPARE_WAL), OpenMode::Existing).unwrap();
            let mut all = vec![1u8; len.max(PREALLOC) as usize];
            f.read_at(&mut all, 0).unwrap();
            assert!(all.iter().all(|&b| b == 0), "prealloc {prealloc}, known {known}");
        }
    }

    /// The spare is made of zeros and renamed only when complete: an aborted
    /// preparation leaves no file, and a fresh one replaces a leftover.
    #[test]
    fn spare_is_complete_or_absent() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let spare = Path::new("/w").join(SPARE_WAL);
        // Aborted after the first chunk: nothing is left.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let abort = || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 1;
        assert!(!prepare_spare(&*vfs, Path::new("/w"), None, PREALLOC, &abort).unwrap());
        assert!(!vfs.exists(&spare).unwrap());
        // A leftover (half written) spare is replaced by a new one.
        vfs.open(&spare, OpenMode::CreateNew).unwrap().write_at(b"junk", 0).unwrap();
        assert!(prepare_spare(&*vfs, Path::new("/w"), None, PREALLOC, &|| false).unwrap());
        let f = vfs.open(&spare, OpenMode::Existing).unwrap();
        assert_eq!(f.len().unwrap(), PREALLOC);
        let mut all = vec![1u8; PREALLOC as usize];
        f.read_at(&mut all, 0).unwrap();
        assert!(all.iter().all(|&b| b == 0));
    }

    /// A crash at any operation of recycling a retired log into the spare
    /// leaves the live log intact and no canonical log but the live one and
    /// the retired one (which a crash before the rename leaves where it was).
    #[test]
    fn crash_during_spare_preparation_leaves_the_chain_intact() {
        let mut crashed_at = 0;
        for point in 1..=8u64 {
            let sim = SimVfs::new(point);
            let (vfs, mut old) = setup(&sim, 1);
            for i in 1..=10 {
                append(&mut old, i);
            }
            old.sync().unwrap();
            let retired_len = old.written();
            let mut live = WalWriter::create(&*vfs, Path::new("/w"), 2, DB, 11, 0).unwrap();
            for i in 11..=13 {
                live.append_group(i, &[&canonical(&batch(i))]).unwrap();
            }
            live.sync().unwrap();
            sim.crash_after(point, CrashKind::PowerLoss);
            let _ = prepare_spare(&*vfs, Path::new("/w"), Some((1, retired_len)), PREALLOC, &|| false);
            let after = if sim.has_crashed() {
                crashed_at += 1;
                sim.take_crash_image().unwrap()
            } else {
                sim.crash_now(CrashKind::PowerLoss)
            };
            let after = after.as_vfs();
            for n in after.list(Path::new("/w")).unwrap().iter().filter(|n| n.ends_with(".wal")) {
                assert!(n == "000001.wal" || n == "000002.wal", "unexpected canonical log {n} (point {point})");
            }
            let scan = replay(&*after, Path::new("/w"), 2, DB, 11, |_, _| Ok(())).unwrap();
            assert_eq!((scan.commits, scan.next_seq), (3, 14), "point {point}");
        }
        assert!(crashed_at >= 5, "the sweep crashed inside the preparation only {crashed_at} times");
    }
}
