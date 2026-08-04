//! Offline labels, evaluation suites, stage metrics, and quality gates.
//!
//! This crate measures the search system. It owns label schemas, manual-suite
//! policy, denominators, and artifact-oriented metrics without changing the
//! production audit behavior.

use serde::Serialize;

mod error;
mod eval;

pub use error::{Error, Result};
pub use eval::{
    AdjudicationSource, CandidateStageSurvival, CountMetric, EvalOutput, EvalRequest, EvaluationMetrics,
    EvaluationRunReport, ExpectedStageVisibility, GoldLabelFact, GoldLabels, GoldPair, HardNegativeSurvival,
    LabelConfidence, LabelEndpointResolution, LabelEndpointStatus, LabelFactSource, LabelLossLayer, LabelPolarity,
    LabelResolutionCandidate, LabelResolutionReport, LabelResolutionStatus, LabelTrace, LabelTraceCount,
    ManualMathlibPrerequisites, ManualSuitePrerequisites, MatchClass, ObservedCandidateSource, ObservedPair,
    ObservedRun, PrerequisiteCheck, PrerequisiteStatus, RecallAtK, SearchStageMetrics, TimingMetrics, TypedGoldLabel,
    load_builtin, parse_json, run, score_run,
};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvalSuite {
    /// Small checked-in corpus used by ordinary eval.
    Default,
    /// Checked-in labels that guard against known false-positive patterns.
    HardNegatives,
    /// Private operator suite over the local Proofs corpus.
    ManualInternal,
    /// Private operator suite comparing local declarations against mathlib.
    ManualMathlib,
    /// Aggregate quality gate over the non-manual suites and available manual children.
    ProductionGate,
}

impl EvalSuite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::HardNegatives => "hard-negatives",
            Self::ManualInternal => "manual-internal",
            Self::ManualMathlib => "manual-mathlib",
            Self::ProductionGate => "production-gate",
        }
    }
}
