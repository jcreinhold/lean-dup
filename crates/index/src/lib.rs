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
    CacheStatus, DeclarationHandle, ExpectedIndexEntry, FingerprintKind, HydratedDeclaration,
    INDEX_DIAGNOSTIC_SCHEMA_VERSION, INDEX_SCHEMA_VERSION, IndexBuildKind, IndexBuildRequest, IndexProvenance,
    IndexProvenanceKind, IndexReference, IndexStore, IndexSummary, OpenedIndex, OpenedIndexFacts, ProbeCacheEntry,
    SemanticFeatureFanout, SemanticFeatureKey, SemanticFeatureMatches, SemanticFingerprintFeature, SemanticRoleFeature,
};

/// Release-facing index schema label.
///
/// The persisted store uses a more specific internal schema string, but
/// diagnostics expose only this storage-neutral label.
pub fn diagnostic_index_schema_version(schema: &str) -> String {
    if schema == INDEX_SCHEMA_VERSION {
        INDEX_DIAGNOSTIC_SCHEMA_VERSION.to_owned()
    } else if schema.starts_with("lean-dup.index.sqlite.") {
        schema.replacen("lean-dup.index.sqlite.", "lean-dup.index.", 1)
    } else if schema.contains("sqlite") {
        "incompatible-index-schema".to_owned()
    } else {
        schema.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{INDEX_DIAGNOSTIC_SCHEMA_VERSION, diagnostic_index_schema_version};

    #[test]
    fn diagnostic_index_schema_labels_do_not_expose_storage_backend() {
        assert_eq!(
            diagnostic_index_schema_version("lean-dup.index.sqlite.v2"),
            INDEX_DIAGNOSTIC_SCHEMA_VERSION
        );
        assert_eq!(
            diagnostic_index_schema_version("lean-dup.index.sqlite.v1"),
            "lean-dup.index.v1"
        );
        assert_eq!(
            diagnostic_index_schema_version("third-party-sqlite-schema"),
            "incompatible-index-schema"
        );
    }
}
