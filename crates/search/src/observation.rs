use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use lean_dup_diagnostics::progress::Reporter;
use lean_dup_index::{HydratedDeclaration, OpenedIndex};

use crate::Result;
use crate::pair_features::{SearchPairFeatures, feature_families, pair_features, vector_evidence};
use crate::retrieval::{
    CandidateExplanation, GeneratedPairEvidence, RetrievalDiagnostics, generated_pair_evidence, retrieve_candidates,
};
use crate::scorer::{
    SearchPairScoring, SearchScoringSummary, SearchScoringVariant, default_summary, score_observation,
};
use crate::semantic_reranking::{
    SearchSemanticObligationYield, SearchSemanticRerankingSummary, summary as semantic_reranking_summary,
};
use crate::vector_candidates::{
    SearchVectorCandidateRequest, SearchVectorCandidateSummary, VectorCandidate, generate_vector_candidates,
};

/// Request for search-stage observations used by offline evaluation.
///
/// The search crate owns retrieval keys and contribution mapping. Evaluation
/// receives stable pair, origin, queue, and feature-family facts without
/// depending on retrieval internals.
pub struct SearchObservationRequest<'a> {
    pub workspace: &'a [HydratedDeclaration],
    pub comparison_indexes: &'a [OpenedIndex],
    pub tracked_pairs: &'a [SearchTrackedPair],
    pub scoring_variant: SearchScoringVariant,
    pub vector_candidates: Option<&'a SearchVectorCandidateRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SearchTrackedPair {
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchObservation {
    pub pairs: Vec<SearchObservedPair>,
    pub visible_groups_found: usize,
    pub visible_groups_total: usize,
    pub scoring: SearchScoringSummary,
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub semantic_obligation_yield: Vec<SearchSemanticObligationYield>,
    pub retrieval: SearchRetrievalObservation,
    #[serde(skip)]
    pub embedding_documents: SearchEmbeddingDocuments,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchRetrievalObservation {
    pub candidate_count: usize,
    pub generated_candidate_count: usize,
    pub ranked_candidate_count: usize,
    pub symbolic_generated_candidate_count: usize,
    pub vector_generated_candidate_count: usize,
    pub merged_generated_candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_feature_fanout_count: usize,
    pub heap_truncations: usize,
    pub candidate_count_by_generation_policy: BTreeMap<String, usize>,
    pub pruned_feature_fanouts: Vec<SearchPrunedFeatureFanout>,
    pub vector_candidates: SearchVectorCandidateSummary,
}

/// Search-owned declaration documents for hidden embedding experiments.
///
/// These documents are intentionally skipped during normal JSON serialization:
/// eval may ask search for document text, but audit/report JSON must not expose
/// normalized statement text or model-input strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEmbeddingDocuments {
    pub policy_id: String,
    pub policy_version: String,
    pub content_availability: SearchEmbeddingContentAvailability,
    pub documents: Vec<SearchEmbeddingDocument>,
}

