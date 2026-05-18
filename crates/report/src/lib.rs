//! Stable report contracts and renderable user-facing output.
//!
//! This crate owns JSON-safe report DTOs, explanation facts, and text wording.
//! It must not own CLI parsing, worker transport, or storage internals.

mod error;
pub mod render;
mod report_contract;
pub mod reports;

pub use error::{Error, Result};
pub use report_contract::{
    AuditExplanations, ComparisonProvenanceEntry, ComparisonProvenanceExplanation, GroupExplanation,
    HiddenGroupExplanation, REPORT_SCHEMA_VERSION, SemanticProbeExplanation, VisibleQueueExplanation,
};
pub use reports::{
    AuditReport, CacheCleanupReportDto, DiffReport, DoctorReport, IndexReport, PerfReport, PerfWorkloadReport, Report,
    ReviewProfileCounts, ShowReport,
};
