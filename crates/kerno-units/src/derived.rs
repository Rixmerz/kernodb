//! E8-02: the in-memory registry of derived data (etapa8-acta.md §3 and §5,
//! ADR-0018 option 2, invariants I-25 and I-38).
//!
//! A derived datum (an embedding, a summary, a cluster) is described by a
//! [`DerivedDescriptor`]: the exact sources it was computed from (unit and
//! revision), the method, its version, the parameters that change the result
//! and a hash of the input. The registry holds descriptors with a
//! [`DerivedState`] and keeps, by itself, the inverse index from a source to
//! the derived data that depend on it, which is what lets a later package
//! mark them stale by crossing a changed source with this index (E8-03).
//!
//! # Identity
//!
//! [`DerivedDescriptor::id`] hashes the sources (as a set), the method, the
//! version, the model profile and the parameters (as a map; ADR-0020 §C.1). The input hash is **not** part of
//! it: rebuilding the same derived datum over changed input is the same
//! derived datum. A new method or version has a new identity, so it is a new
//! entry and the old one is marked `superseded`, never rewritten.
//!
//! # States
//!
//! ```text
//! building -> ready | failed | stale | superseded
//! ready    -> stale | superseded
//! stale    -> building | superseded
//! failed   -> building | superseded
//! superseded: final
//! ```
//!
//! `stale` and `failed` are left only by starting a rebuild (`building`), so a
//! derived datum is never `ready` without having been built after its last
//! change (I-23). Any other move is [`DerivedError::IllegalTransition`].
//!
//! # Scope
//!
//! Memory only: the registry holds no bytes on disk; persisted vectors are
//! [`crate::vector_store`]'s (ADR-0020), which rebuilds this registry from the
//! database. The registry is plain data behind `&mut self`; a caller that shares it wraps
//! it in its own lock. Nothing here reads a database.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use crate::types::{RevisionId, UnitId};

/// Domain tag of the identity hash; bump it if the encoding below changes.
const IDENTITY_DOMAIN: &[u8] = b"kerno.derived.identity.v2";

/// The identity of a derived datum: sources, method, version and parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DerivedId(pub [u8; 32]);

/// Where a derived datum is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedState {
    Ready,
    Building,
    Stale,
    Failed,
    Superseded,
}

impl DerivedState {
    /// Whether the registry allows moving from `self` to `to`.
    pub fn can_become(self, to: DerivedState) -> bool {
        use DerivedState::*;
        matches!(
            (self, to),
            (Building, Ready | Failed | Stale | Superseded)
                | (Ready, Stale | Superseded)
                | (Stale | Failed, Building | Superseded)
        )
    }
}

/// Why a registry call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DerivedError {
    /// A descriptor needs at least one source.
    EmptySources,
    /// A descriptor needs a method name.
    EmptyMethod,
    /// A derived datum with this identity is already registered.
    Duplicate(DerivedId),
    /// No derived datum has this identity.
    Unknown(DerivedId),
    /// The state machine does not allow this move.
    IllegalTransition { id: DerivedId, from: DerivedState, to: DerivedState },
}

impl fmt::Display for DerivedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DerivedError::EmptySources => f.write_str("a derived datum needs at least one source"),
            DerivedError::EmptyMethod => f.write_str("a derived datum needs a method"),
            DerivedError::Duplicate(_) => f.write_str("a derived datum with this identity is already registered"),
            DerivedError::Unknown(_) => f.write_str("no derived datum has this identity"),
            DerivedError::IllegalTransition { from, to, .. } => {
                write!(f, "a derived datum cannot go from {from:?} to {to:?}")
            }
        }
    }
}

impl std::error::Error for DerivedError {}

/// What a derived datum is made of and how it was made.
#[derive(Clone, PartialEq, Eq)]
pub struct DerivedDescriptor {
    sources: BTreeSet<RevisionId>,
    method: String,
    version: u32,
    params: BTreeMap<String, String>,
    input_hash: [u8; 32],
    /// The model profile (ADR-0018, I-25), opaque text; empty when the
    /// datum has none. A persisted vector always has one (ADR-0020 §B.1).
    profile: String,
}

impl fmt::Debug for DerivedDescriptor {
    /// The method, the profile and the parameters are caller-supplied text:
    /// only their sizes are shown (I-37).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DerivedDescriptor")
            .field("sources", &self.sources)
            .field("method_len", &self.method.len())
            .field("version", &self.version)
            .field("profile_len", &self.profile.len())
            .field("params", &self.params.len())
            .finish_non_exhaustive()
    }
}

impl DerivedDescriptor {
    /// A descriptor over `sources` (a set: order and repeats do not matter).
    pub fn new(
        sources: impl IntoIterator<Item = RevisionId>,
        method: impl Into<String>,
        version: u32,
        params: BTreeMap<String, String>,
        input_hash: [u8; 32],
    ) -> Result<Self, DerivedError> {
        let sources: BTreeSet<RevisionId> = sources.into_iter().collect();
        if sources.is_empty() {
            return Err(DerivedError::EmptySources);
        }
        let method = method.into();
        if method.is_empty() {
            return Err(DerivedError::EmptyMethod);
        }
        Ok(DerivedDescriptor { sources, method, version, params, input_hash, profile: String::new() })
    }

