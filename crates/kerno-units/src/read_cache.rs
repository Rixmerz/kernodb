//! A cache of what a read already loaded and verified: schemas, units, the units of an
//! indexed-field range and the relations of a unit.
//!
//! Reading a unit costs a point read and a content hash (three point reads
//! when its head does not carry its payload, `units-v10.md`); `find` also
//! reads and decodes the type's schema on every call. A snapshot at one commit sequence never changes, so what one
//! read verified there, the next read at the same sequence may reuse.
//!
//! # Validity
//!
//! Every entry belongs to one engine sequence, the cache's. A read at a newer
//! sequence drops everything and moves the cache there; a read at an older one
//! (a snapshot taken before a commit and still in use) neither reads nor
//! fills it. There is no "this commit did not touch that unit" exception: any
//! commit empties the cache. It therefore helps the reads between two
//! commits, which is every read of a read-only handle between two `refresh`
//! calls, and nothing else.
//!
//! In memory only, per database handle, outside every transaction; the write
//! path does not use it. A cached unit was verified when it was loaded and is
//! not verified again while it is served: damage that reaches the files after
//! that is found by the next read after a commit, a reopen, or `verify`.
//!
//! `ponytail:` bounded by dropping every unit once their payloads exceed the
//! budget, not by evicting the oldest. Upgrade path: an LRU, if a working set
//! larger than the budget becomes common.

use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::sync::{Arc, Mutex};

use crate::db::{IdHasher, Relation, Unit};
use crate::schema::Schema;
use crate::types::UnitId;

/// Payload bytes of cached units a database holds at most, by default.
pub const DEFAULT_READ_CACHE_BYTES: usize = 32 << 20;

/// What a unit weighs in the budget besides its payload.
const UNIT_OVERHEAD: usize = 160;

#[derive(Default)]
struct Inner {
    seq: u64,
    budget: usize,
    bytes: usize,
    units: HashMap<UnitId, Arc<Unit>, IdHash>,
    schemas: HashMap<String, Option<Arc<Schema>>>,
    ranges: HashMap<RangeKey, Arc<Vec<UnitId>>>,
    relations: HashMap<RelationsKey, Arc<Vec<Relation>>, IdHash>,
}

/// Unit ids are UUIDv7 with random tail bytes: [`IdHasher`] spreads them, no keyed hash needed.
type IdHash = BuildHasherDefault<IdHasher>;

/// An index key range and the limit it was read with.
type RangeKey = (Vec<u8>, Vec<u8>, usize);
/// A unit, the adjacency tag of the direction, and the relation type asked for.
type RelationsKey = (UnitId, u8, Option<String>);

/// What a relation or a range entry weighs in the budget.
const ENTRY_OVERHEAD: usize = 96;

impl Inner {
    /// Whether a read at `seq` may use the cache, moving it forward first.
    fn at(&mut self, seq: u64) -> bool {
        if seq > self.seq {
            self.seq = seq;
            self.schemas.clear();
            self.drop_data();
        }
        seq == self.seq && self.budget > 0
    }

    fn drop_data(&mut self) {
        self.units.clear();
        self.ranges.clear();
        self.relations.clear();
        self.bytes = 0;
    }

    /// Room for `weight` more bytes, by dropping what is held when there is none; `false` for
    /// an entry too large to be worth holding.
    fn admit(&mut self, weight: usize) -> bool {
        if weight > self.budget / 8 {
            return false;
        }
        if self.bytes + weight > self.budget {
            self.drop_data();
        }
        self.bytes += weight;
        true
    }
}

pub(crate) struct ReadCache(Mutex<Inner>);

