//! SST range-deletion block (v1.md §7.4):
//! `count varint ≥ 1 ‖ fragment*`,
//! `fragment := bytes(start) ‖ bytes(end) ‖ seq varint`.

use super::coding::{Reader, put_bytes, put_varint};
use super::limits::MAX_KEY_LEN;
use super::{FormatError, Result};

/// A range tombstone fragment `[start, end)` at `seq`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment<B = Vec<u8>> {
    pub start: B,
    pub end: B,
    pub seq: u64,
}

impl<B: AsRef<[u8]>> Fragment<B> {
    pub fn covers(&self, key: &[u8]) -> bool {
        self.start.as_ref() <= key && key < self.end.as_ref()
    }

    pub fn to_owned_fragment(&self) -> Fragment<Vec<u8>> {
        Fragment { start: self.start.as_ref().to_vec(), end: self.end.as_ref().to_vec(), seq: self.seq }
    }
}

pub fn encode_rangedel<B: AsRef<[u8]>>(fragments: &[Fragment<B>]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, fragments.len() as u64);
    for f in fragments {
        put_bytes(&mut out, f.start.as_ref());
        put_bytes(&mut out, f.end.as_ref());
        put_varint(&mut out, f.seq);
    }
    out
}

/// Parses a range-deletion payload. `count = 0`, `start >= end`, and a
/// fragment that does not start at or after the end of the previous one are
/// SST-F1; a payload cut short or with trailing bytes is SST-O4. The
/// sequence range is checked by [`check_fragment_seqs`].
pub fn decode_rangedel(payload: &[u8], base: u64) -> Result<Vec<Fragment<&[u8]>>> {
    let mut r = Reader::new(payload, base, "SST-O4");
    let count = r.varint()?;
    if count == 0 {
        return Err(FormatError::corruption("SST-F1", base, "range block with count 0"));
    }
    let mut out: Vec<Fragment<&[u8]>> = Vec::with_capacity((count as usize).min(payload.len() / 3));
    for _ in 0..count {
        let at = r.offset();
        let start = r.bytes(MAX_KEY_LEN)?;
        let end = r.bytes(MAX_KEY_LEN)?;
        let seq = r.varint()?;
        if start >= end {
            return Err(FormatError::corruption("SST-F1", at, "fragment with start >= end"));
        }
        if out.last().is_some_and(|prev| prev.end > start) {
            return Err(FormatError::corruption("SST-F1", at, "fragments unordered or overlapping"));
        }
        out.push(Fragment { start, end, seq });
    }
    r.finish()?;
    Ok(out)
}

/// SST-F1: every fragment's `seq` is within `[smallest_seq, largest_seq]`.
/// Errors carry `at`, the offset of the range block.
pub fn check_fragment_seqs<B: AsRef<[u8]>>(
    fragments: &[Fragment<B>],
    smallest_seq: u64,
    largest_seq: u64,
    at: u64,
) -> Result<()> {
    match fragments.iter().position(|f| f.seq < smallest_seq || f.seq > largest_seq) {
        Some(_) => Err(FormatError::corruption("SST-F1", at, "fragment sequence outside the table's range")),
        None => Ok(()),
    }
}

/// Highest sequence of the fragments covering `key` (fragments sorted and
/// disjoint, as SST-F1 requires).
pub fn covering_seq<B: AsRef<[u8]>>(fragments: &[Fragment<B>], key: &[u8]) -> Option<u64> {
    let i = fragments.partition_point(|f| f.end.as_ref() <= key);
    fragments.get(i).filter(|f| f.covers(key)).map(|f| f.seq)
}

/// SST-F2 (writer): turns possibly overlapping tombstones `(start, end, seq)`
/// into sorted, disjoint fragments, each carrying the highest sequence of the
/// tombstones covering it, with adjacent fragments of equal sequence merged.
/// Tombstones with `start >= end` are ignored.
pub fn fragment<B: AsRef<[u8]>>(tombstones: &[(B, B, u64)]) -> Vec<Fragment<Vec<u8>>> {
    let mut bounds: Vec<&[u8]> = Vec::with_capacity(2 * tombstones.len());
    for (s, e, _) in tombstones {
        if s.as_ref() < e.as_ref() {
            bounds.push(s.as_ref());
            bounds.push(e.as_ref());
        }
    }
    bounds.sort_unstable();
    bounds.dedup();
    // For every elementary interval [bounds[i], bounds[i + 1]), the highest
    // covering sequence (0 = uncovered).
    let mut best = vec![0u64; bounds.len().saturating_sub(1)];
    let mut sorted: Vec<(&[u8], &[u8], u64)> = tombstones
        .iter()
        .filter(|(s, e, _)| s.as_ref() < e.as_ref())
        .map(|(s, e, q)| (s.as_ref(), e.as_ref(), *q))
        .collect();
    // Highest sequence first: each interval is assigned once, by the first
    // tombstone that covers it.
    sorted.sort_unstable_by_key(|t| std::cmp::Reverse(t.2));
    let mut assigned = vec![false; best.len()];
    let mut next_free: Vec<usize> = (0..=best.len()).collect();
    fn find(next_free: &mut [usize], mut i: usize) -> usize {
        while next_free[i] != i {
            next_free[i] = next_free[next_free[i]];
            i = next_free[i];
        }
        i
    }
    for (s, e, q) in sorted {
        let lo = bounds.binary_search(&s).unwrap();
        let hi = bounds.binary_search(&e).unwrap();
        let mut i = find(&mut next_free, lo);
        while i < hi {
            best[i] = q;
            assigned[i] = true;
            next_free[i] = i + 1;
            i = find(&mut next_free, i + 1);
        }
    }
    let mut out: Vec<Fragment<Vec<u8>>> = Vec::new();
    for i in 0..best.len() {
        if !assigned[i] {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end.as_slice() == bounds[i] && last.seq == best[i] => last.end = bounds[i + 1].to_vec(),
            _ => out.push(Fragment { start: bounds[i].to_vec(), end: bounds[i + 1].to_vec(), seq: best[i] }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_testkit::rng::Rng;

    #[test]
    fn fragment_matches_brute_force() {
        let mut rng = Rng::new(77);
        for _ in 0..5_000 {
            let n = rng.below(6) as usize;
            let tombs: Vec<(Vec<u8>, Vec<u8>, u64)> =
                (0..n).map(|_| (vec![rng.below(12) as u8], vec![rng.below(12) as u8], 1 + rng.below(5))).collect();
            let frags = fragment(&tombs);
            // Sorted, disjoint, non-empty, merged.
            for w in frags.windows(2) {
                assert!(w[0].end <= w[1].start);
                assert!(!(w[0].end == w[1].start && w[0].seq == w[1].seq));
            }
            for k in 0..13u8 {
                let want = tombs.iter().filter(|(s, e, _)| s[..] <= [k][..] && [k][..] < e[..]).map(|t| t.2).max();
                assert_eq!(covering_seq(&frags, &[k]), want, "tombs {tombs:?} frags {frags:?} key {k}");
            }
            if !frags.is_empty() {
                let enc = encode_rangedel(&frags);
                let dec: Vec<_> = decode_rangedel(&enc, 0).unwrap().iter().map(Fragment::to_owned_fragment).collect();
                assert_eq!(dec, frags);
            }
        }
    }
}
