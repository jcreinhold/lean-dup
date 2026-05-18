//! Candidate generation, semantic evidence, ranking, and source impact.
//!
//! This crate owns the search-quality pipeline from indexed declarations to
//! ranked review groups. It hides retrieval keys, posting expansion, scoring
//! constants, semantic obligation planning, and source scan policy.

use serde::Serialize;

pub mod audit;
mod baseline;
mod ranking;
mod replacement_hints;
mod retrieval;
mod semantic_verification;
mod source_refs;

pub use baseline::{BaselineChange, BaselineDiff, BaselineGroup, BaselineSnapshot, diff, load, save, snapshot};
pub use ranking::{
    ConfidenceTier, RankedGroup, RankedReview, RankingDiagnostics, ReviewAction, ReviewEvidence, ReviewEvidenceMode,
    ReviewFilter, ReviewMember, ReviewPriority, ReviewRelation, SuppressedGroup,
};
pub use retrieval::{
    CandidateExplanation, KeyContribution, RetrievalDiagnostics, RetrievalOutput, retrieve_candidates,
};
pub use semantic_verification::ProbeDiagnostics;

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
