//! Candidate generation, semantic evidence, ranking, and source impact.
//!
//! This crate owns the search-quality pipeline from indexed declarations to
//! ranked review groups. It hides retrieval keys, posting expansion, scoring
//! constants, semantic obligation planning, and source scan policy.

use serde::Serialize;

mod audit;
mod baseline;
mod error;
mod observation;
mod pair_features;
mod ranking;
mod replacement_hints;
mod retrieval;
mod review_policy;
mod scorer;
mod semantic_reranking;
mod semantic_verification;
mod source_refs;

pub use audit::{
    AuditEvidence, AuditGroup, AuditHiddenGroupCounts, AuditHiddenReason, AuditMember, AuditOutput, AuditPairEvidence,
    AuditProbeSummary, AuditQueueCounts, AuditQueueSummary, AuditReplacementHint, AuditRequest, AuditRetrievalSummary,
    AuditReview, AuditReviewDiagnostics, AuditSourceReference, AuditVisibility, AuditVisibilityOptions, DiffOutput,
    SearchBaselineChange, SearchBaselineDiff, SearchBaselineGroup, ShowOutput, run_audit, run_diff, run_show,
};
pub use error::{Error, Result};
pub use observation::{
    SearchCandidateLossFact, SearchCandidateLossStage, SearchCandidateSourceFact, SearchCandidateSourceFamily,
    SearchCandidateTopKStatus, SearchFanoutPolicySummary, SearchObservation, SearchObservationRequest,
    SearchObservedPair, SearchPrunedFeatureFanout, SearchRetrievalObservation, SearchStageObservation,
    SearchStageObservedPair, SearchTrackedPair, observe_search, observe_search_stages, rescore_observation,
};
pub use pair_features::{
    SearchEvidenceMode, SearchModuleRelation, SearchPairFeatures, SearchRoleOverlap, SearchSemanticEvidenceState,
};
pub use review_policy::SearchReviewPolicySummary;
pub use scorer::{SearchPairScoring, SearchScoringSummary, SearchScoringVariant};
pub use semantic_reranking::{
    SearchSemanticObligationFact, SearchSemanticObligationKind, SearchSemanticObligationStatus,
    SearchSemanticObligationYield, SearchSemanticRerankingSummary, SearchSemanticUnavailableReason,
};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProbePolicy {
    Actionable,
    Broad,
}

/// Stable semantic-probe status counts for one planning dimension.
///
/// The dimension key may be a source id or match-class label. Counts describe
/// search-owned planning and result status; they do not expose worker rows,
/// proof obligations, cache keys, or raw Lean terms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProbeStatusBreakdown {
    pub planned: usize,
    pub cached: usize,
    pub worker: usize,
    pub verified: usize,
    pub rejected: usize,
    pub unavailable: usize,
    pub skipped_by_policy: usize,
    pub skipped_by_budget: usize,
    pub timeout: usize,
}
