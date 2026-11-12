//! E8-06: consolidation with provenance (etapa8-acta.md §3, §5.2 and §5.3
//! point 5, decision D12, invariants I-31 and I-43; spec section 10).
//!
//! A *summary* is a derived datum (E8-02, ADR-0018 option 2) whose sources are
//! a non-empty set of units at exact revisions. It is not a new mechanism: it
//! is built, marked stale, rebuilt, superseded and deleted by the machinery of
//! [`crate::derived_stale`] (E8-03) and [`crate::suppression`] (E8-05), and
//! carries the same [`DerivedDescriptor`] (sources, method, version, parameters
//! and input hash). The only addition is a reserved parameter,
//! [`KIND_PARAM`] = `"summary"`, which is part of the identity and lets the
//! methods below refuse a datum that is not a summary.
//!
//! # The original is always readable (I-43)
//!
//! Nothing in this module writes the database: building, rebuilding,
//! superseding or losing a summary touches only the in-memory registry. No
//! read path returns a summary in place of a source: [`DerivedDb::get_unit`]
//! is the database's read and knows nothing of summaries, and the summary text
//! is reached only through [`DerivedDb::summary`], which serves a `ready`
//! summary and nothing else. The consequences for a summary follow the
//! registry: a change to a source marks it stale in the same commit, a new
//! method or version creates a new summary and marks the old one superseded,
//! and deleting any one of its sources deletes it in the same commit.
//!
//! # The adapter
//!
//! The text of a summary is produced by a [`Summarizer`], a trait with no
//! network and no model in this crate (I-31). The crate ships one
//! implementation, [`TruncatingSummarizer`], a fixed and documented reduction
//! meant for tests; a real summarizer lives outside the core (decision D12).
//!
//! # Persistence
//!
//! **Memory only.** ADR-0020 fixes bytes for vectors and for nothing else, and
//! persisting a summary would need a descriptor and a text record it does not
//! fix, so this package invents no on-disk format and the frozen layout does
//! not change. The registry is rebuilt on open by [`open_summaries`] from the
//! descriptors the caller still uses and the sources in the database (acta
//! §5.4): a summary whose sources are all current is recomputed and `ready`,
//! any other is `stale`.
//!
//! No source content or summary text appears in an error or a `Debug` output
//! (I-37).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use kerno_core::contract::TxnEngine;

use crate::db::UnitDb;
use crate::derived::{DerivedDescriptor, DerivedId};
use crate::derived_stale::{DerivedDb, DerivedStaleError, SourceInput};
use crate::types::UnitId;

/// The reserved parameter that marks a derived datum as a summary. It is part
/// of the identity, so a summary never shares an identity with another kind
/// of derived datum over the same units.
pub const KIND_PARAM: &str = "kerno.kind";
const KIND_SUMMARY: &str = "summary";

/// Produces the text of a summary from its sources. The core runs no model and
/// opens no network: an implementation is supplied by the caller (I-31).
pub trait Summarizer {
    /// The method name recorded in the descriptor; not empty.
    fn method(&self) -> &str;
    /// The version of the method; a new version is a new summary.
    fn version(&self) -> u32;
    /// The parameters that change the result. They must not use [`KIND_PARAM`].
    fn params(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    /// The text, from the sources in `(unit, revision)` order; `None` when it
    /// could not be produced (the summary is then `failed`).
    fn summarize(&self, sources: &[SourceInput]) -> Option<Vec<u8>>;
}

/// The summarizer the crate ships, for tests: the first `max_bytes` bytes of
/// each source payload (the whole payload if it is shorter), in source order,
/// joined by one `\n`. Deterministic, with no model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TruncatingSummarizer {
    max_bytes: usize,
    version: u32,
}

impl TruncatingSummarizer {
    pub const METHOD: &'static str = "test.truncate";

    pub fn new(max_bytes: usize, version: u32) -> Self {
        TruncatingSummarizer { max_bytes, version }
    }
}

impl Summarizer for TruncatingSummarizer {
    fn method(&self) -> &str {
        Self::METHOD
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn params(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("max_bytes".to_owned(), self.max_bytes.to_string())])
    }

    fn summarize(&self, sources: &[SourceInput]) -> Option<Vec<u8>> {
        let parts: Vec<&[u8]> = sources.iter().map(|s| &s.payload[..s.payload.len().min(self.max_bytes)]).collect();
        Some(parts.join(&b'\n'))
    }
}

/// Why a consolidation call was refused.
#[derive(Debug)]
pub enum ConsolidationError {
    /// A summary needs at least one source unit.
    NoSources,
    /// The summarizer's parameters use the reserved [`KIND_PARAM`].
    ReservedParam,
    /// The datum is not a summary.
    NotASummary(DerivedId),
    /// The summarizer's method, version or parameters are not those of the
    /// summary it was given to recompute.
    AdapterMismatch(DerivedId),
    /// The derived-data machinery refused.
    Derived(DerivedStaleError),
}

