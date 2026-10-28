//! The MANIFEST, format v1 (`docs/file-format/v1.md` §8): the durable shape
//! of the LSM tree, kept as an edit log.
//!
//! A generation of the file is a `SNAPSHOT` record (the whole state) followed
//! by one appended record per change of the shape (`SESSION_OPEN`, `FLUSH`,
//! `COMPACTION`). A record is one `write_at` and one `sync_data` (W-3). The
//! file is replaced by a new generation holding a single `SNAPSHOT` (W-10:
//! write `MANIFEST.tmp`, `sync_data`, `rename`, `sync_dir`) when it is closed
//! cleanly, when a torn tail was found at open (W-7), when the appended
//! bytes exceed `max(floor, 4 x` the size of record 0`)` and on request.
//! Generations increase by one each time and `edit_seq` continues across
//! them.

use std::path::Path;
use std::sync::Arc;

use kerno_core::vfs::{File, OpenMode, Vfs};
use kerno_core::{Error, Result};

use crate::files::{FileSource, read_error};
use crate::format::End;
use crate::format::header::{HEADER_LEN, Header};
use crate::format::limits::{MAX_FILE_NUMBER, MAX_SEQ};
use crate::format::manifest::{Edit, ManifestReplay, Record, encode_manifest, replay_manifest};

pub use crate::format::manifest::FileMeta;

pub const MANIFEST: &str = "MANIFEST";
pub const MANIFEST_TMP: &str = "MANIFEST.tmp";

/// The floor of the size trigger of a checkpoint (W-10): appended bytes over
/// `max(CHECKPOINT_FLOOR, 4 x record 0)`.
pub const CHECKPOINT_FLOOR: u64 = 4 << 20;

/// A MANIFEST read from disk.
pub struct Loaded {
    /// The validated replay: header, state, how the file ends.
    pub replay: ManifestReplay,
    /// Bytes of record 0 (the `SNAPSHOT`), the base of the size trigger.
    record0_len: u64,
    file: Arc<dyn File>,
}

/// Reads and validates the MANIFEST (HDR-1..9, MAN-R*, MAN-S*). A broken
/// tail after the last valid record is accepted as the format says (MAN-R3);
/// the writer then starts a new generation instead of appending (W-7).
pub fn read(vfs: &dyn Vfs, dir: &Path) -> Result<Loaded> {
    let file = vfs.open(&dir.join(MANIFEST), OpenMode::Existing)?;
    let mut src = FileSource::new(&*file)?;
    let (mut seen, mut second) = (0u64, None);
    let replay = replay_manifest(&mut src, |at, _| {
        if seen == 1 {
            second = Some(at);
        }
        seen += 1;
        Ok(())
    })
    .map_err(|e| read_error(MANIFEST, e))?;
    let record0_len = second.unwrap_or(replay.valid_len) - u64::from(replay.header.header_len);
    Ok(Loaded { replay, record0_len, file })
}

impl Loaded {
    /// The writer that continues this file. `floor` is the size trigger's
    /// floor ([`CHECKPOINT_FLOOR`]).
    pub fn writer(&self, floor: u64) -> ManifestWriter {
        ManifestWriter {
            db_id: self.replay.header.db_id,
            generation: self.replay.header.file_number,
            next_edit_seq: self.replay.next_edit_seq,
            file: Some(self.file.clone()),
            len: self.replay.valid_len,
            record0_len: self.record0_len,
            unsafe_tail: self.replay.end != End::Clean,
            floor,
        }
    }
}

/// The state a new generation records.
pub struct Snapshot<'a> {
    pub next_file: u64,
    pub last_seq: u64,
    pub log_number: u64,
    /// The `CLEAN` flag: requires `log_number == next_file`.
    pub clean: bool,
    /// Seven levels: L0 newest first, deeper levels by `smallest`.
    pub levels: &'a [Vec<FileMeta>],
}

/// Position of the MANIFEST on disk: the generation that is current, the
/// handle records are appended through and the `edit_seq` the next one
/// carries.
pub struct ManifestWriter {
    db_id: [u8; 16],
    generation: u64,
    next_edit_seq: u64,
    /// The current generation (none until the first checkpoint).
    file: Option<Arc<dyn File>>,
    /// Where the next record goes: the end of the last valid one.
    len: u64,
    record0_len: u64,
    /// The file ends in bytes that are not a valid record: nothing may be
    /// appended after them (W-7).
    unsafe_tail: bool,
    floor: u64,
}

