//! Commit-time tree mutation (shadow paging).
//!
//! A commit applies its whole batch in one top-down pass: the batch is
//! sorted and deduplicated, each node on an affected path is copied out of
//! its page *once* into an owned [`MemNode`] (its old page id is recorded as
//! freed), the slice of operations that falls into each child is applied
//! recursively, and every level fixes its children afterwards (drops empty
//! ones, merges underfull ones with a sibling when the result fits, splits
//! oversized ones). Unchanged subtrees stay [`Child::Clean`] references to
//! existing pages.
//!
//! [`Writeout`] then assigns fresh page ids bottom-up (children before
//! parents, so each parent knows its children's crcs), encodes the pages and
//! returns them for writing. Nothing reachable from the last meta or from a
//! live snapshot is ever overwritten: new ids come only from the reusable set
//! or the end of the file.

use std::collections::BTreeMap;
use std::sync::Arc;

use spike_core::{Error, Op, Result, WriteBatch};

use crate::engine::Inner;
use crate::freelist::{FreeState, PageWrites};
use crate::page::{
    BRANCH_KEY_OVERHEAD, Bytes, LeafEntry, NODE_CAPACITY, NODE_HEADER, Node, OVERFLOW_CAPACITY, PAGE_SIZE, PageId,
    PageRef, UNDERFULL, Value, branch_size, encode_overflow_page, inline_fits, leaf_entry_size,
};

/// Sorted, deduplicated operations: `Some(value)` puts, `None` deletes.
pub(crate) type Ops<'b> = [(&'b [u8], Option<&'b [u8]>)];

/// Final operation per key (later operations in the batch win), sorted.
pub(crate) fn dedupe(batch: &WriteBatch) -> Vec<(&[u8], Option<&[u8]>)> {
    let mut map = BTreeMap::new();
    for op in batch.ops() {
        match op {
            Op::Put { key, value } => map.insert(key.as_slice(), Some(value.as_slice())),
            Op::Delete { key } => map.insert(key.as_slice(), None),
            Op::DeleteRange { .. } => unreachable!("delete_range is rejected at the entry of commit"),
        };
    }
    map.into_iter().collect()
}

pub(crate) enum MemValue {
    /// An inline value or an existing overflow chain.
    Stored(Value),
    /// A new large value; its overflow chain is written by [`Writeout`].
    Large(Vec<u8>),
}

pub(crate) struct MemEntry {
    key: Bytes,
    value: MemValue,
}

impl MemEntry {
    fn size(&self) -> usize {
        match &self.value {
            MemValue::Stored(v) => leaf_entry_size(self.key.len(), v),
            MemValue::Large(_) => leaf_entry_size(self.key.len(), &Value::Overflow { len: 0, crc: 0, first: 0 }),
        }
    }
}

/// An owned, modifiable node that will be written to a new page.
pub(crate) enum MemNode {
    Leaf(Vec<MemEntry>),
    Branch { level: u8, keys: Vec<Bytes>, children: Vec<Child> },
}

pub(crate) enum Child {
    /// Unchanged subtree, still in its page.
    Clean(PageRef),
    /// Copied (or new) node, written at commit.
    Dirty(Box<MemNode>),
}

impl MemNode {
    fn from_node(n: &Node) -> MemNode {
        match &n.body {
            crate::page::Body::Leaf(entries) => MemNode::Leaf(
                entries
                    .iter()
                    .map(|e| MemEntry { key: e.key.clone(), value: MemValue::Stored(e.value.clone()) })
                    .collect(),
            ),
            crate::page::Body::Branch { keys, children } => MemNode::Branch {
                level: n.level,
                keys: keys.clone(),
                children: children.iter().map(|&c| Child::Clean(c)).collect(),
            },
        }
    }

    /// Encoded size in bytes (same as [`Node::encoded_size`] once written).
    fn size(&self) -> usize {
        match self {
            MemNode::Leaf(entries) => NODE_HEADER + entries.iter().map(MemEntry::size).sum::<usize>(),
            MemNode::Branch { keys, children, .. } => branch_size(keys, children.len()),
        }
    }
}

