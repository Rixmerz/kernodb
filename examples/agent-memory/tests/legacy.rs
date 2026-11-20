//! A directory written by the previous version of this example still opens.
//!
//! The previous version kept `mem <n> <vec|->` lines in the catalog unit. The
//! directory is built here the way that version built it, with the public API
//! of `kerno-units` only: catalog unit number 1, a memory unit with a persisted
//! vector, and a catalog payload that has the legacy lines.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use agent_memory::{AgentMemory, Embedder, VectorStatus};
use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_units::{
    ChangeMeta, CounterIds, DerivedDb, FixedClock, NewUnit, RevisionId, Timestamp, UnitConfig, UnitDb, VectorSpec,
};

const T0: i64 = 1_700_000_000_000_000;

#[test]
fn a_directory_written_by_the_previous_version_opens() {
    let dir: PathBuf =
        [env!("CARGO_TARGET_TMPDIR"), &format!("agent-memory-legacy-{}", std::process::id())].iter().collect();
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let embedder = Embedder::default();
    let text = "the legacy memory about the staging token";

    // What the previous version wrote.
    {
        let clock = Arc::new(FixedClock::stepping(Timestamp(T0), 1_000_000));
        let cfg = UnitConfig::with_sources(clock, Arc::new(CounterIds::new(1)));
        let db = UnitDb::<KernoDb>::open(StdVfs::shared(), &dir, &cfg).unwrap();
        let (db, _) = DerivedDb::open_persisted(db).unwrap();
        let meta = ChangeMeta::default();
        let mut cs = db.begin_changeset(meta).unwrap();
        let catalog = cs
            .create(NewUnit {
                unit_type: "catalog".into(),
                schema_version: 1,
                metadata: vec![],
                payload: b"next 2\n".to_vec(),
            })
            .unwrap();
        cs.commit().unwrap();

        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let unit = cs
            .create(NewUnit {
                unit_type: "memory".into(),
                schema_version: 1,
                metadata: b"t".to_vec(),
                payload: text.as_bytes().to_vec(),
            })
            .unwrap();
        let vector = cs
            .put_vector(&VectorSpec {
                source: RevisionId::first(unit),
                method: "standin-embed".into(),
                method_version: 1,
                profile: "standin-v1".into(),
                params: embedder.params(),
                vector: embedder.embed(text.as_bytes()),
            })
            .unwrap();
        let hex: String = vector.0.iter().map(|b| format!("{b:02x}")).collect();
        let payload = format!("next 3\nmem 2 {hex}\nmem 9 -\n").into_bytes();
        cs.update(catalog, kerno_units::UnitUpdate { payload, schema_version: None, metadata: None }, None).unwrap();
        cs.commit().unwrap();
        db.close().unwrap();
    }

    let clock = Arc::new(FixedClock::stepping(Timestamp(T0 + 100_000_000), 1_000_000));
    let (mem, report) = AgentMemory::open(&dir, clock, embedder).expect("a legacy directory opens");
    assert_eq!(report.vectors_served, 1);
    assert_eq!(mem.get(2).unwrap().text, text);
    assert_eq!(mem.vector_status(2).unwrap(), VectorStatus::Ready);
    // A line of the legacy catalog for a memory that no longer exists is dropped, not listed.
    assert_eq!(mem.all().unwrap().iter().map(|m| m.id).collect::<Vec<_>>(), [2]);
    let hits = mem.recall_similar(&embedder.embed(b"staging token"), 3).unwrap();
    assert_eq!(hits.iter().map(|h| h.memory.id).collect::<Vec<_>>(), [2]);

    // The counter came from the legacy catalog.
    let b = mem.remember("a new one", &[], None).unwrap();
    assert_eq!(b, 3);
    mem.close().unwrap();
    let clock = Arc::new(FixedClock::stepping(Timestamp(T0 + 200_000_000), 1_000_000));
    let (mem, _) = AgentMemory::open(&dir, clock, embedder).unwrap();
    assert_eq!(mem.all().unwrap().len(), 2);
    mem.close().unwrap();
}
