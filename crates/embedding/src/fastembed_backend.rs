use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fastembed::{InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel};
use hf_hub::{Cache, Repo, RepoType, api::sync::ApiBuilder};
use sha2::Digest;

use crate::profiles::ModelProfile;
use crate::vector_cache::VectorCache;
use crate::{
    EmbeddingAcquisitionPolicy, EmbeddingCacheStatus, EmbeddingCacheSummary, EmbeddingModelFileRole,
    EmbeddingModelFileState, EmbeddingModelSpec, EmbeddingPrepareRequest, EmbeddingPrepareResult,
    EmbeddingRequiredFileStatus, EmbeddingRuntimeCounters, EmbeddingVector, Error, Result, TextEmbeddingBatchRequest,
    TextEmbeddingBatchResult, hex_bytes, resolve_hf_cache_root,
};

const DEFAULT_FASTEMBED_BATCH_SIZE: usize = 256;

struct FastEmbedPreparedFiles {
    onnx: PathBuf,
    tokenizer: PathBuf,
    config: PathBuf,
    special_tokens: PathBuf,
    tokenizer_config: PathBuf,
}

pub(crate) fn prepare_embedding_model(
    request: EmbeddingPrepareRequest,
    profile: ModelProfile,
) -> Result<EmbeddingPrepareResult> {
    let start = Instant::now();
    let cache_root = resolve_hf_cache_root(request.cache_root.clone());
    let (required_files, files, reasons) = resolve_fastembed_files(&request, profile, &cache_root)?;
    let all_available = required_files.iter().all(|file| {
        matches!(
            file.state,
            EmbeddingModelFileState::Present | EmbeddingModelFileState::Downloaded
        )
    });
    let status = if all_available && files.is_some() {
        EmbeddingCacheStatus::Prepared
    } else if required_files
        .iter()
        .any(|file| file.state == EmbeddingModelFileState::Unavailable)
    {
        EmbeddingCacheStatus::Unusable
    } else {
        EmbeddingCacheStatus::NotPrepared
    };

    if status == EmbeddingCacheStatus::Prepared
        && let Some(files) = &files
    {
        let model = load_user_defined_model(profile, files)?;
        drop(model);
    }

    Ok(EmbeddingPrepareResult {
        model: profile.summary(
            &request.model,
            (status == EmbeddingCacheStatus::Prepared)
                .then(|| {
                    files
                        .as_ref()
                        .map(|files| profile_fingerprint(profile, &request.model, files))
                })
                .flatten(),
        ),
        cache: EmbeddingCacheSummary {
            status,
            model: request.model,
            cache_label: Some(cache_root.display().to_string()),
        },
        acquisition_policy: request.acquisition_policy,
        elapsed_ms: start.elapsed().as_millis(),
        total_bytes: sum_known_bytes(&required_files),
        required_files,
        reasons,
    })
}

