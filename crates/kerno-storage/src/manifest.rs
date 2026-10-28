//! The MANIFEST, format v1 (`docs/file-format/v1.md` §8): the durable shape
//! of the LSM tree.
//!
//! Every change of the shape writes a whole new MANIFEST generation that
//! holds a single `SNAPSHOT` record, through the M0 protocol: write
//! `MANIFEST.tmp`, `sync_data`, `rename`, `sync_dir` (W-10). Generations
//! increase by one each time and `edit_seq` continues across them.
// ponytail: one snapshot per generation costs two syncs and a rewrite per
// flush or compaction; WP08 appends records to the current file instead.

use std::path::Path;

use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Error, Result};

use crate::files::{FileSource, read_error, require_read_write};
use crate::format::header::Header;
use crate::format::limits::{MAX_FILE_NUMBER, MAX_SEQ};
use crate::format::manifest::{Edit, ManifestState, Record, encode_manifest, replay_manifest};

pub use crate::format::manifest::FileMeta;

pub const MANIFEST: &str = "MANIFEST";
pub const MANIFEST_TMP: &str = "MANIFEST.tmp";

/// A MANIFEST read from disk.
pub struct Loaded {
    pub db_id: [u8; 16],
    pub state: ManifestState,
    /// Where the next generation continues.
    pub writer: ManifestWriter,
}

/// Reads and validates the MANIFEST (HDR-1..9, MAN-R*, MAN-S*). A broken
/// tail after the first record is accepted as the format says (MAN-R3) and
/// ignored: this engine rewrites the whole file at the start of every
/// session.
pub fn read(vfs: &dyn Vfs, dir: &Path) -> Result<Loaded> {
    let file = vfs.open(&dir.join(MANIFEST), OpenMode::Existing)?;
    let mut src = FileSource::new(&*file)?;
    let replay = replay_manifest(&mut src, |_, _| Ok(())).map_err(|e| read_error(MANIFEST, e))?;
    require_read_write(MANIFEST, &replay.header)?;
    Ok(Loaded {
        db_id: replay.header.db_id,
        state: replay.state,
        writer: ManifestWriter {
            db_id: replay.header.db_id,
            generation: replay.header.file_number,
            next_edit_seq: replay.next_edit_seq,
        },
    })
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

/// Position of the MANIFEST on disk: the generation that is current and the
/// `edit_seq` the next record carries.
#[derive(Clone, Debug)]
pub struct ManifestWriter {
    db_id: [u8; 16],
    generation: u64,
    next_edit_seq: u64,
}

fn exhausted(what: &str) -> Error {
    Error::InvalidArgument(format!("{what} exhausted"))
}

impl ManifestWriter {
    /// The writer of a database that has no MANIFEST yet (DIR-3).
    pub fn new(db_id: [u8; 16]) -> Self {
        ManifestWriter { db_id, generation: 0, next_edit_seq: 1 }
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

    #[test]
    fn generations_chain_through_the_vfs() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        let first = read(&*vfs, dir).unwrap();
        assert_eq!(first.db_id, DB);
        assert_eq!((first.writer.generation, first.writer.next_edit_seq), (1, 2));
        assert_eq!((first.state.next_file, first.state.log_number, first.state.last_seq), (1, 1, 0));

        let levels = sample();
        let snap = Snapshot { next_file: 10, last_seq: 42, log_number: 9, clean: false, levels: &levels };
        let mut w = first.writer;
        w.checkpoint(&*vfs, dir, &snap).unwrap();
        let second = read(&*vfs, dir).unwrap();
        assert_eq!((second.writer.generation, second.writer.next_edit_seq), (2, 3));
        assert_eq!(second.state.levels, levels);
        assert_eq!((second.state.next_file, second.state.log_number, second.state.last_seq), (10, 9, 42));
        assert!(!vfs.exists(&dir.join(MANIFEST_TMP)).unwrap());

        // A clean snapshot needs log_number == next_file.
        let clean = Snapshot { next_file: 10, last_seq: 42, log_number: 10, clean: true, levels: &levels };
        w.checkpoint(&*vfs, dir, &clean).unwrap();
        assert_eq!(read(&*vfs, dir).unwrap().writer.generation, 3);
    }

    #[test]
    fn a_failed_checkpoint_keeps_the_position_and_a_stale_tmp_is_replaced() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        create(&*vfs, dir, DB).unwrap();
        vfs.open(&dir.join(MANIFEST_TMP), OpenMode::CreateNew).unwrap().write_at(b"stale", 0).unwrap();
        let mut w = read(&*vfs, dir).unwrap().writer;
        let levels = sample();
        let bad =
            Snapshot { next_file: MAX_FILE_NUMBER + 1, last_seq: 1, log_number: 1, clean: false, levels: &levels };
        assert!(matches!(w.checkpoint(&*vfs, dir, &bad), Err(Error::InvalidArgument(_))));
        assert_eq!((w.generation, w.next_edit_seq), (1, 2));
        let ok = Snapshot { next_file: 10, last_seq: 42, log_number: 9, clean: false, levels: &levels };
        w.checkpoint(&*vfs, dir, &ok).unwrap();
        assert_eq!(read(&*vfs, dir).unwrap().writer.generation, 2);
        assert!(!vfs.exists(&dir.join(MANIFEST_TMP)).unwrap());
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
