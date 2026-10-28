//! Media-corruption suites: after a clean shutdown, damage the database files
//! (flip one bit, or truncate a file) and check that the engine either still
//! returns exactly the right data or reports an error. Returning different
//! data, or silently going back to an older commit (including "starting
//! over" with an empty database), fails the test.

use std::path::PathBuf;

use kerno_core::Error;
use kerno_core::contract::{Engine, Options, Snapshot};

use super::{Model, Workload, apply, for_each_seed, open, read_all, seeds};
use crate::rng::Rng;
use crate::vfs::{OpenMode, SimVfs};

#[derive(Default, Debug)]
struct Outcomes {
    detected_at_open: u64,
    detected_on_read: u64,
    harmless: u64,
}

/// Damage must be reported as corruption or an I/O error (or, when a
/// background task found it first, as a poisoned handle) — never as an
/// unrelated error such as `InvalidArgument` or `Locked`.
fn assert_damage_error(what: &str, stage: &str, e: &Error) {
    assert!(
        matches!(e, Error::Corruption(_) | Error::Io(_) | Error::Poisoned(_)),
        "{what}: {stage} reported damage with an unexpected error kind: {e}"
    );
}

#[derive(Clone, Copy, Debug)]
enum Damage {
    BitFlip,
    Truncate,
}

pub fn bit_flips<E: Engine>() {
    damage_suite::<E>(Damage::BitFlip);
}

/// Truncation to a random shorter length (a quarter of the time, to zero):
/// lost tails, half-copied files, a file system that "forgot" a resize.
/// Files must be created atomically (e.g. temp file + rename) so that an
/// empty or short file is never mistaken for "no database yet".
pub fn truncations<E: Engine>() {
    damage_suite::<E>(Damage::Truncate);
}

fn damage_suite<E: Engine>(damage: Damage) {
    let outcomes = std::sync::Mutex::new(Outcomes::default());
    for_each_seed(&format!("{damage:?}"), seeds(150), |seed| {
        let mut rng = Rng::new(seed);
        let disk = SimVfs::new(seed);
        let vfs = disk.as_vfs();
        let opts = Options::small_for_tests();
        let mut w = Workload::new(seed);
        w.key_space = 800;
        let mut model = Model::new();
        let db: E = open(&vfs, &opts).unwrap();
        for _ in 0..rng.range(10, 250) {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        let seq = db.snapshot().unwrap().seq();
        db.close().unwrap();

        let files: Vec<(PathBuf, u64)> = disk.files().into_iter().filter(|(_, len)| *len > 0).collect();
        let (path, offset) = if rng.chance(1, 2) {
            // Uniform over all bytes on disk.
            let total: u64 = files.iter().map(|(_, l)| l).sum();
            let mut pick = rng.below(total);
            files
                .iter()
                .find_map(|(p, len)| {
                    if pick < *len {
                        Some((p.clone(), pick))
                    } else {
                        pick -= len;
                        None
                    }
                })
                .unwrap()
        } else {
            // Aimed at structural bytes, which uniform damage almost never
            // hits: the head or tail of a file (headers, footers, metas) or
            // the head of a 4 KiB-aligned block (page and block headers).
            let (p, len) = &files[rng.below(files.len() as u64) as usize];
            let offset = match rng.below(3) {
                0 => rng.below((*len).min(512)),
                1 => len - 1 - rng.below((*len).min(512)),
                _ => {
                    let block = rng.below(len.div_ceil(4096)) * 4096;
                    (block + rng.below(128)).min(len - 1)
                }
            };
            (p.clone(), offset)
        };
        let what = match damage {
            Damage::BitFlip => {
                let mask = 1u8 << rng.below(8);
                disk.corrupt_byte(&path, offset, mask);
                format!("bit {mask:#04x} at {}+{offset}", path.display())
            }
            Damage::Truncate => {
                let len = if rng.chance(1, 4) { 0 } else { offset };
                let f = vfs.open(&path, OpenMode::Existing).unwrap();
                f.set_len(len).unwrap();
                f.sync_data().unwrap();
                format!("{} truncated to {len} bytes", path.display())
            }
        };
        let mut out = outcomes.lock().unwrap();
        let db: E = match open(&vfs, &opts) {
            Ok(db) => db,
            Err(e) => {
                assert_damage_error(&what, "open", &e);
                out.detected_at_open += 1;
                return;
            }
        };
        let snap = match db.snapshot() {
            Ok(s) => s,
            Err(e) => {
                assert_damage_error(&what, "snapshot", &e);
                out.detected_at_open += 1;
                return;
            }
        };
        assert_eq!(snap.seq(), seq, "{what}: database silently rolled back to an older commit");
        let mut detected = false;
        match read_all(&snap) {
            Ok(all) => assert!(all == model, "{what}: scan returned wrong data: {}", super::diff(&model, &all)),
            Err(e) => {
                assert_damage_error(&what, "scan", &e);
                detected = true;
            }
        }
        for (k, v) in &model {
            match snap.get(k) {
                Ok(got) => {
                    assert!(got.as_deref() == Some(v.as_slice()), "{what}: get({}) returned wrong data", super::show(k))
                }
                Err(e) => {
                    assert_damage_error(&what, "get", &e);
                    detected = true;
                }
            }
        }
        if detected {
            out.detected_on_read += 1;
        } else {
            out.harmless += 1;
        }
    });
    eprintln!("{} {damage:?}: {:?}", E::name(), outcomes.lock().unwrap());
}
