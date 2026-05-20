use std::fs;
use std::path::{Path, PathBuf};

use lean_dup_search::{
    SearchEmbeddingDocumentPolicy, SearchObservation, SearchObservedPair, SearchVectorAcquisitionPolicy,
    SearchVectorCandidateRequest, SearchVectorCandidateStatus, SearchVectorCandidateSummary,
    SearchVectorEligibilityPolicy,
};
use serde::Serialize;

use crate::eval::labels::{GoldLabels, TypedGoldLabel};
use crate::eval::scoring::{EvaluationMetrics, GoldPair};
use crate::{Error, Result};

pub const VECTOR_SEARCH_SCHEMA_VERSION: &str = "lean-dup.vector-search.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorSearchRequest {
    pub model_id: String,
    pub revision: Option<String>,
    pub acquisition_policy: SearchVectorAcquisitionPolicy,
    pub model_cache_root: Option<PathBuf>,
    pub text_vector_cache_root: Option<PathBuf>,
    pub corpus_cache_root: Option<PathBuf>,
    pub document_policy: SearchEmbeddingDocumentPolicy,
    pub eligibility_policy: SearchVectorEligibilityPolicy,
}

#[derive(Debug, Clone)]
pub(crate) struct VectorSearchArtifactOutcome {
    pub status: String,
    pub artifact: PathBuf,
    pub metrics: Option<EvaluationMetrics>,
}

