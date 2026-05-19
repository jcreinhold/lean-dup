use std::fs;
use std::path::PathBuf;

use hf_hub::Cache;
use sha2::{Digest, Sha256};

use crate::{
    EmbeddingCacheStatus, EmbeddingCacheSummary, EmbeddingModelFileRole, EmbeddingModelSpec, EmbeddingModelSummary,
    Error, Result, required_model_files, resolve_hf_cache_root,
};

pub(crate) struct PreparedModel {
    pub(crate) summary: EmbeddingModelSummary,
    pub(crate) cache: EmbeddingCacheSummary,
    pub(crate) files: PreparedModelFiles,
}

pub(crate) struct PreparedModelFiles {
    pub(crate) config: PathBuf,
    pub(crate) tokenizer: PathBuf,
    pub(crate) pooling_config: PathBuf,
    pub(crate) weights: PathBuf,
}

pub(crate) fn resolve_prepared_model(model: EmbeddingModelSpec, cache_root: Option<PathBuf>) -> Result<PreparedModel> {
    let cache_root = resolve_hf_cache_root(cache_root);
    let cache = Cache::new(cache_root.clone());
    let cache_repo = cache.repo(crate::model_repo(&model));

    let mut config = None;
    let mut tokenizer = None;
    let mut pooling_config = None;
    let mut weights = None;
    let mut fingerprint_inputs = Vec::new();

    for required in required_model_files() {
        let path = cache_repo
            .get(required.filename)
            .ok_or_else(|| Error::ModelNotPrepared {
                reason: format!("missing required model role {}", required.role.as_str()),
            })?;
        fingerprint_inputs.push(path.clone());
        match required.role {
            EmbeddingModelFileRole::Config => config = Some(path),
            EmbeddingModelFileRole::Tokenizer => tokenizer = Some(path),
            EmbeddingModelFileRole::PoolingConfig => pooling_config = Some(path),
            EmbeddingModelFileRole::Weights => weights = Some(path),
            EmbeddingModelFileRole::TokenizerConfig | EmbeddingModelFileRole::SpecialTokens => {}
        }
    }

    let fingerprint = fingerprint_files(&mut fingerprint_inputs)?;
    Ok(PreparedModel {
        summary: EmbeddingModelSummary {
            id: model.id.clone(),
            revision: model.revision.clone(),
            fingerprint: Some(fingerprint),
        },
        cache: EmbeddingCacheSummary {
            status: EmbeddingCacheStatus::Prepared,
            model,
            cache_label: Some(cache_root.display().to_string()),
        },
        files: PreparedModelFiles {
            config: config.ok_or_else(|| Error::ModelNotPrepared {
                reason: "missing required model role config".to_owned(),
            })?,
            tokenizer: tokenizer.ok_or_else(|| Error::ModelNotPrepared {
                reason: "missing required model role tokenizer".to_owned(),
            })?,
            pooling_config: pooling_config.ok_or_else(|| Error::ModelNotPrepared {
                reason: "missing required model role pooling-config".to_owned(),
            })?,
            weights: weights.ok_or_else(|| Error::ModelNotPrepared {
                reason: "missing required model role weights".to_owned(),
            })?,
        },
    })
}

fn fingerprint_files(paths: &mut [PathBuf]) -> Result<String> {
    paths.sort();
    let mut hasher = Sha256::new();
    for path in paths {
        let bytes = fs::read(path).map_err(|source| Error::Io {
            operation: "fingerprint-model-file",
            source,
        })?;
        hasher.update(bytes.len().to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
