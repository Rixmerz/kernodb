#![no_main]
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kerno_core::Error;
use kerno_storage::{AccessMode, Background, CloseBudget, CompactionConfig, Config, KernoDb, TestConfig};
use kerno_testkit::vfs::{DirImage, SimVfs};
use libfuzzer_sys::fuzz_target;

const DIR: &str = "/db";
const DB: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const WRITE_BUFFER: usize = 4096;

/// The names the engine owns. A directory with any other entry and no
/// MANIFEST is a foreign directory, rejected as InvalidArgument by design
/// (REC-13, DIR-2); the target leaves such names out so that this answer is
/// not reported as a finding.
fn is_engine_name(name: &str) -> bool {
    let numbered = |stem: &str| stem.len() == 6 && stem.bytes().all(|b| b.is_ascii_digit()) && stem != "000000";
    name == "MANIFEST"
        || name == "MANIFEST.tmp"
        || name.strip_suffix(".wal").is_some_and(numbered)
        || name.strip_suffix(".sst").is_some_and(numbered)
}

/// The directory image of the input: a sequence of files, each
/// `name_len: u8, name, data_len: u32 LE, data`. A truncated tail or an empty
/// name ends the image; a name the engine does not own is dropped. `fuzz/seed_open.py` builds
/// the seeds from the directory vectors in this format.
fn image_of(mut data: &[u8]) -> DirImage {
    let mut files = BTreeMap::new();
    while let Some((&n, rest)) = data.split_first() {
        let n = usize::from(n);
        if n == 0 || rest.len() < n + 4 {
            break;
        }
        let (name, rest) = rest.split_at(n);
        let (len, rest) = rest.split_at(4);
        let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
        let Ok(name) = std::str::from_utf8(name) else { break };
        if len > rest.len() {
            break;
        }
        if is_engine_name(name) {
            files.insert(Path::new(DIR).join(name), rest[..len].to_vec());
        }
        data = &rest[len..];
    }
    DirImage { dirs: BTreeSet::from([PathBuf::from(DIR)]), files, unlinked_open: Vec::new() }
}

fn config(access: AccessMode) -> Config {
    Config {
        access,
        create_if_missing: true,
        write_buffer_bytes: WRITE_BUFFER,
        cache_bytes: 1 << 20,
        compaction: CompactionConfig::m0(WRITE_BUFFER),
        close_budget: CloseBudget::Bounded { max_bytes: 64 << 20, max_duration: None },
        wal_preallocate: false,
        test: TestConfig { background: Background::Inline, db_id: Some(DB) },
        ..Config::default()
    }
}

// An open may succeed or fail with Corruption or Incompatible; any other
// error kind, and any panic, is a finding. The one exception is a read-only
// open of an image with no MANIFEST: "no database" is InvalidArgument by
// design (a read-only open never creates), so it is not a finding.
fuzz_target!(|data: &[u8]| {
    let image = image_of(data);
    for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
        let disk = SimVfs::restore(&image);
        let no_database = access == AccessMode::ReadOnly && !image.files.contains_key(&Path::new(DIR).join("MANIFEST"));
        match KernoDb::open(disk.as_vfs(), Path::new(DIR), &config(access)) {
            Ok(_) | Err(Error::Corruption(_)) | Err(Error::Incompatible(_)) => {}
            Err(Error::InvalidArgument(_)) if no_database => {}
            Err(e) => panic!("open returned an unexpected error kind: {e:?}"),
        }
    }
});
