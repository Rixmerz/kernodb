//! Approximate nearest-neighbour search over stored embeddings: an HNSW index
//! ([`hnsw`] holds the graph, this file the persistence and the API).
//!
//! # Contracts
//!
//! * **Never a source of truth.** The vectors of [`crate::vector_store`] are.
//!   The index is derived from them: [`AnnIndex::reconcile`] (through
//!   [`DerivedDb::sync_ann`](crate::DerivedDb::sync_ann)) makes it equal to the
//!   `Ready` vectors of the database, whatever it held before, so a lost or
//!   stale index is repaired by deleting its directory and syncing again.
//! * **Its own engine.** The index is stored through the engine contract of
//!   `kerno-core`, in a directory of its own, not in the units key space: a
//!   units database never sees a record it does not know (`verify_units`),
//!   and the layer version does not move. Every change is one atomic
//!   [`WriteBatch`](kerno_core::WriteBatch): the nodes it touched and the
//!   header (entry point, id counter, generator state).
//! * **One vector per unit.** A node is keyed by unit and carries a 32-byte
//!   `stamp` naming the vector it was built from (the [`DerivedId`] under
//!   `sync_ann`); a unit whose stamp changed is re-indexed.
//! * **Searches read memory.** Opening loads the whole graph. A search does no
//!   I/O and takes `&self`; a write takes `&mut self`.
//! * **Reject, never misread.** A damaged record is `Corruption`, an index
//!   opened with another dimension, metric or `M` is `InvalidArgument`; neither
//!   is repaired silently.
//! * **No content in errors (I-37).** Errors and `Debug` name numbers, never a
//!   vector element.
//!
//! ponytail: the graph and its vectors live in memory (a copy of the vectors
//! `DerivedDb` already holds), which fits a few million small vectors; page the
//! nodes through the engine if that stops being enough.
//!
//! # Record layout (all integers big-endian)
//!
//! | key | value |
//! |---|---|
//! | `0x01` | `format u8 ‖ dim u32 ‖ metric u8 ‖ m u32 ‖ next_id u32 ‖ entry u32 ‖ rng u64`; `entry = u32::MAX` for none |
//! | `0x02 ‖ id u32` | `format u8 ‖ unit[16] ‖ stamp[32] ‖ level u8 ‖ f32 × dim ‖ (count u16 ‖ u32 × count) × (level + 1)` |

mod hnsw;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::vfs::Vfs;

pub use hnsw::{AnnHit, AnnMetric};
use hnsw::{Graph, MAX_LEVEL, Node};

use crate::error::UnitError;
use crate::types::UnitId;
use crate::vector_store::MAX_DIM;

const FORMAT: u8 = 1;
const KEY_META: [u8; 1] = [0x01];
const TAG_NODE: u8 = 0x02;
const META_LEN: usize = 1 + 4 + 1 + 4 + 4 + 4 + 8;
const NO_ENTRY: u32 = u32::MAX;
const PAGE: usize = 256;

/// The parameters of an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnConfig {
    /// Elements per vector, `1..=MAX_DIM`. Fixed for the life of the index.
    pub dim: usize,
    /// Fixed for the life of the index.
    pub metric: AnnMetric,
    /// Links per node above the bottom level (twice that at the bottom),
    /// `2..=256`. Fixed for the life of the index.
    pub m: usize,
    /// Candidates kept while inserting, at least `m`. More is slower to build
    /// and better connected.
    pub ef_construction: usize,
    /// Candidates kept while searching, at least 1 (a search of `k` keeps at
    /// least `k`). More is slower and closer to exact.
    pub ef_search: usize,
    /// Seeds the level generator of a new index; an existing one continues
    /// from the state it stored.
    pub seed: u64,
}

impl AnnConfig {
    /// `M = 16`, `ef_construction = 200`, `ef_search = 64`, seed 0.
    pub fn new(dim: usize, metric: AnnMetric) -> Self {
        AnnConfig { dim, metric, m: 16, ef_construction: 200, ef_search: 64, seed: 0 }
    }

