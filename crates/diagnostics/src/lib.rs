//! Shared diagnostics, progress, performance events, and error projection.
//!
//! This crate is intentionally low in the dependency graph so product crates
//! can report failures and measurements without depending on CLI or report
//! rendering. It owns diagnostic plumbing, not user-facing report contracts.

mod error;
mod logging;
pub mod perf;
pub mod progress;

pub use error::{Error, Result, read, read_to_string};
pub use logging::install_tracing;
