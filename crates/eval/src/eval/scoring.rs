use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::eval::labels::GoldLabels;
use crate::eval::stage_metrics::{SearchStageMetrics, SemanticVerificationStageMetrics};

/// An unordered declaration pair used by evaluation labels and observations.
///
/// Pair identity is independent of candidate direction. This lets a scorer
/// treat `A -> B` and `B -> A` as the same retrieval success.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct GoldPair {
    pub left: String,
    pub right: String,
}

impl GoldPair {
    pub fn new(left: impl Into<String>, right: impl Into<String>) -> Self {
        let left = left.into();
        let right = right.into();
        if left <= right {
            Self { left, right }
        } else {
            Self {
                left: right,
                right: left,
            }
        }
    }
}

/// One retrieved candidate pair observed during an evaluation run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservedPair {
    pub pair: GoldPair,
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
}

/// Observed candidates and measured costs for one corpus run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservedRun {
    pub suite: String,
    pub pairs: Vec<ObservedPair>,
    pub visible_groups: CountMetric,
    pub probe_unavailable: CountMetric,
    pub semantic_verification: SemanticVerificationStageMetrics,
    pub timings: TimingMetrics,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TimingMetrics {
    pub index_load_ms: u128,
    pub retrieval_ms: u128,
    pub probe_ms: u128,
    pub total_ms: u128,
}

/// Evaluation metrics with raw denominators preserved.
///
/// Renderers may choose text or JSON, but percentage-like quantities stay as
/// `found/total` counts so reports cannot hide corpus size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvaluationMetrics {
    pub suite: String,
    pub recall: Vec<RecallAtK>,
    pub shown_queue_precision: CountMetric,
    pub hard_negative_hits: CountMetric,
    pub visible_groups: CountMetric,
    pub probe_unavailable: CountMetric,
    pub stage_metrics: SearchStageMetrics,
    pub candidate_count: usize,
    pub timings: TimingMetrics,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallAtK {
    pub k: usize,
    pub found: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CountMetric {
    pub found: usize,
    pub total: usize,
}

/// Score an observed run against gold positive and hard-negative labels.
///
/// The scorer knows only unordered pairs, candidate ranks, and whether a pair
/// would enter the shown queue. Corpus paths, retrieval weights, probe policy,
/// and report layout belong to callers.
pub fn score_run(labels: &GoldLabels, observed: &ObservedRun, k_values: &[usize]) -> EvaluationMetrics {
    let best_rank_by_pair = best_rank_by_pair(&observed.pairs);
    let shown_pairs = observed
        .pairs
        .iter()
        .filter(|pair| pair.shown)
        .map(|pair| pair.pair.clone())
        .collect::<FxHashSet<_>>();

    let recall = normalized_k_values(k_values)
        .into_iter()
        .map(|k| RecallAtK {
            k,
            found: labels
                .positives
                .iter()
                .filter(|pair| best_rank_by_pair.get(*pair).is_some_and(|rank| *rank <= k))
                .count(),
            total: labels.positives.len(),
        })
        .collect();

    let shown_true_positives = shown_pairs.intersection(&labels.positives).count();
    let hard_negative_hits = shown_pairs.intersection(&labels.hard_negatives).count();

    let stage_metrics = crate::eval::stage_metrics::score(labels, observed, k_values);

    EvaluationMetrics {
        suite: observed.suite.clone(),
        recall,
        shown_queue_precision: CountMetric {
            found: shown_true_positives,
            total: shown_pairs.len(),
        },
        hard_negative_hits: CountMetric {
            found: hard_negative_hits,
            total: labels.hard_negatives.len(),
        },
        visible_groups: observed.visible_groups.clone(),
        probe_unavailable: observed.probe_unavailable.clone(),
        stage_metrics,
        candidate_count: observed.pairs.iter().filter(|pair| pair.ranked).count(),
        timings: observed.timings.clone(),
        peak_memory_bytes: observed.peak_memory_bytes,
    }
}

fn best_rank_by_pair(pairs: &[ObservedPair]) -> FxHashMap<GoldPair, usize> {
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

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::{CountMetric, GoldPair, ObservedPair, ObservedRun, TimingMetrics, score_run};
    use crate::eval::labels::GoldLabels;
    use crate::eval::stage_metrics::SemanticVerificationStageMetrics;

    #[test]
    fn recall_at_k_reports_raw_counts() {
        let labels = labels(["A:B", "C:D"], []);
        let observed = observed([("B", "A", 1, true), ("C", "D", 6, false), ("X", "Y", 1, true)]);

        let metrics = score_run(&labels, &observed, &[1, 5, 10]);

        assert_eq!(metrics.recall[0].k, 1);
        assert_eq!(metrics.recall[0].found, 1);
        assert_eq!(metrics.recall[0].total, 2);
        assert_eq!(metrics.recall[1].found, 1);
        assert_eq!(metrics.recall[2].found, 2);
    }

    #[test]
    fn shown_queue_precision_reports_raw_counts() {
        let labels = labels(["A:B"], ["A:C"]);
        let observed = observed([("B", "A", 1, true), ("C", "A", 2, true), ("D", "E", 3, false)]);

        let metrics = score_run(&labels, &observed, &[5]);

        assert_eq!(metrics.shown_queue_precision.found, 1);
        assert_eq!(metrics.shown_queue_precision.total, 2);
        assert_eq!(metrics.hard_negative_hits.found, 1);
        assert_eq!(metrics.hard_negative_hits.total, 1);
    }

    #[test]
    fn hard_negatives_are_not_positives() {
        let labels = labels(["A:B"], ["A:B", "A:C"]);
        let observed = observed([("A", "B", 1, true), ("A", "C", 2, true)]);

        let metrics = score_run(&labels, &observed, &[5]);

        assert_eq!(metrics.shown_queue_precision.found, 1);
        assert_eq!(metrics.shown_queue_precision.total, 2);
        assert_eq!(metrics.hard_negative_hits.found, 1);
        assert_eq!(metrics.hard_negative_hits.total, 1);
    }

    fn labels<const P: usize, const N: usize>(positives: [&str; P], negatives: [&str; N]) -> GoldLabels {
        let positives = positives.into_iter().map(pair).collect::<FxHashSet<_>>();
        let hard_negatives = negatives
            .into_iter()
            .map(pair)
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

    fn observed<const N: usize>(pairs: [(&str, &str, usize, bool); N]) -> ObservedRun {
        ObservedRun {
            suite: "unit".to_owned(),
            pairs: pairs
                .into_iter()
                .map(|(left, right, rank, shown)| ObservedPair {
                    pair: GoldPair::new(left, right),
                    generated: true,
                    symbolic_generated: true,
                    merged_generated: true,
                    ranked: true,
                    generation_policy: "local_duplicate_audit".to_owned(),
                    rank: Some(rank),
                    shown,
                    origin: "workspace".to_owned(),
                    feature_families: vec!["statement_fingerprint".to_owned()],
                    survived_shown_filter: shown,
                })
                .collect(),
            visible_groups: CountMetric::default(),
            probe_unavailable: CountMetric::default(),
            semantic_verification: SemanticVerificationStageMetrics::default(),
            timings: TimingMetrics::default(),
            peak_memory_bytes: None,
        }
    }

    fn pair(text: &str) -> GoldPair {
        let (left, right) = text.split_once(':').unwrap();
        GoldPair::new(left, right)
    }
}