    fn validate(&self) -> Result<(), UnitError> {
        let bad = |what: String| Err(UnitError::InvalidArgument(format!("ann config: {what}")));
        if self.dim == 0 || self.dim > MAX_DIM as usize {
            return bad(format!("dim {} is outside 1..={MAX_DIM}", self.dim));
        }
        if !(2..=256).contains(&self.m) {
            return bad(format!("m {} is outside 2..=256", self.m));
        }
        if self.ef_construction < self.m {
            return bad(format!("ef_construction {} is below m {}", self.ef_construction, self.m));
        }
        if self.ef_search == 0 {
            return bad("ef_search is 0".into());
        }
        Ok(())
    }
}

/// What [`AnnIndex::reconcile`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Units that had no node.
    pub added: usize,
    /// Units whose stamp changed: their node was replaced.
    pub replaced: usize,
    /// Units that were indexed and are no longer wanted.
    pub removed: usize,
    /// Units already indexed with the wanted stamp.
    pub unchanged: usize,
    /// Wanted vectors the index cannot hold (another dimension, or a zero
    /// vector under cosine); they are left out, as `DerivedDb::vector_set` does.
    pub skipped: usize,
}

/// An HNSW index persisted through an engine `E`.
pub struct AnnIndex<E: Engine> {
    engine: E,
    graph: Graph,
    ef_search: usize,
    /// Set when a commit failed: memory is ahead of the disk, so nothing else
    /// is served or written until the index is reopened.
    poisoned: bool,
}

impl<E: Engine> fmt::Debug for AnnIndex<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnnIndex")
            .field("dim", &self.graph.dim)
            .field("metric", &self.graph.metric)
            .field("m", &self.graph.m)
            .field("len", &self.graph.nodes.len())
            .finish_non_exhaustive()
    }
}

fn corrupt(msg: impl fmt::Display) -> UnitError {
    UnitError::Storage(kerno_core::Error::Corruption(format!("ann index: {msg}")))
}

fn node_key(id: u32) -> [u8; 5] {
    let mut k = [TAG_NODE, 0, 0, 0, 0];
    k[1..].copy_from_slice(&id.to_be_bytes());
    k
}

fn metric_byte(m: AnnMetric) -> u8 {
    match m {
        AnnMetric::Cosine => 1,
        AnnMetric::L2 => 2,
    }
}

fn metric_of(b: u8) -> Option<AnnMetric> {
    match b {
        1 => Some(AnnMetric::Cosine),
        2 => Some(AnnMetric::L2),
        _ => None,
    }
}

// ----------------------------------------------------------------- records

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], UnitError> {
        let Some(s) = self.buf.get(self.pos..self.pos + n) else {
            return Err(corrupt(format!("{} is truncated at byte {}", self.what, self.pos)));
        };
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, UnitError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, UnitError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| corrupt("short u16"))?))
    }

    fn u32(&mut self) -> Result<u32, UnitError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| corrupt("short u32"))?))
    }

    fn u64(&mut self) -> Result<u64, UnitError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| corrupt("short u64"))?))
    }

    fn finish(&self) -> Result<(), UnitError> {
        match self.buf.len() - self.pos {
            0 => Ok(()),
            extra => Err(corrupt(format!("{} has {extra} trailing bytes", self.what))),
        }
    }
}

fn encode_meta(g: &Graph) -> Vec<u8> {
    let mut out = Vec::with_capacity(META_LEN);
    out.push(FORMAT);
    out.extend_from_slice(&(g.dim as u32).to_be_bytes());
    out.push(metric_byte(g.metric));
    out.extend_from_slice(&(g.m as u32).to_be_bytes());
    out.extend_from_slice(&g.next_id.to_be_bytes());
    out.extend_from_slice(&g.entry.unwrap_or(NO_ENTRY).to_be_bytes());
    out.extend_from_slice(&g.rng.to_be_bytes());
    out
}

