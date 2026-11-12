//! E9-01: fixtures of earlier versions (`docs/architecture/etapa9-fixtures.md`).
//!
//! * `v1/structs`: every committed layer-1 record is read by the production
//!   decoder: a positive vector decodes, a `neg_unitNN_*` vector is refused
//!   with rule `UNIT-NN`.
//! * `derived-v2/db-with-vector`: the database directory the ADR-0020
//!   criteria T5 to T7 name (one unit and its vector, closed, layer mark 2).
//!   Its bytes are committed and listed with their SHA-256 in
//!   `derived-v2/MANIFEST`. A new build does not reproduce them byte for byte
//!   (the engine draws the database identity at creation); it reproduces the
//!   file names and sizes, which is what is compared. The refusal by a build
//!   without the feature (T5) is a command on a binary built from the base
//!   commit; the document records it.
//!
//! Every database test runs on both engines (`emulated_log` and `kerno_db`).
//! The directory itself can only be opened by the engine that has files, so
//! `emulated_log` builds the same content in memory and gets the same checks.
//!
//! To regenerate the directory after a deliberate layout change:
//!
//! ```text
//! cargo test --release -p kerno-units --test layer_fixtures -- --ignored regenerate_derived_v2
//! ```

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harness::units::{Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::sha256;
use kerno_testkit::vfs::{DirImage, SimVfs, dir_difference};
use kerno_units::codec::{Key, LAYER_VERSION, LayerMeta, MAX_LAYER_VERSION, Record};
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const FIXTURE_DIR: &str = "tests/vectors/derived-v2/db-with-vector";
const MANIFEST: &str = "tests/vectors/derived-v2/MANIFEST";
const DB: &str = "/db";
/// Seed of the simulated disk that writes the fixture.
const SEED: u64 = 6;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn recipe() -> VectorRecipe {
    let params = BTreeMap::from([("dim".to_owned(), "4".to_owned()), ("norm".to_owned(), "l2".to_owned())]);
    VectorRecipe { method: "embed".into(), version: 1, profile: "model-a".into(), params }
}

fn embed(payload: &[u8]) -> Vec<f32> {
    vec![payload.len() as f32, payload.iter().map(|b| f32::from(*b)).sum(), 0.5, 0.0]
}

fn vcompute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<f32>> {
    inputs.first().map(|i| embed(&i.payload))
}

const PAYLOAD: &[u8] = b"hello";

/// Writes the fixture content: one unit and its vector, closed.
fn build<E: Target>(disk: &SimVfs) -> UnitId {
    let (dd, _) = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg()).unwrap()).unwrap();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let unit = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: PAYLOAD.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    dd.build_vector(unit, &recipe(), ChangeMeta::default(), &vcompute).unwrap();
    dd.close().unwrap();
    unit
}

// ----------------------------------------------------------- the directory

/// Files of the committed directory, by name (read from disk).
fn committed_files() -> BTreeMap<String, Vec<u8>> {
    let dir = root().join(FIXTURE_DIR);
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().expect("utf-8 name");
        out.insert(name, std::fs::read(entry.path()).unwrap());
    }
    out
}

fn image_of(files: &BTreeMap<String, Vec<u8>>) -> DirImage {
    DirImage {
        dirs: BTreeSet::from([PathBuf::from(DB)]),
        files: files.iter().map(|(n, b)| (Path::new(DB).join(n), b.clone())).collect(),
        unlinked_open: Vec::new(),
    }
}

/// `sha256  size  name`, one line per file, in name order.
fn manifest_of(files: &BTreeMap<String, Vec<u8>>) -> String {
    files.iter().map(|(n, b)| format!("{}  {}  {n}\n", sha256::hex(&sha256::digest(b)), b.len())).collect()
}

/// The files the build writes, without the lock file (the engine makes it on open).
fn written_files(disk: &SimVfs) -> BTreeMap<String, Vec<u8>> {
    disk.dump()
        .files
        .into_iter()
        .filter_map(|(p, b)| {
            let name = p.file_name()?.to_str()?.to_owned();
            (name != "LOCK").then_some((name, b))
        })
        .collect()
}

#[test]
#[ignore = "writes the committed fixture; run by hand after a deliberate layout change"]
fn regenerate_derived_v2() {
    let disk = SimVfs::new(SEED);
    build::<KernoDb>(&disk);
    let files = written_files(&disk);
    let dir = root().join(FIXTURE_DIR);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    std::fs::write(root().join(MANIFEST), manifest_of(&files)).unwrap();
}

#[test]
fn derived_v2_manifest_lists_the_committed_files() {
    let files = committed_files();
    assert!(!files.is_empty(), "the fixture directory is empty");
    let want = std::fs::read_to_string(root().join(MANIFEST)).expect("MANIFEST");
    assert_eq!(manifest_of(&files), want, "a file of {FIXTURE_DIR} differs from MANIFEST");
}