pub(crate) fn embed_text_batch(
    request: TextEmbeddingBatchRequest,
    profile: ModelProfile,
) -> Result<TextEmbeddingBatchResult> {
    let cache_root = resolve_hf_cache_root(request.model_cache_root.clone());
    let files = local_fastembed_files(profile, &cache_root)?.ok_or_else(|| Error::ModelNotPrepared {
        reason: "missing required fastembed model files".to_owned(),
    })?;
    let model_fingerprint = profile_fingerprint(profile, &request.model, &files);
    let cache = VectorCache::new(
        request.vector_cache_root,
        model_fingerprint.clone(),
        &request.input_policy,
    );
    let mut runtime = EmbeddingRuntimeCounters::default();
    let mut output_slots = vec![None; request.inputs.len()];
    let mut misses = Vec::new();
    let wrapped_inputs = request
        .inputs
        .iter()
        .map(|input| profile.wrap_text(request.role, &input.text))
        .collect::<Vec<_>>();

    for (index, wrapped_text) in wrapped_inputs.iter().enumerate() {
        if let Some(values) = cache.get(wrapped_text)? {
            runtime.cache_hits = runtime.cache_hits.saturating_add(1);
            output_slots[index] = Some(EmbeddingVector {
                input_id: request.inputs[index].id.clone(),
                values: normalize(values)?,
            });
        } else {
            runtime.cache_misses = runtime.cache_misses.saturating_add(1);
            misses.push(index);
        }
    }

    if !misses.is_empty() {
        let started = Instant::now();
        let mut engine = load_user_defined_model(profile, &files)?;
        runtime.model_load_ms = started.elapsed().as_millis();

        for chunk in misses.chunks(DEFAULT_FASTEMBED_BATCH_SIZE) {
            runtime.batch_count = runtime.batch_count.saturating_add(1);
            let batch = chunk
                .iter()
                .map(|index| wrapped_inputs[*index].as_str())
                .collect::<Vec<_>>();
            let inference_started = Instant::now();
            let vectors = engine
                .embed(batch, Some(DEFAULT_FASTEMBED_BATCH_SIZE))
                .map_err(|source| Error::Runtime {
                    reason: stable_fastembed_error(source.to_string()),
                })?;
            runtime.inference_ms = runtime
                .inference_ms
                .saturating_add(inference_started.elapsed().as_millis());
            for (index, values) in chunk.iter().zip(vectors) {
                let normalized = normalize(values)?;
                cache.put(&wrapped_inputs[*index], &normalized)?;
                output_slots[*index] = Some(EmbeddingVector {
                    input_id: request.inputs[*index].id.clone(),
                    values: normalized,
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
    let vector_dimension = vectors.first().map_or(profile.dimension, |vector| vector.values.len());
    runtime.peak_rss_bytes = lean_dup_diagnostics::perf::peak_rss_bytes();
    Ok(TextEmbeddingBatchResult {
        model: profile.summary(&request.model, Some(model_fingerprint)),
        cache: EmbeddingCacheSummary {
            status: EmbeddingCacheStatus::Prepared,
            model: request.model,
            cache_label: Some(cache_root.display().to_string()),
        },
        input_policy: request.input_policy,
        vector_dimension,
        runtime,
        vectors,
    })
}

fn resolve_fastembed_files(
    request: &EmbeddingPrepareRequest,
    profile: ModelProfile,
    cache_root: &Path,
) -> Result<(
    Vec<EmbeddingRequiredFileStatus>,
    Option<FastEmbedPreparedFiles>,
    Vec<String>,
)> {
    match request.acquisition_policy {
        EmbeddingAcquisitionPolicy::CacheOnly => cache_only_fastembed_files(profile, cache_root),
        EmbeddingAcquisitionPolicy::DownloadIfMissing => download_fastembed_files(profile, cache_root),
    }
}

fn cache_only_fastembed_files(
    profile: ModelProfile,
    cache_root: &Path,
) -> Result<(
    Vec<EmbeddingRequiredFileStatus>,
    Option<FastEmbedPreparedFiles>,
    Vec<String>,
)> {
    let Some(model_name) = profile.fastembed_model() else {
        return Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile".to_owned(),
        });
    };
    let info = TextEmbedding::get_model_info(&model_name).map_err(|source| Error::UnsupportedModel {
        reason: stable_fastembed_error(source.to_string()),
    })?;
    let cache_repo = Cache::new(cache_root.to_path_buf()).repo(model_repo(&info.model_code));
    let required = fastembed_required_files(&info.model_file, &info.additional_files);
    let mut statuses = Vec::new();
    let mut paths = Vec::new();
    for required in required {
        match cache_repo.get(&required.filename) {
            Some(path) => {
                statuses.push(file_status(
                    required.role,
                    EmbeddingModelFileState::Present,
                    &path,
                    None,
                ));
                paths.push((required.role, path));
            }
            None => statuses.push(EmbeddingRequiredFileStatus {
                role: required.role,
                state: EmbeddingModelFileState::Missing,
                bytes: None,
                reason: Some("not-present-in-cache".to_owned()),
            }),
        }
    }
    let reasons = statuses
        .iter()
        .filter(|status| status.state == EmbeddingModelFileState::Missing)
        .map(|status| format!("{}:not-present-in-cache", status.role.as_str()))
        .collect::<Vec<_>>();
    Ok((statuses, files_from_paths(paths), reasons))
}

fn download_fastembed_files(
    profile: ModelProfile,
    cache_root: &Path,
) -> Result<(
    Vec<EmbeddingRequiredFileStatus>,
    Option<FastEmbedPreparedFiles>,
    Vec<String>,
)> {
    let Some(model_name) = profile.fastembed_model() else {
        return Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile".to_owned(),
        });
    };
    let info = TextEmbedding::get_model_info(&model_name).map_err(|source| Error::UnsupportedModel {
        reason: stable_fastembed_error(source.to_string()),
    })?;
    let cache = Cache::new(cache_root.to_path_buf());
    let cache_repo = cache.repo(model_repo(&info.model_code));
    let api_repo = ApiBuilder::from_cache(cache)
        .build()
        .map(|api| api.repo(model_repo(&info.model_code)))
        .map_err(|source| Error::Runtime {
            reason: stable_fastembed_error(source.to_string()),
        })?;
    let required = fastembed_required_files(&info.model_file, &info.additional_files);
    let mut statuses = Vec::new();
    let mut paths = Vec::new();
    for required in required {
        if let Some(path) = cache_repo.get(&required.filename) {
            statuses.push(file_status(
                required.role,
                EmbeddingModelFileState::Present,
                &path,
                None,
            ));
            paths.push((required.role, path));
            continue;
        }
        match api_repo.get(&required.filename) {
            Ok(path) => {
                statuses.push(file_status(
                    required.role,
                    EmbeddingModelFileState::Downloaded,
                    &path,
                    None,
                ));
                paths.push((required.role, path));
            }
            Err(error) => {
                let reason = stable_fastembed_error(error.to_string());
                statuses.push(EmbeddingRequiredFileStatus {
                    role: required.role,
                    state: EmbeddingModelFileState::Unavailable,
                    bytes: None,
                    reason: Some(reason),
                });
            }
        }
    }
    let reasons = statuses
        .iter()
        .filter(|status| status.state == EmbeddingModelFileState::Unavailable)
        .map(|status| {
            format!(
                "{}:{}",
                status.role.as_str(),
                status.reason.as_deref().unwrap_or("unavailable")
            )
        })
        .collect::<Vec<_>>();
    Ok((statuses, files_from_paths(paths), reasons))
}

fn local_fastembed_files(profile: ModelProfile, cache_root: &Path) -> Result<Option<FastEmbedPreparedFiles>> {
    let (_, files, _) = cache_only_fastembed_files(profile, cache_root)?;
    Ok(files)
}

fn load_user_defined_model(profile: ModelProfile, files: &FastEmbedPreparedFiles) -> Result<TextEmbedding> {
    let Some(model_name) = profile.fastembed_model() else {
        return Err(Error::UnsupportedModel {
            reason: "unsupported-model-profile".to_owned(),
        });
    };
    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read_runtime_file(&files.tokenizer, "read-fastembed-tokenizer")?,
        config_file: read_runtime_file(&files.config, "read-fastembed-config")?,
        special_tokens_map_file: read_runtime_file(&files.special_tokens, "read-fastembed-special-tokens")?,
        tokenizer_config_file: read_runtime_file(&files.tokenizer_config, "read-fastembed-tokenizer-config")?,
    };
    let mut model =
        UserDefinedEmbeddingModel::new(read_runtime_file(&files.onnx, "read-fastembed-onnx")?, tokenizer_files);
    if let Some(pooling) = TextEmbedding::get_default_pooling_method(&model_name) {
        model = model.with_pooling(pooling);
    }
    model = model.with_quantization(TextEmbedding::get_quantization_mode(&model_name));
    TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::default()).map_err(|source| {
        Error::Runtime {
            reason: stable_fastembed_error(source.to_string()),
        }
    })
}

