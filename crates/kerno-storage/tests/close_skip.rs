//! The close-time flush skips the LZ4 pass and the intermediate syncs of its
//! output (perf-gaps-2026-11.md, Rank 5). A table written that way must
//! reopen with every key, after a clean close, after a power loss right after
//! it, and after a crash at any step of the close.

mod common;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kerno_core::Durability;
use kerno_core::vfs::{OpenMode, Vfs};
use kerno_storage::Config;
use kerno_storage::format::features::BLOCK_LZ4;
use kerno_testkit::harness::Model;
use kerno_testkit::vfs::{CrashKind, HookAction, HookOpKind, HookPoint, HookVfs, SimVfs};

use common::*;

/// A write buffer large enough that the close flushes many data blocks, and
/// intermediate syncs every 4 KiB for the writers that keep them.
fn big_memtable() -> Config {
    let mut cfg = Config { write_buffer_bytes: 256 << 10, ..config(Durability::Sync) };
    cfg.compaction.bytes_per_sync = Some(4 << 10);
    cfg
}

/// 600 committed values of 200 bytes, all still in the active memtable.
fn filled(vfs: &Arc<dyn Vfs>) -> (kerno_storage::KernoDb, Model) {
    let db = open_in(vfs, &big_memtable()).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 600);
    assert_eq!(stat(&db, "flushes"), 0, "everything waits for the close-time flush");
    (db, model)
}

#[test]
fn a_close_without_the_lz4_pass_and_syncs_survives_a_power_loss_and_reads_every_key() {
    let disk = SimVfs::new(11);
    let (db, model) = filled(&disk.as_vfs());
    db.close().unwrap();

    let closed = disk.crash_now(CrashKind::PowerLoss).dump();
    let ssts = files_with(&closed, "sst");
    assert!(!ssts.is_empty());
    for t in &ssts {
        assert_ne!(le_u64(&closed.files[t], 32) & BLOCK_LZ4, 0, "{t:?} keeps the bit");
    }
    let (_, db) = open_copy(&closed, &big_memtable());
    let db = db.unwrap();
    assert_eq!(stat(&db, "last_open_clean"), 1, "the close was durable");
    assert_eq!(stat(&db, "last_open_replayed_bytes"), 0);
    assert!(model_of(&db) == model);
    db.close().unwrap();
}

/// A failed operation at every mutating step of the close, then the files as
/// they are or as a power loss leaves them: the next open has every key.
#[test]
fn a_crash_at_every_step_of_the_close_time_flush_loses_nothing() {
    let run = |target: Option<u64>| -> (u64, SimVfs, Model, bool) {
        let disk = SimVfs::new(12);
        let armed = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicU64::new(0));
        let (on, counter) = (armed.clone(), seen.clone());
        let vfs: Arc<dyn Vfs> = HookVfs::wrap(disk.as_vfs(), move |op| {
            let mutating = matches!(
                op.kind,
                HookOpKind::Remove
                    | HookOpKind::Rename
                    | HookOpKind::SyncDir
                    | HookOpKind::WriteAt
                    | HookOpKind::SetLen
                    | HookOpKind::SyncData
            ) || (op.kind == HookOpKind::Open && op.mode == Some(OpenMode::CreateNew));
            if on.load(Ordering::SeqCst) && op.point == HookPoint::Before && mutating {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if Some(n) == target {
                    return HookAction::Fail(io::ErrorKind::Other);
                }
            }
            HookAction::Proceed
        });
        let (db, model) = filled(&vfs);
        armed.store(true, Ordering::SeqCst);
        let failed = db.close().is_err();
        (seen.load(Ordering::SeqCst), disk, model, failed)
    };
    let (total, _, _, failed) = run(None);
    assert!(!failed && total > 0);
    // A close tolerates some failures (the removal of obsolete files), so not every op fails it.
    let mut close_failed = 0;
    for k in 0..total {
        let (_, disk, model, failed) = run(Some(k));
        close_failed += u64::from(failed);
        for (kind, image) in [("kill", disk.dump()), ("power loss", disk.crash_now(CrashKind::PowerLoss).dump())] {
            let (_, db) = open_copy(&image, &big_memtable());
            let db = db.unwrap_or_else(|e| panic!("op {k} ({kind}): {e}"));
            assert!(model_of(&db) == model, "op {k} ({kind}): the recovered state differs");
        }
    }
    assert!(close_failed > total / 2, "only {close_failed} of {total} ops failed the close");
}
