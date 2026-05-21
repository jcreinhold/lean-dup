use std::collections::{BTreeMap, BTreeSet};

use lean_dup_search::{SearchSemanticObligationKind, SearchSemanticObligationYield, SearchSemanticRerankingSummary};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::eval::labels::GoldLabels;
use crate::eval::scoring::{CountMetric, GoldPair, ObservedRun, RecallAtK};

/// Stable stage-level denominators for search-quality evaluation.
///
/// The metrics explain where labeled pairs survive or disappear without
/// exposing retrieval keys, SQLite rows, ranking constants, or worker records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchStageMetrics {
    pub candidate_generation_recall: CountMetric,
    pub candidate_source_recall: CandidateSourceRecall,
    pub candidate_stage_recall: CandidateStageSurvival,
    pub top_k_recall_before_final_ranking: Vec<RecallAtK>,
    pub ranked_recall: Vec<RecallAtK>,
    pub visible_queue_precision: CountMetric,
    pub hard_negative_survival: HardNegativeSurvival,
    pub hard_negative_stage_survival: CandidateStageSurvival,
    pub candidate_count_by_origin: BTreeMap<String, usize>,
    pub candidate_count_by_feature_family: BTreeMap<String, usize>,
    pub generated_candidate_count_by_source_family: BTreeMap<String, usize>,
    pub generated_candidate_count_by_source_id: BTreeMap<String, usize>,
    pub generated_candidate_count_by_policy: BTreeMap<String, usize>,
    pub generated_candidate_count_by_feature_family: BTreeMap<String, usize>,
    pub hard_negative_generated_by_feature_family: BTreeMap<String, usize>,
    pub candidate_loss_metrics: CandidateLossMetrics,
    pub semantic_verification: SemanticVerificationStageMetrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CandidateStageSurvival {
    pub symbolic_generated: CountMetric,
    pub merged_generated: CountMetric,
    pub ranked: CountMetric,
    pub visible: CountMetric,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CandidateSourceRecall {
    pub symbolic_only: CountMetric,
    pub semantic_lane_only: CountMetric,
    pub merged: CountMetric,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CandidateLossMetrics {
    pub positive_fanout_pruned: CountMetric,
    pub hard_negative_fanout_pruned: CountMetric,
    pub positive_top_k_dropped: CountMetric,
    pub hard_negative_top_k_dropped: CountMetric,
    pub positive_fanout_pruned_by_feature_family: BTreeMap<String, usize>,
    pub hard_negative_fanout_pruned_by_feature_family: BTreeMap<String, usize>,
    pub positive_top_k_dropped_by_feature_family: BTreeMap<String, usize>,
    pub hard_negative_top_k_dropped_by_feature_family: BTreeMap<String, usize>,
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
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub planned: usize,
    pub cached: usize,
    pub worker: usize,
    pub unavailable: usize,
    pub obligation_yield: Vec<SearchSemanticObligationYield>,
}

pub fn score(labels: &GoldLabels, observed: &ObservedRun, k_values: &[usize]) -> SearchStageMetrics {
    let best_rank_by_pair = best_rank_by_pair(&observed.pairs);
    let generated_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.generated)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let symbolic_generated_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.symbolic_generated)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let merged_generated_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.merged_generated)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let semantic_lane_generated_pairs = observed
        .pairs
        .iter()
        .filter(|pair| {
            pair.candidate_sources
                .iter()
                .any(|source| source.source_family == "lean-semantic")
        })
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let symbolic_only_generated_pairs = symbolic_generated_pairs
        .difference(&semantic_lane_generated_pairs)
        .cloned()
        .collect::<FxHashSet<_>>();
    let semantic_lane_only_generated_pairs = semantic_lane_generated_pairs
        .difference(&symbolic_generated_pairs)
        .cloned()
        .collect::<FxHashSet<_>>();
    let ranked_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.ranked)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let top_k_dropped_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.generated && !pair.ranked)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();
    let fanout_pruned_pairs = observed
        .candidate_losses
        .iter()
        .filter(|loss| loss.loss_stage == "fanout-pruned")
        .map(|loss| loss.pair.clone())
        .collect::<FxHashSet<_>>();
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
                .filter(|pair| generated_pairs.contains(*pair))
                .count(),
            total: labels.positives.len(),
        },
        candidate_source_recall: CandidateSourceRecall {
            symbolic_only: count_labeled(&labels.positives, &symbolic_only_generated_pairs),
            semantic_lane_only: count_labeled(&labels.positives, &semantic_lane_only_generated_pairs),
            merged: count_labeled(&labels.positives, &merged_generated_pairs),
        },
        candidate_stage_recall: CandidateStageSurvival {
            symbolic_generated: count_labeled(&labels.positives, &symbolic_generated_pairs),
            merged_generated: count_labeled(&labels.positives, &merged_generated_pairs),
            ranked: count_labeled(&labels.positives, &ranked_pairs),
            visible: count_labeled(&labels.positives, &shown_pairs),
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
                    .filter(|pair| generated_pairs.contains(*pair))
                    .count(),
                total: labels.hard_negatives.len(),
            },
            top_k: top_k_hard_negatives,
            visible_queue: CountMetric {
                found: shown_pairs.intersection(&labels.hard_negatives).count(),
                total: labels.hard_negatives.len(),
            },
        },
        hard_negative_stage_survival: CandidateStageSurvival {
            symbolic_generated: count_labeled(&labels.hard_negatives, &symbolic_generated_pairs),
            merged_generated: count_labeled(&labels.hard_negatives, &merged_generated_pairs),
            ranked: count_labeled(&labels.hard_negatives, &ranked_pairs),
            visible: count_labeled(&labels.hard_negatives, &shown_pairs),
        },
        candidate_count_by_origin: count_by_origin(observed),
        candidate_count_by_feature_family: count_by_feature_family(observed),
        generated_candidate_count_by_source_family: count_generated_by_source_family(observed),
        generated_candidate_count_by_source_id: count_generated_by_source_id(observed),
        generated_candidate_count_by_policy: count_generated_by_policy(observed),
        generated_candidate_count_by_feature_family: count_generated_by_feature_family(observed),
        hard_negative_generated_by_feature_family: count_generated_hard_negatives_by_feature_family(labels, observed),
        candidate_loss_metrics: CandidateLossMetrics {
            positive_fanout_pruned: count_labeled(&labels.positives, &fanout_pruned_pairs),
            hard_negative_fanout_pruned: count_labeled(&labels.hard_negatives, &fanout_pruned_pairs),
            positive_top_k_dropped: count_labeled(&labels.positives, &top_k_dropped_pairs),
            hard_negative_top_k_dropped: count_labeled(&labels.hard_negatives, &top_k_dropped_pairs),
            positive_fanout_pruned_by_feature_family: count_losses_by_feature_family(
                &labels.positives,
                observed,
                "fanout-pruned",
            ),
            hard_negative_fanout_pruned_by_feature_family: count_losses_by_feature_family(
                &labels.hard_negatives,
                observed,
                "fanout-pruned",
            ),
            positive_top_k_dropped_by_feature_family: count_top_k_dropped_by_feature_family(
                &labels.positives,
                observed,
            ),
            hard_negative_top_k_dropped_by_feature_family: count_top_k_dropped_by_feature_family(
                &labels.hard_negatives,
                observed,
            ),
        },
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
        candidate_source_recall: CandidateSourceRecall {
            symbolic_only: sum_count(runs, |metrics| &metrics.candidate_source_recall.symbolic_only),
            semantic_lane_only: sum_count(runs, |metrics| &metrics.candidate_source_recall.semantic_lane_only),
            merged: sum_count(runs, |metrics| &metrics.candidate_source_recall.merged),
        },
        candidate_stage_recall: CandidateStageSurvival {
            symbolic_generated: sum_count(runs, |metrics| &metrics.candidate_stage_recall.symbolic_generated),
            merged_generated: sum_count(runs, |metrics| &metrics.candidate_stage_recall.merged_generated),
            ranked: sum_count(runs, |metrics| &metrics.candidate_stage_recall.ranked),
            visible: sum_count(runs, |metrics| &metrics.candidate_stage_recall.visible),
        },
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
        hard_negative_stage_survival: CandidateStageSurvival {
            symbolic_generated: sum_count(runs, |metrics| &metrics.hard_negative_stage_survival.symbolic_generated),
            merged_generated: sum_count(runs, |metrics| &metrics.hard_negative_stage_survival.merged_generated),
            ranked: sum_count(runs, |metrics| &metrics.hard_negative_stage_survival.ranked),
            visible: sum_count(runs, |metrics| &metrics.hard_negative_stage_survival.visible),
        },
        candidate_count_by_origin: sum_maps(runs.iter().map(|metrics| &metrics.candidate_count_by_origin)),
        candidate_count_by_feature_family: sum_maps(
            runs.iter().map(|metrics| &metrics.candidate_count_by_feature_family),
        ),
        generated_candidate_count_by_source_family: sum_maps(
            runs.iter()
                .map(|metrics| &metrics.generated_candidate_count_by_source_family),
        ),
        generated_candidate_count_by_source_id: sum_maps(
            runs.iter()
                .map(|metrics| &metrics.generated_candidate_count_by_source_id),
        ),
        generated_candidate_count_by_policy: sum_maps(
            runs.iter().map(|metrics| &metrics.generated_candidate_count_by_policy),
        ),
        generated_candidate_count_by_feature_family: sum_maps(
            runs.iter()
                .map(|metrics| &metrics.generated_candidate_count_by_feature_family),
        ),
        hard_negative_generated_by_feature_family: sum_maps(
            runs.iter()
                .map(|metrics| &metrics.hard_negative_generated_by_feature_family),
        ),
        candidate_loss_metrics: CandidateLossMetrics {
            positive_fanout_pruned: sum_count(runs, |metrics| &metrics.candidate_loss_metrics.positive_fanout_pruned),
            hard_negative_fanout_pruned: sum_count(runs, |metrics| {
                &metrics.candidate_loss_metrics.hard_negative_fanout_pruned
            }),
            positive_top_k_dropped: sum_count(runs, |metrics| &metrics.candidate_loss_metrics.positive_top_k_dropped),
            hard_negative_top_k_dropped: sum_count(runs, |metrics| {
                &metrics.candidate_loss_metrics.hard_negative_top_k_dropped
            }),
            positive_fanout_pruned_by_feature_family: sum_maps(
                runs.iter()
                    .map(|metrics| &metrics.candidate_loss_metrics.positive_fanout_pruned_by_feature_family),
            ),
            hard_negative_fanout_pruned_by_feature_family: sum_maps(runs.iter().map(|metrics| {
                &metrics
                    .candidate_loss_metrics
                    .hard_negative_fanout_pruned_by_feature_family
            })),
            positive_top_k_dropped_by_feature_family: sum_maps(
                runs.iter()
                    .map(|metrics| &metrics.candidate_loss_metrics.positive_top_k_dropped_by_feature_family),
            ),
            hard_negative_top_k_dropped_by_feature_family: sum_maps(runs.iter().map(|metrics| {
                &metrics
                    .candidate_loss_metrics
                    .hard_negative_top_k_dropped_by_feature_family
            })),
        },
        semantic_verification: SemanticVerificationStageMetrics {
            semantic_reranking: SearchSemanticRerankingSummary::default(),
            planned: runs.iter().map(|metrics| metrics.semantic_verification.planned).sum(),
            cached: runs.iter().map(|metrics| metrics.semantic_verification.cached).sum(),
            worker: runs.iter().map(|metrics| metrics.semantic_verification.worker).sum(),
            unavailable: runs
                .iter()
                .map(|metrics| metrics.semantic_verification.unavailable)
                .sum(),
            obligation_yield: aggregate_obligation_yield(
                runs.iter()
                    .flat_map(|metrics| metrics.semantic_verification.obligation_yield.iter()),
            ),
        },
    }
}

