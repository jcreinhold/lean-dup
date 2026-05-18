//! Persisted declaration corpora and cache lifecycle.
//!
//! This crate owns SQLite indexes, cache keys, provenance metadata, latest
//! pointers, compatibility checks, and safe cleanup. Callers build/open/hydrate
//! indexes without learning table layouts or cache directory internals.

pub mod cache;
pub mod cache_lifecycle;
pub mod external_provenance;
pub mod index;

pub use index::*;
