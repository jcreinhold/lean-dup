use std::collections::BTreeMap;

use serde::Serialize;

use crate::pair_features::{SearchEvidenceMode, SearchPairFeatures, SearchSemanticEvidenceState};
use crate::retrieval::RetrievedCandidate;

pub(crate) const SCORER_VERSION: &str = "lean-dup.symbolic-scorer.v1";

pub(crate) const DEFAULT_SCORER_CONFIG: ScorerConfig = ScorerConfig {
    version: SCORER_VERSION,
    weights: ScorerWeights {
        statement_fingerprint: 100.0,
        safe_permutation_fingerprint: 80.0,
        connective_conclusion: 24.0,
        role_feature: 8.0,
        source_module: 4.0,
        static_evidence: 2.0,
        semantic_evidence: 120.0,
    },
    thresholds: ScorerThresholds {
        near_score: 24.0,
        visible_score: 24.0,
        transitional_alias_callers: 8,
        max_displayed_callers: 12,
    },
};

/// Stable scorer variants available to evaluation and offline artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchScoringVariant {
    AllFeatures,
    NoRoleFeatures,
    NoConnectiveConclusionFeatures,
    NoSourceModuleFeatures,
    NoStaticEvidenceFeatures,
    SemanticEvidenceOnlyRerank,
}

impl SearchScoringVariant {
    pub fn all() -> [Self; 6] {
        [
            Self::AllFeatures,
            Self::NoRoleFeatures,
            Self::NoConnectiveConclusionFeatures,
            Self::NoSourceModuleFeatures,
            Self::NoStaticEvidenceFeatures,
            Self::SemanticEvidenceOnlyRerank,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AllFeatures => "all-features",
            Self::NoRoleFeatures => "no-role-features",
            Self::NoConnectiveConclusionFeatures => "no-connective-conclusion-features",
            Self::NoSourceModuleFeatures => "no-source-module-features",
            Self::NoStaticEvidenceFeatures => "no-static-evidence-features",
            Self::SemanticEvidenceOnlyRerank => "semantic-evidence-only-rerank",
        }
    }
}

/// Versioned search scoring facts exposed to eval/report DTOs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchScoringSummary {
    pub version: &'static str,
    pub variant: SearchScoringVariant,
}

impl SearchScoringSummary {
    pub fn new(variant: SearchScoringVariant) -> Self {
        Self {
            version: SCORER_VERSION,
            variant,
        }
    }
}

/// Stable per-pair scorer output for search-quality artifacts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchPairScoring {
    pub version: &'static str,
    pub variant: SearchScoringVariant,
    pub total_score: f64,
    pub component_scores: BTreeMap<String, f64>,
}

pub(crate) struct ScorerConfig {
    pub(crate) version: &'static str,
    pub(crate) weights: ScorerWeights,
    pub(crate) thresholds: ScorerThresholds,
}

pub(crate) struct ScorerWeights {
    pub(crate) statement_fingerprint: f64,
    pub(crate) safe_permutation_fingerprint: f64,
    pub(crate) connective_conclusion: f64,
    pub(crate) role_feature: f64,
    pub(crate) source_module: f64,
    pub(crate) static_evidence: f64,
    pub(crate) semantic_evidence: f64,
}

