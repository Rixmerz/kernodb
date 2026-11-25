//! A recovery open that defers its flush must keep the sequences of the
//! replayed commits that write no keys: when the newest replayed log holds
//! only such commits, the deferred open used to drop them, the close wrote a
//! `last_seq` below the new log's `base_seq`, and the next open refused the
//! database (WAL-R1).

mod common;

use kerno_core::{Durability, WriteBatch};
use kerno_storage::Config;
use kerno_testkit::harness::Model;
use kerno_testkit::vfs::DirImage;

use common::*;

const PUTS: u32 = 60;
const EMPTY_COMMITS: usize = 3;

/// Two live logs after a crash: the first one fills a 16 KiB memtable (so the
/// replay starts a second memtable for the next log), the second one holds
/// only empty commits. Returns the image and the sequence of its last commit.
fn image_with_trailing_empty_commits() -> (DirImage, u64) {
    let (mut image, _) = unclean_image();
    let a = files_with(&image, "wal").pop().expect("a live WAL");
    let a_number = file_number(&a);
    let base = le_u64(&image.files[&a], 64);

    let puts: Vec<WriteBatch> = (0..PUTS)
        .map(|i| {
            let mut b = WriteBatch::new();
            b.put(format!("late-{i:05}"), [i as u8; 300]);
            b
        })
        .collect();
    image.files.insert(a.clone(), wal_bytes(a_number, base, &puts));

    let b_number = a_number + 50;
    let b_base = base + u64::from(PUTS);
    let empties = vec![WriteBatch::new(); EMPTY_COMMITS];
    image.files.insert(path(&format!("{b_number:06}.wal")), wal_bytes(b_number, b_base, &empties));
    (image, b_base + EMPTY_COMMITS as u64 - 1)
}

/// What an eager (not deferred) recovery of `image` reads: the reference.
fn eager_model(image: &DirImage) -> Model {
    kerno_storage::KernoDb::set_defer_recovery_flush(false);
    let (_, db) = open_copy(image, &config(Durability::Sync));
    let db = db.expect("an eager recovery opens the image");
    let model = model_of(&db);
    db.close().unwrap();
    model
}

/// Open deferred, check the replay, close, reopen and check again.
fn open_close_reopen(image: &DirImage, cfg: &Config, last_seq: u64, expected: &Model) {
    let (disk, db) = open_copy(image, cfg);
    let db = db.expect("the deferred recovery open");
    assert!(stat(&db, "last_open_replayed_bytes") > 0);
    assert_eq!(stat(&db, "flushes"), 0, "the recovery flush was deferred");
    assert!(&model_of(&db) == expected, "the replayed commits are readable");
    // No commit between the open and the close: only the replay holds the
    // sequences of the empty commits.
    db.close().expect("close after a deferred recovery whose newest log holds only empty commits");
    let closed = disk.dump();
    assert_eq!(manifest_of(&closed).0.state.last_seq, last_seq, "the MANIFEST holds the last sequence");
    reopens_at(&closed, last_seq, expected);
}

/// Opens `image` (eager) and checks the data and that the next commit follows `last_seq`.
fn reopens_at(image: &DirImage, last_seq: u64, expected: &Model) {
    let (_, db) = open_copy(image, &config(Durability::Sync));
    let db = db.expect("the reopen (WAL-R1)");
    assert!(&model_of(&db) == expected);
    let mut model = expected.clone();
    assert_eq!(put(&db, &mut model, "probe", b"y"), last_seq + 1);
}

/// The flush of the deferred memtable, then a crash: the MANIFEST the flush
/// wrote must still chain to the log that follows it.
fn open_flush_crash_reopen(image: &DirImage, cfg: &Config, last_seq: u64, expected: &Model) {
    let (disk, db) = open_copy(image, cfg);
    let db = db.expect("the deferred recovery open");
    db.hooks().flush().unwrap();
    drop(db);
    reopens_at(&disk.dump(), last_seq, expected);
}

#[test]
fn deferred_recovery_keeps_trailing_empty_commits_inline() {
    let (image, last_seq) = image_with_trailing_empty_commits();
    let expected = eager_model(&image);
    kerno_storage::KernoDb::set_defer_recovery_flush(true);
    let cfg = Config { close_budget: kerno_storage::CloseBudget::Unbounded, ..config(Durability::Sync) };
    open_close_reopen(&image, &cfg, last_seq, &expected);
    kerno_storage::KernoDb::set_defer_recovery_flush(false);
}

#[test]
fn a_flush_after_a_deferred_recovery_keeps_trailing_empty_commits_inline() {
    let (image, last_seq) = image_with_trailing_empty_commits();
    let expected = eager_model(&image);
    kerno_storage::KernoDb::set_defer_recovery_flush(true);
    open_flush_crash_reopen(&image, &config(Durability::Sync), last_seq, &expected);
    kerno_storage::KernoDb::set_defer_recovery_flush(false);
}

#[test]
fn a_flush_after_a_deferred_recovery_keeps_trailing_empty_commits_with_threads() {
    let (image, last_seq) = image_with_trailing_empty_commits();
    let expected = eager_model(&image);
    open_flush_crash_reopen(&image, &threads_config(Durability::Sync), last_seq, &expected);
}

#[test]
fn deferred_recovery_keeps_trailing_empty_commits_with_threads() {
    let (image, last_seq) = image_with_trailing_empty_commits();
    let expected = eager_model(&image);
    open_close_reopen(&image, &threads_config(Durability::Sync), last_seq, &expected);
}