    /// The same descriptor with a model profile, which is part of the
    /// identity (ADR-0020 §C.1).
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = profile.into();
        self
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The exact sources, in `(unit, revision)` order.
    pub fn sources(&self) -> impl Iterator<Item = RevisionId> + '_ {
        self.sources.iter().copied()
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    pub fn input_hash(&self) -> &[u8; 32] {
        &self.input_hash
    }

    /// The identity: equal for descriptors with the same sources, method,
    /// version, profile and parameters, whatever their input hash.
    pub fn id(&self) -> DerivedId {
        let mut h = blake3::Hasher::new();
        h.update(IDENTITY_DOMAIN);
        let mut field = |bytes: &[u8]| {
            h.update(&(bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        };
        field(&(self.sources.len() as u64).to_le_bytes());
        for s in &self.sources {
            field(&s.unit.0);
            field(&s.rev.0.to_le_bytes());
        }
        field(self.method.as_bytes());
        field(&self.version.to_le_bytes());
        field(self.profile.as_bytes());
        field(&(self.params.len() as u64).to_le_bytes());
        for (k, v) in &self.params {
            field(k.as_bytes());
            field(v.as_bytes());
        }
        DerivedId(*h.finalize().as_bytes())
    }
}

#[derive(Debug)]
struct Entry {
    descriptor: DerivedDescriptor,
    state: DerivedState,
}

/// The descriptors and states of the derived data, with the inverse index
/// source → derived data kept by the registry itself.
#[derive(Debug, Default)]
pub struct DerivedRegistry {
    entries: HashMap<DerivedId, Entry>,
    /// Every source revision of every entry, and nothing else. Ordered by
    /// `(unit, revision)` so that "all the revisions of a unit" is a range.
    by_source: BTreeMap<RevisionId, BTreeSet<DerivedId>>,
}

impl DerivedRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Registers a derived datum that starts to be built (`building`).
    pub fn register(&mut self, descriptor: DerivedDescriptor) -> Result<DerivedId, DerivedError> {
        let id = descriptor.id();
        if self.entries.contains_key(&id) {
            return Err(DerivedError::Duplicate(id));
        }
        for s in descriptor.sources() {
            self.by_source.entry(s).or_default().insert(id);
        }
        self.entries.insert(id, Entry { descriptor, state: DerivedState::Building });
        Ok(id)
    }

    pub fn descriptor(&self, id: &DerivedId) -> Option<&DerivedDescriptor> {
        self.entries.get(id).map(|e| &e.descriptor)
    }

    pub fn state(&self, id: &DerivedId) -> Option<DerivedState> {
        self.entries.get(id).map(|e| e.state)
    }

    /// Moves `id` to `to` if the state machine allows it; otherwise nothing
    /// changes.
    pub fn transition(&mut self, id: &DerivedId, to: DerivedState) -> Result<(), DerivedError> {
        let entry = self.entries.get_mut(id).ok_or(DerivedError::Unknown(*id))?;
        if !entry.state.can_become(to) {
            return Err(DerivedError::IllegalTransition { id: *id, from: entry.state, to });
        }
        entry.state = to;
        Ok(())
    }

    /// Removes a derived datum in any state and returns its descriptor.
    pub fn remove(&mut self, id: &DerivedId) -> Result<DerivedDescriptor, DerivedError> {
        let entry = self.entries.remove(id).ok_or(DerivedError::Unknown(*id))?;
        for s in entry.descriptor.sources() {
            if let Some(set) = self.by_source.get_mut(&s) {
                set.remove(id);
                if set.is_empty() {
                    self.by_source.remove(&s);
                }
            }
        }
        Ok(entry.descriptor)
    }

    /// The derived data computed from exactly this revision, by identity.
    pub fn dependents_of(&self, source: RevisionId) -> Vec<DerivedId> {
        self.by_source.get(&source).map(|s| s.iter().copied().collect()).unwrap_or_default()
    }

    /// The derived data computed from any revision of `unit`, by identity.
    pub fn dependents_of_unit(&self, unit: UnitId) -> Vec<DerivedId> {
        let lo = RevisionId { unit, rev: crate::types::RevNo(0) };
        let hi = RevisionId { unit, rev: crate::types::RevNo(u64::MAX) };
        let set: BTreeSet<DerivedId> = self.by_source.range(lo..=hi).flat_map(|(_, ids)| ids.iter().copied()).collect();
        set.into_iter().collect()
    }

    /// Whether the inverse index is exactly what the entries imply: every
    /// source of every entry points at it and nothing else is indexed.
    pub fn index_is_exact(&self) -> bool {
        let mut expected: BTreeMap<RevisionId, BTreeSet<DerivedId>> = BTreeMap::new();
        for (id, e) in &self.entries {
            for s in e.descriptor.sources() {
                expected.entry(s).or_default().insert(*id);
            }
        }
        expected == self.by_source
    }
}