pub(crate) struct VectorSearchReportRun<'a> {
    pub repo_root: &'a Path,
    pub suite: &'a str,
    pub labels: &'a GoldLabels,
    pub observation: &'a SearchObservation,
    pub symbolic_baseline: &'a EvaluationMetrics,
    pub vector_metrics: &'a EvaluationMetrics,
    pub scorer_version: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VectorSearchReport {
    pub schema_version: &'static str,
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub scorer_version: String,
    pub vector_candidates: SearchVectorCandidateSummary,
    pub symbolic_baseline: EvaluationMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search: Option<EvaluationMetrics>,
    pub pairs: Vec<VectorSearchPairReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<VectorSearchChildReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorSearchPairReport {
    pub left: String,
    pub right: String,
    pub label_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<VectorSearchLabelReport>,
    pub symbolic_generated: bool,
    pub vector_generated: bool,
    pub merged_generated: bool,
    pub ranked: bool,
    pub visible: bool,
    pub rank: Option<usize>,
    pub vector_rank: Option<usize>,
    pub vector_score: Option<f64>,
    pub generation_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VectorSearchLabelReport {
    pub polarity: crate::eval::labels::LabelPolarity,
    pub match_class: crate::eval::labels::MatchClass,
    pub expected_stage_visibility: crate::eval::labels::ExpectedStageVisibility,
    pub adjudication_source: crate::eval::labels::AdjudicationSource,
    pub confidence: crate::eval::labels::LabelConfidence,
    pub semantic_verification_required: bool,
    pub static_evidence_acceptable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VectorSearchChildReport {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
}

impl VectorSearchRequest {
    pub(crate) fn to_search_request(&self, suite: &str) -> SearchVectorCandidateRequest {
        SearchVectorCandidateRequest {
            model_id: self.model_id.clone(),
            revision: self.revision.clone(),
            acquisition_policy: self.acquisition_policy,
            model_cache_root: self.model_cache_root.clone(),
            text_vector_cache_root: self.text_vector_cache_root.clone(),
            corpus_cache_root: self
                .corpus_cache_root
                .clone()
                .unwrap_or_else(|| PathBuf::from("target/search-quality/vector-corpus").join(suite)),
            document_policy: self.document_policy,
            eligibility_policy: self.eligibility_policy,
        }
    }
}

pub(crate) fn report(run: VectorSearchReportRun<'_>) -> Result<VectorSearchArtifactOutcome> {
    let status = status_label(run.observation.retrieval.vector_candidates.status);
    let metrics = (run.observation.retrieval.vector_candidates.status == SearchVectorCandidateStatus::Ok)
        .then(|| run.vector_metrics.clone());
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: run.suite.to_owned(),
        status: status.clone(),
        reason: run.observation.retrieval.vector_candidates.reason.clone(),
        scorer_version: run.scorer_version.to_owned(),
        vector_candidates: run.observation.retrieval.vector_candidates.clone(),
        symbolic_baseline: run.symbolic_baseline.clone(),
        vector_search: metrics.clone(),
        pairs: pair_reports(run.labels, run.observation),
        children: Vec::new(),
    };
    let artifact = write_default_artifact(run.repo_root, &report)?;
    Ok(VectorSearchArtifactOutcome {
        status,
        artifact,
        metrics,
    })
}

pub(crate) fn aggregate(
    repo_root: &Path,
    suite: &str,
    scorer_version: &str,
    symbolic_baseline: &EvaluationMetrics,
    children: Vec<VectorSearchChildReport>,
) -> Result<VectorSearchArtifactOutcome> {
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
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: suite.to_owned(),
        status: status.to_owned(),
        reason: (completed.is_empty()).then(|| "no completed child vector search metrics".to_owned()),
        scorer_version: scorer_version.to_owned(),
        vector_candidates: SearchVectorCandidateSummary::default(),
        symbolic_baseline: symbolic_baseline.clone(),
        vector_search: None,
        pairs: Vec::new(),
        children,
    };
    let artifact = write_default_artifact(repo_root, &report)?;
    Ok(VectorSearchArtifactOutcome {
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
) -> VectorSearchChildReport {
    VectorSearchChildReport {
        suite,
        status: status.unwrap_or_else(|| "skipped".to_owned()),
        reason,
        artifact,
        metrics,
    }
}

fn pair_reports(labels: &GoldLabels, observation: &SearchObservation) -> Vec<VectorSearchPairReport> {
    let typed_by_pair = labels
        .typed_pairs
        .iter()
        .map(|label| (label.pair.clone(), label))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut rows = observation
        .pairs
        .iter()
        .map(|pair| {
            let key = GoldPair::new(pair.left.clone(), pair.right.clone());
            let label = typed_by_pair.get(&key).map(|typed| label_report(typed));
            pair_report(pair, key, label)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.left
            .cmp(&right.left)
            .then_with(|| left.right.cmp(&right.right))
            .then_with(|| left.rank.cmp(&right.rank))
    });
    rows
}

fn pair_report(
    pair: &SearchObservedPair,
    key: GoldPair,
    label: Option<VectorSearchLabelReport>,
) -> VectorSearchPairReport {
    VectorSearchPairReport {
        left: key.left,
        right: key.right,
        label_status: label_status(label.as_ref()),
        label,
        symbolic_generated: pair.symbolic_generated,
        vector_generated: pair.vector_generated,
        merged_generated: pair.merged_generated,
        ranked: pair.ranked,
        visible: pair.shown,
        rank: pair.rank,
        vector_rank: pair.vector_rank,
        vector_score: pair.vector_score,
        generation_policy: pair.generation_policy.clone(),
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

fn label_status(label: Option<&VectorSearchLabelReport>) -> String {
    label.map_or_else(
        || "unlabeled".to_owned(),
        |label| match label.polarity {
            crate::eval::labels::LabelPolarity::Positive => "positive".to_owned(),
            crate::eval::labels::LabelPolarity::HardNegative => "hard-negative".to_owned(),
        },
    )
}

fn status_label(status: SearchVectorCandidateStatus) -> String {
    match status {
        SearchVectorCandidateStatus::Disabled | SearchVectorCandidateStatus::Skipped => "skipped",
        SearchVectorCandidateStatus::Failed => "failed",
        SearchVectorCandidateStatus::Ok => "ok",
    }
    .to_owned()
}

pub(crate) fn write_default_artifact(repo_root: &Path, report: &VectorSearchReport) -> Result<PathBuf> {
    let artifact = PathBuf::from("target/search-quality").join(format!("{}-vector-search.json", report.suite));
    let absolute = repo_root.join(&artifact);
    let parent = absolute.parent().ok_or_else(|| Error::Eval {
        message: format!("vector search artifact has no parent: {}", absolute.display()),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        message: "could not create vector search artifact directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&absolute, json).map_err(|source| Error::Io {
        message: "could not write vector search artifact",
        path: absolute,
        source,
    })?;
    Ok(artifact)
}
