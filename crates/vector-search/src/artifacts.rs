use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use lean_dup_eval::{
    CountMetric, EvaluationMetrics, GoldLabelFact, GoldLabels, GoldPair, LabelFactSource, LabelPolarity, TypedGoldLabel,
};

use crate::candidates::VectorCandidateSummary;
use crate::leak_check;
use crate::scoring::{RankedVectorPair, VectorPair};
use crate::{Error, Result, VectorValidationBounds};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VectorSearchReport {
    pub(crate) schema_version: &'static str,
    pub(crate) suite: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    pub(crate) vector_candidates: VectorCandidateSummary,
    pub(crate) symbolic_baseline: EvaluationMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) vector_search: Option<EvaluationMetrics>,
    pub(crate) vector_stage_metrics: VectorStageMetrics,
    pub(crate) scorer_variants: Vec<VectorScorerVariantReport>,
    pub(crate) pairs: Vec<VectorSearchPairReport>,
    pub(crate) validation_bounds: VectorValidationBounds,
    pub(crate) validation_cost: VectorValidationCostSummary,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) children: Vec<VectorSearchChildReport>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct VectorValidationCostSummary {
    pub(crate) peak_rss_bytes: Option<u64>,
    pub(crate) rss_status: String,
    pub(crate) model_cache_bytes: Option<u64>,
    pub(crate) text_vector_cache_bytes: Option<u64>,
    pub(crate) vector_corpus_bytes: Option<u64>,
    pub(crate) eligible_corpus_size: usize,
    pub(crate) query_count: usize,
    pub(crate) top_k: usize,
    pub(crate) top_k_saturated: bool,
    pub(crate) cold_build_ms: u128,
    pub(crate) warm_open_query_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) artifact_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorScorerVariantReport {
    pub(crate) scorer_variant_id: String,
    pub(crate) vector_feature_version: String,
    pub(crate) metrics: EvaluationMetrics,
    pub(crate) vector_stage_metrics: VectorStageMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorSearchPairReport {
    pub(crate) left: String,
    pub(crate) right: String,
    pub(crate) left_hash: String,
    pub(crate) right_hash: String,
    pub(crate) label_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<VectorSearchLabelReport>,
    pub(crate) label_facts: Vec<VectorSearchLabelFactReport>,
    pub(crate) symbolic_generated: bool,
    pub(crate) vector_generated: bool,
    pub(crate) merged_generated: bool,
    pub(crate) ranked: bool,
    pub(crate) visible: bool,
    pub(crate) rank: Option<usize>,
    pub(crate) vector_rank: Option<usize>,
    pub(crate) generation_policies: Vec<String>,
    pub(crate) feature_families: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VectorSearchLabelReport {
    pub(crate) polarity: lean_dup_eval::LabelPolarity,
    pub(crate) match_class: lean_dup_eval::MatchClass,
    pub(crate) expected_stage_visibility: lean_dup_eval::ExpectedStageVisibility,
    pub(crate) adjudication_source: lean_dup_eval::AdjudicationSource,
    pub(crate) confidence: lean_dup_eval::LabelConfidence,
    pub(crate) semantic_verification_required: bool,
    pub(crate) static_evidence_acceptable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VectorSearchLabelFactReport {
    pub(crate) status: String,
    pub(crate) source: LabelFactSource,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VectorSearchChildReport {
    pub(crate) suite: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metrics: Option<EvaluationMetrics>,
}

pub(crate) fn report_path(suite: &str) -> PathBuf {
    PathBuf::from("target/lean-dup/vector-search").join(format!("{suite}.json"))
}

pub(crate) fn write(root: &Path, artifact: PathBuf, report: &VectorSearchReport) -> Result<PathBuf> {
    let json = serde_json::to_string_pretty(report).map_err(|source| Error::Json {
        message: "could not serialize vector search artifact",
        path: artifact.clone(),
        source,
    })?;
    leak_check::check(&json)?;
    let absolute = root.join(&artifact);
    let parent = absolute.parent().ok_or_else(|| Error::InvalidRequest {
        message: format!("vector artifact has no parent: {}", absolute.display()),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        message: "could not create vector artifact directory",
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(&absolute, json).map_err(|source| Error::Io {
        message: "could not write vector search artifact",
        path: absolute,
        source,
    })?;
    Ok(artifact)
}

pub(crate) fn pair_reports(labels: &GoldLabels, ranked: &[RankedVectorPair]) -> Vec<VectorSearchPairReport> {
    let typed_by_pair = labels
        .typed_pairs
        .iter()
        .map(|label| (label.pair.clone(), label))
        .collect::<BTreeMap<_, _>>();
    let facts_by_pair = label_facts_by_pair(labels);
    ranked
        .iter()
        .map(|ranked| {
            let key = row_pair(&ranked.pair);
            let facts = facts_by_pair.get(&key).cloned().unwrap_or_default();
            VectorSearchPairReport {
                left: key.left.clone(),
                right: key.right.clone(),
                left_hash: ranked.pair.left_hash.clone(),
                right_hash: ranked.pair.right_hash.clone(),
                label_status: label_status(labels, &key, &facts),
                label: typed_by_pair.get(&key).map(|label| label_report(label)),
                label_facts: facts.iter().map(label_fact_report).collect(),
                symbolic_generated: ranked.pair.symbolic_generated,
                vector_generated: ranked.pair.vector_generated,
                merged_generated: ranked.pair.merged_generated,
                ranked: ranked.ranked,
                visible: ranked.visible,
                rank: ranked.rank,
                vector_rank: ranked.pair.vector_rank,
                generation_policies: sorted(ranked.pair.generation_policies.clone()),
                feature_families: sorted(ranked.pair.feature_families.clone()),
            }
        })
        .collect()
}

pub(crate) fn vector_stage_metrics(
    labels: &GoldLabels,
    rows: &[VectorSearchPairReport],
    top_k_saturation: CountMetric,
) -> VectorStageMetrics {
    let vector_generated = rows
        .iter()
        .filter(|row| row.vector_generated)
        .map(row_pair_report)
        .collect::<BTreeSet<_>>();
    let symbolic_generated = rows
        .iter()
        .filter(|row| row.symbolic_generated)
        .map(row_pair_report)
        .collect::<BTreeSet<_>>();
    let merged_generated = rows
        .iter()
        .filter(|row| row.merged_generated)
        .map(row_pair_report)
        .collect::<BTreeSet<_>>();
    let ranked = rows
        .iter()
        .filter(|row| row.ranked)
        .map(row_pair_report)
        .collect::<BTreeSet<_>>();
    let visible = rows
        .iter()
        .filter(|row| row.visible)
        .map(row_pair_report)
        .collect::<BTreeSet<_>>();
    let vector_only = vector_generated
        .difference(&symbolic_generated)
        .cloned()
        .collect::<BTreeSet<_>>();
    let symbolic_only = symbolic_generated
        .difference(&vector_generated)
        .cloned()
        .collect::<BTreeSet<_>>();
    VectorStageMetrics {
        vector_top_k_recall: count_labeled(&labels.positives, &vector_generated),
        vector_top_k_precision: CountMetric {
            found: vector_generated
                .iter()
                .filter(|pair| labels.positives.contains(*pair))
                .count(),
            total: vector_generated.len(),
        },
        top_k_saturation,
        vector_only_positives: count_labeled(&labels.positives, &vector_only),
        vector_only_hard_negatives: count_labeled(&labels.hard_negatives, &vector_only),
        symbolic_only_positives: count_labeled(&labels.positives, &symbolic_only),
        symbolic_only_hard_negatives: count_labeled(&labels.hard_negatives, &symbolic_only),
        merged_generated_recall: count_labeled(&labels.positives, &merged_generated),
        ranked_recall: count_labeled(&labels.positives, &ranked),
        visible_precision: CountMetric {
            found: visible.iter().filter(|pair| labels.positives.contains(*pair)).count(),
            total: visible.len(),
        },
        visible_hard_negative_count: count_labeled(&labels.hard_negatives, &visible),
    }
}

fn label_report(label: &TypedGoldLabel) -> VectorSearchLabelReport {
    VectorSearchLabelReport {
        polarity: label.polarity,
        match_class: label.match_class,
        expected_stage_visibility: label.expected_stage_visibility,
        adjudication_source: label.adjudication_source,
        confidence: label.confidence,
        semantic_verification_required: label.semantic_verification_required,
        static_evidence_acceptable: label.static_evidence_acceptable,
    }
}

fn label_fact_report(fact: &GoldLabelFact) -> VectorSearchLabelFactReport {
    VectorSearchLabelFactReport {
        status: fact_status(fact).to_owned(),
        source: fact.source,
    }
}

fn label_status(labels: &GoldLabels, pair: &GoldPair, facts: &[GoldLabelFact]) -> String {
    if labels.positives.contains(pair) {
        if facts
            .iter()
            .any(|fact| fact.polarity == LabelPolarity::Positive && is_expanded_source(fact.source))
        {
            "expanded-positive"
        } else {
            "positive"
        }
        .to_owned()
    } else if labels.hard_negatives.contains(pair) {
        if facts
            .iter()
            .any(|fact| fact.polarity == LabelPolarity::HardNegative && is_expanded_source(fact.source))
        {
            "expanded-hard-negative"
        } else {
            "hard-negative"
        }
        .to_owned()
    } else {
        "unlabeled".to_owned()
    }
}

fn fact_status(fact: &GoldLabelFact) -> &'static str {
    match (fact.polarity, fact.source) {
        (LabelPolarity::Positive, source) if is_expanded_source(source) => "expanded-positive",
        (LabelPolarity::HardNegative, source) if is_expanded_source(source) => "expanded-hard-negative",
        (LabelPolarity::Positive, _) => "positive",
        (LabelPolarity::HardNegative, _) => "hard-negative",
    }
}

fn is_expanded_source(source: LabelFactSource) -> bool {
    matches!(source, LabelFactSource::TypedCluster)
}

fn label_facts_by_pair(labels: &GoldLabels) -> BTreeMap<GoldPair, Vec<GoldLabelFact>> {
    let mut by_pair = BTreeMap::<GoldPair, Vec<GoldLabelFact>>::new();
    for fact in &labels.label_facts {
        by_pair.entry(fact.pair.clone()).or_default().push(fact.clone());
    }
    by_pair
}

fn row_pair(row: &VectorPair) -> GoldPair {
    GoldPair::new(row.left.clone(), row.right.clone())
}

fn row_pair_report(row: &VectorSearchPairReport) -> GoldPair {
    GoldPair::new(row.left.clone(), row.right.clone())
}

fn count_labeled(labels: &rustc_hash::FxHashSet<GoldPair>, observed: &BTreeSet<GoldPair>) -> CountMetric {
    CountMetric {
        found: labels.iter().filter(|pair| observed.contains(*pair)).count(),
        total: labels.len(),
    }
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct VectorStageMetrics {
    pub(crate) vector_top_k_recall: CountMetric,
    pub(crate) vector_top_k_precision: CountMetric,
    pub(crate) top_k_saturation: CountMetric,
    pub(crate) vector_only_positives: CountMetric,
    pub(crate) vector_only_hard_negatives: CountMetric,
    pub(crate) symbolic_only_positives: CountMetric,
    pub(crate) symbolic_only_hard_negatives: CountMetric,
    pub(crate) merged_generated_recall: CountMetric,
    pub(crate) ranked_recall: CountMetric,
    pub(crate) visible_precision: CountMetric,
    pub(crate) visible_hard_negative_count: CountMetric,
}
