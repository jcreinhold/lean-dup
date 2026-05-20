use std::collections::BTreeMap;

use lean_dup_index::HydratedDeclaration;
use serde::Serialize;

use crate::{Error, Result};

pub(crate) const ELIGIBILITY_POLICY_VERSION: &str = "lean-dup.vector-eligibility.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EligibilityPolicy {
    ActionablePublicStatement,
    Broad,
}

impl EligibilityPolicy {
    pub(crate) fn from_id(id: &str) -> Result<Self> {
        match id {
            "actionable-public-statement" => Ok(Self::ActionablePublicStatement),
            "broad" => Ok(Self::Broad),
            other => Err(Error::InvalidRequest {
                message: format!("unsupported eligibility policy: {other}"),
            }),
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::ActionablePublicStatement => "actionable-public-statement",
            Self::Broad => "broad",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EligibleDeclarations {
    pub(crate) summary: EligibilitySummary,
    pub(crate) declarations: Vec<HydratedDeclaration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct EligibilitySummary {
    pub(crate) policy_id: String,
    pub(crate) policy_version: &'static str,
    pub(crate) total: usize,
    pub(crate) eligible: usize,
    pub(crate) skipped_by_reason: BTreeMap<String, usize>,
}

pub(crate) fn filter(declarations: &[HydratedDeclaration], policy: EligibilityPolicy) -> EligibleDeclarations {
    let mut summary = EligibilitySummary {
        policy_id: policy.id().to_owned(),
        policy_version: ELIGIBILITY_POLICY_VERSION,
        total: declarations.len(),
        ..EligibilitySummary::default()
    };
    let mut eligible = Vec::new();
    for declaration in declarations {
        if let Some(reason) = skip_reason(declaration, policy) {
            *summary.skipped_by_reason.entry(reason.to_owned()).or_default() += 1;
        } else {
            summary.eligible += 1;
            eligible.push(declaration.clone());
        }
    }
    EligibleDeclarations {
        summary,
        declarations: eligible,
    }
}

fn skip_reason(declaration: &HydratedDeclaration, policy: EligibilityPolicy) -> Option<&'static str> {
    if declaration.statement_text.trim().is_empty() {
        return Some("missing-statement");
    }
    if declaration.status_flags.iter().any(|flag| flag == "generated") {
        return Some("generated");
    }
    if declaration.visibility == "private" {
        return Some("private");
    }
    if declaration
        .low_signal_markers
        .iter()
        .any(|marker| marker == "low-signal")
    {
        return Some("low-signal");
    }
    if declaration.qualified_name.contains("Synthetic.") {
        return Some("synthetic");
    }
    if matches!(policy, EligibilityPolicy::ActionablePublicStatement) {
        if !matches!(
            declaration.kind.as_str(),
            "theorem" | "lemma" | "axiom" | "def" | "abbrev" | "instance" | "structure" | "class"
        ) {
            return Some("unsupported-kind");
        }
        if declaration.status_flags.iter().any(|flag| flag == "non-actionable") {
            return Some("non-actionable");
        }
    }
    None
}
