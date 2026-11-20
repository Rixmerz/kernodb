//! The HNSW graph in memory (Malkov and Yashunin, 2016), with no I/O: every
//! change records which nodes it touched so [`super::AnnIndex`] can write
//! exactly those.
//!
//! * One node per unit; ids are never reused, so a link to a deleted node can
//!   be told from a link to a live one.
//! * Cosine stores unit-length vectors and uses `1 - dot`; L2 compares squared
//!   distances and takes the root only when it reports one.
//! * Levels are drawn with an integer rule (a level is kept with probability
//!   `1/M`), so a fixed seed gives the same graph on every platform.
//! * Neighbours are chosen with the heuristic of Algorithm 4 (diverse, topped
//!   up with the pruned ones), on insert and when a list overflows.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashSet};

use crate::error::UnitError;
use crate::types::UnitId;

/// The highest level a node can have: `P(level >= 16) = M^-16`.
pub(super) const MAX_LEVEL: usize = 15;

/// The distance an index measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnMetric {
    /// `1 - cos(a, b)`, in `[0, 2]`. A zero vector has no direction: refused.
    Cosine,
    /// The Euclidean distance.
    L2,
}

pub(super) struct Node {
    pub unit: UnitId,
    /// Which vector this node was built from (for `reconcile`); opaque here.
    pub stamp: [u8; 32],
    /// As stored: unit length under cosine.
    pub vector: Vec<f32>,
    /// `links[l]` are the neighbours at level `l`; `links.len() - 1` is the node's level.
    pub links: Vec<Vec<u32>>,
}

/// A node and its distance to whatever it is being compared with. Ordered by
/// distance, then id, so ties never depend on the heap.
#[derive(Clone, Copy, Debug)]
struct Cand {
    d: f32,
    id: u32,
}

impl PartialEq for Cand {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Cand {}
impl PartialOrd for Cand {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cand {
    fn cmp(&self, o: &Self) -> Ordering {
        self.d.total_cmp(&o.d).then(self.id.cmp(&o.id))
    }
}

pub(super) struct Graph {
    pub dim: usize,
    pub metric: AnnMetric,
    pub m: usize,
    pub ef_construction: usize,
    pub nodes: BTreeMap<u32, Node>,
    pub by_unit: BTreeMap<UnitId, u32>,
    /// A node of the highest level.
    pub entry: Option<u32>,
    pub next_id: u32,
    pub rng: u64,
    /// Nodes written or rewritten since the last flush.
    pub dirty: BTreeSet<u32>,
    /// Nodes deleted since the last flush.
    pub removed: BTreeSet<u32>,
}

/// One hit: the unit and its distance to the query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnnHit {
    pub unit: UnitId,
    /// Cosine: `1 - cos`. L2: the Euclidean distance.
    pub distance: f32,
}

impl Graph {
    pub fn new(dim: usize, metric: AnnMetric, m: usize, ef_construction: usize, seed: u64) -> Self {
        Graph {
            dim,
            metric,
            m,
            ef_construction,
            nodes: BTreeMap::new(),
            by_unit: BTreeMap::new(),
            entry: None,
            next_id: 0,
            rng: seed,
            dirty: BTreeSet::new(),
            removed: BTreeSet::new(),
        }
    }

    /// Most links of a node at `level`: twice `M` at the bottom, as in the paper.
    pub fn max_links(&self, level: usize) -> usize {
        if level == 0 { 2 * self.m } else { self.m }
    }

    /// The vector as the index stores it, or why it cannot be indexed.
    pub fn prepare(&self, v: &[f32]) -> Result<Vec<f32>, UnitError> {
        if v.len() != self.dim {
            return Err(UnitError::InvalidArgument(format!(
                "vector has {} elements, the index has dimension {}",
                v.len(),
                self.dim
            )));
        }
        if let Some(i) = v.iter().position(|x| !x.is_finite()) {
            return Err(UnitError::InvalidArgument(format!("element {i} of the vector is not finite")));
        }
        let mut out: Vec<f32> = v.iter().map(|x| if *x == 0.0 { 0.0 } else { *x }).collect();
        if self.metric == AnnMetric::Cosine {
            let norm = out.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt();
            if norm == 0.0 {
                return Err(UnitError::InvalidArgument("a zero vector has no direction under cosine".into()));
            }
            for x in &mut out {
                *x = (f64::from(*x) / norm) as f32;
            }
        }
        Ok(out)
    }

