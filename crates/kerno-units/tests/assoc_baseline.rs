//! E7-00: the baselines B0 and B1 over the frozen associative evaluation set
//! (acta §6.1), measured on a real `UnitDb` over `KernoDb` in the in-memory
//! file system.
//!
//! * B0: the existing query by unit type, in its existing order, cut at 10.
//!   The type is the seed's type.
//! * B1: the one-hop neighbours of the seed through `relations_of`, outgoing
//!   then incoming, each in index order (relation type, then relation id),
//!   without duplicates, cut at 10. No constraint of the query applies.
//!
//! The test loads the corpus, checks that the database holds exactly the
//! relations of the corpus, then prints the report. The report of the
//! committed run is `docs/architecture/etapa7-e700-salida.txt`:
//!
//! ```text
//! cargo test -p kerno-units --release --test assoc_baseline -- --nocapture --exact baselines_over_the_frozen_set
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, CUT, Class, Corpus, Provenance as StoryProvenance, QuerySet};
use kerno_testkit::assoc_metrics::{ndcg_at, recall_at};
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

const CORPUS: &str = include_str!("../../kerno-testkit/data/assoc-eval/corpus.txt");
const QUERIES: &str = include_str!("../../kerno-testkit/data/assoc-eval/queries.txt");

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const CLASSES: [(Class, &str); 4] = [
    (Class::MultiHop, "multi_hop"),
    (Class::Temporal, "temporal"),
    (Class::Provenance, "provenance"),
    (Class::Combined, "combined"),
];

fn provenance(p: &StoryProvenance) -> RelationProvenance {
    match p {
        StoryProvenance::Declared => RelationProvenance::Declared,
        StoryProvenance::Imported(source) => RelationProvenance::Imported { source: source.clone().into_bytes() },
        StoryProvenance::Algorithmic(origin) => RelationProvenance::Algorithmic { origin: origin.clone().into_bytes() },
        StoryProvenance::Inferred { origin, confidence_permille } => RelationProvenance::Inferred {
            origin: origin.clone().into_bytes(),
            confidence_permille: *confidence_permille,
        },
    }
}

/// Loads the corpus into a fresh database: one change set per step, with the
/// step's actor and source. Returns the database and the unit id of each index.
fn load(corpus: &Corpus) -> (UnitDb<KernoDb>, Vec<UnitId>) {
    let cfg = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    let db = UnitDb::<KernoDb>::open(SimVfs::new(1).as_vfs(), Path::new("/assoc"), &cfg).expect("open");
    let mut ids = Vec::with_capacity(corpus.units.len());
    let (mut next_unit, mut next_rel) = (0, 0);
    for change in &corpus.changes {
        let meta = ChangeMeta {
            reason: Some(format!("story step {}", change.step)),
            actor: Some(change.actor.clone().into_bytes()),
            source: Some(change.source.clone().into_bytes()),
            ..ChangeMeta::default()
        };
        let mut cs = db.begin_changeset(meta).expect("begin change set");
        while let Some(u) = corpus.units.get(next_unit).filter(|u| u.step == change.step) {
            let new = NewUnit {
                unit_type: u.unit_type.clone(),
                schema_version: 1,
                metadata: Vec::new(),
                payload: u.label.clone().into_bytes(),
            };
            ids.push(cs.create(new).expect("create unit"));
            next_unit += 1;
        }
        while let Some(r) = corpus.relations.get(next_rel).filter(|r| r.step == change.step) {
            let from = ids[r.from as usize];
            let to = ids[r.to as usize];
            cs.create_relation(from, to, &r.relation_type, provenance(&r.provenance)).expect("create relation");
            next_rel += 1;
        }
        cs.commit().expect("commit change set");
    }
    (db, ids)
}

/// B0: the units of the seed's type, in the order the query returns them.
fn baseline_b0(snap: &UnitSnapshot<KernoDb>, index_of: &HashMap<UnitId, u32>, unit_type: &str) -> Vec<u32> {
    let filter = UnitFilter { unit_type: Some(unit_type.to_owned()), schema_version: None };
    let outcome = snap.query(&filter).expect("query by type");
    outcome.units.iter().take(CUT).map(|u| index_of[&u.id]).collect()
}

