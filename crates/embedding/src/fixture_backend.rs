use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::profiles::ModelProfile;
use crate::vector_cache::VectorCache;
use crate::{
    EmbeddingCacheStatus, EmbeddingCacheSummary, EmbeddingPrepareRequest, EmbeddingPrepareResult,
    EmbeddingRuntimeCounters, EmbeddingVector, Error, Result, TextEmbeddingBatchRequest, TextEmbeddingBatchResult,
    hex_bytes,
};

pub(crate) fn prepare_embedding_model(
    request: EmbeddingPrepareRequest,
    profile: ModelProfile,
) -> Result<EmbeddingPrepareResult> {
    let started = Instant::now();
    Ok(EmbeddingPrepareResult {
        model: profile.summary(&request.model, Some(fixture_fingerprint(profile, &request.model))),
        cache: EmbeddingCacheSummary {
            status: EmbeddingCacheStatus::Prepared,
            model: request.model,
            cache_label: request.cache_root.map(|path| path.display().to_string()),
        },
        acquisition_policy: request.acquisition_policy,
        elapsed_ms: started.elapsed().as_millis(),
        required_files: Vec::new(),
        total_bytes: Some(0),
        reasons: Vec::new(),
    })
}

pub(crate) fn embed_text_batch(
    request: TextEmbeddingBatchRequest,
    profile: ModelProfile,
) -> Result<TextEmbeddingBatchResult> {
    let fingerprint = fixture_fingerprint(profile, &request.model);
    let cache = VectorCache::new(
        request.vector_cache_root,
        fingerprint.clone(),
        request.input_format,
        &request.input_policy,
    );
    let mut runtime = EmbeddingRuntimeCounters::default();
    let mut vectors = Vec::with_capacity(request.inputs.len());
    for input in &request.inputs {
        let wrapped = profile.wrap_text(request.input_format, request.role, &input.text);
        let values = if let Some(values) = cache.get(&wrapped)? {
            runtime.cache_hits = runtime.cache_hits.saturating_add(1);
            normalize(values)?
        } else {
            runtime.cache_misses = runtime.cache_misses.saturating_add(1);
            let values = fixture_vector(&input.id, profile.dimension)?;
            cache.put(&wrapped, &values)?;
            values
        };
        vectors.push(EmbeddingVector {
            input_id: input.id.clone(),
            values,
        });
    }
    runtime.batch_count = usize::from(!request.inputs.is_empty()) as u64;
    runtime.peak_rss_bytes = lean_dup_diagnostics::perf::peak_rss_bytes();
    Ok(TextEmbeddingBatchResult {
        model: profile.summary(&request.model, Some(fingerprint)),
        cache: EmbeddingCacheSummary {
            status: EmbeddingCacheStatus::Prepared,
            model: request.model,
            cache_label: None,
        },
        input_format: request.input_format.summary(),
        input_policy: request.input_policy,
        vector_dimension: profile.dimension,
        runtime,
        vectors,
    })
}

fn fixture_fingerprint(profile: ModelProfile, model: &crate::EmbeddingModelSpec) -> String {
    let mut hasher = Sha256::new();
    hasher.update(profile.fingerprint_seed(model).as_bytes());
    hex_bytes(&hasher.finalize())
}

fn fixture_vector(input_id: &str, dimension: usize) -> Result<Vec<f32>> {
    let mut values = vec![0.0; dimension];
    let lower = input_id.to_ascii_lowercase();
    match lower.as_str() {
        id if id.contains("vector_only_query") || id.contains("vector_only_match") => values[0] = 1.0,
        id if id.contains("lexicaltrap.height") || id.contains("lexicaltrap.height_not_duplicate") => {
            values[1] = 1.0;
        }
        id if id.contains("symbolic_only_query") => values[2] = 1.0,
        id if id.contains("symbolic_only_document") => values[2] = -1.0,
        _ => {
            let mut hasher = Sha256::new();
            hasher.update(input_id.as_bytes());
            let bytes = hasher.finalize();
            for (offset, slot) in values.iter_mut().enumerate().skip(3) {
                let byte = bytes[offset % bytes.len()];
                *slot = (f32::from(byte) / 255.0) * 2.0 - 1.0;
            }
        }
    }
    normalize(values)
}

fn normalize(mut values: Vec<f32>) -> Result<Vec<f32>> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidVector {
            reason: "fixture vector contains non-finite value".to_owned(),
        });
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return Err(Error::InvalidVector {
            reason: "fixture vector has zero norm".to_owned(),
        });
    }
    for value in &mut values {
        *value /= norm;
    }
    Ok(values)
}
