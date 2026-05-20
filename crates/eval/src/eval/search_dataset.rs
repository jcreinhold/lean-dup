use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lean_dup_search::{SearchObservation, SearchObservedPair, SearchPairFeatures, SearchScoringSummary};
use serde::Serialize;

use crate::eval::labels::{GoldLabels, TypedGoldLabel};
use crate::eval::scoring::GoldPair;
use crate::{Error, Result};

pub const SEARCH_DATASET_SCHEMA_VERSION: &str = "lean-dup.search-dataset.v1";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchDataset {
    pub schema_version: &'static str,
    pub suite: String,
    pub scoring: SearchScoringSummary,
    pub semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary,
    pub semantic_obligation_yield: Vec<lean_dup_search::SearchSemanticObligationYield>,
    pub pairs: Vec<SearchDatasetPair>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchDatasetPair {
    pub left: String,
    pub right: String,
    pub label_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<SearchDatasetLabel>,
    pub stage_position: SearchDatasetStagePosition,
    pub final_visibility: SearchDatasetFinalVisibility,
    pub features: SearchPairFeatures,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchDatasetLabel {
    pub polarity: crate::eval::labels::LabelPolarity,
    pub match_class: crate::eval::labels::MatchClass,
    pub expected_stage_visibility: crate::eval::labels::ExpectedStageVisibility,
    pub adjudication_source: crate::eval::labels::AdjudicationSource,
    pub confidence: crate::eval::labels::LabelConfidence,
    pub semantic_verification_required: bool,
    pub static_evidence_acceptable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchDatasetStagePosition {
    pub generated: bool,
    pub ranked: bool,
    pub rank: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchDatasetFinalVisibility {
    pub shown: bool,
    pub survived_shown_filter: bool,
}

pub fn build(suite: &str, labels: &GoldLabels, observation: &SearchObservation) -> SearchDataset {
    let typed_by_pair = labels
        .typed_pairs
        .iter()
        .map(|label| (label.pair.clone(), label))
        .collect::<BTreeMap<_, _>>();
    let mut pairs = observation
        .pairs
        .iter()
        .map(|observed| dataset_pair(observed, &typed_by_pair))
        .collect::<Vec<_>>();
    pairs.sort_by(|left, right| {
        left.left
            .cmp(&right.left)
            .then_with(|| left.right.cmp(&right.right))
            .then_with(|| left.stage_position.rank.cmp(&right.stage_position.rank))
    });
    SearchDataset {
        schema_version: SEARCH_DATASET_SCHEMA_VERSION,
        suite: suite.to_owned(),
        scoring: observation.scoring.clone(),
        semantic_reranking: observation.semantic_reranking.clone(),
        semantic_obligation_yield: observation.semantic_obligation_yield.clone(),
        pairs,
    }
}

pub fn write_default_artifact(repo_root: &Path, dataset: &SearchDataset) -> Result<PathBuf> {
    let artifact = PathBuf::from("target/search-quality").join(format!("{}-dataset.json", dataset.suite));
    let absolute = repo_root.join(&artifact);
    let parent = absolute.parent().ok_or_else(|| Error::Eval {
        message: format!("dataset artifact has no parent: {}", absolute.display()),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        message: "could not create search dataset directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let json = serde_json::to_string_pretty(dataset)?;
    fs::write(&absolute, format!("{json}\n")).map_err(|source| Error::Io {
        message: "could not write search dataset artifact",
        path: absolute,
        source,
    })?;
    Ok(artifact)
}

fn dataset_pair(
    observed: &SearchObservedPair,
    typed_by_pair: &BTreeMap<GoldPair, &TypedGoldLabel>,
) -> SearchDatasetPair {
    let pair = GoldPair::new(observed.left.clone(), observed.right.clone());
    let label = typed_by_pair.get(&pair).map(|typed| SearchDatasetLabel {
        polarity: typed.polarity,
        match_class: typed.match_class,
        expected_stage_visibility: typed.expected_stage_visibility,
        adjudication_source: typed.adjudication_source,
        confidence: typed.confidence,
        semantic_verification_required: typed.semantic_verification_required,
        static_evidence_acceptable: typed.static_evidence_acceptable,
    });
    SearchDatasetPair {
        left: pair.left,
        right: pair.right,
        label_status: label_status(label.as_ref()),
        label,
        stage_position: SearchDatasetStagePosition {
            generated: observed.generated,
            ranked: observed.ranked,
            rank: observed.rank,
        },
        final_visibility: SearchDatasetFinalVisibility {
            shown: observed.shown,
            survived_shown_filter: observed.survived_shown_filter,
        },
        features: observed.features.clone(),
    }
}

fn label_status(label: Option<&SearchDatasetLabel>) -> String {
    label.map_or_else(
        || "unlabeled".to_owned(),
        |label| match label.polarity {
            crate::eval::labels::LabelPolarity::Positive => "positive".to_owned(),
            crate::eval::labels::LabelPolarity::HardNegative => "hard-negative".to_owned(),
        },
    )
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;
    use tempfile::TempDir;

    use super::{SEARCH_DATASET_SCHEMA_VERSION, build, write_default_artifact};
    use crate::eval::labels::{
        AdjudicationSource, ExpectedStageVisibility, GoldLabels, LabelConfidence, LabelPolarity, MatchClass,
        TypedGoldLabel,
    };
    use crate::eval::scoring::GoldPair;
    use lean_dup_search::{
        SearchEvidenceMode, SearchModuleRelation, SearchObservation, SearchObservedPair, SearchPairFeatures,
        SearchRetrievalObservation, SearchSemanticEvidenceState,
    };

    #[test]
    fn typed_labels_join_deterministically_and_unlabeled_pairs_remain() {
        let labels = labels();
        let observation = SearchObservation {
            pairs: vec![
                observed("Z.unlabeled", "A.unlabeled", 2),
                observed("Tiny.same_right", "Tiny.same_left", 1),
            ],
            visible_groups_found: 1,
            visible_groups_total: 2,
            scoring: lean_dup_search::SearchScoringSummary::new(lean_dup_search::SearchScoringVariant::SymbolicOnly),
            semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
            semantic_obligation_yield: Vec::new(),
            retrieval: SearchRetrievalObservation::default(),
            embedding_documents: lean_dup_search::SearchEmbeddingDocuments::default(),
        };

        let dataset = build("unit", &labels, &observation);

        assert_eq!(dataset.schema_version, SEARCH_DATASET_SCHEMA_VERSION);
        assert_eq!(dataset.semantic_reranking.version, "lean-dup.semantic-reranking.v1");
        assert!(dataset.semantic_obligation_yield.is_empty());
        assert_eq!(dataset.pairs[0].left, "A.unlabeled");
        assert_eq!(dataset.pairs[0].label_status, "unlabeled");
        assert!(dataset.pairs[0].label.is_none());
        assert_eq!(dataset.pairs[1].left, "Tiny.same_left");
        assert_eq!(dataset.pairs[1].label_status, "positive");
        assert_eq!(
            dataset.pairs[1].label.as_ref().unwrap().match_class,
            MatchClass::ExactTheoremDuplicate
        );
    }

    #[test]
    fn fixture_dataset_writer_produces_stable_json() {
        let temp = TempDir::new().unwrap();
        let dataset = build(
            "unit",
            &labels(),
            &SearchObservation {
                pairs: vec![observed("Tiny.same_left", "Tiny.same_right", 1)],
                visible_groups_found: 1,
                visible_groups_total: 1,
                scoring: lean_dup_search::SearchScoringSummary::new(
                    lean_dup_search::SearchScoringVariant::SymbolicOnly,
                ),
                semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                semantic_obligation_yield: Vec::new(),
                retrieval: SearchRetrievalObservation::default(),
                embedding_documents: lean_dup_search::SearchEmbeddingDocuments::default(),
            },
        );

        let artifact = write_default_artifact(temp.path(), &dataset).unwrap();
        assert_eq!(
            artifact,
            std::path::PathBuf::from("target/search-quality/unit-dataset.json")
        );
        let json = std::fs::read_to_string(temp.path().join(artifact)).unwrap();
        assert!(json.contains("\"schema_version\": \"lean-dup.search-dataset.v1\""));
        assert!(json.contains("\"match_class\": \"exact-theorem-duplicate\""));
        assert!(!json.contains("/Users/"));
    }

    fn labels() -> GoldLabels {
        let pair = GoldPair::new("Tiny.same_left", "Tiny.same_right");
        GoldLabels {
            suite: "unit".to_owned(),
            positives: FxHashSet::from_iter([pair.clone()]),
            hard_negatives: FxHashSet::default(),
            typed_pairs: vec![TypedGoldLabel {
                pair,
                polarity: LabelPolarity::Positive,
                match_class: MatchClass::ExactTheoremDuplicate,
                expected_stage_visibility: ExpectedStageVisibility::Visible,
                adjudication_source: AdjudicationSource::FixtureIntent,
                confidence: LabelConfidence::High,
                semantic_verification_required: true,
                static_evidence_acceptable: true,
            }],
            label_facts: Vec::new(),
        }
    }

    fn observed(left: &str, right: &str, rank: usize) -> SearchObservedPair {
        SearchObservedPair {
            left: left.to_owned(),
            right: right.to_owned(),
            generated: true,
            symbolic_generated: true,
            vector_generated: false,
            merged_generated: true,
            ranked: true,
            generation_policy: "local_duplicate_audit".to_owned(),
            rank: Some(rank),
            shown: rank == 1,
            left_content_hash: None,
            right_content_hash: None,
            vector_rank: None,
            origin: "workspace".to_owned(),
            feature_families: vec!["statement_fingerprint".to_owned()],
            survived_shown_filter: rank == 1,
            features: SearchPairFeatures {
                retrieval_feature_families: vec!["statement_fingerprint".to_owned()],
                declaration_kinds: vec!["theorem".to_owned()],
                evidence_mode: SearchEvidenceMode::Local,
                vector_evidence: None,
                structural_fingerprint_families: vec!["statement_fingerprint".to_owned()],
                role_overlap: Vec::new(),
                module_relation: SearchModuleRelation::SameModule {
                    module: "Tiny".to_owned(),
                },
                semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                semantic_evidence_state: SearchSemanticEvidenceState::NotRun,
                semantic_obligations: Vec::new(),
                cheap_blockers: Vec::new(),
            },
            scoring: lean_dup_search::SearchPairScoring {
                version: "lean-dup.symbolic-scorer.v1",
                variant: lean_dup_search::SearchScoringVariant::SymbolicOnly,
                total_score: 100.0,
                component_scores: std::collections::BTreeMap::from([("statement_fingerprint".to_owned(), 100.0)]),
            },
        }
    }
}
