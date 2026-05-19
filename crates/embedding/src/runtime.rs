use std::fs;
use std::time::Instant;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use serde::Deserialize;
use tokenizers::{Tokenizer, utils::truncation::TruncationDirection};

use crate::model_cache::resolve_prepared_model;
use crate::pooling::{mean_pool_and_normalize, normalize};
use crate::profiles::ModelProfile;
use crate::vector_cache::VectorCache;
use crate::{
    EmbeddingRuntimeCounters, EmbeddingVector, Error, Result, TextEmbeddingBatchRequest, TextEmbeddingBatchResult,
};

const DEFAULT_BATCH_SIZE: usize = 16;

pub(crate) fn embed_text_batch(
    request: TextEmbeddingBatchRequest,
    profile: ModelProfile,
) -> Result<TextEmbeddingBatchResult> {
    let prepared = resolve_prepared_model(request.model, profile, request.model_cache_root)?;
    let model_fingerprint = prepared.summary.fingerprint.clone().ok_or_else(|| Error::Runtime {
        reason: "prepared model has no fingerprint".to_owned(),
    })?;
    let cache = VectorCache::new(request.vector_cache_root, model_fingerprint, &request.input_policy);
    let mut runtime = EmbeddingRuntimeCounters::default();
    let mut output_slots = vec![None; request.inputs.len()];
    let mut misses = Vec::new();

    for (index, input) in request.inputs.iter().enumerate() {
        if let Some(values) = cache.get(&input.text)? {
            runtime.cache_hits = runtime.cache_hits.saturating_add(1);
            output_slots[index] = Some(EmbeddingVector {
                input_id: input.id.clone(),
                values: normalize(values)?,
            });
        } else {
            runtime.cache_misses = runtime.cache_misses.saturating_add(1);
            misses.push(index);
        }
    }

    if !misses.is_empty() {
        let started = Instant::now();
        let engine = RuntimeEngine::load(&prepared.files)?;
        runtime.model_load_ms = started.elapsed().as_millis();

        for chunk in misses.chunks(DEFAULT_BATCH_SIZE) {
            runtime.batch_count = runtime.batch_count.saturating_add(1);
            let inputs = chunk
                .iter()
                .map(|index| request.inputs[*index].text.as_str())
                .collect::<Vec<_>>();
            let tokenization_started = Instant::now();
            let batch = engine.tokenize(&inputs)?;
            runtime.tokenization_ms = runtime
                .tokenization_ms
                .saturating_add(tokenization_started.elapsed().as_millis());

            let inference_started = Instant::now();
            let vectors = engine.infer(batch)?;
            runtime.inference_ms = runtime
                .inference_ms
                .saturating_add(inference_started.elapsed().as_millis());

            for (index, values) in chunk.iter().zip(vectors) {
                let input = &request.inputs[*index];
                cache.put(&input.text, &values)?;
                output_slots[*index] = Some(EmbeddingVector {
                    input_id: input.id.clone(),
                    values,
                });
            }
        }
    }

    let vectors = output_slots
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Error::Runtime {
            reason: "embedding output assembly missed an input".to_owned(),
        })?;
    let vector_dimension = vectors.first().map_or(0, |vector| vector.values.len());
    runtime.peak_rss_bytes = lean_dup_diagnostics::perf::peak_rss_bytes();
    Ok(TextEmbeddingBatchResult {
        model: prepared.summary,
        cache: prepared.cache,
        input_policy: request.input_policy,
        vector_dimension,
        runtime,
        vectors,
    })
}

struct RuntimeEngine {
    tokenizer: Tokenizer,
    model: BertModel,
    config: BertConfig,
}

struct TokenizedBatch {
    input_ids: Tensor,
    token_type_ids: Tensor,
    attention_mask: Tensor,
    masks: Vec<Vec<u32>>,
}

