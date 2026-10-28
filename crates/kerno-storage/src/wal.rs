//! Write-ahead log, format v1 (`docs/file-format/v1.md` §6): one file per
//! memtable (`NNNNNN.wal`), never reused or appended to after a restart.
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

use crate::files::{FileSource, read_error, require_read_write};
use crate::format::features::{SLOT_LEN, WRITE_INCOMPAT, frame_has_slot};
use crate::format::header::{HEADER_LEN, Header};
use crate::format::wal::{CanonicalCommit, WalHeaderState, WalScan, WalScanParams, frame_len, reseal_frame, scan_wal};

pub fn wal_name(number: u64) -> String {
    format!("{number:06}.wal")
}

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
    pub fn create(vfs: &dyn Vfs, dir: &Path, number: u64, db_id: [u8; 16], base_seq: u64) -> Result<Self> {
        let file = vfs.open(&dir.join(wal_name(number)), OpenMode::CreateNew)?;
        file.write_at(&Header::wal(db_id, number, base_seq).encode(), 0)?;
        vfs.sync_dir(dir)?;
        Ok(WalWriter { file, number, offset: HEADER_LEN as u64, synced_len: 0 })
    }

    pub fn number(&self) -> u64 {
        self.number
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

/// Replays the log `number`, which must start at sequence `expected_seq`
/// (WAL-R1), calling `apply(seq, commit)` for every commit in order. Returns
/// the scan: the header state, the commits and the length of the valid
/// prefix, and how the file ends. Which logs may end torn or have a broken
/// header is the caller's decision (REC-10).
///
/// The commits are provisional until the caller has checked the whole chain
/// of live logs; `apply` may only build in-memory state.
pub fn replay(
    vfs: &dyn Vfs,
    dir: &Path,
    number: u64,
    db_id: [u8; 16],
    expected_seq: u64,
    mut apply: impl FnMut(u64, &CanonicalCommit<Vec<u8>>),
) -> Result<WalScan> {
    let name = wal_name(number);
    let file = vfs.open(&dir.join(&name), OpenMode::Existing)?;
    let mut src = FileSource::new(&*file)?;
    let params = WalScanParams { file_number: number, db_id, expected_seq };
    let mut has_ranges = false;
    let scan = scan_wal(&mut src, &params, |_, frame| {
        for (i, commit) in frame.commits.iter().enumerate() {
            if !commit.ranges.is_empty() {
                // The caller's apply may not accept ranges; the log is refused below.
                has_ranges = true;
                continue;
            }
            apply(frame.first_seq + i as u64, commit);
        }
        Ok(())
    })
    .map_err(|e| read_error(&name, e))?;
    if let WalHeaderState::Valid(header) = &scan.header {
        require_read_write(&name, header)?;
    }
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
    use crate::format::wal::{CommitOp, canonicalize, seal_frame};
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
        let scan = replay(vfs, Path::new("/w"), 1, DB, first, |s, c| got.push((s, c.clone())))?;
        Ok((scan, got))
    }

    fn setup(sim: &SimVfs, base_seq: u64) -> (Arc<dyn Vfs>, WalWriter) {
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, base_seq).unwrap();
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
        let w = WalWriter::create(&*vfs, Path::new("/w"), 1, DB, 5).unwrap();
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
        let r = replay(&*vfs, Path::new("/w"), 1, DB, 1, |_, c| assert!(c.ranges.is_empty()));
        assert!(matches!(r, Err(Error::Incompatible(_))));
    }
}
