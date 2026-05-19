//! Local text embedding boundary for search-quality experiments.
//!
//! This crate owns future model acquisition, tokenizer compatibility, CPU
//! inference, pooling, normalization, batching, vector caching, and embedding
//! runtime counters. Callers provide declaration-summary strings and receive
//! stable model/cache/runtime facts; they do not learn Hugging Face cache
//! layout, model filenames, tokenizer internals, Candle tensor shapes, or
//! vector-cache storage.

mod error;

pub use error::{Error, Result};
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

/// Embed declaration-summary strings locally.
///
/// Prompt 35A defines the API boundary only. Prompt 35B will add explicit model
/// preparation, and Prompt 35C will replace this unsupported result with a CPU
/// runtime implementation.
pub fn embed_text_batch(_request: TextEmbeddingBatchRequest) -> Result<TextEmbeddingBatchResult> {
    Err(Error::UnsupportedUntilRuntimePrompt)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn runtime_is_explicitly_unsupported_until_prompt_35c() {
        let request = TextEmbeddingBatchRequest {
            model: EmbeddingModelSpec::default_experiment_model(),
            input_policy: EmbeddingInputPolicy::default(),
            inputs: vec![TextEmbeddingInput {
                id: "Tiny.same_left".to_owned(),
                text: "name: Tiny.same_left".to_owned(),
            }],
        };
        assert!(matches!(
            embed_text_batch(request),
            Err(Error::UnsupportedUntilRuntimePrompt)
        ));
    }
}
