//! Offline labels, evaluation suites, stage metrics, and quality gates.
//!
//! This crate measures the search system. It owns label schemas, manual-suite
//! policy, denominators, and artifact-oriented metrics without changing the
//! production audit behavior.

use serde::Serialize;

mod eval;

pub use eval::{EvalRequest, EvaluationReport, peak_rss_bytes, render_metrics, run};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvalSuite {
    Default,
    HardNegatives,
    KanproofsInternal,
    KanproofsMathlib,
    ProductionGate,
}

impl EvalSuite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::HardNegatives => "hard-negatives",
            Self::KanproofsInternal => "kanproofs-internal",
            Self::KanproofsMathlib => "kanproofs-mathlib",
            Self::ProductionGate => "production-gate",
        }
    }
}
