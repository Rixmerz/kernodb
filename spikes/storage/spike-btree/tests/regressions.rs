//! Regression tests for defects found by the adversarial review of M0
//! (each one was first reproduced by an independent verifier, and these
//! tests are adapted from those reproductions).

use std::any::Any;
use std::io;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use spike_btree::BTreeEngine;
use spike_core::harness::{Model, Workload, apply, open, read_all};
use spike_core::vfs::{CrashKind, File, OpenMode, SimVfs, Vfs};
use spike_core::{Durability, Engine, Options, Snapshot, WriteBatch};

// ---------------------------------------------------------------------------
// Open trusted a meta that was visible but not durable (B-A).

fn put(k: &str, v: &str) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(k, v);
    b
}

/// Runs the prefix: open, commit 1 & 2 (acked), commit 3 whose meta sync
/// fails (when `fail_meta_sync_at` is Some), drop. Returns the disk.
fn prefix(seed: u64, n3: Option<u64>) -> (SimVfs, u64) {
    let disk = SimVfs::new(seed);
    let vfs = disk.as_vfs();
    let opts = Options::small_for_tests();
    let db: BTreeEngine = open(&vfs, &opts).unwrap();
    assert_eq!(db.commit(&put("a", "1")).unwrap(), 1);
    assert_eq!(db.commit(&put("a", "2")).unwrap(), 2);
    let before = disk.mutating_ops();
    match n3 {
        None => {
            assert_eq!(db.commit(&put("a", "3")).unwrap(), 3);
        }
        Some(n) => {
            disk.fail_after(n, None);
            assert!(db.commit(&put("a", "3")).is_err());
            assert!(disk.fault_injected());
        }
    }
    let used = disk.mutating_ops() - before;
    drop(db);
    (disk, used)
}

#[test]
fn reopen_on_unsynced_meta_then_power_loss_during_next_commit() {
    let opts = Options::small_for_tests();
    // Dry run: how many mutating ops does commit 3 take? The last one is the
    // meta sync_data.
    let (_, n3) = prefix(0, None);
    // Dry run 2: same path with the failed meta sync, reopen, count commit 4.
    let n4 = {
        let (disk, _) = prefix(0, Some(n3));
        let vfs = disk.as_vfs();
        let db: BTreeEngine = open(&vfs, &opts).unwrap();
        assert_eq!(db.snapshot().unwrap().seq(), 3, "reopen sees the non-durable meta 3");
        let before = disk.mutating_ops();
        assert_eq!(db.commit(&put("a", "4")).unwrap(), 4);
        disk.mutating_ops() - before
    };
    eprintln!("commit 3 uses {n3} mutating ops, commit 4 (after reopen) uses {n4}");

    let mut bad = 0;
    let mut outcomes = std::collections::BTreeMap::<String, u32>::new();
    let seeds = 400u64;
    for seed in 0..seeds {
        let (disk, _) = prefix(seed, Some(n3));
        let vfs = disk.as_vfs();
        let db: BTreeEngine = open(&vfs, &opts).unwrap();
        assert_eq!(db.snapshot().unwrap().seq(), 3);
        // Power loss right before commit 4's first sync_data (page barrier):
        // commit 4's page writes and the old pending meta-3 write are all
        // still pending.
        disk.crash_after(n4 - 2, CrashKind::PowerLoss);
        assert!(db.commit(&put("a", "4")).is_err());
        assert!(disk.has_crashed());
        drop(db);
        let image = disk.take_crash_image().expect("crash image");
        // Commit 2 was acknowledged with Durability::Sync: recovery must give
        // seq 2, 3 or 4 with the matching value.
        let outcome = match open::<BTreeEngine>(&image.as_vfs(), &opts) {
            Err(e) => {
                bad += 1;
                if bad <= 3 {
                    eprintln!("seed {seed}: open after power loss FAILED: {e}");
                }
                format!("open error: {e}")
            }
            Ok(db) => {
                let s = db.snapshot().unwrap();
                let seq = s.seq();
                match s.get(b"a") {
                    Ok(v) => {
                        let expect = seq.to_string().into_bytes();
                        if seq < 2 || v.as_deref() != Some(expect.as_slice()) {
                            bad += 1;
                            eprintln!("seed {seed}: seq {seq}, a = {:?}", v.map(String::from_utf8));
                            format!("wrong state seq {seq}")
                        } else {
                            format!("ok seq {seq}")
                        }
                    }
                    Err(e) => {
                        bad += 1;
                        if bad <= 3 {
                            eprintln!("seed {seed}: seq {seq} read FAILED: {e}");
                        }
                        format!("read error at seq {seq}: {e}")
                    }
                }
            }
        };
        *outcomes.entry(outcome).or_default() += 1;
    }
    eprintln!("outcomes over {seeds} seeds:");
    for (k, v) in &outcomes {
        eprintln!("  {v:4}  {k}");
    }
    assert_eq!(bad, 0, "{bad}/{seeds} power losses lost the acknowledged Sync commit 2");
}