fn encode_node(n: &Node) -> Vec<u8> {
    let mut out = vec![FORMAT];
    out.extend_from_slice(&n.unit.0);
    out.extend_from_slice(&n.stamp);
    out.push((n.links.len() - 1) as u8);
    for x in &n.vector {
        out.extend_from_slice(&x.to_bits().to_be_bytes());
    }
    for list in &n.links {
        out.extend_from_slice(&(list.len() as u16).to_be_bytes());
        for id in list {
            out.extend_from_slice(&id.to_be_bytes());
        }
    }
    out
}

fn decode_node(g: &Graph, id: u32, raw: &[u8]) -> Result<Node, UnitError> {
    let mut c = Cursor { buf: raw, pos: 0, what: "node record" };
    let format = c.u8()?;
    if format != FORMAT {
        return Err(corrupt(format!("node {id} has format {format}")));
    }
    let unit = UnitId(c.take(16)?.try_into().map_err(|_| corrupt("short unit id"))?);
    let stamp: [u8; 32] = c.take(32)?.try_into().map_err(|_| corrupt("short stamp"))?;
    let level = usize::from(c.u8()?);
    if level > MAX_LEVEL {
        return Err(corrupt(format!("node {id} has level {level}")));
    }
    let mut vector = Vec::with_capacity(g.dim);
    for _ in 0..g.dim {
        let x = f32::from_bits(c.u32()?);
        if !x.is_finite() {
            return Err(corrupt(format!("node {id} has a non-finite element")));
        }
        vector.push(x);
    }
    let mut links = Vec::with_capacity(level + 1);
    for lc in 0..=level {
        let count = usize::from(c.u16()?);
        if count > g.max_links(lc) {
            return Err(corrupt(format!("node {id} has {count} links at level {lc}, the most is {}", g.max_links(lc))));
        }
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            let to = c.u32()?;
            if to >= g.next_id || to == id {
                return Err(corrupt(format!("node {id} links to {to}, outside the ids ever issued")));
            }
            list.push(to);
        }
        links.push(list);
    }
    c.finish()?;
    Ok(Node { unit, stamp, vector, links })
}

/// The graph an engine holds, or an empty one built from `cfg` if it holds none.
fn load<E: Engine>(engine: &E, cfg: &AnnConfig) -> Result<Graph, UnitError> {
    let snap = engine.snapshot()?;
    let Some(raw) = snap.get(&KEY_META)? else {
        if !snap.scan(Bound::Included(&[TAG_NODE][..]), Bound::Unbounded, 1)?.is_empty() {
            return Err(corrupt("node records without a header"));
        }
        return Ok(Graph::new(cfg.dim, cfg.metric, cfg.m, cfg.ef_construction, cfg.seed));
    };
    let mut c = Cursor { buf: &raw, pos: 0, what: "header" };
    let format = c.u8()?;
    if format != FORMAT {
        return Err(corrupt(format!("header has format {format}")));
    }
    let dim = c.u32()? as usize;
    let metric = metric_of(c.u8()?).ok_or_else(|| corrupt("header has an unknown metric"))?;
    let m = c.u32()? as usize;
    let next_id = c.u32()?;
    let entry = c.u32()?;
    let rng = c.u64()?;
    c.finish()?;
    if (dim, metric, m) != (cfg.dim, cfg.metric, cfg.m) {
        return Err(UnitError::InvalidArgument(format!(
            "the index was built with dim {dim}, metric {metric:?}, m {m}; opened with dim {}, metric {:?}, m {}",
            cfg.dim, cfg.metric, cfg.m
        )));
    }
    let mut g = Graph::new(dim, metric, m, cfg.ef_construction, rng);
    g.next_id = next_id;
    let (lo, hi) = ([TAG_NODE], [TAG_NODE + 1]);
    let mut last: Option<Vec<u8>> = None;
    loop {
        let start = match &last {
            Some(k) => Bound::Excluded(k.as_slice()),
            None => Bound::Included(&lo[..]),
        };
        let page = snap.scan(start, Bound::Excluded(&hi[..]), PAGE)?;
        for (k, v) in &page {
            let id = u32::from_be_bytes(k[1..].try_into().map_err(|_| corrupt("a node key has the wrong length"))?);
            let node = decode_node(&g, id, v)?;
            if g.by_unit.insert(node.unit, id).is_some() {
                return Err(corrupt(format!("two nodes for one unit (node {id})")));
            }
            g.nodes.insert(id, node);
        }
        last = page.last().map(|(k, _)| k.clone());
        if page.len() < PAGE {
            break;
        }
    }
    let top = g.nodes.values().map(|n| n.links.len()).max();
    match (entry == NO_ENTRY, g.nodes.get(&entry)) {
        (true, _) if g.nodes.is_empty() => {}
        (false, Some(e)) if Some(e.links.len()) == top => g.entry = Some(entry),
        _ => return Err(corrupt(format!("entry {entry} is not a node of the highest level"))),
    }
    // A link to a deleted node, or to one too low for the level, is dead: drop
    // it here so it does not count against the degree.
    let levels: BTreeMap<u32, usize> = g.nodes.iter().map(|(i, n)| (*i, n.links.len())).collect();
    for n in g.nodes.values_mut() {
        for (lc, list) in n.links.iter_mut().enumerate() {
            list.retain(|to| levels.get(to).is_some_and(|l| *l > lc));
        }
    }
    Ok(g)
}

