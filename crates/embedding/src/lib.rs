//! Local text embedding boundary for search-quality experiments.
//!
//! This crate owns model profiles, explicit model acquisition, CPU embedding,
//! normalization, batching, vector caching, and embedding runtime counters.
//! Callers provide declaration-document strings and receive stable
//! model/cache/runtime facts; they do not learn Hugging Face cache layout,
//! model filenames, FastEmbed/ONNX internals, or vector-cache storage.

mod error;
mod fastembed_backend;
mod profiles;
mod vector_cache;

pub use error::{Error, Result};
use std::path::PathBuf;

use hf_hub::Cache;
use profiles::{BGE_SMALL_MODEL_ID, resolve_profile};
use serde::{Deserialize, Serialize};

/// Version of the declaration-document input contract consumed by embeddings.
pub const EMBEDDING_INPUT_POLICY_VERSION: &str = "lean-dup.embedding-document.v1";

/// A model the embedding crate can prepare and run.
///
/// Identified by model id, not filesystem path: model preparation goes through
/// an explicit cache policy ([`EmbeddingAcquisitionPolicy`]). Default audit
/// paths must not download or load a model implicitly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingModelSpec {
    pub id: String,
    pub revision: Option<String>,
}

impl EmbeddingModelSpec {
    /// The default model id for hidden embedding experiments.
    pub fn default_experiment_model() -> Self {
        Self {
            id: BGE_SMALL_MODEL_ID.to_owned(),
            revision: None,
        }
    }
}

/// Whether model preparation may use the network.
///
/// `CacheOnly` validates already-prepared local files and never downloads.
/// `DownloadIfMissing` is intended only for explicit developer preparation or
/// hidden experiment commands that requested acquisition.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingAcquisitionPolicy {
    CacheOnly,
    DownloadIfMissing,
}

/// Request to validate or prepare a local embedding model.
///
/// The optional cache root selects the Hugging Face cache root for this
/// operation. When it is absent, the embedding crate resolves the local cache
/// policy internally from the standard Hugging Face environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingPrepareRequest {
    pub model: EmbeddingModelSpec,
    pub acquisition_policy: EmbeddingAcquisitionPolicy,
    pub cache_root: Option<PathBuf>,
}

impl EmbeddingPrepareRequest {
    /// Build the default explicit preparation request used by the hidden CLI
    /// command.
    pub fn default_download_request() -> Self {
        Self {
            model: EmbeddingModelSpec::default_experiment_model(),
            acquisition_policy: EmbeddingAcquisitionPolicy::DownloadIfMissing,
            cache_root: None,
        }
    }
}

/// Stable identity facts for a resolved embedding model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingModelSummary {
    pub id: String,
    pub revision: Option<String>,
    pub fingerprint: Option<String>,
    pub profile_id: String,
    pub backend_family: String,
    pub dimension: usize,
    pub input_roles: Vec<String>,
}

/// Prepared-model cache state visible to callers and artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingCacheStatus {
    NotPrepared,
    Prepared,
    Unusable,
    Skipped,
}

/// Stable cache facts without exposing model-file layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingCacheSummary {
    pub status: EmbeddingCacheStatus,
    pub model: EmbeddingModelSpec,
    pub cache_label: Option<String>,
}

/// Stable prepared-model role used in preparation diagnostics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingModelFileRole {
    Config,
    Tokenizer,
    TokenizerConfig,
    SpecialTokens,
    RuntimeModel,
}

/// Local status for one required model-file role.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingModelFileState {
    Present,
    Downloaded,
    Missing,
    Unavailable,
}

/// Prepared-state fact for a required model-file role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingRequiredFileStatus {
    pub role: EmbeddingModelFileRole,
    pub state: EmbeddingModelFileState,
    pub bytes: Option<u64>,
    pub reason: Option<String>,
}

/// Result of an explicit model preparation or validation request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingPrepareResult {
    pub model: EmbeddingModelSummary,
    pub cache: EmbeddingCacheSummary,
    pub acquisition_policy: EmbeddingAcquisitionPolicy,
    pub elapsed_ms: u128,
    pub required_files: Vec<EmbeddingRequiredFileStatus>,
    pub total_bytes: Option<u64>,
    pub reasons: Vec<String>,
}