#[test]
fn reopen_on_unsynced_meta_then_power_loss_during_close_byte_tearing() {
    let opts = Options::small_for_tests();
    let (_, n3) = prefix(0, None);
    let mut bad = 0;
    let mut outcomes = std::collections::BTreeMap::<String, u32>::new();
    let seeds = 400u64;
    for seed in 0..seeds {
        let (disk, _) = prefix(seed, Some(n3));
        disk.set_tear_unit(1);
        let vfs = disk.as_vfs();
        let db: BTreeEngine = open(&vfs, &opts).unwrap();
        assert_eq!(db.snapshot().unwrap().seq(), 3);
        // close = write meta 3 into slot 0, then sync_data. Crash before the sync.
        disk.crash_after(2, CrashKind::PowerLoss);
        assert!(db.close().is_err());
        let image = disk.take_crash_image().expect("crash image");
        let outcome = match open::<BTreeEngine>(&image.as_vfs(), &opts) {
            Err(e) => {
                bad += 1;
                format!("open error: {e}")
            }
            Ok(db) => {
                let s = db.snapshot().unwrap();
                let seq = s.seq();
                match s.get(b"a") {
                    Ok(v) if seq >= 2 && v.as_deref() == Some(seq.to_string().as_bytes()) => format!("ok seq {seq}"),
                    Ok(v) => {
                        bad += 1;
                        format!("wrong state seq {seq} a={:?}", v.map(String::from_utf8))
                    }
                    Err(e) => {
                        bad += 1;
                        format!("read error at seq {seq}: {e}")
                    }
                }
            }
        };
        *outcomes.entry(outcome).or_default() += 1;
    }
    eprintln!("close variant outcomes over {seeds} seeds:");
    for (k, v) in &outcomes {
        eprintln!("  {v:4}  {k}");
    }
    assert_eq!(bad, 0, "{bad}/{seeds} power losses during close lost the acknowledged Sync commit 2");
}

// ---------------------------------------------------------------------------
// One damaged copy of the newest meta after an unclean shutdown rolled the
// database back to the previous commit (B-C).

mod newest_meta {
    use super::*;

    const DIR: &str = "/db";
    const DATA: &str = "/db/data.kbt";

    #[test]
    fn damaged_newest_meta_after_unclean_shutdown_does_not_roll_back() {
        let disk = SimVfs::new(77);
        let vfs = disk.as_vfs();
        let opts = Options { durability: Durability::Sync, ..Options::small_for_tests() };
        let db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts).unwrap();
        let mut model = Model::new();
        let mut w = Workload::new(77);
        for _ in 0..39 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        let model39 = model.clone();
        // Commit 40: an acknowledged Sync commit with a recognisable key.
        let mut b = WriteBatch::new();
        b.put(b"ACKED-COMMIT-40".to_vec(), b"must survive".to_vec());
        assert_eq!(db.commit(&b).unwrap(), 40);
        apply(&mut model, &b);
        // Process exit: no clean close (Drop does no I/O).
        drop(db);

