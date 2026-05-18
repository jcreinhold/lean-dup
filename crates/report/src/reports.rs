use std::path::PathBuf;

use lean_dup_diagnostics::perf::{PerfEvent, PerfSummary};
use lean_dup_eval::EvaluationReport;
use lean_dup_index::{CacheCleanupReport, CacheDiagnostics, CacheStatus, ComparisonEvidenceMode};
use lean_dup_search::ReviewProfile;
use lean_dup_search::audit::{
    AuditOutput, ConfidenceTier, DiffOutput, ProbeDiagnostics, RankedGroup, RankedReview, ReplacementHint,
    ReviewAction, ReviewEvidence, ReviewEvidenceMode, ReviewMember, ReviewPriority, ReviewRelation, ShowOutput,
    review_filter,
};
use serde::{Deserialize, Serialize};

use crate::report_contract::{AuditExplanations, GroupExplanation};

#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Report {
    Doctor(DoctorReport),
    CacheCleanup(CacheCleanupReportDto),
    Index(IndexReport),
    IndexMathlib(IndexReport),
    Audit(Box<AuditReport>),
    Eval(EvaluationReport),
    Perf(PerfReport),
    Show(Box<ShowReport>),
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
    pub cache: CacheDiagnosticsReport,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheDiagnosticsReport {
    pub cache_root: PathBuf,
    pub total_disk_bytes: u64,
    pub labels: Vec<CacheLabelDiagnosticsReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheLabelDiagnosticsReport {
    pub label: String,
    pub label_dir: PathBuf,
    pub disk_bytes: u64,
    pub latest: CacheLatestDiagnosticsReport,
    pub entries: Vec<CacheEntryDiagnosticsReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheLatestDiagnosticsReport {
    pub pointer_path: PathBuf,
    pub status: String,
    pub index_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheEntryDiagnosticsReport {
    pub index_dir: PathBuf,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleanupReportDto {
    pub status: &'static str,
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
    pub index_dir: PathBuf,
    pub disk_bytes: u64,
    pub reason: String,
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
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub include_generated: bool,
    pub show_noise: bool,
    pub review_profile: ReviewProfile,
    pub profile_counts: ReviewProfileCounts,
    pub retrieval: RetrievalReport,
    pub comparison_provenance: Vec<ComparisonProvenanceReportDto>,
    pub semantic_verification: SemanticVerificationReport,
    pub explanations: AuditExplanations,
    pub review: ReviewReport,
    pub visible_groups: Vec<ReviewGroupReport>,
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
    pub group: ReviewGroupReport,
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
    pub diff: BaselineDiffReport,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalReport {
    pub candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_postings: usize,
    pub heap_truncations: usize,
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
    pub verified_results: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewReport {
    pub groups: Vec<ReviewGroupReport>,
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
    pub target_decl: Option<String>,
    pub target_module: Option<String>,
    pub evidence_mode: String,
    pub probe_summary: Option<String>,
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
    pub file: String,
    pub start: SourcePointReport,
    pub end: SourcePointReport,
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
    pub caller_count: usize,
    pub displayed_callers: Vec<SourceReferenceReport>,
    pub notes: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceReferenceReport {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineDiffReport {
    pub baseline: String,
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

pub fn audit_report(output: AuditOutput) -> AuditReport {
    let filter = review_filter(output.review_profile, output.include_generated, output.show_noise);
    let visible_ranked_groups = output
        .review
        .visible_groups(filter)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let visible_group_count = visible_ranked_groups.len();
    let profile_counts = profile_counts(&output.review);
    let explanations = crate::report_contract::explain_audit(
        &output.review,
        &visible_ranked_groups,
        filter,
        &output.semantic_verification,
        &output.comparison_provenance,
    );
    let retrieval = RetrievalReport {
        candidate_count: output.retrieval.candidate_count,
        hydrated_external_count: output.retrieval.hydrated_external_count,
        pruned_postings: output.retrieval.pruned_postings.len(),
        heap_truncations: output.retrieval.heap_truncations.len(),
    };
    let comparison_provenance = output
        .comparison_provenance
        .iter()
        .map(comparison_provenance_report)
        .collect();
    let semantic_verification = semantic_verification_report(&output.semantic_verification);
    let review = review_report(&output.review);
    let visible_groups = visible_ranked_groups.iter().map(group_report).collect();
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
        compare_indexes: output.compare_indexes,
        compare_mathlib: output.compare_mathlib,
        include_generated: output.include_generated,
        show_noise: output.show_noise,
        review_profile: output.review_profile,
        profile_counts,
        retrieval,
        comparison_provenance,
        semantic_verification,
        explanations,
        review,
        visible_groups,
        visible_group_count,
        saved_baseline: output.saved_baseline,
        message: "audit ranking queue generated",
    }
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
                    status: format!("{:?}", label.latest.status).to_ascii_lowercase(),
                    index_dir: label.latest.index_dir,
                },
                entries: label
                    .entries
                    .into_iter()
                    .map(|entry| CacheEntryDiagnosticsReport {
                        index_dir: entry.index_dir,
                        index_path: entry.index_path,
                        status: format!("{:?}", entry.status).to_ascii_lowercase(),
                        active_latest: entry.active_latest,
                        expected_current: entry.expected_current,
                        schema_version: entry.schema_version,
                        provenance_kind: format!("{:?}", entry.provenance_kind).to_ascii_lowercase(),
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
    let filter = review_filter(
        output.audit.review_profile,
        output.audit.include_generated,
        output.audit.show_noise,
    );
    let explanation = crate::report_contract::explain_group(&output.group, filter);
    let group = group_report(&output.group);
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

fn semantic_verification_report(report: &ProbeDiagnostics) -> SemanticVerificationReport {
    SemanticVerificationReport {
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
        verified_results: report.verified_results,
    }
}

fn review_report(review: &RankedReview) -> ReviewReport {
    ReviewReport {
        groups: review.groups.iter().map(group_report).collect(),
        suppressed_count: review.suppressed.len(),
        diagnostics: ReviewDiagnosticsReport {
            candidate_pairs: review.diagnostics.candidate_pairs,
            emitted_groups: review.diagnostics.emitted_groups,
            suppressed_groups: review.diagnostics.suppressed_groups,
        },
        candidate_pairs: review.diagnostics.candidate_pairs,
        emitted_groups: review.diagnostics.emitted_groups,
    }
}

fn group_report(group: &RankedGroup) -> ReviewGroupReport {
    ReviewGroupReport {
        id: group.id.clone(),
        pair_id: group.pair_id.clone(),
        relation: relation_label(group.relation).to_owned(),
        members: group.members.iter().map(member_report).collect(),
        evidence: group.evidence.iter().map(evidence_report).collect(),
        signals: group.signals.clone(),
        blockers: group.blockers.clone(),
        confidence: confidence_label(group.confidence).to_owned(),
        review_priority: priority_label(group.review_priority).to_owned(),
        recommended_action: action_label(group.recommended_action).to_owned(),
        target_decl: group.target_decl.clone(),
        target_module: group.target_module.clone(),
        evidence_mode: evidence_mode_label(group.evidence_mode).to_owned(),
        probe_summary: group.probe_summary.clone(),
        local_caller_count: group.local_caller_count,
        replacement_hint: group.replacement_hint.as_ref().map(replacement_hint_report),
    }
}

fn member_report(member: &ReviewMember) -> ReviewMemberReport {
    ReviewMemberReport {
        declaration_id: member.declaration_id.clone(),
        origin: member.origin.clone(),
        module: member.module.clone(),
        qualified_name: member.qualified_name.clone(),
        display_name: member.display_name.clone(),
        kind: member.kind.clone(),
        visibility: member.visibility.clone(),
        source_span: member.source_span.as_ref().map(|span| SourceSpanReport {
            file: span.file.clone(),
            start: SourcePointReport {
                line: span.start.line as usize,
                column: span.start.column as usize,
            },
            end: SourcePointReport {
                line: span.end.line as usize,
                column: span.end.column as usize,
            },
        }),
        status_flags: member.status_flags.clone(),
    }
}

fn evidence_report(evidence: &ReviewEvidence) -> ReviewEvidenceReport {
    ReviewEvidenceReport {
        kind: evidence.kind.clone(),
        role: evidence.role.clone(),
        display: evidence.display.clone(),
        score: evidence.score,
        summary: evidence.summary(),
    }
}

fn replacement_hint_report(hint: &ReplacementHint) -> ReplacementHintReport {
    ReplacementHintReport {
        target_decl: hint.target_decl.clone(),
        target_module: hint.target_module.clone(),
        import_status: format!("{:?}", hint.import_status).to_ascii_lowercase(),
        caller_count: hint.caller_count,
        displayed_callers: hint
            .displayed_callers
            .iter()
            .map(|caller| SourceReferenceReport {
                file: caller.file.clone(),
                line: caller.line,
                column: caller.column,
                text: caller.text.clone(),
            })
            .collect(),
        notes: hint.notes.clone(),
        blockers: hint.blockers.clone(),
    }
}

fn baseline_diff_report(diff: &lean_dup_search::audit::SearchBaselineDiff) -> BaselineDiffReport {
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

fn baseline_group_report(group: &lean_dup_search::audit::SearchBaselineGroup) -> BaselineGroupReport {
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

fn profile_counts(review: &RankedReview) -> ReviewProfileCounts {
    ReviewProfileCounts {
        mathlib: review
            .visible_groups(review_filter(ReviewProfile::Mathlib, false, false))
            .len(),
        internal: review
            .visible_groups(review_filter(ReviewProfile::Internal, false, false))
            .len(),
        api_design: review
            .visible_groups(review_filter(ReviewProfile::ApiDesign, false, false))
            .len(),
        noise: review
            .visible_groups(review_filter(ReviewProfile::Noise, false, false))
            .len(),
    }
}

fn comparison_evidence_mode_label(mode: ComparisonEvidenceMode) -> &'static str {
    match mode {
        ComparisonEvidenceMode::Static => "static",
        ComparisonEvidenceMode::SourceBackedNotImportable => "source-backed-not-importable",
        ComparisonEvidenceMode::ProofGrade => "proof-grade",
    }
}

fn evidence_mode_label(mode: ReviewEvidenceMode) -> &'static str {
    match mode {
        ReviewEvidenceMode::Static => "static",
        ReviewEvidenceMode::SourceBackedNotImportable => "source-backed-not-importable",
        ReviewEvidenceMode::ProofGrade => "proof-grade",
    }
}

fn relation_label(relation: ReviewRelation) -> &'static str {
    match relation {
        ReviewRelation::ExactStatement => "exact-statement",
        ReviewRelation::PermutedStatement => "permuted-statement",
        ReviewRelation::ConnectiveEquivalent => "connective-equivalent",
        ReviewRelation::Specialization => "specialization",
        ReviewRelation::SourceClone => "source-clone",
        ReviewRelation::SubsumptionCandidate => "subsumption-candidate",
        ReviewRelation::NearStatement => "near-statement",
    }
}

fn action_label(action: ReviewAction) -> &'static str {
    match action {
        ReviewAction::AlreadyInMathlib => "already-in-mathlib",
        ReviewAction::LocalAlias => "local-alias",
        ReviewAction::ReplaceLocalUses => "replace-local-uses",
        ReviewAction::MergeGeneralization => "merge-generalization",
        ReviewAction::SpecializationOf => "specialization-of",
        ReviewAction::ProbableSourceClone => "probable-source-clone",
        ReviewAction::ManualReview => "manual-review",
    }
}

fn priority_label(priority: ReviewPriority) -> &'static str {
    match priority {
        ReviewPriority::High => "high",
        ReviewPriority::Medium => "medium",
        ReviewPriority::Low => "low",
        ReviewPriority::Noise => "noise",
    }
}

fn confidence_label(confidence: ConfidenceTier) -> &'static str {
    match confidence {
        ConfidenceTier::High => "high",
        ConfidenceTier::Medium => "medium",
        ConfidenceTier::Low => "low",
        ConfidenceTier::Noise => "noise",
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
