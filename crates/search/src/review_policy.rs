use std::collections::BTreeSet;

use serde::Serialize;

use crate::retrieval::KeyContribution;
use lean_dup_index::HydratedDeclaration;

pub(crate) const REVIEW_POLICY_VERSION: &str = "lean-dup.symbolic-review-policy.v2";

/// Stable review-policy facts exposed to evaluation and reports.
///
/// Callers may record this identifier to explain which symbolic cleanup queue
/// policy made visibility decisions. The ranking thresholds, feature weights,
/// and blocker rules remain owned by `lean-dup-search`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SearchReviewPolicySummary {
    pub version: &'static str,
}

pub(crate) fn summary() -> SearchReviewPolicySummary {
    SearchReviewPolicySummary {
        version: REVIEW_POLICY_VERSION,
    }
}

pub(crate) fn symbolic_observation_visible(
    left: &HydratedDeclaration,
    right: &HydratedDeclaration,
    contributions: &[KeyContribution],
) -> bool {
    if !visibility_blockers(left, right, contributions).is_empty() {
        return false;
    }
    if theorem_like(left) && theorem_like(right) {
        return has_contribution(contributions, "statement-fingerprint")
            || has_contribution(contributions, "safe-permutation-fingerprint");
    }
    false
}

pub(crate) fn visibility_blockers(
    left: &HydratedDeclaration,
    right: &HydratedDeclaration,
    contributions: &[KeyContribution],
) -> BTreeSet<String> {
    let mut blockers = BTreeSet::new();
    if is_generated(left) || is_generated(right) {
        blockers.insert("generated-declaration".to_owned());
    }
    if non_public(left) || non_public(right) {
        blockers.insert("non-public-declaration".to_owned());
    }
    if low_signal(left) || low_signal(right) {
        blockers.insert("low-signal-declaration".to_owned());
    }
    if broad_head_only(left, right, contributions) {
        blockers.insert("broad-head-only".to_owned());
    }
    if typeclass_instance_noise(left) || typeclass_instance_noise(right) {
        blockers.insert("typeclass-instance-noise".to_owned());
    }
    let has_proof_grade_or_source_clone = has_contribution(contributions, "source-fingerprint")
        || has_contribution(contributions, "semantic-proof-grade");
    if !(theorem_like(left) && theorem_like(right) || has_proof_grade_or_source_clone) {
        blockers.insert("non-theorem-static-only".to_owned());
    }
    blockers
}

pub(crate) fn theorem_like(declaration: &HydratedDeclaration) -> bool {
    matches!(declaration.kind.as_str(), "theorem" | "axiom")
}

fn is_generated(declaration: &HydratedDeclaration) -> bool {
    declaration.status_flags.iter().any(|flag| flag == "generated")
}

fn non_public(declaration: &HydratedDeclaration) -> bool {
    declaration.visibility != "public"
}

fn low_signal(declaration: &HydratedDeclaration) -> bool {
    !declaration.low_signal_markers.is_empty()
}

fn typeclass_instance_noise(declaration: &HydratedDeclaration) -> bool {
    declaration.kind == "instance" || declaration.display_name.starts_with("inst")
}

fn broad_head_only(left: &HydratedDeclaration, right: &HydratedDeclaration, contributions: &[KeyContribution]) -> bool {
    if contributions.is_empty() {
        return false;
    }
    let broad_heads = left
        .low_signal_markers
        .iter()
        .chain(right.low_signal_markers.iter())
        .filter_map(|marker| marker.strip_prefix("broad_head:"))
        .collect::<BTreeSet<_>>();
    !broad_heads.is_empty()
        && contributions.iter().all(|contribution| {
            contribution.kind == "role-feature"
                && contribution
                    .display
                    .as_deref()
                    .is_some_and(|display| broad_heads.contains(display))
        })
}

fn has_contribution(contributions: &[KeyContribution], kind: &str) -> bool {
    contributions.iter().any(|contribution| contribution.kind == kind)
}

#[cfg(test)]
mod tests {
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, RoleFeature};

    use super::{symbolic_observation_visible, visibility_blockers};
    use crate::retrieval::KeyContribution;

    #[test]
    fn default_symbolic_visibility_keeps_public_theorem_statement_pairs() {
        let left = declaration("Tiny.left", "theorem");
        let right = declaration("Tiny.right", "theorem");
        let contributions = vec![contribution("statement-fingerprint")];

        assert!(symbolic_observation_visible(&left, &right, &contributions));
    }

    #[test]
    fn default_symbolic_visibility_hides_diagnostic_static_pairs() {
        let mut low_signal = declaration("Tiny.low", "theorem");
        low_signal.low_signal_markers.push("broad_head:Eq".to_owned());
        let private = declaration("Tiny.private", "theorem").with_visibility("private");
        let definition = declaration("Tiny.defn", "def");
        let public_theorem = declaration("Tiny.public", "theorem");
        let statement = vec![contribution("statement-fingerprint")];

        assert!(!symbolic_observation_visible(&low_signal, &public_theorem, &statement));
        assert!(!symbolic_observation_visible(&private, &public_theorem, &statement));
        assert!(!symbolic_observation_visible(&definition, &public_theorem, &statement));

        let blockers = visibility_blockers(&definition, &public_theorem, &statement);
        assert!(blockers.contains("non-theorem-static-only"));
    }

    fn declaration(name: &str, kind: &str) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id(name),
            declaration_id: format!("workspace:Tiny:{name}"),
            origin: "workspace".to_owned(),
            module: "Tiny".to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap_or(name).to_owned(),
            kind: kind.to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: String::new(),
            docstring_text: None,
            definition_body_summary: None,
            status_flags: Vec::new(),
            feature_version: "test".to_owned(),
            fingerprints: Fingerprints {
                statement: String::new(),
                safe_binder_permutation: String::new(),
                connective_shape: String::new(),
                conclusion_shape: String::new(),
            },
            role_features: vec![RoleFeature {
                role: "conclusion_const".to_owned(),
                key: "Nat".to_owned(),
                display: Some("Nat".to_owned()),
            }],
            binder_count: 0,
            low_signal_markers: Vec::new(),
        }
    }

    trait TestDeclarationExt {
        fn with_visibility(self, visibility: &str) -> Self;
    }

    impl TestDeclarationExt for HydratedDeclaration {
        fn with_visibility(mut self, visibility: &str) -> Self {
            self.visibility = visibility.to_owned();
            self
        }
    }

    fn contribution(kind: &str) -> KeyContribution {
        KeyContribution {
            kind: kind.to_owned(),
            role: None,
            display: None,
            key: "test".to_owned(),
            score: 100.0,
        }
    }
}