/// Result of applying operations to one node: zero nodes (it became empty),
/// one, or several after a split, with the separators between them.
struct Pieces {
    nodes: Vec<MemNode>,
    seps: Vec<Bytes>,
}

pub(crate) enum NewRoot {
    Empty,
    Tree { root: Child, level: u8 },
}

/// Splits items of the given sizes into consecutive groups that each fit in
/// `cap` bytes, as evenly as possible. Returns the group lengths.
pub(crate) fn plan_split(sizes: &[usize], cap: usize) -> Vec<usize> {
    let total: usize = sizes.iter().sum();
    if total <= cap {
        return vec![sizes.len()];
    }
    let target = total.div_ceil(total.div_ceil(cap));
    let mut lens = Vec::new();
    let (mut cur, mut n) = (0, 0);
    for &s in sizes {
        debug_assert!(s <= cap);
        if n > 0 && (cur + s > cap || cur >= target) {
            lens.push(n);
            (cur, n) = (0, 0);
        }
        cur += s;
        n += 1;
    }
    lens.push(n);
    lens
}

fn split_leaf(entries: Vec<MemEntry>) -> Pieces {
    let mut pieces = Pieces { nodes: Vec::new(), seps: Vec::new() };
    if entries.is_empty() {
        return pieces;
    }
    let sizes: Vec<usize> = entries.iter().map(MemEntry::size).collect();
    let mut it = entries.into_iter();
    for (g, len) in plan_split(&sizes, NODE_CAPACITY).into_iter().enumerate() {
        let group: Vec<MemEntry> = it.by_ref().take(len).collect();
        if g > 0 {
            pieces.seps.push(group[0].key.clone());
        }
        pieces.nodes.push(MemNode::Leaf(group));
    }
    pieces
}

fn split_branch(level: u8, keys: Vec<Bytes>, children: Vec<Child>) -> Pieces {
    let mut pieces = Pieces { nodes: Vec::new(), seps: Vec::new() };
    if children.is_empty() {
        return pieces;
    }
    // Item j = child j plus the separator before it (an over-estimate for
    // the first child of each group, whose separator moves up instead).
    let sizes: Vec<usize> = (0..children.len())
        .map(|j| crate::page::CHILD_REF_SIZE + if j == 0 { 0 } else { BRANCH_KEY_OVERHEAD + keys[j - 1].len() })
        .collect();
    let mut keys = keys.into_iter();
    let mut children = children.into_iter();
    for (g, len) in plan_split(&sizes, NODE_CAPACITY).into_iter().enumerate() {
        if g > 0 {
            pieces.seps.push(keys.next().expect("separator between groups"));
        }
        let ch: Vec<Child> = children.by_ref().take(len).collect();
        let ks: Vec<Bytes> = keys.by_ref().take(len - 1).collect();
        pieces.nodes.push(MemNode::Branch { level, keys: ks, children: ch });
    }
    pieces
}

/// Read side of a commit: loads and copies nodes, collects freed pages.
pub(crate) struct Txn<'a> {
    inner: &'a Inner,
    /// High-water mark of the tree being modified (bounds page ids read).
    page_count: u64,
    /// Pages of the previous tree that the new tree no longer uses.
    pub freed: Vec<PageId>,
    /// Tree nodes this commit replaced (evicted from the cache afterwards).
    pub replaced: Vec<PageRef>,
    /// Overflow values this commit released, as (first page, chain crc)
    /// (evicted from the cache afterwards).
    pub released_values: Vec<(PageId, u32)>,
    /// Change in the number of keys.
    pub entries_delta: i64,
}

impl<'a> Txn<'a> {
    pub(crate) fn new(inner: &'a Inner, page_count: u64) -> Self {
        Txn {
            inner,
            page_count,
            freed: Vec::new(),
            replaced: Vec::new(),
            released_values: Vec::new(),
            entries_delta: 0,
        }
    }

    fn read(&self, r: PageRef, level: u8) -> Result<Arc<Node>> {
        self.inner.read_node(r, level, self.page_count)
    }

    /// Copy-on-write: an owned copy of the node; its page becomes free.
    fn cow(&mut self, r: PageRef, level: u8) -> Result<MemNode> {
        let node = self.read(r, level)?;
        self.freed.push(r.id);
        self.replaced.push(r);
        Ok(MemNode::from_node(&node))
    }