impl fmt::Display for ConsolidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConsolidationError::NoSources => f.write_str("a summary needs at least one source"),
            ConsolidationError::ReservedParam => f.write_str("a summarizer parameter uses a reserved name"),
            ConsolidationError::NotASummary(_) => f.write_str("the derived datum is not a summary"),
            ConsolidationError::AdapterMismatch(_) => {
                f.write_str("the summarizer is not the one that made this summary")
            }
            ConsolidationError::Derived(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ConsolidationError {}

impl From<DerivedStaleError> for ConsolidationError {
    fn from(e: DerivedStaleError) -> Self {
        ConsolidationError::Derived(e)
    }
}

/// The parameters of the descriptor: the summarizer's, plus the kind marker.
fn marked_params(s: &dyn Summarizer) -> Result<BTreeMap<String, String>, ConsolidationError> {
    let mut params = s.params();
    if params.insert(KIND_PARAM.to_owned(), KIND_SUMMARY.to_owned()).is_some() {
        return Err(ConsolidationError::ReservedParam);
    }
    Ok(params)
}

fn is_summary_descriptor(d: &DerivedDescriptor) -> bool {
    d.params().get(KIND_PARAM).map(String::as_str) == Some(KIND_SUMMARY)
}

/// Whether `d` was made by `s`: same method, version and parameters.
fn made_by(d: &DerivedDescriptor, s: &dyn Summarizer) -> Result<bool, ConsolidationError> {
    Ok(d.method() == s.method() && d.version() == s.version() && *d.params() == marked_params(s)?)
}

/// Opens the registry of summaries over `db` (see the module documentation,
/// "Persistence"). `known` are the descriptors of the summaries the caller
/// still uses; each must have been made by `summarizer`. The ids returned are
/// those of the known summaries dropped because a source is gone.
pub fn open_summaries<E: TxnEngine>(
    db: UnitDb<E>,
    known: impl IntoIterator<Item = DerivedDescriptor>,
    summarizer: &dyn Summarizer,
) -> Result<(DerivedDb<E>, Vec<DerivedId>), ConsolidationError> {
    let known: Vec<DerivedDescriptor> = known.into_iter().collect();
    for d in &known {
        if !is_summary_descriptor(d) {
            return Err(ConsolidationError::NotASummary(d.id()));
        }
        if !made_by(d, summarizer)? {
            return Err(ConsolidationError::AdapterMismatch(d.id()));
        }
    }
    Ok(DerivedDb::open(db, known, &|_, inputs| summarizer.summarize(inputs))?)
}

impl<E: TxnEngine> DerivedDb<E> {
    /// Builds a summary over the current revision of each of `units`.
    pub fn build_summary(
        &self,
        units: impl IntoIterator<Item = UnitId>,
        summarizer: &dyn Summarizer,
    ) -> Result<DerivedId, ConsolidationError> {
        let units: BTreeSet<UnitId> = units.into_iter().collect();
        if units.is_empty() {
            return Err(ConsolidationError::NoSources);
        }
        let params = marked_params(summarizer)?;
        Ok(self.build(units, summarizer.method(), summarizer.version(), params, &|_, inputs| {
            summarizer.summarize(inputs)
        })?)
    }

    /// Rebuilds a `stale` or `failed` summary from its sources that are still
    /// valid. `summarizer` must be the one that made it.
    pub fn rebuild_summary(
        &self,
        id: &DerivedId,
        summarizer: &dyn Summarizer,
    ) -> Result<DerivedId, ConsolidationError> {
        let desc = self.summary_descriptor(id)?;
        if !made_by(&desc, summarizer)? {
            return Err(ConsolidationError::AdapterMismatch(*id));
        }
        Ok(self.rebuild(id, &|_, inputs| summarizer.summarize(inputs))?)
    }

    /// Creates the summary of a new method, version or parameters over the
    /// still-valid sources of `old` and marks `old` `superseded`.
    pub fn supersede_summary(
        &self,
        old: &DerivedId,
        summarizer: &dyn Summarizer,
    ) -> Result<DerivedId, ConsolidationError> {
        self.summary_descriptor(old)?;
        let params = marked_params(summarizer)?;
        Ok(self.supersede(old, summarizer.method(), summarizer.version(), params, &|_, inputs| {
            summarizer.summarize(inputs)
        })?)
    }

    /// The text of a `ready` summary. A summary in any other state, and a
    /// datum that is not a summary, has none. This is not a read of a unit:
    /// [`DerivedDb::get_unit`] never returns it.
    pub fn summary(&self, id: &DerivedId) -> Option<Vec<u8>> {
        self.descriptor(id).filter(is_summary_descriptor)?;
        self.value(id)
    }

    /// Whether `id` is a registered summary.
    pub fn is_summary(&self, id: &DerivedId) -> bool {
        self.descriptor(id).is_some_and(|d| is_summary_descriptor(&d))
    }

    fn summary_descriptor(&self, id: &DerivedId) -> Result<DerivedDescriptor, ConsolidationError> {
        match self.descriptor(id) {
            Some(d) if is_summary_descriptor(&d) => Ok(d),
            Some(_) => Err(ConsolidationError::NotASummary(*id)),
            None => Err(DerivedStaleError::Derived(crate::derived::DerivedError::Unknown(*id)).into()),
        }
    }
}