fn aggregate_obligation_yield<'a>(
    items: impl Iterator<Item = &'a SearchSemanticObligationYield>,
) -> Vec<SearchSemanticObligationYield> {
    let mut by_kind = BTreeMap::<SearchSemanticObligationKind, SearchSemanticObligationYield>::new();
    for item in items {
        let aggregate = by_kind
            .entry(item.kind)
            .or_insert_with(|| SearchSemanticObligationYield {
                kind: item.kind,
                ..SearchSemanticObligationYield::default()
            });
        aggregate.planned += item.planned;
        aggregate.verified += item.verified;
        aggregate.rejected += item.rejected;
        aggregate.unavailable += item.unavailable;
        aggregate.cached += item.cached;
        aggregate.worker_pairs += item.worker_pairs;
    }
    by_kind.into_values().collect()
}

fn count_labeled(
    labels: &FxHashSet<crate::eval::scoring::GoldPair>,
    observed: &FxHashSet<crate::eval::scoring::GoldPair>,
) -> CountMetric {
    CountMetric {
        found: labels.iter().filter(|pair| observed.contains(*pair)).count(),
        total: labels.len(),
    }
}

fn count_by_origin(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for pair in observed.pairs.iter().filter(|pair| pair.ranked) {
        *counts.entry(pair.origin.clone()).or_insert(0) += 1;
    }
    counts
}