    fn take(&mut self, c: Child, level: u8) -> Result<MemNode> {
        match c {
            Child::Clean(r) => self.cow(r, level),
            Child::Dirty(n) => Ok(*n),
        }
    }

    fn child_size(&self, c: &Child, level: u8) -> Result<usize> {
        match c {
            Child::Clean(r) => Ok(self.read(*r, level)?.encoded_size),
            Child::Dirty(n) => Ok(n.size()),
        }
    }

    /// Frees the overflow chain of a value that is overwritten or deleted.
    fn release_value(&mut self, v: MemValue) -> Result<()> {
        if let MemValue::Stored(Value::Overflow { len, crc, first }) = v {
            let ids = self.inner.overflow_pages(first, len, crc, self.page_count)?;
            self.freed.extend(ids);
            self.released_values.push((first, crc));
        }
        Ok(())
    }

    /// Applies `ops` to the tree rooted at `root` (of `height` levels).
    pub(crate) fn apply_root(&mut self, root: Option<PageRef>, height: u8, ops: &Ops) -> Result<NewRoot> {
        let (node, mut level) = match root {
            None => (MemNode::Leaf(Vec::new()), 0),
            Some(r) => (self.cow(r, height - 1)?, height - 1),
        };
        let mut pieces = self.apply(node, ops)?;
        // The root split: grow the tree until one node remains.
        while pieces.nodes.len() > 1 {
            level += 1;
            let children = pieces.nodes.into_iter().map(|n| Child::Dirty(Box::new(n))).collect();
            pieces = split_branch(level, pieces.seps, children);
        }
        let Some(node) = pieces.nodes.pop() else {
            return Ok(NewRoot::Empty);
        };
        // Collapse roots with a single child.
        let mut root = Child::Dirty(Box::new(node));
        while let Child::Dirty(n) = &root
            && let MemNode::Branch { children, .. } = &**n
            && children.len() == 1
        {
            let Child::Dirty(n) = root else { unreachable!() };
            let MemNode::Branch { mut children, .. } = *n else { unreachable!() };
            root = children.pop().expect("single child");
            level -= 1;
        }
        Ok(NewRoot::Tree { root, level })
    }

    fn apply(&mut self, node: MemNode, ops: &Ops) -> Result<Pieces> {
        match node {
            MemNode::Leaf(entries) => self.apply_leaf(entries, ops),
            MemNode::Branch { level, keys, children } => self.apply_branch(level, keys, children, ops),
        }
    }

    fn apply_leaf(&mut self, entries: Vec<MemEntry>, ops: &Ops) -> Result<Pieces> {
        let mut out = Vec::with_capacity(entries.len() + ops.len());
        let mut old = entries.into_iter().peekable();
        for &(key, op) in ops {
            while let Some(e) = old.next_if(|e| *e.key < *key) {
                out.push(e);
            }
            let existing = old.next_if(|e| *e.key == *key);
            let key = match existing {
                Some(e) => {
                    self.release_value(e.value)?;
                    if op.is_none() {
                        self.entries_delta -= 1;
                    }
                    e.key
                }
                None => {
                    if op.is_some() {
                        self.entries_delta += 1;
                    }
                    Bytes::new(key)
                }
            };
            if let Some(v) = op {
                let value = if inline_fits(key.len(), v.len()) {
                    MemValue::Stored(Value::Inline(Bytes::new(v)))
                } else {
                    MemValue::Large(v.to_vec())
                };
                out.push(MemEntry { key, value });
            }
        }
        out.extend(old);
        Ok(split_leaf(out))
    }

