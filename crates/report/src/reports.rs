use std::path::{Path, PathBuf};

use lean_dup_diagnostics::perf::{PerfEvent, PerfSummary};
use lean_dup_eval::EvalOutput;
use lean_dup_index::{CacheCleanupReport, CacheDiagnostics, CacheStatus, ComparisonEvidenceMode};
use lean_dup_search::{
    AuditEvidence, AuditGroup, AuditMember, AuditOutput, AuditProbeSummary, AuditReplacementHint, AuditReview,
    AuditVisibilityOptions, DiffOutput, SearchBaselineDiff, SearchBaselineGroup, SearchScoringSummary,
    SearchSemanticObligationFact, SearchSemanticObligationYield, SearchSemanticRerankingSummary, ShowOutput,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::report_contract::{AuditExplanations, GroupExplanation};

#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Report {
    Doctor(DoctorReport),
    CacheCleanup(CacheCleanupReportDto),
    Index(IndexReport),
    IndexMathlib(IndexReport),
    Audit(Box<AuditReport>),
    Eval(Box<EvalReportDto>),
    Perf(PerfReport),
    Show(Box<ShowReport>),
    Diff(DiffReport),
    Baseline(BaselineReport),
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineReport {
    pub status: &'static str,
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub cache_root: PathBuf,
    pub action: &'static str,
    /// Populated by `list` and `show`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub baselines: Vec<BaselineSummaryReport>,
    /// Populated by `delete` (the removed baseline name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineSummaryReport {
    pub name: String,
    #[serde(serialize_with = "serialize_path_ref")]
    pub path: PathBuf,
    pub workspace_fingerprint: String,
    pub group_count: usize,
    pub disk_bytes: u64,
    /// Populated by `show` only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalReportDto {
    pub report_schema_version: &'static str,
    pub status: String,
    pub suite: String,
    pub scorer_version: String,
    pub review_policy_version: String,
    pub metrics: EvalMetricsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_prerequisites: Option<ManualSuitePrerequisitesDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_resolution: Option<LabelResolutionReportDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_dataset_artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_ablation_artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<EvalRunReportDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalRunReportDto {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_policy_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvalMetricsDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub manual: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_prerequisites: Option<ManualSuitePrerequisitesDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_resolution: Option<LabelResolutionReportDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelResolutionReportDto {
    pub status: lean_dup_eval::LabelResolutionStatus,
    pub positives: LabelTraceCountDto,
    pub hard_negatives: LabelTraceCountDto,
    pub blockers: Vec<String>,
    pub traces: Vec<LabelTraceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelTraceCountDto {
    pub resolved: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelTraceDto {
    pub left: String,
    pub right: String,
    pub polarity: lean_dup_eval::LabelPolarity,
    pub match_class: lean_dup_eval::MatchClass,
    pub left_resolution: LabelEndpointResolutionDto,
    pub right_resolution: LabelEndpointResolutionDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_pair: Option<LabelPairDto>,
    pub generated: bool,
    pub ranked: bool,
    pub rank: Option<usize>,
    pub visible: bool,
    pub lost_layer: lean_dup_eval::LabelLossLayer,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelPairDto {
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelEndpointResolutionDto {
    pub requested: String,
    pub status: lean_dup_eval::LabelEndpointStatus,
    pub candidates: Vec<LabelResolutionCandidateDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelResolutionCandidateDto {
    pub qualified_name: String,
    pub origin: String,
    pub kind: String,
    pub visibility: String,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManualSuitePrerequisitesDto {
    pub suite: String,
    pub workspace_path: Option<PathBuf>,
    pub module_selector: String,
    pub workspace: PrerequisiteCheckDto,
    pub labels: PrerequisiteCheckDto,
    pub compiled_oleans: PrerequisiteCheckDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mathlib: Option<ManualMathlibPrerequisitesDto>,
    pub next_command: String,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManualMathlibPrerequisitesDto {
    pub source_workspace: PrerequisiteCheckDto,
    pub compiled_oleans: PrerequisiteCheckDto,
    pub external_comparison_artifacts: PrerequisiteCheckDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrerequisiteCheckDto {
    pub status: lean_dup_eval::PrerequisiteStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvalMetricsDto {
    pub suite: String,
    pub recall: Vec<EvalRecallAtKDto>,
    pub shown_queue_precision: EvalCountMetricDto,
    pub hard_negative_hits: EvalCountMetricDto,
    pub visible_groups: EvalCountMetricDto,
    pub probe_unavailable: EvalCountMetricDto,
    pub stage_metrics: EvalStageMetricsDto,
    pub candidate_count: usize,
    pub timings: EvalTimingMetricsDto,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvalRecallAtKDto {
    pub k: usize,
    pub found: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalCountMetricDto {
    pub found: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalTimingMetricsDto {
    pub index_load_ms: u128,
    pub retrieval_ms: u128,
    pub probe_ms: u128,
    pub total_ms: u128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalStageMetricsDto {
    pub candidate_generation_recall: EvalCountMetricDto,
    pub candidate_source_recall: EvalCandidateSourceRecallDto,
    pub candidate_stage_recall: EvalCandidateStageSurvivalDto,
    pub top_k_recall_before_final_ranking: Vec<EvalRecallAtKDto>,
    pub ranked_recall: Vec<EvalRecallAtKDto>,
    pub visible_queue_precision: EvalCountMetricDto,
    pub hard_negative_survival: EvalHardNegativeSurvivalDto,
    pub hard_negative_stage_survival: EvalCandidateStageSurvivalDto,
    pub candidate_count_by_origin: std::collections::BTreeMap<String, usize>,
    pub candidate_count_by_feature_family: std::collections::BTreeMap<String, usize>,
    pub generated_candidate_count_by_source_family: std::collections::BTreeMap<String, usize>,
    pub generated_candidate_count_by_source_id: std::collections::BTreeMap<String, usize>,
    pub generated_candidate_count_by_policy: std::collections::BTreeMap<String, usize>,
    pub generated_candidate_count_by_feature_family: std::collections::BTreeMap<String, usize>,
    pub hard_negative_generated_by_feature_family: std::collections::BTreeMap<String, usize>,
    pub candidate_loss_metrics: EvalCandidateLossMetricsDto,
    pub semantic_verification: EvalSemanticVerificationStageMetricsDto,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalCandidateStageSurvivalDto {
    pub symbolic_generated: EvalCountMetricDto,
    pub merged_generated: EvalCountMetricDto,
    pub ranked: EvalCountMetricDto,
    pub visible: EvalCountMetricDto,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalCandidateSourceRecallDto {
    pub symbolic_only: EvalCountMetricDto,
    pub semantic_lane_only: EvalCountMetricDto,
    pub merged: EvalCountMetricDto,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalCandidateLossMetricsDto {
    pub positive_fanout_pruned: EvalCountMetricDto,
    pub hard_negative_fanout_pruned: EvalCountMetricDto,
    pub positive_top_k_dropped: EvalCountMetricDto,
    pub hard_negative_top_k_dropped: EvalCountMetricDto,
    pub positive_fanout_pruned_by_feature_family: std::collections::BTreeMap<String, usize>,
    pub hard_negative_fanout_pruned_by_feature_family: std::collections::BTreeMap<String, usize>,
    pub positive_top_k_dropped_by_feature_family: std::collections::BTreeMap<String, usize>,
    pub hard_negative_top_k_dropped_by_feature_family: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalHardNegativeSurvivalDto {
    pub candidate_generation: EvalCountMetricDto,
    pub top_k: Vec<EvalCountAtKDto>,
    pub visible_queue: EvalCountMetricDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvalCountAtKDto {
    pub k: usize,
    pub found: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvalSemanticVerificationStageMetricsDto {
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub planned: usize,
    pub cached: usize,
    pub worker: usize,
    pub unavailable: usize,
    pub obligation_yield: Vec<SearchSemanticObligationYield>,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub report_schema_version: &'static str,
    pub release: ReleaseIdentityReport,
    pub status: &'static str,
    #[serde(serialize_with = "serialize_path_ref")]
    pub requested_workspace: PathBuf,
    #[serde(serialize_with = "serialize_path_ref")]
    pub lake_root: PathBuf,
    #[serde(serialize_with = "serialize_path_ref")]
    pub lakefile: PathBuf,
    pub module_roots: Vec<String>,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub cache: CacheDiagnosticsReport,
    pub worker: WorkerDiagnosticsReport,
    pub lean_version: String,
    pub require_oleans: bool,
    pub missing_oleans: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseIdentityReport {
    pub package: String,
    pub version: String,
    pub git_revision: String,
    pub build_profile: String,
    pub report_schema_version: String,
    pub index_schema_version: String,
    pub cache_key_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerDiagnosticsReport {
    pub protocol_version: String,
    pub worker_version: String,
    pub lean_version: String,
    pub extract_version: String,
    pub features_version: String,
    pub probe_version: String,
    pub supported_commands: Vec<String>,
    pub supported_capabilities: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheDiagnosticsReport {
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub cache_root: PathBuf,
    pub total_disk_bytes: u64,
    pub labels: Vec<CacheLabelDiagnosticsReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheLabelDiagnosticsReport {
    pub label: String,
    #[serde(serialize_with = "serialize_path_ref")]
    pub label_dir: PathBuf,
    pub disk_bytes: u64,
    pub latest: CacheLatestDiagnosticsReport,
    pub entries: Vec<CacheEntryDiagnosticsReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheLatestDiagnosticsReport {
    #[serde(serialize_with = "serialize_path_ref")]
    pub pointer_path: PathBuf,
    pub status: String,
    #[serde(serialize_with = "serialize_option_path_ref")]
    pub index_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheEntryDiagnosticsReport {
    #[serde(serialize_with = "serialize_path_ref")]
    pub index_dir: PathBuf,
    #[serde(serialize_with = "serialize_path_ref")]
    pub index_path: PathBuf,
    pub status: String,
    pub active_latest: bool,
    pub expected_current: bool,
    pub schema_version: Option<String>,
    pub provenance_kind: String,
    pub declaration_count: Option<usize>,
    pub disk_bytes: u64,
    pub reasons: Vec<String>,
}

impl CacheEntryDiagnosticsReport {
    /// True if `cache-cleanup` would remove this entry. Mirrors the predicate
    /// in `lean_dup_index::cleanup_cache`: an entry is protected iff it is
    /// pointed to by the latest pointer OR is the currently-expected index;
    /// otherwise it is reclaimable.
    pub fn is_reclaimable(&self) -> bool {
        !self.active_latest && !self.expected_current
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleanupReportDto {
    pub status: &'static str,
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub cache_root: PathBuf,
    pub executed: bool,
    pub removable_count: usize,
    pub protected_count: usize,
    pub bytes_to_remove: u64,
    pub bytes_removed: u64,
    pub removed_entries: Vec<CacheCleanupEntryReport>,
    pub protected_entries: Vec<CacheCleanupEntryReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleanupEntryReport {
    pub label: String,
    #[serde(serialize_with = "serialize_path_ref")]
    pub index_dir: PathBuf,
    pub disk_bytes: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathReferenceReport {
    pub kind: &'static str,
    pub fingerprint: String,
}

/// Stable path diagnostic for release-facing report output.
///
/// Reports expose the role and digest of local paths, not filesystem layout.
/// Index, project, and worker crates keep owning the concrete paths they need
/// to build, open, or invalidate caches.
pub fn path_reference(path: &Path) -> PathReferenceReport {
    PathReferenceReport {
        kind: path_kind(path),
        fingerprint: path_fingerprint(path),
    }
}

pub(crate) fn path_diagnostic_label(path: &Path) -> String {
    let reference = path_reference(path);
    path_reference_label(&reference)
}

pub(crate) fn cache_root_diagnostic_label(path: &Path) -> String {
    format!("cache-root {}", path_fingerprint(path))
}

pub(crate) fn path_reference_label(reference: &PathReferenceReport) -> String {
    format!("{} {}", reference.kind, reference.fingerprint)
}

fn serialize_path_ref<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    path_reference(path).serialize(serializer)
}

fn serialize_cache_root_ref<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    PathReferenceReport {
        kind: "cache-root",
        fingerprint: path_fingerprint(path),
    }
    .serialize(serializer)
}

fn serialize_option_path_ref<S>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    path.as_deref().map(path_reference).serialize(serializer)
}

fn path_kind(path: &Path) -> &'static str {
    let path_text = path.to_string_lossy();
    let filename = path.file_name().and_then(|name| name.to_str());
    if filename == Some("lakefile.toml") || filename == Some("lakefile.lean") {
        "lake-config"
    } else if filename == Some("index.sqlite") {
        "cache-store"
    } else if filename == Some("latest.json") {
        "cache-pointer"
    } else if path_text.contains("/indexes/") || path_text.contains("\\indexes\\") {
        "cache-entry"
    } else if path_text.contains("cache") || path_text.contains(".cache") {
        "cache-root"
    } else {
        "workspace-root"
    }
}

fn path_fingerprint(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let mut short = String::with_capacity(24);
    for byte in digest.iter().take(12) {
        short.push_str(&format!("{byte:02x}"));
    }
    format!("sha256:{short}")
}

#[derive(Debug, Serialize)]
pub struct AuditReport {
    pub report_schema_version: &'static str,
    pub status: &'static str,
    pub workspace: AuditWorkspaceReport,
    pub cache: AuditCacheReport,
    pub options: AuditOptionsReport,
    pub scoring: SearchScoringSummary,
    pub review_policy: lean_dup_search::SearchReviewPolicySummary,
    pub queue_counts: ReviewQueueCounts,
    pub retrieval: RetrievalReport,
    pub comparison_provenance: Vec<ComparisonProvenanceReportDto>,
    pub semantic_verification: SemanticVerificationReport,
    pub explanations: AuditExplanations,
    pub review: ReviewReport,
    pub visible_groups: Vec<ReviewGroupReport>,
    pub visible_group_count: usize,
    pub visible_groups_emitted: usize,
    pub visible_group_limit: usize,
    pub visible_groups_truncated: bool,
    pub saved_baseline: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_baseline_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_baseline_group_count: Option<usize>,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AuditWorkspaceReport {
    #[serde(serialize_with = "serialize_path_ref")]
    pub requested_workspace: PathBuf,
    #[serde(serialize_with = "serialize_path_ref")]
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
}

#[derive(Debug, Serialize)]
pub struct AuditCacheReport {
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub root: PathBuf,
    pub fingerprint: String,
}

#[derive(Debug, Serialize)]
pub struct AuditOptionsReport {
    pub include_private: bool,
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub include_generated: bool,
    pub visibility: AuditVisibilityOptions,
}

#[derive(Debug, Serialize)]
pub struct ReviewQueueCounts {
    pub cleanup: usize,
    pub with_private: usize,
    pub with_low_priority: usize,
    pub diagnostics: usize,
}

#[derive(Debug, Serialize)]
pub struct ShowReport {
    pub status: &'static str,
    #[serde(serialize_with = "serialize_path_ref")]
    pub requested_workspace: PathBuf,
    #[serde(serialize_with = "serialize_path_ref")]
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub group: ReviewGroupReport,
    pub explanation: GroupExplanation,
}

#[derive(Debug, Serialize)]
pub struct DiffReport {
    pub status: &'static str,
    #[serde(serialize_with = "serialize_path_ref")]
    pub requested_workspace: PathBuf,
    #[serde(serialize_with = "serialize_path_ref")]
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    #[serde(serialize_with = "serialize_cache_root_ref")]
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub diff: BaselineDiffReport,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalReport {
    pub fanout_policy_id: String,
    pub candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_feature_fanouts: usize,
    pub heap_truncations: usize,
    pub top_k_saturation_by_source_id: std::collections::BTreeMap<String, usize>,
    pub pruned_feature_fanout_by_family: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonProvenanceReportDto {
    pub label: Option<String>,
    pub origin: String,
    pub evidence_mode: String,
    pub declaration_count: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticVerificationReport {
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub enabled: bool,
    pub policy: String,
    pub budget: usize,
    pub per_declaration_cap: usize,
    pub chunk_size: usize,
    pub candidates_considered: usize,
    pub planned_pairs: usize,
    pub skipped_by_policy: usize,
    pub skipped_by_budget: usize,
    pub cheap_summary_rejects: usize,
    pub planned_exact_theorem: usize,
    pub planned_permuted_theorem: usize,
    pub planned_replacement: usize,
    pub planned_reducible_definition: usize,
    pub planned_specialization: usize,
    pub planned_local_duplicate: usize,
    pub cached_hits: usize,
    pub worker_pairs: usize,
    pub worker_batches: usize,
    pub recovered_failures: usize,
    pub unavailable_results: usize,
    pub unavailable_unsupported: usize,
    pub unavailable_missing: usize,
    pub unavailable_timeout: usize,
    pub unavailable_internal: usize,
    pub unavailable_by_reason: std::collections::BTreeMap<String, usize>,
    pub unavailable_by_obligation: std::collections::BTreeMap<String, usize>,
    pub unavailable_by_module: std::collections::BTreeMap<String, usize>,
    pub unavailable_by_origin: std::collections::BTreeMap<String, usize>,
    pub status_by_source: std::collections::BTreeMap<String, lean_dup_search::ProbeStatusBreakdown>,
    pub status_by_match_class: std::collections::BTreeMap<String, lean_dup_search::ProbeStatusBreakdown>,
    pub verified_results: usize,
    pub rejected_results: usize,
    pub obligation_yield: Vec<SearchSemanticObligationYield>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewReport {
    pub group_count: usize,
    pub suppressed_count: usize,
    pub diagnostics: ReviewDiagnosticsReport,
    pub candidate_pairs: usize,
    pub emitted_groups: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewDiagnosticsReport {
    pub candidate_pairs: usize,
    pub emitted_groups: usize,
    pub suppressed_groups: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewGroupReport {
    pub family_id: String,
    pub id: String,
    pub pair_id: String,
    pub pair_count: usize,
    pub pair_ids: Vec<String>,
    pub pair_evidence: Vec<ReviewPairEvidenceReport>,
    pub pair_evidence_truncated: bool,
    pub relation: String,
    pub members: Vec<ReviewMemberReport>,
    pub evidence: Vec<ReviewEvidenceReport>,
    pub signals: Vec<String>,
    pub blockers: Vec<String>,
    pub confidence: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub target_decl: Option<String>,
    pub target_module: Option<String>,
    pub evidence_mode: String,
    pub probe_summary: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub semantic_obligations: Vec<SearchSemanticObligationFact>,
    pub local_caller_count: usize,
    pub replacement_hint: Option<ReplacementHintReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewPairEvidenceReport {
    pub id: String,
    pub pair_id: String,
    pub relation: String,
    pub members: Vec<ReviewMemberReport>,
    pub evidence: Vec<ReviewEvidenceReport>,
    pub signals: Vec<String>,
    pub blockers: Vec<String>,
    pub confidence: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub evidence_mode: String,
    pub probe_summary: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub semantic_obligations: Vec<SearchSemanticObligationFact>,
    pub local_caller_count: usize,
    pub replacement_hint: Option<ReplacementHintReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewMemberReport {
    pub declaration_id: String,
    pub origin: String,
    pub module: String,
    pub qualified_name: String,
    pub display_name: String,
    pub kind: String,
    pub visibility: String,
    pub source_span: Option<SourceSpanReport>,
    pub status_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceSpanReport {
    pub file: PathReferenceReport,
    pub start: SourcePointReport,
    pub end: SourcePointReport,
    /// Resolved absolute path on the current machine. Populated only when the
    /// caller (currently `show`) has the workspace root in scope. Optional and
    /// additive on the JSON wire schema; consumers that want stable fingerprints
    /// keep using `file`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourcePointReport {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewEvidenceReport {
    pub kind: String,
    pub role: Option<String>,
    pub display: Option<String>,
    pub score: f64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplacementHintReport {
    pub target_decl: String,
    pub target_module: String,
    pub import_status: String,
    pub caller_impact: String,
    pub caller_count: usize,
    pub displayed_callers: Vec<SourceReferenceReport>,
    pub callers_truncated: bool,
    pub notes: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceReferenceReport {
    pub file: PathReferenceReport,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineDiffReport {
    pub baseline: String,
    #[serde(serialize_with = "serialize_path_ref")]
    pub baseline_path: PathBuf,
    pub appeared: Vec<BaselineGroupReport>,
    pub disappeared: Vec<BaselineGroupReport>,
    pub changed: Vec<BaselineChangeReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineGroupReport {
    pub id: String,
    pub relation: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub member_ids: Vec<String>,
    pub evidence_summary: Vec<String>,
    pub evidence_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineChangeReport {
    pub id: String,
    pub before: BaselineGroupReport,
    pub after: BaselineGroupReport,
}

pub fn eval_report(report: EvalOutput) -> EvalReportDto {
    EvalReportDto {
        report_schema_version: crate::report_contract::REPORT_SCHEMA_VERSION,
        status: report.status,
        suite: report.suite,
        scorer_version: report.scorer_version,
        review_policy_version: report.review_policy_version,
        metrics: eval_metrics_dto(report.metrics),
        artifact_path: None,
        manual_prerequisites: report.manual_prerequisites.map(manual_prerequisites_dto),
        label_resolution: report.label_resolution.map(label_resolution_dto),
        search_dataset_artifact: report.search_dataset_artifact,
        scorer_ablation_artifact: report.scorer_ablation_artifact,
        runs: report
            .runs
            .into_iter()
            .map(|run| EvalRunReportDto {
                suite: run.suite,
                status: run.status,
                scorer_version: run.scorer_version,
                review_policy_version: run.review_policy_version,
                metrics: run.metrics.map(eval_metrics_dto),
                reason: run.reason,
                manual: run.manual,
                manual_prerequisites: run.manual_prerequisites.map(manual_prerequisites_dto),
                label_resolution: run.label_resolution.map(label_resolution_dto),
            })
            .collect(),
    }
}

fn label_resolution_dto(report: lean_dup_eval::LabelResolutionReport) -> LabelResolutionReportDto {
    LabelResolutionReportDto {
        status: report.status,
        positives: label_trace_count_dto(report.positives),
        hard_negatives: label_trace_count_dto(report.hard_negatives),
        blockers: report.blockers,
        traces: report.traces.into_iter().map(label_trace_dto).collect(),
    }
}

fn label_trace_count_dto(count: lean_dup_eval::LabelTraceCount) -> LabelTraceCountDto {
    LabelTraceCountDto {
        resolved: count.resolved,
        total: count.total,
    }
}

fn label_trace_dto(trace: lean_dup_eval::LabelTrace) -> LabelTraceDto {
    LabelTraceDto {
        left: trace.left,
        right: trace.right,
        polarity: trace.polarity,
        match_class: trace.match_class,
        left_resolution: label_endpoint_resolution_dto(trace.left_resolution),
        right_resolution: label_endpoint_resolution_dto(trace.right_resolution),
        canonical_pair: trace.canonical_pair.map(|pair| LabelPairDto {
            left: pair.left,
            right: pair.right,
        }),
        generated: trace.generated,
        ranked: trace.ranked,
        rank: trace.rank,
        visible: trace.visible,
        lost_layer: trace.lost_layer,
        reason: trace.reason,
    }
}

fn label_endpoint_resolution_dto(resolution: lean_dup_eval::LabelEndpointResolution) -> LabelEndpointResolutionDto {
    LabelEndpointResolutionDto {
        requested: resolution.requested,
        status: resolution.status,
        candidates: resolution
            .candidates
            .into_iter()
            .map(|candidate| LabelResolutionCandidateDto {
                qualified_name: candidate.qualified_name,
                origin: candidate.origin,
                kind: candidate.kind,
                visibility: candidate.visibility,
                skipped: candidate.skipped,
            })
            .collect(),
    }
}

fn manual_prerequisites_dto(prerequisites: lean_dup_eval::ManualSuitePrerequisites) -> ManualSuitePrerequisitesDto {
    ManualSuitePrerequisitesDto {
        suite: prerequisites.suite,
        workspace_path: prerequisites.workspace_path,
        module_selector: prerequisites.module_selector,
        workspace: prerequisite_check_dto(prerequisites.workspace),
        labels: prerequisite_check_dto(prerequisites.labels),
        compiled_oleans: prerequisite_check_dto(prerequisites.compiled_oleans),
        mathlib: prerequisites.mathlib.map(|mathlib| ManualMathlibPrerequisitesDto {
            source_workspace: prerequisite_check_dto(mathlib.source_workspace),
            compiled_oleans: prerequisite_check_dto(mathlib.compiled_oleans),
            external_comparison_artifacts: prerequisite_check_dto(mathlib.external_comparison_artifacts),
        }),
        next_command: prerequisites.next_command,
        blockers: prerequisites.blockers,
    }
}

fn prerequisite_check_dto(check: lean_dup_eval::PrerequisiteCheck) -> PrerequisiteCheckDto {
    PrerequisiteCheckDto {
        status: check.status,
        detail: check.detail,
    }
}

fn eval_metrics_dto(metrics: lean_dup_eval::EvaluationMetrics) -> EvalMetricsDto {
    EvalMetricsDto {
        suite: metrics.suite,
        recall: metrics
            .recall
            .into_iter()
            .map(|recall| EvalRecallAtKDto {
                k: recall.k,
                found: recall.found,
                total: recall.total,
            })
            .collect(),
        shown_queue_precision: eval_count_metric_dto(metrics.shown_queue_precision),
        hard_negative_hits: eval_count_metric_dto(metrics.hard_negative_hits),
        visible_groups: eval_count_metric_dto(metrics.visible_groups),
        probe_unavailable: eval_count_metric_dto(metrics.probe_unavailable),
        stage_metrics: EvalStageMetricsDto {
            candidate_generation_recall: eval_count_metric_dto(metrics.stage_metrics.candidate_generation_recall),
            candidate_source_recall: EvalCandidateSourceRecallDto {
                symbolic_only: eval_count_metric_dto(metrics.stage_metrics.candidate_source_recall.symbolic_only),
                semantic_lane_only: eval_count_metric_dto(
                    metrics.stage_metrics.candidate_source_recall.semantic_lane_only,
                ),
                merged: eval_count_metric_dto(metrics.stage_metrics.candidate_source_recall.merged),
            },
            candidate_stage_recall: EvalCandidateStageSurvivalDto {
                symbolic_generated: eval_count_metric_dto(
                    metrics.stage_metrics.candidate_stage_recall.symbolic_generated,
                ),
                merged_generated: eval_count_metric_dto(metrics.stage_metrics.candidate_stage_recall.merged_generated),
                ranked: eval_count_metric_dto(metrics.stage_metrics.candidate_stage_recall.ranked),
                visible: eval_count_metric_dto(metrics.stage_metrics.candidate_stage_recall.visible),
            },
            top_k_recall_before_final_ranking: metrics
                .stage_metrics
                .top_k_recall_before_final_ranking
                .into_iter()
                .map(|recall| EvalRecallAtKDto {
                    k: recall.k,
                    found: recall.found,
                    total: recall.total,
                })
                .collect(),
            ranked_recall: metrics
                .stage_metrics
                .ranked_recall
                .into_iter()
                .map(|recall| EvalRecallAtKDto {
                    k: recall.k,
                    found: recall.found,
                    total: recall.total,
                })
                .collect(),
            visible_queue_precision: eval_count_metric_dto(metrics.stage_metrics.visible_queue_precision),
            hard_negative_survival: EvalHardNegativeSurvivalDto {
                candidate_generation: eval_count_metric_dto(
                    metrics.stage_metrics.hard_negative_survival.candidate_generation,
                ),
                top_k: metrics
                    .stage_metrics
                    .hard_negative_survival
                    .top_k
                    .into_iter()
                    .map(|count| EvalCountAtKDto {
                        k: count.k,
                        found: count.found,
                        total: count.total,
                    })
                    .collect(),
                visible_queue: eval_count_metric_dto(metrics.stage_metrics.hard_negative_survival.visible_queue),
            },
            hard_negative_stage_survival: EvalCandidateStageSurvivalDto {
                symbolic_generated: eval_count_metric_dto(
                    metrics.stage_metrics.hard_negative_stage_survival.symbolic_generated,
                ),
                merged_generated: eval_count_metric_dto(
                    metrics.stage_metrics.hard_negative_stage_survival.merged_generated,
                ),
                ranked: eval_count_metric_dto(metrics.stage_metrics.hard_negative_stage_survival.ranked),
                visible: eval_count_metric_dto(metrics.stage_metrics.hard_negative_stage_survival.visible),
            },
            candidate_count_by_origin: metrics.stage_metrics.candidate_count_by_origin,
            candidate_count_by_feature_family: metrics.stage_metrics.candidate_count_by_feature_family,
            generated_candidate_count_by_source_family: metrics
                .stage_metrics
                .generated_candidate_count_by_source_family,
            generated_candidate_count_by_source_id: metrics.stage_metrics.generated_candidate_count_by_source_id,
            generated_candidate_count_by_policy: metrics.stage_metrics.generated_candidate_count_by_policy,
            generated_candidate_count_by_feature_family: metrics
                .stage_metrics
                .generated_candidate_count_by_feature_family,
            hard_negative_generated_by_feature_family: metrics.stage_metrics.hard_negative_generated_by_feature_family,
            candidate_loss_metrics: EvalCandidateLossMetricsDto {
                positive_fanout_pruned: eval_count_metric_dto(
                    metrics.stage_metrics.candidate_loss_metrics.positive_fanout_pruned,
                ),
                hard_negative_fanout_pruned: eval_count_metric_dto(
                    metrics.stage_metrics.candidate_loss_metrics.hard_negative_fanout_pruned,
                ),
                positive_top_k_dropped: eval_count_metric_dto(
                    metrics.stage_metrics.candidate_loss_metrics.positive_top_k_dropped,
                ),
                hard_negative_top_k_dropped: eval_count_metric_dto(
                    metrics.stage_metrics.candidate_loss_metrics.hard_negative_top_k_dropped,
                ),
                positive_fanout_pruned_by_feature_family: metrics
                    .stage_metrics
                    .candidate_loss_metrics
                    .positive_fanout_pruned_by_feature_family,
                hard_negative_fanout_pruned_by_feature_family: metrics
                    .stage_metrics
                    .candidate_loss_metrics
                    .hard_negative_fanout_pruned_by_feature_family,
                positive_top_k_dropped_by_feature_family: metrics
                    .stage_metrics
                    .candidate_loss_metrics
                    .positive_top_k_dropped_by_feature_family,
                hard_negative_top_k_dropped_by_feature_family: metrics
                    .stage_metrics
                    .candidate_loss_metrics
                    .hard_negative_top_k_dropped_by_feature_family,
            },
            semantic_verification: EvalSemanticVerificationStageMetricsDto {
                semantic_reranking: metrics.stage_metrics.semantic_verification.semantic_reranking.clone(),
                planned: metrics.stage_metrics.semantic_verification.planned,
                cached: metrics.stage_metrics.semantic_verification.cached,
                worker: metrics.stage_metrics.semantic_verification.worker,
                unavailable: metrics.stage_metrics.semantic_verification.unavailable,
                obligation_yield: metrics.stage_metrics.semantic_verification.obligation_yield.clone(),
            },
        },
        candidate_count: metrics.candidate_count,
        timings: EvalTimingMetricsDto {
            index_load_ms: metrics.timings.index_load_ms,
            retrieval_ms: metrics.timings.retrieval_ms,
            probe_ms: metrics.timings.probe_ms,
            total_ms: metrics.timings.total_ms,
        },
        peak_memory_bytes: metrics.peak_memory_bytes,
    }
}

fn eval_count_metric_dto(metric: lean_dup_eval::CountMetric) -> EvalCountMetricDto {
    EvalCountMetricDto {
        found: metric.found,
        total: metric.total,
    }
}

pub fn audit_report(output: AuditOutput) -> AuditReport {
    let visible_group_count = output.visible_group_count;
    let visible_groups_emitted = output.visible_groups.len();
    let queue_counts = ReviewQueueCounts {
        cleanup: output.queue_counts.cleanup,
        with_private: output.queue_counts.with_private,
        with_low_priority: output.queue_counts.with_low_priority,
        diagnostics: output.queue_counts.diagnostics,
    };
    let explanations = crate::report_contract::explain_audit(
        &output.review,
        &output.visible_groups,
        &output.queue_summary,
        &output.semantic_verification,
        &output.comparison_provenance,
    );
    let retrieval = RetrievalReport {
        fanout_policy_id: output.retrieval.fanout_policy_id.clone(),
        candidate_count: output.retrieval.candidate_count,
        hydrated_external_count: output.retrieval.hydrated_external_count,
        pruned_feature_fanouts: output.retrieval.pruned_feature_fanout_count,
        heap_truncations: output.retrieval.heap_truncations,
        top_k_saturation_by_source_id: output.retrieval.top_k_saturation_by_source_id.clone(),
        pruned_feature_fanout_by_family: output.retrieval.pruned_feature_fanout_by_family.clone(),
    };
    let comparison_provenance = output
        .comparison_provenance
        .iter()
        .map(comparison_provenance_report)
        .collect();
    let semantic_verification = semantic_verification_report(&output.semantic_verification);
    let review = review_report(&output.review);
    let visible_groups = output.visible_groups.iter().map(group_report).collect();
    AuditReport {
        report_schema_version: crate::report_contract::REPORT_SCHEMA_VERSION,
        status: "ok",
        workspace: AuditWorkspaceReport {
            requested_workspace: output.requested_workspace,
            lake_root: output.lake_root,
            selected_roots: output.selected_roots,
            source_count: output.source_count,
        },
        cache: AuditCacheReport {
            root: output.cache_root,
            fingerprint: output.cache_fingerprint,
        },
        options: AuditOptionsReport {
            include_private: output.include_private,
            compare_indexes: output.compare_indexes,
            compare_mathlib: output.compare_mathlib,
            include_generated: output.include_generated,
            visibility: output.visibility,
        },
        scoring: output.scoring,
        review_policy: output.review_policy,
        queue_counts,
        retrieval,
        comparison_provenance,
        semantic_verification,
        explanations,
        review,
        visible_groups,
        visible_group_count,
        visible_groups_emitted,
        visible_group_limit: output.visible_group_limit,
        visible_groups_truncated: output.visible_groups_truncated,
        saved_baseline_group_count: output.saved_baseline.as_ref().map(|_| output.saved_baseline_group_count),
        saved_baseline: output.saved_baseline,
        saved_baseline_name: output.saved_baseline_name,
        message: "audit ranking queue generated",
    }
}

/// Serialize a unit-like serde enum (e.g. `CorruptPointer`) to its kebab-case
/// label (e.g. `corrupt-pointer`) by going through the enum's `Serialize` impl.
/// Avoids re-deriving on each enum and avoids `format!("{:?}", x)` which
/// collapses CamelCase into one lowercase word.
fn kebab_case_label<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .unwrap_or_default()
}

pub fn cache_diagnostics_report(diagnostics: CacheDiagnostics) -> CacheDiagnosticsReport {
    CacheDiagnosticsReport {
        cache_root: diagnostics.cache_root,
        total_disk_bytes: diagnostics.total_disk_bytes,
        labels: diagnostics
            .labels
            .into_iter()
            .map(|label| CacheLabelDiagnosticsReport {
                label: label.label,
                label_dir: label.label_dir,
                disk_bytes: label.disk_bytes,
                latest: CacheLatestDiagnosticsReport {
                    pointer_path: label.latest.pointer_path,
                    status: kebab_case_label(&label.latest.status),
                    index_dir: label.latest.index_dir,
                },
                entries: label
                    .entries
                    .into_iter()
                    .map(|entry| CacheEntryDiagnosticsReport {
                        index_dir: entry.index_dir,
                        index_path: entry.index_path,
                        status: kebab_case_label(&entry.status),
                        active_latest: entry.active_latest,
                        expected_current: entry.expected_current,
                        schema_version: entry
                            .schema_version
                            .as_deref()
                            .map(lean_dup_index::diagnostic_index_schema_version),
                        provenance_kind: kebab_case_label(&entry.provenance_kind),
                        declaration_count: entry.declaration_count,
                        disk_bytes: entry.disk_bytes,
                        reasons: entry.reasons,
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn cache_cleanup_report(report: CacheCleanupReport) -> CacheCleanupReportDto {
    CacheCleanupReportDto {
        status: report.status,
        cache_root: report.cache_root,
        executed: report.executed,
        removable_count: report.removable_count,
        protected_count: report.protected_count,
        bytes_to_remove: report.bytes_to_remove,
        bytes_removed: report.bytes_removed,
        removed_entries: report
            .removed_entries
            .into_iter()
            .map(cache_cleanup_entry_report)
            .collect(),
        protected_entries: report
            .protected_entries
            .into_iter()
            .map(cache_cleanup_entry_report)
            .collect(),
    }
}

fn cache_cleanup_entry_report(entry: lean_dup_index::CacheCleanupEntry) -> CacheCleanupEntryReport {
    CacheCleanupEntryReport {
        label: entry.label,
        index_dir: entry.index_dir,
        disk_bytes: entry.disk_bytes,
        reason: entry.reason,
    }
}

pub fn show_report(output: ShowOutput) -> ShowReport {
    let explanation = crate::report_contract::explain_group(&output.group);
    let group = group_report_with_lake(&output.group, Some(&output.audit.lake_root));
    ShowReport {
        status: "ok",
        requested_workspace: output.audit.requested_workspace,
        lake_root: output.audit.lake_root,
        selected_roots: output.audit.selected_roots,
        source_count: output.audit.source_count,
        cache_root: output.audit.cache_root,
        cache_fingerprint: output.audit.cache_fingerprint,
        group,
        explanation,
    }
}

pub fn diff_report(output: DiffOutput) -> DiffReport {
    DiffReport {
        status: "ok",
        requested_workspace: output.requested_workspace,
        lake_root: output.lake_root,
        selected_roots: output.selected_roots,
        source_count: output.source_count,
        cache_root: output.cache_root,
        cache_fingerprint: output.cache_fingerprint,
        diff: baseline_diff_report(&output.diff),
    }
}

fn comparison_provenance_report(report: &lean_dup_index::ComparisonProvenanceReport) -> ComparisonProvenanceReportDto {
    ComparisonProvenanceReportDto {
        label: report.label.clone(),
        origin: report.origin.clone(),
        evidence_mode: comparison_evidence_mode_label(report.evidence_mode).to_owned(),
        declaration_count: report.declaration_count,
        reason: report.reason.clone(),
    }
}

fn semantic_verification_report(report: &AuditProbeSummary) -> SemanticVerificationReport {
    SemanticVerificationReport {
        semantic_reranking: report.semantic_reranking.clone(),
        enabled: report.enabled,
        policy: report.policy.clone(),
        budget: report.budget,
        per_declaration_cap: report.per_declaration_cap,
        chunk_size: report.chunk_size,
        candidates_considered: report.candidates_considered,
        planned_pairs: report.planned_pairs,
        skipped_by_policy: report.skipped_by_policy,
        skipped_by_budget: report.skipped_by_budget,
        cheap_summary_rejects: report.cheap_summary_rejects,
        planned_exact_theorem: report.planned_exact_theorem,
        planned_permuted_theorem: report.planned_permuted_theorem,
        planned_replacement: report.planned_replacement,
        planned_reducible_definition: report.planned_reducible_definition,
        planned_specialization: report.planned_specialization,
        planned_local_duplicate: report.planned_local_duplicate,
        cached_hits: report.cached_hits,
        worker_pairs: report.worker_pairs,
        worker_batches: report.worker_batches,
        recovered_failures: report.recovered_failures,
        unavailable_results: report.unavailable_results,
        unavailable_unsupported: report.unavailable_unsupported,
        unavailable_missing: report.unavailable_missing,
        unavailable_timeout: report.unavailable_timeout,
        unavailable_internal: report.unavailable_internal,
        unavailable_by_reason: report.unavailable_by_reason.clone(),
        unavailable_by_obligation: report.unavailable_by_obligation.clone(),
        unavailable_by_module: report.unavailable_by_module.clone(),
        unavailable_by_origin: report.unavailable_by_origin.clone(),
        status_by_source: report.status_by_source.clone(),
        status_by_match_class: report.status_by_match_class.clone(),
        verified_results: report.verified_results,
        rejected_results: report.rejected_results,
        obligation_yield: report.obligation_yield.clone(),
    }
}

fn review_report(review: &AuditReview) -> ReviewReport {
    ReviewReport {
        group_count: review.group_count,
        suppressed_count: review.suppressed_count,
        diagnostics: ReviewDiagnosticsReport {
            candidate_pairs: review.diagnostics.candidate_pairs,
            emitted_groups: review.diagnostics.emitted_groups,
            suppressed_groups: review.diagnostics.suppressed_groups,
        },
        candidate_pairs: review.diagnostics.candidate_pairs,
        emitted_groups: review.diagnostics.emitted_groups,
    }
}

fn group_report(group: &AuditGroup) -> ReviewGroupReport {
    group_report_with_lake(group, None)
}

/// Project an `AuditGroup` to a wire-shaped report. When `lake_root` is
/// supplied, each member span gains a resolved `local_path`; otherwise it
/// stays `None`. Only `show` passes the lake root through — `audit` keeps
/// fingerprint-only paths so the JSON wire format stays portable.
fn group_report_with_lake(group: &AuditGroup, lake_root: Option<&Path>) -> ReviewGroupReport {
    ReviewGroupReport {
        family_id: group.family_id.clone(),
        id: group.id.clone(),
        pair_id: group.pair_id.clone(),
        pair_count: group.pair_count,
        pair_ids: group.pair_ids.clone(),
        pair_evidence: group
            .pair_evidence
            .iter()
            .map(|pair| pair_evidence_report(pair, lake_root))
            .collect(),
        pair_evidence_truncated: group.pair_evidence_truncated,
        relation: group.relation.clone(),
        members: group.members.iter().map(|m| member_report(m, lake_root)).collect(),
        evidence: group.evidence.iter().map(evidence_report).collect(),
        signals: group.signals.clone(),
        blockers: group.blockers.clone(),
        confidence: group.confidence.clone(),
        review_priority: group.review_priority.clone(),
        recommended_action: group.recommended_action.clone(),
        target_decl: group.target_decl.clone(),
        target_module: group.target_module.clone(),
        evidence_mode: group.evidence_mode.clone(),
        probe_summary: group.probe_summary.clone(),
        semantic_obligations: group.semantic_obligations.clone(),
        local_caller_count: group.local_caller_count,
        replacement_hint: group.replacement_hint.as_ref().map(replacement_hint_report),
    }
}

fn pair_evidence_report(
    pair: &lean_dup_search::AuditPairEvidence,
    lake_root: Option<&Path>,
) -> ReviewPairEvidenceReport {
    ReviewPairEvidenceReport {
        id: pair.id.clone(),
        pair_id: pair.pair_id.clone(),
        relation: pair.relation.clone(),
        members: pair.members.iter().map(|m| member_report(m, lake_root)).collect(),
        evidence: pair.evidence.iter().map(evidence_report).collect(),
        signals: pair.signals.clone(),
        blockers: pair.blockers.clone(),
        confidence: pair.confidence.clone(),
        review_priority: pair.review_priority.clone(),
        recommended_action: pair.recommended_action.clone(),
        evidence_mode: pair.evidence_mode.clone(),
        probe_summary: pair.probe_summary.clone(),
        semantic_obligations: pair.semantic_obligations.clone(),
        local_caller_count: pair.local_caller_count,
        replacement_hint: pair.replacement_hint.as_ref().map(replacement_hint_report),
    }
}

fn member_report(member: &AuditMember, lake_root: Option<&Path>) -> ReviewMemberReport {
    ReviewMemberReport {
        declaration_id: member.declaration_id.clone(),
        origin: member.origin.clone(),
        module: member.module.clone(),
        qualified_name: member.qualified_name.clone(),
        display_name: member.display_name.clone(),
        kind: member.kind.clone(),
        visibility: member.visibility.clone(),
        source_span: member.source_span.as_ref().map(|span| SourceSpanReport {
            file: path_reference(Path::new(&span.file)),
            start: SourcePointReport {
                line: span.start.line as usize,
                column: span.start.column as usize,
            },
            end: SourcePointReport {
                line: span.end.line as usize,
                column: span.end.column as usize,
            },
            local_path: lake_root.map(|root| root.join(&span.file).to_string_lossy().into_owned()),
        }),
        status_flags: member.status_flags.clone(),
    }
}

fn evidence_report(evidence: &AuditEvidence) -> ReviewEvidenceReport {
    ReviewEvidenceReport {
        kind: evidence.kind.clone(),
        role: evidence.role.clone(),
        display: evidence.display.clone(),
        score: evidence.score,
        summary: evidence.summary.clone(),
    }
}

fn replacement_hint_report(hint: &AuditReplacementHint) -> ReplacementHintReport {
    ReplacementHintReport {
        target_decl: hint.target_decl.clone(),
        target_module: hint.target_module.clone(),
        import_status: hint.import_status.clone(),
        caller_impact: hint.caller_impact.clone(),
        caller_count: hint.caller_count,
        displayed_callers: hint
            .displayed_callers
            .iter()
            .map(|caller| SourceReferenceReport {
                file: path_reference(&caller.file),
                line: caller.line,
                column: caller.column,
                text: caller.text.clone(),
            })
            .collect(),
        callers_truncated: hint.callers_truncated,
        notes: hint.notes.clone(),
        blockers: hint.blockers.clone(),
    }
}

fn baseline_diff_report(diff: &SearchBaselineDiff) -> BaselineDiffReport {
    BaselineDiffReport {
        baseline: diff.baseline.clone(),
        baseline_path: diff.baseline_path.clone(),
        appeared: diff.appeared.iter().map(baseline_group_report).collect(),
        disappeared: diff.disappeared.iter().map(baseline_group_report).collect(),
        changed: diff
            .changed
            .iter()
            .map(|change| BaselineChangeReport {
                id: change.id.clone(),
                before: baseline_group_report(&change.before),
                after: baseline_group_report(&change.after),
            })
            .collect(),
    }
}

fn baseline_group_report(group: &SearchBaselineGroup) -> BaselineGroupReport {
    BaselineGroupReport {
        id: group.id.clone(),
        relation: group.relation.clone(),
        review_priority: group.review_priority.clone(),
        recommended_action: group.recommended_action.clone(),
        member_ids: group.member_ids.clone(),
        evidence_summary: group.evidence_summary.clone(),
        evidence_digest: group.evidence_digest.clone(),
    }
}

fn comparison_evidence_mode_label(mode: ComparisonEvidenceMode) -> &'static str {
    match mode {
        ComparisonEvidenceMode::Static => "static",
        ComparisonEvidenceMode::SourceBackedNotImportable => "source-backed-not-importable",
        ComparisonEvidenceMode::ProofGrade => "proof-grade",
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
