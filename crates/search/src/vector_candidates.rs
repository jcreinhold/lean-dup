use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use lean_dup_embedding::{
    EmbeddingAcquisitionPolicy, EmbeddingCacheStatus, EmbeddingInputPolicy, EmbeddingInputRole, EmbeddingModelSpec,
    EmbeddingPrepareRequest, TextEmbeddingBatchRequest, TextEmbeddingInput, embed_text_batch, prepare_embedding_model,
};
use lean_dup_index::HydratedDeclaration;
use lean_dup_vector_index::{
    VectorCorpusBuildRequest, VectorCorpusOpenRequest, VectorCorpusProvenance, VectorCorpusQueryRequest,
    VectorCorpusStatus, VectorDeclaration, build_vector_corpus, open_vector_corpus, query_vector_corpus,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::observation::{
    SearchEmbeddingDocumentPolicy, SearchEmbeddingDocuments, embedding_documents_for_declarations_with_policy,
};

const VECTOR_CANDIDATE_POLICY_VERSION: &str = "lean-dup.vector-candidate.v1";
const VECTOR_CANDIDATE_TOP_K: usize = 32;

/// Hidden vector candidate-generation request.
///
/// This request enables an experiment-only search path. Search owns candidate
/// policy; embedding and vector-index details remain behind their crate-root
/// facades.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchVectorCandidateRequest {
    pub model_id: String,
    pub revision: Option<String>,
    pub acquisition_policy: SearchVectorAcquisitionPolicy,
    pub model_cache_root: Option<PathBuf>,
    pub text_vector_cache_root: Option<PathBuf>,
    pub corpus_cache_root: PathBuf,
    pub document_policy: SearchEmbeddingDocumentPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchVectorAcquisitionPolicy {
    CacheOnly,
    DownloadIfMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchVectorCandidateStatus {
    Disabled,
    Skipped,
    Failed,
    Ok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchVectorCorpusStatus {
    Built,
    Reused,
    Missing,
    Stale,
    Unusable,
}

/// Stable vector candidate-generation facts exposed to eval artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchVectorCandidateSummary {
    pub version: &'static str,
    pub status: SearchVectorCandidateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profile_id: Option<String>,
    pub acquisition_policy: SearchVectorAcquisitionPolicy,
    pub document_policy_id: String,
    pub document_policy_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_status: Option<SearchVectorCorpusStatus>,
    pub query_declaration_count: usize,
    pub corpus_declaration_count: usize,
    pub vector_generated_candidate_count: usize,
    pub corpus_build_ms: u128,
    pub query_ms: u128,
    pub embedding_ms: u128,
}

impl Default for SearchVectorCandidateSummary {
    fn default() -> Self {
        Self {
            version: VECTOR_CANDIDATE_POLICY_VERSION,
            status: SearchVectorCandidateStatus::Disabled,
            reason: None,
            model_id: String::new(),
            model_profile_id: None,
            acquisition_policy: SearchVectorAcquisitionPolicy::CacheOnly,
            document_policy_id: String::new(),
            document_policy_version: String::new(),
            corpus_status: None,
            query_declaration_count: 0,
            corpus_declaration_count: 0,
            vector_generated_candidate_count: 0,
            corpus_build_ms: 0,
            query_ms: 0,
            embedding_ms: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VectorCandidateOutput {
    pub(crate) summary: SearchVectorCandidateSummary,
    pub(crate) candidates: Vec<VectorCandidate>,
}

#[derive(Debug, Clone)]
pub(crate) struct VectorCandidate {
    pub(crate) anchor_name: String,
    pub(crate) declaration: HydratedDeclaration,
    pub(crate) score: f32,
    pub(crate) rank: usize,
}

pub(crate) fn generate_vector_candidates(
    request: &SearchVectorCandidateRequest,
    workspace: &[HydratedDeclaration],
    comparison_declarations: &[HydratedDeclaration],
) -> VectorCandidateOutput {
    let query_documents = embedding_documents_for_declarations_with_policy(workspace, request.document_policy);
    let corpus_declarations = if comparison_declarations.is_empty() {
        workspace
    } else {
        comparison_declarations
    };
    let corpus_documents =
        embedding_documents_for_declarations_with_policy(corpus_declarations, request.document_policy);
    let model = EmbeddingModelSpec {
        id: request.model_id.clone(),
        revision: request.revision.clone(),
    };
    let mut summary = SearchVectorCandidateSummary {
        version: VECTOR_CANDIDATE_POLICY_VERSION,
        status: SearchVectorCandidateStatus::Skipped,
        reason: None,
        model_id: request.model_id.clone(),
        model_profile_id: None,
        acquisition_policy: request.acquisition_policy,
        document_policy_id: query_documents.policy_id.clone(),
        document_policy_version: query_documents.policy_version.clone(),
        corpus_status: None,
        query_declaration_count: query_documents.documents.len(),
        corpus_declaration_count: corpus_documents.documents.len(),
        vector_generated_candidate_count: 0,
        corpus_build_ms: 0,
        query_ms: 0,
        embedding_ms: 0,
    };
    if workspace.is_empty() || corpus_declarations.is_empty() {
        summary.reason = Some("empty-vector-corpus".to_owned());
        return VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        };
    }

    let prepare = match prepare_embedding_model(EmbeddingPrepareRequest {
        model: model.clone(),
        acquisition_policy: request.acquisition_policy.into(),
        cache_root: request.model_cache_root.clone(),
    }) {
        Ok(result) => result,
        Err(error) => {
            summary.status = SearchVectorCandidateStatus::Failed;
            summary.reason = Some(stable_embedding_error(&error));
            return VectorCandidateOutput {
                summary,
                candidates: Vec::new(),
            };
        }
    };
    summary.model_profile_id = Some(prepare.model.profile_id.clone());
    if prepare.cache.status != EmbeddingCacheStatus::Prepared {
        summary.reason = Some(
            match prepare.cache.status {
                EmbeddingCacheStatus::NotPrepared => "vector-model-not-prepared",
                EmbeddingCacheStatus::Unusable => "vector-model-unusable",
                EmbeddingCacheStatus::Skipped => "vector-model-skipped",
                EmbeddingCacheStatus::Prepared => "vector-model-prepared",
            }
            .to_owned(),
        );
        return VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        };
    }

    let corpus_embedding = match embed_documents(request, &model, EmbeddingInputRole::Document, &corpus_documents) {
        Ok(result) => result,
        Err(error) => {
            summary.status = SearchVectorCandidateStatus::Failed;
            summary.reason = Some(stable_embedding_error(&error));
            return VectorCandidateOutput {
                summary,
                candidates: Vec::new(),
            };
        }
    };
    let query_embedding = match embed_documents(request, &model, EmbeddingInputRole::Query, &query_documents) {
        Ok(result) => result,
        Err(error) => {
            summary.status = SearchVectorCandidateStatus::Failed;
            summary.reason = Some(stable_embedding_error(&error));
            return VectorCandidateOutput {
                summary,
                candidates: Vec::new(),
            };
        }
    };
    summary.embedding_ms = corpus_embedding
        .runtime
        .model_load_ms
        .saturating_add(corpus_embedding.runtime.inference_ms)
        .saturating_add(query_embedding.runtime.model_load_ms)
        .saturating_add(query_embedding.runtime.inference_ms);
    summary.model_profile_id = Some(corpus_embedding.model.profile_id.clone());

    let corpus_vectors = corpus_embedding
        .vectors
        .iter()
        .map(|vector| (vector.input_id.clone(), vector.values.clone()))
        .collect::<BTreeMap<_, _>>();
    let query_vectors = query_embedding
        .vectors
        .iter()
        .map(|vector| (vector.input_id.clone(), vector.values.clone()))
        .collect::<BTreeMap<_, _>>();
    let corpus_by_name = corpus_declarations
        .iter()
        .map(|declaration| (declaration.qualified_name.clone(), declaration.clone()))
        .collect::<BTreeMap<_, _>>();

    let provenance = VectorCorpusProvenance {
        source_corpus_fingerprint: source_corpus_fingerprint(&corpus_documents),
        embedding_model_profile_id: corpus_embedding.model.profile_id.clone(),
        embedding_model_fingerprint: corpus_embedding.model.fingerprint.clone().unwrap_or_else(|| {
            fallback_model_fingerprint(&corpus_embedding.model.id, corpus_embedding.model.revision.as_deref())
        }),
        document_policy_id: corpus_documents.policy_id.clone(),
        document_policy_version: corpus_documents.policy_version.clone(),
        vector_dimension: corpus_embedding.vector_dimension,
        normalization: "l2".to_owned(),
    };
    let declarations = corpus_documents
        .documents
        .iter()
        .filter_map(|document| {
            let vector = corpus_vectors.get(&document.declaration_name)?;
            Some(VectorDeclaration {
                declaration_id: document.declaration_name.clone(),
                declaration_name: document.declaration_name.clone(),
                module_name: document.module_name.clone(),
                declaration_kind: document.declaration_kind.clone(),
                content_hash: document.content_hash.clone(),
                vector: vector.clone(),
            })
        })
        .collect::<Vec<_>>();

    let build_started = Instant::now();
    let build = match build_vector_corpus(VectorCorpusBuildRequest {
        cache_root: request.corpus_cache_root.clone(),
        provenance: provenance.clone(),
        declarations,
    }) {
        Ok(output) => output,
        Err(error) => {
            summary.status = SearchVectorCandidateStatus::Failed;
            summary.reason = Some(stable_vector_error(&error));
            return VectorCandidateOutput {
                summary,
                candidates: Vec::new(),
            };
        }
    };
    summary.corpus_build_ms = build.counters.build_ms.max(build_started.elapsed().as_millis());
    summary.corpus_status = Some(build.summary.status.into());
    let corpus = match open_vector_corpus(VectorCorpusOpenRequest {
        cache_root: request.corpus_cache_root.clone(),
        provenance,
    }) {
        Ok(corpus) => corpus,
        Err(error) => {
            summary.status = SearchVectorCandidateStatus::Failed;
            summary.reason = Some(stable_vector_error(&error));
            return VectorCandidateOutput {
                summary,
                candidates: Vec::new(),
            };
        }
    };

    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for query_document in &query_documents.documents {
        let Some(query_vector) = query_vectors.get(&query_document.declaration_name) else {
            continue;
        };
        let query_started = Instant::now();
        let nearest = match query_vector_corpus(
            &corpus,
            &VectorCorpusQueryRequest {
                query_vector: query_vector.clone(),
                limit: VECTOR_CANDIDATE_TOP_K.saturating_add(1),
            },
        ) {
            Ok(output) => output,
            Err(error) => {
                summary.status = SearchVectorCandidateStatus::Failed;
                summary.reason = Some(stable_vector_error(&error));
                return VectorCandidateOutput {
                    summary,
                    candidates: Vec::new(),
                };
            }
        };
        summary.query_ms = summary.query_ms.saturating_add(nearest.counters.query_ms);
        summary.query_ms = summary.query_ms.saturating_add(query_started.elapsed().as_millis());
        let mut rank = 0;
        for candidate in nearest.nearest {
            if candidate.declaration_name == query_document.declaration_name {
                continue;
            }
            let Some(declaration) = corpus_by_name.get(&candidate.declaration_name) else {
                continue;
            };
            rank += 1;
            if rank > VECTOR_CANDIDATE_TOP_K {
                break;
            }
            let key = pair_key(&query_document.declaration_name, &candidate.declaration_name);
            if seen.insert(key) {
                candidates.push(VectorCandidate {
                    anchor_name: query_document.declaration_name.clone(),
                    declaration: declaration.clone(),
                    score: candidate.score,
                    rank,
                });
            }
        }
    }
    candidates.sort_by(|left, right| {
        left.anchor_name
            .cmp(&right.anchor_name)
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| left.declaration.qualified_name.cmp(&right.declaration.qualified_name))
    });
    summary.status = SearchVectorCandidateStatus::Ok;
    summary.reason = None;
    summary.vector_generated_candidate_count = candidates.len();
    VectorCandidateOutput { summary, candidates }
}

fn embed_documents(
    request: &SearchVectorCandidateRequest,
    model: &EmbeddingModelSpec,
    role: EmbeddingInputRole,
    documents: &SearchEmbeddingDocuments,
) -> Result<lean_dup_embedding::TextEmbeddingBatchResult, lean_dup_embedding::Error> {
    embed_text_batch(TextEmbeddingBatchRequest {
        model: model.clone(),
        role,
        input_policy: EmbeddingInputPolicy {
            policy_id: documents.policy_id.clone(),
            version: documents.policy_version.clone(),
            includes_declaration_name: documents.policy_id == "name-and-formal-statement",
            includes_normalized_statement: documents.policy_id != "informal-or-formal",
            uses_informal_text_when_available: documents.policy_id == "informal-or-formal",
        },
        inputs: documents
            .text_inputs()
            .into_iter()
            .map(|input| TextEmbeddingInput {
                id: input.declaration_name,
                text: input.text,
            })
            .collect(),
        model_cache_root: request.model_cache_root.clone(),
        vector_cache_root: request.text_vector_cache_root.clone(),
    })
}

fn source_corpus_fingerprint(documents: &SearchEmbeddingDocuments) -> String {
    let mut hasher = Sha256::new();
    hasher.update(documents.policy_id.as_bytes());
    hasher.update([0]);
    hasher.update(documents.policy_version.as_bytes());
    for document in &documents.documents {
        hasher.update([0]);
        hasher.update(document.declaration_name.as_bytes());
        hasher.update([0]);
        hasher.update(document.content_hash.as_bytes());
    }
    hex_bytes(&hasher.finalize())
}

fn fallback_model_fingerprint(id: &str, revision: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update([0]);
    if let Some(revision) = revision {
        hasher.update(revision.as_bytes());
    }
    hex_bytes(&hasher.finalize())
}

fn stable_embedding_error(error: &lean_dup_embedding::Error) -> String {
    match error {
        lean_dup_embedding::Error::EmptyModelId => "empty-model-id",
        lean_dup_embedding::Error::EmptyRevision => "empty-model-revision",
        lean_dup_embedding::Error::ModelNotPrepared { .. } => "vector-model-not-prepared",
        lean_dup_embedding::Error::UnsupportedModel { .. } => "unsupported-model-profile",
        lean_dup_embedding::Error::Io { .. } => "embedding-runtime-io",
        lean_dup_embedding::Error::Json { .. } => "embedding-runtime-json",
        lean_dup_embedding::Error::Runtime { .. } => "embedding-runtime-failed",
        lean_dup_embedding::Error::VectorCache { .. } => "embedding-vector-cache-failed",
        lean_dup_embedding::Error::InvalidVector { .. } => "embedding-vector-invalid",
    }
    .to_owned()
}

fn stable_vector_error(error: &lean_dup_vector_index::VectorIndexError) -> String {
    match error {
        lean_dup_vector_index::VectorIndexError::InvalidRequest { .. } => "vector-corpus-invalid-request",
        lean_dup_vector_index::VectorIndexError::CorpusUnavailable { status, .. } => match status {
            VectorCorpusStatus::Built => "vector-corpus-built-unavailable",
            VectorCorpusStatus::Reused => "vector-corpus-reused-unavailable",
            VectorCorpusStatus::Missing => "vector-corpus-missing",
            VectorCorpusStatus::Stale => "vector-corpus-stale",
            VectorCorpusStatus::Unusable => "vector-corpus-unusable",
        },
        lean_dup_vector_index::VectorIndexError::Storage { .. } => "vector-corpus-storage-failed",
        lean_dup_vector_index::VectorIndexError::Manifest { .. } => "vector-corpus-manifest-failed",
        lean_dup_vector_index::VectorIndexError::Io { .. } => "vector-corpus-io-failed",
    }
    .to_owned()
}

fn pair_key(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

impl From<SearchVectorAcquisitionPolicy> for EmbeddingAcquisitionPolicy {
    fn from(value: SearchVectorAcquisitionPolicy) -> Self {
        match value {
            SearchVectorAcquisitionPolicy::CacheOnly => Self::CacheOnly,
            SearchVectorAcquisitionPolicy::DownloadIfMissing => Self::DownloadIfMissing,
        }
    }
}

impl From<VectorCorpusStatus> for SearchVectorCorpusStatus {
    fn from(value: VectorCorpusStatus) -> Self {
        match value {
            VectorCorpusStatus::Built => Self::Built,
            VectorCorpusStatus::Reused => Self::Reused,
            VectorCorpusStatus::Missing => Self::Missing,
            VectorCorpusStatus::Stale => Self::Stale,
            VectorCorpusStatus::Unusable => Self::Unusable,
        }
    }
}