pub(crate) struct ScorerThresholds {
    pub(crate) near_score: f64,
    pub(crate) visible_score: f64,
    pub(crate) transitional_alias_callers: usize,
    pub(crate) max_displayed_callers: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScoredObservation {
    pub(crate) ranked: bool,
    pub(crate) shown: bool,
    pub(crate) survived_shown_filter: bool,
    pub(crate) scoring: SearchPairScoring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RankingScoreFacts {
    pub(crate) exact_static: bool,
    pub(crate) permuted_static: bool,
    pub(crate) connective_static: bool,
    pub(crate) near: bool,
    pub(crate) strong_static_semantic_candidate: bool,
}

pub(crate) fn default_summary() -> SearchScoringSummary {
    SearchScoringSummary::new(SearchScoringVariant::AllFeatures)
}

pub(crate) fn thresholds() -> &'static ScorerThresholds {
    &DEFAULT_SCORER_CONFIG.thresholds
}

pub(crate) fn score_observation(
    features: &SearchPairFeatures,
    variant: SearchScoringVariant,
    default_ranked: bool,
    default_shown: bool,
) -> ScoredObservation {
    let scoring = score_features(features, variant);
    if variant == SearchScoringVariant::AllFeatures {
        return ScoredObservation {
            ranked: default_ranked,
            shown: default_shown,
            survived_shown_filter: default_shown,
            scoring,
        };
    }
    let ranked = default_ranked && scoring.total_score > 0.0;
    let shown = ranked && scoring.total_score >= DEFAULT_SCORER_CONFIG.thresholds.visible_score;
    ScoredObservation {
        ranked,
        shown,
        survived_shown_filter: shown,
        scoring,
    }
}

pub(crate) fn ranking_score_facts(
    candidate: &RetrievedCandidate,
    theorem_pair: bool,
    static_evidence_allowed: bool,
) -> RankingScoreFacts {
    let has_statement = has_contribution(candidate, "statement-fingerprint");
    let has_permutation = has_contribution(candidate, "safe-permutation-fingerprint");
    let has_connective = has_contribution(candidate, "connective-fingerprint");
    let has_conclusion = has_contribution(candidate, "conclusion-fingerprint");
    RankingScoreFacts {
        exact_static: static_evidence_allowed && theorem_pair && has_statement,
        permuted_static: static_evidence_allowed && theorem_pair && has_permutation,
        connective_static: static_evidence_allowed && theorem_pair && has_connective,
        near: candidate.score >= DEFAULT_SCORER_CONFIG.thresholds.near_score || has_conclusion,
        strong_static_semantic_candidate: has_statement || has_permutation || has_connective,
    }
}

pub(crate) fn score_features(features: &SearchPairFeatures, variant: SearchScoringVariant) -> SearchPairScoring {
    let mut components = BTreeMap::new();
    let weights = &DEFAULT_SCORER_CONFIG.weights;
    if variant != SearchScoringVariant::SemanticEvidenceOnlyRerank {
        for family in &features.retrieval_feature_families {
            match family.as_str() {
                "statement_fingerprint" => {
                    add_component(&mut components, "statement_fingerprint", weights.statement_fingerprint)
                }
                "safe_permutation_fingerprint" => add_component(
                    &mut components,
                    "safe_permutation_fingerprint",
                    weights.safe_permutation_fingerprint,
                ),
                "connective_fingerprint" | "conclusion_fingerprint"
                    if variant != SearchScoringVariant::NoConnectiveConclusionFeatures =>
                {
                    add_component(&mut components, "connective_conclusion", weights.connective_conclusion)
                }
                family if family.starts_with("role_") && variant != SearchScoringVariant::NoRoleFeatures => {
                    add_component(&mut components, "role_features", weights.role_feature)
                }
                _ => {}
            }
        }
        if variant != SearchScoringVariant::NoSourceModuleFeatures {
            let module_score = match &features.module_relation {
                crate::pair_features::SearchModuleRelation::SameModule { .. } => weights.source_module,
                crate::pair_features::SearchModuleRelation::DifferentModules { .. } => weights.source_module / 2.0,
            };
            add_component(&mut components, "source_module", module_score);
        }
        if features.evidence_mode == SearchEvidenceMode::Static
            && variant != SearchScoringVariant::NoStaticEvidenceFeatures
        {
            add_component(&mut components, "static_evidence", weights.static_evidence);
        }
    }
    if features.semantic_evidence_state != SearchSemanticEvidenceState::NotRun {
        add_component(&mut components, "semantic_evidence", weights.semantic_evidence);
    }
    let total_score = components.values().sum();
    SearchPairScoring {
        version: DEFAULT_SCORER_CONFIG.version,
        variant,
        total_score,
        component_scores: components,
    }
}

fn add_component(components: &mut BTreeMap<String, f64>, family: &'static str, score: f64) {
    *components.entry(family.to_owned()).or_insert(0.0) += score;
}

fn has_contribution(candidate: &RetrievedCandidate, kind: &str) -> bool {
    candidate
        .explanation
        .contributions
        .iter()
        .any(|contribution| contribution.kind == kind)
}

#[cfg(test)]
mod tests {
    use crate::pair_features::{
        SearchEvidenceMode, SearchModuleRelation, SearchPairFeatures, SearchSemanticEvidenceState,
    };

    use super::{SearchScoringVariant, score_features, score_observation};

    #[test]
    fn ablations_disable_only_their_feature_families() {
        let features = features();

        let all = score_features(&features, SearchScoringVariant::AllFeatures);
        let no_role = score_features(&features, SearchScoringVariant::NoRoleFeatures);
        let no_connective = score_features(&features, SearchScoringVariant::NoConnectiveConclusionFeatures);

        assert!(all.component_scores.contains_key("role_features"));
        assert!(!no_role.component_scores.contains_key("role_features"));
        assert!(all.component_scores.contains_key("connective_conclusion"));
        assert!(!no_connective.component_scores.contains_key("connective_conclusion"));
        assert_eq!(
            all.component_scores.get("statement_fingerprint"),
            no_role.component_scores.get("statement_fingerprint")
        );
    }

    #[test]
    fn semantic_only_without_semantic_evidence_is_deterministic_and_hidden() {
        let scored = score_observation(
            &features(),
            SearchScoringVariant::SemanticEvidenceOnlyRerank,
            true,
            true,
        );

        assert_eq!(scored.scoring.total_score, 0.0);
        assert!(!scored.ranked);
        assert!(!scored.shown);
        assert!(scored.scoring.component_scores.is_empty());
    }

    #[test]
    fn scorer_output_uses_stable_families_not_raw_payloads() {
        let json = serde_json::to_string(&score_features(&features(), SearchScoringVariant::AllFeatures)).unwrap();

        for forbidden in [
            "sqlite",
            "posting",
            "IndexQuery",
            "FeatureMatch",
            "raw statement",
            "secret-role-key",
        ] {
            assert!(!json.contains(forbidden), "{forbidden} leaked in {json}");
        }
        assert!(json.contains("statement_fingerprint"));
        assert!(json.contains("role_features"));
    }

    fn features() -> SearchPairFeatures {
        SearchPairFeatures {
            retrieval_feature_families: vec![
                "statement_fingerprint".to_owned(),
                "connective_fingerprint".to_owned(),
                "role_conclusion_const".to_owned(),
            ],
            declaration_kinds: vec!["theorem".to_owned()],
            evidence_mode: SearchEvidenceMode::Static,
            structural_fingerprint_families: vec!["statement_fingerprint".to_owned()],
            role_overlap: Vec::new(),
            module_relation: SearchModuleRelation::SameModule {
                module: "Tiny".to_owned(),
            },
            semantic_evidence_state: SearchSemanticEvidenceState::NotRun,
            cheap_blockers: Vec::new(),
        }
    }
}