    fn apply_branch(&mut self, level: u8, keys: Vec<Bytes>, children: Vec<Child>, ops: &Ops) -> Result<Pieces> {
        let mut new_keys: Vec<Bytes> = Vec::with_capacity(keys.len() + 1);
        let mut new_children: Vec<Child> = Vec::with_capacity(children.len() + 1);
        // Children that were modified (only those are checked for underflow).
        let mut touched: Vec<bool> = Vec::with_capacity(children.len() + 1);
        let mut push = |sep: Option<Bytes>, child: Child, t: bool| {
            // The first child has no lower separator; when earlier children
            // were dropped, the separator of the new first child goes too.
            if !new_children.is_empty() {
                new_keys.push(sep.expect("non-first child has a separator"));
            }
            new_children.push(child);
            touched.push(t);
        };
        let mut rest = ops;
        let mut lower: Option<Bytes> = None;
        let mut keys = keys.into_iter();
        for child in children {
            let upper = keys.next();
            let cut = match &upper {
                Some(u) => rest.partition_point(|(k, _)| *k < &**u),
                None => rest.len(),
            };
            let (mine, tail) = rest.split_at(cut);
            rest = tail;
            if mine.is_empty() {
                push(lower.take(), child, false);
            } else {
                let node = self.take(child, level - 1)?;
                let pieces = self.apply(node, mine)?;
                let mut sep = lower.take();
                let mut seps = pieces.seps.into_iter();
                for (i, n) in pieces.nodes.into_iter().enumerate() {
                    if i > 0 {
                        sep = seps.next();
                    }
                    push(sep.take(), Child::Dirty(Box::new(n)), true);
                }
            }
            lower = upper;
        }
        // Merge underfull modified children with a neighbour when the
        // result fits in a page.
        let child_level = level - 1;
        let mut j = 0;
        while j < new_children.len() {
            if touched[j] && self.child_size(&new_children[j], child_level)? < UNDERFULL {
                if j + 1 < new_children.len()
                    && self.try_merge(child_level, &mut new_keys, &mut new_children, &mut touched, j)?
                {
                    continue;
                }
                if j > 0 && self.try_merge(child_level, &mut new_keys, &mut new_children, &mut touched, j - 1)? {
                    j -= 1;
                    continue;
                }
            }
            j += 1;
        }
        Ok(split_branch(level, new_keys, new_children))
    }

    /// Merges children `a` and `a + 1` if the result fits in one page.
    fn try_merge(
        &mut self,
        level: u8,
        keys: &mut Vec<Bytes>,
        children: &mut Vec<Child>,
        touched: &mut Vec<bool>,
        a: usize,
    ) -> Result<bool> {
        let sa = self.child_size(&children[a], level)?;
        let sb = self.child_size(&children[a + 1], level)?;
        let merged = sa + sb - NODE_HEADER + if level == 0 { 0 } else { BRANCH_KEY_OVERHEAD + keys[a].len() };
        if merged > PAGE_SIZE {
            return Ok(false);
        }
        let right = children.remove(a + 1);
        touched.remove(a + 1);
        let left = children.remove(a);
        let sep = keys.remove(a);
        let left = self.take(left, level)?;
        let right = self.take(right, level)?;
        let node = match (left, right) {
            (MemNode::Leaf(mut l), MemNode::Leaf(r)) => {
                l.extend(r);
                MemNode::Leaf(l)
            }
            (
                MemNode::Branch { level: lv, keys: mut lk, children: mut lc },
                MemNode::Branch { keys: rk, children: rc, .. },
            ) => {
                lk.push(sep);
                lk.extend(rk);
                lc.extend(rc);
                MemNode::Branch { level: lv, keys: lk, children: lc }
            }
            _ => return Err(Error::corruption("siblings of different kinds")),
        };
        debug_assert!(node.size() <= PAGE_SIZE);
        children.insert(a, Child::Dirty(Box::new(node)));
        touched[a] = true;
        Ok(true)
    }
}

/// Write side of a commit: assigns page ids and encodes pages. Performs no
/// I/O itself.
pub(crate) struct Writeout<'f> {
    free: &'f mut FreeState,
    /// Pages taken from the reusable set (for the free-list log).
    pub allocated: Vec<PageId>,
    pub writes: PageWrites,
    /// Freshly written nodes, inserted into the cache after the commit.
    pub nodes: Vec<(PageRef, Arc<Node>)>,
    pub overflow_pages: u64,
}

impl<'f> Writeout<'f> {
    pub(crate) fn new(free: &'f mut FreeState) -> Self {
        Writeout { free, allocated: Vec::new(), writes: Vec::new(), nodes: Vec::new(), overflow_pages: 0 }
    }

