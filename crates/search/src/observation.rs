use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use lean_dup_index::{HydratedDeclaration, OpenedIndex};

use crate::pair_features::{SearchPairFeatures, feature_families, pair_features};
use crate::retrieval::{GeneratedPairEvidence, RetrievalDiagnostics, generated_pair_evidence, retrieve_candidates};
use crate::review_policy;
use crate::scorer::{
    SearchPairScoring, SearchScoringSummary, SearchScoringVariant, default_summary, score_observation,
};
use crate::semantic_reranking::{
    SearchSemanticObligationYield, SearchSemanticRerankingSummary, summary as semantic_reranking_summary,
};
use crate::{Error, Result};

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
    pub review_policy: crate::review_policy::SearchReviewPolicySummary,
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub semantic_obligation_yield: Vec<SearchSemanticObligationYield>,
    pub retrieval: SearchRetrievalObservation,
}

/// Stable compact search-stage facts for ordinary eval metrics.
///
/// Eval uses this surface when it only needs pair identity, stage survival,
/// feature-family labels, and retrieval counters. Search keeps detailed
/// feature vectors, scorer component maps, retrieval keys, and ranking
/// constants private unless callers explicitly request detailed observations.
/// Non-symbolic scorer ablations require the detailed observation surface.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchStageObservation {
    pub pairs: Vec<SearchStageObservedPair>,
    pub visible_groups_found: usize,
    pub visible_groups_total: usize,
    pub scoring: SearchScoringSummary,
    pub review_policy: crate::review_policy::SearchReviewPolicySummary,
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub semantic_obligation_yield: Vec<SearchSemanticObligationYield>,
    pub retrieval: SearchRetrievalObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchStageObservedPair {
    pub left: String,
    pub right: String,
    pub left_declaration_id: String,
    pub right_declaration_id: String,
    pub generated: bool,
    pub symbolic_generated: bool,
    pub merged_generated: bool,
    pub ranked: bool,
    pub generation_policy: String,
    pub rank: Option<usize>,
    pub shown: bool,
    pub origin: String,
    pub feature_families: Vec<String>,
    pub candidate_sources: Vec<SearchCandidateSourceFact>,
    pub survived_shown_filter: bool,
}

