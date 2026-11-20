//! Phase C: crash recovery of schemas and user indexes (ADR-0021).
//!
//! A crash before every mutating I/O operation of a scenario that defines a
//! schema, writes units under it, adds an index to existing units, archives,
//! unarchives, deletes and drops an index. After each recovery the store must
//! be byte for byte the one after a whole prefix of change sets (I-21), so the
//! schema, the unit and its index entries are all there or all absent;
//! `verify_units` must find nothing; every index must answer what a scan of the
//! payloads says; and the recovered database must still refuse a duplicate of a
//! unique value and accept a new unit.

#![cfg(feature = "test-support")]

mod harness;

use harness::units::*;
use kerno_core::Durability;
use kerno_core::contract::TxnEngine;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::harness::{for_each_seed, seeds};
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{CrashKind, SimVfs};
use kerno_units::*;

const ACCT: &str = "acct";
const STEPS: u64 = 7;

fn doc(name: &str, age: i64, email: &str) -> Vec<u8> {
    Document::new()
        .with("name", FieldValue::Text(name.into()))
        .with("age", FieldValue::Int(age))
        .with("email", FieldValue::Text(email.into()))
        .encode()
        .expect("encode")
}

/// The `n`-th (from 1) id minted in change set `step` ([`StepIds`]).
fn at(step: u64, n: u64) -> UnitId {
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&((step << 16) + n).to_be_bytes());
    UnitId(id)
}

fn new_unit(payload: Vec<u8>) -> NewUnit {
    NewUnit { unit_type: ACCT.into(), schema_version: 1, metadata: Vec::new(), payload }
}

fn update(payload: Vec<u8>) -> UnitUpdate {
    UnitUpdate { payload, schema_version: None, metadata: None }
}

/// Change set `step` of the scenario, one commit each.
fn run_step<E: TxnEngine>(db: &UnitDb<E>, ids: &StepIds, step: u64) -> Result<(), UnitError> {
    ids.start_step(step);
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    match step {
        1 => {
            cs.define_schema(
                ACCT,
                vec![
                    FieldDef::new("name", FieldType::Text).required(),
                    FieldDef::new("age", FieldType::Int).indexed(),
                    FieldDef::new("email", FieldType::Text).unique(),
                ],
            )?;
            for (n, e) in [(1, "a"), (2, "b"), (3, "c"), (4, "d")] {
                cs.create(new_unit(doc(e, n, &format!("{e}@x"))))?;
            }
        }
        2 => {
            cs.update(at(1, 1), update(doc("a", 9, "a@x")), None)?;
            cs.create(new_unit(doc("e", 1, "e@x")))?;
        }
        3 => cs.create_index(ACCT, "name")?,
        4 => {
            cs.archive(at(1, 2), None)?;
            cs.update(at(1, 3), update(doc("c", 3, "c2@x")), None)?;
        }
        5 => {
            // The deleted unit's unique value is free again in the same change set.
            cs.delete(at(1, 4), None)?;
            cs.create(new_unit(doc("d2", 4, "d@x")))?;
        }
        6 => {
            cs.unarchive(at(1, 2), None)?;
        }
        _ => {
            cs.drop_index(ACCT, "name")?;
            cs.update(at(2, 1), update(doc("e", 2, "e@x")), None)?;
        }
    }
    cs.commit().map(|_| ())
}

/// Change sets acknowledged and attempted by a run on `disk` (the failed one
/// may or may not have committed).
fn run_scenario<E: Target>(disk: &SimVfs, cfg: &UnitConfig, ids: &StepIds) -> (u64, u64) {
    let Ok(db) = open_units::<E>(disk.as_vfs(), cfg) else { return (0, 0) };
    for step in 1..=STEPS {
        if run_step(&db, ids, step).is_err() {
            return (step - 1, step);
        }
    }
    let _ = db.close();
    (STEPS, STEPS)
}

/// The raw store after each whole change set, from an undisturbed
/// `Emulated<LogEngine>`.
fn oracle() -> Vec<Dump> {
    let disk = SimVfs::new(0);
    let ids = StepIds::new();
    let db = open_units::<Emulated<LogEngine>>(disk.as_vfs(), &unit_config(&ids, Durability::Sync)).expect("open");
    let mut dumps = vec![raw(&db)];
    for step in 1..=STEPS {
        run_step(&db, &ids, step).expect("the oracle step");
        dumps.push(raw(&db));
    }
    dumps
}

