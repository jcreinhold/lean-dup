use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lean_dup_embedding::{
    EmbeddingAcquisitionPolicy, EmbeddingCacheStatus, EmbeddingInputPolicy, EmbeddingInputRole, EmbeddingModelSpec,
    EmbeddingModelSummary, EmbeddingPrepareRequest, EmbeddingRuntimeCounters, TextEmbeddingBatchRequest,
    TextEmbeddingInput, embed_text_batch, prepare_embedding_model,
};
use lean_dup_search::{SearchEmbeddingDocuments, SearchObservation, SearchObservedPair};
use serde::Serialize;

use crate::eval::labels::{GoldLabels, TypedGoldLabel};
use crate::eval::scoring::{EvaluationMetrics, GoldPair, ObservedPair, ObservedRun, TimingMetrics, score_run};
use crate::{Error, Result};

pub const EMBEDDING_RERANK_SCHEMA_VERSION: &str = "lean-dup.embedding-rerank.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingRerankRequest {
    pub model: EmbeddingModelSpec,
    pub acquisition_policy: EmbeddingAcquisitionPolicy,
    pub model_cache_root: Option<PathBuf>,
    pub vector_cache_root: Option<PathBuf>,
}

impl Default for EmbeddingRerankRequest {
    fn default() -> Self {
        Self {
            model: EmbeddingModelSpec::default_experiment_model(),
            acquisition_policy: EmbeddingAcquisitionPolicy::CacheOnly,
            model_cache_root: None,
            vector_cache_root: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EmbeddingRerankArtifactOutcome {
    pub status: String,
    pub artifact: PathBuf,
    pub metrics: Option<EvaluationMetrics>,
}

pub(crate) struct EmbeddingRerankRun<'a> {
    pub repo_root: &'a Path,
    pub suite: &'a str,
    pub request: &'a EmbeddingRerankRequest,
    pub labels: &'a GoldLabels,
    pub observation: &'a SearchObservation,
    pub baseline_metrics: &'a EvaluationMetrics,
    pub scorer_version: &'a str,
    pub k_values: &'a [usize],
}

struct ReportBase<'a> {
    suite: &'a str,
    request: &'a EmbeddingRerankRequest,
    documents: &'a SearchEmbeddingDocuments,
    baseline_metrics: &'a EvaluationMetrics,
    scorer_version: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRerankReport {
    pub schema_version: &'static str,
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub model: EmbeddingRerankModelReport,
    pub cache: EmbeddingRerankCacheReport,
    pub acquisition_policy: EmbeddingAcquisitionPolicy,
    pub input_policy_id: String,
    pub input_policy_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<EmbeddingRuntimeCounters>,
    pub symbolic_baseline: EmbeddingRerankBaselineReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_rerank: Option<EmbeddingRerankMetricsReport>,
    pub pairs: Vec<EmbeddingRerankPairReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<EmbeddingRerankChildReport>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRerankModelReport {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRerankCacheReport {
    pub status: EmbeddingCacheStatus,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRerankBaselineReport {
    pub scorer_version: String,
    pub metrics: EvaluationMetrics,
    pub visible_budget: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRerankMetricsReport {
    pub metrics: EvaluationMetrics,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRerankChildReport {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct EmbeddingRerankPairReport {
    pub left: String,
    pub right: String,
    pub label_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<EmbeddingRerankLabelReport>,
    pub baseline_rank: Option<usize>,
    pub baseline_visible: bool,
    pub left_content_hash: String,
    pub right_content_hash: String,
    pub embedding_similarity: Option<f64>,
    pub embedding_rank: Option<usize>,
    pub embedding_top_budget_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EmbeddingRerankLabelReport {
    pub polarity: crate::eval::labels::LabelPolarity,
    pub match_class: crate::eval::labels::MatchClass,
    pub expected_stage_visibility: crate::eval::labels::ExpectedStageVisibility,
    pub adjudication_source: crate::eval::labels::AdjudicationSource,
    pub confidence: crate::eval::labels::LabelConfidence,
    pub semantic_verification_required: bool,
    pub static_evidence_acceptable: bool,
}

pub(crate) fn run(run: EmbeddingRerankRun<'_>) -> Result<EmbeddingRerankArtifactOutcome> {
    let base = ReportBase {
        suite: run.suite,
        request: run.request,
        documents: &run.observation.embedding_documents,
        baseline_metrics: run.baseline_metrics,
        scorer_version: run.scorer_version,
    };
    let prepare_result = match prepare_embedding_model(EmbeddingPrepareRequest {
        model: run.request.model.clone(),
        acquisition_policy: run.request.acquisition_policy,
        cache_root: run.request.model_cache_root.clone(),
    }) {
        Ok(result) => result,
        Err(error) => {
            let report = skipped_or_failed_report(
                &base,
                "failed",
                Some(stable_error_reason(&error)),
                EmbeddingCacheStatus::Skipped,
                Vec::new(),
                None,
            );
            let artifact = write_default_artifact(run.repo_root, &report)?;
            return Ok(EmbeddingRerankArtifactOutcome {
                status: "failed".to_owned(),
                artifact,
                metrics: None,
            });
        }
    };

    if prepare_result.cache.status != EmbeddingCacheStatus::Prepared {
        let cache_status = prepare_result.cache.status.clone();
        let report = skipped_or_failed_report(
            &base,
            "skipped",
            Some(skip_reason(cache_status.clone())),
            cache_status,
            Vec::new(),
            Some(model_report_from_summary(prepare_result.model)),
        );
        let artifact = write_default_artifact(run.repo_root, &report)?;
        return Ok(EmbeddingRerankArtifactOutcome {
            status: "skipped".to_owned(),
            artifact,
            metrics: None,
        });
    }

    let embedding_result = match embed_text_batch(TextEmbeddingBatchRequest {
        model: run.request.model.clone(),
        role: EmbeddingInputRole::Document,
        input_policy: embedding_input_policy(&run.observation.embedding_documents),
        inputs: run
            .observation
            .embedding_documents
            .text_inputs()
            .iter()
            .map(|input| TextEmbeddingInput {
                id: input.declaration_name.clone(),
                text: input.text.clone(),
            })
            .collect(),
        model_cache_root: run.request.model_cache_root.clone(),
        vector_cache_root: run.request.vector_cache_root.clone(),
    }) {
        Ok(result) => result,
        Err(error) => {
            let report = skipped_or_failed_report(
                &base,
                "failed",
                Some(stable_error_reason(&error)),
                EmbeddingCacheStatus::Prepared,
                Vec::new(),
                None,
            );
            let artifact = write_default_artifact(run.repo_root, &report)?;
            return Ok(EmbeddingRerankArtifactOutcome {
                status: "failed".to_owned(),
                artifact,
                metrics: None,
            });
        }
    };

    let vectors = embedding_result
        .vectors
        .iter()
        .map(|vector| (vector.input_id.clone(), vector.values.clone()))
        .collect::<BTreeMap<_, _>>();
    let (pairs, metrics) = build_pairs_and_metrics(
        run.labels,
        run.observation,
        run.baseline_metrics.shown_queue_precision.total,
        &vectors,
        Some(&embedding_result.runtime),
        run.k_values,
    );
    let report = EmbeddingRerankReport {
        schema_version: EMBEDDING_RERANK_SCHEMA_VERSION,
        suite: run.suite.to_owned(),
        status: "ok".to_owned(),
        reason: None,
        model: model_report_from_summary(embedding_result.model),
        cache: EmbeddingRerankCacheReport {
            status: embedding_result.cache.status,
        },
        acquisition_policy: run.request.acquisition_policy,
        input_policy_id: run.observation.embedding_documents.policy_id.clone(),
        input_policy_version: run.observation.embedding_documents.policy_version.clone(),
        runtime: Some(embedding_result.runtime),
        symbolic_baseline: baseline_report(run.scorer_version, run.baseline_metrics),
        embedding_rerank: Some(EmbeddingRerankMetricsReport {
            metrics: metrics.clone(),
        }),
        pairs,
        children: Vec::new(),
    };
    let artifact = write_default_artifact(run.repo_root, &report)?;
    Ok(EmbeddingRerankArtifactOutcome {
        status: "ok".to_owned(),
        artifact,
        metrics: Some(metrics),
    })
}

pub(crate) fn aggregate(
    repo_root: &Path,
    request: &EmbeddingRerankRequest,
    suite: &str,
    baseline_metrics: &EvaluationMetrics,
    scorer_version: &str,
    children: Vec<EmbeddingRerankChildReport>,
) -> Result<EmbeddingRerankArtifactOutcome> {
    let completed = children
        .iter()
        .filter_map(|child| child.metrics.as_ref())
        .collect::<Vec<_>>();
    let status = if completed.is_empty() {
        "skipped"
    } else if children.iter().any(|child| child.status == "failed") {
        "failed"
    } else if children.iter().any(|child| child.status == "skipped") {
        "incomplete"
    } else {
        "ok"
    };
    let report = EmbeddingRerankReport {
        schema_version: EMBEDDING_RERANK_SCHEMA_VERSION,
        suite: suite.to_owned(),
        status: status.to_owned(),
        reason: (completed.is_empty()).then(|| "no completed child embedding rerank metrics".to_owned()),
        model: EmbeddingRerankModelReport {
            id: request.model.id.clone(),
            revision: request.model.revision.clone(),
            fingerprint: None,
            profile_id: None,
            backend_family: None,
            dimension: None,
            input_roles: Vec::new(),
        },
        cache: EmbeddingRerankCacheReport {
            status: EmbeddingCacheStatus::Skipped,
        },
        acquisition_policy: request.acquisition_policy,
        input_policy_id: "aggregate".to_owned(),
        input_policy_version: lean_dup_embedding::EMBEDDING_INPUT_POLICY_VERSION.to_owned(),
        runtime: None,
        symbolic_baseline: baseline_report(scorer_version, baseline_metrics),
        embedding_rerank: None,
        pairs: Vec::new(),
        children,
    };
    let artifact = write_default_artifact(repo_root, &report)?;
    Ok(EmbeddingRerankArtifactOutcome {
        status: status.to_owned(),
        artifact,
        metrics: None,
    })
}

pub(crate) fn child_report(
    suite: String,
    status: Option<String>,
    artifact: Option<PathBuf>,
    metrics: Option<EvaluationMetrics>,
    reason: Option<String>,
) -> EmbeddingRerankChildReport {
    EmbeddingRerankChildReport {
        suite,
        status: status.unwrap_or_else(|| "skipped".to_owned()),
        reason,
        artifact,
        metrics,
    }
}

fn skipped_or_failed_report(
    base: &ReportBase<'_>,
    status: &str,
    reason: Option<String>,
    cache_status: EmbeddingCacheStatus,
    children: Vec<EmbeddingRerankChildReport>,
    model: Option<EmbeddingRerankModelReport>,
) -> EmbeddingRerankReport {
    EmbeddingRerankReport {
        schema_version: EMBEDDING_RERANK_SCHEMA_VERSION,
        suite: base.suite.to_owned(),
        status: status.to_owned(),
        reason,
        model: model.unwrap_or_else(|| EmbeddingRerankModelReport {
            id: base.request.model.id.clone(),
            revision: base.request.model.revision.clone(),
            fingerprint: None,
            profile_id: None,
            backend_family: None,
            dimension: None,
            input_roles: Vec::new(),
        }),
        cache: EmbeddingRerankCacheReport { status: cache_status },
        acquisition_policy: base.request.acquisition_policy,
        input_policy_id: base.documents.policy_id.clone(),
        input_policy_version: base.documents.policy_version.clone(),
        runtime: None,
        symbolic_baseline: baseline_report(base.scorer_version, base.baseline_metrics),
        embedding_rerank: None,
        pairs: Vec::new(),
        children,
    }
}

fn model_report_from_summary(summary: EmbeddingModelSummary) -> EmbeddingRerankModelReport {
    EmbeddingRerankModelReport {
        id: summary.id,
        revision: summary.revision,
        fingerprint: summary.fingerprint,
        profile_id: Some(summary.profile_id),
        backend_family: Some(summary.backend_family),
        dimension: Some(summary.dimension),
        input_roles: summary.input_roles,
    }
}

fn build_pairs_and_metrics(
    labels: &GoldLabels,
    observation: &SearchObservation,
    visible_budget: usize,
    vectors: &BTreeMap<String, Vec<f32>>,
    runtime: Option<&EmbeddingRuntimeCounters>,
    k_values: &[usize],
) -> (Vec<EmbeddingRerankPairReport>, EvaluationMetrics) {
    let typed_by_pair = labels
        .typed_pairs
        .iter()
        .map(|label| (label.pair.clone(), label))
        .collect::<BTreeMap<_, _>>();
    let content_hash_by_name = observation
        .embedding_documents
        .documents
        .iter()
        .map(|document| (document.declaration_name.clone(), document.content_hash.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut scored = observation
        .pairs
        .iter()
        .enumerate()
        .map(|(index, pair)| PairScore {
            index,
            similarity: pair_similarity(pair, vectors),
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                observation.pairs[left.index]
                    .left
                    .cmp(&observation.pairs[right.index].left)
            })
            .then_with(|| {
                observation.pairs[left.index]
                    .right
                    .cmp(&observation.pairs[right.index].right)
            })
    });
    let rank_by_index = scored
        .iter()
        .enumerate()
        .filter_map(|(rank_index, score)| score.similarity.map(|_| (score.index, rank_index + 1)))
        .collect::<BTreeMap<_, _>>();

    let mut rows = observation
        .pairs
        .iter()
        .enumerate()
        .map(|(index, observed)| {
            let pair = GoldPair::new(observed.left.clone(), observed.right.clone());
            let label = typed_by_pair.get(&pair).map(|typed| label_report(typed));
            let embedding_rank = rank_by_index.get(&index).copied();
            EmbeddingRerankPairReport {
                left: pair.left,
                right: pair.right,
                label_status: label_status(label.as_ref()),
                label,
                baseline_rank: observed.rank,
                baseline_visible: observed.shown,
                left_content_hash: content_hash_by_name.get(&observed.left).cloned().unwrap_or_default(),
                right_content_hash: content_hash_by_name.get(&observed.right).cloned().unwrap_or_default(),
                embedding_similarity: pair_similarity(observed, vectors),
                embedding_rank,
                embedding_top_budget_visible: embedding_rank.is_some_and(|rank| rank <= visible_budget),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.left
            .cmp(&right.left)
            .then_with(|| left.right.cmp(&right.right))
            .then_with(|| left.baseline_rank.cmp(&right.baseline_rank))
    });

    let observed_run = ObservedRun {
        suite: labels.suite.clone(),
        pairs: observation
            .pairs
            .iter()
            .enumerate()
            .map(|(index, observed)| {
                let rank = rank_by_index.get(&index).copied();
                ObservedPair {
                    pair: GoldPair::new(observed.left.clone(), observed.right.clone()),
                    generated: observed.generated,
                    symbolic_generated: observed.symbolic_generated,
                    vector_generated: observed.vector_generated,
                    merged_generated: observed.merged_generated,
                    ranked: rank.is_some(),
                    generation_policy: observed.generation_policy.clone(),
                    rank,
                    shown: rank.is_some_and(|rank| rank <= visible_budget),
                    origin: observed.origin.clone(),
                    feature_families: observed.feature_families.clone(),
                    survived_shown_filter: rank.is_some_and(|rank| rank <= visible_budget),
                }
            })
            .collect(),
        visible_groups: crate::eval::scoring::CountMetric {
            found: visible_budget.min(rank_by_index.len()),
            total: rank_by_index.len(),
        },
        probe_unavailable: crate::eval::scoring::CountMetric::default(),
        semantic_verification: crate::eval::stage_metrics::SemanticVerificationStageMetrics {
            semantic_reranking: observation.semantic_reranking.clone(),
            obligation_yield: observation.semantic_obligation_yield.clone(),
            ..crate::eval::stage_metrics::SemanticVerificationStageMetrics::default()
        },
        timings: TimingMetrics {
            index_load_ms: 0,
            retrieval_ms: runtime.map_or(0, runtime_total_ms),
            probe_ms: 0,
            total_ms: runtime.map_or(0, runtime_total_ms),
        },
        peak_memory_bytes: runtime.and_then(|counters| counters.peak_rss_bytes),
    };
    let metrics = score_run(labels, &observed_run, k_values);
    (rows, metrics)
}

fn runtime_total_ms(counters: &EmbeddingRuntimeCounters) -> u128 {
    counters.model_load_ms.saturating_add(counters.inference_ms)
}

fn pair_similarity(pair: &SearchObservedPair, vectors: &BTreeMap<String, Vec<f32>>) -> Option<f64> {
    let left = vectors.get(&pair.left)?;
    let right = vectors.get(&pair.right)?;
    cosine_similarity(left, right)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f64> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left, right) in left.iter().zip(right.iter()) {
        let left = f64::from(*left);
        let right = f64::from(*right);
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }
    Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
}

fn embedding_input_policy(documents: &SearchEmbeddingDocuments) -> EmbeddingInputPolicy {
    EmbeddingInputPolicy {
        policy_id: documents.policy_id.clone(),
        version: documents.policy_version.clone(),
        includes_declaration_name: documents.policy_id == "name-and-formal-statement",
        includes_normalized_statement: documents.policy_id != "informal-or-formal",
        uses_informal_text_when_available: documents.policy_id == "informal-or-formal",
    }
}

fn baseline_report(scorer_version: &str, metrics: &EvaluationMetrics) -> EmbeddingRerankBaselineReport {
    EmbeddingRerankBaselineReport {
        scorer_version: scorer_version.to_owned(),
        metrics: metrics.clone(),
        visible_budget: metrics.shown_queue_precision.total,
    }
}

fn label_report(label: &TypedGoldLabel) -> EmbeddingRerankLabelReport {
    EmbeddingRerankLabelReport {
        polarity: label.polarity,
        match_class: label.match_class,
        expected_stage_visibility: label.expected_stage_visibility,
        adjudication_source: label.adjudication_source,
        confidence: label.confidence,
        semantic_verification_required: label.semantic_verification_required,
        static_evidence_acceptable: label.static_evidence_acceptable,
    }
}

fn label_status(label: Option<&EmbeddingRerankLabelReport>) -> String {
    label.map_or_else(
        || "unlabeled".to_owned(),
        |label| match label.polarity {
            crate::eval::labels::LabelPolarity::Positive => "positive".to_owned(),
            crate::eval::labels::LabelPolarity::HardNegative => "hard-negative".to_owned(),
        },
    )
}

fn skip_reason(status: EmbeddingCacheStatus) -> String {
    match status {
        EmbeddingCacheStatus::NotPrepared => "skipped_no_prepared_embedding_model".to_owned(),
        EmbeddingCacheStatus::Unusable => "skipped_unusable_embedding_model".to_owned(),
        EmbeddingCacheStatus::Skipped => "skipped_embedding_model".to_owned(),
        EmbeddingCacheStatus::Prepared => "embedding_model_prepared".to_owned(),
    }
}

fn stable_error_reason(error: &lean_dup_embedding::Error) -> String {
    error.to_string()
}

pub(crate) fn write_default_artifact(repo_root: &Path, report: &EmbeddingRerankReport) -> Result<PathBuf> {
    let artifact = PathBuf::from("target/search-quality").join(format!("{}-embedding-rerank.json", report.suite));
    let absolute = repo_root.join(&artifact);
    let parent = absolute.parent().ok_or_else(|| Error::Eval {
        message: format!("embedding rerank artifact has no parent: {}", absolute.display()),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        message: "could not create embedding rerank artifact directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&absolute, format!("{json}\n")).map_err(|source| Error::Io {
        message: "could not write embedding rerank artifact",
        path: absolute,
        source,
    })?;
    Ok(artifact)
}

#[derive(Debug, Clone)]
struct PairScore {
    index: usize,
    similarity: Option<f64>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rustc_hash::FxHashSet;
    use tempfile::TempDir;

    use super::{
        EMBEDDING_RERANK_SCHEMA_VERSION, EmbeddingRerankRequest, ReportBase, build_pairs_and_metrics,
        skipped_or_failed_report, write_default_artifact,
    };
    use crate::eval::labels::{
        AdjudicationSource, ExpectedStageVisibility, GoldLabels, LabelConfidence, LabelPolarity, MatchClass,
        TypedGoldLabel,
    };
    use crate::eval::scoring::{CountMetric, EvaluationMetrics, GoldPair, RecallAtK, TimingMetrics};
    use crate::eval::stage_metrics::SearchStageMetrics;
    use lean_dup_embedding::EmbeddingCacheStatus;
    use lean_dup_search::{
        SearchEmbeddingDocument, SearchEmbeddingDocuments, SearchEvidenceMode, SearchModuleRelation, SearchObservation,
        SearchObservedPair, SearchPairFeatures, SearchRetrievalObservation, SearchScoringSummary, SearchScoringVariant,
        SearchSemanticEvidenceState,
    };

    #[test]
    fn embedding_rerank_uses_symbolic_shown_budget_and_labels() {
        let labels = labels();
        let observation = observation();
        let vectors = BTreeMap::from([
            ("Tiny.same_left".to_owned(), vec![1.0, 0.0]),
            ("Tiny.same_right".to_owned(), vec![1.0, 0.0]),
            ("Tiny.noise_left".to_owned(), vec![0.0, 1.0]),
            ("Tiny.noise_right".to_owned(), vec![0.0, 1.0]),
        ]);

        let (pairs, metrics) = build_pairs_and_metrics(&labels, &observation, 1, &vectors, None, &[1, 5]);

        assert_eq!(pairs[0].left, "Tiny.noise_left");
        assert_eq!(pairs[0].label_status, "hard-negative");
        assert_eq!(pairs[0].left_content_hash, "hash-Tiny.noise_left");
        assert_eq!(pairs[0].right_content_hash, "hash-Tiny.noise_right");
        assert_eq!(pairs[1].left, "Tiny.same_left");
        assert_eq!(pairs[1].label_status, "positive");
        assert!(pairs.iter().filter(|pair| pair.embedding_top_budget_visible).count() == 1);
        assert_eq!(metrics.shown_queue_precision.total, 1);
    }

    #[test]
    fn skipped_artifact_has_schema_and_no_private_paths() {
        let temp = TempDir::new().unwrap();
        let request = EmbeddingRerankRequest::default();
        let baseline = baseline_metrics();
        let observation = observation();
        let base = ReportBase {
            suite: "unit",
            request: &request,
            documents: &observation.embedding_documents,
            baseline_metrics: &baseline,
            scorer_version: "lean-dup.symbolic-scorer.v1",
        };
        let report = skipped_or_failed_report(
            &base,
            "skipped",
            Some("skipped_no_prepared_embedding_model".to_owned()),
            EmbeddingCacheStatus::NotPrepared,
            Vec::new(),
            None,
        );

        let artifact = write_default_artifact(temp.path(), &report).unwrap();
        let json = std::fs::read_to_string(temp.path().join(artifact)).unwrap();

        assert!(json.contains(EMBEDDING_RERANK_SCHEMA_VERSION));
        assert!(json.contains("skipped_no_prepared_embedding_model"));
        assert!(!json.contains("/Users/"));
        assert!(!json.contains("model.safetensors"));
        assert!(!json.contains("sqlite"));
        assert!(!json.contains("posting"));
        assert!(!json.contains("statement_text"));
        assert!(!json.contains("Tiny theorem text"));
    }

    fn labels() -> GoldLabels {
        let positive = GoldPair::new("Tiny.same_left", "Tiny.same_right");
        let negative = GoldPair::new("Tiny.noise_left", "Tiny.noise_right");
        GoldLabels {
            suite: "unit".to_owned(),
            positives: FxHashSet::from_iter([positive.clone()]),
            hard_negatives: FxHashSet::from_iter([negative.clone()]),
            typed_pairs: vec![
                TypedGoldLabel {
                    pair: positive,
                    polarity: LabelPolarity::Positive,
                    match_class: MatchClass::ExactTheoremDuplicate,
                    expected_stage_visibility: ExpectedStageVisibility::Visible,
                    adjudication_source: AdjudicationSource::FixtureIntent,
                    confidence: LabelConfidence::High,
                    semantic_verification_required: false,
                    static_evidence_acceptable: true,
                },
                TypedGoldLabel {
                    pair: negative,
                    polarity: LabelPolarity::HardNegative,
                    match_class: MatchClass::HardNegative,
                    expected_stage_visibility: ExpectedStageVisibility::Hidden,
                    adjudication_source: AdjudicationSource::FixtureIntent,
                    confidence: LabelConfidence::High,
                    semantic_verification_required: false,
                    static_evidence_acceptable: true,
                },
            ],
            label_facts: Vec::new(),
        }
    }

    fn observation() -> SearchObservation {
        SearchObservation {
            pairs: vec![
                observed("Tiny.same_left", "Tiny.same_right", Some(1), true),
                observed("Tiny.noise_left", "Tiny.noise_right", Some(2), false),
            ],
            visible_groups_found: 1,
            visible_groups_total: 2,
            scoring: SearchScoringSummary::new(SearchScoringVariant::SymbolicOnly),
            semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
            semantic_obligation_yield: Vec::new(),
            retrieval: SearchRetrievalObservation::default(),
            embedding_documents: SearchEmbeddingDocuments {
                policy_id: "name-and-formal-statement".to_owned(),
                policy_version: lean_dup_embedding::EMBEDDING_INPUT_POLICY_VERSION.to_owned(),
                documents: vec![
                    embedding_document("Tiny.same_left"),
                    embedding_document("Tiny.same_right"),
                    embedding_document("Tiny.noise_left"),
                    embedding_document("Tiny.noise_right"),
                ],
            },
        }
    }

    fn embedding_document(name: &str) -> SearchEmbeddingDocument {
        SearchEmbeddingDocument {
            declaration_name: name.to_owned(),
            module_name: "Tiny".to_owned(),
            declaration_kind: "theorem".to_owned(),
            normalized_formal_statement: "Tiny theorem text".to_owned(),
            informal_text: None,
            content_hash: format!("hash-{name}"),
        }
    }

    fn observed(left: &str, right: &str, rank: Option<usize>, shown: bool) -> SearchObservedPair {
        SearchObservedPair {
            left: left.to_owned(),
            right: right.to_owned(),
            generated: true,
            symbolic_generated: true,
            vector_generated: false,
            merged_generated: true,
            ranked: rank.is_some(),
            generation_policy: "local_duplicate_audit".to_owned(),
            rank,
            shown,
            left_content_hash: None,
            right_content_hash: None,
            vector_score: None,
            vector_rank: None,
            origin: "workspace".to_owned(),
            feature_families: vec!["statement_fingerprint".to_owned()],
            survived_shown_filter: shown,
            features: SearchPairFeatures {
                retrieval_feature_families: vec!["statement_fingerprint".to_owned()],
                declaration_kinds: vec!["theorem".to_owned()],
                evidence_mode: SearchEvidenceMode::Local,
                vector_evidence: None,
                structural_fingerprint_families: vec!["statement_fingerprint".to_owned()],
                role_overlap: Vec::new(),
                module_relation: SearchModuleRelation::SameModule {
                    module: "Tiny".to_owned(),
                },
                semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                semantic_evidence_state: SearchSemanticEvidenceState::NotRun,
                semantic_obligations: Vec::new(),
                cheap_blockers: Vec::new(),
            },
            scoring: lean_dup_search::SearchPairScoring {
                version: "lean-dup.symbolic-scorer.v1",
                variant: SearchScoringVariant::SymbolicOnly,
                total_score: 1.0,
                component_scores: BTreeMap::new(),
            },
        }
    }

    fn baseline_metrics() -> EvaluationMetrics {
        EvaluationMetrics {
            suite: "unit".to_owned(),
            recall: vec![RecallAtK {
                k: 1,
                found: 1,
                total: 1,
            }],
            shown_queue_precision: CountMetric { found: 1, total: 1 },
            hard_negative_hits: CountMetric { found: 0, total: 1 },
            visible_groups: CountMetric { found: 1, total: 2 },
            probe_unavailable: CountMetric::default(),
            stage_metrics: SearchStageMetrics::default(),
            candidate_count: 2,
            timings: TimingMetrics::default(),
            peak_memory_bytes: None,
        }
    }
}
