use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use lean_dup_diagnostics::progress::Reporter;
use lean_dup_embedding::{
    EmbeddingAcquisitionPolicy, EmbeddingInputFormat, EmbeddingInputRole, EmbeddingModelSpec, EmbeddingPrepareRequest,
    TextEmbeddingBatchRequest, TextEmbeddingInput, embed_text_batch, model_spec_for_profile, prepare_embedding_model,
};
use lean_dup_index::HydratedDeclaration;
use lean_dup_vector_index::{
    PreparedVectorCorpus, VectorCorpusPrepareRequest, VectorCorpusProvenance, VectorCorpusStatus, VectorDeclaration,
    VectorNearestDeclaration, prepare_vector_corpus,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::documents::{self, DocumentPolicy, SemanticDocuments};
use crate::eligibility::{self, EligibilityPolicy, EligibilitySummary};
use crate::{Error, Result, VectorAcquisitionPolicy, VectorValidationRequest};

const VECTOR_CANDIDATE_POLICY_VERSION: &str = "lean-dup.vector-candidate.v2";
pub(crate) const VECTOR_CANDIDATE_TOP_K: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct VectorCandidateOutput {
    pub(crate) summary: VectorCandidateSummary,
    pub(crate) candidates: Vec<VectorCandidate>,
}

#[derive(Debug, Clone)]
pub(crate) struct VectorCandidate {
    pub(crate) anchor_name: String,
    pub(crate) anchor_content_hash: String,
    pub(crate) declaration: HydratedDeclaration,
    pub(crate) declaration_content_hash: String,
    pub(crate) score: f32,
    pub(crate) rank: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorCandidateSummary {
    pub(crate) version: &'static str,
    pub(crate) status: VectorCandidateStatus,
    pub(crate) reason: Option<String>,
    pub(crate) model_id: String,
    pub(crate) model_profile_id: String,
    pub(crate) input_format_id: String,
    pub(crate) input_format_version: String,
    pub(crate) acquisition_policy: VectorAcquisitionPolicy,
    pub(crate) document_policy_id: String,
    pub(crate) document_policy_version: String,
    pub(crate) corpus_status: Option<VectorCorpusStatus>,
    pub(crate) query_declaration_count: usize,
    pub(crate) corpus_declaration_count: usize,
    pub(crate) query_eligibility: EligibilitySummary,
    pub(crate) corpus_eligibility: EligibilitySummary,
    pub(crate) query_document_content: documents::ContentAvailability,
    pub(crate) corpus_document_content: documents::ContentAvailability,
    pub(crate) top_k: usize,
    pub(crate) eligible_corpus_size: usize,
    pub(crate) top_k_saturated: bool,
    pub(crate) vector_generated_candidate_count: usize,
    pub(crate) model_prepare_ms: u128,
    pub(crate) corpus_build_ms: u128,
    pub(crate) corpus_open_ms: u128,
    pub(crate) query_ms: u128,
    pub(crate) embedding_ms: u128,
    pub(crate) total_ms: u128,
    pub(crate) model_cache_bytes: Option<u64>,
    pub(crate) text_vector_cache_bytes: Option<u64>,
    pub(crate) vector_corpus_bytes: Option<u64>,
}

impl Default for VectorCandidateSummary {
    fn default() -> Self {
        Self {
            version: VECTOR_CANDIDATE_POLICY_VERSION,
            status: VectorCandidateStatus::Skipped,
            reason: None,
            model_id: String::new(),
            model_profile_id: String::new(),
            input_format_id: "asymmetric-query-document".to_owned(),
            input_format_version: lean_dup_embedding::EMBEDDING_INPUT_FORMAT_VERSION.to_owned(),
            acquisition_policy: VectorAcquisitionPolicy::CacheOnly,
            document_policy_id: "name-and-statement".to_owned(),
            document_policy_version: documents::DOCUMENT_POLICY_VERSION.to_owned(),
            corpus_status: None,
            query_declaration_count: 0,
            corpus_declaration_count: 0,
            query_eligibility: EligibilitySummary::default(),
            corpus_eligibility: EligibilitySummary::default(),
            query_document_content: documents::ContentAvailability::default(),
            corpus_document_content: documents::ContentAvailability::default(),
            top_k: VECTOR_CANDIDATE_TOP_K,
            eligible_corpus_size: 0,
            top_k_saturated: false,
            vector_generated_candidate_count: 0,
            model_prepare_ms: 0,
            corpus_build_ms: 0,
            corpus_open_ms: 0,
            query_ms: 0,
            embedding_ms: 0,
            total_ms: 0,
            model_cache_bytes: None,
            text_vector_cache_bytes: None,
            vector_corpus_bytes: None,
        }
    }
}

impl VectorCandidateSummary {
    pub(crate) fn skipped_for_budget(
        request: &VectorValidationRequest,
        query_count: usize,
        corpus_count: usize,
        reason: String,
    ) -> Self {
        Self {
            status: VectorCandidateStatus::Skipped,
            reason: Some(reason),
            model_profile_id: request.profile_id.clone(),
            input_format_id: request.input_format_id.clone(),
            acquisition_policy: request.acquisition_policy,
            document_policy_id: request.document_policy_id.clone(),
            query_declaration_count: query_count,
            corpus_declaration_count: corpus_count,
            top_k_saturated: VECTOR_CANDIDATE_TOP_K >= corpus_count,
            eligible_corpus_size: corpus_count,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VectorCandidateStatus {
    Skipped,
    Failed,
    Ok,
}

pub(crate) fn generate(
    request: &VectorValidationRequest,
    workspace: &[HydratedDeclaration],
    comparison_declarations: &[HydratedDeclaration],
    reporter: &mut Reporter,
) -> Result<VectorCandidateOutput> {
    let total_started = Instant::now();
    reporter.event(
        "vector-search.declarations",
        None,
        None,
        "starting vector candidate generation",
    );

    let input_format = input_format(&request.input_format_id)?;
    let document_policy = DocumentPolicy::from_id(&request.document_policy_id)?;
    let eligibility_policy = EligibilityPolicy::from_id(&request.eligibility_policy_id)?;
    let corpus_declarations = if comparison_declarations.is_empty() {
        workspace
    } else {
        comparison_declarations
    };

    let query_eligibility = eligibility::filter(workspace, eligibility_policy);
    let corpus_eligibility = eligibility::filter(corpus_declarations, eligibility_policy);
    let query_documents = documents::build(document_policy, &query_eligibility.declarations);
    let corpus_documents = documents::build(document_policy, &corpus_eligibility.declarations);
    let mut summary = empty_summary(
        request,
        document_policy,
        &query_eligibility,
        &corpus_eligibility,
        &query_documents,
        &corpus_documents,
    );

    if query_documents.documents.is_empty() {
        summary.status = VectorCandidateStatus::Skipped;
        summary.reason = Some("no-eligible-vector-queries".to_owned());
        summary.total_ms = total_started.elapsed().as_millis();
        return Ok(VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        });
    }
    if corpus_documents.documents.is_empty() {
        summary.status = VectorCandidateStatus::Skipped;
        summary.reason = Some("no-eligible-vector-corpus".to_owned());
        summary.total_ms = total_started.elapsed().as_millis();
        return Ok(VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        });
    }

    let model = model_spec_for_profile(&request.profile_id, request.revision.clone())?;
    let prepare_started = Instant::now();
    let prepare = prepare_embedding_model(EmbeddingPrepareRequest {
        model: model.clone(),
        acquisition_policy: request.acquisition_policy.into(),
        cache_root: request.model_cache_root.clone(),
    })?;
    summary.model_prepare_ms = prepare.elapsed_ms.max(prepare_started.elapsed().as_millis());
    summary.model_id = prepare.model.id.clone();
    summary.model_profile_id = prepare.model.profile_id.clone();
    summary.model_cache_bytes = prepare.total_bytes;

    let corpus_embedding = embed_documents(
        request,
        &model,
        input_format,
        EmbeddingInputRole::Document,
        &corpus_documents,
    )?;
    let query_embedding = embed_documents(
        request,
        &model,
        input_format,
        EmbeddingInputRole::Query,
        &query_documents,
    )?;
    summary.embedding_ms = corpus_embedding
        .runtime
        .model_load_ms
        .saturating_add(corpus_embedding.runtime.inference_ms)
        .saturating_add(query_embedding.runtime.model_load_ms)
        .saturating_add(query_embedding.runtime.inference_ms);
    summary.input_format_id = corpus_embedding.input_format.id.clone();
    summary.input_format_version = corpus_embedding.input_format.version.clone();
    summary.text_vector_cache_bytes = request.text_vector_cache_root.as_deref().map(disk_bytes);

    let corpus_vectors = corpus_embedding
        .vectors
        .into_iter()
        .map(|vector| (vector.input_id, vector.values))
        .collect::<BTreeMap<_, _>>();
    let query_vectors = query_embedding
        .vectors
        .into_iter()
        .map(|vector| (vector.input_id, vector.values))
        .collect::<BTreeMap<_, _>>();
    let corpus_by_name = corpus_eligibility
        .declarations
        .iter()
        .map(|declaration| (declaration.qualified_name.as_str(), declaration))
        .collect::<BTreeMap<_, _>>();
    let document_hash_by_name = corpus_documents
        .documents
        .iter()
        .map(|document| (document.declaration_name.as_str(), document.content_hash.as_str()))
        .collect::<BTreeMap<_, _>>();

    let provenance = VectorCorpusProvenance {
        source_corpus_fingerprint: source_corpus_fingerprint(&corpus_documents),
        embedding_model_profile_id: corpus_embedding.model.profile_id.clone(),
        embedding_model_fingerprint: corpus_embedding.model.fingerprint.clone().unwrap_or_else(|| {
            fallback_model_fingerprint(&corpus_embedding.model.id, corpus_embedding.model.revision.as_deref())
        }),
        embedding_input_format_id: corpus_embedding.input_format.id.clone(),
        embedding_input_format_version: corpus_embedding.input_format.version.clone(),
        document_policy_id: corpus_documents.policy.id().to_owned(),
        document_policy_version: documents::DOCUMENT_POLICY_VERSION.to_owned(),
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
    let corpus_started = Instant::now();
    let corpus = prepare_vector_corpus(VectorCorpusPrepareRequest {
        cache_root: request.corpus_cache_root.clone(),
        provenance,
        declarations,
    })?;
    summary.corpus_build_ms = corpus
        .prepare_counters()
        .build_ms
        .max(corpus_started.elapsed().as_millis());
    summary.corpus_open_ms = corpus.prepare_counters().open_ms;
    summary.corpus_status = Some(corpus.summary().status);
    summary.vector_corpus_bytes = Some(disk_bytes(&request.corpus_cache_root));

    let candidates = query_all(
        &corpus,
        &query_documents,
        &query_vectors,
        &corpus_by_name,
        &document_hash_by_name,
        reporter,
    )?;
    summary.query_ms = candidates.query_ms;
    summary.vector_generated_candidate_count = candidates.candidates.len();
    summary.status = VectorCandidateStatus::Ok;
    summary.total_ms = total_started.elapsed().as_millis();
    Ok(VectorCandidateOutput {
        summary,
        candidates: candidates.candidates,
    })
}

fn empty_summary(
    request: &VectorValidationRequest,
    policy: DocumentPolicy,
    query_eligibility: &eligibility::EligibleDeclarations,
    corpus_eligibility: &eligibility::EligibleDeclarations,
    query_documents: &SemanticDocuments,
    corpus_documents: &SemanticDocuments,
) -> VectorCandidateSummary {
    VectorCandidateSummary {
        version: VECTOR_CANDIDATE_POLICY_VERSION,
        status: VectorCandidateStatus::Failed,
        reason: None,
        model_id: String::new(),
        model_profile_id: request.profile_id.clone(),
        input_format_id: request.input_format_id.clone(),
        input_format_version: lean_dup_embedding::EMBEDDING_INPUT_FORMAT_VERSION.to_owned(),
        acquisition_policy: request.acquisition_policy,
        document_policy_id: policy.id().to_owned(),
        document_policy_version: documents::DOCUMENT_POLICY_VERSION.to_owned(),
        corpus_status: None,
        query_declaration_count: query_documents.documents.len(),
        corpus_declaration_count: corpus_documents.documents.len(),
        query_eligibility: query_eligibility.summary.clone(),
        corpus_eligibility: corpus_eligibility.summary.clone(),
        query_document_content: query_documents.availability.clone(),
        corpus_document_content: corpus_documents.availability.clone(),
        top_k: VECTOR_CANDIDATE_TOP_K,
        eligible_corpus_size: corpus_documents.documents.len(),
        top_k_saturated: VECTOR_CANDIDATE_TOP_K >= corpus_documents.documents.len(),
        vector_generated_candidate_count: 0,
        model_prepare_ms: 0,
        corpus_build_ms: 0,
        corpus_open_ms: 0,
        query_ms: 0,
        embedding_ms: 0,
        total_ms: 0,
        model_cache_bytes: None,
        text_vector_cache_bytes: None,
        vector_corpus_bytes: None,
    }
}

struct QueryAllOutput {
    candidates: Vec<VectorCandidate>,
    query_ms: u128,
}

fn query_all(
    corpus: &PreparedVectorCorpus,
    query_documents: &SemanticDocuments,
    query_vectors: &BTreeMap<String, Vec<f32>>,
    corpus_by_name: &BTreeMap<&str, &HydratedDeclaration>,
    document_hash_by_name: &BTreeMap<&str, &str>,
    reporter: &mut Reporter,
) -> Result<QueryAllOutput> {
    let mut candidates = BTreeMap::<(String, String), VectorCandidate>::new();
    let mut query_ms: u128 = 0;
    for (query_index, query_document) in query_documents.documents.iter().enumerate() {
        reporter.event(
            "vector-search.query",
            Some(u64::try_from(query_index).unwrap_or(u64::MAX)),
            Some(u64::try_from(query_documents.documents.len()).unwrap_or(u64::MAX)),
            "querying vector corpus",
        );
        let Some(query_vector) = query_vectors.get(&query_document.declaration_name) else {
            continue;
        };
        let started = Instant::now();
        let nearest = corpus.query(&lean_dup_vector_index::VectorCorpusQueryRequest {
            query_vector: query_vector.clone(),
            limit: VECTOR_CANDIDATE_TOP_K,
        })?;
        query_ms = query_ms.saturating_add(nearest.counters.query_ms.max(started.elapsed().as_millis()));
        for (offset, candidate) in nearest.nearest.iter().enumerate() {
            if candidate.declaration_name == query_document.declaration_name {
                continue;
            }
            let Some(declaration) = corpus_by_name.get(candidate.declaration_name.as_str()) else {
                continue;
            };
            let key = pair_key(&query_document.declaration_name, &candidate.declaration_name);
            let vector = vector_candidate(
                query_document,
                candidate,
                declaration,
                document_hash_by_name,
                offset + 1,
            );
            candidates
                .entry(key)
                .and_modify(|current| {
                    if should_replace(current, &vector) {
                        *current = vector.clone();
                    }
                })
                .or_insert(vector);
        }
    }
    Ok(QueryAllOutput {
        candidates: candidates.into_values().collect(),
        query_ms,
    })
}

fn vector_candidate(
    query_document: &documents::SemanticDocument,
    candidate: &VectorNearestDeclaration,
    declaration: &HydratedDeclaration,
    document_hash_by_name: &BTreeMap<&str, &str>,
    rank: usize,
) -> VectorCandidate {
    VectorCandidate {
        anchor_name: query_document.declaration_name.clone(),
        anchor_content_hash: query_document.content_hash.clone(),
        declaration: (*declaration).clone(),
        declaration_content_hash: document_hash_by_name
            .get(candidate.declaration_name.as_str())
            .copied()
            .unwrap_or(candidate.content_hash.as_str())
            .to_owned(),
        score: candidate.score,
        rank,
    }
}

fn should_replace(current: &VectorCandidate, next: &VectorCandidate) -> bool {
    next.rank < current.rank || (next.rank == current.rank && next.score > current.score)
}

fn embed_documents(
    request: &VectorValidationRequest,
    model: &EmbeddingModelSpec,
    input_format: EmbeddingInputFormat,
    role: EmbeddingInputRole,
    documents: &SemanticDocuments,
) -> Result<lean_dup_embedding::TextEmbeddingBatchResult> {
    Ok(embed_text_batch(TextEmbeddingBatchRequest {
        model: model.clone(),
        role,
        input_format,
        input_policy: documents.policy.embedding_policy(),
        inputs: documents
            .documents
            .iter()
            .map(|document| TextEmbeddingInput {
                id: document.declaration_name.clone(),
                text: document.text.clone(),
            })
            .collect(),
        model_cache_root: request.model_cache_root.clone(),
        vector_cache_root: request.text_vector_cache_root.clone(),
    })?)
}

fn source_corpus_fingerprint(documents: &SemanticDocuments) -> String {
    let mut hasher = Sha256::new();
    hasher.update(documents.policy.id().as_bytes());
    hasher.update(documents::DOCUMENT_POLICY_VERSION.as_bytes());
    for document in &documents.documents {
        hasher.update(document.declaration_name.as_bytes());
        hasher.update(document.content_hash.as_bytes());
    }
    documents::hex_bytes(&hasher.finalize())
}

fn fallback_model_fingerprint(id: &str, revision: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update(revision.unwrap_or("default").as_bytes());
    documents::hex_bytes(&hasher.finalize())
}

fn input_format(id: &str) -> Result<EmbeddingInputFormat> {
    match id {
        "symmetric-document" => Ok(EmbeddingInputFormat::SymmetricDocument),
        "asymmetric-query-document" => Ok(EmbeddingInputFormat::AsymmetricQueryDocument),
        other => Err(Error::InvalidRequest {
            message: format!("unsupported input format: {other}"),
        }),
    }
}

fn pair_key(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

fn disk_bytes(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

impl From<VectorAcquisitionPolicy> for EmbeddingAcquisitionPolicy {
    fn from(value: VectorAcquisitionPolicy) -> Self {
        match value {
            VectorAcquisitionPolicy::CacheOnly => Self::CacheOnly,
            VectorAcquisitionPolicy::DownloadIfMissing => Self::DownloadIfMissing,
        }
    }
}
