//! `Config::lazy_tables`: a read-only handle answers the same as an eager one, and a table
//! with damaged meta blocks is found by the read that needs it.

use std::ops::Bound;
use std::path::Path;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_core::{Error, WriteBatch};
use kerno_storage::{AccessMode, Config, KernoDb};

/// A fresh directory under the system's temporary one, removed when dropped.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("kerno-lazy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        Dir(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fill(dir: &Path) {
    let cfg = Config { allow_unverified_platform: true, write_buffer_bytes: 16 << 10, ..Config::default() };
    let db = KernoDb::open(StdVfs::shared(), dir, &cfg).unwrap();
    for batch in 0..40u32 {
        let mut wb = WriteBatch::new();
        for i in 0..200u32 {
            let k = format!("key-{:06}", batch * 200 + i);
            wb.put(k, format!("value-{batch}-{i}").repeat(8));
        }
        db.commit(&wb).unwrap();
    }
    db.hooks().flush().unwrap();
    db.close().unwrap();
}

fn read_only(dir: &Path, lazy: bool) -> std::result::Result<KernoDb, Error> {
    let cfg = Config {
        access: AccessMode::ReadOnly,
        create_if_missing: false,
        allow_unverified_platform: true,
        lazy_tables: lazy,
        ..Config::default()
    };
    KernoDb::open(StdVfs::shared(), dir, &cfg)
}

fn tables(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "sst"))
        .collect();
    v.sort();
    v
}

#[test]
fn a_lazy_handle_reads_what_an_eager_one_reads() {
    let dir = Dir::new("same");
    fill(dir.path());
    assert!(tables(dir.path()).len() >= 2, "the fixture must span several tables");
    let (eager, lazy) = (read_only(dir.path(), false).unwrap(), read_only(dir.path(), true).unwrap());
    let (a, b) = (eager.snapshot().unwrap(), lazy.snapshot().unwrap());
    assert_eq!(a.seq(), b.seq());
    for k in ["key-000000", "key-004321", "key-007999", "key-999999", "a", "z"] {
        assert_eq!(a.get(k.as_bytes()).unwrap(), b.get(k.as_bytes()).unwrap(), "{k}");
    }
    let all = |s: &<KernoDb as Engine>::Snapshot| s.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap();
    let rows = all(&a);
    assert_eq!(rows.len(), 8000);
    assert_eq!(rows, all(&b));
}

#[test]
fn a_damaged_table_fails_the_eager_open_and_the_lazy_read() {
    let dir = Dir::new("damaged");
    fill(dir.path());
    // Damage the tail of one table: its footer and meta blocks.
    let victim = tables(dir.path()).pop().unwrap();
    let mut bytes = std::fs::read(&victim).unwrap();
    let n = bytes.len();
    for b in &mut bytes[n - 64..] {
        *b ^= 0x5a;
    }
    std::fs::write(&victim, bytes).unwrap();

    assert!(matches!(read_only(dir.path(), false), Err(Error::Corruption(_))), "eager: refused at open");
    let lazy = read_only(dir.path(), true).expect("lazy: the open reads no table");
    let snap = lazy.snapshot().unwrap();
    let scan = snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX);
    assert!(matches!(scan, Err(Error::Corruption(_))), "lazy: the read that needs the table reports it");
}