fn exhausted(what: &str) -> Error {
    Error::InvalidArgument(format!("{what} exhausted"))
}

impl ManifestWriter {
    /// The writer of a database that has no MANIFEST yet (DIR-3).
    pub fn new(db_id: [u8; 16]) -> Self {
        ManifestWriter {
            db_id,
            generation: 0,
            next_edit_seq: 1,
            file: None,
            len: 0,
            record0_len: 0,
            unsafe_tail: false,
            floor: CHECKPOINT_FLOOR,
        }
    }

    /// Bytes appended to this generation after record 0.
    pub fn appended_bytes(&self) -> u64 {
        self.len.saturating_sub(HEADER_LEN as u64 + self.record0_len)
    }

    /// Bytes of the current file.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the next change must be a checkpoint rather than an append:
    /// a torn tail (W-7) or appended bytes over `max(floor, 4 x record 0)`.
    pub fn checkpoint_due(&self) -> bool {
        self.unsafe_tail || self.appended_bytes() > self.floor.max(4 * self.record0_len)
    }

    /// Appends `edit` as the next record: one `write_at` and one
    /// `sync_data`, durable when this returns `Ok` (W-3). Returns the bytes
    /// of the record. On failure the writer keeps its position and the
    /// caller poisons the handle: the tail may now be invalid, and nothing
    /// is appended after it (W-7).
    pub fn append(&mut self, edit: Edit) -> Result<u64> {
        let file = self.file.as_ref().filter(|_| !self.unsafe_tail).ok_or_else(|| {
            Error::Corruption("internal: append to a MANIFEST that needs a checkpoint first (W-7)".into())
        })?;
        let edit_seq = self.next_edit_seq;
        if edit_seq > MAX_SEQ {
            return Err(exhausted("MANIFEST edit sequence"));
        }
        let bytes = Record { edit_seq, edit, ignored: Vec::new() }.encode(self.generation);
        file.write_at(&bytes, self.len)?;
        file.sync_data()?;
        self.len += bytes.len() as u64;
        self.next_edit_seq = edit_seq + 1;
        Ok(bytes.len() as u64)
    }

    /// Atomically replaces the MANIFEST with the next generation (W-10);
    /// durable when this returns `Ok`. On failure the writer keeps its
    /// position and the caller poisons the handle.
    pub fn checkpoint(&mut self, vfs: &dyn Vfs, dir: &Path, snap: &Snapshot<'_>) -> Result<()> {
        let generation = self.generation.checked_add(1).ok_or_else(|| exhausted("MANIFEST generation"))?;
        let edit_seq = self.next_edit_seq;
        if edit_seq > MAX_SEQ {
            return Err(exhausted("MANIFEST edit sequence"));
        }
        if snap.next_file > MAX_FILE_NUMBER {
            return Err(exhausted("file number space"));
        }
        let record = Record {
            edit_seq,
            edit: Edit::Snapshot {
                next_file: snap.next_file,
                last_seq: snap.last_seq,
                log_number: snap.log_number,
                clean: snap.clean,
                prev_generation: self.generation,
                files: snap.levels.iter().flatten().cloned().collect(),
            },
            ignored: Vec::new(),
        };
        let bytes = encode_manifest(&Header::manifest(self.db_id, generation, edit_seq), &[record]);
        let tmp = dir.join(MANIFEST_TMP);
        if vfs.exists(&tmp)? {
            vfs.remove_file(&tmp)?;
        }
        let f = vfs.open(&tmp, OpenMode::CreateNew)?;
        f.write_at(&bytes, 0)?;
        f.sync_data()?;
        vfs.rename(&tmp, &dir.join(MANIFEST))?;
        vfs.sync_dir(dir)?;
        self.generation = generation;
        self.next_edit_seq = edit_seq + 1;
        self.file = Some(f);
        self.len = bytes.len() as u64;
        self.record0_len = self.len - HEADER_LEN as u64;
        self.unsafe_tail = false;
        Ok(())
    }
}

