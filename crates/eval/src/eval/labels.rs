use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;

use crate::EvalSuite;
use crate::eval::scoring::GoldPair;
use lean_dup_report::{Error, Result};

/// Gold duplicate and non-duplicate labels for one evaluation corpus.
///
/// Labels identify declarations by stable display names such as qualified Lean
/// names. Corpus files may group related declarations as clusters; callers of
/// this module receive normalized unordered pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldLabels {
    pub suite: String,
    pub positives: FxHashSet<GoldPair>,
    pub hard_negatives: FxHashSet<GoldPair>,
    pub typed_pairs: Vec<TypedGoldLabel>,
}

/// A task-specific adjudication for one unordered declaration pair.
///
/// Scoring still consumes the normalized positive and hard-negative sets. This
/// metadata records why a pair is labeled so later search-quality stages can
/// measure by match class without exposing label-file layout to callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedGoldLabel {
    pub pair: GoldPair,
    pub polarity: LabelPolarity,
    pub match_class: MatchClass,
    pub expected_stage_visibility: ExpectedStageVisibility,
    pub adjudication_source: AdjudicationSource,
    pub confidence: LabelConfidence,
    pub semantic_verification_required: bool,
    pub static_evidence_acceptable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelPolarity {
    Positive,
    HardNegative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchClass {
    ExactTheoremDuplicate,
    BinderPermutationDuplicate,
    ReducibleDefinitionDuplicate,
    ReplacementCandidate,
    SpecializationGeneralization,
    LocalCleanupDuplicate,
    StaticStructuralSimilarity,
    NonActionableRelatedTheorem,
    HardNegative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedStageVisibility {
    Candidate,
    Ranked,
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdjudicationSource {
    FixtureIntent,
    ManualInspection,
    Prompt27Evidence,
    PythonEraRegression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Deserialize)]
struct LabelFile {
    suite: String,
    #[serde(default)]
    positive_clusters: Vec<LabelCluster>,
    #[serde(default)]
    positive_pairs: Vec<LabelPair>,
    #[serde(default)]
    hard_negative_clusters: Vec<LabelCluster>,
    #[serde(default)]
    hard_negative_pairs: Vec<LabelPair>,
    #[serde(default)]
    typed_pairs: Vec<RawTypedPair>,
}

#[derive(Debug, Deserialize)]
struct LabelCluster {
    members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LabelPair {
    left: String,
    right: String,
}

#[derive(Debug, Deserialize)]
struct RawTypedPair {
    left: String,
    right: String,
    polarity: Option<LabelPolarity>,
    match_class: Option<MatchClass>,
    expected_stage_visibility: Option<ExpectedStageVisibility>,
    adjudication_source: Option<AdjudicationSource>,
    confidence: Option<LabelConfidence>,
    semantic_verification_required: Option<bool>,
    static_evidence_acceptable: Option<bool>,
}

pub fn load_builtin(suite: EvalSuite) -> Result<GoldLabels> {
    let json = match suite {
        EvalSuite::Default => include_str!("../../eval-data/default.json"),
        EvalSuite::HardNegatives => include_str!("../../eval-data/hard-negatives.json"),
        EvalSuite::KanproofsInternal => include_str!("../../eval-data/kanproofs-internal.json"),
        EvalSuite::KanproofsMathlib => include_str!("../../eval-data/kanproofs-mathlib.json"),
        EvalSuite::ProductionGate => {
            return Err(lean_dup_report::Error::Eval {
                message: "production-gate is an aggregate suite without one label file".to_owned(),
            });
        }
    };
    parse(json)
}

fn parse(json: &str) -> Result<GoldLabels> {
    let file: LabelFile = serde_json::from_str(json)?;
    let mut positives = expand_clusters(&file.positive_clusters);
    positives.extend(expand_pairs(&file.positive_pairs));
    let mut hard_negatives = expand_clusters(&file.hard_negative_clusters);
    hard_negatives.extend(expand_pairs(&file.hard_negative_pairs));
    let typed_pairs = validate_typed_pairs(file.typed_pairs)?;
    for typed in &typed_pairs {
        match typed.polarity {
            LabelPolarity::Positive => {
                positives.insert(typed.pair.clone());
            }
            LabelPolarity::HardNegative => {
                hard_negatives.insert(typed.pair.clone());
            }
        }
    }
    hard_negatives.retain(|pair| !positives.contains(pair));
    Ok(GoldLabels {
        suite: file.suite,
        positives,
        hard_negatives,
        typed_pairs,
    })
}

fn validate_typed_pairs(raw_pairs: Vec<RawTypedPair>) -> Result<Vec<TypedGoldLabel>> {
    let mut labels = Vec::with_capacity(raw_pairs.len());
    let mut seen: FxHashMap<GoldPair, (LabelPolarity, MatchClass)> = FxHashMap::default();
    for raw in raw_pairs {
        let pair = GoldPair::new(raw.left, raw.right);
        let label = TypedGoldLabel {
            pair: pair.clone(),
            polarity: required(raw.polarity, "polarity", &pair)?,
            match_class: required(raw.match_class, "match_class", &pair)?,
            expected_stage_visibility: required(raw.expected_stage_visibility, "expected_stage_visibility", &pair)?,
            adjudication_source: required(raw.adjudication_source, "adjudication_source", &pair)?,
            confidence: required(raw.confidence, "confidence", &pair)?,
            semantic_verification_required: required(
                raw.semantic_verification_required,
                "semantic_verification_required",
                &pair,
            )?,
            static_evidence_acceptable: required(raw.static_evidence_acceptable, "static_evidence_acceptable", &pair)?,
        };
        if let Some((polarity, match_class)) = seen.get(&label.pair)
            && (*polarity != label.polarity || *match_class != label.match_class)
        {
            return Err(Error::Eval {
                message: format!(
                    "contradictory typed labels for {} / {}",
                    label.pair.left, label.pair.right
                ),
            });
        }
        seen.insert(label.pair.clone(), (label.polarity, label.match_class));
        labels.push(label);
    }
    Ok(labels)
}

fn required<T>(value: Option<T>, field: &'static str, pair: &GoldPair) -> Result<T> {
    value.ok_or_else(|| Error::Eval {
        message: format!(
            "typed label for {} / {} is missing required field `{field}`",
            pair.left, pair.right
        ),
    })
}

fn expand_clusters(clusters: &[LabelCluster]) -> FxHashSet<GoldPair> {
    let mut pairs = FxHashSet::default();
    for cluster in clusters {
        for left_index in 0..cluster.members.len() {
            for right_index in left_index + 1..cluster.members.len() {
                pairs.insert(GoldPair::new(
                    cluster.members[left_index].clone(),
                    cluster.members[right_index].clone(),
                ));
            }
        }
    }
    pairs
}

fn expand_pairs(pairs: &[LabelPair]) -> impl Iterator<Item = GoldPair> + '_ {
    pairs
        .iter()
        .map(|pair| GoldPair::new(pair.left.clone(), pair.right.clone()))
}

#[cfg(test)]
mod tests {
    use super::{AdjudicationSource, ExpectedStageVisibility, LabelConfidence, LabelPolarity, MatchClass, parse};
    use crate::eval::scoring::GoldPair;

    #[test]
    fn cluster_expansion_is_direction_insensitive() {
        let labels = parse(
            r#"{
              "suite": "unit",
              "positive_clusters": [{"id": "p", "members": ["B", "A", "C"]}],
              "positive_pairs": [{"left": "Q", "right": "P"}],
              "hard_negative_clusters": [{"id": "n", "members": ["D", "A"]}]
            }"#,
        )
        .unwrap();

        assert!(labels.positives.contains(&GoldPair::new("A", "B")));
        assert!(labels.positives.contains(&GoldPair::new("A", "C")));
        assert!(labels.positives.contains(&GoldPair::new("B", "C")));
        assert!(labels.positives.contains(&GoldPair::new("P", "Q")));
        assert!(labels.hard_negatives.contains(&GoldPair::new("A", "D")));
        assert!(labels.typed_pairs.is_empty());
    }

    #[test]
    fn legacy_hard_negative_pairs_are_dropped_only_when_positive() {
        let labels = parse(
            r#"{
              "suite": "unit",
              "positive_pairs": [{"left": "A", "right": "B"}],
              "hard_negative_pairs": [
                {"left": "B", "right": "A"},
                {"left": "A", "right": "C"}
              ]
            }"#,
        )
        .unwrap();

        assert!(!labels.hard_negatives.contains(&GoldPair::new("A", "B")));
        assert!(labels.hard_negatives.contains(&GoldPair::new("A", "C")));
    }

    #[test]
    fn typed_pairs_are_normalized_and_preserved() {
        let labels = parse(
            r#"{
              "suite": "unit",
              "typed_pairs": [{
                "left": "B",
                "right": "A",
                "polarity": "positive",
                "match_class": "exact-theorem-duplicate",
                "expected_stage_visibility": "visible",
                "adjudication_source": "fixture-intent",
                "confidence": "high",
                "semantic_verification_required": true,
                "static_evidence_acceptable": false
              }]
            }"#,
        )
        .unwrap();

        let pair = GoldPair::new("A", "B");
        assert!(labels.positives.contains(&pair));
        assert_eq!(labels.typed_pairs.len(), 1);
        assert_eq!(labels.typed_pairs[0].pair, pair);
        assert_eq!(labels.typed_pairs[0].polarity, LabelPolarity::Positive);
        assert_eq!(labels.typed_pairs[0].match_class, MatchClass::ExactTheoremDuplicate);
        assert_eq!(
            labels.typed_pairs[0].expected_stage_visibility,
            ExpectedStageVisibility::Visible
        );
        assert_eq!(
            labels.typed_pairs[0].adjudication_source,
            AdjudicationSource::FixtureIntent
        );
        assert_eq!(labels.typed_pairs[0].confidence, LabelConfidence::High);
        assert!(labels.typed_pairs[0].semantic_verification_required);
        assert!(!labels.typed_pairs[0].static_evidence_acceptable);
    }

    #[test]
    fn typed_positive_and_hard_negative_contradiction_fails() {
        let error = parse(
            r#"{
              "suite": "unit",
              "typed_pairs": [
                {
                  "left": "A",
                  "right": "B",
                  "polarity": "positive",
                  "match_class": "exact-theorem-duplicate",
                  "expected_stage_visibility": "visible",
                  "adjudication_source": "fixture-intent",
                  "confidence": "high",
                  "semantic_verification_required": true,
                  "static_evidence_acceptable": false
                },
                {
                  "left": "B",
                  "right": "A",
                  "polarity": "hard-negative",
                  "match_class": "hard-negative",
                  "expected_stage_visibility": "hidden",
                  "adjudication_source": "fixture-intent",
                  "confidence": "high",
                  "semantic_verification_required": false,
                  "static_evidence_acceptable": false
                }
              ]
            }"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("contradictory typed labels"));
    }

    #[test]
    fn typed_label_requires_adjudication_source_and_confidence() {
        let missing_source = parse(
            r#"{
              "suite": "unit",
              "typed_pairs": [{
                "left": "A",
                "right": "B",
                "polarity": "positive",
                "match_class": "exact-theorem-duplicate",
                "expected_stage_visibility": "visible",
                "semantic_verification_required": true,
                "static_evidence_acceptable": false
              }]
            }"#,
        )
        .unwrap_err();
        assert!(missing_source.to_string().contains("adjudication_source"));

        let missing_confidence = parse(
            r#"{
              "suite": "unit",
              "typed_pairs": [{
                "left": "A",
                "right": "B",
                "polarity": "positive",
                "match_class": "exact-theorem-duplicate",
                "expected_stage_visibility": "visible",
                "adjudication_source": "fixture-intent",
                "semantic_verification_required": true,
                "static_evidence_acceptable": false
              }]
            }"#,
        )
        .unwrap_err();
        assert!(missing_confidence.to_string().contains("confidence"));
    }

    #[test]
    fn builtin_default_labels_accept_legacy_and_typed_entries() {
        let labels = parse(include_str!("../../eval-data/default.json")).unwrap();

        assert!(
            labels
                .positives
                .contains(&GoldPair::new("Tiny.same_left", "Tiny.same_right"))
        );
        assert!(
            labels
                .typed_pairs
                .iter()
                .any(|label| label.match_class == MatchClass::ReducibleDefinitionDuplicate)
        );
        assert!(
            labels
                .typed_pairs
                .iter()
                .any(|label| label.match_class == MatchClass::HardNegative)
        );
    }
}