fn read_runtime_file(path: &Path, operation: &'static str) -> Result<Vec<u8>> {
    fs::read(path).map_err(|source| Error::Io { operation, source })
}

#[derive(Debug, Clone)]
struct FastEmbedRequiredFile {
    role: EmbeddingModelFileRole,
    filename: String,
}

fn fastembed_required_files(model_file: &str, additional_files: &[String]) -> Vec<FastEmbedRequiredFile> {
    let mut files = vec![
        FastEmbedRequiredFile {
            role: EmbeddingModelFileRole::RuntimeModel,
            filename: model_file.to_owned(),
        },
        FastEmbedRequiredFile {
            role: EmbeddingModelFileRole::Config,
            filename: "config.json".to_owned(),
        },
        FastEmbedRequiredFile {
            role: EmbeddingModelFileRole::Tokenizer,
            filename: "tokenizer.json".to_owned(),
        },
        FastEmbedRequiredFile {
            role: EmbeddingModelFileRole::TokenizerConfig,
            filename: "tokenizer_config.json".to_owned(),
        },
        FastEmbedRequiredFile {
            role: EmbeddingModelFileRole::SpecialTokens,
            filename: "special_tokens_map.json".to_owned(),
        },
    ];
    for filename in additional_files {
        files.push(FastEmbedRequiredFile {
            role: EmbeddingModelFileRole::RuntimeModel,
            filename: filename.clone(),
        });
    }
    files
}

