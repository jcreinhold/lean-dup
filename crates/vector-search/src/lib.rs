//! Hidden semantic/vector-search experiment slice.
//!
//! This crate is the only workspace crate that may combine embedding runtime,
//! vector corpus mechanics, search observations, and vector-validation
//! artifacts. Core symbolic search/eval/report crates remain unaware of this
//! experiment so the whole slice can be deleted without changing their APIs.

mod artifacts;
mod candidates;
mod documents;
mod eligibility;
mod leak_check;
mod scoring;
mod workload;

use std::path::{Path, PathBuf};

use lean_dup_diagnostics::progress::Reporter;
use serde::{Deserialize, Serialize};

pub const VECTOR_SEARCH_SCHEMA_VERSION: &str = "lean-dup.vector-search.v3";

/// One hidden vector-validation run request.
///
/// Callers name the workload and operator-controlled cache/artifact roots.
/// Model runtime details, vector-corpus storage, text formatting, and artifact
/// row construction belong to this crate and are intentionally not represented
/// in ordinary search/eval/report APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorValidationRequest {
    pub(crate) suite: String,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) mathlib_workspace: Option<PathBuf>,
    pub(crate) manual_module: Option<String>,
    pub(crate) profile_id: String,
    pub(crate) revision: Option<String>,
    pub(crate) acquisition_policy: VectorAcquisitionPolicy,
    pub(crate) input_format_id: String,
    pub(crate) document_policy_id: String,
    pub(crate) eligibility_policy_id: String,
    pub(crate) model_cache_root: Option<PathBuf>,
    pub(crate) text_vector_cache_root: Option<PathBuf>,
    pub(crate) corpus_cache_root: PathBuf,
    pub(crate) artifact_root: Option<PathBuf>,
    pub(crate) k_values: Vec<usize>,
    pub(crate) bounds: VectorValidationBounds,
}

impl VectorValidationRequest {
    /// Build a request with stable hidden-experiment defaults.
    pub fn new(suite: impl Into<String>, corpus_cache_root: impl Into<PathBuf>) -> Self {
        Self {
            suite: suite.into(),
            workspace: None,
            mathlib_workspace: None,
            manual_module: None,
            profile_id: "bge-small-en-v1.5".to_owned(),
            revision: None,
            acquisition_policy: VectorAcquisitionPolicy::CacheOnly,
            input_format_id: "asymmetric-query-document".to_owned(),
            document_policy_id: "name-and-statement".to_owned(),
            eligibility_policy_id: "actionable-public-statement".to_owned(),
            model_cache_root: None,
            text_vector_cache_root: None,
            corpus_cache_root: corpus_cache_root.into(),
            artifact_root: None,
            k_values: vec![1, 5, 10],
            bounds: VectorValidationBounds::default(),
        }
    }

    pub fn with_workspace(mut self, workspace: Option<PathBuf>) -> Self {
        self.workspace = workspace;
        self
    }

    pub fn with_mathlib_workspace(mut self, mathlib_workspace: Option<PathBuf>) -> Self {
        self.mathlib_workspace = mathlib_workspace;
        self
    }

    pub fn with_manual_module(mut self, manual_module: Option<String>) -> Self {
        self.manual_module = manual_module;
        self
    }

    pub fn with_profile(mut self, profile_id: impl Into<String>, revision: Option<String>) -> Self {
        self.profile_id = profile_id.into();
        self.revision = revision;
        self
    }

    pub fn with_acquisition_policy(mut self, acquisition_policy: VectorAcquisitionPolicy) -> Self {
        self.acquisition_policy = acquisition_policy;
        self
    }

    pub fn with_input_format(mut self, input_format_id: impl Into<String>) -> Self {
        self.input_format_id = input_format_id.into();
        self
    }

    pub fn with_document_policy(mut self, document_policy_id: impl Into<String>) -> Self {
        self.document_policy_id = document_policy_id.into();
        self
    }

    pub fn with_eligibility_policy(mut self, eligibility_policy_id: impl Into<String>) -> Self {
        self.eligibility_policy_id = eligibility_policy_id.into();
        self
    }

    pub fn with_model_cache_root(mut self, model_cache_root: Option<PathBuf>) -> Self {
        self.model_cache_root = model_cache_root;
        self
    }

    pub fn with_text_vector_cache_root(mut self, text_vector_cache_root: Option<PathBuf>) -> Self {
        self.text_vector_cache_root = text_vector_cache_root;
        self
    }

    pub fn with_artifact_root(mut self, artifact_root: Option<PathBuf>) -> Self {
        self.artifact_root = artifact_root;
        self
    }

    pub fn with_k_values(mut self, k_values: Vec<usize>) -> Self {
        self.k_values = k_values;
        self
    }

    pub fn with_bounds(mut self, bounds: VectorValidationBounds) -> Self {
        self.bounds = bounds;
        self
    }

    pub fn suite(&self) -> &str {
        &self.suite
    }

