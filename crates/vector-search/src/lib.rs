//! Hidden semantic/vector-search experiment slice.
//!
//! This crate is the only workspace crate that may combine embedding runtime,
//! vector corpus mechanics, search observations, and vector-validation
//! artifacts. Core symbolic search/eval/report crates remain unaware of this
//! experiment so the whole slice can be deleted without changing their APIs.

use std::path::PathBuf;

use lean_dup_diagnostics::progress::Reporter;
use serde::{Deserialize, Serialize};

pub const VECTOR_SEARCH_SCHEMA_VERSION: &str = "lean-dup.vector-search.v3";

/// One hidden vector-validation run request.
///
/// Callers provide stable experiment ids, cache roots, and budgets. Model
/// runtime details, vector-corpus storage, text formatting, and artifact row
/// construction belong to this crate and are intentionally not represented in
/// ordinary search/eval/report APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorValidationRequest {
    pub suite: String,
    pub profile_id: String,
    pub input_format_id: String,
    pub document_policy_id: String,
    pub eligibility_policy_id: String,
    pub model_cache_root: Option<PathBuf>,
    pub text_vector_cache_root: Option<PathBuf>,
    pub corpus_cache_root: PathBuf,
    pub artifact_root: Option<PathBuf>,
    pub bounds: VectorValidationBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorValidationBounds {
    pub max_declarations: usize,
    pub max_queries: usize,
    pub max_runtime_ms: u128,
    pub max_rss_bytes: u64,
}

impl Default for VectorValidationBounds {
    fn default() -> Self {
        Self {
            max_declarations: 5_000,
            max_queries: 1_000,
            max_runtime_ms: 900_000,
            max_rss_bytes: 8 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorValidationOutcome {
    pub schema_version: &'static str,
    pub status: VectorValidationStatus,
    pub suite: String,
    pub artifact: Option<PathBuf>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VectorValidationStatus {
    Skipped,
    Failed,
    Ok,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Embedding(#[from] lean_dup_embedding::Error),

    #[error("{0}")]
    VectorIndex(#[from] lean_dup_vector_index::VectorIndexError),

    #[error("vector validation request is invalid: {message}")]
    InvalidRequest { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Run the hidden vector-search validation workflow.
///
/// This is the only public entry point for semantic/vector experiments. It is
/// deliberately coarse-grained: deleting the experiment removes this crate and
/// its embedding/vector-index dependencies without changing symbolic callers.
pub fn run_vector_validation(
    request: VectorValidationRequest,
    reporter: &mut Reporter,
) -> Result<VectorValidationOutcome> {
    validate_request(&request)?;
    reporter.event(
        "vector-search.validation",
        None,
        None,
        "vector validation is isolated from symbolic audit/eval",
    );
    Ok(VectorValidationOutcome {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        status: VectorValidationStatus::Skipped,
        suite: request.suite,
        artifact: None,
        reason: Some("vector validation slice is detached; command wiring moves here next".to_owned()),
    })
}

fn validate_request(request: &VectorValidationRequest) -> Result<()> {
    if request.suite.trim().is_empty() {
        return Err(Error::InvalidRequest {
            message: "suite id is empty".to_owned(),
        });
    }
    if request.profile_id.trim().is_empty() {
        return Err(Error::InvalidRequest {
            message: "profile id is empty".to_owned(),
        });
    }
    if request.input_format_id.trim().is_empty() {
        return Err(Error::InvalidRequest {
            message: "input format id is empty".to_owned(),
        });
    }
    if request.document_policy_id.trim().is_empty() {
        return Err(Error::InvalidRequest {
            message: "document policy id is empty".to_owned(),
        });
    }
    if request.eligibility_policy_id.trim().is_empty() {
        return Err(Error::InvalidRequest {
            message: "eligibility policy id is empty".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn public_entry_point_keeps_vector_status_out_of_core_crates() {
        let cache = TempDir::new().unwrap();
        let request = VectorValidationRequest {
            suite: "fixture".to_owned(),
            profile_id: "bge-small-en-v1.5".to_owned(),
            input_format_id: "asymmetric-query-document".to_owned(),
            document_policy_id: "name-and-statement".to_owned(),
            eligibility_policy_id: "actionable-public-statement".to_owned(),
            model_cache_root: None,
            text_vector_cache_root: None,
            corpus_cache_root: cache.path().join("corpus"),
            artifact_root: None,
            bounds: VectorValidationBounds::default(),
        };

        let outcome = run_vector_validation(request, &mut Reporter::new(false, false)).unwrap();

        assert_eq!(outcome.schema_version, VECTOR_SEARCH_SCHEMA_VERSION);
        assert_eq!(outcome.status, VectorValidationStatus::Skipped);
        assert!(outcome.reason.is_some());
    }
}
