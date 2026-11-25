//! A deferred recovery replays several live logs into one memtable (or leaves
//! a trailing memtable that holds no keys): once the flush of that memtable is
//! in the MANIFEST, none of those logs may stay on disk (W-5), not only the
//! first one.

mod common;

use kerno_core::{Durability, WriteBatch};
use kerno_testkit::vfs::DirImage;

use common::*;

/// The crash image of `unclean_image` with its live log replaced by
/// `logs.len()` logs, `logs[i]` holding the commits of `logs[i]` in order
/// (numbers `first + 10 * i`, sequences chained from the first one).
fn image_with_logs(logs: &[Vec<WriteBatch>]) -> DirImage {
    let (mut image, _) = unclean_image();
    let live = files_with(&image, "wal").pop().expect("a live WAL");
    let first = file_number(&live);
    let mut base = le_u64(&image.files[&live], 64);
    image.files.remove(&live);
    for (i, commits) in logs.iter().enumerate() {
        let number = first + 10 * i as u64;
        image.files.insert(path(&format!("{number:06}.wal")), wal_bytes(number, base, commits));
        base += commits.len() as u64;
    }
    image
}

fn puts(prefix: &str, n: u32, size: usize) -> Vec<WriteBatch> {
    (0..n)
        .map(|i| {
            let mut b = WriteBatch::new();
            b.put(format!("{prefix}-{i:05}"), vec![i as u8; size]);
            b
        })
        .collect()
}

/// Opens `image` deferred, flushes on demand and returns the logs left: the
/// replayed data must read back before and after, and after a reopen.
fn logs_left_after_a_deferred_flush(image: &DirImage, threads: bool) -> usize {
    kerno_storage::KernoDb::set_defer_recovery_flush(false);
    let (_, eager) = open_copy(image, &config(Durability::Sync));
    let expected = model_of(&eager.unwrap());

    kerno_storage::KernoDb::set_defer_recovery_flush(true);
    let cfg = if threads { threads_config(Durability::Sync) } else { config(Durability::Sync) };
    let (disk, db) = open_copy(image, &cfg);
    let db = db.unwrap();
    assert!(model_of(&db) == expected, "the replayed commits are readable");
    db.hooks().flush().unwrap();
    assert!(model_of(&db) == expected);
    let flushed = disk.dump();
    let left = files_with(&flushed, "wal").len();
    drop(db);
    kerno_storage::KernoDb::set_defer_recovery_flush(false);
    let (_, db) = open_copy(&flushed, &config(Durability::Sync));
    assert!(model_of(&db.unwrap()) == expected, "a reopen after the flush");
    left
}

#[test]
fn a_flush_retires_every_log_a_deferred_recovery_replayed_into_one_memtable() {
    // Three logs of about 3 KiB each: one 16 KiB memtable holds them all.
    let image = image_with_logs(&[puts("a", 10, 300), puts("b", 10, 300), puts("c", 10, 300)]);
    assert_eq!(files_with(&image, "wal").len(), 3);
    assert_eq!(logs_left_after_a_deferred_flush(&image, false), 1, "only the live log is left");
}

#[test]
fn a_flush_retires_the_trailing_log_that_holds_only_empty_commits() {
    // The first log fills the memtable, the second holds no keys.
    let image = image_with_logs(&[puts("a", 60, 300), vec![WriteBatch::new(); 3]]);
    assert_eq!(files_with(&image, "wal").len(), 2);
    assert_eq!(logs_left_after_a_deferred_flush(&image, false), 1, "only the live log is left");
}

#[test]
fn a_flush_with_threads_retires_every_log_of_a_deferred_memtable() {
    let image = image_with_logs(&[puts("a", 10, 300), puts("b", 10, 300), puts("c", 10, 300)]);
    assert_eq!(logs_left_after_a_deferred_flush(&image, true), 1, "only the live log is left");
}