/// Every indexed field answers what a scan of the payloads says.
fn indexes_equal_the_payloads<E: TxnEngine>(db: &UnitDb<E>, ctx: &str) {
    let snap = db.snapshot().unwrap_or_else(|e| panic!("{ctx}: snapshot: {e}"));
    let Some(schema) = snap.schema(ACCT).unwrap_or_else(|e| panic!("{ctx}: schema: {e}")) else { return };
    let units = snap
        .reference_filter(&UnitFilter { unit_type: Some(ACCT.into()), schema_version: None })
        .unwrap_or_else(|e| panic!("{ctx}: reference: {e}"));
    let docs: Vec<(UnitId, Document)> = units
        .iter()
        .map(|u| (u.id, Document::decode(u.payload.as_deref().expect("payload")).expect("document")))
        .collect();
    for field in schema.fields().iter().filter(|f| f.indexed) {
        // Every value present, and one absent.
        let mut values: Vec<FieldValue> = docs.iter().filter_map(|(_, d)| d.get(&field.name).cloned()).collect();
        values.push(match field.ty {
            FieldType::Int => FieldValue::Int(1_000),
            _ => FieldValue::Text("absent".into()),
        });
        for v in values {
            let want: Vec<UnitId> =
                docs.iter().filter(|(_, d)| d.get(&field.name) == Some(&v)).map(|(id, _)| *id).collect();
            let got: Vec<UnitId> = snap
                .find_by_field(ACCT, &field.name, &v)
                .unwrap_or_else(|e| panic!("{ctx}: find_by_field {}: {e}", field.name))
                .iter()
                .map(|u| u.id)
                .collect();
            assert_eq!(got, want, "{ctx}: the index of {:?} differs from a scan", field.name);
        }
    }
}

/// Recovered state: a whole prefix, clean, indexes equal the payloads.
fn check<E: TxnEngine>(db: &UnitDb<E>, oracle: &[Dump], lo: u64, hi: u64, ctx: &str) -> u64 {
    let s = seq(db);
    assert!(lo <= s && s <= hi, "{ctx}: recovered {s} change sets, expected {lo}..={hi}");
    let got = raw(db);
    assert!(
        got == oracle[s as usize],
        "{ctx}: the store is not the one after {s} whole change sets: {}",
        first_difference(&oracle[s as usize], &got)
    );
    verified(db, ctx);
    indexes_equal_the_payloads(db, ctx);
    s
}

fn schema_every_point<E: Target>(kind: CrashKind) {
    let name = format!("schema_every_point {} {kind:?}", E::NAME);
    let oracle = oracle();
    for_each_seed(&name, seeds(2), |seed| {
        let ids = StepIds::new();
        let cfg = unit_config(&ids, Durability::Sync);
        let dry = SimVfs::new(0);
        assert_eq!(run_scenario::<E>(&dry, &cfg, &ids).0, STEPS, "the dry run must complete");
        let total = dry.mutating_ops() + 5;
        eprintln!("{name} seed {seed}: {total} points");
        for point in 1..=total {
            let ctx = format!("{name} seed {seed}: crash before mutating op #{point} of {total}");
            let disk = SimVfs::new(seed.wrapping_mul(1_000_003) ^ point);
            if point % 2 == 1 {
                disk.set_tear_unit(1);
            }
            disk.crash_after(point, kind);
            let (acked, attempted) = run_scenario::<E>(&disk, &cfg, &ids);
            let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
            let db = open_units::<E>(image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
            let s = check(&db, &oracle, acked, attempted, &ctx);
            if s == 0 {
                continue;
            }
            // The recovered database keeps its rules: a duplicate of a unique
            // value is refused, a new unit is accepted and indexed.
            ids.start_step(100);
            let dup = {
                let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
                cs.create(new_unit(doc("dup", 5, "e@x")))
            };
            let taken = db
                .snapshot()
                .expect("snapshot")
                .find_by_field(ACCT, "email", &FieldValue::Text("e@x".into()))
                .expect("find")
                .len();
            assert_eq!(dup.is_err(), taken == 1, "{ctx}: a unique value is refused exactly when it is held");
            ids.start_step(101);
            let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
            cs.create(new_unit(doc("fresh", 7, "fresh@x"))).unwrap_or_else(|e| panic!("{ctx}: a new unit: {e}"));
            cs.commit().unwrap_or_else(|e| panic!("{ctx}: commit: {e}"));
            verified(&db, &format!("{ctx}, after one more unit"));
            indexes_equal_the_payloads(&db, &format!("{ctx}, after one more unit"));
        }
    });
}

macro_rules! sweeps {
    ($m:ident, $e:ty) => {
        mod $m {
            type E = $e;

            #[test]
            fn schema_every_point_power_loss() {
                super::schema_every_point::<E>(super::CrashKind::PowerLoss);
            }
            #[test]
            fn schema_every_point_process_kill() {
                super::schema_every_point::<E>(super::CrashKind::ProcessKill);
            }
        }
    };
}

sweeps!(emulated_log, kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>);
sweeps!(kerno, kerno_storage::KernoDb);
