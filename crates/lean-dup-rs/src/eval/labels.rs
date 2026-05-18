use rustc_hash::FxHashSet;
use serde::Deserialize;

use crate::cli::EvalSuite;
use crate::error::Result;
use crate::eval::scoring::GoldPair;

/// Gold duplicate and non-duplicate labels for one evaluation corpus.
///
/// Labels identify declarations by stable display names such as qualified Lean
/// names. Corpus files may group related declarations as clusters; callers of
/// this module receive normalized unordered pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoldLabels {
    pub(crate) suite: String,
    pub(crate) positives: FxHashSet<GoldPair>,
    pub(crate) hard_negatives: FxHashSet<GoldPair>,
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

pub(crate) fn load_builtin(suite: EvalSuite) -> Result<GoldLabels> {
    let json = match suite {
        EvalSuite::Default => include_str!("../../eval-data/default.json"),
        EvalSuite::HardNegatives => include_str!("../../eval-data/hard-negatives.json"),
        EvalSuite::KanproofsInternal => include_str!("../../eval-data/kanproofs-internal.json"),
        EvalSuite::KanproofsMathlib => include_str!("../../eval-data/kanproofs-mathlib.json"),
        EvalSuite::ProductionGate => {
            return Err(crate::error::Error::Eval {
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
    hard_negatives.retain(|pair| !positives.contains(pair));
    Ok(GoldLabels {
        suite: file.suite,
        positives,
        hard_negatives,
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
    use super::parse;
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
    }

    #[test]
    fn direct_hard_negative_pairs_are_dropped_only_when_positive() {
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
}
