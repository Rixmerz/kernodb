//! The self-check of the trigram index, run by `verify_units` (UNIT-52).
//!
//! The verifier walks every key once. It hands each head payload it meets
//! (`Active` head revision) to [`Check::expect`] and each record of the index to
//! [`Check::record`]; after the walk [`Check::findings`] reports what the records of each
//! type's index say against one another ([`crate::runs::Check`]: the manifest, the document
//! records, the number pages and the run pages) and compares, unit by unit, the trigrams the
//! payload calls for with those the runs hold for the unit's live number.
//! A unit is compared by the number of its trigrams and an order-free digest
//! of them, not by the trigrams themselves, so the check keeps a few bytes
//! per unit whatever the size of its payload.
//!
//! A finding names a key and a unit id, never a trigram: trigrams are the
//! content of the payloads.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{Gram, TAG_TRIGRAM, Trigrams, decode_prefix, grams};
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
    /// The index of each unit type that has a record, by type.
    types: BTreeMap<String, crate::runs::Check>,
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

    /// One record of the walk, met in key order.
    pub(crate) fn record(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        let r = self.keep(k, v);
        self.damaged |= r.is_err();
        r
    }

    fn keep(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        let (unit_type, prefix_len) = decode_prefix(k)?;
        let index = self
            .types
            .entry(unit_type.to_owned())
            .or_insert_with(|| crate::runs::Check::new(&k[..prefix_len]));
        let found = &mut self.found;
        index.record::<Trigrams>(k, v, &mut |list, unit, _, _| {
            let f = found.entry(unit).or_insert_with(|| Found {
                key: k.to_vec(),
                unit_type: unit_type.to_owned(),
                digest: (0, 0),
                two_types: false,
            });
            f.two_types |= f.unit_type != unit_type;
            add(&mut f.digest, [list[0], list[1], list[2]]);
        })
    }

    /// The first trigram record of the store, if it has any.
    pub(crate) fn first_key(&self) -> Option<&[u8]> {
        self.types.values().filter_map(crate::runs::Check::first_key).min()
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
        let mut out = Vec::new();
        if self.damaged {
            return out;
        }
        for index in self.types.values() {
            out.extend(index.findings("the trigram index"));
        }
        let wanted = |unit: UnitId| type_of(unit).filter(|t| indexed.contains(*t));
        let mut units: Vec<&UnitId> = self.expected.keys().collect();
        units.sort_unstable();
        for unit in units {
            let (Some(unit_type), Some(want)) = (wanted(*unit), self.expected.get(unit)) else { continue };
            match self.found.get(unit) {
                None if want.0 == 0 => {}
                None => out.push((head_key(*unit), format!("unit {unit} has content but is in no trigram run"))),
                Some(f) if f.two_types || f.unit_type != unit_type || f.digest != *want => {
                    out.push((f.key.clone(), format!("unit {unit}: its trigram postings disagree with its payload")));
                }
                Some(_) => {}
            }
        }
        for (unit, f) in self.found.iter().filter(|(u, _)| wanted(**u).is_none()) {
            out.push((
                f.key.clone(),
                format!("trigram runs hold unit {unit}, which is not an Active unit of a substring-indexed type"),
            ));
        }
        // A document record of a unit the index must not hold, or of one with no posting.
        for (unit_type, index) in &self.types {
            for unit in index.docs.keys() {
                if !self.found.contains_key(unit) {
                    out.push((
                        crate::runs::doc_key(&super::type_prefix(unit_type), *unit),
                        format!("unit {unit} has a trigram document record and no posting"),
                    ));
                }
            }
        }
        out
    }
}
