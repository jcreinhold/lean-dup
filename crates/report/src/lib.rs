//! Stable report contracts and renderable user-facing output.
//!
//! This crate owns JSON-safe report DTOs, explanation facts, and text wording.
//! It must not own CLI parsing, worker transport, or storage internals.

mod error;
mod render;
mod report_contract;
mod reports;

pub use error::{Error, Result};
pub use render::render_text;
pub use report_contract::{
    AuditExplanations, ComparisonProvenanceEntry, ComparisonProvenanceExplanation, GroupExplanation,
    HiddenGroupExplanation, REPORT_SCHEMA_VERSION, SemanticProbeExplanation, VisibleQueueExplanation,
};
pub use reports::{
    AuditReport, BaselineChangeReport, BaselineDiffReport, BaselineGroupReport, CacheCleanupEntryReport,
    CacheCleanupReportDto, CacheDiagnosticsReport, CacheEntryDiagnosticsReport, CacheLabelDiagnosticsReport,
    CacheLatestDiagnosticsReport, ComparisonProvenanceReportDto, DiffReport, DoctorReport, EmbeddingPrepareReportDto,
    EmbeddingRequiredFileReportDto, EvalCountAtKDto, EvalCountMetricDto, EvalHardNegativeSurvivalDto, EvalMetricsDto,
    EvalRecallAtKDto, EvalReportDto, EvalRunReportDto, EvalSemanticVerificationStageMetricsDto, EvalStageMetricsDto,
    EvalTimingMetricsDto, IndexReport, PerfReport, PerfWorkloadReport, Report, RetrievalReport,
    ReviewDiagnosticsReport, ReviewEvidenceReport, ReviewGroupReport, ReviewMemberReport, ReviewProfileCounts,
    ReviewReport, SemanticVerificationReport, ShowReport, SourcePointReport, SourceReferenceReport, SourceSpanReport,
    audit_report, cache_cleanup_report, cache_diagnostics_report, diff_report, eval_report, show_report,
};
