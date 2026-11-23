//! The self-check of the trigram index, run by `verify_units` (UNIT-52).
//!
//! The verifier walks every key once. It hands each head payload it meets
//! (`Active` head revision) to [`Check::expect`] and each block of a posting
//! list to [`Check::record`]; after the walk [`Check::findings`] compares,
//! unit by unit, the trigrams the payload calls for with those the blocks
//! hold. The walk is in key order, so `record` also checks that the blocks of
//! a list hold consecutive ranges, each within its bound.
//! A unit is compared by the number of its trigrams and an order-free digest
//! of them, not by the trigrams themselves, so the check keeps a few bytes
//! per unit whatever the size of its payload.
//!
//! A finding names a key and a unit id, never a trigram: trigrams are the
//! content of the payloads.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{Gram, TAG_TRIGRAM, decode_block, decode_key, grams};
use crate::error::DecodeError;
use crate::types::UnitId;

/// How many trigrams, and the sum of their digests.
type Digest = (u32, u64);

fn add(d: &mut Digest, gram: Gram) {
    let n = u64::from(gram[0]) << 16 | u64::from(gram[1]) << 8 | u64::from(gram[2]);
    // A multiplication by an odd constant spreads the 24 bits, so two sets of trigrams
    // with the same sum are not found by removing one entry and adding its neighbour.
    *d = (d.0 + 1, d.1.wrapping_add((n + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
}

/// The entries the store has of one unit.
struct Found {
    /// The first of them, for a finding.
    key: Vec<u8>,
    unit_type: String,
    digest: Digest,
    /// An entry of this unit is under another unit type.
    two_types: bool,
}

#[derive(Default)]
pub(crate) struct Check {
    /// What the head payload of each `Active` unit calls for, whatever its type.
    expected: HashMap<UnitId, Digest>,
    found: BTreeMap<UnitId, Found>,
    /// The list key and bound of the last block recorded (`None`: the tail).
    last: Option<(Vec<u8>, Option<UnitId>)>,
    /// Blocks out of their range: `(key, detail)`.
    misplaced: Vec<(Vec<u8>, String)>,
    /// A record did not decode: that is a structure finding already, and the
    /// comparison would only repeat it.
    damaged: bool,
}

impl Check {
    /// Whether a key with this first byte is a trigram entry.
    pub(crate) fn owns(tag: Option<&u8>) -> bool {
        tag == Some(&TAG_TRIGRAM)
    }

    /// The head payload of an `Active` unit.
    pub(crate) fn expect(&mut self, unit: UnitId, payload: &[u8]) {
        let mut digest = (0, 0);
        for gram in grams(payload) {
            add(&mut digest, gram);
        }
        self.expected.insert(unit, digest);
    }

    /// One block of the walk, met in key order.
    pub(crate) fn record(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        let r = decode_key(k).and_then(|decoded| decode_block(v).map(|units| (decoded, units)));
        self.damaged |= r.is_err();
        let ((unit_type, gram, bound), units) = r?;
        // The list key is the block key without its bound.
        let list = k[..2 + unit_type.len() + 3].to_vec();
        let floor = match self.last.take() {
            Some((l, b)) if l == list => b,
            _ => None,
        };
        let (lowest, highest) = (units[0], units[units.len() - 1]);
        if floor.is_some_and(|f| lowest <= f) || bound.is_some_and(|b| highest > b) {
            self.misplaced.push((k.to_vec(), "a trigram block holds a unit outside its range".into()));
        }
        self.last = Some((list, bound));
        for unit in units {
            let found = self.found.entry(unit).or_insert_with(|| Found {
                key: k.to_vec(),
                unit_type: unit_type.clone(),
                digest: (0, 0),
                two_types: false,
            });
            found.two_types |= found.unit_type != unit_type;
            add(&mut found.digest, gram);
        }
        Ok(())
    }

    /// The first trigram entry of the store, if it has any.
    pub(crate) fn first_key(&self) -> Option<&[u8]> {
        self.found.values().map(|f| f.key.as_slice()).min()
    }

    /// `(key, detail)` of every disagreement. `type_of` is the unit type of an `Active` unit
    /// (`None` for any other), `indexed` the types with the `SUBSTRING` option, and `head_key`
    /// the key a missing entry is reported under.
    pub(crate) fn findings<'a>(
        &self,
        type_of: impl Fn(UnitId) -> Option<&'a str>,
        indexed: &BTreeSet<String>,
        head_key: impl Fn(UnitId) -> Vec<u8>,
    ) -> Vec<(Vec<u8>, String)> {
        let mut out = self.misplaced.clone();
        if self.damaged {
            return out;
        }
        let wanted = |unit: UnitId| type_of(unit).filter(|t| indexed.contains(*t));
        let mut units: Vec<&UnitId> = self.expected.keys().collect();
        units.sort_unstable();
        for unit in units {
            let (Some(unit_type), Some(want)) = (wanted(*unit), self.expected.get(unit)) else { continue };
            match self.found.get(unit) {
                None if want.0 == 0 => {}
                None => out.push((head_key(*unit), format!("unit {unit} has content but is in no trigram block"))),
                Some(f) if f.two_types || f.unit_type != unit_type || f.digest != *want => {
                    out.push((f.key.clone(), format!("unit {unit}: its trigram blocks disagree with its payload")));
                }
                Some(_) => {}
            }
        }
        for (unit, f) in self.found.iter().filter(|(u, _)| wanted(**u).is_none()) {
            out.push((
                f.key.clone(),
                format!("trigram blocks hold unit {unit}, which is not an Active unit of a substring-indexed type"),
            ));
        }
        out
    }
}