/// Public contract for how declaration documents are converted to model input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingInputPolicy {
    pub policy_id: String,
    pub version: String,
    pub includes_declaration_name: bool,
    pub includes_normalized_statement: bool,
    pub uses_informal_text_when_available: bool,
}

impl Default for EmbeddingInputPolicy {
    fn default() -> Self {
        Self {
            policy_id: "name-and-formal-statement".to_owned(),
            version: EMBEDDING_INPUT_POLICY_VERSION.to_owned(),
            includes_declaration_name: true,
            includes_normalized_statement: true,
            uses_informal_text_when_available: false,
        }
    }
}

/// The semantic role of a text input for profile-specific model wrapping.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingInputRole {
    Document,
    Query,
}

/// One text input for local embedding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextEmbeddingInput {
    pub id: String,
    pub text: String,
}

/// Batch request for local text embeddings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextEmbeddingBatchRequest {
    pub model: EmbeddingModelSpec,
    pub role: EmbeddingInputRole,
    pub input_policy: EmbeddingInputPolicy,
    pub inputs: Vec<TextEmbeddingInput>,
    pub model_cache_root: Option<PathBuf>,
    pub vector_cache_root: Option<PathBuf>,
}

/// A normalized embedding vector for one input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingVector {
    pub input_id: String,
    pub values: Vec<f32>,
}

/// Runtime counters that can be reported without exposing implementation
/// details.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingRuntimeCounters {
    pub model_load_ms: u128,
    pub inference_ms: u128,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub batch_count: u64,
    pub peak_rss_bytes: Option<u64>,
}

/// Batch embedding result visible to eval artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextEmbeddingBatchResult {
    pub model: EmbeddingModelSummary,
    pub cache: EmbeddingCacheSummary,
    pub input_policy: EmbeddingInputPolicy,
    pub vector_dimension: usize,
    pub runtime: EmbeddingRuntimeCounters,
    pub vectors: Vec<EmbeddingVector>,
}

/// Embed declaration-document strings locally using a prepared CPU model.
///
/// This operation is cache-only: it validates model files prepared by
/// `prepare_embedding_model` and never downloads. Callers receive normalized
/// vectors and stable runtime counters without learning tokenizer, tensor,
/// pooling, or vector-cache layout.
pub fn embed_text_batch(request: TextEmbeddingBatchRequest) -> Result<TextEmbeddingBatchResult> {
    let profile = resolve_profile(&request.model)?;
    ensure_profile_enabled(profile)?;
    fastembed_backend::embed_text_batch(request, profile)
}

/// Validate or prepare an embedding model in the local Hugging Face cache.
///
/// This is the only crate-root acquisition capability. Callers choose whether
/// the operation may download; file names, snapshot layout, and cache probing
/// remain private to this crate.
pub fn prepare_embedding_model(request: EmbeddingPrepareRequest) -> Result<EmbeddingPrepareResult> {
    validate_prepare_request(&request)?;
    let profile = resolve_profile(&request.model)?;
    ensure_profile_enabled(profile)?;
    fastembed_backend::prepare_embedding_model(request, profile)
}

fn ensure_profile_enabled(profile: profiles::ModelProfile) -> Result<()> {
    if profile.support_status == profiles::ProfileSupportStatus::UnsupportedNotEnabled {
        return Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile:not-enabled".to_owned(),
        });
    }
    Ok(())
}

impl EmbeddingModelFileRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Tokenizer => "tokenizer",
            Self::TokenizerConfig => "tokenizer-config",
            Self::SpecialTokens => "special-tokens",
            Self::RuntimeModel => "runtime-model",
        }
    }
}

fn validate_prepare_request(request: &EmbeddingPrepareRequest) -> Result<()> {
    if request.model.id.trim().is_empty() {
        return Err(Error::EmptyModelId);
    }
    if request
        .model
        .revision
        .as_deref()
        .is_some_and(|revision| revision.trim().is_empty())
    {
        return Err(Error::EmptyRevision);
    }
    Ok(())
}

pub(crate) fn resolve_hf_cache_root(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    if let Some(path) = std::env::var_os("HF_HUB_CACHE") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("HF_HOME") {
        return PathBuf::from(path).join("hub");
    }
    Cache::from_env().path().clone()
}

