use std::collections::BTreeSet;

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
    pub(crate) positives: BTreeSet<GoldPair>,
    pub(crate) hard_negatives: BTreeSet<GoldPair>,
}

#[derive(Debug, Deserialize)]
struct LabelFile {
    suite: String,
    positive_clusters: Vec<LabelCluster>,
    hard_negative_clusters: Vec<LabelCluster>,
}

#[derive(Debug, Deserialize)]
struct LabelCluster {
    members: Vec<String>,
}

pub(crate) fn load_builtin(suite: EvalSuite) -> Result<GoldLabels> {
    let json = match suite {
        EvalSuite::Default => include_str!("../../eval-data/default.json"),
        EvalSuite::KanproofsInternal => include_str!("../../eval-data/kanproofs-internal.json"),
        EvalSuite::KanproofsMathlib => include_str!("../../eval-data/kanproofs-mathlib.json"),
    };
    parse(json)
}

fn parse(json: &str) -> Result<GoldLabels> {
    let file: LabelFile = serde_json::from_str(json)?;
    let positives = expand_clusters(&file.positive_clusters);
    let hard_negatives = expand_clusters(&file.hard_negative_clusters)
        .difference(&positives)
        .cloned()
        .collect();
    Ok(GoldLabels {
        suite: file.suite,
        positives,
        hard_negatives,
    })
}

fn expand_clusters(clusters: &[LabelCluster]) -> BTreeSet<GoldPair> {
    let mut pairs = BTreeSet::new();
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
              "hard_negative_clusters": [{"id": "n", "members": ["D", "A"]}]
            }"#,
        )
        .unwrap();

        assert!(labels.positives.contains(&GoldPair::new("A", "B")));
        assert!(labels.positives.contains(&GoldPair::new("A", "C")));
        assert!(labels.positives.contains(&GoldPair::new("B", "C")));
        assert!(labels.hard_negatives.contains(&GoldPair::new("A", "D")));
    }
}
