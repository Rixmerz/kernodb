//! Pluggable embedding providers (ROADMAP Phase B).
//!
//! The core runs no model and opens no network (I-31). What turns the text of
//! a unit into a vector is an [`EmbeddingProvider`] the caller registers on
//! the database with [`UnitDb::set_embedding_provider`]; [`DerivedDb::embed_units`]
//! then reads the text of the units, asks the provider once for all of them
//! and persists each vector through the existing vector store
//! ([`crate::vector_store`], ADR-0020), in one change set. Nothing here adds an
//! on-disk format: an embedding is a persisted vector whose recipe is
//! [`METHOD`] / [`METHOD_VERSION`], the provider's [`profile`] and its `dim`.
//!
//! ```text
//! unit payload (UTF-8) ──► provider.embed(texts) ──► put_vector ──► commit
//!                              (model or API)         (one change set)
//! ```
//!
//! # What ships
//!
//! * [`HashingProvider`]: deterministic feature hashing, no model, no network.
//!   For tests and for a first pass at wiring; its vectors capture shared
//!   words, not meaning.
//! * With the cargo feature `embed-openai` (off by default): `OpenAiCompatible`,
//!   a blocking client of the `POST {base_url}/embeddings` endpoint that
//!   OpenAI, Azure OpenAI, Ollama, llama.cpp's server, vLLM and LM Studio
//!   all speak. It is the way to use a remote API **or** a local model: only
//!   the base URL changes. HTTPS uses the platform's TLS and trust store
//!   (SecureTransport, SChannel or OpenSSL: on Linux, building with the feature
//!   needs the OpenSSL development files); no TLS library is vendored.
//!
//! # Plugging a model in
//!
//! No key is ever written in code or in the database. The key is read from
//! the environment when the provider is built:
//!
//! ```text
//! // cargo build --features kerno-units/embed-openai
//! //
//! //   export KERNO_EMBED_BASE_URL=https://api.openai.com/v1   # default
//! //   export KERNO_EMBED_MODEL=text-embedding-3-small
//! //   export KERNO_EMBED_DIM=1536
//! //   export KERNO_EMBED_API_KEY=...                          # your key
//! //
//! // A local model through Ollama needs no key:
//! //   export KERNO_EMBED_BASE_URL=http://localhost:11434/v1
//! //   export KERNO_EMBED_MODEL=nomic-embed-text
//! //   export KERNO_EMBED_DIM=768
//! let provider = kerno_units::embed::OpenAiCompatible::from_env()?;
//! db.set_embedding_provider(std::sync::Arc::new(provider));
//! let ids = derived.embed_units(&[unit], ChangeMeta::default())?;
//! ```
//!
//! `OpenAiCompatible::new(base_url, model, dim)` and `with_api_key_env("NAME")`
//! give the same without the fixed variable names. Any other model (an
//! in-process one, a gRPC service) is an `impl EmbeddingProvider` in the
//! caller's crate; the trait is three methods and needs nothing from here.
//!
//! # Contracts
//!
//! * **The unit is the truth.** A vector is derived data; the provider may be
//!   swapped, and units re-embedded, at any time. The recipe (provider
//!   [`profile`] and `dim`) is part of the datum's identity, so vectors of two
//!   models never share an id. **Give each model its own profile.**
//! * **Text units only.** The text of a unit is its payload as UTF-8; an empty
//!   payload (nothing to embed: a zero vector has no direction and APIs refuse
//!   an empty input) or one that is not UTF-8 is refused
//!   ([`EmbedError::NotText`]) and nothing is written. A unit that does not
//!   exist or is deleted is [`EmbedError::Derived`] with
//!   [`DerivedStaleError::MissingSource`], also writing nothing.
//! * **Provider output is checked, not trusted.** One vector per text, each of
//!   length [`dim`], every element finite; otherwise [`EmbedError::BadOutput`]
//!   and nothing is written.
//! * **The provider runs outside any transaction.** The call to a model or a
//!   network happens before the change set is opened.
//! * **Editing a unit stales its vector** (I-23); embedding the unit again
//!   writes a new datum over the new revision. Embedding the same revision
//!   with the same recipe twice is refused as a duplicate.
//! * **No content in errors (I-37).** Errors name units and counts, never
//!   text or vector elements.
//!
//! [`profile`]: EmbeddingProvider::profile
//! [`dim`]: EmbeddingProvider::dim

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError};

use kerno_core::contract::TxnEngine;

use crate::db::{ChangeMeta, UnitDb};
use crate::derived::DerivedId;
use crate::derived_stale::{DerivedDb, DerivedStaleError};
use crate::types::UnitId;
use crate::vector_store::VectorSpec;

mod hashing;
#[cfg(feature = "embed-openai")]
pub mod openai;

pub use hashing::HashingProvider;
#[cfg(feature = "embed-openai")]
pub use openai::OpenAiCompatible;