fn count_by_feature_family(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for pair in observed.pairs.iter().filter(|pair| pair.ranked) {
        for family in &pair.feature_families {
            *counts.entry(family.clone()).or_insert(0) += 1;
        }
    }
    counts
}

fn count_generated_by_policy(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for pair in observed.pairs.iter().filter(|pair| pair.generated) {
        *counts.entry(pair.generation_policy.clone()).or_insert(0) += 1;
    }
    counts
}

fn count_generated_by_source_family(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut seen = BTreeSet::new();
    for pair in observed.pairs.iter().filter(|pair| pair.generated) {
        for source in &pair.candidate_sources {
            seen.insert((source.source_family.clone(), pair.pair.clone()));
        }
    }
    let mut counts = BTreeMap::new();
    for (family, _) in seen {
        *counts.entry(family).or_insert(0) += 1;
    }
    counts
}

fn count_generated_by_source_id(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut seen = BTreeSet::new();
    for pair in observed.pairs.iter().filter(|pair| pair.generated) {
        for source in &pair.candidate_sources {
            seen.insert((source.source_id.clone(), pair.pair.clone()));
        }
    }
    let mut counts = BTreeMap::new();
    for (source_id, _) in seen {
        *counts.entry(source_id).or_insert(0) += 1;
    }
    counts
}