impl RuntimeEngine {
    fn load(files: &crate::model_cache::PreparedModelFiles) -> Result<Self> {
        let config_json = fs::read_to_string(&files.config).map_err(|source| Error::Io {
            operation: "read-model-config",
            source,
        })?;
        let config = serde_json::from_str::<BertConfig>(&config_json).map_err(|source| Error::Json {
            artifact: "bert-config",
            source,
        })?;
        if config.model_type.as_deref() != Some("bert") {
            return Err(Error::UnsupportedModel {
                reason: "only BERT-family sentence-transformer models are supported".to_owned(),
            });
        }
        validate_pooling_config(&files.pooling_config)?;

        let tokenizer = Tokenizer::from_file(&files.tokenizer).map_err(|source| Error::Tokenizer {
            reason: stable_external_error(source.to_string()),
        })?;

        let device = Device::Cpu;
        let weights = fs::read(&files.weights).map_err(|source| Error::Io {
            operation: "read-model-weights",
            source,
        })?;
        let var_builder =
            VarBuilder::from_buffered_safetensors(weights, DType::F32, &device).map_err(|source| Error::Runtime {
                reason: stable_external_error(source.to_string()),
            })?;
        let model = BertModel::load(var_builder, &config).map_err(|source| Error::Runtime {
            reason: stable_external_error(source.to_string()),
        })?;
        Ok(Self {
            tokenizer,
            model,
            config,
        })
    }

    fn tokenize(&self, texts: &[&str]) -> Result<TokenizedBatch> {
        let mut encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|source| Error::Tokenizer {
                reason: stable_external_error(source.to_string()),
            })?;
        let max_model_len = self.config.max_position_embeddings.max(1);
        for encoding in &mut encodings {
            encoding.truncate(max_model_len, 0, TruncationDirection::Right);
        }
        let sequence_len = encodings
            .iter()
            .map(tokenizers::Encoding::len)
            .max()
            .unwrap_or(1)
            .max(1);
        let batch_len = encodings.len();
        let mut input_ids = Vec::with_capacity(batch_len.saturating_mul(sequence_len));
        let mut token_type_ids = Vec::with_capacity(batch_len.saturating_mul(sequence_len));
        let mut attention_mask = Vec::with_capacity(batch_len.saturating_mul(sequence_len));
        let mut masks = Vec::with_capacity(batch_len);

        for encoding in &encodings {
            let ids = encoding.get_ids();
            let types = encoding.get_type_ids();
            let mask = encoding.get_attention_mask();
            let mut row_mask = Vec::with_capacity(sequence_len);
            for position in 0..sequence_len {
                input_ids.push(*ids.get(position).unwrap_or(&0));
                token_type_ids.push(*types.get(position).unwrap_or(&0));
                let value = *mask.get(position).unwrap_or(&0);
                attention_mask.push(value);
                row_mask.push(value);
            }
            masks.push(row_mask);
        }

        let shape = (batch_len, sequence_len);
        Ok(TokenizedBatch {
            input_ids: Tensor::from_vec(input_ids, shape, &Device::Cpu).map_err(candle_error)?,
            token_type_ids: Tensor::from_vec(token_type_ids, shape, &Device::Cpu).map_err(candle_error)?,
            attention_mask: Tensor::from_vec(attention_mask, shape, &Device::Cpu).map_err(candle_error)?,
            masks,
        })
    }

    fn infer(&self, batch: TokenizedBatch) -> Result<Vec<Vec<f32>>> {
        let hidden_states = self
            .model
            .forward(&batch.input_ids, &batch.token_type_ids, Some(&batch.attention_mask))
            .map_err(candle_error)?
            .to_vec3::<f32>()
            .map_err(candle_error)?;
        mean_pool_and_normalize(hidden_states, &batch.masks)
    }
}

#[derive(Debug, Deserialize)]
struct PoolingConfig {
    pooling_mode_cls_token: bool,
    pooling_mode_mean_tokens: bool,
    pooling_mode_max_tokens: bool,
    pooling_mode_mean_sqrt_len_tokens: bool,
}

