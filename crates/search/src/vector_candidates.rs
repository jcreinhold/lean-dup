use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use lean_dup_embedding::{
    EmbeddingAcquisitionPolicy, EmbeddingCacheStatus, EmbeddingInputFormat, EmbeddingInputPolicy, EmbeddingInputRole,
    EmbeddingModelSpec, EmbeddingPrepareRequest, TextEmbeddingBatchRequest, TextEmbeddingInput, embed_text_batch,
    model_spec_for_profile, prepare_embedding_model,
};
use lean_dup_index::HydratedDeclaration;
use lean_dup_vector_index::{
    VectorCorpusBuildRequest, VectorCorpusOpenRequest, VectorCorpusProvenance, VectorCorpusQueryRequest,
    VectorCorpusStatus, VectorDeclaration, build_vector_corpus, open_vector_corpus, query_vector_corpus,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::observation::{
    SearchEmbeddingContentAvailability, SearchEmbeddingDocumentPolicy, SearchEmbeddingDocuments,
    embedding_documents_for_declarations_with_policy,
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
    pub profile_id: String,
    pub revision: Option<String>,
    pub input_format: SearchVectorInputFormat,
    pub acquisition_policy: SearchVectorAcquisitionPolicy,
    pub model_cache_root: Option<PathBuf>,
    pub text_vector_cache_root: Option<PathBuf>,
    pub corpus_cache_root: PathBuf,
    pub document_policy: SearchEmbeddingDocumentPolicy,
    pub eligibility_policy: SearchVectorEligibilityPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchVectorAcquisitionPolicy {
    CacheOnly,
    DownloadIfMissing,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchVectorInputFormat {
    SymmetricDocument,
    #[default]
    AsymmetricQueryDocument,
}

impl SearchVectorInputFormat {
    pub fn id(self) -> &'static str {
        match self {
            Self::SymmetricDocument => "symmetric-document",
            Self::AsymmetricQueryDocument => "asymmetric-query-document",
        }
    }

    pub fn version(self) -> &'static str {
        "lean-dup.embedding-input-format.v1"
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchVectorEligibilityPolicy {
    #[default]
    ActionablePublicStatement,
    Broad,
}

impl SearchVectorEligibilityPolicy {
    pub fn id(self) -> &'static str {
        match self {
            Self::ActionablePublicStatement => "actionable-public-statement",
            Self::Broad => "broad",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchVectorEligibilitySummary {
    pub policy_id: String,
    pub policy_version: &'static str,
    pub total: usize,
    pub eligible: usize,
    pub skipped_by_reason: BTreeMap<String, usize>,
}

/// Stable vector candidate-generation facts exposed to eval artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchVectorCandidateSummary {
    pub version: &'static str,
    pub status: SearchVectorCandidateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub model_id: String,
    pub model_profile_id: String,
    pub input_format_id: String,
    pub input_format_version: String,
    pub acquisition_policy: SearchVectorAcquisitionPolicy,
    pub document_policy_id: String,
    pub document_policy_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_status: Option<SearchVectorCorpusStatus>,
    pub query_declaration_count: usize,
    pub corpus_declaration_count: usize,
    pub query_eligibility: SearchVectorEligibilitySummary,
    pub corpus_eligibility: SearchVectorEligibilitySummary,
    pub query_document_content: SearchEmbeddingContentAvailability,
    pub corpus_document_content: SearchEmbeddingContentAvailability,
    pub top_k: usize,
    pub eligible_corpus_size: usize,
    pub top_k_saturated: bool,
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
            model_profile_id: String::new(),
            input_format_id: SearchVectorInputFormat::default().id().to_owned(),
            input_format_version: SearchVectorInputFormat::default().version().to_owned(),
            acquisition_policy: SearchVectorAcquisitionPolicy::CacheOnly,
            document_policy_id: String::new(),
            document_policy_version: String::new(),
            corpus_status: None,
            query_declaration_count: 0,
            corpus_declaration_count: 0,
            query_eligibility: SearchVectorEligibilitySummary::default(),
            corpus_eligibility: SearchVectorEligibilitySummary::default(),
            query_document_content: SearchEmbeddingContentAvailability::default(),
            corpus_document_content: SearchEmbeddingContentAvailability::default(),
            top_k: VECTOR_CANDIDATE_TOP_K,
            eligible_corpus_size: 0,
            top_k_saturated: false,
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
    pub(crate) anchor_content_hash: String,
    pub(crate) declaration: HydratedDeclaration,
    pub(crate) declaration_content_hash: String,
    pub(crate) score: f32,
    pub(crate) rank: usize,
}

struct EligibleDeclarations {
    summary: SearchVectorEligibilitySummary,
    declarations: Vec<HydratedDeclaration>,
}

pub(crate) fn generate_vector_candidates(
    request: &SearchVectorCandidateRequest,
    workspace: &[HydratedDeclaration],
    comparison_declarations: &[HydratedDeclaration],
) -> VectorCandidateOutput {
    let corpus_declarations = if comparison_declarations.is_empty() {
        workspace
    } else {
        comparison_declarations
    };
    let query_eligibility = eligible_declarations(workspace, request.eligibility_policy);
    let corpus_eligibility = eligible_declarations(corpus_declarations, request.eligibility_policy);
    let query_documents =
        embedding_documents_for_declarations_with_policy(&query_eligibility.declarations, request.document_policy);
    let corpus_documents =
        embedding_documents_for_declarations_with_policy(&corpus_eligibility.declarations, request.document_policy);
    let mut summary = SearchVectorCandidateSummary {
        version: VECTOR_CANDIDATE_POLICY_VERSION,
        status: SearchVectorCandidateStatus::Skipped,
        reason: None,
        model_id: String::new(),
        model_profile_id: request.profile_id.clone(),
        input_format_id: request.input_format.id().to_owned(),
        input_format_version: request.input_format.version().to_owned(),
        acquisition_policy: request.acquisition_policy,
        document_policy_id: query_documents.policy_id.clone(),
        document_policy_version: query_documents.policy_version.clone(),
        corpus_status: None,
        query_declaration_count: query_documents.documents.len(),
        corpus_declaration_count: corpus_documents.documents.len(),
        query_eligibility: query_eligibility.summary,
        corpus_eligibility: corpus_eligibility.summary,
        query_document_content: query_documents.content_availability.clone(),
        corpus_document_content: corpus_documents.content_availability.clone(),
        top_k: VECTOR_CANDIDATE_TOP_K,
        eligible_corpus_size: corpus_documents.documents.len(),
        top_k_saturated: VECTOR_CANDIDATE_TOP_K >= corpus_documents.documents.len(),
        vector_generated_candidate_count: 0,
        corpus_build_ms: 0,
        query_ms: 0,
        embedding_ms: 0,
    };
    if workspace.is_empty() || corpus_declarations.is_empty() {
        summary.reason = Some("empty-vector-input".to_owned());
        return VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        };
    }
    if query_documents.documents.is_empty() {
        summary.reason = Some("no-eligible-vector-queries".to_owned());
        return VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        };
    }
    if corpus_documents.documents.is_empty() {
        summary.reason = Some("no-eligible-vector-corpus".to_owned());
        return VectorCandidateOutput {
            summary,
            candidates: Vec::new(),
        };
    }

    let model = match model_spec_for_profile(&request.profile_id, request.revision.clone()) {
        Ok(model) => {
            summary.model_id = model.id.clone();
            model
        }
        Err(error) => {
            summary.status = SearchVectorCandidateStatus::Failed;
            summary.reason = Some(stable_embedding_error(&error));
            return VectorCandidateOutput {
                summary,
                candidates: Vec::new(),
            };
        }
    };

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
    summary.model_profile_id = prepare.model.profile_id.clone();
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
    summary.model_profile_id = corpus_embedding.model.profile_id.clone();
    summary.input_format_id = corpus_embedding.input_format.id.clone();
    summary.input_format_version = corpus_embedding.input_format.version.clone();

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
    let corpus_by_name = corpus_eligibility
        .declarations
        .iter()
        .map(|declaration| (declaration.qualified_name.clone(), declaration.clone()))
        .collect::<BTreeMap<_, _>>();

    let provenance = VectorCorpusProvenance {
        source_corpus_fingerprint: source_corpus_fingerprint(&corpus_documents),
        embedding_model_profile_id: corpus_embedding.model.profile_id.clone(),
        embedding_model_fingerprint: corpus_embedding.model.fingerprint.clone().unwrap_or_else(|| {
            fallback_model_fingerprint(&corpus_embedding.model.id, corpus_embedding.model.revision.as_deref())
        }),
        embedding_input_format_id: corpus_embedding.input_format.id.clone(),
        embedding_input_format_version: corpus_embedding.input_format.version.clone(),
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
                limit: summary.top_k.saturating_add(1),
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
            if rank > summary.top_k {
                break;
            }
            let key = pair_key(&query_document.declaration_name, &candidate.declaration_name);
            if seen.insert(key) {
                candidates.push(VectorCandidate {
                    anchor_name: query_document.declaration_name.clone(),
                    anchor_content_hash: query_document.content_hash.clone(),
                    declaration: declaration.clone(),
                    declaration_content_hash: candidate.content_hash,
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

fn eligible_declarations(
    declarations: &[HydratedDeclaration],
    policy: SearchVectorEligibilityPolicy,
) -> EligibleDeclarations {
    let mut summary = SearchVectorEligibilitySummary {
        policy_id: policy.id().to_owned(),
        policy_version: VECTOR_CANDIDATE_POLICY_VERSION,
        total: declarations.len(),
        eligible: 0,
        skipped_by_reason: BTreeMap::new(),
    };
    let mut eligible = Vec::new();
    for declaration in declarations {
        match vector_skip_reason(declaration, policy) {
            Some(reason) => {
                *summary.skipped_by_reason.entry(reason.to_owned()).or_default() += 1;
            }
            None => {
                summary.eligible += 1;
                eligible.push(declaration.clone());
            }
        }
    }
    EligibleDeclarations {
        summary,
        declarations: eligible,
    }
}

fn vector_skip_reason(
    declaration: &HydratedDeclaration,
    policy: SearchVectorEligibilityPolicy,
) -> Option<&'static str> {
    let missing_statement = declaration.statement_text.split_whitespace().next().is_none();
    match policy {
        SearchVectorEligibilityPolicy::Broad => missing_statement.then_some("missing-statement"),
        SearchVectorEligibilityPolicy::ActionablePublicStatement => {
            if declaration.status_flags.iter().any(|flag| flag == "generated") {
                return Some("generated");
            }
            if declaration.visibility != "public" {
                return Some("private");
            }
            if is_synthetic_declaration(declaration) {
                return Some("synthetic");
            }
            if !declaration.low_signal_markers.is_empty() {
                return Some("low-signal");
            }
            if missing_statement {
                return Some("missing-statement");
            }
            if is_not_actionable_declaration(declaration) {
                return Some("not-actionable");
            }
            if !is_supported_vector_kind(declaration) {
                return Some("unsupported-kind");
            }
            None
        }
    }
}

fn is_synthetic_declaration(declaration: &HydratedDeclaration) -> bool {
    declaration.declaration_id.starts_with("synthetic:")
        || declaration.module == "Synthetic"
        || declaration.qualified_name.starts_with("Synthetic.")
}

fn is_not_actionable_declaration(declaration: &HydratedDeclaration) -> bool {
    declaration.kind == "instance" || declaration.display_name.starts_with("inst")
}

fn is_supported_vector_kind(declaration: &HydratedDeclaration) -> bool {
    matches!(
        declaration.kind.as_str(),
        "theorem" | "lemma" | "axiom" | "def" | "definition" | "abbrev"
    )
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
        input_format: request.input_format.into(),
        input_policy: EmbeddingInputPolicy {
            policy_id: documents.policy_id.clone(),
            version: documents.policy_version.clone(),
            includes_declaration_name: matches!(
                documents.policy_id.as_str(),
                "name-and-statement" | "definition-aware" | "docstring-augmented"
            ),
            includes_statement: true,
            includes_definition_body_summary: matches!(
                documents.policy_id.as_str(),
                "definition-aware" | "docstring-augmented"
            ),
            includes_docstring: documents.policy_id == "docstring-augmented",
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

impl From<SearchVectorInputFormat> for EmbeddingInputFormat {
    fn from(value: SearchVectorInputFormat) -> Self {
        match value {
            SearchVectorInputFormat::SymmetricDocument => Self::SymmetricDocument,
            SearchVectorInputFormat::AsymmetricQueryDocument => Self::AsymmetricQueryDocument,
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

#[cfg(test)]
mod tests {
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::Fingerprints;
    use tempfile::TempDir;

    use super::{
        SearchVectorAcquisitionPolicy, SearchVectorCandidateRequest, SearchVectorCandidateStatus,
        SearchVectorEligibilityPolicy, SearchVectorInputFormat, eligible_declarations, generate_vector_candidates,
    };
    use crate::SearchEmbeddingDocumentPolicy;

    #[test]
    fn actionable_public_statement_policy_records_stable_skip_reasons() {
        let rows = vec![
            declaration("Generated.item", "theorem", "public").with_flag("generated"),
            declaration("Private.item", "theorem", "private"),
            declaration("Synthetic.item", "theorem", "public"),
            declaration("LowSignal.item", "theorem", "public").with_low_signal("broad_head:Eq"),
            declaration("Missing.item", "theorem", "public").with_statement(""),
            declaration("Instance.item", "instance", "public"),
            declaration("Struct.item", "structure", "public"),
            declaration("Useful.item", "theorem", "public"),
        ];

        let eligible = eligible_declarations(&rows, SearchVectorEligibilityPolicy::ActionablePublicStatement);

        assert_eq!(eligible.summary.policy_id, "actionable-public-statement");
        assert_eq!(eligible.summary.total, 8);
        assert_eq!(eligible.summary.eligible, 1);
        assert_eq!(
            eligible.summary.skipped_by_reason,
            std::collections::BTreeMap::from([
                ("generated".to_owned(), 1),
                ("private".to_owned(), 1),
                ("synthetic".to_owned(), 1),
                ("low-signal".to_owned(), 1),
                ("missing-statement".to_owned(), 1),
                ("not-actionable".to_owned(), 1),
                ("unsupported-kind".to_owned(), 1),
            ])
        );
        assert_eq!(eligible.declarations[0].qualified_name, "Useful.item");
    }

    #[test]
    fn broad_policy_includes_normally_excluded_declarations_with_text() {
        let rows = vec![
            declaration("Generated.item", "theorem", "public").with_flag("generated"),
            declaration("Private.item", "theorem", "private"),
            declaration("Synthetic.item", "theorem", "public"),
            declaration("LowSignal.item", "theorem", "public").with_low_signal("broad_head:Eq"),
            declaration("Missing.item", "theorem", "public").with_statement(""),
        ];

        let eligible = eligible_declarations(&rows, SearchVectorEligibilityPolicy::Broad);

        assert_eq!(eligible.summary.policy_id, "broad");
        assert_eq!(eligible.summary.total, 5);
        assert_eq!(eligible.summary.eligible, 4);
        assert_eq!(
            eligible.summary.skipped_by_reason,
            std::collections::BTreeMap::from([("missing-statement".to_owned(), 1)])
        );
    }

    #[test]
    fn vector_generation_reports_no_eligible_query_without_model_work() {
        let rows = vec![declaration("Synthetic.item", "theorem", "public")];
        let cache = TempDir::new().unwrap();

        let output = generate_vector_candidates(
            &request(
                cache.path().join("corpus"),
                SearchVectorEligibilityPolicy::ActionablePublicStatement,
            ),
            &rows,
            &[],
        );

        assert_eq!(output.summary.status, SearchVectorCandidateStatus::Skipped);
        assert_eq!(output.summary.reason.as_deref(), Some("no-eligible-vector-queries"));
        assert_eq!(output.summary.query_eligibility.eligible, 0);
        assert_eq!(output.summary.corpus_eligibility.eligible, 0);
        assert!(output.summary.top_k_saturated);
        assert!(output.candidates.is_empty());
    }

    #[test]
    fn vector_summary_records_top_k_saturation_from_eligible_corpus_size() {
        let small = (0..2)
            .map(|index| declaration(&format!("Useful.small_{index}"), "theorem", "public"))
            .collect::<Vec<_>>();
        let large = (0..40)
            .map(|index| declaration(&format!("Useful.large_{index}"), "theorem", "public"))
            .collect::<Vec<_>>();
        let cache = TempDir::new().unwrap();

        let small_output = generate_vector_candidates(
            &request(cache.path().join("small"), SearchVectorEligibilityPolicy::Broad),
            &small,
            &[],
        );
        let large_output = generate_vector_candidates(
            &request(cache.path().join("large"), SearchVectorEligibilityPolicy::Broad),
            &large,
            &[],
        );

        assert_eq!(small_output.summary.top_k, 32);
        assert_eq!(small_output.summary.eligible_corpus_size, 2);
        assert!(small_output.summary.top_k_saturated);
        assert_eq!(large_output.summary.top_k, 32);
        assert_eq!(large_output.summary.eligible_corpus_size, 40);
        assert!(!large_output.summary.top_k_saturated);
    }

    #[test]
    fn realistic_validation_fixture_records_non_saturated_eligibility_contract() {
        let mut rows = (0..72)
            .map(|index| declaration(&format!("VectorFixture.useful_{index:02}"), "theorem", "public"))
            .collect::<Vec<_>>();
        rows.extend([
            declaration("VectorFixture.generated", "theorem", "public").with_flag("generated"),
            declaration("VectorFixture.private_item", "theorem", "private"),
            declaration("Synthetic.fixture_noise", "theorem", "public"),
            declaration("VectorFixture.low_signal", "theorem", "public").with_low_signal("broad_head:Eq"),
            declaration("VectorFixture.missing_statement", "theorem", "public").with_statement(""),
            declaration("VectorFixture.instance_noise", "instance", "public"),
            declaration("VectorFixture.structure_noise", "structure", "public"),
        ]);
        let cache = TempDir::new().unwrap();

        let output = generate_vector_candidates(
            &request(
                cache.path().join("realistic"),
                SearchVectorEligibilityPolicy::ActionablePublicStatement,
            ),
            &rows,
            &[],
        );

        assert_eq!(output.summary.query_eligibility.total, 79);
        assert_eq!(output.summary.query_eligibility.eligible, 72);
        assert_eq!(output.summary.corpus_eligibility.eligible, 72);
        assert_eq!(output.summary.top_k, 32);
        assert_eq!(output.summary.eligible_corpus_size, 72);
        assert!(!output.summary.top_k_saturated);
        assert_eq!(
            output.summary.query_eligibility.skipped_by_reason,
            std::collections::BTreeMap::from([
                ("generated".to_owned(), 1),
                ("private".to_owned(), 1),
                ("synthetic".to_owned(), 1),
                ("low-signal".to_owned(), 1),
                ("missing-statement".to_owned(), 1),
                ("not-actionable".to_owned(), 1),
                ("unsupported-kind".to_owned(), 1),
            ])
        );
        assert_eq!(output.summary.reason.as_deref(), Some("unsupported-model-profile"));
        assert!(output.candidates.is_empty());
    }

    fn request(
        corpus_cache_root: std::path::PathBuf,
        eligibility_policy: SearchVectorEligibilityPolicy,
    ) -> SearchVectorCandidateRequest {
        SearchVectorCandidateRequest {
            profile_id: "unsupported-profile".to_owned(),
            revision: None,
            input_format: SearchVectorInputFormat::AsymmetricQueryDocument,
            acquisition_policy: SearchVectorAcquisitionPolicy::CacheOnly,
            model_cache_root: None,
            text_vector_cache_root: None,
            corpus_cache_root,
            document_policy: SearchEmbeddingDocumentPolicy::NameAndStatement,
            eligibility_policy,
        }
    }

    fn declaration(name: &str, kind: &str, visibility: &str) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::for_test(name),
            declaration_id: format!("workspace:{name}"),
            origin: "workspace".to_owned(),
            module: name.split('.').next().unwrap_or("Fixture").to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap_or(name).to_owned(),
            kind: kind.to_owned(),
            visibility: visibility.to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: "example statement".to_owned(),
            docstring_text: None,
            definition_body_summary: None,
            status_flags: Vec::new(),
            feature_version: "test".to_owned(),
            fingerprints: Fingerprints {
                statement: format!("{name}:statement"),
                safe_binder_permutation: String::new(),
                connective_shape: String::new(),
                conclusion_shape: String::new(),
            },
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: Vec::new(),
        }
    }

    trait DeclarationTestExt {
        fn with_flag(self, flag: &str) -> Self;
        fn with_low_signal(self, marker: &str) -> Self;
        fn with_statement(self, statement: &str) -> Self;
    }

    impl DeclarationTestExt for HydratedDeclaration {
        fn with_flag(mut self, flag: &str) -> Self {
            self.status_flags.push(flag.to_owned());
            self
        }

        fn with_low_signal(mut self, marker: &str) -> Self {
            self.low_signal_markers.push(marker.to_owned());
            self
        }

        fn with_statement(mut self, statement: &str) -> Self {
            self.statement_text = statement.to_owned();
            self
        }
    }
}