        // Slot 0 holds commit 40 (commit 1 went to slot 1). Flip one bit of its
        // sequence field (byte 20), as media corruption would.
        disk.corrupt_byte(Path::new(DATA), 20, 0x01);

        let reopened = BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts);
        let db = match reopened {
            Err(e) => {
                eprintln!("open reported the damage: {e}");
                return; // would be acceptable behaviour
            }
            Ok(db) => db,
        };
        let snap = db.snapshot().unwrap();
        let seq = snap.seq();
        let all = read_all(&snap);
        let got40 = snap.get(b"ACKED-COMMIT-40");
        eprintln!("reopened at seq {seq}; read_all ok = {}; get(ACKED-COMMIT-40) = {:?}", all.is_ok(), got40);
        if let Ok(all) = &all {
            eprintln!("contents == model after commit 39: {}", *all == model39);
            eprintln!("contents == model after commit 40: {}", *all == model);
        }
        db.verify().unwrap();
        eprintln!("verify() passes on the rolled-back state");
        drop(snap);

        // Clean close without new commits writes meta 39 into the damaged slot,
        // making the rollback permanent and undetectable.
        db.close().unwrap();
        let db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts).unwrap();
        let seq_after_close = db.snapshot().unwrap().seq();
        eprintln!("after clean close and reopen: seq {seq_after_close}");
        // A new commit reuses sequence number 40 with different content.
        let mut b = WriteBatch::new();
        b.put(b"other".to_vec(), b"content".to_vec());
        let reused = db.commit(&b).unwrap();
        eprintln!("next commit got seq {reused}");
        drop(db);

        assert_eq!(
            seq,
            40,
            "acknowledged Sync commit 40 silently lost: open succeeded at seq {seq} with no error \
         (read_all ok = {}, get(ACKED-COMMIT-40) = {:?})",
            all.is_ok(),
            got40
        );
    }
}

// ---------------------------------------------------------------------------
// A crc-valid but malformed file made commit panic (B-E).

mod crafted {
    use super::*;

    const PAGE: usize = 4096;

    fn leaf(id: u64, entries: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut p = Vec::with_capacity(PAGE);
        p.push(1u8); // leaf
        p.push(0u8); // level
        p.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        p.extend_from_slice(&id.to_le_bytes());
        for (k, v) in entries {
            p.extend_from_slice(&(k.len() as u16).to_le_bytes());
            p.push(0); // inline
            p.extend_from_slice(&(v.len() as u32).to_le_bytes());
            p.extend_from_slice(k);
            p.extend_from_slice(v);
        }
        assert!(p.len() <= PAGE, "leaf too big: {}", p.len());
        p.resize(PAGE, 0);
        p
    }

