//! Stable report contracts and renderable user-facing output.
//!
//! This crate owns JSON-safe report DTOs, explanation facts, and text wording.
//! It must not own CLI parsing, worker transport, or storage internals.

pub use lean_dup_diagnostics::{Error, Result};

pub mod render;
pub mod report_contract;
pub mod reports;

pub use reports::{
    AuditReport, DiffReport, DoctorReport, IndexReport, PerfReport, PerfWorkloadReport, Report, ReviewProfileCounts,
    ShowReport,
};
