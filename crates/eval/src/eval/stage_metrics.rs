use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::eval::labels::GoldLabels;
use crate::eval::scoring::{CountMetric, GoldPair, ObservedRun, RecallAtK};
use lean_dup_search::retrieval::KeyContribution;

/// Stable stage-level denominators for search-quality evaluation.
///
/// The metrics explain where labeled pairs survive or disappear without
/// exposing retrieval keys, SQLite rows, ranking constants, or worker records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchStageMetrics {
    pub candidate_generation_recall: CountMetric,
    pub top_k_recall_before_final_ranking: Vec<RecallAtK>,
    pub ranked_recall: Vec<RecallAtK>,
    pub visible_queue_precision: CountMetric,
    pub hard_negative_survival: HardNegativeSurvival,
    pub candidate_count_by_origin: BTreeMap<String, usize>,
    pub candidate_count_by_feature_family: BTreeMap<String, usize>,
    pub semantic_verification: SemanticVerificationStageMetrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HardNegativeSurvival {
    pub candidate_generation: CountMetric,
    pub top_k: Vec<CountAtK>,
    pub visible_queue: CountMetric,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CountAtK {
    pub k: usize,
    pub found: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SemanticVerificationStageMetrics {
    pub planned: usize,
    pub cached: usize,
    pub worker: usize,
    pub unavailable: usize,
}

pub fn score(labels: &GoldLabels, observed: &ObservedRun, k_values: &[usize]) -> SearchStageMetrics {
    let best_rank_by_pair = best_rank_by_pair(&observed.pairs);
    let shown_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.survived_shown_filter)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let k_values = normalized_k_values(k_values);

    let top_k_recall_before_final_ranking = k_values
        .iter()
        .map(|k| RecallAtK {
            k: *k,
            found: labels
                .positives
                .iter()
                .filter(|pair| best_rank_by_pair.get(*pair).is_some_and(|rank| *rank <= *k))
                .count(),
            total: labels.positives.len(),
        })
        .collect::<Vec<_>>();

    let top_k_hard_negatives = k_values
        .iter()
        .map(|k| CountAtK {
            k: *k,
            found: labels
                .hard_negatives
                .iter()
                .filter(|pair| best_rank_by_pair.get(*pair).is_some_and(|rank| *rank <= *k))
                .count(),
            total: labels.hard_negatives.len(),
        })
        .collect::<Vec<_>>();

    SearchStageMetrics {
        candidate_generation_recall: CountMetric {
            found: labels
                .positives
                .iter()
                .filter(|pair| best_rank_by_pair.contains_key(*pair))
                .count(),
            total: labels.positives.len(),
        },
        ranked_recall: top_k_recall_before_final_ranking.clone(),
        top_k_recall_before_final_ranking,
        visible_queue_precision: CountMetric {
            found: shown_pairs.intersection(&labels.positives).count(),
            total: shown_pairs.len(),
        },
        hard_negative_survival: HardNegativeSurvival {
            candidate_generation: CountMetric {
                found: labels
                    .hard_negatives
                    .iter()
                    .filter(|pair| best_rank_by_pair.contains_key(*pair))
                    .count(),
                total: labels.hard_negatives.len(),
            },
            top_k: top_k_hard_negatives,
            visible_queue: CountMetric {
                found: shown_pairs.intersection(&labels.hard_negatives).count(),
                total: labels.hard_negatives.len(),
            },
        },
        candidate_count_by_origin: count_by_origin(observed),
        candidate_count_by_feature_family: count_by_feature_family(observed),
        semantic_verification: observed.semantic_verification.clone(),
    }
}

pub fn aggregate(_suite: &str, runs: &[&SearchStageMetrics]) -> SearchStageMetrics {
    let k_values = {
        let mut values = runs
            .iter()
            .flat_map(|metrics| {
                metrics
                    .ranked_recall
                    .iter()
                    .map(|recall| recall.k)
                    .chain(metrics.hard_negative_survival.top_k.iter().map(|count| count.k))
            })
            .collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        values
    };

    SearchStageMetrics {
        candidate_generation_recall: sum_count(runs, |metrics| &metrics.candidate_generation_recall),
        top_k_recall_before_final_ranking: sum_recall(&k_values, runs, |metrics| {
            &metrics.top_k_recall_before_final_ranking
        }),
        ranked_recall: sum_recall(&k_values, runs, |metrics| &metrics.ranked_recall),
        visible_queue_precision: sum_count(runs, |metrics| &metrics.visible_queue_precision),
        hard_negative_survival: HardNegativeSurvival {
            candidate_generation: sum_count(runs, |metrics| &metrics.hard_negative_survival.candidate_generation),
            top_k: sum_count_at_k(&k_values, runs),
            visible_queue: sum_count(runs, |metrics| &metrics.hard_negative_survival.visible_queue),
        },
        candidate_count_by_origin: sum_maps(runs.iter().map(|metrics| &metrics.candidate_count_by_origin)),
        candidate_count_by_feature_family: sum_maps(
            runs.iter().map(|metrics| &metrics.candidate_count_by_feature_family),
        ),
        semantic_verification: SemanticVerificationStageMetrics {
            planned: runs.iter().map(|metrics| metrics.semantic_verification.planned).sum(),
            cached: runs.iter().map(|metrics| metrics.semantic_verification.cached).sum(),
            worker: runs.iter().map(|metrics| metrics.semantic_verification.worker).sum(),
            unavailable: runs
                .iter()
                .map(|metrics| metrics.semantic_verification.unavailable)
                .sum(),
        },
    }
}

/// Convert retrieval evidence into stable feature-family labels for eval.
///
/// Families are intentionally coarser than retrieval keys: they let quality
/// reports explain which kind of evidence produced candidates without exposing
/// key values, table names, or Lean-owned feature encodings.
pub fn feature_families(contributions: &[KeyContribution]) -> Vec<String> {
    let mut families = contributions
        .iter()
        .map(feature_family)
        .collect::<FxHashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if families.is_empty() {
        families.push("unknown".to_owned());
    }
    families.sort();
    families
}

fn feature_family(contribution: &KeyContribution) -> String {
    match contribution.kind.as_str() {
        "statement-fingerprint" => "statement_fingerprint".to_owned(),
        "safe-permutation-fingerprint" => "safe_permutation_fingerprint".to_owned(),
        "connective-fingerprint" => "connective_fingerprint".to_owned(),
        "conclusion-fingerprint" => "conclusion_fingerprint".to_owned(),
        "role-feature" => match contribution.role.as_deref() {
            Some("conclusion_const") => "role_conclusion_const".to_owned(),
            Some("hypothesis_const") => "role_hypothesis_const".to_owned(),
            Some("conclusion_head" | "hypothesis_head" | "binder_domain_head") => "role_head".to_owned(),
            _ => "role_other".to_owned(),
        },
        _ => "other".to_owned(),
    }
}

fn count_by_origin(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for pair in &observed.pairs {
        *counts.entry(pair.origin.clone()).or_insert(0) += 1;
    }
    counts
}

fn count_by_feature_family(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for pair in &observed.pairs {
        for family in &pair.feature_families {
            *counts.entry(family.clone()).or_insert(0) += 1;
        }
    }
    counts
}

fn best_rank_by_pair(pairs: &[crate::eval::scoring::ObservedPair]) -> FxHashMap<GoldPair, usize> {
    let mut ranks: FxHashMap<GoldPair, usize> = FxHashMap::default();
    for observed in pairs {
        ranks
            .entry(observed.pair.clone())
            .and_modify(|rank| *rank = (*rank).min(observed.rank))
            .or_insert(observed.rank);
    }
    ranks
}

fn normalized_k_values(k_values: &[usize]) -> Vec<usize> {
    let mut values = if k_values.is_empty() {
        vec![1, 5, 10]
    } else {
        k_values.iter().copied().filter(|k| *k > 0).collect()
    };
    values.sort_unstable();
    values.dedup();
    values
}

fn sum_count<'a>(
    runs: &[&'a SearchStageMetrics],
    metric: impl Fn(&'a SearchStageMetrics) -> &'a CountMetric,
) -> CountMetric {
    CountMetric {
        found: runs.iter().map(|run| metric(run).found).sum(),
        total: runs.iter().map(|run| metric(run).total).sum(),
    }
}

fn sum_recall<'a>(
    k_values: &[usize],
    runs: &[&'a SearchStageMetrics],
    metric: impl Fn(&'a SearchStageMetrics) -> &'a [RecallAtK],
) -> Vec<RecallAtK> {
    k_values
        .iter()
        .map(|k| RecallAtK {
            k: *k,
            found: runs
                .iter()
                .filter_map(|metrics| metric(metrics).iter().find(|recall| recall.k == *k))
                .map(|recall| recall.found)
                .sum(),
            total: runs
                .iter()
                .filter_map(|metrics| metric(metrics).iter().find(|recall| recall.k == *k))
                .map(|recall| recall.total)
                .sum(),
        })
        .collect()
}

fn sum_count_at_k(k_values: &[usize], runs: &[&SearchStageMetrics]) -> Vec<CountAtK> {
    k_values
        .iter()
        .map(|k| CountAtK {
            k: *k,
            found: runs
                .iter()
                .filter_map(|metrics| metrics.hard_negative_survival.top_k.iter().find(|count| count.k == *k))
                .map(|count| count.found)
                .sum(),
            total: runs
                .iter()
                .filter_map(|metrics| metrics.hard_negative_survival.top_k.iter().find(|count| count.k == *k))
                .map(|count| count.total)
                .sum(),
        })
        .collect()
}

fn sum_maps<'a>(maps: impl Iterator<Item = &'a BTreeMap<String, usize>>) -> BTreeMap<String, usize> {
    let mut summed = BTreeMap::new();
    for map in maps {
        for (key, value) in map {
            *summed.entry(key.clone()).or_insert(0) += value;
        }
    }
    summed
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::{SemanticVerificationStageMetrics, feature_families, score};
    use crate::eval::labels::GoldLabels;
    use crate::eval::scoring::{CountMetric, GoldPair, ObservedPair, ObservedRun, TimingMetrics};
    use lean_dup_search::retrieval::KeyContribution;

    #[test]
    fn stage_metrics_separate_generated_and_top_k_recall() {
        let labels = labels(["A:B", "C:D"], []);
        let observed = observed([
            pair("A", "B", 1, true, "workspace", ["statement_fingerprint"]),
            pair("C", "D", 4, false, "workspace", ["role_other"]),
        ]);

        let metrics = score(&labels, &observed, &[1]);

        assert_eq!(metrics.candidate_generation_recall.found, 2);
        assert_eq!(metrics.candidate_generation_recall.total, 2);
        assert_eq!(metrics.top_k_recall_before_final_ranking[0].found, 1);
        assert_eq!(metrics.top_k_recall_before_final_ranking[0].total, 2);
    }

    #[test]
    fn hard_negatives_are_counted_at_each_stage() {
        let labels = labels(["A:B"], ["A:C", "D:E"]);
        let observed = observed([
            pair("A", "C", 1, true, "workspace", ["statement_fingerprint"]),
            pair("D", "E", 4, false, "workspace", ["role_other"]),
        ]);

        let metrics = score(&labels, &observed, &[1]);

        assert_eq!(metrics.hard_negative_survival.candidate_generation.found, 2);
        assert_eq!(metrics.hard_negative_survival.top_k[0].found, 1);
        assert_eq!(metrics.hard_negative_survival.visible_queue.found, 1);
    }

    #[test]
    fn feature_family_names_hide_raw_keys() {
        let families = feature_families(&[
            contribution("statement-fingerprint", None, "opaque-statement-key"),
            contribution("role-feature", Some("conclusion_const"), "opaque-role-key"),
            contribution("role-feature", Some("binder_domain_head"), "opaque-head-key"),
        ]);

        assert_eq!(
            families,
            vec![
                "role_conclusion_const".to_owned(),
                "role_head".to_owned(),
                "statement_fingerprint".to_owned()
            ]
        );
        assert!(!families.iter().any(|family| family.contains("opaque")));
    }

    #[test]
    fn semantic_counts_default_to_zero_for_retrieval_only_runs() {
        let labels = labels(["A:B"], []);
        let observed = observed([pair("A", "B", 1, true, "workspace", ["statement_fingerprint"])]);

        let metrics = score(&labels, &observed, &[1]);

        assert_eq!(
            metrics.semantic_verification,
            SemanticVerificationStageMetrics::default()
        );
    }

    fn labels<const P: usize, const N: usize>(positives: [&str; P], negatives: [&str; N]) -> GoldLabels {
        let positives = positives.into_iter().map(gold_pair).collect::<FxHashSet<_>>();
        let hard_negatives = negatives
            .into_iter()
            .map(gold_pair)
            .filter(|pair| !positives.contains(pair))
            .collect();
        GoldLabels {
            suite: "unit".to_owned(),
            positives,
            hard_negatives,
            typed_pairs: Vec::new(),
        }
    }

    fn observed<const N: usize>(pairs: [ObservedPair; N]) -> ObservedRun {
        ObservedRun {
            suite: "unit".to_owned(),
            pairs: pairs.into_iter().collect(),
            visible_groups: CountMetric::default(),
            probe_unavailable: CountMetric::default(),
            semantic_verification: SemanticVerificationStageMetrics::default(),
            timings: TimingMetrics::default(),
            peak_memory_bytes: None,
        }
    }

    fn pair<const F: usize>(
        left: &str,
        right: &str,
        rank: usize,
        shown: bool,
        origin: &str,
        families: [&str; F],
    ) -> ObservedPair {
        ObservedPair {
            pair: GoldPair::new(left, right),
            rank,
            shown,
            origin: origin.to_owned(),
            feature_families: families.into_iter().map(str::to_owned).collect(),
            survived_shown_filter: shown,
        }
    }

    fn gold_pair(text: &str) -> GoldPair {
        let (left, right) = text.split_once(':').unwrap();
        GoldPair::new(left, right)
    }

    fn contribution(kind: &str, role: Option<&str>, key: &str) -> KeyContribution {
        KeyContribution {
            kind: kind.to_owned(),
            role: role.map(str::to_owned),
            display: None,
            key: key.to_owned(),
            score: 1.0,
        }
    }
}
