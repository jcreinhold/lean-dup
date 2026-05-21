use std::fs;
use std::path::{Path, PathBuf};

use lean_dup_search::{SearchScoringVariant, SearchSemanticObligationYield, SearchSemanticRerankingSummary};
use serde::Serialize;

use crate::eval::scoring::EvaluationMetrics;
use crate::{Error, Result};

pub const SCORER_ABLATION_SCHEMA_VERSION: &str = "lean-dup.scorer-ablation.v1";

#[derive(Debug, Clone, Serialize)]
pub struct ScorerAblationReport {
    pub schema_version: &'static str,
    pub suite: String,
    pub scorer_version: String,
    pub review_policy_version: String,
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub semantic_obligation_yield: Vec<SearchSemanticObligationYield>,
    pub variants: Vec<ScorerAblationVariantReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ScorerAblationChildReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScorerAblationChildReport {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub variants: Vec<ScorerAblationVariantReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScorerAblationVariantReport {
    pub variant: SearchScoringVariant,
    pub status: String,
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub semantic_obligation_yield: Vec<SearchSemanticObligationYield>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn write_default_artifact(repo_root: &Path, report: &ScorerAblationReport) -> Result<PathBuf> {
    let artifact = PathBuf::from("target/search-quality").join(format!("{}-scorer-ablations.json", report.suite));
    let absolute = repo_root.join(&artifact);
    let parent = absolute.parent().ok_or_else(|| Error::Eval {
        message: format!("scorer ablation artifact has no parent: {}", absolute.display()),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        message: "could not create scorer ablation directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&absolute, format!("{json}\n")).map_err(|source| Error::Io {
        message: "could not write scorer ablation artifact",
        path: absolute,
        source,
    })?;
    Ok(artifact)
}

pub fn report(
    suite: impl Into<String>,
    scorer_version: impl Into<String>,
    review_policy_version: impl Into<String>,
    semantic_reranking: SearchSemanticRerankingSummary,
    semantic_obligation_yield: Vec<SearchSemanticObligationYield>,
    variants: Vec<ScorerAblationVariantReport>,
    children: Vec<ScorerAblationChildReport>,
) -> ScorerAblationReport {
    ScorerAblationReport {
        schema_version: SCORER_ABLATION_SCHEMA_VERSION,
        suite: suite.into(),
        scorer_version: scorer_version.into(),
        review_policy_version: review_policy_version.into(),
        semantic_reranking,
        semantic_obligation_yield,
        variants,
        children,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{SCORER_ABLATION_SCHEMA_VERSION, ScorerAblationVariantReport, report, write_default_artifact};
    use crate::eval::scoring::{CountMetric, EvaluationMetrics, RecallAtK, TimingMetrics};
    use crate::eval::stage_metrics::SearchStageMetrics;
    use lean_dup_search::SearchScoringVariant;

    #[test]
    fn writer_emits_stable_schema_and_variant_names() {
        let temp = TempDir::new().unwrap();
        let report = report(
            "unit",
            "lean-dup.symbolic-scorer.v2",
            "lean-dup.symbolic-review-policy.v2",
            lean_dup_search::SearchSemanticRerankingSummary::default(),
            Vec::new(),
            vec![ScorerAblationVariantReport {
                variant: SearchScoringVariant::AllFeatures,
                status: "ok".to_owned(),
                semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                semantic_obligation_yield: Vec::new(),
                metrics: Some(metrics()),
                reason: None,
            }],
            Vec::new(),
        );

        let artifact = write_default_artifact(temp.path(), &report).unwrap();
        let json = std::fs::read_to_string(temp.path().join(artifact)).unwrap();

        assert!(json.contains(SCORER_ABLATION_SCHEMA_VERSION));
        assert!(json.contains("all-features"));
        assert!(json.contains("lean-dup.semantic-reranking.v1"));
        assert!(!json.contains("sqlite"));
        assert!(!json.contains("posting"));
    }

    fn metrics() -> EvaluationMetrics {
        EvaluationMetrics {
            suite: "unit".to_owned(),
            recall: vec![RecallAtK {
                k: 10,
                found: 1,
                total: 1,
            }],
            shown_queue_precision: CountMetric { found: 1, total: 1 },
            hard_negative_hits: CountMetric { found: 0, total: 0 },
            visible_groups: CountMetric { found: 1, total: 1 },
            probe_unavailable: CountMetric { found: 0, total: 0 },
            stage_metrics: SearchStageMetrics::default(),
            candidate_count: 1,
            timings: TimingMetrics::default(),
            peak_memory_bytes: None,
        }
    }
}
