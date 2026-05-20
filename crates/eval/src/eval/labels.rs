use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::EvalSuite;
use crate::eval::scoring::GoldPair;
use crate::{Error, Result};

/// Gold duplicate and non-duplicate labels for one evaluation corpus.
///
/// Labels identify declarations by stable display names such as qualified Lean
/// names. Corpus files may group same-class declarations as typed clusters;
/// callers of this module receive normalized unordered pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldLabels {
    pub suite: String,
    pub positives: FxHashSet<GoldPair>,
    pub hard_negatives: FxHashSet<GoldPair>,
    pub typed_pairs: Vec<TypedGoldLabel>,
    pub label_facts: Vec<GoldLabelFact>,
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

/// Stable label provenance for one normalized pair.
///
/// Scoring uses the positive and hard-negative sets. Artifacts use these facts
/// to explain whether a pair was directly typed or expanded from a typed
/// cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldLabelFact {
    pub pair: GoldPair,
    pub polarity: LabelPolarity,
    pub source: LabelFactSource,
    pub typed: Option<TypedGoldLabel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelFactSource {
    TypedPair,
    TypedCluster,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelPolarity {
    Positive,
    HardNegative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedStageVisibility {
    Candidate,
    Ranked,
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdjudicationSource {
    FixtureIntent,
    ManualInspection,
    Prompt27Evidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelFile {
    suite: String,
    #[serde(default)]
    typed_clusters: Vec<RawTypedCluster>,
    #[serde(default)]
    typed_pairs: Vec<RawTypedPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTypedCluster {
    id: String,
    members: Vec<String>,
    polarity: Option<LabelPolarity>,
    match_class: Option<MatchClass>,
    expected_stage_visibility: Option<ExpectedStageVisibility>,
    adjudication_source: Option<AdjudicationSource>,
    confidence: Option<LabelConfidence>,
    semantic_verification_required: Option<bool>,
    static_evidence_acceptable: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
        EvalSuite::ManualInternal => include_str!("../../eval-data/manual-internal.json"),
        EvalSuite::ManualMathlib => include_str!("../../eval-data/manual-mathlib.json"),
        EvalSuite::ProductionGate => {
            return Err(Error::Eval {
                message: "production-gate is an aggregate suite without one label file".to_owned(),
            });
        }
    };
    parse_json(json)
}

pub fn parse_json(json: &str) -> Result<GoldLabels> {
    let file: LabelFile = serde_json::from_str(json)?;
    let typed_clusters = expand_typed_clusters(file.typed_clusters)?;
    let typed_pairs = validate_typed_pairs(file.typed_pairs)?;
    let (typed_pairs, label_facts) = validate_typed_labels(typed_clusters, typed_pairs)?;
    let mut positives = FxHashSet::default();
    let mut hard_negatives = FxHashSet::default();
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
    Ok(GoldLabels {
        suite: file.suite,
        positives,
        hard_negatives,
        typed_pairs,
        label_facts,
    })
}

fn expand_typed_clusters(raw_clusters: Vec<RawTypedCluster>) -> Result<Vec<TypedGoldLabel>> {
    let mut labels = Vec::new();
    for raw in raw_clusters {
        if raw.members.len() < 2 {
            return Err(Error::Eval {
                message: format!("typed cluster `{}` must contain at least two members", raw.id),
            });
        }
        for left_index in 0..raw.members.len() {
            for right_index in left_index + 1..raw.members.len() {
                let pair = GoldPair::new(raw.members[left_index].clone(), raw.members[right_index].clone());
                labels.push(TypedGoldLabel {
                    pair: pair.clone(),
                    polarity: required(raw.polarity, "polarity", &pair)?,
                    match_class: required(raw.match_class, "match_class", &pair)?,
                    expected_stage_visibility: required(
                        raw.expected_stage_visibility,
                        "expected_stage_visibility",
                        &pair,
                    )?,
                    adjudication_source: required(raw.adjudication_source, "adjudication_source", &pair)?,
                    confidence: required(raw.confidence, "confidence", &pair)?,
                    semantic_verification_required: required(
                        raw.semantic_verification_required,
                        "semantic_verification_required",
                        &pair,
                    )?,
                    static_evidence_acceptable: required(
                        raw.static_evidence_acceptable,
                        "static_evidence_acceptable",
                        &pair,
                    )?,
                });
            }
        }
    }
    Ok(labels)
}

fn validate_typed_pairs(raw_pairs: Vec<RawTypedPair>) -> Result<Vec<TypedGoldLabel>> {
    raw_pairs
        .into_iter()
        .map(|raw| {
            let pair = GoldPair::new(raw.left, raw.right);
            Ok(TypedGoldLabel {
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
                static_evidence_acceptable: required(
                    raw.static_evidence_acceptable,
                    "static_evidence_acceptable",
                    &pair,
                )?,
            })
        })
        .collect()
}

fn validate_typed_labels(
    typed_clusters: Vec<TypedGoldLabel>,
    typed_pairs: Vec<TypedGoldLabel>,
) -> Result<(Vec<TypedGoldLabel>, Vec<GoldLabelFact>)> {
    let mut labels = Vec::with_capacity(typed_clusters.len() + typed_pairs.len());
    let mut facts = Vec::with_capacity(typed_clusters.len() + typed_pairs.len());
    let mut seen: FxHashMap<GoldPair, TypedGoldLabel> = FxHashMap::default();
    for (source, typed) in typed_clusters
        .into_iter()
        .map(|typed| (LabelFactSource::TypedCluster, typed))
        .chain(typed_pairs.into_iter().map(|typed| (LabelFactSource::TypedPair, typed)))
    {
        if let Some(previous) = seen.get(&typed.pair) {
            if previous != &typed {
                return Err(Error::Eval {
                    message: format!(
                        "contradictory typed labels for {} / {}",
                        typed.pair.left, typed.pair.right
                    ),
                });
            }
            facts.push(GoldLabelFact {
                pair: typed.pair.clone(),
                polarity: typed.polarity,
                source,
                typed: Some(typed),
            });
            continue;
        }
        seen.insert(typed.pair.clone(), typed.clone());
        facts.push(GoldLabelFact {
            pair: typed.pair.clone(),
            polarity: typed.polarity,
            source,
            typed: Some(typed.clone()),
        });
        labels.push(typed);
    }
    Ok((labels, facts))
}

fn required<T>(value: Option<T>, field: &'static str, pair: &GoldPair) -> Result<T> {
    value.ok_or_else(|| Error::Eval {
        message: format!(
            "typed label for {} / {} is missing required field `{field}`",
            pair.left, pair.right
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AdjudicationSource, ExpectedStageVisibility, LabelConfidence, LabelFactSource, LabelPolarity, MatchClass,
        parse_json,
    };
    use crate::Error;
    use crate::eval::scoring::GoldPair;

    #[test]
    fn typed_cluster_expansion_is_direction_insensitive() {
        let labels = parse_json(
            r#"{
              "suite": "unit",
              "typed_clusters": [{
                "id": "p",
                "members": ["B", "A", "C"],
                "polarity": "positive",
                "match_class": "exact-theorem-duplicate",
                "expected_stage_visibility": "visible",
                "adjudication_source": "fixture-intent",
                "confidence": "high",
                "semantic_verification_required": true,
                "static_evidence_acceptable": true
              }],
              "typed_pairs": [{
                "left": "Q",
                "right": "P",
                "polarity": "hard-negative",
                "match_class": "hard-negative",
                "expected_stage_visibility": "hidden",
                "adjudication_source": "fixture-intent",
                "confidence": "high",
                "semantic_verification_required": false,
                "static_evidence_acceptable": false
              }]
            }"#,
        )
        .unwrap();

        assert!(labels.positives.contains(&GoldPair::new("A", "B")));
        assert!(labels.positives.contains(&GoldPair::new("A", "C")));
        assert!(labels.positives.contains(&GoldPair::new("B", "C")));
        assert!(labels.hard_negatives.contains(&GoldPair::new("P", "Q")));
        assert_eq!(labels.typed_pairs.len(), 4);
        assert!(
            labels
                .label_facts
                .iter()
                .any(|fact| { fact.pair == GoldPair::new("A", "B") && fact.source == LabelFactSource::TypedCluster })
        );
    }

    #[test]
    fn legacy_label_fields_are_rejected() {
        let error = parse_json(
            r#"{
              "suite": "unit",
              "positive_pairs": [{"left": "A", "right": "B"}]
            }"#,
        )
        .unwrap_err();

        match error {
            Error::Json(source) => {
                let message = source.to_string();
                assert!(message.contains("unknown field"));
                assert!(message.contains("positive_pairs"));
            }
            other => panic!("expected JSON schema error, got {other}"),
        }
    }

    #[test]
    fn typed_pairs_are_normalized_and_preserved() {
        let labels = parse_json(
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
        let error = parse_json(
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
        let missing_source = parse_json(
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

        let missing_confidence = parse_json(
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
    fn builtin_default_labels_accept_typed_entries() {
        let labels = parse_json(include_str!("../../eval-data/default.json")).unwrap();

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