    fn artifact_root(&self) -> PathBuf {
        self.artifact_root.clone().unwrap_or_else(repo_root)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VectorAcquisitionPolicy {
    CacheOnly,
    DownloadIfMissing,
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
    Eval(#[from] lean_dup_eval::Error),

    #[error("{0}")]
    Index(#[from] lean_dup_index::Error),

    #[error("{0}")]
    Project(#[from] lean_dup_project::Error),

    #[error("{0}")]
    Search(#[from] lean_dup_search::Error),

    #[error("{0}")]
    VectorIndex(#[from] lean_dup_vector_index::VectorIndexError),

    #[error("{message}")]
    Io {
        message: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{message}")]
    Json {
        message: &'static str,
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("vector validation request is invalid: {message}")]
    InvalidRequest { message: String },

    #[error("vector validation artifact leak check failed: {message}")]
    Leak { message: String },
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
    workload::run(request, reporter)
}

fn validate_request(request: &VectorValidationRequest) -> Result<()> {
    for (field, value) in [
        ("suite id", request.suite.as_str()),
        ("profile id", request.profile_id.as_str()),
        ("input format id", request.input_format_id.as_str()),
        ("document policy id", request.document_policy_id.as_str()),
        ("eligibility policy id", request.eligibility_policy_id.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(Error::InvalidRequest {
                message: format!("{field} is empty"),
            });
        }
    }
    if request.bounds.max_declarations == 0 || request.bounds.max_queries == 0 {
        return Err(Error::InvalidRequest {
            message: "validation bounds must allow at least one declaration and one query".to_owned(),
        });
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("vector-search crate lives under crates/")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn public_entry_point_writes_non_saturated_fixture_artifact() {
        let temp = TempDir::new().expect("temp dir");
        let request = VectorValidationRequest::new("vector-fixture", temp.path().join("corpus"))
            .with_profile("fixture-deterministic-v1", None)
            .with_model_cache_root(Some(temp.path().join("model")))
            .with_text_vector_cache_root(Some(temp.path().join("vectors")))
            .with_artifact_root(Some(temp.path().join("artifacts")));

        let outcome = run_vector_validation(request, &mut Reporter::new(false, false)).expect("vector fixture");

        assert_eq!(outcome.schema_version, VECTOR_SEARCH_SCHEMA_VERSION);
        assert_eq!(outcome.status, VectorValidationStatus::Ok);
        let artifact = outcome.artifact.expect("artifact path");
        let json = std::fs::read_to_string(temp.path().join("artifacts").join(&artifact)).expect("read artifact");
        let report = serde_json::from_str::<Value>(&json).expect("valid vector artifact");
        assert!(json.contains("\"top_k_saturated\": false"));
        assert!(json.contains("\"vector_only_positives\""));
        assert!(!json.contains(&format!("{}:", "query")));
        assert!(!json.contains(&format!("{}:", "passage")));
        assert_eq!(report["schema_version"], VECTOR_SEARCH_SCHEMA_VERSION);
        assert_eq!(report["vector_candidates"]["top_k"], 32);
        assert!(
            report["vector_candidates"]["eligible_corpus_size"]
                .as_u64()
                .expect("eligible corpus size")
                > report["vector_candidates"]["top_k"].as_u64().expect("top k")
        );
        assert_eq!(report["vector_stage_metrics"]["top_k_saturation"]["found"], 0);
        assert_eq!(report["vector_stage_metrics"]["vector_only_positives"]["found"], 1);
        assert_eq!(report["vector_stage_metrics"]["symbolic_only_positives"]["found"], 1);
        assert_eq!(report["vector_stage_metrics"]["vector_only_hard_negatives"]["found"], 1);
        assert_eq!(
            report["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["generated"],
            1
        );
        assert_eq!(
            report["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["private"],
            1
        );
        let pairs = report["pairs"].as_array().expect("pair rows");
        assert_eq!(
            pairs
                .iter()
                .filter(|row| row["left"] == "VectorFixture.vector_only_match"
                    && row["right"] == "VectorFixture.vector_only_query")
                .count(),
            1
        );
        let vector_only = pairs
            .iter()
            .find(|row| {
                row["left"] == "VectorFixture.vector_only_match" && row["right"] == "VectorFixture.vector_only_query"
            })
            .expect("vector-only positive row");
        assert_eq!(vector_only["label_status"], "expanded-positive");
        assert_eq!(vector_only["symbolic_generated"], false);
        assert_eq!(vector_only["vector_generated"], true);
    }

    #[test]
    fn budget_exceeded_validation_writes_stable_partial_artifact() {
        let temp = TempDir::new().expect("temp dir");
        let request = VectorValidationRequest::new("vector-fixture", temp.path().join("corpus"))
            .with_profile("fixture-deterministic-v1", None)
            .with_model_cache_root(Some(temp.path().join("model")))
            .with_text_vector_cache_root(Some(temp.path().join("vectors")))
            .with_artifact_root(Some(temp.path().join("artifacts")))
            .with_bounds(VectorValidationBounds {
                max_declarations: 1,
                max_queries: 1,
                max_runtime_ms: u128::MAX,
                max_rss_bytes: u64::MAX,
            });

        let outcome = run_vector_validation(request, &mut Reporter::new(false, false)).expect("vector fixture");

        assert_eq!(outcome.status, VectorValidationStatus::Skipped);
        assert!(
            outcome
                .reason
                .as_deref()
                .expect("skip reason")
                .contains("query-count-budget-exceeded")
        );
        let artifact = outcome.artifact.expect("artifact path");
        let json = std::fs::read_to_string(temp.path().join("artifacts").join(&artifact)).expect("read artifact");
        let report = serde_json::from_str::<Value>(&json).expect("valid vector artifact");
        assert_eq!(report["status"], "skipped");
        assert_eq!(report["vector_candidates"]["status"], "skipped");
        assert_eq!(report["pairs"].as_array().expect("pair rows").len(), 0);
        assert_eq!(report["vector_search"], Value::Null);
    }
}
