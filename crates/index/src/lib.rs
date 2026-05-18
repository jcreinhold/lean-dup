//! Persisted declaration corpora and cache lifecycle.
//!
//! This crate owns SQLite indexes, cache keys, provenance metadata, latest
//! pointers, compatibility checks, and safe cleanup. Callers build/open/hydrate
//! indexes without learning table layouts or cache directory internals.

mod cache;
mod cache_lifecycle;
mod error;
mod external_provenance;
mod index;

pub use cache::{CACHE_KEY_VERSION, CacheFacts, cache_root, resolve_cache, workspace_fingerprint};
pub use cache_lifecycle::{
    CacheCleanupEntry, CacheCleanupReport, CacheDiagnostics, CacheEntryDiagnostics, CacheEntryStatus,
    CacheLabelDiagnostics, CacheLatestDiagnostics, CacheLatestStatus, CleanupPolicy, cleanup_cache, diagnose_cache,
};
pub use error::{Error, Result};
pub use external_provenance::{
    ComparisonEvidenceMode, ComparisonEvidencePolicy, ComparisonProvenance, ComparisonProvenanceReport,
    resolve as resolve_comparison_provenance,
};
pub use index::{
    CacheStatus, DeclarationHandle, ExpectedIndexEntry, FingerprintKind, HydratedDeclaration, INDEX_SCHEMA_VERSION,
    IndexBuildKind, IndexBuildRequest, IndexProvenance, IndexProvenanceKind, IndexReference, IndexStore, IndexSummary,
    OpenedIndex, OpenedIndexFacts, ProbeCacheEntry, SemanticFeatureFanout, SemanticFeatureKey, SemanticFeatureMatches,
    SemanticFingerprintFeature, SemanticRoleFeature,
};