    fn branch(id: u64, level: u8, child0: (u64, u32), rest: &[(&[u8], (u64, u32))]) -> Vec<u8> {
        let mut p = Vec::with_capacity(PAGE);
        p.push(2u8); // branch
        p.push(level);
        p.extend_from_slice(&(rest.len() as u16).to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(&child0.0.to_le_bytes());
        p.extend_from_slice(&child0.1.to_le_bytes());
        for (k, (cid, ccrc)) in rest {
            p.extend_from_slice(&(k.len() as u16).to_le_bytes());
            p.extend_from_slice(k);
            p.extend_from_slice(&cid.to_le_bytes());
            p.extend_from_slice(&ccrc.to_le_bytes());
        }
        p.resize(PAGE, 0);
        p
    }

    fn meta(seq: u64, root: (u64, u32), height: u32, page_count: u64, entries: u64) -> Vec<u8> {
        let mut p = vec![0u8; PAGE];
        p[0..8].copy_from_slice(b"KRNOBTRE");
        p[8..12].copy_from_slice(&1u32.to_le_bytes());
        p[12..16].copy_from_slice(&(PAGE as u32).to_le_bytes());
        p[16..24].copy_from_slice(&seq.to_le_bytes());
        p[24..32].copy_from_slice(&root.0.to_le_bytes());
        p[32..36].copy_from_slice(&root.1.to_le_bytes());
        p[36..40].copy_from_slice(&height.to_le_bytes());
        p[40..48].copy_from_slice(&page_count.to_le_bytes());
        // free head 0/0, reserved 0, free_log_pages 0, free_count 0
        p[80..88].copy_from_slice(&entries.to_le_bytes());
        let crc = crc32c::crc32c(&p[..88]);
        p[88..92].copy_from_slice(&crc.to_le_bytes());
        p
    }

    #[test]
    fn crafted_crc_valid_file_makes_commit_fail_cleanly() {
        let sim = SimVfs::new(7);
        let vfs = sim.as_vfs();
        let dir = Path::new("/db");
        vfs.create_dir_all(dir).unwrap();

        // Leaf A (page 2) = {a, z}: 'z' >= separator 'm', but each leaf is
        // internally sorted, so Node::decode accepts it.
        let a = leaf(2, &[(b"a", b"va"), (b"z", b"vz")]);
        // Leaf B (page 3) = {n}
        let b = leaf(3, &[(b"n", b"vn")]);
        // Leaf C (page 4): four maximal inline entries -> a full page, so A+B do
        // not merge further with it and the root stays a branch.
        let big = vec![7u8; 1011];
        let c = leaf(4, &[(b"s0", &big), (b"s1", &big), (b"s2", &big), (b"s3", &big)]);
        let (ca, cb, cc) = (crc32c::crc32c(&a), crc32c::crc32c(&b), crc32c::crc32c(&c));
        let root = branch(5, 1, (2, ca), &[(b"m", (3, cb)), (b"s", (4, cc))]);
        let cr = crc32c::crc32c(&root);
        let m = meta(1, (5, cr), 2, 6, 7);

        let f = vfs.open(&dir.join("data.kbt"), OpenMode::CreateNew).unwrap();
        let mut img = Vec::new();
        for p in [&m, &m, &a, &b, &c, &root] {
            img.extend_from_slice(p);
        }
        f.write_at(&img, 0).unwrap();
        f.sync_data().unwrap();
        drop(f);
        vfs.sync_dir(dir).unwrap();

        let engine = BTreeEngine::open(vfs.clone(), dir, &Options::default()).expect("crafted file opens");
        println!("opened crafted file OK");

        // Read side: key 'z' is stored but the lookup routes to C and misses it.
        let snap = engine.snapshot().unwrap();
        let gz = snap.get(b"z");
        println!("get(z) = {gz:?}");
        let ga = snap.get(b"a");
        println!("get(a) = {:?}", ga.as_ref().map(|o| o.as_ref().map(|v| String::from_utf8_lossy(v).to_string())));
        let scan = snap.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap();
        let keys: Vec<String> = scan.iter().map(|(k, _)| String::from_utf8_lossy(k).to_string()).collect();
        println!("scan keys = {keys:?}");
        drop(snap);
        let silent_miss = matches!(gz, Ok(None));

        // Write side: put('b') goes into A, A is underfull, merges with B into
        // {a, b, z, n}; encode succeeds, then the re-decode `.expect` panics.
        let mut batch = WriteBatch::new();
        batch.put("b", "vb");
        let r = catch_unwind(AssertUnwindSafe(|| engine.commit(&batch)));
        let panicked = match &r {
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                println!("commit PANICKED: {msg}");
                true
            }
            Ok(res) => {
                println!("commit returned {res:?}");
                false
            }
        };

        // After the panic, the handle is not poisoned (parking_lot mutex,
        // poison flag never set): a further commit is accepted or panics again.
        if panicked {
            let mut b2 = WriteBatch::new();
            b2.put("s5", "x");
            let r2 = catch_unwind(AssertUnwindSafe(|| engine.commit(&b2)));
            match &r2 {
                Ok(res) => println!("second commit after panic returned {res:?}"),
                Err(_) => println!("second commit after panic panicked too"),
            }
        }

        // Reads of a crafted file whose crcs are valid may miss keys (crc32c is
        // not an authentication code; see the threat model, T-04), but writes
        // must fail cleanly instead of panicking.
        let _ = silent_miss;
        assert!(
            !panicked,
            "commit panicked on a crc-valid but structurally inconsistent file instead of returning Error::Corruption"
        );
        assert!(matches!(r, Ok(Err(spike_core::Error::Corruption(_)))), "expected Corruption, got {r:?}");
    }
}

