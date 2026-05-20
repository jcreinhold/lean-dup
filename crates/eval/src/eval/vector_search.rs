use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use lean_dup_search::{
    SearchEmbeddingDocumentPolicy, SearchObservation, SearchObservedPair, SearchScoringVariant,
    SearchVectorAcquisitionPolicy, SearchVectorCandidateRequest, SearchVectorCandidateStatus,
    SearchVectorCandidateSummary, SearchVectorEligibilityPolicy, rescore_observation,
};
use serde::Serialize;

use crate::eval::labels::{GoldLabelFact, GoldLabels, LabelFactSource, LabelPolarity, TypedGoldLabel};
use crate::eval::scoring::{
    CountMetric, EvaluationMetrics, GoldPair, ObservedPair, ObservedRun, TimingMetrics, score_run,
};
use crate::{Error, Result};

pub const VECTOR_SEARCH_SCHEMA_VERSION: &str = "lean-dup.vector-search.v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorSearchRequest {
    pub model_id: String,
    pub revision: Option<String>,
    pub acquisition_policy: SearchVectorAcquisitionPolicy,
    pub model_cache_root: Option<PathBuf>,
    pub text_vector_cache_root: Option<PathBuf>,
    pub corpus_cache_root: Option<PathBuf>,
    pub document_policy: SearchEmbeddingDocumentPolicy,
    pub eligibility_policy: SearchVectorEligibilityPolicy,
}

#[derive(Debug, Clone)]
pub(crate) struct VectorSearchArtifactOutcome {
    pub status: String,
    pub artifact: PathBuf,
    pub metrics: Option<EvaluationMetrics>,
}

