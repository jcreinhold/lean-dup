use serde::Serialize;

use lean_dup_index::{HydratedDeclaration, OpenedIndex};

use crate::Result;
use crate::pair_features::{SearchPairFeatures, feature_families, pair_features};
use crate::retrieval::{CandidateExplanation, RetrievalDiagnostics, retrieve_candidates};

/// Request for search-stage observations used by offline evaluation.
///
/// The search crate owns retrieval keys and contribution mapping. Evaluation
/// receives stable pair, origin, queue, and feature-family facts without
/// depending on retrieval internals.
pub struct SearchObservationRequest<'a> {
    pub workspace: &'a [HydratedDeclaration],
    pub comparison_indexes: &'a [OpenedIndex],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchObservation {
    pub pairs: Vec<SearchObservedPair>,
    pub visible_groups_found: usize,
    pub visible_groups_total: usize,
    pub retrieval: SearchRetrievalObservation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchRetrievalObservation {
    pub candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_feature_fanout_count: usize,
    pub heap_truncations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchObservedPair {
    pub left: String,
    pub right: String,
    pub rank: usize,
    pub shown: bool,
    pub origin: String,
    pub feature_families: Vec<String>,
    pub survived_shown_filter: bool,
    pub features: SearchPairFeatures,
}

pub fn observe_search(request: SearchObservationRequest<'_>) -> Result<SearchObservation> {
    let output = retrieve_candidates(request.workspace, request.comparison_indexes)?;
    let mut pairs = Vec::new();
    for set in &output.candidate_sets {
        for (index, candidate) in set.candidates.iter().enumerate() {
            let shown = is_shown_queue_candidate(&candidate.explanation);
            pairs.push(SearchObservedPair {
                left: set.anchor.qualified_name.clone(),
                right: candidate.declaration.qualified_name.clone(),
                rank: index + 1,
                shown,
                origin: candidate.declaration.origin.clone(),
                feature_families: feature_families(&candidate.explanation.contributions),
                survived_shown_filter: shown,
                features: pair_features(
                    &set.anchor,
                    &candidate.declaration,
                    &candidate.explanation.contributions,
                ),
            });
        }
    }
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
        retrieval: retrieval_observation(&output.diagnostics),
    })
}

fn retrieval_observation(diagnostics: &RetrievalDiagnostics) -> SearchRetrievalObservation {
    SearchRetrievalObservation {
        candidate_count: diagnostics.candidate_count,
        hydrated_external_count: diagnostics.hydrated_external_count,
        pruned_feature_fanout_count: diagnostics.pruned_postings.len(),
        heap_truncations: diagnostics.heap_truncations.len(),
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
