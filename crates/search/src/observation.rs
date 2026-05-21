use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use lean_dup_index::{HydratedDeclaration, OpenedIndex};

use crate::Result;
use crate::pair_features::{SearchPairFeatures, feature_families, pair_features};
use crate::retrieval::{GeneratedPairEvidence, RetrievalDiagnostics, generated_pair_evidence, retrieve_candidates};
use crate::review_policy;
use crate::scorer::{
    SearchPairScoring, SearchScoringSummary, SearchScoringVariant, default_summary, score_observation,
};
use crate::semantic_reranking::{
    SearchSemanticObligationYield, SearchSemanticRerankingSummary, summary as semantic_reranking_summary,
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
    pub generated: bool,
    pub symbolic_generated: bool,
    pub merged_generated: bool,
    pub ranked: bool,
    pub generation_policy: String,
    pub rank: Option<usize>,
    pub shown: bool,
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
    let output = retrieve_candidates(request.workspace, request.comparison_indexes)?;
    let mut pairs = Vec::new();
    let mut ranked_pair_ids = BTreeSet::new();
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
            ranked_pair_ids.insert(pair_key(
                &set.anchor.qualified_name,
                &candidate.declaration.qualified_name,
            ));
            pairs.push(SearchObservedPair {
                left: set.anchor.qualified_name.clone(),
                right: candidate.declaration.qualified_name.clone(),
                generated: true,
                symbolic_generated: true,
                merged_generated: true,
                ranked: scored.ranked,
                generation_policy: generation_policy_for_ranked(&candidate.declaration),
                rank: scored.ranked.then_some(index + 1),
                shown: scored.shown,
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
    let default_shown = review_policy::symbolic_observation_visible(anchor, candidate, &evidence.contributions);
    let scored = score_observation(&features, variant, false, default_shown);
    SearchObservedPair {
        left: anchor.qualified_name.clone(),
        right: candidate.qualified_name.clone(),
        generated: true,
        symbolic_generated: true,
        merged_generated: true,
        ranked: scored.ranked,
        generation_policy: evidence.policy,
        rank: None,
        shown: scored.shown,
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

#[cfg(test)]
mod tests {
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, RoleFeature};

    use super::{
        SearchObservationRequest, SearchScoringVariant, SearchTrackedPair, observe_search, rescore_observation,
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
