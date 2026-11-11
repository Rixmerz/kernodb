//! E5-01: the normalized fingerprint of a query (etapa5-acta.md §3, ADR-0015).
//!
//! A query is reduced to its **shape**: which conditions it sets and where a
//! literal stood, with every literal replaced by a typed placeholder
//! (`?text`, `?u32`, `?unit_id`). The shape of
//! `UnitFilter { unit_type: Some("invoice"), .. }` is
//! `units[unit_type=?text]`, whatever the type is. The shape text holds no
//! literal, but it is still query structure, so only the `diagnostic` level
//! shows it; every other level carries the [`Fingerprint`].
//!
//! The fingerprint is a **keyed**, versioned hash of the shape. A shape has
//! few possible values, so an unkeyed hash of it would fall to a dictionary
//! attack (ADR-0015); the key ([`FingerprintKey`]) is local to the process
//! and has no way out: no accessor, no `Debug` of its bytes, no serialization.
//! Fingerprints are therefore comparable only under the same key, which is
//! the point (acta §8, decision 4). The algorithm and the version are part of
//! the fingerprint, so a change of either can never be mistaken for a
//! change of query.
//!
//! # Hash input (version 1)
//!
//! `BLAKE3-keyed(key, "kerno.query-fingerprint.v1" ‖ 0x00 ‖ shape)`, first 16
//! bytes. The label separates this use of the key from any other and carries
//! the version; the shape text is injective over shapes, so two different
//! shapes never share an input.
//!
//! ponytail: the key is not zeroed on drop (no dependency for it) and
//! [`FingerprintKey::generate`] reads `/dev/urandom`, so it is Unix only.
//! Upgrade path: `getrandom` and `zeroize` if the observability key ever
//! leaves a single trusted process.

use std::fmt;
use std::io::Read;

use crate::assoc::{AssocDirection, AssocQuery};
use crate::index::UnitFilter;
use crate::types::{Direction, UnitId};

/// Registered with every fingerprint: the hash it was computed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FingerprintAlg {
    /// BLAKE3 in keyed mode, truncated to 16 bytes.
    Blake3Keyed,
}

/// The version of the shape grammar and of the hash input. Bumped when either
/// changes; the fixed vectors in `tests/fingerprint.rs` pin version 1.
pub const FINGERPRINT_VERSION: u8 = 1;

const LABEL: &[u8] = b"kerno.query-fingerprint.v1\0";
const DIGEST_LEN: usize = 16;

/// What a query asks for. Not a literal: there are two kinds, fixed by the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QueryKind {
    /// A [`UnitFilter`] over the active units of a snapshot.
    Units,
    /// The relations of one unit (`relations_of`).
    Relations,
    /// An [`AssocQuery`]: a multi-hop traversal from seed units (E7-02).
    Associative,
}

/// The relation query: the relations that start at or end at `unit`,
/// optionally of one type (`UnitSnapshot::relations_of`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationQuery {
    pub unit: UnitId,
    pub direction: Direction,
    pub relation_type: Option<String>,
}

/// A query reduced to its shape; see the [module documentation](self).
/// [`text`](Self::text) is the shape and is `diagnostic`-level material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedQuery {
    kind: QueryKind,
    text: String,
}

impl NormalizedQuery {
    /// The shape of a unit filter: `units[unit_type=?text,schema_version=?u32]`
    /// with only the conditions that are set, in that order.
    pub fn of_filter(filter: &UnitFilter) -> Self {
        let mut conditions = Vec::new();
        if filter.unit_type.is_some() {
            conditions.push("unit_type=?text");
        }
        if filter.schema_version.is_some() {
            conditions.push("schema_version=?u32");
        }
        NormalizedQuery { kind: QueryKind::Units, text: format!("units[{}]", conditions.join(",")) }
    }

    /// The shape of a relation query: `relations[out:unit=?unit_id,relation_type=?text]`.
    /// The direction is structure (two possible values), not a literal.
    pub fn of_relations(query: &RelationQuery) -> Self {
        let direction = match query.direction {
            Direction::Out => "out",
            Direction::In => "in",
        };
        let mut text = format!("relations[{direction}:unit=?unit_id");
        if query.relation_type.is_some() {
            text.push_str(",relation_type=?text");
        }
        text.push(']');
        NormalizedQuery { kind: QueryKind::Relations, text }
    }