pub(crate) fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_is_the_prompt_35_candidate() {
        let model = EmbeddingModelSpec::default_experiment_model();
        assert_eq!(model.id, "BAAI/bge-small-en-v1.5");
        assert_eq!(model.revision, None);
    }

    #[test]
    fn supported_bge_profile_reports_stable_summary() -> Result<()> {
        let model = EmbeddingModelSpec::default_experiment_model();
        let profile = resolve_profile(&model)?;
        let summary = profile.summary(&model, Some("fingerprint".to_owned()));
        assert_eq!(summary.profile_id, "bge-small-en-v1.5");
        assert_eq!(summary.backend_family, "fastembed");
        assert_eq!(summary.dimension, 384);
        assert!(summary.input_roles.contains(&"document".to_owned()));
        assert!(summary.input_roles.contains(&"query".to_owned()));
        Ok(())
    }

    #[test]
    fn unsupported_model_fails_before_cache_or_runtime() {
        let model = EmbeddingModelSpec {
            id: "arbitrary/model".to_owned(),
            revision: None,
        };
        assert!(matches!(
            prepare_embedding_model(EmbeddingPrepareRequest {
                model,
                acquisition_policy: EmbeddingAcquisitionPolicy::CacheOnly,
                cache_root: None,
            }),
            Err(Error::UnsupportedModel { reason }) if reason == "unsupported-model-profile"
        ));
    }

    #[test]
    fn input_policy_names_stable_contract() {
        let policy = EmbeddingInputPolicy::default();
        assert_eq!(policy.policy_id, "name-and-formal-statement");
        assert_eq!(policy.version, "lean-dup.embedding-document.v1");
        assert!(policy.includes_declaration_name);
        assert!(policy.includes_normalized_statement);
        assert!(!policy.uses_informal_text_when_available);
    }

    #[test]
    fn profile_wrapping_is_role_aware_and_private() -> Result<()> {
        let model = EmbeddingModelSpec::default_experiment_model();
        let profile = resolve_profile(&model)?;
        let document = profile.wrap_text(EmbeddingInputRole::Document, "P = Q");
        let query = profile.wrap_text(EmbeddingInputRole::Query, "P = Q");
        assert_ne!(document, query);
        assert!(!document.is_empty());
        assert!(!query.is_empty());
        Ok(())
    }

    #[test]
    fn runtime_is_cache_only_and_requires_prepared_model() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let request = TextEmbeddingBatchRequest {
            model: EmbeddingModelSpec::default_experiment_model(),
            role: EmbeddingInputRole::Document,
            input_policy: EmbeddingInputPolicy::default(),
            inputs: vec![TextEmbeddingInput {
                id: "Tiny.same_left".to_owned(),
                text: "name: Tiny.same_left".to_owned(),
            }],
            model_cache_root: Some(temp.path().to_path_buf()),
            vector_cache_root: Some(temp.path().join("vectors")),
        };
        assert!(matches!(embed_text_batch(request), Err(Error::ModelNotPrepared { .. })));
        Ok(())
    }

    #[test]
    fn default_prepare_request_downloads_only_when_explicit() {
        let request = EmbeddingPrepareRequest::default_download_request();
        assert_eq!(request.model.id, "BAAI/bge-small-en-v1.5");
        assert_eq!(
            request.acquisition_policy,
            EmbeddingAcquisitionPolicy::DownloadIfMissing
        );
    }

    #[test]
    fn cache_only_empty_cache_reports_not_prepared() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let result = prepare_embedding_model(EmbeddingPrepareRequest {
            model: EmbeddingModelSpec::default_experiment_model(),
            acquisition_policy: EmbeddingAcquisitionPolicy::CacheOnly,
            cache_root: Some(temp.path().to_path_buf()),
        })?;
        assert_eq!(result.cache.status, EmbeddingCacheStatus::NotPrepared);
        assert_eq!(
            result.model.profile_id, "bge-small-en-v1.5",
            "cache-only default uses the FastEmbed BGE profile"
        );
        assert!(result.required_files.len() >= 5);
        assert!(
            result
                .required_files
                .iter()
                .any(|file| file.role == EmbeddingModelFileRole::RuntimeModel)
        );
        assert!(
            result
                .required_files
                .iter()
                .any(|file| file.role == EmbeddingModelFileRole::Tokenizer)
        );
        assert!(result.required_files.iter().all(|file| {
            file.state == EmbeddingModelFileState::Missing && file.reason.as_deref() == Some("not-present-in-cache")
        }));
        assert_eq!(result.total_bytes, None);
        Ok(())
    }
}
