//! Persistent declaration-vector corpus boundary.
//!
//! This crate owns vector-corpus persistence, provenance validation, nearest
//! declaration lookup, and backend diagnostics. Callers prepare a declaration
//! corpus and query an opaque handle; database layout, index parameters,
//! backend rows, and score conversion stay private.

mod error;
mod lancedb_backend;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use error::{Result, VectorIndexError};

pub const VECTOR_INDEX_SCHEMA_VERSION: &str = "lean-dup.vector-index.v1";

/// Provenance that decides whether a persisted vector corpus can be reused.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorCorpusProvenance {
    pub source_corpus_fingerprint: String,
    pub embedding_model_profile_id: String,
    pub embedding_model_fingerprint: String,
    pub embedding_input_format_id: String,
    pub embedding_input_format_version: String,
    pub document_policy_id: String,
    pub document_policy_version: String,
    pub vector_dimension: usize,
    pub normalization: String,
}

/// One declaration vector stored in a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VectorDeclaration {
    pub declaration_id: String,
    pub declaration_name: String,
    pub module_name: String,
    pub declaration_kind: String,
    pub content_hash: String,
    pub vector: Vec<f32>,
}

/// Request to prepare a persisted declaration-vector corpus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VectorCorpusPrepareRequest {
    pub cache_root: PathBuf,
    pub provenance: VectorCorpusProvenance,
    pub declarations: Vec<VectorDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct VectorCorpusBuildRequest {
    pub cache_root: PathBuf,
    pub provenance: VectorCorpusProvenance,
    pub declarations: Vec<VectorDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VectorCorpusOpenRequest {
    pub cache_root: PathBuf,
    pub provenance: VectorCorpusProvenance,
}

/// Stable corpus state reported without exposing backend mechanics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VectorCorpusStatus {
    Built,
    Reused,
    Missing,
    Stale,
    Unusable,
}

/// Stable corpus facts visible to search/eval artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorCorpusSummary {
    pub schema_version: String,
    pub status: VectorCorpusStatus,
    pub provenance: VectorCorpusProvenance,
    pub declaration_count: usize,
    pub vector_dimension: usize,
}

/// Build counters reported without backend row or index details.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorCorpusBuildCounters {
    pub input_declarations: usize,
    pub stored_declarations: usize,
    pub build_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorCorpusPrepareCounters {
    pub input_declarations: usize,
    pub stored_declarations: usize,
    pub build_ms: u128,
    pub open_ms: u128,
    pub previous_status: VectorCorpusStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VectorCorpusBuildOutput {
    pub summary: VectorCorpusSummary,
    pub previous_status: VectorCorpusStatus,
    pub counters: VectorCorpusBuildCounters,
}

/// Opaque handle to a prepared vector corpus.
#[derive(Debug, Clone)]
pub struct PreparedVectorCorpus {
    corpus: VectorCorpus,
    prepare_counters: VectorCorpusPrepareCounters,
}

impl PreparedVectorCorpus {
    /// Return stable facts for this corpus.
    pub fn summary(&self) -> &VectorCorpusSummary {
        self.corpus.summary()
    }

    /// Return build/reuse/open counters recorded during preparation.
    pub fn prepare_counters(&self) -> &VectorCorpusPrepareCounters {
        &self.prepare_counters
    }

    /// Query nearest declarations from this prepared corpus.
    ///
    /// Scores are normalized so larger values mean closer declarations; backend
    /// distance metrics and search parameters are private.
    ///
    /// # Errors
    ///
    /// Returns an error when the query vector or limit is invalid, or when the
    /// private backend cannot execute the nearest-declaration query.
    pub fn query(&self, request: &VectorCorpusQueryRequest) -> Result<VectorCorpusQueryOutput> {
        validate_query(self.corpus.summary.vector_dimension, request)?;
        lancedb_backend::query_vector_corpus(&self.corpus, request)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VectorCorpus {
    cache_root: PathBuf,
    summary: VectorCorpusSummary,
}

impl VectorCorpus {
    fn summary(&self) -> &VectorCorpusSummary {
        &self.summary
    }

    fn new(cache_root: PathBuf, summary: VectorCorpusSummary) -> Self {
        Self { cache_root, summary }
    }
}

/// Request nearest declarations for one query vector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VectorCorpusQueryRequest {
    pub query_vector: Vec<f32>,
    pub limit: usize,
}

/// One nearest declaration result. Higher scores are closer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VectorNearestDeclaration {
    pub declaration_id: String,
    pub declaration_name: String,
    pub module_name: String,
    pub declaration_kind: String,
    pub content_hash: String,
    pub score: f32,
}

/// Query counters reported without backend plan details.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorCorpusQueryCounters {
    pub requested_limit: usize,
    pub returned: usize,
    pub query_ms: u128,
}

