use std::path::PathBuf;

use lean_dup_diagnostics::perf::{PerfEvent, PerfSummary};
use lean_dup_eval::EvaluationReport;
use lean_dup_index::CacheStatus;
use lean_dup_index::ComparisonProvenanceReport;
use lean_dup_index::{CacheCleanupReport, CacheDiagnostics};
use lean_dup_search::BaselineDiff;
use lean_dup_search::ProbeDiagnostics;
use lean_dup_search::RetrievalDiagnostics;
use lean_dup_search::ReviewProfile;
use lean_dup_search::audit::{AuditOutput, review_filter};
use lean_dup_search::{RankedGroup, RankedReview, ReviewPriority};
use serde::{Deserialize, Serialize};

use crate::report_contract::{AuditExplanations, GroupExplanation};
use crate::{Error, Result};

#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Report {
    Doctor(DoctorReport),
    CacheCleanup(CacheCleanupReport),
    Index(IndexReport),
    IndexMathlib(IndexReport),
    Audit(Box<AuditReport>),
    Eval(EvaluationReport),
    Perf(PerfReport),
    Show(ShowReport),
    Diff(DiffReport),
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub status: &'static str,
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub lakefile: PathBuf,
    pub module_roots: Vec<String>,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub cache: CacheDiagnostics,
    pub lean_version: String,
    pub require_oleans: bool,
    pub missing_oleans: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IndexReport {
    pub status: &'static str,
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub label: String,
    pub cache_status: CacheStatus,
    pub index_path: PathBuf,
    pub index_dir: PathBuf,
    pub declaration_count: usize,
    pub diagnostics: Vec<String>,
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct AuditReport {
    pub report_schema_version: &'static str,
    pub status: &'static str,
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub include_private: bool,
    pub include_imports: bool,
    pub import_roots: Vec<String>,
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub threshold: f64,
    pub include_generated: bool,
    pub show_noise: bool,
    pub min_priority: ReviewPriority,
    pub review_profile: ReviewProfile,
    pub profile_counts: ReviewProfileCounts,
    pub retrieval: RetrievalDiagnostics,
    pub comparison_provenance: Vec<ComparisonProvenanceReport>,
    pub semantic_verification: ProbeDiagnostics,
    pub explanations: AuditExplanations,
    pub review: RankedReview,
    pub visible_groups: Vec<RankedGroup>,
    pub visible_group_count: usize,
    pub saved_baseline: Option<PathBuf>,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReviewProfileCounts {
    pub mathlib: usize,
    pub internal: usize,
    pub api_design: usize,
    pub noise: usize,
}

#[derive(Debug, Serialize)]
pub struct ShowReport {
    pub status: &'static str,
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub group: RankedGroup,
    pub explanation: GroupExplanation,
}

#[derive(Debug, Serialize)]
pub struct DiffReport {
    pub status: &'static str,
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub diff: BaselineDiff,
}

pub fn audit_report(output: AuditOutput) -> AuditReport {
    let filter = review_filter(
        output.review_profile,
        output.include_generated,
        output.show_noise,
        output.min_priority,
    );
    let visible_groups = output
        .review
        .visible_groups(filter)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let visible_group_count = visible_groups.len();
    let profile_counts = profile_counts(&output.review);
    let explanations = crate::report_contract::explain_audit(
        &output.review,
        &visible_groups,
        filter,
        &output.semantic_verification,
        &output.comparison_provenance,
    );
    AuditReport {
        report_schema_version: crate::report_contract::REPORT_SCHEMA_VERSION,
        status: "ok",
        requested_workspace: output.requested_workspace,
        lake_root: output.lake_root,
        selected_roots: output.selected_roots,
        source_count: output.source_count,
        cache_root: output.cache_root,
        cache_fingerprint: output.cache_fingerprint,
        include_private: output.include_private,
        include_imports: output.include_imports,
        import_roots: output.import_roots,
        compare_indexes: output.compare_indexes,
        compare_mathlib: output.compare_mathlib,
        threshold: output.threshold,
        include_generated: output.include_generated,
        show_noise: output.show_noise,
        min_priority: output.min_priority,
        review_profile: output.review_profile,
        profile_counts,
        retrieval: output.retrieval,
        comparison_provenance: output.comparison_provenance,
        semantic_verification: output.semantic_verification,
        explanations,
        review: output.review,
        visible_groups,
        visible_group_count,
        saved_baseline: output.saved_baseline,
        message: "audit ranking queue generated",
    }
}

pub fn show_report(output: AuditOutput, requested_group: &str) -> Result<ShowReport> {
    let filter = review_filter(
        output.review_profile,
        output.include_generated,
        output.show_noise,
        output.min_priority,
    );
    let group = output
        .review
        .groups
        .iter()
        .find(|group| group.id == requested_group)
        .cloned()
        .ok_or_else(|| Error::Index {
            message: format!("unknown audit group: {requested_group}"),
        })?;
    let explanation = crate::report_contract::explain_group(&group, filter);
    Ok(ShowReport {
        status: "ok",
        requested_workspace: output.requested_workspace,
        lake_root: output.lake_root,
        selected_roots: output.selected_roots,
        source_count: output.source_count,
        cache_root: output.cache_root,
        cache_fingerprint: output.cache_fingerprint,
        group,
        explanation,
    })
}

pub fn diff_report(output: AuditOutput, baseline_name: String) -> Result<DiffReport> {
    let (baseline_path, saved) = lean_dup_search::load(&output.cache_root, &baseline_name)?;
    let current = lean_dup_search::snapshot(&output.review, output.cache_fingerprint.clone());
    let diff = lean_dup_search::diff(baseline_name, baseline_path, saved, current);
    Ok(DiffReport {
        status: "ok",
        requested_workspace: output.requested_workspace,
        lake_root: output.lake_root,
        selected_roots: output.selected_roots,
        source_count: output.source_count,
        cache_root: output.cache_root,
        cache_fingerprint: output.cache_fingerprint,
        diff,
    })
}

fn profile_counts(review: &RankedReview) -> ReviewProfileCounts {
    ReviewProfileCounts {
        mathlib: review
            .visible_groups(review_filter(ReviewProfile::Mathlib, false, false, ReviewPriority::Low))
            .len(),
        internal: review
            .visible_groups(review_filter(
                ReviewProfile::Internal,
                false,
                false,
                ReviewPriority::Low,
            ))
            .len(),
        api_design: review
            .visible_groups(review_filter(
                ReviewProfile::ApiDesign,
                false,
                false,
                ReviewPriority::Low,
            ))
            .len(),
        noise: review
            .visible_groups(review_filter(ReviewProfile::Noise, false, false, ReviewPriority::Low))
            .len(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfWorkloadReport {
    pub workload: String,
    pub command: Vec<String>,
    pub cache_state: String,
    pub exit_code: i32,
    pub elapsed_ms: u128,
    pub peak_memory_bytes: Option<u64>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub candidate_count: Option<u64>,
    pub hydrated_declarations: Option<u64>,
    pub review_groups: Option<u64>,
    pub visible_groups: Option<u64>,
    pub semantic_planned_pairs: Option<u64>,
    pub semantic_cached_hits: Option<u64>,
    pub semantic_worker_pairs: Option<u64>,
    pub semantic_unavailable_results: Option<u64>,
    pub probe_batches: Option<u64>,
    pub probe_pairs: Option<u64>,
    pub profile_timings_ms: std::collections::BTreeMap<String, u128>,
    pub events: Vec<PerfEvent>,
    pub summary: PerfSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfReport {
    pub status: &'static str,
    pub workload: String,
    pub cache_root: PathBuf,
    pub report: PerfWorkloadReport,
}