/// `method` of the descriptor of an embedding.
pub const METHOD: &str = "embedding";
/// `method_version` of the descriptor of an embedding.
pub const METHOD_VERSION: u32 = 1;

/// Turns text into vectors. Implemented by the caller (a model, an API);
/// registered with [`UnitDb::set_embedding_provider`].
pub trait EmbeddingProvider: Send + Sync {
    /// The length of every vector this provider returns: `1..=4096`.
    fn dim(&self) -> usize;

    /// One vector per text, in the order of `texts`, each of length
    /// [`dim`](Self::dim). Called with the texts of a whole batch at once, so
    /// a remote provider can send one request. An error message must not
    /// contain the texts (I-37) nor a credential.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// The model identity recorded in the descriptor (ADR-0018, I-25): opaque
    /// text of 1..=64 bytes, for instance `"openai/text-embedding-3-small"`.
    /// Two models of the same dimension must not share it, or a swap goes
    /// unnoticed. The default only suits a deployment that uses one model.
    fn profile(&self) -> &str {
        "default"
    }
}

/// Why an embedding call was refused.
#[derive(Debug)]
pub enum EmbedError {
    /// No provider is registered on the database.
    NoProvider,
    /// The unit has no payload, or it is not UTF-8.
    NotText(UnitId),
    /// The provider could not produce vectors (network, model, configuration).
    Provider(String),
    /// The provider answered, but not with one vector of length `dim` per text.
    BadOutput(String),
    /// The vector store refused (a duplicate, a missing unit, a storage error).
    Derived(DerivedStaleError),
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedError::NoProvider => f.write_str("no embedding provider is registered"),
            EmbedError::NotText(u) => write!(f, "unit {u} has no UTF-8 text payload"),
            EmbedError::Provider(m) => write!(f, "the embedding provider failed: {m}"),
            EmbedError::BadOutput(m) => write!(f, "the embedding provider returned unusable output: {m}"),
            EmbedError::Derived(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for EmbedError {}

impl From<DerivedStaleError> for EmbedError {
    fn from(e: DerivedStaleError) -> Self {
        EmbedError::Derived(e)
    }
}

impl From<crate::UnitError> for EmbedError {
    fn from(e: crate::UnitError) -> Self {
        EmbedError::Derived(e.into())
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// Registers the provider [`DerivedDb::embed_units`] will use, and returns
    /// the one it replaces. In memory only: register it again after every open.
    pub fn set_embedding_provider(&self, provider: Arc<dyn EmbeddingProvider>) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedder.lock().unwrap_or_else(PoisonError::into_inner).replace(provider)
    }

    /// The registered provider, if any.
    pub fn embedding_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedder.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl<E: TxnEngine> DerivedDb<E> {
    /// Embeds the text of the current revision of each of `units` with the
    /// registered provider and persists the vectors as `Ready` derived data,
    /// all in one change set. Returns their ids, in the order of `units`.
    /// See the module documentation for the contracts.
    pub fn embed_units(&self, units: &[UnitId], meta: ChangeMeta) -> Result<Vec<DerivedId>, EmbedError> {
        let provider = self.db.embedding_provider().ok_or(EmbedError::NoProvider)?;
        if units.is_empty() {
            return Ok(Vec::new());
        }
        let mut inputs = Vec::with_capacity(units.len());
        let mut texts = Vec::with_capacity(units.len());
        for &unit in units {
            let input = self.read_input(unit)?.ok_or(DerivedStaleError::MissingSource(unit))?;
            if input.payload.is_empty() {
                return Err(EmbedError::NotText(unit));
            }
            texts.push(String::from_utf8(input.payload.clone()).map_err(|_| EmbedError::NotText(unit))?);
            inputs.push(input);
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let dim = provider.dim();
        let vectors = provider.embed(&refs)?;
        if vectors.len() != units.len() {
            return Err(EmbedError::BadOutput(format!("{} vectors for {} texts", vectors.len(), units.len())));
        }
        if let Some(i) = vectors.iter().position(|v| v.len() != dim) {
            return Err(EmbedError::BadOutput(format!("vector {i} has {} elements, dim is {dim}", vectors[i].len())));
        }
        if let Some(i) = vectors.iter().position(|v| !v.iter().all(|x| x.is_finite())) {
            return Err(EmbedError::BadOutput(format!("vector {i} has a non-finite element")));
        }
        let params = BTreeMap::from([("dim".to_owned(), dim.to_string())]);
        let mut cs = self.begin_changeset(meta)?;
        let mut ids = Vec::with_capacity(units.len());
        for (input, vector) in inputs.iter().zip(vectors) {
            let spec = VectorSpec {
                source: input.id,
                method: METHOD.to_owned(),
                method_version: METHOD_VERSION,
                profile: provider.profile().to_owned(),
                params: params.clone(),
                vector,
            };
            ids.push(cs.put_vector(&spec)?);
        }
        cs.commit()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests;