/// Result of a nearest-declaration query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VectorCorpusQueryOutput {
    pub summary: VectorCorpusSummary,
    pub nearest: Vec<VectorNearestDeclaration>,
    pub counters: VectorCorpusQueryCounters,
}

/// Prepare a persisted declaration-vector corpus.
///
/// Matching provenance reuses the existing corpus. Missing, stale, or unusable
/// storage is rebuilt from the supplied declarations. The returned handle is
/// already opened for queries, so callers do not coordinate build/open order.
///
/// # Errors
///
/// Returns an error when provenance is incomplete, vectors are invalid, the
/// corpus cannot be persisted, or the prepared corpus cannot be opened.
pub fn prepare_vector_corpus(request: VectorCorpusPrepareRequest) -> Result<PreparedVectorCorpus> {
    validate_provenance(&request.provenance)?;
    validate_declarations(&request.provenance, &request.declarations)?;
    tracing::debug!(declarations = request.declarations.len(), "preparing vector corpus");
    let build = lancedb_backend::build_vector_corpus(VectorCorpusBuildRequest {
        cache_root: request.cache_root.clone(),
        provenance: request.provenance.clone(),
        declarations: request.declarations,
    })?;
    let open_started = std::time::Instant::now();
    let corpus = lancedb_backend::open_vector_corpus(VectorCorpusOpenRequest {
        cache_root: request.cache_root,
        provenance: request.provenance,
    })?;
    Ok(PreparedVectorCorpus {
        corpus,
        prepare_counters: VectorCorpusPrepareCounters {
            input_declarations: build.counters.input_declarations,
            stored_declarations: build.counters.stored_declarations,
            build_ms: build.counters.build_ms,
            open_ms: open_started.elapsed().as_millis(),
            previous_status: build.previous_status,
        },
    })
}

fn validate_provenance(provenance: &VectorCorpusProvenance) -> Result<()> {
    if provenance.vector_dimension == 0 {
        return Err(VectorIndexError::invalid("vector dimension must be nonzero"));
    }
    for (field, value) in [
        ("source corpus fingerprint", &provenance.source_corpus_fingerprint),
        ("embedding model profile id", &provenance.embedding_model_profile_id),
        ("embedding model fingerprint", &provenance.embedding_model_fingerprint),
        ("embedding input format id", &provenance.embedding_input_format_id),
        (
            "embedding input format version",
            &provenance.embedding_input_format_version,
        ),
        ("document policy id", &provenance.document_policy_id),
        ("document policy version", &provenance.document_policy_version),
        ("normalization", &provenance.normalization),
    ] {
        if value.trim().is_empty() {
            return Err(VectorIndexError::invalid(format!("{field} must be nonempty")));
        }
    }
    Ok(())
}

fn validate_declarations(provenance: &VectorCorpusProvenance, declarations: &[VectorDeclaration]) -> Result<()> {
    if declarations.is_empty() {
        return Err(VectorIndexError::invalid(
            "vector corpus requires at least one declaration",
        ));
    }
    for declaration in declarations {
        for (field, value) in [
            ("declaration id", &declaration.declaration_id),
            ("declaration name", &declaration.declaration_name),
            ("module name", &declaration.module_name),
            ("declaration kind", &declaration.declaration_kind),
            ("content hash", &declaration.content_hash),
        ] {
            if value.trim().is_empty() {
                return Err(VectorIndexError::invalid(format!("{field} must be nonempty")));
            }
        }
        validate_vector(
            provenance.vector_dimension,
            &provenance.normalization,
            &declaration.vector,
        )?;
    }
    Ok(())
}

fn validate_query(dimension: usize, request: &VectorCorpusQueryRequest) -> Result<()> {
    if request.limit == 0 {
        return Err(VectorIndexError::invalid("query limit must be nonzero"));
    }
    validate_vector(dimension, "l2", &request.query_vector)
}