fn validate_pooling_config(path: &std::path::Path) -> Result<()> {
    let json = fs::read_to_string(path).map_err(|source| Error::Io {
        operation: "read-pooling-config",
        source,
    })?;
    let config = serde_json::from_str::<PoolingConfig>(&json).map_err(|source| Error::Json {
        artifact: "pooling-config",
        source,
    })?;
    if config.pooling_mode_mean_tokens
        && !config.pooling_mode_cls_token
        && !config.pooling_mode_max_tokens
        && !config.pooling_mode_mean_sqrt_len_tokens
    {
        return Ok(());
    }
    Err(Error::UnsupportedModel {
        reason: "only attention-mask mean pooling is supported".to_owned(),
    })
}

fn candle_error(source: candle_core::Error) -> Error {
    Error::Runtime {
        reason: stable_external_error(source.to_string()),
    }
}

fn stable_external_error(error: String) -> String {
    error.lines().next().unwrap_or("runtime-error").to_owned()
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{EmbeddingInputPolicy, TextEmbeddingInput};

    fn fake_vector(text: &str, dimension: usize) -> Result<Vec<f32>> {
        let digest = Sha256::digest(text.as_bytes());
        let mut values = Vec::with_capacity(dimension);
        for index in 0..dimension {
            let byte = digest.get(index % digest.len()).copied().unwrap_or(0);
            values.push(f32::from(byte) + 1.0);
        }
        normalize(values)
    }

    #[test]
    fn fake_backend_vectors_are_deterministic_and_normalized() -> Result<()> {
        let first = fake_vector("Tiny.same_left", 8)?;
        let second = fake_vector("Tiny.same_left", 8)?;
        let other = fake_vector("Tiny.same_right", 8)?;
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!((first.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-5);
        Ok(())
    }

    #[test]
    fn missing_prepared_model_is_reported_without_download() -> Result<()> {
        let temp = tempfile::TempDir::new().map_err(|source| Error::Io {
            operation: "tempdir",
            source,
        })?;
        let request = TextEmbeddingBatchRequest {
            model: legacy_minilm_model(),
            input_policy: EmbeddingInputPolicy::default(),
            inputs: vec![TextEmbeddingInput {
                id: "Tiny.same_left".to_owned(),
                text: "name: Tiny.same_left".to_owned(),
            }],
            model_cache_root: Some(temp.path().to_path_buf()),
            vector_cache_root: Some(temp.path().join("vectors")),
        };
        let profile = crate::profiles::resolve_profile(&request.model)?;
        assert!(matches!(
            embed_text_batch(request, profile),
            Err(Error::ModelNotPrepared { .. })
        ));
        Ok(())
    }

    #[test]
    #[ignore = "requires `cargo run -p lean-dup-cli -- embedding prepare --policy download-if-missing --model-id sentence-transformers/all-MiniLM-L6-v2` first"]
    fn prepared_legacy_minilm_model_produces_normalized_vectors_or_clean_skip() -> Result<()> {
        let request = TextEmbeddingBatchRequest {
            model: legacy_minilm_model(),
            input_policy: EmbeddingInputPolicy::default(),
            inputs: vec![TextEmbeddingInput {
                id: "smoke".to_owned(),
                text: "name: Tiny.same_left\nmodule: Tiny\nkind: theorem".to_owned(),
            }],
            model_cache_root: None,
            vector_cache_root: Some(std::path::PathBuf::from("target/embedding-smoke/vectors")),
        };
        let profile = crate::profiles::resolve_profile(&request.model)?;
        match embed_text_batch(request, profile) {
            Ok(result) => {
                assert_eq!(result.vector_dimension, 384);
                let vector = result.vectors.first().ok_or_else(|| Error::InvalidVector {
                    reason: "missing smoke vector".to_owned(),
                })?;
                assert!((vector.values.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-4);
                Ok(())
            }
            Err(Error::ModelNotPrepared { .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn legacy_minilm_model() -> crate::EmbeddingModelSpec {
        crate::EmbeddingModelSpec {
            id: "sentence-transformers/all-MiniLM-L6-v2".to_owned(),
            revision: None,
        }
    }
}