    fn alloc(&mut self) -> PageId {
        self.free.alloc(&mut self.allocated)
    }

    pub(crate) fn write_child(&mut self, c: Child) -> Result<PageRef> {
        match c {
            Child::Clean(r) => Ok(r),
            Child::Dirty(n) => self.write_node(*n),
        }
    }

    fn write_node(&mut self, n: MemNode) -> Result<PageRef> {
        let node = match n {
            MemNode::Leaf(entries) => Node::new_leaf(
                entries
                    .into_iter()
                    .map(|e| LeafEntry {
                        value: match e.value {
                            MemValue::Stored(v) => v,
                            MemValue::Large(bytes) => self.write_overflow(&bytes),
                        },
                        key: e.key,
                    })
                    .collect(),
            ),
            MemNode::Branch { level, keys, children } => {
                let children = children.into_iter().map(|c| self.write_child(c)).collect::<Result<Vec<_>>>()?;
                Node::new_branch(level, keys, children)
            }
        };
        let id = self.alloc();
        let page = node.encode(id);
        let r = PageRef { id, crc: crc32c::crc32c(&page) };
        // Cache the node decoded from its own page image, so that it does not
        // keep the (older) pages its entries were copied from alive. Decoding
        // only fails if the input tree was malformed (e.g. a crafted file
        // whose crcs are valid but whose keys violate separator bounds).
        let cached = Node::decode(&page, id, u64::MAX)
            .map_err(|e| Error::corruption(format!("commit produced an invalid node from malformed input: {e}")))?;
        debug_assert!(cached == node);
        self.writes.push((id, page));
        self.nodes.push((r, Arc::new(cached)));
        Ok(r)
    }

    /// Writes `bytes` as a chain of overflow pages. The returned reference's
    /// crc covers the full images of all pages of the chain, in order.
    fn write_overflow(&mut self, bytes: &[u8]) -> Value {
        let n = bytes.len().div_ceil(OVERFLOW_CAPACITY);
        let ids: Vec<PageId> = (0..n).map(|_| self.alloc()).collect();
        let mut crc = 0;
        for (i, chunk) in bytes.chunks(OVERFLOW_CAPACITY).enumerate() {
            let page = encode_overflow_page(ids.get(i + 1).copied().unwrap_or(0), chunk);
            crc = crc32c::crc32c_append(crc, &page);
            self.writes.push((ids[i], page));
        }
        self.overflow_pages += n as u64;
        Value::Overflow { len: bytes.len() as u32, crc, first: ids[0] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_split_boundaries() {
        assert_eq!(plan_split(&[], 100), vec![0]);
        assert_eq!(plan_split(&[50, 50], 100), vec![2], "exactly full stays one group");
        assert_eq!(plan_split(&[50, 51], 100), vec![1, 1]);
        assert_eq!(plan_split(&[10; 21], 100).iter().sum::<usize>(), 21);
        for n in 1..300 {
            for item in [1, 7, 33, 100] {
                let sizes = vec![item; n];
                let lens = plan_split(&sizes, 100);
                assert_eq!(lens.iter().sum::<usize>(), n);
                assert!(lens.iter().all(|&l| l > 0 && l * item <= 100), "{n} x {item}: {lens:?}");
                let total = n * item;
                if total > 100 {
                    // Never more than one group beyond the minimum.
                    assert!(lens.len() <= total.div_ceil(100) + 1, "{n} x {item}: {lens:?}");
                }
            }
        }
        // Two-way splits are balanced.
        let lens = plan_split(&[10; 12], 100);
        assert_eq!(lens, vec![6, 6]);
    }

    #[test]
    fn dedupe_keeps_last_operation_per_key() {
        let mut b = WriteBatch::new();
        b.put("b", "1").put("a", "x").delete("b").put("c", "3").put("b", "2").delete("c");
        let ops = dedupe(&b);
        assert_eq!(ops, vec![(&b"a"[..], Some(&b"x"[..])), (&b"b"[..], Some(&b"2"[..])), (&b"c"[..], None)]);
    }
}
