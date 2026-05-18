//! Candidate generation, semantic evidence, ranking, and source impact.
//!
//! This crate owns the search-quality pipeline from indexed declarations to
//! ranked review groups. It hides retrieval keys, posting expansion, scoring
//! constants, semantic obligation planning, and source scan policy.

use serde::Serialize;

pub mod audit;
mod baseline;
mod error;
pub mod observation;
mod ranking;
mod replacement_hints;
mod retrieval;
mod semantic_verification;
mod source_refs;

pub use error::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewProfile {
    Mathlib,
    Internal,
    ApiDesign,
    Noise,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProbePolicy {
    Actionable,
    Broad,
}
