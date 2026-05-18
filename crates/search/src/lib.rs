//! Candidate generation, semantic evidence, ranking, and source impact.
//!
//! This crate owns the search-quality pipeline from indexed declarations to
//! ranked review groups. It hides retrieval keys, posting expansion, scoring
//! constants, semantic obligation planning, and source scan policy.

use clap::ValueEnum;
use serde::Serialize;

pub mod baseline;
pub mod ranking;
pub mod replacement_hints;
pub mod report_contract;
pub mod retrieval;
pub mod semantic_verification;
pub mod source_refs;

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewProfile {
    Mathlib,
    Internal,
    ApiDesign,
    Noise,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProbePolicy {
    Actionable,
    Broad,
}
