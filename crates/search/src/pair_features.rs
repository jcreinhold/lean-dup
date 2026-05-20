use std::collections::{BTreeMap, BTreeSet};

use lean_dup_index::HydratedDeclaration;
use serde::Serialize;

use crate::retrieval::KeyContribution;
use crate::semantic_reranking::{
    SearchSemanticObligationFact, SearchSemanticRerankingSummary, summary as semantic_reranking_summary,
};
use crate::semantic_verification::{EvidenceStatus, SemanticEvidence};

/// Stable pair-feature facts for search-quality datasets.
///
/// These facts describe the candidate pair at the search boundary. They name
/// feature families, evidence modes, and cheap blockers without exposing
/// retrieval keys, source text, SQLite storage, or Lean expression payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchPairFeatures {
    pub retrieval_feature_families: Vec<String>,
    pub declaration_kinds: Vec<String>,
    pub evidence_mode: SearchEvidenceMode,
    pub structural_fingerprint_families: Vec<String>,
    pub role_overlap: Vec<SearchRoleOverlap>,
    pub module_relation: SearchModuleRelation,
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub semantic_evidence_state: SearchSemanticEvidenceState,
    pub semantic_obligations: Vec<SearchSemanticObligationFact>,
    pub cheap_blockers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchEvidenceMode {
    Local,
    SourceBacked,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRoleOverlap {
    pub family: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SearchModuleRelation {
    SameModule { module: String },
    DifferentModules { left_module: String, right_module: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchSemanticEvidenceState {
    NotRun,
    Verified,
    Rejected,
    Unavailable,
}

pub(crate) fn pair_features(
    left: &HydratedDeclaration,
    right: &HydratedDeclaration,
    contributions: &[KeyContribution],
) -> SearchPairFeatures {
    pair_features_with_semantic(left, right, contributions, None)
}

pub(crate) fn pair_features_with_semantic(
    left: &HydratedDeclaration,
    right: &HydratedDeclaration,
    contributions: &[KeyContribution],
    semantic: Option<&SemanticEvidence>,
) -> SearchPairFeatures {
    SearchPairFeatures {
        retrieval_feature_families: feature_families(contributions),
        declaration_kinds: declaration_kinds(left, right),
        evidence_mode: evidence_mode(left, right),
        structural_fingerprint_families: structural_fingerprint_families(left, right),
        role_overlap: role_overlap(left, right),
        module_relation: module_relation(left, right),
        semantic_reranking: semantic_reranking_summary(),
        semantic_evidence_state: semantic_evidence_state(semantic),
        semantic_obligations: semantic
            .map(|evidence| vec![evidence.semantic_obligation_fact()])
            .unwrap_or_default(),
        cheap_blockers: cheap_blockers(left, right, contributions),
    }
}

fn semantic_evidence_state(semantic: Option<&SemanticEvidence>) -> SearchSemanticEvidenceState {
    match semantic.map(|evidence| evidence.status) {
        None => SearchSemanticEvidenceState::NotRun,
        Some(EvidenceStatus::Verified) => SearchSemanticEvidenceState::Verified,
        Some(EvidenceStatus::Rejected) => SearchSemanticEvidenceState::Rejected,
        Some(EvidenceStatus::Unavailable) => SearchSemanticEvidenceState::Unavailable,
    }
}

pub(crate) fn feature_families(contributions: &[KeyContribution]) -> Vec<String> {
    let mut families = contributions
        .iter()
        .map(feature_family)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if families.is_empty() {
        families.push("unknown".to_owned());
    }
    families
}

fn feature_family(contribution: &KeyContribution) -> String {
    contribution.feature_family()
}

fn role_family(role: Option<&str>) -> String {
    match role {
        Some("conclusion_const") => "role_conclusion_const".to_owned(),
        Some("hypothesis_const") => "role_hypothesis_const".to_owned(),
        Some("conclusion_head" | "hypothesis_head" | "binder_domain_head") => "role_head".to_owned(),
        _ => "role_other".to_owned(),
    }
}

fn declaration_kinds(left: &HydratedDeclaration, right: &HydratedDeclaration) -> Vec<String> {
    let mut kinds = BTreeSet::new();
    kinds.insert(left.kind.clone());
    kinds.insert(right.kind.clone());
    kinds.into_iter().collect()
}

fn evidence_mode(left: &HydratedDeclaration, right: &HydratedDeclaration) -> SearchEvidenceMode {
    if left.origin == right.origin {
        SearchEvidenceMode::Local
    } else if left.source_span.is_some() && right.source_span.is_some() {
        SearchEvidenceMode::SourceBacked
    } else {
        SearchEvidenceMode::Static
    }
}

fn structural_fingerprint_families(left: &HydratedDeclaration, right: &HydratedDeclaration) -> Vec<String> {
    let mut families = Vec::new();
    if !left.fingerprints.statement.is_empty() && left.fingerprints.statement == right.fingerprints.statement {
        families.push("statement_fingerprint".to_owned());
    }
    if !left.fingerprints.safe_binder_permutation.is_empty()
        && left.fingerprints.safe_binder_permutation == right.fingerprints.safe_binder_permutation
    {
        families.push("safe_permutation_fingerprint".to_owned());
    }
    if !left.fingerprints.connective_shape.is_empty()
        && left.fingerprints.connective_shape == right.fingerprints.connective_shape
    {
        families.push("connective_fingerprint".to_owned());
    }
    if !left.fingerprints.conclusion_shape.is_empty()
        && left.fingerprints.conclusion_shape == right.fingerprints.conclusion_shape
    {
        families.push("conclusion_fingerprint".to_owned());
    }
    families
}

fn role_overlap(left: &HydratedDeclaration, right: &HydratedDeclaration) -> Vec<SearchRoleOverlap> {
    let right_features = right
        .role_features
        .iter()
        .map(|feature| (feature.role.as_str(), feature.key.as_str()))
        .collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::<String, usize>::new();
    for feature in &left.role_features {
        if right_features.contains(&(feature.role.as_str(), feature.key.as_str())) {
            *counts.entry(role_family(Some(&feature.role))).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|(family, count)| SearchRoleOverlap { family, count })
        .collect()
}

fn module_relation(left: &HydratedDeclaration, right: &HydratedDeclaration) -> SearchModuleRelation {
    if left.module == right.module {
        SearchModuleRelation::SameModule {
            module: left.module.clone(),
        }
    } else {
        SearchModuleRelation::DifferentModules {
            left_module: left.module.clone(),
            right_module: right.module.clone(),
        }
    }
}

fn cheap_blockers(
    left: &HydratedDeclaration,
    right: &HydratedDeclaration,
    contributions: &[KeyContribution],
) -> Vec<String> {
    let mut blockers = BTreeSet::new();
    if left.status_flags.iter().any(|flag| flag == "generated")
        || right.status_flags.iter().any(|flag| flag == "generated")
    {
        blockers.insert("generated".to_owned());
    }
    if left.visibility != "public" || right.visibility != "public" {
        blockers.insert("non_public".to_owned());
    }
    if !left.low_signal_markers.is_empty() || !right.low_signal_markers.is_empty() {
        blockers.insert("low_signal_marker".to_owned());
    }
    if !contributions.is_empty()
        && contributions
            .iter()
            .all(|contribution| feature_family(contribution) == "role_head")
    {
        blockers.insert("role_head_only".to_owned());
    }
    blockers.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, RoleFeature};

    use super::{SearchEvidenceMode, SearchModuleRelation, SearchSemanticEvidenceState, pair_features};
    use crate::retrieval::KeyContribution;

    #[test]
    fn pair_features_emit_stable_sorted_facts() {
        let left = declaration("A.left", "A", "theorem", "workspace").with_role("conclusion_const", "opaque-a");
        let right = declaration("A.right", "A", "theorem", "workspace").with_role("conclusion_const", "opaque-a");
        let features = pair_features(
            &left,
            &right,
            &[
                contribution("role-feature", Some("conclusion_const")),
                contribution("statement-fingerprint", None),
            ],
        );

        assert_eq!(
            features.retrieval_feature_families,
            ["role_conclusion_const", "statement_fingerprint"]
        );
        assert_eq!(features.role_overlap[0].family, "role_conclusion_const");
        assert_eq!(features.role_overlap[0].count, 1);
        assert_eq!(features.evidence_mode, SearchEvidenceMode::Local);
        assert_eq!(
            features.module_relation,
            SearchModuleRelation::SameModule { module: "A".to_owned() }
        );
        assert_eq!(features.semantic_evidence_state, SearchSemanticEvidenceState::NotRun);
    }

    #[test]
    fn pair_features_distinguish_source_backed_from_static() {
        let left = declaration("A.left", "A", "theorem", "workspace").source_backed();
        let source_backed = declaration("B.right", "B", "theorem", "external:fixture").source_backed();
        let static_only = declaration("C.right", "C", "theorem", "external:static");

        assert_eq!(
            pair_features(&left, &source_backed, &[]).evidence_mode,
            SearchEvidenceMode::SourceBacked
        );
        assert_eq!(
            pair_features(&left, &static_only, &[]).evidence_mode,
            SearchEvidenceMode::Static
        );
    }

    #[test]
    fn serialized_features_do_not_contain_raw_keys_or_source_payloads() {
        let left = declaration("A.left", "A", "theorem", "workspace")
            .with_statement_fingerprint("secret-statement-key")
            .with_role("conclusion_const", "secret-role-key")
            .source_backed();
        let right = declaration("B.right", "B", "theorem", "external:fixture")
            .with_statement_fingerprint("secret-statement-key")
            .with_role("conclusion_const", "secret-role-key")
            .source_backed();
        let features = pair_features(
            &left,
            &right,
            &[KeyContribution {
                kind: "statement-fingerprint".to_owned(),
                role: None,
                display: Some("secret-display".to_owned()),
                key: "secret-contribution-key".to_owned(),
                score: 1.0,
            }],
        );

        let json = serde_json::to_string(&features).unwrap();
        for forbidden in [
            "secret-statement-key",
            "secret-role-key",
            "secret-display",
            "secret-contribution-key",
            "statement_text",
            "/Users/",
        ] {
            assert!(!json.contains(forbidden), "leaked {forbidden}: {json}");
        }
    }

    #[derive(Clone)]
    struct DeclarationBuilder(HydratedDeclaration);

    impl DeclarationBuilder {
        fn with_statement_fingerprint(mut self, value: &str) -> Self {
            self.0.fingerprints.statement = value.to_owned();
            self
        }

        fn with_role(mut self, role: &str, key: &str) -> Self {
            self.0.role_features.push(RoleFeature {
                role: role.to_owned(),
                key: key.to_owned(),
                display: Some("display text must not serialize".to_owned()),
            });
            self
        }

        fn source_backed(mut self) -> HydratedDeclaration {
            self.0.source_span = Some(lean_dup_worker::SourceSpan {
                file: "A.lean".to_owned(),
                start: lean_dup_worker::SourcePoint { line: 1, column: 1 },
                end: lean_dup_worker::SourcePoint { line: 1, column: 2 },
            });
            self.0
        }
    }

    impl std::ops::Deref for DeclarationBuilder {
        type Target = HydratedDeclaration;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    fn declaration(name: &str, module: &str, kind: &str, origin: &str) -> DeclarationBuilder {
        DeclarationBuilder(HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id(name),
            declaration_id: name.to_owned(),
            origin: origin.to_owned(),
            module: module.to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.to_owned(),
            kind: kind.to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: "raw statement text must not serialize".to_owned(),
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
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: Vec::new(),
        })
    }

    fn contribution(kind: &str, role: Option<&str>) -> KeyContribution {
        KeyContribution {
            kind: kind.to_owned(),
            role: role.map(str::to_owned),
            display: None,
            key: "opaque-key".to_owned(),
            score: 1.0,
        }
    }
}