/// Stable candidate-source facts attached to an observed declaration pair.
///
/// Search owns source-specific generation, fanout, and merge policy. Eval may
/// count these facts by source id or source family, but callers must not infer
/// retrieval keys, posting layout, scorer internals, or backend details from
/// this surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchCandidateSourceFact {
    pub source_id: String,
    pub source_family: SearchCandidateSourceFamily,
    pub pair_id: String,
    pub left_declaration_id: String,
    pub right_declaration_id: String,
    pub origin: String,
    pub generation_rank: Option<usize>,
    pub top_k_status: SearchCandidateTopKStatus,
    pub top_k_saturated: bool,
    pub feature_families: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchCandidateSourceFamily {
    Symbolic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchCandidateTopKStatus {
    Selected,
    GeneratedNotSelected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchRetrievalObservation {
    pub candidate_count: usize,
    pub generated_candidate_count: usize,
    pub ranked_candidate_count: usize,
    pub symbolic_generated_candidate_count: usize,
    pub merged_generated_candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_feature_fanout_count: usize,
    pub heap_truncations: usize,
    pub candidate_count_by_generation_policy: BTreeMap<String, usize>,
    pub pruned_feature_fanouts: Vec<SearchPrunedFeatureFanout>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchObservedPair {
    pub left: String,
    pub right: String,
    pub left_declaration_id: String,
    pub right_declaration_id: String,
    pub generated: bool,
    pub symbolic_generated: bool,
    pub merged_generated: bool,
    pub ranked: bool,
    pub generation_policy: String,
    pub rank: Option<usize>,
    pub shown: bool,
    pub origin: String,
    pub feature_families: Vec<String>,
    pub candidate_sources: Vec<SearchCandidateSourceFact>,
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
    let output = retrieve_candidates(request.workspace, request.comparison_indexes)?;
    let mut pairs = Vec::new();
    let mut ranked_pair_ids = BTreeSet::new();
    let top_k_saturated = top_k_saturation_by_anchor(&output.diagnostics);
    for set in &output.candidate_sets {
        for (index, candidate) in set.candidates.iter().enumerate() {
            let shown = review_policy::symbolic_observation_visible(
                &set.anchor,
                &candidate.declaration,
                &candidate.explanation.contributions,
            );
            let features = pair_features(
                &set.anchor,
                &candidate.declaration,
                &candidate.explanation.contributions,
            );
            let scored = score_observation(&features, request.scoring_variant, true, shown);
            let rank = index + 1;
            let feature_families = feature_families(&candidate.explanation.contributions);
            ranked_pair_ids.insert(pair_key(
                &set.anchor.qualified_name,
                &candidate.declaration.qualified_name,
            ));
            pairs.push(SearchObservedPair {
                left: set.anchor.qualified_name.clone(),
                right: candidate.declaration.qualified_name.clone(),
                left_declaration_id: set.anchor.declaration_id.clone(),
                right_declaration_id: candidate.declaration.declaration_id.clone(),
                generated: true,
                symbolic_generated: true,
                merged_generated: true,
                ranked: scored.ranked,
                generation_policy: generation_policy_for_ranked(&candidate.declaration),
                rank: scored.ranked.then_some(rank),
                shown: scored.shown,
                origin: candidate.declaration.origin.clone(),
                feature_families: feature_families.clone(),
                candidate_sources: vec![symbolic_source_fact(
                    &set.anchor,
                    &candidate.declaration,
                    &candidate.declaration.origin,
                    Some(rank),
                    SearchCandidateTopKStatus::Selected,
                    top_k_saturated.contains(&set.anchor.declaration_id),
                    feature_families,
                )],
                survived_shown_filter: scored.survived_shown_filter,
                features,
                scoring: scored.scoring,
            });
        }
    }
    let index_facts = tracked_index_facts(request.comparison_indexes)?;
    pairs.extend(tracked_generated_pairs(&request, &ranked_pair_ids, &index_facts)?);
    let merged_generated_count = pairs.iter().filter(|pair| pair.merged_generated).count();
    let visible_groups_found = output
        .candidate_sets
        .iter()
        .filter(|set| {
            set.candidates.iter().any(|candidate| {
                review_policy::symbolic_observation_visible(
                    &set.anchor,
                    &candidate.declaration,
                    &candidate.explanation.contributions,
                )
            })
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
        review_policy: review_policy::summary(),
        semantic_reranking: semantic_reranking_summary(),
        semantic_obligation_yield: Vec::new(),
        retrieval: retrieval_observation(&output.diagnostics, merged_generated_count),
    })
}

pub fn observe_search_stages(request: SearchObservationRequest<'_>) -> Result<SearchStageObservation> {
    if !matches!(
        request.scoring_variant,
        SearchScoringVariant::AllFeatures | SearchScoringVariant::SymbolicOnly
    ) {
        return Err(Error::Search {
            message: format!(
                "compact search-stage observation only supports symbolic scorer variants; use detailed observations for {}",
                request.scoring_variant.label()
            ),
        });
    }
    let output = retrieve_candidates(request.workspace, request.comparison_indexes)?;
    let mut pairs = Vec::new();
    let mut ranked_pair_ids = BTreeSet::new();
    let top_k_saturated = top_k_saturation_by_anchor(&output.diagnostics);
    for set in &output.candidate_sets {
        for (index, candidate) in set.candidates.iter().enumerate() {
            let shown = review_policy::symbolic_observation_visible(
                &set.anchor,
                &candidate.declaration,
                &candidate.explanation.contributions,
            );
            let rank = index + 1;
            let feature_families = feature_families(&candidate.explanation.contributions);
            ranked_pair_ids.insert(pair_key(
                &set.anchor.qualified_name,
                &candidate.declaration.qualified_name,
            ));
            pairs.push(SearchStageObservedPair {
                left: set.anchor.qualified_name.clone(),
                right: candidate.declaration.qualified_name.clone(),
                left_declaration_id: set.anchor.declaration_id.clone(),
                right_declaration_id: candidate.declaration.declaration_id.clone(),
                generated: true,
                symbolic_generated: true,
                merged_generated: true,
                ranked: true,
                generation_policy: generation_policy_for_ranked(&candidate.declaration),
                rank: Some(rank),
                shown,
                origin: candidate.declaration.origin.clone(),
                feature_families: feature_families.clone(),
                candidate_sources: vec![symbolic_source_fact(
                    &set.anchor,
                    &candidate.declaration,
                    &candidate.declaration.origin,
                    Some(rank),
                    SearchCandidateTopKStatus::Selected,
                    top_k_saturated.contains(&set.anchor.declaration_id),
                    feature_families,
                )],
                survived_shown_filter: shown,
            });
        }
    }
    let index_facts = tracked_index_facts(request.comparison_indexes)?;
    pairs.extend(tracked_generated_stage_pairs(&request, &ranked_pair_ids, &index_facts)?);
    let merged_generated_count = pairs.iter().filter(|pair| pair.merged_generated).count();
    let visible_groups_found = output
        .candidate_sets
        .iter()
        .filter(|set| {
            set.candidates.iter().any(|candidate| {
                review_policy::symbolic_observation_visible(
                    &set.anchor,
                    &candidate.declaration,
                    &candidate.explanation.contributions,
                )
            })
        })
        .count();
    let visible_groups_total = output.candidate_sets.len();
    Ok(SearchStageObservation {
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
        review_policy: review_policy::summary(),
        semantic_reranking: semantic_reranking_summary(),
        semantic_obligation_yield: Vec::new(),
        retrieval: retrieval_observation(&output.diagnostics, merged_generated_count),
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
        review_policy: observation.review_policy,
        semantic_reranking: observation.semantic_reranking.clone(),
        semantic_obligation_yield: observation.semantic_obligation_yield.clone(),
        retrieval: observation.retrieval.clone(),
    }
}

fn rankable_for_variant(pair: &SearchObservedPair, _variant: SearchScoringVariant) -> bool {
    pair.ranked
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

fn retrieval_observation(
    diagnostics: &RetrievalDiagnostics,
    merged_generated_count: usize,
) -> SearchRetrievalObservation {
    SearchRetrievalObservation {
        candidate_count: diagnostics.candidate_count,
        generated_candidate_count: diagnostics.generated_candidate_count,
        ranked_candidate_count: diagnostics.ranked_candidate_count,
        symbolic_generated_candidate_count: diagnostics.generated_candidate_count,
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
    }
}

fn top_k_saturation_by_anchor(diagnostics: &RetrievalDiagnostics) -> BTreeSet<String> {
    diagnostics
        .heap_truncations
        .iter()
        .map(|truncation| truncation.anchor_declaration_id.clone())
        .collect()
}

fn symbolic_source_fact(
    anchor: &HydratedDeclaration,
    candidate: &HydratedDeclaration,
    origin: &str,
    generation_rank: Option<usize>,
    top_k_status: SearchCandidateTopKStatus,
    top_k_saturated: bool,
    feature_families: Vec<String>,
) -> SearchCandidateSourceFact {
    SearchCandidateSourceFact {
        source_id: "symbolic-retrieval".to_owned(),
        source_family: SearchCandidateSourceFamily::Symbolic,
        pair_id: stable_declaration_pair_id(&anchor.declaration_id, &candidate.declaration_id),
        left_declaration_id: anchor.declaration_id.clone(),
        right_declaration_id: candidate.declaration_id.clone(),
        origin: origin.to_owned(),
        generation_rank,
        top_k_status,
        top_k_saturated,
        feature_families,
    }
}

fn stable_declaration_pair_id(left: &str, right: &str) -> String {
    if left <= right {
        format!("{left}::{right}")
    } else {
        format!("{right}::{left}")
    }
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

fn tracked_generated_stage_pairs(
    request: &SearchObservationRequest<'_>,
    ranked_pair_ids: &BTreeSet<(String, String)>,
    index_facts: &[lean_dup_index::OpenedIndexFacts],
) -> Result<Vec<SearchStageObservedPair>> {
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
        observed.push(generated_stage_pair(oriented.anchor, oriented.candidate, evidence));
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
    let default_shown = review_policy::symbolic_observation_visible(anchor, candidate, &evidence.contributions);
    let scored = score_observation(&features, variant, false, default_shown);
    SearchObservedPair {
        left: anchor.qualified_name.clone(),
        right: candidate.qualified_name.clone(),
        left_declaration_id: anchor.declaration_id.clone(),
        right_declaration_id: candidate.declaration_id.clone(),
        generated: true,
        symbolic_generated: true,
        merged_generated: true,
        ranked: scored.ranked,
        generation_policy: evidence.policy,
        rank: None,
        shown: scored.shown,
        origin: candidate.origin.clone(),
        feature_families: feature_families.clone(),
        candidate_sources: vec![symbolic_source_fact(
            anchor,
            candidate,
            &candidate.origin,
            None,
            SearchCandidateTopKStatus::GeneratedNotSelected,
            false,
            feature_families,
        )],
        survived_shown_filter: scored.survived_shown_filter,
        features,
        scoring: scored.scoring,
    }
}

fn generated_stage_pair(
    anchor: &HydratedDeclaration,
    candidate: &HydratedDeclaration,
    evidence: GeneratedPairEvidence,
) -> SearchStageObservedPair {
    let default_shown = review_policy::symbolic_observation_visible(anchor, candidate, &evidence.contributions);
    let feature_families = feature_families(&evidence.contributions);
    SearchStageObservedPair {
        left: anchor.qualified_name.clone(),
        right: candidate.qualified_name.clone(),
        left_declaration_id: anchor.declaration_id.clone(),
        right_declaration_id: candidate.declaration_id.clone(),
        generated: true,
        symbolic_generated: true,
        merged_generated: true,
        ranked: false,
        generation_policy: evidence.policy,
        rank: None,
        shown: default_shown,
        origin: candidate.origin.clone(),
        feature_families: feature_families.clone(),
        candidate_sources: vec![symbolic_source_fact(
            anchor,
            candidate,
            &candidate.origin,
            None,
            SearchCandidateTopKStatus::GeneratedNotSelected,
            false,
            feature_families,
        )],
        survived_shown_filter: default_shown,
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

#[cfg(test)]
mod tests {
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, RoleFeature};

    use super::{
        SearchCandidateSourceFamily, SearchCandidateTopKStatus, SearchObservationRequest, SearchScoringVariant,
        SearchTrackedPair, observe_search, observe_search_stages, rescore_observation,
    };

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
        assert_eq!(pair.candidate_sources.len(), 1);
        let source = &pair.candidate_sources[0];
        assert_eq!(source.source_id, "symbolic-retrieval");
        assert_eq!(source.source_family, SearchCandidateSourceFamily::Symbolic);
        assert_eq!(source.top_k_status, SearchCandidateTopKStatus::GeneratedNotSelected);
        assert_eq!(source.generation_rank, None);
        assert!(source.feature_families.contains(&"statement_fingerprint".to_owned()));
        assert!(observation.retrieval.generated_candidate_count > observation.retrieval.ranked_candidate_count);
    }

    #[test]
    fn rescoring_does_not_rerun_generation_or_expose_private_keys() {
        let rows = generated_rows(3);
        let observation = observe_search(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::SymbolicOnly,
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
    fn compact_stage_observation_preserves_eval_stage_facts() {
        let rows = generated_rows(3);
        let detailed = observe_search(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::SymbolicOnly,
        })
        .unwrap();
        let compact = observe_search_stages(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::SymbolicOnly,
        })
        .unwrap();

        assert_eq!(compact.visible_groups_found, detailed.visible_groups_found);
        assert_eq!(compact.visible_groups_total, detailed.visible_groups_total);
        assert_eq!(compact.retrieval, detailed.retrieval);
        let detailed_pairs = detailed
            .pairs
            .iter()
            .map(|pair| {
                (
                    pair.left.clone(),
                    pair.right.clone(),
                    pair.generated,
                    pair.ranked,
                    pair.rank,
                    pair.shown,
                    pair.feature_families.clone(),
                    pair.candidate_sources.clone(),
                )
            })
            .collect::<Vec<_>>();
        let compact_pairs = compact
            .pairs
            .iter()
            .map(|pair| {
                (
                    pair.left.clone(),
                    pair.right.clone(),
                    pair.generated,
                    pair.ranked,
                    pair.rank,
                    pair.shown,
                    pair.feature_families.clone(),
                    pair.candidate_sources.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(compact_pairs, detailed_pairs);
    }

    #[test]
    fn symbolic_source_facts_record_selected_rank_and_saturation_without_private_keys() {
        let rows = generated_rows(100);
        let observation = observe_search_stages(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::SymbolicOnly,
        })
        .unwrap();

        let saturated = observation
            .pairs
            .iter()
            .flat_map(|pair| pair.candidate_sources.iter())
            .find(|source| source.top_k_saturated)
            .expect("saturated symbolic source fact");

        assert_eq!(saturated.source_id, "symbolic-retrieval");
        assert_eq!(saturated.source_family, SearchCandidateSourceFamily::Symbolic);
        assert_eq!(saturated.top_k_status, SearchCandidateTopKStatus::Selected);
        assert!(saturated.generation_rank.is_some());
        assert!(!saturated.pair_id.contains("same-statement"));
        assert!(!saturated.pair_id.contains("same-role"));
    }

    #[test]
    fn compact_stage_observation_rejects_ablation_variants() {
        let rows = generated_rows(3);
        let error = observe_search_stages(SearchObservationRequest {
            workspace: &rows,
            comparison_indexes: &[],
            tracked_pairs: &[],
            scoring_variant: SearchScoringVariant::NoRoleFeatures,
        })
        .unwrap_err();

        assert!(error.to_string().contains("use detailed observations"));
    }

    fn generated_rows(count: usize) -> Vec<HydratedDeclaration> {
        (0..count)
            .map(|index| HydratedDeclaration {
                handle: DeclarationHandle::from_fixture_id(format!("synthetic-{index}")),
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
