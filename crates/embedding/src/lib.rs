//! Local text embedding boundary for search-quality experiments.
//!
//! This crate owns future model acquisition, tokenizer compatibility, CPU
//! inference, pooling, normalization, batching, vector caching, and embedding
//! runtime counters. Callers provide declaration-summary strings and receive
//! stable model/cache/runtime facts; they do not learn Hugging Face cache
//! layout, model filenames, tokenizer internals, Candle tensor shapes, or
//! vector-cache storage.

mod error;
mod model_cache;
mod pooling;
mod runtime;
mod vector_cache;

pub use error::{Error, Result};
use std::path::PathBuf;
use std::time::Instant;

use hf_hub::{Cache, Repo, RepoType, api::sync::ApiBuilder};
use serde::{Deserialize, Serialize};

/// Version of the declaration-summary input contract consumed by embeddings.
pub const EMBEDDING_INPUT_POLICY_VERSION: &str = "lean-dup.embedding-input.v1";

/// A model the embedding crate can prepare and run.
///
/// The default is intentionally a model identity, not a filesystem path. Prompt
/// 35B will teach this crate how to prepare the model through an explicit cache
/// policy; default audit paths must not download or load it implicitly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingModelSpec {
    pub id: String,
    pub revision: Option<String>,
}

impl EmbeddingModelSpec {
    /// Return the first candidate model for the embedding experiment sequence.
    pub fn default_experiment_model() -> Self {
        Self {
            id: "sentence-transformers/all-MiniLM-L6-v2".to_owned(),
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

/// Stable model-file role used in preparation diagnostics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingModelFileRole {
    Config,
    Tokenizer,
    TokenizerConfig,
    SpecialTokens,
    PoolingConfig,
    Weights,
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

/// Public contract for how declaration summaries are converted to model input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingInputPolicy {
    pub version: String,
    pub includes_declaration_name: bool,
    pub includes_module_name: bool,
    pub includes_declaration_kind: bool,
    pub includes_normalized_statement: bool,
    pub includes_feature_summaries: bool,
}

impl Default for EmbeddingInputPolicy {
    fn default() -> Self {
        Self {
            version: EMBEDDING_INPUT_POLICY_VERSION.to_owned(),
            includes_declaration_name: true,
            includes_module_name: true,
            includes_declaration_kind: true,
            includes_normalized_statement: true,
            includes_feature_summaries: true,
        }
    }
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
    pub tokenization_ms: u128,
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

/// Embed declaration-summary strings locally using a prepared CPU model.
///
/// This operation is cache-only: it validates model files prepared by
/// `prepare_embedding_model` and never downloads. Callers receive normalized
/// vectors and stable runtime counters without learning tokenizer, tensor,
/// pooling, or vector-cache layout.
pub fn embed_text_batch(request: TextEmbeddingBatchRequest) -> Result<TextEmbeddingBatchResult> {
    runtime::embed_text_batch(request)
}

/// Validate or prepare an embedding model in the local Hugging Face cache.
///
/// This is the only crate-root acquisition capability. Callers choose whether
/// the operation may download; file names, snapshot layout, and cache probing
/// remain private to this crate.
pub fn prepare_embedding_model(request: EmbeddingPrepareRequest) -> Result<EmbeddingPrepareResult> {
    validate_prepare_request(&request)?;
    let start = Instant::now();
    let cache_root = resolve_hf_cache_root(request.cache_root.clone());
    let cache = Cache::new(cache_root.clone());
    let repo = model_repo(&request.model);
    let cache_repo = cache.repo(repo.clone());
    let api_repo = if request.acquisition_policy == EmbeddingAcquisitionPolicy::DownloadIfMissing {
        ApiBuilder::from_cache(cache).build().map(|api| api.repo(repo)).ok()
    } else {
        None
    };
    let api_build_error =
        if request.acquisition_policy == EmbeddingAcquisitionPolicy::DownloadIfMissing && api_repo.is_none() {
            Some("could-not-create-hugging-face-api".to_owned())
        } else {
            None
        };

    let mut required_files = Vec::new();
    for required in required_model_files() {
        let cached_path = cache_repo.get(required.filename);
        let status = if let Some(path) = cached_path {
            file_status(required.role, EmbeddingModelFileState::Present, &path, None)
        } else if let Some(api_repo) = &api_repo {
            match api_repo.get(required.filename) {
                Ok(path) => file_status(required.role, EmbeddingModelFileState::Downloaded, &path, None),
                Err(error) => EmbeddingRequiredFileStatus {
                    role: required.role,
                    state: EmbeddingModelFileState::Unavailable,
                    bytes: None,
                    reason: Some(stable_download_reason(error.to_string())),
                },
            }
        } else if request.acquisition_policy == EmbeddingAcquisitionPolicy::DownloadIfMissing {
            EmbeddingRequiredFileStatus {
                role: required.role,
                state: EmbeddingModelFileState::Unavailable,
                bytes: None,
                reason: Some("api-unavailable".to_owned()),
            }
        } else {
            EmbeddingRequiredFileStatus {
                role: required.role,
                state: EmbeddingModelFileState::Missing,
                bytes: None,
                reason: Some("not-present-in-cache".to_owned()),
            }
        };
        required_files.push(status);
    }

    let mut reasons = Vec::new();
    if let Some(reason) = api_build_error {
        reasons.push(reason);
    }
    for file in &required_files {
        if !matches!(
            file.state,
            EmbeddingModelFileState::Present | EmbeddingModelFileState::Downloaded
        ) {
            reasons.push(format!(
                "{}:{}",
                file.role.as_str(),
                file.reason.as_deref().unwrap_or("unavailable")
            ));
        }
    }

    let status = if required_files.iter().all(|file| {
        matches!(
            file.state,
            EmbeddingModelFileState::Present | EmbeddingModelFileState::Downloaded
        )
    }) {
        EmbeddingCacheStatus::Prepared
    } else if required_files
        .iter()
        .any(|file| file.state == EmbeddingModelFileState::Unavailable)
    {
        EmbeddingCacheStatus::Unusable
    } else {
        EmbeddingCacheStatus::NotPrepared
    };
    let total_bytes = sum_known_bytes(&required_files);
    Ok(EmbeddingPrepareResult {
        model: EmbeddingModelSummary {
            id: request.model.id.clone(),
            revision: request.model.revision.clone(),
            fingerprint: None,
        },
        cache: EmbeddingCacheSummary {
            status,
            model: request.model,
            cache_label: Some(cache_root.display().to_string()),
        },
        acquisition_policy: request.acquisition_policy,
        elapsed_ms: start.elapsed().as_millis(),
        required_files,
        total_bytes,
        reasons,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RequiredModelFile {
    pub(crate) role: EmbeddingModelFileRole,
    pub(crate) filename: &'static str,
}

pub(crate) fn required_model_files() -> &'static [RequiredModelFile] {
    &[
        RequiredModelFile {
            role: EmbeddingModelFileRole::Config,
            filename: "config.json",
        },
        RequiredModelFile {
            role: EmbeddingModelFileRole::Tokenizer,
            filename: "tokenizer.json",
        },
        RequiredModelFile {
            role: EmbeddingModelFileRole::TokenizerConfig,
            filename: "tokenizer_config.json",
        },
        RequiredModelFile {
            role: EmbeddingModelFileRole::SpecialTokens,
            filename: "special_tokens_map.json",
        },
        RequiredModelFile {
            role: EmbeddingModelFileRole::PoolingConfig,
            filename: "1_Pooling/config.json",
        },
        RequiredModelFile {
            role: EmbeddingModelFileRole::Weights,
            filename: "model.safetensors",
        },
    ]
}

impl EmbeddingModelFileRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Tokenizer => "tokenizer",
            Self::TokenizerConfig => "tokenizer-config",
            Self::SpecialTokens => "special-tokens",
            Self::PoolingConfig => "pooling-config",
            Self::Weights => "weights",
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

pub(crate) fn model_repo(model: &EmbeddingModelSpec) -> Repo {
    match &model.revision {
        Some(revision) => Repo::with_revision(model.id.clone(), RepoType::Model, revision.clone()),
        None => Repo::new(model.id.clone(), RepoType::Model),
    }
}

fn file_status(
    role: EmbeddingModelFileRole,
    state: EmbeddingModelFileState,
    path: &std::path::Path,
    reason: Option<String>,
) -> EmbeddingRequiredFileStatus {
    EmbeddingRequiredFileStatus {
        role,
        state,
        bytes: path.metadata().ok().map(|metadata| metadata.len()),
        reason,
    }
}

fn stable_download_reason(error: String) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("404") || lower.contains("not found") {
        "remote-file-missing".to_owned()
    } else if lower.contains("dns") || lower.contains("network") || lower.contains("connection") {
        "network-error".to_owned()
    } else {
        "download-failed".to_owned()
    }
}

fn sum_known_bytes(files: &[EmbeddingRequiredFileStatus]) -> Option<u64> {
    let mut total = 0_u64;
    for file in files {
        let bytes = file.bytes?;
        total = total.saturating_add(bytes);
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn default_model_is_the_prompt_35_candidate() {
        let model = EmbeddingModelSpec::default_experiment_model();
        assert_eq!(model.id, "sentence-transformers/all-MiniLM-L6-v2");
        assert_eq!(model.revision, None);
    }

    #[test]
    fn input_policy_names_stable_contract() {
        let policy = EmbeddingInputPolicy::default();
        assert_eq!(policy.version, "lean-dup.embedding-input.v1");
        assert!(policy.includes_declaration_name);
        assert!(policy.includes_module_name);
        assert!(policy.includes_declaration_kind);
        assert!(policy.includes_normalized_statement);
        assert!(policy.includes_feature_summaries);
    }

    #[test]
    fn runtime_is_cache_only_and_requires_prepared_model() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let request = TextEmbeddingBatchRequest {
            model: EmbeddingModelSpec::default_experiment_model(),
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
        assert_eq!(request.model.id, "sentence-transformers/all-MiniLM-L6-v2");
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
        assert_eq!(result.required_files.len(), required_model_files().len());
        assert!(result.required_files.iter().all(|file| {
            file.state == EmbeddingModelFileState::Missing && file.reason.as_deref() == Some("not-present-in-cache")
        }));
        assert_eq!(result.total_bytes, None);
        Ok(())
    }

    #[test]
    fn fake_prepared_cache_validates_required_roles() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        write_fake_model_cache(temp.path(), true)?;
        let result = prepare_embedding_model(EmbeddingPrepareRequest {
            model: EmbeddingModelSpec::default_experiment_model(),
            acquisition_policy: EmbeddingAcquisitionPolicy::CacheOnly,
            cache_root: Some(temp.path().to_path_buf()),
        })?;
        assert_eq!(result.cache.status, EmbeddingCacheStatus::Prepared);
        assert!(result.reasons.is_empty());
        assert_eq!(result.model.fingerprint, None);
        assert!(result.total_bytes.is_some());
        assert!(result.required_files.iter().all(|file| {
            file.state == EmbeddingModelFileState::Present && file.bytes.is_some() && file.reason.is_none()
        }));
        Ok(())
    }

    #[test]
    fn missing_safetensors_weight_keeps_cache_not_prepared() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        write_fake_model_cache(temp.path(), false)?;
        let result = prepare_embedding_model(EmbeddingPrepareRequest {
            model: EmbeddingModelSpec::default_experiment_model(),
            acquisition_policy: EmbeddingAcquisitionPolicy::CacheOnly,
            cache_root: Some(temp.path().to_path_buf()),
        })?;
        assert_eq!(result.cache.status, EmbeddingCacheStatus::NotPrepared);
        let weights = result
            .required_files
            .iter()
            .find(|file| file.role == EmbeddingModelFileRole::Weights);
        assert!(matches!(
            weights,
            Some(file) if file.state == EmbeddingModelFileState::Missing
        ));
        Ok(())
    }

    fn write_fake_model_cache(root: &Path, include_weights: bool) -> std::io::Result<()> {
        let repo = root.join("models--sentence-transformers--all-MiniLM-L6-v2");
        let commit = "fake-commit";
        fs::create_dir_all(repo.join("refs"))?;
        fs::write(repo.join("refs/main"), commit)?;
        let snapshot = repo.join("snapshots").join(commit);
        for required in required_model_files() {
            if !include_weights && required.role == EmbeddingModelFileRole::Weights {
                continue;
            }
            let path = snapshot.join(required.filename);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, format!("fake {}", required.role.as_str()))?;
        }
        Ok(())
    }
}
