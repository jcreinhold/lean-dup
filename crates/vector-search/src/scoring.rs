use std::collections::{BTreeMap, BTreeSet};

use lean_dup_eval::{CountMetric, GoldPair, ObservedCandidateSource, ObservedPair, ObservedRun, TimingMetrics};
use lean_dup_search::{SearchObservation, SearchObservedPair};
use serde::Serialize;

use crate::candidates::{VectorCandidate, VectorCandidateSummary};

pub(crate) const VECTOR_FEATURE_VERSION: &str = "lean-dup.vector-evidence.v2";
const VECTOR_VISIBLE_SCORE: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VectorScorerVariant {
    SymbolicOnly,
    VectorEvidenceOnly,
    SymbolicPlusVector,
}

impl VectorScorerVariant {
    pub(crate) fn all() -> [Self; 3] {
        [Self::SymbolicOnly, Self::VectorEvidenceOnly, Self::SymbolicPlusVector]
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::SymbolicOnly => "symbolic-only",
            Self::VectorEvidenceOnly => "vector-evidence-only",
            Self::SymbolicPlusVector => "symbolic-plus-vector",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorPair {
    pub(crate) left: String,
    pub(crate) right: String,
    pub(crate) left_hash: String,
    pub(crate) right_hash: String,
    pub(crate) symbolic_generated: bool,
    pub(crate) vector_generated: bool,
    pub(crate) merged_generated: bool,
    pub(crate) base_ranked: bool,
    pub(crate) base_visible: bool,
    pub(crate) generation_policies: Vec<String>,
    pub(crate) feature_families: Vec<String>,
    pub(crate) symbolic_rank: Option<usize>,
    pub(crate) vector_rank: Option<usize>,
    pub(crate) vector_score: Option<f32>,
    pub(crate) symbolic_score: f64,
    pub(crate) origin: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RankedVectorPair {
    pub(crate) pair: VectorPair,
    pub(crate) ranked: bool,
    pub(crate) visible: bool,
    pub(crate) rank: Option<usize>,
    pub(crate) total_score: f64,
    pub(crate) component_scores: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VisibleGroups {
    pub(crate) found: usize,
    pub(crate) total: usize,
}

pub(crate) fn merge_pairs(symbolic: &SearchObservation, vector_candidates: Vec<VectorCandidate>) -> Vec<VectorPair> {
    let mut pairs = BTreeMap::<GoldPair, VectorPair>::new();
    for pair in &symbolic.pairs {
        let key = GoldPair::new(pair.left.clone(), pair.right.clone());
        pairs.insert(key, symbolic_pair(pair));
    }
    for candidate in vector_candidates {
        let key = GoldPair::new(
            candidate.anchor_name.clone(),
            candidate.declaration.qualified_name.clone(),
        );
        pairs
            .entry(key)
            .and_modify(|pair| update_with_vector(pair, &candidate))
            .or_insert_with(|| vector_pair(candidate));
    }
    pairs.into_values().collect()
}

pub(crate) fn rank_pairs(pairs: &[VectorPair], variant: VectorScorerVariant) -> (Vec<RankedVectorPair>, VisibleGroups) {
    let mut ranked = pairs
        .iter()
        .cloned()
        .map(|pair| score_pair(pair, variant))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .total_score
            .partial_cmp(&left.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.pair
                    .vector_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&right.pair.vector_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| {
                left.pair
                    .symbolic_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&right.pair.symbolic_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| left.pair.left.cmp(&right.pair.left))
            .then_with(|| left.pair.right.cmp(&right.pair.right))
    });
    let mut next_rank = 1;
    for pair in &mut ranked {
        if pair.ranked {
            pair.rank = Some(next_rank);
            next_rank += 1;
        }
    }
    let total = ranked
        .iter()
        .filter(|pair| pair.ranked)
        .map(|pair| pair.pair.left.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let found = ranked
        .iter()
        .filter(|pair| pair.visible)
        .map(|pair| pair.pair.left.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    (ranked, VisibleGroups { found, total })
}

pub(crate) fn observed_run(
    suite: &str,
    pairs: &[RankedVectorPair],
    visible_groups: &VisibleGroups,
    timings: TimingMetrics,
    peak_memory_bytes: Option<u64>,
) -> ObservedRun {
    ObservedRun {
        suite: suite.to_owned(),
        pairs: pairs
            .iter()
            .map(|pair| ObservedPair {
                pair: GoldPair::new(pair.pair.left.clone(), pair.pair.right.clone()),
                generated: pair.pair.merged_generated,
                symbolic_generated: pair.pair.symbolic_generated,
                merged_generated: pair.pair.merged_generated,
                ranked: pair.ranked,
                generation_policy: pair.pair.generation_policies.join("+"),
                rank: pair.rank,
                shown: pair.visible,
                origin: pair.pair.origin.clone(),
                feature_families: pair.pair.feature_families.clone(),
                candidate_sources: observed_candidate_sources(&pair.pair),
                survived_shown_filter: pair.visible,
            })
            .collect(),
        candidate_losses: Vec::new(),
        visible_groups: CountMetric {
            found: visible_groups.found,
            total: visible_groups.total,
        },
        probe_unavailable: CountMetric { found: 0, total: 0 },
        semantic_verification: Default::default(),
        timings,
        peak_memory_bytes,
    }
}

fn observed_candidate_sources(pair: &VectorPair) -> Vec<ObservedCandidateSource> {
    let mut sources = Vec::new();
    if pair.symbolic_generated {
        sources.push(ObservedCandidateSource {
            source_id: "symbolic-retrieval".to_owned(),
            source_family: "symbolic".to_owned(),
            pair_id: stable_pair_id(&pair.left_hash, &pair.right_hash),
            left_declaration_id: pair.left_hash.clone(),
            right_declaration_id: pair.right_hash.clone(),
            origin: pair.origin.clone(),
            generation_rank: pair.symbolic_rank,
            top_k_status: if pair.symbolic_rank.is_some() {
                "selected"
            } else {
                "generated-not-selected"
            }
            .to_owned(),
            top_k_saturated: false,
            feature_families: pair.feature_families.clone(),
        });
    }
    if pair.vector_generated {
        sources.push(ObservedCandidateSource {
            source_id: "vector-nearest-neighbor".to_owned(),
            source_family: "vector".to_owned(),
            pair_id: stable_pair_id(&pair.left_hash, &pair.right_hash),
            left_declaration_id: pair.left_hash.clone(),
            right_declaration_id: pair.right_hash.clone(),
            origin: pair.origin.clone(),
            generation_rank: pair.vector_rank,
            top_k_status: if pair.vector_rank.is_some() {
                "selected"
            } else {
                "generated-not-selected"
            }
            .to_owned(),
            top_k_saturated: false,
            feature_families: vec!["vector_similarity".to_owned()],
        });
    }
    sources
}

fn stable_pair_id(left: &str, right: &str) -> String {
    if left <= right {
        format!("{left}::{right}")
    } else {
        format!("{right}::{left}")
    }
}

pub(crate) fn top_k_saturation(summary: &VectorCandidateSummary) -> CountMetric {
    CountMetric {
        found: if summary.top_k_saturated {
            summary.query_declaration_count
        } else {
            0
        },
        total: summary.query_declaration_count,
    }
}

fn score_pair(pair: VectorPair, variant: VectorScorerVariant) -> RankedVectorPair {
    let mut components = BTreeMap::new();
    match variant {
        VectorScorerVariant::SymbolicOnly => {
            if pair.base_ranked {
                components.insert("symbolic".to_owned(), pair.symbolic_score);
            }
            RankedVectorPair {
                ranked: pair.base_ranked,
                visible: pair.base_visible,
                rank: pair.symbolic_rank,
                total_score: pair.symbolic_score,
                component_scores: components,
                pair,
            }
        }
        VectorScorerVariant::VectorEvidenceOnly => {
            let vector_score = vector_component(&pair);
            if vector_score > 0.0 {
                components.insert("vector_rank".to_owned(), vector_score);
            }
            RankedVectorPair {
                ranked: pair.vector_generated && vector_score > 0.0,
                visible: pair.vector_generated && vector_score >= VECTOR_VISIBLE_SCORE,
                rank: None,
                total_score: vector_score,
                component_scores: components,
                pair,
            }
        }
        VectorScorerVariant::SymbolicPlusVector => {
            let vector_score = vector_component(&pair);
            if pair.symbolic_score > 0.0 {
                components.insert("symbolic".to_owned(), pair.symbolic_score);
            }
            if vector_score > 0.0 {
                components.insert("vector_rank".to_owned(), vector_score);
            }
            let total_score = pair.symbolic_score + vector_score;
            RankedVectorPair {
                ranked: pair.merged_generated && total_score > 0.0,
                visible: pair.merged_generated && total_score >= VECTOR_VISIBLE_SCORE,
                rank: None,
                total_score,
                component_scores: components,
                pair,
            }
        }
    }
}

fn vector_component(pair: &VectorPair) -> f64 {
    let rank = pair.vector_rank.unwrap_or(usize::MAX);
    if rank == usize::MAX {
        return 0.0;
    }
    let rank_component = 32.0 / rank as f64;
    let score_component = f64::from(pair.vector_score.unwrap_or(0.0).max(0.0)) * 16.0;
    rank_component + score_component
}

fn symbolic_pair(pair: &SearchObservedPair) -> VectorPair {
    VectorPair {
        left: pair.left.clone(),
        right: pair.right.clone(),
        left_hash: declaration_hash(&pair.left),
        right_hash: declaration_hash(&pair.right),
        symbolic_generated: pair.symbolic_generated,
        vector_generated: false,
        merged_generated: pair.merged_generated,
        base_ranked: pair.ranked,
        base_visible: pair.shown,
        generation_policies: vec![pair.generation_policy.clone()],
        feature_families: pair.feature_families.clone(),
        symbolic_rank: pair.rank,
        vector_rank: None,
        vector_score: None,
        symbolic_score: pair.scoring.total_score,
        origin: pair.origin.clone(),
    }
}

fn update_with_vector(pair: &mut VectorPair, candidate: &VectorCandidate) {
    pair.vector_generated = true;
    pair.merged_generated = pair.symbolic_generated || pair.vector_generated;
    pair.left_hash = candidate.anchor_content_hash.clone();
    pair.right_hash = candidate.declaration_content_hash.clone();
    pair.origin = candidate.declaration.origin.clone();
    if !pair
        .generation_policies
        .iter()
        .any(|policy| policy == "vector-candidate-generation")
    {
        pair.generation_policies.push("vector-candidate-generation".to_owned());
    }
    if !pair.feature_families.iter().any(|family| family == "vector_similarity") {
        pair.feature_families.push("vector_similarity".to_owned());
    }
    if pair.vector_rank.is_none_or(|rank| candidate.rank < rank) {
        pair.vector_rank = Some(candidate.rank);
    }
    if pair.vector_score.is_none_or(|score| candidate.score > score) {
        pair.vector_score = Some(candidate.score);
    }
}

fn vector_pair(candidate: VectorCandidate) -> VectorPair {
    VectorPair {
        left: candidate.anchor_name,
        right: candidate.declaration.qualified_name.clone(),
        left_hash: candidate.anchor_content_hash,
        right_hash: candidate.declaration_content_hash,
        symbolic_generated: false,
        vector_generated: true,
        merged_generated: true,
        base_ranked: false,
        base_visible: false,
        generation_policies: vec!["vector-candidate-generation".to_owned()],
        feature_families: vec!["vector_similarity".to_owned()],
        symbolic_rank: None,
        vector_rank: Some(candidate.rank),
        vector_score: Some(candidate.score),
        symbolic_score: 0.0,
        origin: candidate.declaration.origin,
    }
}

fn declaration_hash(name: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest as _;
    hasher.update(name.as_bytes());
    crate::documents::hex_bytes(&hasher.finalize())
}