    /// The shape of an associative query: `assoc[seeds=?unit_ids,dir=both,
    /// depth=?u32,limit=?u32,budget=?u64` followed by the conditions that are
    /// set: `,types=?texts`, `,window=?seqs`, `,kinds=?kinds`, `,actor=?bytes`,
    /// `,source=?bytes`. The direction is structure (three possible values);
    /// every set, number and identifier is a placeholder, so not even how
    /// many seeds, types or kinds the query has is in the shape.
    pub fn of_assoc(query: &AssocQuery) -> Self {
        let direction = match query.direction {
            AssocDirection::Out => "out",
            AssocDirection::In => "in",
            AssocDirection::Both => "both",
        };
        let mut text = format!("assoc[seeds=?unit_ids,dir={direction},depth=?u32,limit=?u32,budget=?u64");
        let conditions = [
            (query.relation_types.is_some(), ",types=?texts"),
            (query.window.is_some(), ",window=?seqs"),
            (query.provenance.kinds.is_some(), ",kinds=?kinds"),
            (query.provenance.actor.is_some(), ",actor=?bytes"),
            (query.provenance.source.is_some(), ",source=?bytes"),
        ];
        for (set, condition) in conditions {
            if set {
                text.push_str(condition);
            }
        }
        text.push(']');
        NormalizedQuery { kind: QueryKind::Associative, text }
    }

    pub fn kind(&self) -> QueryKind {
        self.kind
    }

    /// The shape text. It holds no literal; still, show it only at `diagnostic`.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The keyed, versioned hash of the shape.
    pub fn fingerprint(&self, key: &FingerprintKey) -> Fingerprint {
        let mut hasher = blake3::Hasher::new_keyed(&key.0);
        hasher.update(LABEL);
        hasher.update(self.text.as_bytes());
        let mut digest = [0u8; DIGEST_LEN];
        digest.copy_from_slice(&hasher.finalize().as_bytes()[..DIGEST_LEN]);
        Fingerprint { kind: self.kind, alg: FingerprintAlg::Blake3Keyed, version: FINGERPRINT_VERSION, digest }
    }
}

/// The fingerprint of a query shape: safe to put in an aggregate or a
/// fingerprint-level observation. It reveals the query kind (two values) and
/// nothing about literals, and without the key it cannot be matched to a
/// shape.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    kind: QueryKind,
    alg: FingerprintAlg,
    version: u8,
    digest: [u8; DIGEST_LEN],
}

impl Fingerprint {
    pub fn kind(&self) -> QueryKind {
        self.kind
    }

    pub fn alg(&self) -> FingerprintAlg {
        self.alg
    }

    pub fn version(&self) -> u8 {
        self.version
    }

    pub fn digest(&self) -> &[u8; DIGEST_LEN] {
        &self.digest
    }
}

/// `units/blake3-keyed/v1/<32 hex digits>`: algorithm and version registered
/// in the text itself.
impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            QueryKind::Units => "units",
            QueryKind::Relations => "relations",
            QueryKind::Associative => "assoc",
        };
        let alg = match self.alg {
            FingerprintAlg::Blake3Keyed => "blake3-keyed",
        };
        write!(f, "{kind}/{alg}/v{}/", self.version)?;
        self.digest.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

/// The local key of the fingerprint hash. It never leaves the process: it has
/// no accessor, and `Debug` hides its bytes. It is not comparable on purpose:
/// a secret gets no variable-time `==` (CWE-208); compare fingerprints.
#[derive(Clone)]
pub struct FingerprintKey([u8; 32]);

impl FingerprintKey {
    /// A key from given bytes: for reproducible tests, not for production.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        FingerprintKey(bytes)
    }

    /// A fresh random key from the operating system.
    pub fn generate() -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(FingerprintKey(bytes))
    }
}

impl fmt::Debug for FingerprintKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FingerprintKey(..)")
    }
}
