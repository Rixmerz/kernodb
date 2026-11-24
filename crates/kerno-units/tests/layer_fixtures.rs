//! E9-01: the committed record vectors of the units layer
//! (`docs/architecture/stage9-fixtures.md`).
//!
//! * `v1/structs`: every committed record of the current record set is read by
//!   the production decoder: a positive vector decodes, a `neg_unitNN_*`
//!   vector is refused with rule `UNIT-NN`.
//!
//! The `derived-v2/db-with-vector` directory fixture (a database at layer mark
//! 2) was removed with the format cut of ADR-0023: no build opens a database
//! below layer 7, so it describes nothing this build can read.

use std::path::PathBuf;

use kerno_units::codec::Record;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

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
    assert_eq!((positive, negative), (35, 107));
}