fn model_repo(model_code: &str) -> Repo {
    Repo::new(model_code.to_owned(), RepoType::Model)
}

fn files_from_paths(paths: Vec<(EmbeddingModelFileRole, PathBuf)>) -> Option<FastEmbedPreparedFiles> {
    let mut onnx = None;
    let mut tokenizer = None;
    let mut config = None;
    let mut special_tokens = None;
    let mut tokenizer_config = None;
    for (role, path) in paths {
        match role {
            EmbeddingModelFileRole::RuntimeModel => onnx = onnx.or(Some(path)),
            EmbeddingModelFileRole::Config => config = Some(path),
            EmbeddingModelFileRole::Tokenizer => tokenizer = Some(path),
            EmbeddingModelFileRole::TokenizerConfig => tokenizer_config = Some(path),
            EmbeddingModelFileRole::SpecialTokens => special_tokens = Some(path),
        }
    }
    Some(FastEmbedPreparedFiles {
        onnx: onnx?,
        tokenizer: tokenizer?,
        config: config?,
        special_tokens: special_tokens?,
        tokenizer_config: tokenizer_config?,
    })
}

fn file_status(
    role: EmbeddingModelFileRole,
    state: EmbeddingModelFileState,
    path: &Path,
    reason: Option<String>,
) -> EmbeddingRequiredFileStatus {
    EmbeddingRequiredFileStatus {
        role,
        state,
        bytes: path.metadata().ok().map(|metadata| metadata.len()),
        reason,
    }
}

fn profile_fingerprint(profile: ModelProfile, model: &EmbeddingModelSpec, files: &FastEmbedPreparedFiles) -> String {
    let mut paths = vec![
        files.onnx.clone(),
        files.tokenizer.clone(),
        files.config.clone(),
        files.special_tokens.clone(),
        files.tokenizer_config.clone(),
    ];
    paths.sort();
    let mut hasher = sha2::Sha256::new();
    hasher.update(profile.fingerprint_seed(model).as_bytes());
    for path in paths {
        if let Ok(bytes) = fs::read(path) {
            hasher.update(bytes.len().to_le_bytes());
            hasher.update(bytes);
        }
    }
    hex_bytes(&hasher.finalize())
}

fn stable_fastembed_error(error: String) -> String {
    let first = error.lines().next().unwrap_or("fastembed-error");
    let lower = first.to_ascii_lowercase();
    if lower.contains("network") || lower.contains("connection") || lower.contains("dns") {
        "network-error".to_owned()
    } else if lower.contains("not found") || lower.contains("404") {
        "remote-file-missing".to_owned()
    } else {
        first.to_owned()
    }
}

fn normalize(values: Vec<f32>) -> Result<Vec<f32>> {
    let norm = values
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err(Error::InvalidVector {
            reason: "zero-or-nonfinite-norm".to_owned(),
        });
    }
    Ok(values
        .into_iter()
        .map(|value| (f64::from(value) / norm) as f32)
        .collect())
}

fn sum_known_bytes(files: &[EmbeddingRequiredFileStatus]) -> Option<u64> {
    let mut total = 0_u64;
    for file in files {
        let bytes = file.bytes?;
        total = total.saturating_add(bytes);
    }
    Some(total)
}