/// DIR-3 step 3: the first MANIFEST of a new database, generation 1 with an
/// empty `CLEAN` snapshot.
pub fn create(vfs: &dyn Vfs, dir: &Path, db_id: [u8; 16]) -> Result<()> {
    let empty = vec![Vec::new(); crate::compaction::NUM_LEVELS];
    let snap = Snapshot { next_file: 1, last_seq: 0, log_number: 1, clean: true, levels: &empty };
    ManifestWriter::new(db_id).checkpoint(vfs, dir, &snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_testkit::vfs::SimVfs;

    const DB: [u8; 16] = [3; 16];

    fn meta(level: u8, number: u64, lo: &str, hi: &str, seqs: (u64, u64)) -> FileMeta {
        FileMeta {
            level,
            number,
            size: 100 * number,
            point_entries: number,
            range_fragments: 0,
            smallest_seq: seqs.0,
            largest_seq: seqs.1,
            largest_exclusive: false,
            smallest: lo.into(),
            largest: hi.into(),
        }
    }

    fn sample() -> Vec<Vec<FileMeta>> {
        let mut levels = vec![Vec::new(); 7];
        levels[0] = vec![meta(0, 8, "a", "z", (30, 40)), meta(0, 5, "c", "d", (10, 20))];
        levels[1] = vec![meta(1, 3, "a", "b", (1, 9)), meta(1, 4, "c", "c", (1, 9)), meta(1, 6, "d", "k", (1, 9))];
        levels[3] = vec![meta(3, 1, "", "zzz", (1, 5))];
        levels
    }

    fn writer_of(vfs: &dyn Vfs, dir: &Path) -> ManifestWriter {
        read(vfs, dir).unwrap().writer(CHECKPOINT_FLOOR)
    }

    #[test]
    fn generations_chain_through_the_vfs() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        let first = read(&*vfs, dir).unwrap();
        assert_eq!(first.replay.header.db_id, DB);
        let mut w = first.writer(CHECKPOINT_FLOOR);
        assert_eq!((w.generation, w.next_edit_seq), (1, 2));
        let st = &first.replay.state;
        assert_eq!((st.next_file, st.log_number, st.last_seq), (1, 1, 0));

        let levels = sample();
        let snap = Snapshot { next_file: 10, last_seq: 42, log_number: 9, clean: false, levels: &levels };
        w.checkpoint(&*vfs, dir, &snap).unwrap();
        let second = read(&*vfs, dir).unwrap();
        let w2 = second.writer(CHECKPOINT_FLOOR);
        assert_eq!((w2.generation, w2.next_edit_seq), (2, 3));
        assert_eq!(second.replay.state.levels, levels);
        let st = &second.replay.state;
        assert_eq!((st.next_file, st.log_number, st.last_seq), (10, 9, 42));
        assert!(!vfs.exists(&dir.join(MANIFEST_TMP)).unwrap());

        // A clean snapshot needs log_number == next_file.
        let clean = Snapshot { next_file: 10, last_seq: 42, log_number: 10, clean: true, levels: &levels };
        w.checkpoint(&*vfs, dir, &clean).unwrap();
        assert_eq!(writer_of(&*vfs, dir).generation, 3);
    }

    #[test]
    fn a_failed_checkpoint_keeps_the_position_and_a_stale_tmp_is_replaced() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        vfs.open(&dir.join(MANIFEST_TMP), OpenMode::CreateNew).unwrap().write_at(b"stale", 0).unwrap();
        let mut w = writer_of(&*vfs, dir);
        let levels = sample();
        let bad =
            Snapshot { next_file: MAX_FILE_NUMBER + 1, last_seq: 1, log_number: 1, clean: false, levels: &levels };
        assert!(matches!(w.checkpoint(&*vfs, dir, &bad), Err(Error::InvalidArgument(_))));
        assert_eq!((w.generation, w.next_edit_seq), (1, 2));
        let ok = Snapshot { next_file: 10, last_seq: 42, log_number: 9, clean: false, levels: &levels };
        w.checkpoint(&*vfs, dir, &ok).unwrap();
        assert_eq!(writer_of(&*vfs, dir).generation, 2);
        assert!(!vfs.exists(&dir.join(MANIFEST_TMP)).unwrap());
    }

    fn flush_edit(next_file: u64, last_seq: u64, file: FileMeta) -> Edit {
        Edit::Flush { next_file, last_seq, log_number: next_file - 1, reason: None, files: vec![file] }
    }

    #[test]
    fn appended_records_replay_in_order_and_cost_one_write_and_one_sync() {
        use kerno_testkit::vfs::CountingVfs;
        let (vfs, io) = CountingVfs::wrap(SimVfs::new(3).as_vfs());
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        let mut w = writer_of(&*vfs, dir);
        let before = io.snapshot();
        let len = w.append(flush_edit(10, 5, meta(0, 8, "a", "z", (1, 5)))).unwrap();
        let d = io.snapshot().since(&before);
        assert_eq!((d.writes, d.file_syncs, d.dir_syncs), (1, 1, 0));
        w.append(flush_edit(12, 9, meta(0, 11, "b", "c", (6, 9)))).unwrap();
        assert!(w.appended_bytes() > len);
        let loaded = read(&*vfs, dir).unwrap();
        let st = &loaded.replay.state;
        assert_eq!((st.next_file, st.last_seq, st.log_number), (12, 9, 11));
        assert_eq!(st.levels[0].iter().map(|f| f.number).collect::<Vec<_>>(), [11, 8]);
        assert_eq!((loaded.replay.records, loaded.replay.end, loaded.replay.clean), (3, End::Clean, false));
        let w2 = loaded.writer(CHECKPOINT_FLOOR);
        assert_eq!((w2.next_edit_seq, w2.len, w2.appended_bytes()), (4, w.len, w.appended_bytes()));
    }

    #[test]
    fn a_torn_tail_is_not_appended_after_and_the_size_trigger_counts_appended_bytes() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        let mut w = writer_of(&*vfs, dir);
        w.append(flush_edit(10, 5, meta(0, 8, "a", "z", (1, 5)))).unwrap();
        // Garbage after the last record: a torn tail (MAN-R3).
        let f = vfs.open(&dir.join(MANIFEST), OpenMode::Existing).unwrap();
        f.write_at(&[0xAB; 40], w.len()).unwrap();
        let mut torn = writer_of(&*vfs, dir);
        assert!(torn.unsafe_tail && torn.checkpoint_due());
        assert!(matches!(torn.append(flush_edit(12, 9, meta(0, 11, "b", "c", (6, 9)))), Err(Error::Corruption(_))));
        let levels = sample();
        let snap = Snapshot { next_file: 20, last_seq: 42, log_number: 19, clean: false, levels: &levels };
        torn.checkpoint(&*vfs, dir, &snap).unwrap();
        assert!(!torn.unsafe_tail && !torn.checkpoint_due());

        // The trigger is max(floor, 4 x record 0) over the appended bytes.
        let mut small = read(&*vfs, dir).unwrap().writer(1);
        assert!(!small.checkpoint_due());
        let record0 = small.len() - HEADER_LEN as u64;
        let mut n = 30;
        while !small.checkpoint_due() {
            n += 1;
            small.append(flush_edit(n + 1, n, meta(0, n, "a", "z", (n, n)))).unwrap();
        }
        assert!(small.appended_bytes() > 4 * record0, "{} vs 4 x {record0}", small.appended_bytes());
    }

    #[test]
    fn damage_and_unknown_versions_are_reported_by_class() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        let path = dir.join(MANIFEST);
        let len = vfs.open(&path, OpenMode::Existing).unwrap().len().unwrap();
        for off in 0..len {
            sim.corrupt_byte(&path, off, 0x01);
            assert!(matches!(read(&*vfs, dir), Err(Error::Corruption(_))), "flip at {off} not detected as corruption");
            sim.corrupt_byte(&path, off, 0x01);
        }
        for cut in 0..len {
            let f = vfs.open(&path, OpenMode::Existing).unwrap();
            let mut bytes = vec![0; len as usize];
            f.read_at(&mut bytes, 0).unwrap();
            f.set_len(cut).unwrap();
            assert!(matches!(read(&*vfs, dir), Err(Error::Corruption(_))), "truncation to {cut}");
            f.set_len(len).unwrap();
            f.write_at(&bytes, 0).unwrap();
        }
        assert!(read(&*vfs, dir).is_ok());
    }
}