fn validate_vector(dimension: usize, normalization: &str, vector: &[f32]) -> Result<()> {
    if vector.len() != dimension {
        return Err(VectorIndexError::invalid(format!(
            "vector dimension mismatch: expected {dimension}, got {}",
            vector.len()
        )));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(VectorIndexError::invalid("vector values must be finite"));
    }
    if normalization == "l2" {
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if !(0.98..=1.02).contains(&norm) {
            return Err(VectorIndexError::invalid("l2-normalized vectors must have unit length"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use tempfile::TempDir;

    use super::*;

    fn provenance() -> VectorCorpusProvenance {
        VectorCorpusProvenance {
            source_corpus_fingerprint: "fixture-source".to_owned(),
            embedding_model_profile_id: "bge-small-en-v1.5".to_owned(),
            embedding_model_fingerprint: "fixture-model".to_owned(),
            embedding_input_format_id: "asymmetric-query-document".to_owned(),
            embedding_input_format_version: "lean-dup.embedding-input-format.v1".to_owned(),
            document_policy_id: "name-and-statement".to_owned(),
            document_policy_version: "lean-dup.embedding-document.v1".to_owned(),
            vector_dimension: 2,
            normalization: "l2".to_owned(),
        }
    }

    fn declaration(id: &str, name: &str, vector: [f32; 2]) -> VectorDeclaration {
        VectorDeclaration {
            declaration_id: id.to_owned(),
            declaration_name: name.to_owned(),
            module_name: "Fixture.Module".to_owned(),
            declaration_kind: "theorem".to_owned(),
            content_hash: format!("hash-{id}"),
            vector: vector.to_vec(),
        }
    }

    fn declarations() -> Vec<VectorDeclaration> {
        vec![
            declaration("a", "Fixture.alpha", [1.0, 0.0]),
            declaration("b", "Fixture.beta", [0.0, 1.0]),
            declaration("c", "Fixture.gamma", [0.707_106_77, 0.707_106_77]),
        ]
    }

    #[test]
    fn corpus_build_open_reopen_and_query_are_deterministic() {
        let temp = TempDir::new().expect("create temp dir");
        let prepared = prepare_vector_corpus(VectorCorpusPrepareRequest {
            cache_root: temp.path().to_path_buf(),
            provenance: provenance(),
            declarations: declarations(),
        })
        .expect("build corpus");
        assert_eq!(prepared.summary().status, VectorCorpusStatus::Reused);
        assert_eq!(prepared.prepare_counters().previous_status, VectorCorpusStatus::Missing);
        assert_eq!(prepared.summary().declaration_count, 3);

        let reused = prepare_vector_corpus(VectorCorpusPrepareRequest {
            cache_root: temp.path().to_path_buf(),
            provenance: provenance(),
            declarations: declarations(),
        })
        .expect("reuse corpus");
        assert_eq!(reused.summary().status, VectorCorpusStatus::Reused);
        assert_eq!(reused.prepare_counters().previous_status, VectorCorpusStatus::Reused);

        assert_eq!(prepared.summary(), reused.summary());

        let first = prepared
            .query(&VectorCorpusQueryRequest {
                query_vector: vec![1.0, 0.0],
                limit: 2,
            })
            .expect("query corpus");
        let second = reused
            .query(&VectorCorpusQueryRequest {
                query_vector: vec![1.0, 0.0],
                limit: 2,
            })
            .expect("query reopened corpus");
        assert_eq!(first.nearest, second.nearest);
        assert_eq!(first.nearest[0].declaration_name, "Fixture.alpha");
        assert!(first.nearest[0].score > first.nearest[1].score);
    }

    #[test]
    fn stale_provenance_is_reported_before_reuse() {
        let temp = TempDir::new().expect("create temp dir");
        prepare_vector_corpus(VectorCorpusPrepareRequest {
            cache_root: temp.path().to_path_buf(),
            provenance: provenance(),
            declarations: declarations(),
        })
        .expect("build corpus");

        let mut stale = provenance();
        stale.embedding_model_fingerprint = "different-model".to_owned();
        let rebuilt = prepare_vector_corpus(VectorCorpusPrepareRequest {
            cache_root: temp.path().to_path_buf(),
            provenance: stale,
            declarations: declarations(),
        })
        .expect("stale corpus should rebuild");
        assert_eq!(rebuilt.prepare_counters().previous_status, VectorCorpusStatus::Stale);
    }

    #[test]
    fn invalid_vectors_and_limits_are_rejected() {
        let mut bad = declarations();
        bad[0].vector = vec![f32::NAN, 0.0];
        let error = prepare_vector_corpus(VectorCorpusPrepareRequest {
            cache_root: TempDir::new().expect("create temp dir").path().to_path_buf(),
            provenance: provenance(),
            declarations: bad,
        })
        .expect_err("nan vector should fail");
        assert!(matches!(error, VectorIndexError::InvalidRequest { .. }));

        let temp = TempDir::new().expect("create temp dir");
        let corpus = prepare_vector_corpus(VectorCorpusPrepareRequest {
            cache_root: temp.path().to_path_buf(),
            provenance: provenance(),
            declarations: declarations(),
        })
        .expect("build corpus");
        let zero_limit = corpus.query(&VectorCorpusQueryRequest {
            query_vector: vec![1.0, 0.0],
            limit: 0,
        });
        assert!(matches!(zero_limit, Err(VectorIndexError::InvalidRequest { .. })));
    }
}
