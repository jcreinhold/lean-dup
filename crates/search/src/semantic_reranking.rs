use serde::{Deserialize, Serialize};

pub(crate) const SEMANTIC_RERANKING_VERSION: &str = "lean-dup.semantic-reranking.v1";

/// Versioned semantic-reranking policy facts exposed to eval/report DTOs.
///
/// The version names the search-owned obligation policy. It does not expose
/// worker transport, probe chunking, cache keys, or Lean expression payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchSemanticRerankingSummary {
    pub version: &'static str,
}

impl Default for SearchSemanticRerankingSummary {
    fn default() -> Self {
        Self {
            version: SEMANTIC_RERANKING_VERSION,
        }
    }
}

/// Stable proof-obligation classes used by semantic reranking artifacts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchSemanticObligationKind {
    #[default]
    ExactTheorem,
    PermutedTheorem,
    Replacement,
    ReducibleDefinition,
    Specialization,
    LocalDuplicate,
}

impl SearchSemanticObligationKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ExactTheorem => "exact-theorem",
            Self::PermutedTheorem => "permuted-theorem",
            Self::Replacement => "replacement",
            Self::ReducibleDefinition => "reducible-definition",
            Self::Specialization => "specialization",
            Self::LocalDuplicate => "local-duplicate",
        }
    }
}

/// Result status for one planned semantic obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchSemanticObligationStatus {
    Planned,
    Verified,
    Rejected,
    Unavailable,
    Cached,
}

/// Stable reason labels for unavailable semantic obligations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchSemanticUnavailableReason {
    MissingDeclaration,
    Unsupported,
    OpaqueOrUnreducible,
    Timeout,
    InternalError,
    Unknown,
}

/// Aggregate yield counters for one semantic-obligation kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchSemanticObligationYield {
    pub kind: SearchSemanticObligationKind,
    pub planned: usize,
    pub verified: usize,
    pub rejected: usize,
    pub unavailable: usize,
    pub cached: usize,
    pub worker_pairs: usize,
}

/// Per-pair semantic obligation fact for search-quality datasets and reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSemanticObligationFact {
    pub kind: SearchSemanticObligationKind,
    pub status: SearchSemanticObligationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<SearchSemanticUnavailableReason>,
}

pub(crate) fn summary() -> SearchSemanticRerankingSummary {
    SearchSemanticRerankingSummary::default()
}

pub(crate) fn sorted_yield(mut yields: Vec<SearchSemanticObligationYield>) -> Vec<SearchSemanticObligationYield> {
    yields.sort_by_key(|item| item.kind);
    yields
}