/// B1: one-hop neighbours of the seed, outgoing then incoming, deduplicated.
fn baseline_b1(snap: &UnitSnapshot<KernoDb>, index_of: &HashMap<UnitId, u32>, seed: UnitId) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for (dir, pick_other) in [(Direction::Out, true), (Direction::In, false)] {
        for rel in snap.relations_of(seed, dir, None).expect("relations_of") {
            let other = if pick_other { rel.to } else { rel.from };
            let index = index_of[&other];
            if !out.contains(&index) {
                out.push(index);
            }
        }
    }
    out.truncate(CUT);
    out
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

#[test]
fn baselines_over_the_frozen_set() {
    let corpus = assoc_eval::parse_corpus(CORPUS).expect("corpus parses");
    let set: QuerySet = assoc_eval::parse_queries(QUERIES).expect("queries parse");

    let (db, ids) = load(&corpus);
    let index_of: HashMap<UnitId, u32> = ids.iter().enumerate().map(|(i, id)| (*id, i as u32)).collect();

    // The database holds exactly the corpus: every relation of the story is
    // readable from its source, with its type, its end and its full provenance.
    let snap = db.snapshot().expect("snapshot");
    for r in &corpus.relations {
        let from = ids[r.from as usize];
        let stored = snap.relations_of(from, Direction::Out, Some(r.relation_type.as_str())).expect("relations_of");
        let found = stored.iter().any(|s| s.to == ids[r.to as usize] && s.provenance == provenance(&r.provenance));
        assert!(found, "relation {} is missing or altered in the database", r.index);
    }
    let units = snap.reference_scan(|_| true).expect("scan").len();
    assert_eq!(units, corpus.units.len(), "every unit of the corpus is active");

    let mut rows = Vec::new();
    for q in &set.queries {
        let b0 = baseline_b0(&snap, &index_of, &q.unit_type);
        let b1 = baseline_b1(&snap, &index_of, ids[q.seed as usize]);
        assert!(b0.len() <= CUT && b1.len() <= CUT, "{}: a baseline is cut at {CUT}", q.id);
        rows.push((q, b0, b1));
    }

    println!(
        "assoc-eval baselines: corpus {} units, {} relations; {} queries; cut {CUT}",
        corpus.units.len(),
        corpus.relations.len(),
        set.queries.len()
    );
    println!("query      class       B0 R@10  B0 nDCG@10  B1 R@10  B1 nDCG@10");
    let mut per_query = Vec::new();
    for (q, b0, b1) in &rows {
        let (r0, n0) = (recall_at(b0, &q.expected, CUT), ndcg_at(b0, &q.expected, CUT));
        let (r1, n1) = (recall_at(b1, &q.expected, CUT), ndcg_at(b1, &q.expected, CUT));
        println!("{:<10} {:<11} {:>7.3} {:>11.3} {:>8.3} {:>11.3}", q.id, q.class.name(), r0, n0, r1, n1);
        per_query.push((q.class, [r0, n0, r1, n1]));
    }
    println!("summary (mean over the queries)");
    println!("scope        queries  B0 R@10  B0 nDCG@10  B1 R@10  B1 nDCG@10");
    let summarize = |label: &str, scores: &[[f64; 4]]| {
        let column = |i: usize| mean(&scores.iter().map(|r| r[i]).collect::<Vec<_>>());
        println!(
            "{:<12} {:>7} {:>7.3} {:>11.3} {:>8.3} {:>11.3}",
            label,
            scores.len(),
            column(0),
            column(1),
            column(2),
            column(3)
        );
    };
    let all: Vec<[f64; 4]> = per_query.iter().map(|(_, m)| *m).collect();
    summarize("overall", &all);
    for (class, name) in CLASSES {
        let rows: Vec<[f64; 4]> = per_query.iter().filter(|(c, _)| *c == class).map(|(_, m)| *m).collect();
        summarize(name, &rows);
    }
}
