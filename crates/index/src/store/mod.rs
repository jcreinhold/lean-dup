//! The semantic corpus: an on-disk `SQLite` index of declaration feature rows and
//! postings over opaque feature keys, read through the retrieval crate's
//! [`Corpus`](lean_semantic_search_retrieval::Corpus) trait.
//!
//! This code came from the `lean-semantic-search-store` crate at version 0.8.0.
//! `lean-dup` was that crate's only user, so the code now lives here, private to
//! `lean-dup-index`. The file format did not change: this module opens a corpus
//! that the crate wrote.
//!
//! Each index cache entry keeps two `SQLite` files whose rows share the opaque,
//! stable `declaration_id`. `index.sqlite` holds display rows, the probe cache,
//! and cache metadata. The corpus beside it holds only the semantic index:
//! postings over opaque keys, the document total, and each declaration's
//! contract [`DeclarationFeatureRow`](lean_semantic_search_contract::DeclarationFeatureRow).
//!
//! [`StoreBuilder`] builds a corpus and publishes it with one rename. [`Store`]
//! opens it read-only and implements `Corpus`, so retrieval ranks over the file
//! without loading it into memory. A corpus records the caller's opaque corpus
//! token and the retrieval policy version, but nothing reads them back: the
//! index metadata decides whether a cache entry is current.

#[cfg(test)]
mod fixtures;
mod read;
mod schema;
mod write;

pub(crate) use read::Store;
pub(crate) use write::{Ingest, StoreBuilder};

use std::fmt;

/// An error from building or opening a persisted corpus.
///
/// The [`Corpus`](lean_semantic_search_retrieval::Corpus) read methods are
/// infallible by trait contract — a `Store` validates its schema and metadata at
/// [`Store::open`], so subsequent reads degrade to empty results rather than
/// surfacing an error. This type covers only the fallible build and open steps.
#[derive(Debug)]
pub(crate) enum StoreError {
    /// A `SQLite` operation failed.
    Sqlite(rusqlite::Error),
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// A feature row could not be serialized to JSON.
    Json(serde_json::Error),
    /// The opened store's schema version is not the one this build understands.
    SchemaMismatch {
        /// The schema version stored in the file.
        found: String,
        /// The schema version this build writes and reads.
        expected: &'static str,
    },
    /// A required metadata fact was absent from the opened store.
    MissingMetadata(String),
    /// A stored metadata value was present but unparseable.
    Corrupt(&'static str),
    /// The builder was used after it had already been published.
    Closed,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Json(error) => write!(formatter, "json error: {error}"),
            Self::SchemaMismatch { found, expected } => {
                write!(formatter, "store schema version {found} is not the expected {expected}")
            }
            Self::MissingMetadata(key) => write!(formatter, "store is missing required metadata: {key}"),
            Self::Corrupt(key) => write!(formatter, "store metadata value is corrupt: {key}"),
            Self::Closed => write!(formatter, "store builder has already been published"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::SchemaMismatch { .. } | Self::MissingMetadata(_) | Self::Corrupt(_) | Self::Closed => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