fn count_generated_by_feature_family(observed: &ObservedRun) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for pair in observed.pairs.iter().filter(|pair| pair.generated) {
        for family in &pair.feature_families {
            *counts.entry(family.clone()).or_insert(0) += 1;
        }
    }
    counts
}

fn count_generated_hard_negatives_by_feature_family(
    labels: &GoldLabels,
    observed: &ObservedRun,
) -> BTreeMap<String, usize> {
    let mut seen_by_family = BTreeSet::new();
    for pair in observed
        .pairs
        .iter()
        .filter(|pair| pair.generated && labels.hard_negatives.contains(&pair.pair))
    {
        for family in &pair.feature_families {
            seen_by_family.insert((family.clone(), pair.pair.clone()));
        }
    }
    let mut counts = BTreeMap::new();
    for (family, _) in seen_by_family {
        *counts.entry(family).or_insert(0) += 1;
    }
    counts
}

fn count_losses_by_feature_family(
    labels: &FxHashSet<GoldPair>,
    observed: &ObservedRun,
    loss_stage: &str,
) -> BTreeMap<String, usize> {
    let mut seen_by_family = BTreeSet::new();
    for loss in observed
        .candidate_losses
        .iter()
        .filter(|loss| loss.loss_stage == loss_stage && labels.contains(&loss.pair))
    {
        seen_by_family.insert((loss.feature_family.clone(), loss.pair.clone()));
    }
    let mut counts = BTreeMap::new();
    for (family, _) in seen_by_family {
        *counts.entry(family).or_insert(0) += 1;
    }
    counts
}