// --------------------------------------------------------------------- API

impl<E: Engine> AnnIndex<E> {
    /// Opens the index stored in `dir`, creating it empty if there is none,
    /// and loads its graph. Opening writes nothing.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a bad `cfg`, or one whose `dim`, `metric` or `m`
    /// differ from the stored index; `Storage(Corruption)` for a damaged
    /// record; the engine's errors (`Locked` if `dir` is open elsewhere).
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options, cfg: AnnConfig) -> Result<Self, UnitError> {
        cfg.validate()?;
        let engine = E::open(vfs, dir, opts)?;
        let graph = match load(&engine, &cfg) {
            Ok(g) => g,
            Err(e) => {
                let _ = engine.close();
                return Err(e);
            }
        };
        Ok(AnnIndex { engine, graph, ef_search: cfg.ef_search, poisoned: false })
    }

    /// Clean shutdown of the engine; everything committed is durable.
    pub fn close(self) -> Result<(), UnitError> {
        Ok(self.engine.close()?)
    }

    pub fn dim(&self) -> usize {
        self.graph.dim
    }

    pub fn metric(&self) -> AnnMetric {
        self.graph.metric
    }

    /// Units indexed.
    pub fn len(&self) -> usize {
        self.graph.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.graph.nodes.is_empty()
    }

    pub fn contains(&self, unit: &UnitId) -> bool {
        self.graph.by_unit.contains_key(unit)
    }

    /// The stamp `unit` was indexed with.
    pub fn stamp(&self, unit: &UnitId) -> Option<[u8; 32]> {
        self.graph.by_unit.get(unit).and_then(|id| self.graph.nodes.get(id)).map(|n| n.stamp)
    }

    fn check(&self) -> Result<(), UnitError> {
        if self.poisoned {
            return Err(UnitError::Storage(kerno_core::Error::Poisoned(
                "an ann index write failed to commit; reopen the index".into(),
            )));
        }
        Ok(())
    }

    /// Whether `n` more nodes can be given an id.
    fn room(&self, n: usize) -> Result<(), UnitError> {
        if u64::from(self.graph.next_id) + n as u64 >= u64::from(NO_ENTRY) {
            return Err(UnitError::InvalidArgument(
                "the index has used every node id; rebuild it with `reconcile` over an empty directory".into(),
            ));
        }
        Ok(())
    }

    /// Writes what the graph changed since the last call, in one batch.
    fn persist(&mut self) -> Result<(), UnitError> {
        let g = &mut self.graph;
        if g.dirty.is_empty() && g.removed.is_empty() {
            return Ok(());
        }
        let mut batch = WriteBatch::new();
        for id in &g.removed {
            batch.delete(node_key(*id));
        }
        for id in &g.dirty {
            if let Some(n) = g.nodes.get(id) {
                batch.put(node_key(*id), encode_node(n));
            }
        }
        batch.put(KEY_META, encode_meta(g));
        g.dirty.clear();
        g.removed.clear();
        self.engine.commit(&batch).map(drop).map_err(|e| {
            self.poisoned = true;
            e.into()
        })
    }

    /// Indexes `vector` for `unit` under `stamp`, replacing what the unit had.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a vector of another dimension, with a non-finite
    /// element, or zero under cosine; nothing changes then.
    pub fn insert(&mut self, unit: UnitId, stamp: [u8; 32], vector: &[f32]) -> Result<(), UnitError> {
        self.check()?;
        self.room(1)?;
        let prepared = self.graph.prepare(vector)?;
        self.graph.insert(unit, stamp, prepared);
        self.persist()
    }

    /// Removes `unit`; `false` if it was not indexed.
    pub fn remove(&mut self, unit: &UnitId) -> Result<bool, UnitError> {
        self.check()?;
        let found = self.graph.remove(unit);
        self.persist()?;
        Ok(found)
    }

    /// The `k` units nearest to `query`, nearest first (ties by unit id), with
    /// their distances: `1 - cos` under cosine, the Euclidean distance under L2.
    /// Approximate: raise `ef_search` for recall.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a query the index would refuse as a vector.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<AnnHit>, UnitError> {
        self.check()?;
        let q = self.graph.prepare(query)?;
        Ok(self.graph.search(&q, k, self.ef_search))
    }

    /// Makes the index hold exactly `desired`, one `(unit, stamp, vector)` per
    /// unit (a later entry for a unit wins): nodes of units not listed are
    /// removed, those with another stamp replaced, new ones added. Committed
    /// once; a call that changes nothing writes nothing.
    ///
    /// A listed vector the index cannot hold is counted in
    /// [`SyncReport::skipped`] and its unit is treated as not listed. A vector
    /// whose unit already has that stamp is not read.
    pub fn reconcile<'a>(
        &mut self,
        desired: impl IntoIterator<Item = (UnitId, [u8; 32], &'a [f32])>,
    ) -> Result<SyncReport, UnitError> {
        enum Want {
            Keep,
            Put([u8; 32], Vec<f32>),
            Skip,
        }
        self.check()?;
        let mut want: BTreeMap<UnitId, Want> = BTreeMap::new();
        for (unit, stamp, vector) in desired {
            let w = if self.stamp(&unit) == Some(stamp) {
                Want::Keep
            } else {
                self.graph.prepare(vector).map_or(Want::Skip, |v| Want::Put(stamp, v))
            };
            want.insert(unit, w);
        }
        let gone: Vec<UnitId> =
            self.graph.by_unit.keys().filter(|u| matches!(want.get(u), None | Some(Want::Skip))).copied().collect();
        let puts = want.values().filter(|w| matches!(w, Want::Put(..))).count();
        self.room(puts)?;
        let mut report = SyncReport { removed: gone.len(), ..SyncReport::default() };
        for u in &gone {
            self.graph.remove(u);
        }
        for (unit, w) in want {
            match w {
                Want::Skip => report.skipped += 1,
                Want::Keep => report.unchanged += 1,
                Want::Put(stamp, vector) => {
                    if self.graph.by_unit.contains_key(&unit) {
                        report.replaced += 1;
                    } else {
                        report.added += 1;
                    }
                    self.graph.insert(unit, stamp, vector);
                }
            }
        }
        self.persist()?;
        Ok(report)
    }
}