impl Default for SearchEmbeddingDocuments {
    fn default() -> Self {
        let policy = SearchEmbeddingDocumentPolicy::default();
        Self {
            policy_id: policy.id().to_owned(),
            policy_version: SEARCH_EMBEDDING_DOCUMENT_POLICY_VERSION.to_owned(),
            content_availability: SearchEmbeddingContentAvailability::default(),
            documents: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchEmbeddingContentAvailability {
    pub total: usize,
    pub with_docstring: usize,
    pub with_definition_body_summary: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEmbeddingDocument {
    pub declaration_name: String,
    pub module_name: String,
    pub declaration_kind: String,
    pub normalized_statement: String,
    pub docstring_text: Option<String>,
    pub definition_body_summary: Option<String>,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEmbeddingDocumentInput {
    pub declaration_name: String,
    pub text: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchEmbeddingDocumentPolicy {
    Statement,
    #[default]
    NameAndStatement,
    DefinitionAware,
    DocstringAugmented,
}

impl SearchEmbeddingDocumentPolicy {
    pub fn id(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::NameAndStatement => "name-and-statement",
            Self::DefinitionAware => "definition-aware",
            Self::DocstringAugmented => "docstring-augmented",
        }
    }
}

impl SearchEmbeddingDocuments {
    pub fn text_inputs(&self) -> Vec<SearchEmbeddingDocumentInput> {
        let policy = SearchEmbeddingDocumentPolicy::from_id(&self.policy_id)
            .unwrap_or(SearchEmbeddingDocumentPolicy::NameAndStatement);
        self.documents
            .iter()
            .map(|document| SearchEmbeddingDocumentInput {
                declaration_name: document.declaration_name.clone(),
                text: document_text(document, policy),
                content_hash: document.content_hash.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchObservedPair {
    pub left: String,
    pub right: String,
    pub generated: bool,
    pub symbolic_generated: bool,
    pub vector_generated: bool,
    pub merged_generated: bool,
    pub ranked: bool,
    pub generation_policy: String,
    pub rank: Option<usize>,
    pub shown: bool,
    pub left_content_hash: Option<String>,
    pub right_content_hash: Option<String>,
    pub vector_rank: Option<usize>,
    pub origin: String,
    pub feature_families: Vec<String>,
    pub survived_shown_filter: bool,
    pub features: SearchPairFeatures,
    pub scoring: SearchPairScoring,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchPrunedFeatureFanout {
    pub policy: String,
    pub source: String,
    pub reason: String,
    pub feature_family: String,
    pub count: usize,
}

pub fn observe_search(request: SearchObservationRequest<'_>) -> Result<SearchObservation> {
    observe_search_inner(request, None)
}

/// Run search observation while reporting stable workflow progress.
///
/// Hidden validation workflows use this entry point to make long vector phases
/// visible to operators. Events name search-level phases and counts; runtime,
/// model, and vector-storage internals stay behind their owning crate facades.
pub fn observe_search_with_progress(
    request: SearchObservationRequest<'_>,
    reporter: &mut Reporter,
) -> Result<SearchObservation> {
    observe_search_inner(request, Some(reporter))
}

fn observe_search_inner(
    request: SearchObservationRequest<'_>,
    reporter: Option<&mut Reporter>,
) -> Result<SearchObservation> {
    let output = retrieve_candidates(request.workspace, request.comparison_indexes)?;
    let mut pairs = Vec::new();
    let mut ranked_pair_ids = BTreeSet::new();
    for set in &output.candidate_sets {
        for (index, candidate) in set.candidates.iter().enumerate() {
            let shown = is_shown_queue_candidate(&candidate.explanation);
            let features = pair_features(
                &set.anchor,
                &candidate.declaration,
                &candidate.explanation.contributions,
            );
            let scored = score_observation(&features, request.scoring_variant, true, shown);
            ranked_pair_ids.insert(pair_key(
                &set.anchor.qualified_name,
                &candidate.declaration.qualified_name,
            ));
            pairs.push(SearchObservedPair {
                left: set.anchor.qualified_name.clone(),
                right: candidate.declaration.qualified_name.clone(),
                generated: true,
                symbolic_generated: true,
                vector_generated: false,
                merged_generated: true,
                ranked: scored.ranked,
                generation_policy: generation_policy_for_ranked(&candidate.declaration),
                rank: scored.ranked.then_some(index + 1),
                shown: scored.shown,
                left_content_hash: None,
                right_content_hash: None,
                vector_rank: None,
                origin: candidate.declaration.origin.clone(),
                feature_families: feature_families(&candidate.explanation.contributions),
                survived_shown_filter: scored.survived_shown_filter,
                features,
                scoring: scored.scoring,
            });
        }
    }
    let index_facts = tracked_index_facts(request.comparison_indexes)?;
    pairs.extend(tracked_generated_pairs(&request, &ranked_pair_ids, &index_facts)?);
    let (vector_summary, vector_generated_count) = if let Some(vector_request) = request.vector_candidates {
        let comparison_declarations = all_comparison_declarations(request.comparison_indexes)?;
        let vector_output =
            generate_vector_candidates(vector_request, request.workspace, &comparison_declarations, reporter);
        let count = merge_vector_candidates(
            request.workspace,
            &mut pairs,
            vector_output.candidates,
            request.scoring_variant,
        );
        (vector_output.summary, count)
    } else {
        (SearchVectorCandidateSummary::default(), 0)
    };
    let merged_generated_count = pairs.iter().filter(|pair| pair.merged_generated).count();
    let visible_groups_found = output
        .candidate_sets
        .iter()
        .filter(|set| {
            set.candidates
                .iter()
                .any(|candidate| is_shown_queue_candidate(&candidate.explanation))
        })
        .count();
    let visible_groups_total = output.candidate_sets.len();
    Ok(SearchObservation {
        pairs,
        visible_groups_found,
        visible_groups_total,
        scoring: if matches!(
            request.scoring_variant,
            SearchScoringVariant::AllFeatures | SearchScoringVariant::SymbolicOnly
        ) {
            default_summary()
        } else {
            SearchScoringSummary::new(request.scoring_variant)
        },
        semantic_reranking: semantic_reranking_summary(),
        semantic_obligation_yield: Vec::new(),
        retrieval: retrieval_observation(
            &output.diagnostics,
            vector_summary,
            vector_generated_count,
            merged_generated_count,
        ),
        embedding_documents: embedding_documents(&output.candidate_sets),
    })
}

/// Re-score one search observation with a fixed scorer variant.
///
/// Evaluation uses this to run ablations without re-running retrieval or
/// exposing scorer internals. Candidate generation facts remain unchanged;
/// ranked and visible facts are recalculated from stable pair features.
pub fn rescore_observation(observation: &SearchObservation, variant: SearchScoringVariant) -> SearchObservation {
    if variant == observation.scoring.variant {
        return observation.clone();
    }
    let mut pairs = observation
        .pairs
        .iter()
        .map(|pair| {
            let candidate_rankable = rankable_for_variant(pair, variant);
            let scored = score_observation(&pair.features, variant, candidate_rankable, pair.shown);
            let mut rescored = pair.clone();
            rescored.ranked = scored.ranked;
            rescored.shown = scored.shown;
            rescored.survived_shown_filter = scored.survived_shown_filter;
            rescored.scoring = scored.scoring;
            rescored
        })
        .collect::<Vec<_>>();
    rerank_pairs(&mut pairs);
    let (visible_groups_found, visible_groups_total) = visible_group_counts(&pairs);
    SearchObservation {
        pairs,
        visible_groups_found,
        visible_groups_total,
        scoring: SearchScoringSummary::new(variant),
        semantic_reranking: observation.semantic_reranking.clone(),
        semantic_obligation_yield: observation.semantic_obligation_yield.clone(),
        retrieval: observation.retrieval.clone(),
        embedding_documents: observation.embedding_documents.clone(),
    }
}

fn uses_vector_evidence(variant: SearchScoringVariant) -> bool {
    matches!(
        variant,
        SearchScoringVariant::VectorEvidenceOnly | SearchScoringVariant::SymbolicPlusVector
    )
}

fn rankable_for_variant(pair: &SearchObservedPair, variant: SearchScoringVariant) -> bool {
    match variant {
        SearchScoringVariant::VectorEvidenceOnly => pair.vector_generated && pair.features.vector_evidence.is_some(),
        SearchScoringVariant::SymbolicPlusVector => pair.merged_generated,
        _ => pair.ranked,
    }
}

fn visible_group_counts(pairs: &[SearchObservedPair]) -> (usize, usize) {
    let total = pairs
        .iter()
        .filter(|pair| pair.ranked)
        .map(|pair| pair.left.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let visible = pairs
        .iter()
        .filter(|pair| pair.shown)
        .map(|pair| pair.left.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    (visible, total)
}

fn rerank_pairs(pairs: &mut [SearchObservedPair]) {
    pairs.sort_by(|left, right| {
        right
            .scoring
            .total_score
            .partial_cmp(&left.scoring.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.rank.unwrap_or(usize::MAX).cmp(&right.rank.unwrap_or(usize::MAX)))
            .then_with(|| left.left.cmp(&right.left))
            .then_with(|| left.right.cmp(&right.right))
    });
    let mut next_rank = 1;
    for pair in pairs {
        if pair.ranked {
            pair.rank = Some(next_rank);
            next_rank += 1;
        } else {
            pair.rank = None;
        }
    }
}

const SEARCH_EMBEDDING_DOCUMENT_POLICY_VERSION: &str = "lean-dup.embedding-document.v1";

fn embedding_documents(candidate_sets: &[crate::retrieval::CandidateSet]) -> SearchEmbeddingDocuments {
    let policy = SearchEmbeddingDocumentPolicy::default();
    let mut by_name = BTreeMap::<String, SearchEmbeddingDocument>::new();
    for set in candidate_sets {
        by_name
            .entry(set.anchor.qualified_name.clone())
            .or_insert_with(|| embedding_document_for(&set.anchor, policy));
        for candidate in &set.candidates {
            by_name
                .entry(candidate.declaration.qualified_name.clone())
                .or_insert_with(|| embedding_document_for(&candidate.declaration, policy));
        }
    }
    SearchEmbeddingDocuments {
        policy_id: policy.id().to_owned(),
        policy_version: SEARCH_EMBEDDING_DOCUMENT_POLICY_VERSION.to_owned(),
        content_availability: content_availability(by_name.values()),
        documents: by_name.into_values().collect(),
    }
}

pub(crate) fn embedding_documents_for_declarations_with_policy(
    declarations: &[HydratedDeclaration],
    policy: SearchEmbeddingDocumentPolicy,
) -> SearchEmbeddingDocuments {
    let documents = declarations
        .iter()
        .map(|declaration| embedding_document_for(declaration, policy))
        .collect::<Vec<_>>();
    SearchEmbeddingDocuments {
        policy_id: policy.id().to_owned(),
        policy_version: SEARCH_EMBEDDING_DOCUMENT_POLICY_VERSION.to_owned(),
        content_availability: content_availability(documents.iter()),
        documents,
    }
}

fn embedding_document_for(
    declaration: &HydratedDeclaration,
    policy: SearchEmbeddingDocumentPolicy,
) -> SearchEmbeddingDocument {
    let normalized_statement = normalize_statement(&declaration.statement_text);
    let mut document = SearchEmbeddingDocument {
        declaration_name: declaration.qualified_name.clone(),
        module_name: declaration.module.clone(),
        declaration_kind: declaration.kind.clone(),
        normalized_statement,
        docstring_text: declaration.docstring_text.as_deref().map(normalize_statement),
        definition_body_summary: declaration.definition_body_summary.as_deref().map(normalize_statement),
        content_hash: String::new(),
    };
    document.content_hash = content_hash_for(&document, policy);
    document
}

fn content_availability<'a>(
    documents: impl IntoIterator<Item = &'a SearchEmbeddingDocument>,
) -> SearchEmbeddingContentAvailability {
    let mut availability = SearchEmbeddingContentAvailability::default();
    for document in documents {
        availability.total += 1;
        if document
            .docstring_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            availability.with_docstring += 1;
        }
        if document
            .definition_body_summary
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            availability.with_definition_body_summary += 1;
        }
    }
    availability
}

fn normalize_statement(statement: &str) -> String {
    statement.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn document_text(document: &SearchEmbeddingDocument, policy: SearchEmbeddingDocumentPolicy) -> String {
    match policy {
        SearchEmbeddingDocumentPolicy::Statement => document.normalized_statement.clone(),
        SearchEmbeddingDocumentPolicy::NameAndStatement => {
            format!("{}\n{}", document.declaration_name, document.normalized_statement)
        }
        SearchEmbeddingDocumentPolicy::DefinitionAware => {
            let mut parts = vec![
                document.declaration_name.as_str(),
                document.normalized_statement.as_str(),
            ];
            if let Some(body) = document
                .definition_body_summary
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                parts.push(body);
            }
            parts.join("\n")
        }
        SearchEmbeddingDocumentPolicy::DocstringAugmented => {
            let mut parts = Vec::new();
            if let Some(docstring) = document
                .docstring_text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                parts.push(docstring);
            }
            parts.push(document.declaration_name.as_str());
            parts.push(document.normalized_statement.as_str());
            if let Some(body) = document
                .definition_body_summary
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                parts.push(body);
            }
            parts.join("\n")
        }
    }
}

fn content_hash_for(document: &SearchEmbeddingDocument, policy: SearchEmbeddingDocumentPolicy) -> String {
    let mut hasher = Sha256::new();
    hasher.update(policy.id().as_bytes());
    hasher.update([0]);
    hasher.update(SEARCH_EMBEDDING_DOCUMENT_POLICY_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(document_text(document, policy).as_bytes());
    hex_bytes(&hasher.finalize())
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

impl SearchEmbeddingDocumentPolicy {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "statement" => Some(Self::Statement),
            "name-and-statement" => Some(Self::NameAndStatement),
            "definition-aware" => Some(Self::DefinitionAware),
            "docstring-augmented" => Some(Self::DocstringAugmented),
            _ => None,
        }
    }
}

fn retrieval_observation(
    diagnostics: &RetrievalDiagnostics,
    vector_candidates: SearchVectorCandidateSummary,
    vector_generated_count: usize,
    merged_generated_count: usize,
) -> SearchRetrievalObservation {
    SearchRetrievalObservation {
        candidate_count: diagnostics.candidate_count,
        generated_candidate_count: diagnostics.generated_candidate_count,
        ranked_candidate_count: diagnostics.ranked_candidate_count,
        symbolic_generated_candidate_count: diagnostics.generated_candidate_count,
        vector_generated_candidate_count: vector_generated_count,
        merged_generated_candidate_count: merged_generated_count,
        hydrated_external_count: diagnostics.hydrated_external_count,
        pruned_feature_fanout_count: diagnostics.pruned_postings.len(),
        heap_truncations: diagnostics.heap_truncations.len(),
        candidate_count_by_generation_policy: diagnostics.candidate_count_by_generation_policy.clone(),
        pruned_feature_fanouts: diagnostics
            .pruned_feature_fanouts
            .iter()
            .map(|item| SearchPrunedFeatureFanout {
                policy: item.policy.clone(),
                source: item.source.clone(),
                reason: item.reason.clone(),
                feature_family: item.feature_family.clone(),
                count: item.count,
            })
            .collect(),
        vector_candidates,
    }
}

fn is_shown_queue_candidate(explanation: &CandidateExplanation) -> bool {
    explanation.contributions.iter().any(|contribution| {
        matches!(
            contribution.kind.as_str(),
            "statement-fingerprint" | "safe-permutation-fingerprint" | "connective-fingerprint"
        )
    })
}

fn merge_vector_candidates(
    workspace: &[HydratedDeclaration],
    pairs: &mut Vec<SearchObservedPair>,
    vector_candidates: Vec<VectorCandidate>,
    variant: SearchScoringVariant,
) -> usize {
    let workspace_by_name = workspace
        .iter()
        .map(|declaration| (declaration.qualified_name.clone(), declaration))
        .collect::<BTreeMap<_, _>>();
    let mut pair_index_by_key = pairs
        .iter()
        .enumerate()
        .map(|(index, pair)| (pair_key(&pair.left, &pair.right), index))
        .collect::<BTreeMap<_, _>>();
    let vector_count = vector_candidates.len();
    for vector in vector_candidates {
        let key = pair_key(&vector.anchor_name, &vector.declaration.qualified_name);
        if let Some(index) = pair_index_by_key.get(&key).copied() {
            if let Some(pair) = pairs.get_mut(index) {
                pair.vector_generated = true;
                pair.merged_generated = pair.symbolic_generated || pair.vector_generated;
                pair.vector_rank = Some(vector.rank);
                pair.features.vector_evidence = Some(vector_evidence(f64::from(vector.score), vector.rank));
                let scored =
                    score_observation(&pair.features, variant, rankable_for_variant(pair, variant), pair.shown);
                pair.ranked = scored.ranked;
                pair.shown = scored.shown;
                pair.survived_shown_filter = scored.survived_shown_filter;
                pair.scoring = scored.scoring;
                if !pair.feature_families.iter().any(|family| family == "vector_similarity") {
                    pair.feature_families.push("vector_similarity".to_owned());
                    pair.feature_families.sort();
                    pair.feature_families.dedup();
                }
                pair.left_content_hash = Some(vector.anchor_content_hash.clone());
                pair.right_content_hash = Some(vector.declaration_content_hash.clone());
            }
            continue;
        }
        let Some(anchor) = workspace_by_name.get(&vector.anchor_name) else {
            continue;
        };
        let feature_families = vec!["vector_similarity".to_owned()];
        let mut features = pair_features(anchor, &vector.declaration, &[]);
        features.retrieval_feature_families = feature_families.clone();
        features.vector_evidence = Some(vector_evidence(f64::from(vector.score), vector.rank));
        let vector_rankable = !matches!(
            variant,
            SearchScoringVariant::SymbolicOnly | SearchScoringVariant::AllFeatures
        );
        let scored = score_observation(&features, variant, vector_rankable, false);
        let observed = SearchObservedPair {
            left: anchor.qualified_name.clone(),
            right: vector.declaration.qualified_name.clone(),
            generated: true,
            symbolic_generated: false,
            vector_generated: true,
            merged_generated: true,
            ranked: scored.ranked,
            generation_policy: generation_policy_for_vector(&vector.declaration),
            rank: scored.ranked.then_some(vector.rank),
            shown: scored.shown,
            left_content_hash: Some(vector.anchor_content_hash),
            right_content_hash: Some(vector.declaration_content_hash),
            vector_rank: Some(vector.rank),
            origin: vector.declaration.origin.clone(),
            feature_families,
            survived_shown_filter: scored.survived_shown_filter,
            features,
            scoring: scored.scoring,
        };
        pair_index_by_key.insert(key, pairs.len());
        pairs.push(observed);
    }
    if uses_vector_evidence(variant) {
        rerank_pairs(pairs);
        return vector_count;
    }
    pairs.sort_by(|left, right| {
        left.left
            .cmp(&right.left)
            .then_with(|| left.rank.unwrap_or(usize::MAX).cmp(&right.rank.unwrap_or(usize::MAX)))
            .then_with(|| left.right.cmp(&right.right))
    });
    vector_count
}

fn all_comparison_declarations(indexes: &[OpenedIndex]) -> Result<Vec<HydratedDeclaration>> {
    let mut declarations = Vec::new();
    for index in indexes {
        let handles = index.all_handles()?;
        declarations.extend(index.hydrate(&handles)?);
    }
    Ok(declarations)
}

fn tracked_generated_pairs(
    request: &SearchObservationRequest<'_>,
    ranked_pair_ids: &BTreeSet<(String, String)>,
    index_facts: &[lean_dup_index::OpenedIndexFacts],
) -> Result<Vec<SearchObservedPair>> {
    if request.tracked_pairs.is_empty() {
        return Ok(Vec::new());
    }
    let declarations = tracked_declarations(request)?;
    let mut observed = Vec::new();
    let mut seen = BTreeSet::new();
    for tracked in request.tracked_pairs {
        let key = pair_key(&tracked.left, &tracked.right);
        if ranked_pair_ids.contains(&key) || !seen.insert(key) {
            continue;
        }
        let Some(left) = declarations.get(&tracked.left) else {
            continue;
        };
        let Some(right) = declarations.get(&tracked.right) else {
            continue;
        };
        let Some(oriented) = orient_pair(left, right, request.comparison_indexes, index_facts) else {
            continue;
        };
        let Some(evidence) = generated_pair_evidence(
            request.workspace,
            oriented.anchor,
            oriented.candidate,
            oriented.external,
        )?
        else {
            continue;
        };
        observed.push(generated_observed_pair(
            oriented.anchor,
            oriented.candidate,
            evidence,
            request.scoring_variant,
        ));
    }
    observed.sort_by(|left, right| left.left.cmp(&right.left).then_with(|| left.right.cmp(&right.right)));
    Ok(observed)
}

#[derive(Clone)]
struct LocatedDeclaration {
    declaration: HydratedDeclaration,
    comparison_index: Option<usize>,
}

struct OrientedTrackedPair<'a> {
    anchor: &'a HydratedDeclaration,
    candidate: &'a HydratedDeclaration,
    external: Option<(&'a OpenedIndex, &'a lean_dup_index::OpenedIndexFacts)>,
}

fn tracked_declarations(request: &SearchObservationRequest<'_>) -> Result<BTreeMap<String, LocatedDeclaration>> {
    let requested_names = request
        .tracked_pairs
        .iter()
        .flat_map(|pair| [pair.left.clone(), pair.right.clone()])
        .collect::<BTreeSet<_>>();
    let mut declarations = BTreeMap::new();
    for declaration in request.workspace {
        if requested_names.contains(&declaration.qualified_name) {
            declarations.insert(
                declaration.qualified_name.clone(),
                LocatedDeclaration {
                    declaration: declaration.clone(),
                    comparison_index: None,
                },
            );
        }
    }
    let missing = requested_names
        .into_iter()
        .filter(|name| !declarations.contains_key(name))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(declarations);
    }
    for (index, opened) in request.comparison_indexes.iter().enumerate() {
        let still_missing = missing
            .iter()
            .filter(|name| !declarations.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        if still_missing.is_empty() {
            break;
        }
        for declaration in opened.declarations_named(&still_missing)? {
            let name = declaration.qualified_name.clone();
            declarations.entry(name).or_insert_with(|| LocatedDeclaration {
                declaration,
                comparison_index: Some(index),
            });
        }
    }
    Ok(declarations)
}

fn orient_pair<'a>(
    left: &'a LocatedDeclaration,
    right: &'a LocatedDeclaration,
    indexes: &'a [OpenedIndex],
    index_facts: &'a [lean_dup_index::OpenedIndexFacts],
) -> Option<OrientedTrackedPair<'a>> {
    match (left.comparison_index, right.comparison_index) {
        (None, None) => Some(OrientedTrackedPair {
            anchor: &left.declaration,
            candidate: &right.declaration,
            external: None,
        }),
        (None, Some(index)) => index_facts.get(index).and_then(|facts| {
            indexes.get(index).map(|opened| OrientedTrackedPair {
                anchor: &left.declaration,
                candidate: &right.declaration,
                external: Some((opened, facts)),
            })
        }),
        (Some(index), None) => index_facts.get(index).and_then(|facts| {
            indexes.get(index).map(|opened| OrientedTrackedPair {
                anchor: &right.declaration,
                candidate: &left.declaration,
                external: Some((opened, facts)),
            })
        }),
        (Some(_), Some(_)) => None,
    }
}

fn generated_observed_pair(
    anchor: &HydratedDeclaration,
    candidate: &HydratedDeclaration,
    evidence: GeneratedPairEvidence,
    variant: SearchScoringVariant,
) -> SearchObservedPair {
    let feature_families = feature_families(&evidence.contributions);
    let features = pair_features(anchor, candidate, &evidence.contributions);
    let scored = score_observation(&features, variant, false, false);
    SearchObservedPair {
        left: anchor.qualified_name.clone(),
        right: candidate.qualified_name.clone(),
        generated: true,
        symbolic_generated: true,
        vector_generated: false,
        merged_generated: true,
        ranked: scored.ranked,
        generation_policy: evidence.policy,
        rank: None,
        shown: scored.shown,
        left_content_hash: None,
        right_content_hash: None,
        vector_rank: None,
        origin: candidate.origin.clone(),
        feature_families,
        survived_shown_filter: scored.survived_shown_filter,
        features,
        scoring: scored.scoring,
    }
}

fn tracked_index_facts(indexes: &[OpenedIndex]) -> Result<Vec<lean_dup_index::OpenedIndexFacts>> {
    let mut facts = Vec::with_capacity(indexes.len());
    for index in indexes {
        facts.push(index.facts()?);
    }
    Ok(facts)
}

fn pair_key(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

fn generation_policy_for_ranked(candidate: &HydratedDeclaration) -> String {
    if candidate.origin == "workspace" {
        "local_duplicate_audit".to_owned()
    } else if candidate.origin == "mathlib" {
        "mathlib_comparison".to_owned()
    } else if candidate.source_span.is_some() {
        "source_backed_external_comparison".to_owned()
    } else {
        "static_external_comparison".to_owned()
    }
}

fn generation_policy_for_vector(candidate: &HydratedDeclaration) -> String {
    if candidate.origin == "workspace" {
        "vector_local_duplicate_audit".to_owned()
    } else if candidate.origin == "mathlib" {
        "vector_mathlib_comparison".to_owned()
    } else if candidate.source_span.is_some() {
        "vector_source_backed_external_comparison".to_owned()
    } else {
        "vector_static_external_comparison".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, RoleFeature};

    use super::{
        SearchEmbeddingDocumentPolicy, SearchEmbeddingDocuments, SearchObservation, SearchObservationRequest,
        SearchRetrievalObservation, SearchScoringSummary, SearchScoringVariant, SearchTrackedPair, content_hash_for,
        merge_vector_candidates, observe_search, rescore_observation,
    };
    use crate::vector_candidates::{SearchVectorCandidateStatus, VectorCandidate};

    #[test]
    fn tracked_pairs_record_generated_before_ranked_selection() {
        let rows = generated_rows(100);
        let tracked = vec![SearchTrackedPair {
            left: "Synthetic.generated_0".to_owned(),
            right: "Synthetic.generated_1".to_owned(),
        }];

        let observation = observe_search(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &tracked,
            scoring_variant: SearchScoringVariant::SymbolicOnly,
            vector_candidates: None,
        })
        .unwrap();

        let pair = observation
            .pairs
            .iter()
            .find(|pair| {
                (pair.left == "Synthetic.generated_0" && pair.right == "Synthetic.generated_1")
                    || (pair.left == "Synthetic.generated_1" && pair.right == "Synthetic.generated_0")
            })
            .expect("tracked generated pair");
        assert!(pair.generated);
        assert!(!pair.ranked);
        assert_eq!(pair.rank, None);
        assert_eq!(pair.generation_policy, "local_duplicate_audit");
        assert!(pair.feature_families.contains(&"statement_fingerprint".to_owned()));
        assert!(observation.retrieval.generated_candidate_count > observation.retrieval.ranked_candidate_count);
        assert_eq!(
            observation.retrieval.vector_candidates.status,
            SearchVectorCandidateStatus::Disabled
        );
    }

    #[test]
    fn vector_only_candidates_are_generated_but_not_ranked_by_symbolic_baseline() {
        let rows = generated_rows(2);
        let mut pairs = Vec::new();

        let count = merge_vector_candidates(
            &rows,
            &mut pairs,
            vec![VectorCandidate {
                anchor_name: "Synthetic.generated_0".to_owned(),
                anchor_content_hash: "hash-left".to_owned(),
                declaration: rows[1].clone(),
                declaration_content_hash: "hash-right".to_owned(),
                score: 0.95,
                rank: 1,
            }],
            SearchScoringVariant::SymbolicOnly,
        );

        assert_eq!(count, 1);
        assert_eq!(pairs.len(), 1);
        let pair = &pairs[0];
        assert!(pair.generated);
        assert!(!pair.symbolic_generated);
        assert!(pair.vector_generated);
        assert!(pair.merged_generated);
        assert!(!pair.ranked);
        assert!(!pair.shown);
        assert_eq!(pair.vector_rank, Some(1));
        assert_eq!(pair.left_content_hash.as_deref(), Some("hash-left"));
        assert_eq!(pair.right_content_hash.as_deref(), Some("hash-right"));
        assert_eq!(pair.generation_policy, "vector_local_duplicate_audit");
    }

    #[test]
    fn vector_evidence_variant_ranks_vector_only_candidates_from_stable_facts() {
        let rows = generated_rows(2);
        let mut pairs = Vec::new();

        merge_vector_candidates(
            &rows,
            &mut pairs,
            vec![VectorCandidate {
                anchor_name: "Synthetic.generated_0".to_owned(),
                anchor_content_hash: "hash-left".to_owned(),
                declaration: rows[1].clone(),
                declaration_content_hash: "hash-right".to_owned(),
                score: 0.95,
                rank: 1,
            }],
            SearchScoringVariant::VectorEvidenceOnly,
        );

        let pair = &pairs[0];
        assert!(pair.ranked);
        assert!(pair.shown);
        let evidence = pair.features.vector_evidence.as_ref().expect("vector evidence");
        assert_eq!(evidence.score_bucket, "very-high");
        assert_eq!(evidence.rank_bucket, "rank-1");
        assert!(evidence.top_k_member);
        assert!(pair.scoring.component_scores.contains_key("vector_rank"));
        assert!(!pair.scoring.component_scores.contains_key("statement_fingerprint"));
    }

    #[test]
    fn rescoring_counts_visible_groups_by_anchor_not_visible_pair_rows() {
        let rows = generated_rows(3);
        let mut pairs = Vec::new();
        merge_vector_candidates(
            &rows,
            &mut pairs,
            vec![
                VectorCandidate {
                    anchor_name: "Synthetic.generated_0".to_owned(),
                    anchor_content_hash: "hash-left".to_owned(),
                    declaration: rows[1].clone(),
                    declaration_content_hash: "hash-right-1".to_owned(),
                    score: 0.95,
                    rank: 1,
                },
                VectorCandidate {
                    anchor_name: "Synthetic.generated_0".to_owned(),
                    anchor_content_hash: "hash-left".to_owned(),
                    declaration: rows[2].clone(),
                    declaration_content_hash: "hash-right-2".to_owned(),
                    score: 0.94,
                    rank: 2,
                },
            ],
            SearchScoringVariant::SymbolicOnly,
        );
        let observation = SearchObservation {
            pairs,
            visible_groups_found: 0,
            visible_groups_total: 1,
            scoring: SearchScoringSummary::new(SearchScoringVariant::SymbolicOnly),
            semantic_reranking: crate::SearchSemanticRerankingSummary::default(),
            semantic_obligation_yield: Vec::new(),
            retrieval: SearchRetrievalObservation::default(),
            embedding_documents: SearchEmbeddingDocuments::default(),
        };

        let rescored = rescore_observation(&observation, SearchScoringVariant::VectorEvidenceOnly);

        assert_eq!(rescored.pairs.iter().filter(|pair| pair.shown).count(), 2);
        assert_eq!(rescored.visible_groups_found, 1);
        assert_eq!(rescored.visible_groups_total, 1);
        assert!(rescored.visible_groups_found <= rescored.visible_groups_total);
    }

    #[test]
    fn rescoring_does_not_rerun_generation_or_expose_private_keys() {
        let rows = generated_rows(3);
        let observation = observe_search(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::SymbolicOnly,
            vector_candidates: None,
        })
        .unwrap();

        let semantic_only = rescore_observation(&observation, SearchScoringVariant::SemanticEvidenceOnlyRerank);

        assert_eq!(
            semantic_only.retrieval.generated_candidate_count,
            observation.retrieval.generated_candidate_count
        );
        assert_eq!(
            semantic_only.scoring.variant,
            SearchScoringVariant::SemanticEvidenceOnlyRerank
        );
        assert!(semantic_only.pairs.iter().all(|pair| !pair.shown));
    }

    #[test]
    fn embedding_documents_are_deterministic_and_not_serialized() {
        let rows = generated_rows(3);
        let observation = observe_search(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::SymbolicOnly,
            vector_candidates: None,
        })
        .unwrap();

        assert_eq!(observation.embedding_documents.policy_id, "name-and-statement");
        assert_eq!(
            observation.embedding_documents.policy_version,
            "lean-dup.embedding-document.v1"
        );
        assert!(
            observation
                .embedding_documents
                .documents
                .windows(2)
                .all(|window| window[0].declaration_name <= window[1].declaration_name)
        );
        let text_inputs = observation.embedding_documents.text_inputs();
        assert!(
            text_inputs
                .iter()
                .any(|input| input.text == "Synthetic.generated_0\nraw statement text must not serialize")
        );
        assert!(text_inputs.iter().all(|input| !input.text.contains("features:")));
        assert!(text_inputs.iter().all(|input| !input.text.contains("same-role")));
        let first_hash = &observation.embedding_documents.documents[0].content_hash;
        assert_eq!(first_hash.len(), 64);
        assert_ne!(
            first_hash,
            &content_hash_for(
                &observation.embedding_documents.documents[0],
                SearchEmbeddingDocumentPolicy::Statement,
            )
        );
        let mut changed = observation.embedding_documents.documents[0].clone();
        changed.normalized_statement.push_str(" changed");
        assert_ne!(
            first_hash,
            &content_hash_for(&changed, SearchEmbeddingDocumentPolicy::NameAndStatement)
        );

        let json = serde_json::to_string(&observation).unwrap();
        assert!(!json.contains("embedding_documents"));
        assert!(!json.contains("raw statement text must not serialize"));
        assert!(!json.contains(first_hash));
        assert!(!json.contains("same-role"));
    }

    fn generated_rows(count: usize) -> Vec<HydratedDeclaration> {
        (0..count)
            .map(|index| HydratedDeclaration {
                handle: DeclarationHandle::for_test(format!("synthetic-{index}")),
                declaration_id: format!("synthetic:generated:{index}"),
                origin: "workspace".to_owned(),
                module: "Synthetic".to_owned(),
                qualified_name: format!("Synthetic.generated_{index}"),
                display_name: format!("generated_{index}"),
                kind: "theorem".to_owned(),
                visibility: "public".to_owned(),
                modifiers: Vec::new(),
                source_span: None,
                statement_text: "raw statement text must not serialize".to_owned(),
                docstring_text: None,
                definition_body_summary: None,
                status_flags: Vec::new(),
                feature_version: "test".to_owned(),
                fingerprints: Fingerprints {
                    statement: "same-statement".to_owned(),
                    safe_binder_permutation: String::new(),
                    connective_shape: String::new(),
                    conclusion_shape: String::new(),
                },
                role_features: vec![RoleFeature {
                    role: "conclusion_const".to_owned(),
                    key: "same-role".to_owned(),
                    display: Some("Same".to_owned()),
                }],
                binder_count: 0,
                low_signal_markers: Vec::new(),
            })
            .collect()
    }
}