pub(crate) struct VectorSearchReportRun<'a> {
    pub repo_root: &'a Path,
    pub suite: &'a str,
    pub labels: &'a GoldLabels,
    pub observation: &'a SearchObservation,
    pub symbolic_baseline: &'a EvaluationMetrics,
    pub vector_metrics: &'a EvaluationMetrics,
    pub scorer_version: &'a str,
    pub k_values: &'a [usize],
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VectorSearchReport {
    pub schema_version: &'static str,
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub scorer_version: String,
    pub vector_candidates: SearchVectorCandidateSummary,
    pub symbolic_baseline: EvaluationMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search: Option<EvaluationMetrics>,
    pub vector_stage_metrics: VectorStageMetrics,
    pub scorer_variants: Vec<VectorScorerVariantReport>,
    pub pairs: Vec<VectorSearchPairReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<VectorSearchChildReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorScorerVariantReport {
    pub scorer_variant_id: String,
    pub vector_feature_version: String,
    pub metrics: EvaluationMetrics,
    pub vector_stage_metrics: VectorStageMetrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct VectorStageMetrics {
    pub vector_top_k_recall: CountMetric,
    pub vector_top_k_precision: CountMetric,
    pub top_k_saturation: CountMetric,
    pub vector_only_positives: CountMetric,
    pub vector_only_hard_negatives: CountMetric,
    pub symbolic_only_positives: CountMetric,
    pub symbolic_only_hard_negatives: CountMetric,
    pub merged_generated_recall: CountMetric,
    pub ranked_recall: CountMetric,
    pub visible_precision: CountMetric,
    pub visible_hard_negative_count: CountMetric,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct VectorSearchPairReport {
    pub left: String,
    pub right: String,
    pub left_hash: String,
    pub right_hash: String,
    pub label_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<VectorSearchLabelReport>,
    pub label_facts: Vec<VectorSearchLabelFactReport>,
    pub symbolic_generated: bool,
    pub vector_generated: bool,
    pub merged_generated: bool,
    pub ranked: bool,
    pub visible: bool,
    pub rank: Option<usize>,
    pub vector_rank: Option<usize>,
    pub vector_score: Option<f64>,
    pub generation_policies: Vec<String>,
    pub feature_families: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VectorSearchLabelReport {
    pub polarity: crate::eval::labels::LabelPolarity,
    pub match_class: crate::eval::labels::MatchClass,
    pub expected_stage_visibility: crate::eval::labels::ExpectedStageVisibility,
    pub adjudication_source: crate::eval::labels::AdjudicationSource,
    pub confidence: crate::eval::labels::LabelConfidence,
    pub semantic_verification_required: bool,
    pub static_evidence_acceptable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VectorSearchLabelFactReport {
    pub status: String,
    pub source: LabelFactSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VectorSearchChildReport {
    pub suite: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EvaluationMetrics>,
}

impl VectorSearchRequest {
    pub(crate) fn to_search_request(&self, suite: &str) -> SearchVectorCandidateRequest {
        SearchVectorCandidateRequest {
            model_id: self.model_id.clone(),
            revision: self.revision.clone(),
            acquisition_policy: self.acquisition_policy,
            model_cache_root: self.model_cache_root.clone(),
            text_vector_cache_root: self.text_vector_cache_root.clone(),
            corpus_cache_root: self
                .corpus_cache_root
                .clone()
                .unwrap_or_else(|| PathBuf::from("target/search-quality/vector-corpus").join(suite)),
            document_policy: self.document_policy,
            eligibility_policy: self.eligibility_policy,
        }
    }
}

pub(crate) fn report(run: VectorSearchReportRun<'_>) -> Result<VectorSearchArtifactOutcome> {
    let status = status_label(run.observation.retrieval.vector_candidates.status);
    let metrics = (run.observation.retrieval.vector_candidates.status == SearchVectorCandidateStatus::Ok)
        .then(|| run.vector_metrics.clone());
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: run.suite.to_owned(),
        status: status.clone(),
        reason: run.observation.retrieval.vector_candidates.reason.clone(),
        scorer_version: run.scorer_version.to_owned(),
        vector_candidates: run.observation.retrieval.vector_candidates.clone(),
        symbolic_baseline: run.symbolic_baseline.clone(),
        vector_search: metrics.clone(),
        vector_stage_metrics: vector_stage_metrics(run.labels, run.observation),
        scorer_variants: scorer_variant_reports(&run),
        pairs: pair_reports(run.labels, run.observation),
        children: Vec::new(),
    };
    let artifact = write_default_artifact(run.repo_root, &report)?;
    Ok(VectorSearchArtifactOutcome {
        status,
        artifact,
        metrics,
    })
}

pub(crate) fn aggregate(
    repo_root: &Path,
    suite: &str,
    scorer_version: &str,
    symbolic_baseline: &EvaluationMetrics,
    children: Vec<VectorSearchChildReport>,
) -> Result<VectorSearchArtifactOutcome> {
    let completed = children
        .iter()
        .filter_map(|child| child.metrics.as_ref())
        .collect::<Vec<_>>();
    let status = if completed.is_empty() {
        "skipped"
    } else if children.iter().any(|child| child.status == "failed") {
        "failed"
    } else if children.iter().any(|child| child.status == "skipped") {
        "incomplete"
    } else {
        "ok"
    };
    let report = VectorSearchReport {
        schema_version: VECTOR_SEARCH_SCHEMA_VERSION,
        suite: suite.to_owned(),
        status: status.to_owned(),
        reason: (completed.is_empty()).then(|| "no completed child vector search metrics".to_owned()),
        scorer_version: scorer_version.to_owned(),
        vector_candidates: SearchVectorCandidateSummary::default(),
        symbolic_baseline: symbolic_baseline.clone(),
        vector_search: None,
        vector_stage_metrics: VectorStageMetrics::default(),
        scorer_variants: Vec::new(),
        pairs: Vec::new(),
        children,
    };
    let artifact = write_default_artifact(repo_root, &report)?;
    Ok(VectorSearchArtifactOutcome {
        status: status.to_owned(),
        artifact,
        metrics: None,
    })
}

pub(crate) fn child_report(
    suite: String,
    status: Option<String>,
    artifact: Option<PathBuf>,
    metrics: Option<EvaluationMetrics>,
    reason: Option<String>,
) -> VectorSearchChildReport {
    VectorSearchChildReport {
        suite,
        status: status.unwrap_or_else(|| "skipped".to_owned()),
        reason,
        artifact,
        metrics,
    }
}

fn scorer_variant_reports(run: &VectorSearchReportRun<'_>) -> Vec<VectorScorerVariantReport> {
    if run.observation.retrieval.vector_candidates.status != SearchVectorCandidateStatus::Ok {
        return Vec::new();
    }
    [
        SearchScoringVariant::SymbolicOnly,
        SearchScoringVariant::VectorEvidenceOnly,
        SearchScoringVariant::SymbolicPlusVector,
    ]
    .into_iter()
    .map(|variant| {
        let observation = if variant == run.observation.scoring.variant {
            run.observation.clone()
        } else {
            rescore_observation(run.observation, variant)
        };
        let observed = ObservedRun {
            suite: run.suite.to_owned(),
            pairs: observed_pairs(&observation),
            visible_groups: CountMetric {
                found: observation.visible_groups_found,
                total: observation.visible_groups_total,
            },
            probe_unavailable: CountMetric { found: 0, total: 0 },
            semantic_verification: run.vector_metrics.stage_metrics.semantic_verification.clone(),
            timings: TimingMetrics {
                index_load_ms: run.vector_metrics.timings.index_load_ms,
                retrieval_ms: 0,
                probe_ms: run.vector_metrics.timings.probe_ms,
                total_ms: 0,
            },
            peak_memory_bytes: run.vector_metrics.peak_memory_bytes,
        };
        let metrics = score_run(run.labels, &observed, run.k_values);
        VectorScorerVariantReport {
            scorer_variant_id: variant.label().to_owned(),
            vector_feature_version: vector_feature_version(&observation),
            vector_stage_metrics: vector_stage_metrics(run.labels, &observation),
            metrics,
        }
    })
    .collect()
}

fn vector_feature_version(observation: &SearchObservation) -> String {
    observation
        .pairs
        .iter()
        .filter_map(|pair| pair.features.vector_evidence.as_ref())
        .map(|evidence| evidence.version.clone())
        .next()
        .unwrap_or_else(|| "none".to_owned())
}

fn observed_pairs(output: &SearchObservation) -> Vec<ObservedPair> {
    output
        .pairs
        .iter()
        .map(|pair| ObservedPair {
            pair: GoldPair::new(pair.left.clone(), pair.right.clone()),
            generated: pair.generated,
            symbolic_generated: pair.symbolic_generated,
            vector_generated: pair.vector_generated,
            merged_generated: pair.merged_generated,
            ranked: pair.ranked,
            generation_policy: pair.generation_policy.clone(),
            rank: pair.rank,
            shown: pair.shown,
            origin: pair.origin.clone(),
            feature_families: pair.feature_families.clone(),
            survived_shown_filter: pair.survived_shown_filter,
        })
        .collect()
}

fn pair_reports(labels: &GoldLabels, observation: &SearchObservation) -> Vec<VectorSearchPairReport> {
    let document_hashes = document_hashes(observation);
    let label_facts = label_facts_by_pair(labels);
    let typed_by_pair = labels
        .typed_pairs
        .iter()
        .map(|label| (label.pair.clone(), label))
        .collect::<BTreeMap<_, _>>();
    let mut rows_by_pair = BTreeMap::<GoldPair, PairAccumulator>::new();
    for pair in &observation.pairs {
        let key = GoldPair::new(pair.left.clone(), pair.right.clone());
        rows_by_pair
            .entry(key.clone())
            .or_insert_with(|| PairAccumulator::new(key.clone(), &document_hashes))
            .add(pair);
    }
    rows_by_pair
        .into_iter()
        .map(|(key, row)| {
            let facts = label_facts.get(&key).cloned().unwrap_or_default();
            let typed = typed_by_pair.get(&key).copied();
            row.finish(
                typed.map(label_report),
                facts.iter().map(label_fact_report).collect(),
                label_status(labels, &key, &facts),
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct PairAccumulator {
    key: GoldPair,
    left_hash: String,
    right_hash: String,
    symbolic_generated: bool,
    vector_generated: bool,
    merged_generated: bool,
    ranked: bool,
    visible: bool,
    rank: Option<usize>,
    vector_rank: Option<usize>,
    vector_score: Option<f64>,
    generation_policies: BTreeSet<String>,
    feature_families: BTreeSet<String>,
}

impl PairAccumulator {
    fn new(key: GoldPair, document_hashes: &BTreeMap<String, String>) -> Self {
        let left_hash = document_hashes
            .get(&key.left)
            .cloned()
            .unwrap_or_else(|| declaration_hash(&key.left));
        let right_hash = document_hashes
            .get(&key.right)
            .cloned()
            .unwrap_or_else(|| declaration_hash(&key.right));
        Self {
            key,
            left_hash,
            right_hash,
            symbolic_generated: false,
            vector_generated: false,
            merged_generated: false,
            ranked: false,
            visible: false,
            rank: None,
            vector_rank: None,
            vector_score: None,
            generation_policies: BTreeSet::new(),
            feature_families: BTreeSet::new(),
        }
    }

    fn add(&mut self, pair: &SearchObservedPair) {
        self.symbolic_generated |= pair.symbolic_generated;
        self.vector_generated |= pair.vector_generated;
        self.merged_generated |= pair.merged_generated;
        self.ranked |= pair.ranked;
        self.visible |= pair.shown;
        self.rank = min_optional(self.rank, pair.rank);
        self.vector_rank = min_optional(self.vector_rank, pair.vector_rank);
        self.vector_score = max_optional(self.vector_score, pair.vector_score);
        if self.left_hash == declaration_hash(&self.key.left)
            && let Some(hash) = oriented_hash(
                &self.key.left,
                &pair.left,
                &pair.right,
                &pair.left_content_hash,
                &pair.right_content_hash,
            )
        {
            self.left_hash = hash;
        }
        if self.right_hash == declaration_hash(&self.key.right)
            && let Some(hash) = oriented_hash(
                &self.key.right,
                &pair.left,
                &pair.right,
                &pair.left_content_hash,
                &pair.right_content_hash,
            )
        {
            self.right_hash = hash;
        }
        self.generation_policies.insert(pair.generation_policy.clone());
        self.feature_families.extend(pair.feature_families.iter().cloned());
    }

    fn finish(
        self,
        label: Option<VectorSearchLabelReport>,
        label_facts: Vec<VectorSearchLabelFactReport>,
        label_status: String,
    ) -> VectorSearchPairReport {
        VectorSearchPairReport {
            left: self.key.left,
            right: self.key.right,
            left_hash: self.left_hash,
            right_hash: self.right_hash,
            label_status,
            label,
            label_facts,
            symbolic_generated: self.symbolic_generated,
            vector_generated: self.vector_generated,
            merged_generated: self.merged_generated,
            ranked: self.ranked,
            visible: self.visible,
            rank: self.rank,
            vector_rank: self.vector_rank,
            vector_score: self.vector_score,
            generation_policies: self.generation_policies.into_iter().collect(),
            feature_families: self.feature_families.into_iter().collect(),
        }
    }
}

fn label_report(label: &TypedGoldLabel) -> VectorSearchLabelReport {
    VectorSearchLabelReport {
        polarity: label.polarity,
        match_class: label.match_class,
        expected_stage_visibility: label.expected_stage_visibility,
        adjudication_source: label.adjudication_source,
        confidence: label.confidence,
        semantic_verification_required: label.semantic_verification_required,
        static_evidence_acceptable: label.static_evidence_acceptable,
    }
}

fn label_fact_report(fact: &GoldLabelFact) -> VectorSearchLabelFactReport {
    VectorSearchLabelFactReport {
        status: fact_status(fact).to_owned(),
        source: fact.source,
        skipped_reason: fact.skipped_reason,
    }
}

fn label_status(labels: &GoldLabels, pair: &GoldPair, facts: &[GoldLabelFact]) -> String {
    if labels.positives.contains(pair) {
        if facts
            .iter()
            .any(|fact| fact.polarity == Some(LabelPolarity::Positive) && is_expanded_source(fact.source))
            && !facts
                .iter()
                .any(|fact| fact.polarity == Some(LabelPolarity::Positive) && !is_expanded_source(fact.source))
        {
            "expanded-positive".to_owned()
        } else {
            "positive".to_owned()
        }
    } else if labels.hard_negatives.contains(pair) {
        if facts
            .iter()
            .any(|fact| fact.polarity == Some(LabelPolarity::HardNegative) && is_expanded_source(fact.source))
            && !facts
                .iter()
                .any(|fact| fact.polarity == Some(LabelPolarity::HardNegative) && !is_expanded_source(fact.source))
        {
            "expanded-hard-negative".to_owned()
        } else {
            "hard-negative".to_owned()
        }
    } else if facts.iter().any(|fact| fact.skipped_reason.is_some()) {
        "skipped".to_owned()
    } else {
        "unlabeled".to_owned()
    }
}

fn fact_status(fact: &GoldLabelFact) -> &'static str {
    match (fact.polarity, fact.source) {
        (Some(LabelPolarity::Positive), LabelFactSource::ExpandedPositiveCluster) => "expanded-positive",
        (Some(LabelPolarity::HardNegative), LabelFactSource::ExpandedHardNegativeCluster) => "expanded-hard-negative",
        (Some(LabelPolarity::Positive), _) => "positive",
        (Some(LabelPolarity::HardNegative), _) => "hard-negative",
        (None, _) => "skipped",
    }
}

fn is_expanded_source(source: LabelFactSource) -> bool {
    matches!(
        source,
        LabelFactSource::ExpandedPositiveCluster | LabelFactSource::ExpandedHardNegativeCluster
    )
}

fn label_facts_by_pair(labels: &GoldLabels) -> BTreeMap<GoldPair, Vec<GoldLabelFact>> {
    let mut facts = BTreeMap::<GoldPair, Vec<GoldLabelFact>>::new();
    for fact in &labels.label_facts {
        facts.entry(fact.pair.clone()).or_default().push(fact.clone());
    }
    for facts in facts.values_mut() {
        facts.sort_by(|left, right| {
            fact_status(left)
                .cmp(fact_status(right))
                .then_with(|| format!("{:?}", left.source).cmp(&format!("{:?}", right.source)))
        });
    }
    facts
}

fn document_hashes(observation: &SearchObservation) -> BTreeMap<String, String> {
    observation
        .embedding_documents
        .documents
        .iter()
        .map(|document| (document.declaration_name.clone(), document.content_hash.clone()))
        .collect()
}

fn oriented_hash(
    requested: &str,
    left: &str,
    right: &str,
    left_hash: &Option<String>,
    right_hash: &Option<String>,
) -> Option<String> {
    if requested == left {
        left_hash.clone()
    } else if requested == right {
        right_hash.clone()
    } else {
        None
    }
}

fn min_optional(left: Option<usize>, right: Option<usize>) -> Option<usize> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn max_optional(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn declaration_hash(name: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

fn vector_stage_metrics(labels: &GoldLabels, observation: &SearchObservation) -> VectorStageMetrics {
    let rows = pair_reports(labels, observation);
    let vector_generated = rows
        .iter()
        .filter(|row| row.vector_generated)
        .map(row_pair)
        .collect::<BTreeSet<_>>();
    let symbolic_generated = rows
        .iter()
        .filter(|row| row.symbolic_generated)
        .map(row_pair)
        .collect::<BTreeSet<_>>();
    let merged_generated = rows
        .iter()
        .filter(|row| row.merged_generated)
        .map(row_pair)
        .collect::<BTreeSet<_>>();
    let ranked = rows
        .iter()
        .filter(|row| row.ranked)
        .map(row_pair)
        .collect::<BTreeSet<_>>();
    let visible = rows
        .iter()
        .filter(|row| row.visible)
        .map(row_pair)
        .collect::<BTreeSet<_>>();
    let vector_only = vector_generated
        .difference(&symbolic_generated)
        .cloned()
        .collect::<BTreeSet<_>>();
    let symbolic_only = symbolic_generated
        .difference(&vector_generated)
        .cloned()
        .collect::<BTreeSet<_>>();
    let saturation = if observation.retrieval.vector_candidates.top_k_saturated {
        observation.retrieval.vector_candidates.query_declaration_count
    } else {
        0
    };

    VectorStageMetrics {
        vector_top_k_recall: count_labeled(&labels.positives, &vector_generated),
        vector_top_k_precision: CountMetric {
            found: vector_generated
                .iter()
                .filter(|pair| labels.positives.contains(*pair))
                .count(),
            total: vector_generated.len(),
        },
        top_k_saturation: CountMetric {
            found: saturation,
            total: observation.retrieval.vector_candidates.query_declaration_count,
        },
        vector_only_positives: count_labeled(&labels.positives, &vector_only),
        vector_only_hard_negatives: count_labeled(&labels.hard_negatives, &vector_only),
        symbolic_only_positives: count_labeled(&labels.positives, &symbolic_only),
        symbolic_only_hard_negatives: count_labeled(&labels.hard_negatives, &symbolic_only),
        merged_generated_recall: count_labeled(&labels.positives, &merged_generated),
        ranked_recall: count_labeled(&labels.positives, &ranked),
        visible_precision: CountMetric {
            found: visible.iter().filter(|pair| labels.positives.contains(*pair)).count(),
            total: visible.len(),
        },
        visible_hard_negative_count: count_labeled(&labels.hard_negatives, &visible),
    }
}

fn row_pair(row: &VectorSearchPairReport) -> GoldPair {
    GoldPair::new(row.left.clone(), row.right.clone())
}

fn count_labeled(labels: &rustc_hash::FxHashSet<GoldPair>, observed: &BTreeSet<GoldPair>) -> CountMetric {
    CountMetric {
        found: labels.iter().filter(|pair| observed.contains(*pair)).count(),
        total: labels.len(),
    }
}

fn status_label(status: SearchVectorCandidateStatus) -> String {
    match status {
        SearchVectorCandidateStatus::Disabled | SearchVectorCandidateStatus::Skipped => "skipped",
        SearchVectorCandidateStatus::Failed => "failed",
        SearchVectorCandidateStatus::Ok => "ok",
    }
    .to_owned()
}

pub(crate) fn write_default_artifact(repo_root: &Path, report: &VectorSearchReport) -> Result<PathBuf> {
    let artifact = PathBuf::from("target/search-quality").join(format!("{}-vector-search.json", report.suite));
    let absolute = repo_root.join(&artifact);
    let parent = absolute.parent().ok_or_else(|| Error::Eval {
        message: format!("vector search artifact has no parent: {}", absolute.display()),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        message: "could not create vector search artifact directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&absolute, json).map_err(|source| Error::Io {
        message: "could not write vector search artifact",
        path: absolute,
        source,
    })?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::{VectorSearchReportRun, pair_reports, scorer_variant_reports, vector_stage_metrics};
    use crate::eval::labels::{
        AdjudicationSource, GoldLabelFact, GoldLabels, LabelConfidence, LabelFactSource, LabelPolarity, MatchClass,
        TypedGoldLabel,
    };
    use crate::eval::scoring::GoldPair;
    use crate::eval::scoring::{CountMetric, EvaluationMetrics, TimingMetrics};
    use lean_dup_search::{
        SearchEmbeddingDocument, SearchEmbeddingDocuments, SearchEvidenceMode, SearchModuleRelation, SearchObservation,
        SearchObservedPair, SearchPairFeatures, SearchRetrievalObservation, SearchScoringSummary, SearchScoringVariant,
        SearchSemanticEvidenceState, SearchVectorCandidateStatus, SearchVectorCandidateSummary,
        SearchVectorEligibilitySummary, SearchVectorEvidence,
    };

    #[test]
    fn artifact_rows_deduplicate_unordered_pairs_and_keep_best_facts() {
        let labels = empty_labels();
        let observation = observation(vec![
            observed("B", "A", Some(7), false, true, false, None, None, "symbolic_audit"),
            observed(
                "A",
                "B",
                Some(3),
                true,
                false,
                true,
                Some(0.70),
                Some(3),
                "vector_mathlib",
            ),
            observed(
                "A",
                "B",
                Some(5),
                false,
                false,
                true,
                Some(0.92),
                Some(2),
                "vector_mathlib",
            ),
        ]);

        let rows = pair_reports(&labels, &observation);

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.left, "A");
        assert_eq!(row.right, "B");
        assert!(row.symbolic_generated);
        assert!(row.vector_generated);
        assert!(row.merged_generated);
        assert!(row.ranked);
        assert!(row.visible);
        assert_eq!(row.rank, Some(3));
        assert_eq!(row.vector_rank, Some(2));
        assert_eq!(row.vector_score, Some(0.92));
        assert_eq!(row.generation_policies, vec!["symbolic_audit", "vector_mathlib"]);
        assert_eq!(row.left_hash, "hash-A");
        assert_eq!(row.right_hash, "hash-B");
    }

    #[test]
    fn artifact_rows_report_expanded_and_skipped_label_facts() {
        let expanded_positive = GoldPair::new("A", "B");
        let expanded_hard_negative = GoldPair::new("C", "D");
        let skipped = GoldPair::new("E", "F");
        let labels = GoldLabels {
            suite: "unit".to_owned(),
            positives: FxHashSet::from_iter([expanded_positive.clone()]),
            hard_negatives: FxHashSet::from_iter([expanded_hard_negative.clone()]),
            typed_pairs: Vec::new(),
            label_facts: vec![
                fact(
                    expanded_positive,
                    Some(LabelPolarity::Positive),
                    LabelFactSource::ExpandedPositiveCluster,
                ),
                fact(
                    expanded_hard_negative,
                    Some(LabelPolarity::HardNegative),
                    LabelFactSource::ExpandedHardNegativeCluster,
                ),
                GoldLabelFact {
                    pair: skipped,
                    polarity: None,
                    source: LabelFactSource::ConflictResolved,
                    typed: None,
                    skipped_reason: Some("positive-label-wins"),
                },
            ],
        };
        let observation = observation(vec![
            observed("A", "B", Some(1), false, true, false, None, None, "symbolic"),
            observed("C", "D", Some(2), false, true, false, None, None, "symbolic"),
            observed("E", "F", Some(3), false, true, false, None, None, "symbolic"),
        ]);

        let rows = pair_reports(&labels, &observation);

        assert_eq!(rows[0].label_status, "expanded-positive");
        assert_eq!(rows[0].label_facts[0].status, "expanded-positive");
        assert_eq!(rows[1].label_status, "expanded-hard-negative");
        assert_eq!(rows[1].label_facts[0].status, "expanded-hard-negative");
        assert_eq!(rows[2].label_status, "skipped");
        assert_eq!(rows[2].label_facts[0].skipped_reason, Some("positive-label-wins"));
    }

    #[test]
    fn vector_stage_metrics_report_vector_and_symbolic_only_denominators() {
        let labels = GoldLabels {
            suite: "unit".to_owned(),
            positives: FxHashSet::from_iter([
                GoldPair::new("A", "B"),
                GoldPair::new("C", "D"),
                GoldPair::new("E", "F"),
            ]),
            hard_negatives: FxHashSet::from_iter([GoldPair::new("G", "H"), GoldPair::new("I", "J")]),
            typed_pairs: Vec::new(),
            label_facts: Vec::new(),
        };
        let mut observation = observation(vec![
            observed("A", "B", Some(1), false, false, true, Some(0.9), Some(1), "vector"),
            observed("C", "D", Some(2), true, true, false, None, None, "symbolic"),
            observed("E", "F", Some(3), false, true, true, Some(0.8), Some(2), "merged"),
            observed("G", "H", Some(4), false, false, true, Some(0.7), Some(3), "vector"),
            observed("I", "J", Some(5), false, true, false, None, None, "symbolic"),
            observed("K", "L", Some(6), false, false, true, Some(0.6), Some(4), "vector"),
        ]);
        observation.retrieval.vector_candidates = SearchVectorCandidateSummary {
            top_k_saturated: true,
            query_declaration_count: 5,
            ..SearchVectorCandidateSummary::default()
        };

        let metrics = vector_stage_metrics(&labels, &observation);

        assert_eq!(metrics.vector_top_k_recall.found, 2);
        assert_eq!(metrics.vector_top_k_recall.total, 3);
        assert_eq!(metrics.vector_top_k_precision.found, 2);
        assert_eq!(metrics.vector_top_k_precision.total, 4);
        assert_eq!(metrics.top_k_saturation.found, 5);
        assert_eq!(metrics.top_k_saturation.total, 5);
        assert_eq!(metrics.vector_only_positives.found, 1);
        assert_eq!(metrics.symbolic_only_positives.found, 1);
        assert_eq!(metrics.vector_only_hard_negatives.found, 1);
        assert_eq!(metrics.symbolic_only_hard_negatives.found, 1);
        assert_eq!(metrics.merged_generated_recall.found, 3);
        assert_eq!(metrics.ranked_recall.found, 3);
        assert_eq!(metrics.visible_precision.found, 1);
        assert_eq!(metrics.visible_precision.total, 1);
    }

    #[test]
    fn realistic_vector_validation_fixture_is_non_saturated_and_has_required_label_classes() {
        let labels = GoldLabels {
            suite: "realistic-vector-fixture".to_owned(),
            positives: FxHashSet::from_iter([
                GoldPair::new("VectorOnly.query", "VectorOnly.document"),
                GoldPair::new("SymbolicOnly.query", "SymbolicOnly.document"),
            ]),
            hard_negatives: FxHashSet::from_iter([GoldPair::new(
                "LexicalTrap.height",
                "LexicalTrap.height_not_duplicate",
            )]),
            typed_pairs: Vec::new(),
            label_facts: Vec::new(),
        };
        let mut observation = observation(vec![
            observed(
                "VectorOnly.query",
                "VectorOnly.document",
                Some(1),
                false,
                false,
                true,
                Some(0.93),
                Some(1),
                "vector_source_backed_external_comparison",
            ),
            observed(
                "SymbolicOnly.query",
                "SymbolicOnly.document",
                Some(2),
                true,
                true,
                false,
                None,
                None,
                "source_backed_external_comparison",
            ),
            observed(
                "LexicalTrap.height",
                "LexicalTrap.height_not_duplicate",
                Some(3),
                false,
                false,
                true,
                Some(0.88),
                Some(2),
                "vector_source_backed_external_comparison",
            ),
        ]);
        observation.retrieval.vector_candidates = SearchVectorCandidateSummary {
            status: SearchVectorCandidateStatus::Ok,
            query_eligibility: SearchVectorEligibilitySummary {
                policy_id: "actionable-public-statement".to_owned(),
                policy_version: "lean-dup.vector-candidate.v1",
                total: 80,
                eligible: 72,
                skipped_by_reason: std::collections::BTreeMap::from([
                    ("generated".to_owned(), 1),
                    ("private".to_owned(), 1),
                    ("synthetic".to_owned(), 1),
                    ("low-signal".to_owned(), 2),
                    ("missing-statement".to_owned(), 1),
                    ("not-actionable".to_owned(), 1),
                    ("unsupported-kind".to_owned(), 1),
                ]),
            },
            corpus_eligibility: SearchVectorEligibilitySummary {
                policy_id: "actionable-public-statement".to_owned(),
                policy_version: "lean-dup.vector-candidate.v1",
                total: 80,
                eligible: 72,
                skipped_by_reason: std::collections::BTreeMap::new(),
            },
            top_k: 32,
            eligible_corpus_size: 72,
            query_declaration_count: 72,
            corpus_declaration_count: 72,
            top_k_saturated: false,
            ..SearchVectorCandidateSummary::default()
        };

        let metrics = vector_stage_metrics(&labels, &observation);
        let rows = pair_reports(&labels, &observation);

        assert_eq!(observation.retrieval.vector_candidates.top_k, 32);
        assert_eq!(observation.retrieval.vector_candidates.eligible_corpus_size, 72);
        assert!(!observation.retrieval.vector_candidates.top_k_saturated);
        assert_eq!(metrics.top_k_saturation.found, 0);
        assert_eq!(metrics.top_k_saturation.total, 72);
        assert_eq!(metrics.vector_only_positives.found, 1);
        assert_eq!(metrics.vector_only_positives.total, 2);
        assert_eq!(metrics.symbolic_only_positives.found, 1);
        assert_eq!(metrics.symbolic_only_positives.total, 2);
        assert_eq!(metrics.vector_only_hard_negatives.found, 1);
        assert_eq!(metrics.vector_only_hard_negatives.total, 1);
        assert_eq!(metrics.vector_top_k_recall.found, 1);
        assert_eq!(metrics.vector_top_k_recall.total, 2);
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter()
                .any(|row| row.label_status == "positive" && row.vector_generated)
        );
        assert!(
            rows.iter()
                .any(|row| row.label_status == "positive" && row.symbolic_generated)
        );
        assert!(
            rows.iter()
                .any(|row| row.label_status == "hard-negative" && row.vector_generated)
        );
    }

    #[test]
    fn scorer_variant_reports_measure_vector_evidence_separately() {
        let labels = GoldLabels {
            suite: "unit".to_owned(),
            positives: FxHashSet::from_iter([GoldPair::new("A", "B")]),
            hard_negatives: FxHashSet::default(),
            typed_pairs: Vec::new(),
            label_facts: Vec::new(),
        };
        let mut observation = observation(vec![observed(
            "A",
            "B",
            Some(1),
            false,
            false,
            true,
            Some(0.94),
            Some(1),
            "vector",
        )]);
        observation.retrieval.vector_candidates.status = SearchVectorCandidateStatus::Ok;
        let metrics = metrics();

        let reports = scorer_variant_reports(&VectorSearchReportRun {
            repo_root: std::path::Path::new("."),
            suite: "unit",
            labels: &labels,
            observation: &observation,
            symbolic_baseline: &metrics,
            vector_metrics: &metrics,
            scorer_version: "lean-dup.symbolic-scorer.v1",
            k_values: &[1],
        });

        assert_eq!(reports.len(), 3);
        let symbolic = reports
            .iter()
            .find(|report| report.scorer_variant_id == "symbolic-only")
            .expect("symbolic variant");
        let vector_only = reports
            .iter()
            .find(|report| report.scorer_variant_id == "vector-evidence-only")
            .expect("vector variant");
        assert_eq!(symbolic.metrics.shown_queue_precision.total, 0);
        assert_eq!(vector_only.metrics.shown_queue_precision.found, 1);
        assert_eq!(vector_only.vector_feature_version, "lean-dup.vector-evidence.v1");
    }

    fn empty_labels() -> GoldLabels {
        GoldLabels {
            suite: "unit".to_owned(),
            positives: FxHashSet::default(),
            hard_negatives: FxHashSet::default(),
            typed_pairs: Vec::new(),
            label_facts: Vec::new(),
        }
    }

    fn fact(pair: GoldPair, polarity: Option<LabelPolarity>, source: LabelFactSource) -> GoldLabelFact {
        GoldLabelFact {
            pair,
            polarity,
            source,
            typed: None,
            skipped_reason: None,
        }
    }

    fn observation(pairs: Vec<SearchObservedPair>) -> SearchObservation {
        SearchObservation {
            pairs,
            visible_groups_found: 0,
            visible_groups_total: 0,
            scoring: SearchScoringSummary::new(SearchScoringVariant::SymbolicOnly),
            semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
            semantic_obligation_yield: Vec::new(),
            retrieval: SearchRetrievalObservation::default(),
            embedding_documents: SearchEmbeddingDocuments {
                policy_id: "name-and-formal-statement".to_owned(),
                policy_version: "test".to_owned(),
                documents: ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L"]
                    .into_iter()
                    .map(|name| SearchEmbeddingDocument {
                        declaration_name: name.to_owned(),
                        module_name: "Test".to_owned(),
                        declaration_kind: "theorem".to_owned(),
                        normalized_formal_statement: "hidden".to_owned(),
                        informal_text: None,
                        content_hash: format!("hash-{name}"),
                    })
                    .collect(),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn observed(
        left: &str,
        right: &str,
        rank: Option<usize>,
        shown: bool,
        symbolic_generated: bool,
        vector_generated: bool,
        vector_score: Option<f64>,
        vector_rank: Option<usize>,
        generation_policy: &str,
    ) -> SearchObservedPair {
        SearchObservedPair {
            left: left.to_owned(),
            right: right.to_owned(),
            generated: true,
            symbolic_generated,
            vector_generated,
            merged_generated: symbolic_generated || vector_generated,
            ranked: rank.is_some(),
            generation_policy: generation_policy.to_owned(),
            rank,
            shown,
            left_content_hash: Some(format!("hash-{left}")),
            right_content_hash: Some(format!("hash-{right}")),
            vector_score,
            vector_rank,
            origin: "workspace".to_owned(),
            feature_families: if vector_generated && !symbolic_generated {
                vec!["vector_similarity".to_owned()]
            } else {
                vec!["statement_fingerprint".to_owned()]
            },
            survived_shown_filter: shown,
            features: SearchPairFeatures {
                retrieval_feature_families: Vec::new(),
                declaration_kinds: vec!["theorem".to_owned()],
                evidence_mode: SearchEvidenceMode::Local,
                vector_evidence: vector_score.zip(vector_rank).map(|(score, rank)| SearchVectorEvidence {
                    version: "lean-dup.vector-evidence.v1".to_owned(),
                    score_bucket: if score >= 0.90 { "very-high" } else { "high" }.to_owned(),
                    rank_bucket: if rank == 1 { "rank-1" } else { "rank-2-3" }.to_owned(),
                    reciprocal_rank_micros: (1_000_000usize / rank.max(1)) as u32,
                }),
                structural_fingerprint_families: Vec::new(),
                role_overlap: Vec::new(),
                module_relation: SearchModuleRelation::SameModule {
                    module: "Test".to_owned(),
                },
                semantic_reranking: lean_dup_search::SearchSemanticRerankingSummary::default(),
                semantic_evidence_state: SearchSemanticEvidenceState::NotRun,
                semantic_obligations: Vec::new(),
                cheap_blockers: Vec::new(),
            },
            scoring: lean_dup_search::SearchPairScoring {
                version: "lean-dup.symbolic-scorer.v1",
                variant: SearchScoringVariant::SymbolicOnly,
                total_score: 0.0,
                component_scores: std::collections::BTreeMap::new(),
            },
        }
    }

    #[allow(dead_code)]
    fn typed_label(pair: GoldPair, polarity: LabelPolarity) -> TypedGoldLabel {
        TypedGoldLabel {
            pair,
            polarity,
            match_class: if polarity == LabelPolarity::Positive {
                MatchClass::ExactTheoremDuplicate
            } else {
                MatchClass::HardNegative
            },
            expected_stage_visibility: crate::eval::labels::ExpectedStageVisibility::Visible,
            adjudication_source: AdjudicationSource::FixtureIntent,
            confidence: LabelConfidence::High,
            semantic_verification_required: false,
            static_evidence_acceptable: true,
        }
    }

    fn metrics() -> EvaluationMetrics {
        EvaluationMetrics {
            suite: "unit".to_owned(),
            recall: Vec::new(),
            shown_queue_precision: CountMetric::default(),
            hard_negative_hits: CountMetric::default(),
            visible_groups: CountMetric::default(),
            probe_unavailable: CountMetric::default(),
            stage_metrics: crate::eval::stage_metrics::SearchStageMetrics::default(),
            candidate_count: 0,
            timings: TimingMetrics::default(),
            peak_memory_bytes: None,
        }
    }
}