    fn dist(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.metric {
            AnnMetric::L2 => a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum(),
            AnnMetric::Cosine => (1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()).max(0.0),
        }
    }

    fn reported(&self, d: f32) -> f32 {
        if self.metric == AnnMetric::L2 { d.sqrt() } else { d }
    }

    fn next_rand(&mut self) -> u64 {
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn draw_level(&mut self) -> usize {
        let m = self.m as u64;
        let mut level = 0;
        while level < MAX_LEVEL && self.next_rand().is_multiple_of(m) {
            level += 1;
        }
        level
    }

    fn top_level(&self) -> usize {
        self.entry.and_then(|e| self.nodes.get(&e)).map_or(0, |n| n.links.len() - 1)
    }

    /// The `ef` nodes nearest to `q` that are reachable at `level` from `eps`,
    /// nearest first. Links to nodes that no longer exist are skipped.
    fn search_layer(&self, q: &[f32], eps: &[Cand], ef: usize, level: usize) -> Vec<Cand> {
        let mut visited: HashSet<u32> = eps.iter().map(|c| c.id).collect();
        let mut todo: BinaryHeap<Reverse<Cand>> = eps.iter().map(|c| Reverse(*c)).collect();
        let mut found: BinaryHeap<Cand> = eps.iter().copied().collect();
        while found.len() > ef {
            found.pop();
        }
        while let Some(Reverse(c)) = todo.pop() {
            if found.len() >= ef && found.peek().is_some_and(|worst| c.d > worst.d) {
                break;
            }
            let Some(node) = self.nodes.get(&c.id) else { continue };
            for &n in node.links.get(level).map_or(&[][..], Vec::as_slice) {
                if !visited.insert(n) {
                    continue;
                }
                let Some(other) = self.nodes.get(&n) else { continue };
                let cand = Cand { d: self.dist(q, &other.vector), id: n };
                if found.len() < ef || found.peek().is_some_and(|worst| cand < *worst) {
                    todo.push(Reverse(cand));
                    found.push(cand);
                    if found.len() > ef {
                        found.pop();
                    }
                }
            }
        }
        found.into_sorted_vec()
    }

    /// Algorithm 4: from `cands` (nearest first, distances to the base point)
    /// keep those nearer to the base than to any one already kept, then top up
    /// with the discarded ones, up to `max`.
    fn select(&self, cands: &[Cand], max: usize) -> Vec<u32> {
        let mut kept: Vec<Cand> = Vec::with_capacity(max);
        let mut discarded: Vec<Cand> = Vec::new();
        for c in cands {
            if kept.len() >= max {
                break;
            }
            let Some(cv) = self.nodes.get(&c.id).map(|n| &n.vector) else { continue };
            let diverse = kept.iter().filter_map(|r| self.nodes.get(&r.id)).all(|r| self.dist(cv, &r.vector) > c.d);
            if diverse { kept.push(*c) } else { discarded.push(*c) }
        }
        kept.extend(discarded.into_iter().take(max - kept.len()));
        kept.into_iter().map(|c| c.id).collect()
    }

    /// `ids` (live nodes of at least `level`) sorted by distance to `base`.
    fn nearest_first(&self, base: &[f32], ids: impl IntoIterator<Item = u32>, level: usize) -> Vec<Cand> {
        let mut out: Vec<Cand> = ids
            .into_iter()
            .filter_map(|id| {
                let n = self.nodes.get(&id)?;
                (n.links.len() > level).then(|| Cand { d: self.dist(base, &n.vector), id })
            })
            .collect();
        out.sort();
        out
    }

    /// Adds `new` to the links of `at` on `level`, pruning the list if it overflows.
    fn add_link(&mut self, at: u32, level: usize, new: u32) {
        let Some(node) = self.nodes.get(&at) else { return };
        let Some(list) = node.links.get(level) else { return };
        let mut ids: BTreeSet<u32> = list.iter().copied().collect();
        ids.insert(new);
        let max = self.max_links(level);
        let links = if ids.len() <= max {
            ids.into_iter().collect()
        } else {
            let sorted = self.nearest_first(&node.vector, ids, level);
            self.select(&sorted, max)
        };
        if let Some(node) = self.nodes.get_mut(&at) {
            node.links[level] = links;
            self.dirty.insert(at);
        }
    }

    /// Indexes `vector` (already `prepare`d) for `unit`, replacing the unit's
    /// previous node. The caller has checked that an id is left.
    pub fn insert(&mut self, unit: UnitId, stamp: [u8; 32], vector: Vec<f32>) {
        self.remove(&unit);
        let level = self.draw_level();
        let id = self.next_id;
        self.next_id += 1;
        let mut links: Vec<Vec<u32>> = vec![Vec::new(); level + 1];
        let top = self.top_level();
        if let Some(entry) = self.entry.filter(|e| self.nodes.contains_key(e)) {
            let start = self.nodes.get(&entry).map_or(f32::MAX, |n| self.dist(&vector, &n.vector));
            let mut eps = vec![Cand { d: start, id: entry }];
            for lc in ((level + 1)..=top).rev() {
                eps = self.search_layer(&vector, &eps, 1, lc);
            }
            for lc in (0..=level.min(top)).rev() {
                let found = self.search_layer(&vector, &eps, self.ef_construction, lc);
                links[lc] = self.select(&found, self.m);
                eps = found;
            }
        }
        let linked: Vec<(usize, Vec<u32>)> = links.iter().cloned().enumerate().collect();
        self.nodes.insert(id, Node { unit, stamp, vector, links });
        self.by_unit.insert(unit, id);
        self.dirty.insert(id);
        for (lc, neighbours) in linked {
            for n in neighbours {
                self.add_link(n, lc, id);
            }
        }
        if self.entry.is_none_or(|e| !self.nodes.contains_key(&e)) || level > top {
            self.entry = Some(id);
        }
    }

    /// Deletes the node of `unit`. Each node it pointed to gets its links
    /// rebuilt from its own and the deleted node's, so the graph stays
    /// connected through the hole.
    ///
    /// ponytail: a node that points to the deleted one without being pointed
    /// to by it keeps a dangling id until its list is next rewritten; searches
    /// skip it. Finding those in-links would scan every node; add a reverse
    /// index if deletes become the common case.
    pub fn remove(&mut self, unit: &UnitId) -> bool {
        let Some(id) = self.by_unit.remove(unit) else { return false };
        let Some(gone) = self.nodes.remove(&id) else { return false };
        self.removed.insert(id);
        self.dirty.remove(&id);
        for (lc, outs) in gone.links.iter().enumerate() {
            for &n in outs {
                let Some(node) = self.nodes.get(&n) else { continue };
                let Some(list) = node.links.get(lc) else { continue };
                let ids: BTreeSet<u32> = list.iter().chain(outs).copied().filter(|x| *x != n && *x != id).collect();
                let sorted = self.nearest_first(&node.vector, ids, lc);
                let links = self.select(&sorted, self.max_links(lc));
                if let Some(node) = self.nodes.get_mut(&n) {
                    node.links[lc] = links;
                    self.dirty.insert(n);
                }
            }
        }
        if self.entry == Some(id) {
            // ponytail: a scan, but only when the one entry node goes.
            self.entry = self.nodes.iter().max_by_key(|(i, n)| (n.links.len(), Reverse(**i))).map(|(i, _)| *i);
        }
        true
    }

    /// The `k` nearest units to `query` (already `prepare`d), searching with a
    /// candidate list of `max(ef, k)`; nearest first, ties by unit id.
    pub fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<AnnHit> {
        let Some(entry) = self.entry.and_then(|e| self.nodes.get(&e).map(|n| (e, n))) else {
            return Vec::new();
        };
        let mut eps = vec![Cand { d: self.dist(query, &entry.1.vector), id: entry.0 }];
        for lc in (1..=self.top_level()).rev() {
            eps = self.search_layer(query, &eps, 1, lc);
        }
        let mut hits: Vec<AnnHit> = self
            .search_layer(query, &eps, ef.max(k), 0)
            .into_iter()
            .filter_map(|c| self.nodes.get(&c.id).map(|n| AnnHit { unit: n.unit, distance: self.reported(c.d) }))
            .collect();
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.unit.cmp(&b.unit)));
        hits.truncate(k);
        hits
    }
}
