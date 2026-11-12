//! The frozen derived-data evaluation set (E8-01) loaded into a units
//! database with one persisted vector per unit (E8-04), and the pieces the
//! measurement of E8-07 shares between `tests/derived_measure.rs` and
//! `examples/derived_bench.rs`.
//!
//! Written for E8-07 and not taken from `harness/assoc.rs`: that loader mints
//! unit ids in creation order, which would give the tie-break by identifier
//! back to the creation order (acta §6.1, R-E8-13). Here a unit's id is its
//! permuted corpus identifier.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use kerno_core::contract::TxnEngine;
use kerno_testkit::derived_eval::{self, DIM};
use kerno_units::*;

pub const CORPUS: &str = include_str!("../../../kerno-testkit/data/derived-eval/corpus.txt");
pub const VECTORS: &str = include_str!("../../../kerno-testkit/data/derived-eval/vectors.txt");
pub const QUERIES: &str = include_str!("../../../kerno-testkit/data/derived-eval/queries.txt");
pub const MANIFEST: &str = include_str!("../../../kerno-testkit/data/derived-eval/MANIFEST");

pub const T0: Timestamp = Timestamp(1_700_000_000_000_000);

/// How the measured vectors are made: the method, its version and the model
/// profile of the seeded test function (acta D5: no model).
pub const METHOD: &str = "seeded-vector";
pub const METHOD_VERSION: u32 = 1;
pub const PROFILE: &str = "seeded-test-vectors";

/// The unit id of a corpus identifier: `0 … 0 ‖ (id + 1)` big-endian, so ids
/// sort as the permuted identifiers do.
pub fn unit_id(corpus_id: u32) -> UnitId {
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&(u64::from(corpus_id) + 1).to_be_bytes());
    UnitId(id)
}

/// Mints the ids of the corpus load in the order the load asks for them: the
/// units of a step, then its relations (a relation id comes from the same
/// source). Units get their permuted corpus identifier, relations an id above
/// `2^40`; after the corpus, ids above `2^32` in order.
pub struct PermutedIds {
    order: Vec<UnitId>,
    next: AtomicUsize,
}

impl PermutedIds {
    pub fn new(corpus: &derived_eval::Corpus) -> Self {
        let (mut units, mut relations) = (corpus.units.iter().peekable(), corpus.relations.iter().peekable());
        let mut order = Vec::with_capacity(corpus.units.len() + corpus.relations.len());
        for change in &corpus.changes {
            while let Some(u) = units.next_if(|u| u.step == change.step) {
                order.push(unit_id(u.id));
            }
            while relations.next_if(|r| r.step == change.step).is_some() {
                let mut id = [0u8; 16];
                id[8..].copy_from_slice(&((1u64 << 40) + order.len() as u64).to_be_bytes());
                order.push(UnitId(id));
            }
        }
        PermutedIds { order, next: AtomicUsize::new(0) }
    }
}

impl UnitIdSource for PermutedIds {
    fn next_id(&self) -> UnitId {
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        self.order.get(n).copied().unwrap_or_else(|| {
            let mut id = [0u8; 16];
            id[8..].copy_from_slice(&((1u64 << 32) + (n - self.order.len()) as u64).to_be_bytes());
            UnitId(id)
        })
    }
}

pub fn recipe() -> VectorRecipe {
    VectorRecipe {
        method: METHOD.into(),
        version: METHOD_VERSION,
        profile: PROFILE.into(),
        params: BTreeMap::from([("dim".to_owned(), DIM.to_string())]),
    }
}

/// The vector of a corpus identifier as the store keeps it.
pub fn f32_vector(v: &[i32; DIM]) -> Vec<f32> {
    v.iter().map(|x| *x as f32).collect()
}

pub fn spec(source: RevisionId, vector: Vec<f32>) -> VectorSpec {
    let r = recipe();
    VectorSpec { source, method: r.method, method_version: r.version, profile: r.profile, params: r.params, vector }
}

pub fn story_provenance(p: &derived_eval::Provenance) -> RelationProvenance {
    use derived_eval::Provenance as P;
    match p {
        P::Declared => RelationProvenance::Declared,
        P::Imported(source) => RelationProvenance::Imported { source: source.clone().into_bytes() },
        P::Algorithmic(origin) => RelationProvenance::Algorithmic { origin: origin.clone().into_bytes() },
        P::Inferred(origin, confidence) => {
            RelationProvenance::Inferred { origin: origin.clone().into_bytes(), confidence_permille: *confidence }
        }
    }
}

/// What a load wrote, by corpus identifier.
pub struct Loaded {
    /// The unit id of each corpus identifier.
    pub units: Vec<UnitId>,
    /// The persisted vector of each corpus identifier, when `with_vectors`.
    pub derived: Vec<Option<DerivedId>>,
}

/// Loads the corpus: one change set per step with the step's actor and source,
/// and, with `with_vectors`, the vector of each unit persisted in the change
/// set that creates it. A step is the commit sequence of its change set.
pub fn load<E: TxnEngine>(
    dd: &DerivedDb<E>,
    corpus: &derived_eval::Corpus,
    vectors: &[[i32; DIM]],
    with_vectors: bool,
) -> Result<Loaded, Box<dyn std::error::Error>> {
    let n = corpus.units.len();
    let mut loaded = Loaded { units: vec![unit_id(0); n], derived: vec![None; n] };
    let (mut next_unit, mut next_rel) = (0, 0);
    for change in &corpus.changes {
        let meta = ChangeMeta {
            actor: Some(change.actor.clone().into_bytes()),
            source: Some(change.source.clone().into_bytes()),
            ..ChangeMeta::default()
        };
        let mut cs = dd.begin_changeset(meta)?;
        while let Some(u) = corpus.units.get(next_unit).filter(|u| u.step == change.step) {
            let new = NewUnit {
                unit_type: u.kind.name().to_owned(),
                schema_version: 1,
                metadata: Vec::new(),
                payload: u.label.clone().into_bytes(),
            };
            let id = cs.create(new)?;
            loaded.units[u.id as usize] = id;
            if with_vectors {
                let head = cs.get_unit(id)?.ok_or("a created unit is not readable")?.head;
                let source = RevisionId { unit: id, rev: head };
                loaded.derived[u.id as usize] =
                    Some(cs.put_vector(&spec(source, f32_vector(&vectors[u.id as usize])))?);
            }
            next_unit += 1;
        }
        while let Some(r) = corpus.relations.get(next_rel).filter(|r| r.step == change.step) {
            cs.create_relation(
                loaded.units[r.from as usize],
                loaded.units[r.to as usize],
                r.kind,
                story_provenance(&r.provenance),
            )?;
            next_rel += 1;
        }
        let info = cs.commit()?;
        if info.commit_seq.0 != change.step {
            return Err(format!("step {} committed as sequence {}", change.step, info.commit_seq.0).into());
        }
    }
    Ok(loaded)
}