impl ReadCache {
    pub(crate) fn new(budget: usize) -> Self {
        ReadCache(Mutex::new(Inner { budget, ..Inner::default() }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // The cache holds no invariant a panic could break: use it as it is.
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets the budget; `0` turns the cache off. Either way it starts empty.
    pub(crate) fn set_budget(&self, budget: usize) {
        let mut c = self.lock();
        c.budget = budget;
        c.schemas.clear();
        c.drop_data();
    }

    /// Drops everything held; the budget stays.
    pub(crate) fn clear(&self) {
        let mut c = self.lock();
        c.schemas.clear();
        c.drop_data();
    }

    pub(crate) fn unit(&self, seq: u64, id: UnitId) -> Option<Arc<Unit>> {
        let mut c = self.lock();
        if c.at(seq) { c.units.get(&id).cloned() } else { None }
    }

    pub(crate) fn put_unit(&self, seq: u64, unit: &Arc<Unit>) {
        let weight = UNIT_OVERHEAD + unit.payload.as_ref().map_or(0, Vec::len) + unit.metadata.len();
        let mut c = self.lock();
        if c.at(seq) && !c.units.contains_key(&unit.id) && c.admit(weight) {
            c.units.insert(unit.id, Arc::clone(unit));
        }
    }

    /// The unit ids an index range scan of `[start, end)` with `limit` returned.
    pub(crate) fn range(&self, seq: u64, key: &RangeKey) -> Option<Arc<Vec<UnitId>>> {
        let mut c = self.lock();
        if c.at(seq) { c.ranges.get(key).cloned() } else { None }
    }

    pub(crate) fn put_range(&self, seq: u64, key: RangeKey, ids: Vec<UnitId>) {
        let weight = ENTRY_OVERHEAD + key.0.len() + key.1.len() + ids.len() * 16;
        let mut c = self.lock();
        if c.at(seq) && !c.ranges.contains_key(&key) && c.admit(weight) {
            c.ranges.insert(key, Arc::new(ids));
        }
    }

    pub(crate) fn relations(&self, seq: u64, key: &RelationsKey) -> Option<Arc<Vec<Relation>>> {
        let mut c = self.lock();
        if c.at(seq) { c.relations.get(key).cloned() } else { None }
    }

    pub(crate) fn put_relations(&self, seq: u64, key: RelationsKey, relations: &[Relation]) {
        let weight = ENTRY_OVERHEAD + relations.iter().map(|r| ENTRY_OVERHEAD + r.relation_type.len()).sum::<usize>();
        let mut c = self.lock();
        if c.at(seq) && !c.relations.contains_key(&key) && c.admit(weight) {
            c.relations.insert(key, Arc::new(relations.to_vec()));
        }
    }

    /// `Some(None)`: the type is known to have no schema at `seq`.
    pub(crate) fn schema(&self, seq: u64, unit_type: &str) -> Option<Option<Arc<Schema>>> {
        let mut c = self.lock();
        if c.at(seq) { c.schemas.get(unit_type).cloned() } else { None }
    }

    pub(crate) fn put_schema(&self, seq: u64, unit_type: &str, schema: Option<&Schema>) {
        let mut c = self.lock();
        // Type names come from callers: do not let a stream of unknown names grow the map.
        if c.at(seq) && c.schemas.len() < 1024 {
            c.schemas.insert(unit_type.to_owned(), schema.map(|s| Arc::new(s.clone())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_sequence_empties_the_cache_and_an_older_one_bypasses_it() {
        let c = ReadCache::new(1 << 20);
        c.put_schema(5, "t", None);
        assert_eq!(c.schema(5, "t").map(|s| s.is_none()), Some(true));
        assert!(c.schema(4, "t").is_none(), "an older snapshot does not read the cache");
        c.put_schema(4, "old", None);
        assert!(c.schema(5, "old").is_none(), "an older snapshot does not fill it");
        assert!(c.schema(5, "t").is_some());
        assert!(c.schema(6, "t").is_none(), "a newer sequence starts empty");
        assert!(c.schema(5, "t").is_none(), "and the cache has moved on");
    }

    #[test]
    fn a_zero_budget_is_off() {
        let c = ReadCache::new(0);
        c.put_schema(1, "t", None);
        assert!(c.schema(1, "t").is_none());
    }
}