// ---------------------------------------------------------------------------
// A commit sequence at u64::MAX wrapped around (B-F).

mod exhausted_seq {
    use super::*;

    const PAGE: usize = 4096;
    const CRC_AT: usize = 88;

    fn craft_seq(vfs: &SimVfs, dir: &Path, slot: usize, seq: u64) {
        let f = vfs.as_vfs().open(&dir.join("data.kbt"), OpenMode::Existing).unwrap();
        let mut p = vec![0u8; PAGE];
        f.read_at(&mut p, (slot * PAGE) as u64).unwrap();
        p[16..24].copy_from_slice(&seq.to_le_bytes());
        let crc = crc32c::crc32c(&p[..CRC_AT]);
        p[CRC_AT..CRC_AT + 4].copy_from_slice(&crc.to_le_bytes());
        f.write_at(&p, (slot * PAGE) as u64).unwrap();
        f.sync_data().unwrap();
    }

    #[test]
    fn exhausted_commit_sequence_is_an_error_not_a_wrap() {
        let sim = SimVfs::new(7);
        let vfs = sim.as_vfs();
        let dir = Path::new("/db");
        let opts = Options::small_for_tests(); // Durability::Sync
        {
            let e = BTreeEngine::open(vfs.clone(), dir, &opts).unwrap();
            let mut b = WriteBatch::new();
            b.put(b"a".to_vec(), b"1".to_vec());
            assert_eq!(e.commit(&b).unwrap(), 1);
            e.close().unwrap();
        }
        // Only slot 0 claims seq u64::MAX; slot 1 still holds seq 1.
        craft_seq(&sim, dir, 0, u64::MAX);

        let e = BTreeEngine::open(vfs.clone(), dir, &opts).unwrap();
        let s0 = e.snapshot().unwrap();
        println!("after open: seq = {}", s0.seq());
        drop(s0);
        let mut b = WriteBatch::new();
        b.put(b"k".to_vec(), b"v".to_vec());
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.commit(&b)));
        let seq = r.expect("commit must not panic on an exhausted sequence");
        assert!(matches!(seq, Err(spike_core::Error::Corruption(_))), "expected Corruption, got {seq:?}");
    }
}

// ---------------------------------------------------------------------------
// The database directory was never synced into its parent.

struct Recorder {
    inner: Arc<dyn Vfs>,
    dir_syncs: Mutex<Vec<PathBuf>>,
}

impl Vfs for Recorder {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.inner.create_dir_all(path)
    }
    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        self.inner.open(path, mode)
    }
    fn exists(&self, path: &Path) -> io::Result<bool> {
        self.inner.exists(path)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.inner.remove_file(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.inner.rename(from, to)
    }
    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        self.inner.list(dir)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.dir_syncs.lock().push(dir.to_path_buf());
        self.inner.sync_dir(dir)
    }
    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
        self.inner.lock(path)
    }
}

#[test]
fn open_makes_the_database_directory_durable_in_its_parent() {
    let rec = Arc::new(Recorder { inner: SimVfs::new(1).as_vfs(), dir_syncs: Mutex::new(Vec::new()) });
    let db = BTreeEngine::open(rec.clone(), Path::new("/data/apps/db"), &Options::small_for_tests()).unwrap();
    db.close().unwrap();
    let syncs = rec.dir_syncs.lock().clone();
    assert!(syncs.contains(&PathBuf::from("/data/apps")), "parent directory never synced: {syncs:?}");
    assert!(syncs.contains(&PathBuf::from("/data/apps/db")), "database directory never synced: {syncs:?}");
}