fn count_top_k_dropped_by_feature_family(
    labels: &FxHashSet<GoldPair>,
    observed: &ObservedRun,
) -> BTreeMap<String, usize> {
    let mut seen_by_family = BTreeSet::new();
    for pair in observed
        .pairs
        .iter()
        .filter(|pair| pair.generated && !pair.ranked && labels.contains(&pair.pair))
    {
        for family in &pair.feature_families {
            seen_by_family.insert((family.clone(), pair.pair.clone()));
        }
    }
    let mut counts = BTreeMap::new();
    for (family, _) in seen_by_family {
        *counts.entry(family).or_insert(0) += 1;
    }
    counts
}

fn best_rank_by_pair(pairs: &[crate::eval::scoring::ObservedPair]) -> FxHashMap<GoldPair, usize> {
    let mut ranks: FxHashMap<GoldPair, usize> = FxHashMap::default();
    for observed in pairs {
        let Some(observed_rank) = observed.rank else {
            continue;
        };
        ranks
            .entry(observed.pair.clone())
            .and_modify(|rank| *rank = (*rank).min(observed_rank))
            .or_insert(observed_rank);
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

    use super::{SearchStageMetrics, SemanticVerificationStageMetrics, aggregate, score};
    use crate::eval::labels::GoldLabels;
    use crate::eval::scoring::{
        CountMetric, GoldPair, ObservedCandidateLoss, ObservedPair, ObservedRun, TimingMetrics,
    };

    #[test]
    fn stage_metrics_separate_generated_and_top_k_recall() {
        let labels = labels(["A:B", "C:D"], []);
        let observed = observed([
            pair("A", "B", 1, true, "workspace", ["statement_fingerprint"]),
            generated_pair("C", "D", "workspace", ["role_other"]),
        ]);

        let metrics = score(&labels, &observed, &[1]);

        assert_eq!(metrics.candidate_generation_recall.found, 2);
        assert_eq!(metrics.candidate_generation_recall.total, 2);
        assert_eq!(metrics.top_k_recall_before_final_ranking[0].found, 1);
        assert_eq!(metrics.top_k_recall_before_final_ranking[0].total, 2);
        assert_eq!(
            metrics.generated_candidate_count_by_policy.get("local_duplicate_audit"),
            Some(&2)
        );
        assert_eq!(
            metrics.generated_candidate_count_by_source_family.get("symbolic"),
            Some(&2)
        );
        assert_eq!(
            metrics.generated_candidate_count_by_source_id.get("symbolic-retrieval"),
            Some(&2)
        );
        assert_eq!(metrics.candidate_loss_metrics.positive_top_k_dropped.found, 1);
        assert_eq!(
            metrics
                .candidate_loss_metrics
                .positive_top_k_dropped_by_feature_family
                .get("role_other"),
            Some(&1)
        );
    }

    #[test]
    fn fanout_loss_metrics_count_labeled_pairs_by_feature_family() {
        let labels = labels(["A:B"], ["C:D"]);
        let mut observed = observed([]);
        observed.candidate_losses = vec![
            candidate_loss("A", "B", "fanout-pruned", "role_conclusion_const"),
            candidate_loss("C", "D", "fanout-pruned", "role_conclusion_const"),
        ];

        let metrics = score(&labels, &observed, &[1]);

        assert_eq!(metrics.candidate_loss_metrics.positive_fanout_pruned.found, 1);
        assert_eq!(metrics.candidate_loss_metrics.positive_fanout_pruned.total, 1);
        assert_eq!(metrics.candidate_loss_metrics.hard_negative_fanout_pruned.found, 1);
        assert_eq!(
            metrics
                .candidate_loss_metrics
                .positive_fanout_pruned_by_feature_family
                .get("role_conclusion_const"),
            Some(&1)
        );
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
        assert_eq!(
            metrics
                .hard_negative_generated_by_feature_family
                .get("statement_fingerprint"),
            Some(&1)
        );
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

    #[test]
    fn source_recall_distinguishes_symbolic_and_semantic_lane_only_pairs() {
        let labels = labels(["A:B", "C:D", "E:F"], []);
        let observed = observed([
            pair("A", "B", 1, true, "workspace", ["statement_fingerprint"]),
            semantic_lane_pair("C", "D", 2, true, "workspace", ["role_conclusion_const"]),
            merged_pair(
                "E",
                "F",
                3,
                true,
                "workspace",
                ["statement_fingerprint", "role_conclusion_const"],
            ),
        ]);

        let metrics = score(&labels, &observed, &[3]);

        assert_eq!(metrics.candidate_source_recall.symbolic_only.found, 1);
        assert_eq!(metrics.candidate_source_recall.symbolic_only.total, 3);
        assert_eq!(metrics.candidate_source_recall.semantic_lane_only.found, 1);
        assert_eq!(metrics.candidate_source_recall.semantic_lane_only.total, 3);
        assert_eq!(metrics.candidate_source_recall.merged.found, 3);
        assert_eq!(
            metrics.generated_candidate_count_by_source_family.get("symbolic"),
            Some(&2)
        );
        assert_eq!(
            metrics.generated_candidate_count_by_source_family.get("lean-semantic"),
            Some(&2)
        );
    }

    #[test]
    fn semantic_obligation_yield_aggregates_by_kind() {
        let mut first = SearchStageMetrics::default();
        first.semantic_verification.obligation_yield = vec![lean_dup_search::SearchSemanticObligationYield {
            kind: lean_dup_search::SearchSemanticObligationKind::ExactTheorem,
            planned: 2,
            verified: 1,
            rejected: 0,
            unavailable: 1,
            cached: 1,
            worker_pairs: 1,
        }];
        let mut second = SearchStageMetrics::default();
        second.semantic_verification.obligation_yield = vec![lean_dup_search::SearchSemanticObligationYield {
            kind: lean_dup_search::SearchSemanticObligationKind::ExactTheorem,
            planned: 3,
            verified: 2,
            rejected: 1,
            unavailable: 0,
            cached: 0,
            worker_pairs: 3,
        }];

        let metrics = aggregate("unit", &[&first, &second]);

        assert_eq!(metrics.semantic_verification.obligation_yield.len(), 1);
        let exact = &metrics.semantic_verification.obligation_yield[0];
        assert_eq!(exact.planned, 5);
        assert_eq!(exact.verified, 3);
        assert_eq!(exact.rejected, 1);
        assert_eq!(exact.unavailable, 1);
        assert_eq!(exact.cached, 1);
        assert_eq!(exact.worker_pairs, 4);
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
            label_facts: Vec::new(),
        }
    }

    fn observed<const N: usize>(pairs: [ObservedPair; N]) -> ObservedRun {
        ObservedRun {
            suite: "unit".to_owned(),
            pairs: pairs.into_iter().collect(),
            candidate_losses: Vec::new(),
            visible_groups: CountMetric::default(),
            probe_unavailable: CountMetric::default(),
            semantic_verification: SemanticVerificationStageMetrics::default(),
            timings: TimingMetrics::default(),
            peak_memory_bytes: None,
        }
    }

    fn candidate_loss(left: &str, right: &str, loss_stage: &str, feature_family: &str) -> ObservedCandidateLoss {
        ObservedCandidateLoss {
            pair: GoldPair::new(left, right),
            loss_stage: loss_stage.to_owned(),
            source_id: "symbolic-retrieval".to_owned(),
            source_family: "symbolic".to_owned(),
            policy: "local_duplicate_audit".to_owned(),
            source: "workspace".to_owned(),
            reason: "role-posting-limit".to_owned(),
            feature_family: feature_family.to_owned(),
            count: 513,
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
            generated: true,
            symbolic_generated: true,
            merged_generated: true,
            ranked: true,
            generation_policy: "local_duplicate_audit".to_owned(),
            rank: Some(rank),
            shown,
            origin: origin.to_owned(),
            feature_families: families.into_iter().map(str::to_owned).collect(),
            candidate_sources: candidate_sources(left, right, origin, families),
            survived_shown_filter: shown,
        }
    }

    fn generated_pair<const F: usize>(left: &str, right: &str, origin: &str, families: [&str; F]) -> ObservedPair {
        ObservedPair {
            pair: GoldPair::new(left, right),
            generated: true,
            symbolic_generated: true,
            merged_generated: true,
            ranked: false,
            generation_policy: "local_duplicate_audit".to_owned(),
            rank: None,
            shown: false,
            origin: origin.to_owned(),
            feature_families: families.into_iter().map(str::to_owned).collect(),
            candidate_sources: candidate_sources(left, right, origin, families),
            survived_shown_filter: false,
        }
    }

    fn semantic_lane_pair<const F: usize>(
        left: &str,
        right: &str,
        rank: usize,
        shown: bool,
        origin: &str,
        families: [&str; F],
    ) -> ObservedPair {
        let mut pair = pair(left, right, rank, shown, origin, families);
        pair.symbolic_generated = false;
        pair.candidate_sources = candidate_sources_with_family(
            left,
            right,
            origin,
            "lean-semantic.binder-role-shape.v1",
            "lean-semantic",
            families,
        );
        pair
    }

    fn merged_pair<const F: usize>(
        left: &str,
        right: &str,
        rank: usize,
        shown: bool,
        origin: &str,
        families: [&str; F],
    ) -> ObservedPair {
        let mut pair = pair(left, right, rank, shown, origin, families);
        pair.candidate_sources.push(candidate_source(
            left,
            right,
            origin,
            "lean-semantic.statement-meaning.v1",
            "lean-semantic",
            families,
        ));
        pair
    }

    fn candidate_sources<const F: usize>(
        left: &str,
        right: &str,
        origin: &str,
        families: [&str; F],
    ) -> Vec<crate::eval::scoring::ObservedCandidateSource> {
        vec![candidate_source(
            left,
            right,
            origin,
            "symbolic-retrieval",
            "symbolic",
            families,
        )]
    }

    fn candidate_sources_with_family<const F: usize>(
        left: &str,
        right: &str,
        origin: &str,
        source_id: &str,
        source_family: &str,
        families: [&str; F],
    ) -> Vec<crate::eval::scoring::ObservedCandidateSource> {
        vec![candidate_source(
            left,
            right,
            origin,
            source_id,
            source_family,
            families,
        )]
    }

    fn candidate_source<const F: usize>(
        left: &str,
        right: &str,
        origin: &str,
        source_id: &str,
        source_family: &str,
        families: [&str; F],
    ) -> crate::eval::scoring::ObservedCandidateSource {
        let pair = GoldPair::new(left, right);
        crate::eval::scoring::ObservedCandidateSource {
            source_id: source_id.to_owned(),
            source_family: source_family.to_owned(),
            pair_id: format!("{}::{}", pair.left, pair.right),
            left_declaration_id: left.to_owned(),
            right_declaration_id: right.to_owned(),
            origin: origin.to_owned(),
            generation_rank: None,
            top_k_status: "generated-not-selected".to_owned(),
            top_k_saturated: false,
            feature_families: families.into_iter().map(str::to_owned).collect(),
        }
    }

    fn gold_pair(text: &str) -> GoldPair {
        let (left, right) = text.split_once(':').unwrap();
        GoldPair::new(left, right)
    }
}