#[test]
fn derived_v2_directory_has_the_shape_the_build_writes() {
    let disk = SimVfs::new(SEED);
    build::<KernoDb>(&disk);
    let built = written_files(&disk);
    let committed = committed_files();
    assert_eq!(
        built.keys().collect::<Vec<_>>(),
        committed.keys().collect::<Vec<_>>(),
        "the build writes other file names than the fixture"
    );
    // Not byte for byte: the engine draws a database identity at creation, so
    // two builds differ in the bytes that hold it, not in size.
    for (name, bytes) in &built {
        assert_eq!(bytes.len(), committed[name].len(), "{name}: size differs from the committed fixture");
    }
}

// --------------------------------------------- the fixture, on both engines

/// The disk to check: the committed directory for the engine that has
/// files, the same content built in memory for the other.
fn disk_of<E: Target>() -> SimVfs {
    if E::NAME == <KernoDb as Target>::NAME {
        SimVfs::restore(&image_of(&committed_files()))
    } else {
        let disk = SimVfs::new(SEED);
        build::<E>(&disk);
        disk
    }
}

fn mark_bytes<E: Target>(db: &UnitDb<E>) -> Vec<u8> {
    let key = Key::LayerMeta { name: "layer".into() }.encode().unwrap();
    raw(db).into_iter().find(|(k, _)| k == &key).expect("layer mark").1
}

/// T6: the reader of layer 1 refuses the mark with UNIT-16 and the version found.
fn layer1_reader_refuses_the_fixture<E: Target>() {
    let disk = disk_of::<E>();
    let db = open_units::<E>(disk.as_vfs(), &cfg()).unwrap();
    let mark = mark_bytes(&db);
    let e = LayerMeta::decode(&mark).unwrap_err();
    assert_eq!((e.rule, e.found_layer_version), (UnitRule::Unit16, Some(2)));
    assert_eq!(e.class(), ErrorClass::IncompatibleLayer);
    assert!(matches!(UnitError::from(e), UnitError::IncompatibleLayer { found: 2, supported: 1 }));
    assert_eq!(LAYER_VERSION, 1);
    assert_eq!(LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).unwrap().layer_version, 2);
    db.close().unwrap();
}

/// T7: this build opens it, serves the vector and the unit, and finds nothing wrong.
fn this_build_opens_and_serves_the_fixture<E: Target>() {
    let disk = disk_of::<E>();
    let (dd, report) = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg()).unwrap()).unwrap();
    assert_eq!(report.served, 1, "{report:?}");
    assert!(report.demoted.is_empty() && report.findings.is_empty(), "{report:?}");
    assert!(dd.get_unit(UnitId(unit_bytes())).unwrap().is_some());
    dd.close().unwrap();
    let db = open_units::<E>(disk.as_vfs(), &cfg()).unwrap();
    let v = db.verify_units().unwrap();
    assert!(v.is_clean(), "{:?}", v.findings);
}

/// The unit id the first change set mints: the counter starts at 1.
fn unit_bytes() -> [u8; 16] {
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&1u64.to_be_bytes());
    id
}

/// T7, second half: opening the committed directory changes no file but the
/// ones an open of the bare engine changes (the engine starts a log of its own).
fn opening_the_fixture_adds_nothing<E: Target>() {
    let image = disk_of::<E>().dump();
    let plain = SimVfs::restore(&image);
    drop(open_units::<E>(plain.as_vfs(), &cfg()).unwrap());
    let layered = SimVfs::restore(&image);
    let (dd, _) = DerivedDb::open_persisted(open_units::<E>(layered.as_vfs(), &cfg()).unwrap()).unwrap();
    drop(dd);
    if let Some(d) = dir_difference(&plain.dump(), &layered.dump(), &["LOCK"]) {
        panic!("opening the derived layer wrote more than the engine does: {d}");
    }
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            #[test]
            fn layer1_reader_refuses_the_fixture() {
                super::layer1_reader_refuses_the_fixture::<$e>();
            }
            #[test]
            fn this_build_opens_and_serves_the_fixture() {
                super::this_build_opens_and_serves_the_fixture::<$e>();
            }
            #[test]
            fn opening_the_fixture_adds_nothing() {
                super::opening_the_fixture_adds_nothing::<$e>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);

// ------------------------------------------------------------- v1 structs

/// `key_len: u32be ‖ key ‖ value`.
fn split(bytes: &[u8]) -> (&[u8], &[u8]) {
    let n = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    (&bytes[4..4 + n], &bytes[4 + n..])
}

#[test]
fn v1_struct_vectors_are_read_by_the_production_decoder() {
    let dir = root().join("tests/vectors/v1/structs");
    let (mut positive, mut negative) = (0, 0);
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    for path in names {
        let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let bytes = std::fs::read(&path).unwrap();
        let (key, value) = split(&bytes);
        let got = Record::decode(key, value);
        match stem.strip_prefix("neg_unit") {
            None => {
                got.unwrap_or_else(|e| panic!("{stem}: {e:?}"));
                positive += 1;
            }
            Some(rest) => {
                let want: u8 = rest.split('_').next().unwrap().parse().unwrap();
                let e = got.expect_err(&stem);
                assert_eq!(e.rule.number(), want, "{stem}: {e:?}");
                negative += 1;
            }
        }
    }
    // The counts are those of the committed directory: a vector added or lost shows up.
    assert_eq!((positive, negative), (33, 100));
}
