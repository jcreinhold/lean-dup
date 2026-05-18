//! Shared diagnostics, progress, performance events, and error projection.
//!
//! This crate is intentionally low in the dependency graph so the product
//! crates can report failures and measurements without depending on the CLI.
//! It must not learn SQLite table layouts, retrieval keys, or worker transport
//! details beyond typed worker errors.

mod error;
pub mod perf;
pub mod progress;

pub use error::{Error, Result, read, read_to_string};
