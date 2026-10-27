//! Media-corruption suite: after a clean shutdown, flip one bit somewhere in
//! the database files and check that the engine either still returns exactly
//! the right data or reports an error. Returning different data, or silently
//! going back to an older commit, fails the test.

use std::path::PathBuf;

use super::{Model, Workload, apply, for_each_seed, open, read_all, seeds};
use crate::engine::{Engine, Options, Snapshot};
use crate::rng::Rng;
use crate::vfs::SimVfs;

#[derive(Default, Debug)]
struct Outcomes {
    detected_at_open: u64,
    detected_on_read: u64,
    harmless: u64,
}

pub fn bit_flips<E: Engine>() {
    let outcomes = std::sync::Mutex::new(Outcomes::default());
    for_each_seed("bit_flips", seeds(150), |seed| {
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
        let total: u64 = files.iter().map(|(_, l)| l).sum();
        let mut pick = rng.below(total);
        let (path, offset) = files
            .iter()
            .find_map(|(p, len)| {
                if pick < *len {
                    Some((p.clone(), pick))
                } else {
                    pick -= len;
                    None
                }
            })
            .unwrap();
        let mask = 1u8 << rng.below(8);
        disk.corrupt_byte(&path, offset, mask);
        let what = format!("bit {mask:#04x} at {}+{offset}", path.display());

        let mut out = outcomes.lock().unwrap();
        let db: E = match open(&vfs, &opts) {
            Ok(db) => db,
            Err(_) => {
                out.detected_at_open += 1;
                return;
            }
        };
        let snap = match db.snapshot() {
            Ok(s) => s,
            Err(_) => {
                out.detected_at_open += 1;
                return;
            }
        };
        assert_eq!(snap.seq(), seq, "{what}: database silently rolled back to an older commit");
        let mut detected = false;
        match read_all(&snap) {
            Ok(all) => assert!(all == model, "{what}: scan returned wrong data: {}", super::diff(&model, &all)),
            Err(_) => detected = true,
        }
        for (k, v) in &model {
            match snap.get(k) {
                Ok(got) => {
                    assert!(got.as_deref() == Some(v.as_slice()), "{what}: get({}) returned wrong data", super::show(k))
                }
                Err(_) => detected = true,
            }
        }
        if detected {
            out.detected_on_read += 1;
        } else {
            out.harmless += 1;
        }
    });
    eprintln!("{} bit flips: {:?}", E::name(), outcomes.lock().unwrap());
}
