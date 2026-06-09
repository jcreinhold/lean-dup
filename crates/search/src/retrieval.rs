use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::Serialize;

use lean_dup_diagnostics::perf::{self, CostClass};
use lean_dup_index::{
    DeclarationHandle, FingerprintKind, HydratedDeclaration, OpenedIndex, SemanticFeatureKey,
    SemanticFingerprintFeature, SemanticRoleFeature,
};

use crate::Result;

const TOP_K_PER_WORKSPACE_DECLARATION: usize = 80;
const SEMANTIC_LANE_TOP_K_PER_WORKSPACE_DECLARATION: usize = 24;
const ROLE_POSTING_LIMIT: usize = 512;
const BROAD_HEAD_POSTING_LIMIT: usize = 64;
pub(crate) const FANOUT_POLICY_ID: &str = "lean-dup.fanout-policy.v1";

/// Candidate retrieval results for a workspace corpus.
///
/// The output is grouped by workspace declaration. Each group contains a
/// bounded, score-ordered set of declarations worth sending to later ranking
/// or semantic-probe stages, plus diagnostics that explain pruning and
/// hydration decisions.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub struct RetrievalOutput {
    pub candidate_sets: Vec<CandidateSet>,
    pub diagnostics: RetrievalDiagnostics,
}

/// Retrieved candidates for one workspace declaration.
///
/// The anchor is always a workspace declaration supplied by the caller.
/// Candidates may come from the same workspace corpus or from one of the
/// opened comparison indexes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub struct CandidateSet {
    pub anchor: HydratedDeclaration,
    pub candidates: Vec<RetrievedCandidate>,
}

/// One candidate declaration selected for a workspace declaration.
///
/// `score` is only an ordering signal for retrieval. Later stages decide
/// review priority, replacement actions, and report wording.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub struct RetrievedCandidate {
    pub pair_id: String,
    pub declaration: HydratedDeclaration,
    pub score: f64,
    pub explanation: CandidateExplanation,
    pub(crate) source_evidence: Vec<CandidateSourceEvidence>,
}

/// Why retrieval selected a candidate.
///
/// The contributions are semantic keys emitted by Lean and matched through an
/// index. They are evidence for retrieval only, not proof certificates.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub struct CandidateExplanation {
    pub contributions: Vec<KeyContribution>,
}

/// Retrieval-level counters and pruning records.
///
/// Diagnostics are intended for audit logs and later `show` output. They
/// explain how much work was avoided without exposing the index storage shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[allow(dead_code)]
pub struct RetrievalDiagnostics {
    pub fanout_policy: FanoutPolicySummary,
    pub candidate_count: usize,
    pub generated_candidate_count: usize,
    pub ranked_candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_postings: Vec<PrunedPosting>,
    pub pruned_feature_fanouts: Vec<PrunedFeatureFanout>,
    pub heap_truncations: Vec<HeapTruncation>,
    pub candidate_count_by_generation_policy: BTreeMap<String, usize>,
    pub top_k_saturation_by_source_id: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct FanoutPolicySummary {
    pub policy_id: String,
    pub symbolic_top_k_per_anchor: usize,
    pub semantic_lane_top_k_per_anchor: usize,
    pub role_posting_limit: usize,
    pub broad_head_posting_limit: usize,
}

impl Default for FanoutPolicySummary {
    fn default() -> Self {
        Self {
            policy_id: FANOUT_POLICY_ID.to_owned(),
            symbolic_top_k_per_anchor: TOP_K_PER_WORKSPACE_DECLARATION,
            semantic_lane_top_k_per_anchor: SEMANTIC_LANE_TOP_K_PER_WORKSPACE_DECLARATION,
            role_posting_limit: ROLE_POSTING_LIMIT,
            broad_head_posting_limit: BROAD_HEAD_POSTING_LIMIT,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GeneratedPairEvidence {
    pub policy: String,
    pub contributions: Vec<KeyContribution>,
    pub source_evidence: Vec<CandidateSourceEvidence>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TrackedPairPolicyEvidence {
    pub generated: Option<GeneratedPairEvidence>,
    pub losses: Vec<CandidateLossEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateLossEvidence {
    pub source_id: String,
    pub source_family: CandidateSourceFamily,
    pub loss_stage: CandidateLossStage,
    pub policy: String,
    pub source: String,
    pub reason: String,
    pub feature_family: String,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CandidateLossStage {
    FanoutPruned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub(crate) struct CandidateSourceEvidence {
    pub source_id: String,
    pub source_family: CandidateSourceFamily,
    pub generation_rank: Option<usize>,
    pub top_k_status: CandidateTopKStatus,
    pub top_k_saturated: bool,
    pub feature_families: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CandidateSourceFamily {
    Symbolic,
    LeanSemantic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CandidateTopKStatus {
    Selected,
    GeneratedNotSelected,
}

/// One semantic key contribution to a retrieved candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub struct KeyContribution {
    pub kind: String,
    pub role: Option<String>,
    pub display: Option<String>,
    pub key: String,
    pub score: f64,
}

impl KeyContribution {
    pub(crate) fn feature_family(&self) -> String {
        match self.kind.as_str() {
            "statement-fingerprint" => "statement_fingerprint".to_owned(),
            "safe-permutation-fingerprint" => "safe_permutation_fingerprint".to_owned(),
            "connective-fingerprint" => "connective_fingerprint".to_owned(),
            "conclusion-fingerprint" => "conclusion_fingerprint".to_owned(),
            "role-feature" => match self.role.as_deref() {
                Some("conclusion_const") => "role_conclusion_const".to_owned(),
                Some("hypothesis_const") => "role_hypothesis_const".to_owned(),
                Some("conclusion_head" | "hypothesis_head" | "binder_domain_head") => "role_head".to_owned(),
                _ => "role_other".to_owned(),
            },
            _ => "other".to_owned(),
        }
    }
}

/// A semantic posting that retrieval chose not to expand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct PrunedPosting {
    pub anchor_declaration_id: String,
    pub source: String,
    pub reason: String,
    pub kind: String,
    pub role: Option<String>,
    pub display: Option<String>,
    pub count: usize,
}

/// A broad semantic feature fanout that candidate generation did not expand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct PrunedFeatureFanout {
    pub policy: String,
    pub source: String,
    pub reason: String,
    pub feature_family: String,
    pub count: usize,
}

/// Per-anchor count of lower-scoring candidates dropped from the bounded set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct HeapTruncation {
    pub anchor_declaration_id: String,
    pub dropped_count: usize,
}

/// Retrieve bounded candidate declarations for each workspace declaration.
///
/// Callers provide already-hydrated workspace declarations and any opened
/// comparison indexes. Retrieval returns semantic candidates and explanations;
/// it does not perform ranking, probing, reporting, or replacement-hint policy.
#[allow(dead_code)]
pub fn retrieve_candidates(workspace: &[HydratedDeclaration], indexes: &[OpenedIndex]) -> Result<RetrievalOutput> {
    perf::record_count(
        CostClass::RetrievalRanking,
        "retrieval.workspace_declarations",
        workspace.len() as u64,
    );
    perf::record_count(
        CostClass::RetrievalRanking,
        "retrieval.external_indexes",
        indexes.len() as u64,
    );
    perf::measure_result(CostClass::RetrievalRanking, "retrieval.total", || {
        retrieve_candidates_inner(workspace, indexes)
    })
}

fn retrieve_candidates_inner(workspace: &[HydratedDeclaration], indexes: &[OpenedIndex]) -> Result<RetrievalOutput> {
    let mut diagnostics = RetrievalDiagnostics::default();
    if workspace.is_empty() {
        return Ok(RetrievalOutput {
            candidate_sets: Vec::new(),
            diagnostics,
        });
    }

    let workspace_plans = workspace.iter().map(planned_keys).collect::<Vec<_>>();
    let local_postings = local_postings(&workspace_plans);
    let local_counts = local_counts(&local_postings);
    let index_facts = indexes
        .iter()
        .map(OpenedIndex::facts)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let external_counts = external_counts(indexes, &workspace_plans)?;
    let external_postings = external_postings(indexes, &workspace_plans, &external_counts)?;

    let mut selected_by_anchor = Vec::with_capacity(workspace.len());
    let mut external_needed: BTreeMap<usize, Vec<DeclarationHandle>> = BTreeMap::new();

    for anchor_index in 0..workspace.len() {
        let mut accumulators: HashMap<CandidateId, CandidateAccumulator> = HashMap::default();
        let plans = sorted_plans(&workspace_plans[anchor_index]);
        for plan in plans {
            add_local_matches(
                workspace,
                anchor_index,
                plan,
                &local_postings,
                &local_counts,
                &mut accumulators,
                &mut diagnostics,
            );
            add_external_matches(
                workspace,
                anchor_index,
                plan,
                ExternalMatchContext {
                    indexes,
                    index_facts: &index_facts,
                    counts: &external_counts,
                    postings: &external_postings,
                },
                &mut accumulators,
                &mut diagnostics,
            );
        }

        let selected = select_top(&workspace[anchor_index].declaration_id, accumulators, &mut diagnostics);
        for candidate in &selected {
            if let CandidateId::External { index, handle } = &candidate.id {
                external_needed.entry(*index).or_default().push(handle.clone());
            }
        }
        selected_by_anchor.push(selected);
    }

    let hydrated_external = hydrate_external(indexes, external_needed, &mut diagnostics)?;
    let mut candidate_sets = Vec::new();
    for (anchor_index, selected) in selected_by_anchor.into_iter().enumerate() {
        let mut candidates = Vec::new();
        for candidate in selected {
            let declaration = match &candidate.id {
                CandidateId::Workspace(index) => workspace[*index].clone(),
                CandidateId::External { index, handle } => hydrated_external
                    .get(&(*index, handle.clone()))
                    .expect("selected external handles are hydrated")
                    .clone(),
            };
            let pair_id = pair_id(&workspace[anchor_index], &declaration);
            candidates.push(RetrievedCandidate {
                pair_id,
                declaration,
                score: candidate.score,
                explanation: CandidateExplanation {
                    contributions: candidate.contributions,
                },
                source_evidence: candidate.source_evidence,
            });
        }
        if !candidates.is_empty() {
            candidate_sets.push(CandidateSet {
                anchor: workspace[anchor_index].clone(),
                candidates,
            });
        }
    }
    diagnostics.candidate_count = candidate_sets.iter().map(|set| set.candidates.len()).sum::<usize>();
    perf::record_count(
        CostClass::RetrievalRanking,
        "retrieval.candidates",
        diagnostics.candidate_count as u64,
    );
    perf::record_count(
        CostClass::RetrievalRanking,
        "retrieval.hydrated_external",
        diagnostics.hydrated_external_count as u64,
    );

    Ok(RetrievalOutput {
        candidate_sets,
        diagnostics,
    })
}

#[derive(Debug, Clone)]
struct PlannedKey {
    key: SemanticFeatureKey,
    contribution: KeyContribution,
    base_weight: f64,
    broad_head: bool,
    admits_candidate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum CandidateId {
    Workspace(usize),
    External { index: usize, handle: DeclarationHandle },
}

#[derive(Debug, Clone)]
struct CandidateAccumulator {
    id: CandidateId,
    policy: CandidateGenerationPolicy,
    score: f64,
    admitted: bool,
    contributions: BTreeMap<String, KeyContribution>,
}

#[derive(Debug, Clone)]
struct SelectedCandidate {
    id: CandidateId,
    score: f64,
    contributions: Vec<KeyContribution>,
    source_evidence: Vec<CandidateSourceEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SemanticCandidateLane {
    StatementMeaning,
    BinderRoleShape,
}

impl SemanticCandidateLane {
    fn all() -> [Self; 2] {
        [Self::StatementMeaning, Self::BinderRoleShape]
    }

    fn source_id(self) -> &'static str {
        match self {
            Self::StatementMeaning => "lean-semantic.statement-meaning.v1",
            Self::BinderRoleShape => "lean-semantic.binder-role-shape.v1",
        }
    }

    fn accepts(self, contribution: &KeyContribution) -> bool {
        match self {
            Self::StatementMeaning => matches!(
                contribution.kind.as_str(),
                "statement-fingerprint"
                    | "safe-permutation-fingerprint"
                    | "connective-fingerprint"
                    | "conclusion-fingerprint"
            ),
            Self::BinderRoleShape => contribution.kind == "role-feature",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateGenerationPolicy {
    LocalDuplicateAudit,
    MathlibComparison,
    StaticExternalComparison,
    SourceBackedExternalComparison,
}

impl CandidateGenerationPolicy {
    fn label(self) -> &'static str {
        match self {
            Self::LocalDuplicateAudit => "local_duplicate_audit",
            Self::MathlibComparison => "mathlib_comparison",
            Self::StaticExternalComparison => "static_external_comparison",
            Self::SourceBackedExternalComparison => "source_backed_external_comparison",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct HeapEntry {
    score_micros: i64,
    tie_breaker: String,
    candidate_index: usize,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score_micros
            .cmp(&other.score_micros)
            .then_with(|| self.tie_breaker.cmp(&other.tie_breaker))
            .then_with(|| self.candidate_index.cmp(&other.candidate_index))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn planned_keys(declaration: &HydratedDeclaration) -> Vec<PlannedKey> {
    let mut plans = vec![
        fingerprint_plan(
            FingerprintKind::Statement,
            &declaration.fingerprints.statement,
            "statement-fingerprint",
            100.0,
        ),
        fingerprint_plan(
            FingerprintKind::SafeBinderPermutation,
            &declaration.fingerprints.safe_binder_permutation,
            "safe-permutation-fingerprint",
            85.0,
        ),
        fingerprint_plan(
            FingerprintKind::ConnectiveShape,
            &declaration.fingerprints.connective_shape,
            "connective-fingerprint",
            65.0,
        ),
        fingerprint_plan(
            FingerprintKind::ConclusionShape,
            &declaration.fingerprints.conclusion_shape,
            "conclusion-fingerprint",
            45.0,
        ),
    ];
    plans.retain(|plan| !plan.key.is_empty());

    let broad_heads = broad_heads(declaration);
    for feature in &declaration.role_features {
        if feature.key.is_empty() {
            continue;
        }
        let broad_head = feature
            .display
            .as_ref()
            .is_some_and(|display| broad_heads.contains(display))
            && feature.role.ends_with("_head");
        let admits_candidate = !broad_head && feature.role != "binder_domain_head";
        let base_weight = role_weight(&feature.role, broad_head);
        plans.push(PlannedKey {
            key: SemanticFeatureKey::RoleFeature(SemanticRoleFeature {
                role: feature.role.clone(),
                key: feature.key.clone(),
            }),
            contribution: KeyContribution {
                kind: "role-feature".to_owned(),
                role: Some(feature.role.clone()),
                display: feature.display.clone(),
                key: feature.key.clone(),
                score: 0.0,
            },
            base_weight,
            broad_head,
            admits_candidate,
        });
    }
    plans
}

pub(crate) fn tracked_pair_policy_evidence(
    workspace: &[HydratedDeclaration],
    anchor: &HydratedDeclaration,
    candidate: &HydratedDeclaration,
    external: Option<(&OpenedIndex, &lean_dup_index::OpenedIndexFacts)>,
) -> Result<TrackedPairPolicyEvidence> {
    let anchor_plans = planned_keys(anchor);
    let candidate_keys = planned_keys(candidate)
        .into_iter()
        .map(|plan| plan.key)
        .collect::<HashSet<_>>();
    let policy = external.map_or(CandidateGenerationPolicy::LocalDuplicateAudit, |(_, facts)| {
        policy_for_external(facts)
    });
    let total_documents = external.map_or(workspace.len(), |(_, facts)| facts.declaration_count);
    let local_counts = if external.is_none() {
        let workspace_plans = workspace.iter().map(planned_keys).collect::<Vec<_>>();
        Some(local_counts(&local_postings(&workspace_plans)))
    } else {
        None
    };
    let mut contributions = BTreeMap::new();
    let mut losses = Vec::new();
    let mut admitted = false;
    for plan in sorted_plans(&anchor_plans) {
        if !candidate_keys.contains(&plan.key) {
            continue;
        }
        let count = if let Some((opened, _)) = external {
            opened.feature_fanout(std::slice::from_ref(&plan.key))?.count(&plan.key)
        } else {
            local_counts
                .as_ref()
                .and_then(|counts| counts.get(&plan.key).copied())
                .unwrap_or(0)
        };
        if count == 0 || count > posting_limit(plan) {
            if count > posting_limit(plan) {
                losses.push(CandidateLossEvidence {
                    source_id: "symbolic-retrieval".to_owned(),
                    source_family: CandidateSourceFamily::Symbolic,
                    loss_stage: CandidateLossStage::FanoutPruned,
                    policy: policy.label().to_owned(),
                    source: external.map_or_else(|| "workspace".to_owned(), |(_, facts)| facts.origin.clone()),
                    reason: prune_reason_for_plan(plan).to_owned(),
                    feature_family: plan.contribution.feature_family(),
                    count,
                });
            }
            continue;
        }
        if !plan.admits_candidate && !admitted {
            continue;
        }
        admitted |= plan.admits_candidate;
        let mut contribution = plan.contribution.clone();
        contribution.score = plan.base_weight * rarity_weight(total_documents, count);
        contributions
            .entry(contribution_sort_key(&contribution))
            .and_modify(|existing: &mut KeyContribution| existing.score += contribution.score)
            .or_insert(contribution);
    }
    if !admitted {
        return Ok(TrackedPairPolicyEvidence {
            generated: None,
            losses,
        });
    }
    let source_evidence = generated_pair_source_evidence(contributions.values());
    Ok(TrackedPairPolicyEvidence {
        generated: Some(GeneratedPairEvidence {
            policy: policy.label().to_owned(),
            source_evidence,
            contributions: contributions.into_values().collect(),
        }),
        losses: Vec::new(),
    })
}

fn fingerprint_plan(kind: FingerprintKind, key: &str, label: &'static str, base_weight: f64) -> PlannedKey {
    PlannedKey {
        key: SemanticFeatureKey::Fingerprint(SemanticFingerprintFeature {
            kind,
            key: key.to_owned(),
        }),
        contribution: KeyContribution {
            kind: label.to_owned(),
            role: None,
            display: None,
            key: key.to_owned(),
            score: 0.0,
        },
        base_weight,
        broad_head: false,
        admits_candidate: true,
    }
}

fn role_weight(role: &str, broad_head: bool) -> f64 {
    if broad_head {
        1.0
    } else {
        match role {
            "conclusion_const" => 18.0,
            "hypothesis_const" => 10.0,
            "conclusion_head" => 8.0,
            "hypothesis_head" => 4.0,
            "binder_domain_head" => 3.0,
            _ => 2.0,
        }
    }
}

fn broad_heads(declaration: &HydratedDeclaration) -> HashSet<String> {
    declaration
        .low_signal_markers
        .iter()
        .filter_map(|marker| marker.strip_prefix("broad_head:"))
        .map(str::to_owned)
        .collect()
}

fn sorted_plans(plans: &[PlannedKey]) -> Vec<&PlannedKey> {
    let mut sorted = plans.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        right
            .admits_candidate
            .cmp(&left.admits_candidate)
            .then_with(|| {
                right
                    .base_weight
                    .partial_cmp(&left.base_weight)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| contribution_sort_key(&left.contribution).cmp(&contribution_sort_key(&right.contribution)))
    });
    sorted
}

fn local_postings(plans_by_decl: &[Vec<PlannedKey>]) -> HashMap<SemanticFeatureKey, Vec<usize>> {
    let mut postings: HashMap<SemanticFeatureKey, Vec<usize>> = HashMap::default();
    for (index, plans) in plans_by_decl.iter().enumerate() {
        let mut seen = HashSet::default();
        for plan in plans {
            if seen.insert(plan.key.clone()) {
                postings.entry(plan.key.clone()).or_default().push(index);
            }
        }
    }
    postings
}

fn local_counts(postings: &HashMap<SemanticFeatureKey, Vec<usize>>) -> HashMap<SemanticFeatureKey, usize> {
    postings
        .iter()
        .map(|(key, handles)| (key.clone(), handles.len()))
        .collect()
}

fn external_counts(
    indexes: &[OpenedIndex],
    workspace_plans: &[Vec<PlannedKey>],
) -> Result<HashMap<(usize, SemanticFeatureKey), usize>> {
    let keys = unique_keys(workspace_plans);
    let mut counts = HashMap::default();
    for (index, opened) in indexes.iter().enumerate() {
        for (key, count) in opened.feature_fanout(&keys)?.iter() {
            counts.insert((index, key.clone()), count);
        }
    }
    Ok(counts)
}

fn external_postings(
    indexes: &[OpenedIndex],
    workspace_plans: &[Vec<PlannedKey>],
    external_counts: &HashMap<(usize, SemanticFeatureKey), usize>,
) -> Result<HashMap<(usize, SemanticFeatureKey), Vec<DeclarationHandle>>> {
    let keys = unique_keys(workspace_plans);
    let mut postings = HashMap::default();
    for (index, opened) in indexes.iter().enumerate() {
        let selected = keys
            .iter()
            .filter(|key| {
                let count = external_counts.get(&(index, (*key).clone())).copied().unwrap_or(0);
                count > 0 && count <= posting_limit_for_key(workspace_plans, key)
            })
            .cloned()
            .collect::<Vec<_>>();
        for (key, handles) in opened.handles_matching_features(&selected)?.iter() {
            postings
                .entry((index, key.clone()))
                .or_insert_with(Vec::new)
                .extend(handles.iter().cloned());
        }
    }
    Ok(postings)
}

fn unique_keys(workspace_plans: &[Vec<PlannedKey>]) -> Vec<SemanticFeatureKey> {
    let mut keys = workspace_plans
        .iter()
        .flat_map(|plans| plans.iter().map(|plan| plan.key.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn add_local_matches(
    workspace: &[HydratedDeclaration],
    anchor_index: usize,
    plan: &PlannedKey,
    local_postings: &HashMap<SemanticFeatureKey, Vec<usize>>,
    local_counts: &HashMap<SemanticFeatureKey, usize>,
    accumulators: &mut HashMap<CandidateId, CandidateAccumulator>,
    diagnostics: &mut RetrievalDiagnostics,
) {
    let Some(matches) = local_postings.get(&plan.key) else {
        return;
    };
    let count = local_counts.get(&plan.key).copied().unwrap_or(matches.len());
    if count > posting_limit(plan) {
        diagnostics.pruned_postings.push(PrunedPosting {
            anchor_declaration_id: workspace[anchor_index].declaration_id.clone(),
            source: "workspace".to_owned(),
            reason: prune_reason_for_plan(plan).to_owned(),
            kind: plan.contribution.kind.clone(),
            role: plan.contribution.role.clone(),
            display: plan.contribution.display.clone(),
            count,
        });
        record_pruned_feature_fanout(
            diagnostics,
            CandidateGenerationPolicy::LocalDuplicateAudit,
            "workspace",
            plan,
            count,
        );
        return;
    }
    for candidate_index in matches {
        if *candidate_index == anchor_index {
            continue;
        }
        let score = plan.base_weight * rarity_weight(workspace.len(), count);
        add_contribution(
            CandidateId::Workspace(*candidate_index),
            CandidateGenerationPolicy::LocalDuplicateAudit,
            plan,
            score,
            accumulators,
        );
    }
}

struct ExternalMatchContext<'a> {
    indexes: &'a [OpenedIndex],
    index_facts: &'a [lean_dup_index::OpenedIndexFacts],
    counts: &'a HashMap<(usize, SemanticFeatureKey), usize>,
    postings: &'a HashMap<(usize, SemanticFeatureKey), Vec<DeclarationHandle>>,
}

fn add_external_matches(
    workspace: &[HydratedDeclaration],
    anchor_index: usize,
    plan: &PlannedKey,
    context: ExternalMatchContext<'_>,
    accumulators: &mut HashMap<CandidateId, CandidateAccumulator>,
    diagnostics: &mut RetrievalDiagnostics,
) {
    for (index, facts) in context.index_facts.iter().enumerate().take(context.indexes.len()) {
        let policy = policy_for_external(facts);
        let count = context.counts.get(&(index, plan.key.clone())).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        if count > posting_limit(plan) {
            diagnostics.pruned_postings.push(PrunedPosting {
                anchor_declaration_id: workspace[anchor_index].declaration_id.clone(),
                source: facts.origin.clone(),
                reason: prune_reason_for_plan(plan).to_owned(),
                kind: plan.contribution.kind.clone(),
                role: plan.contribution.role.clone(),
                display: plan.contribution.display.clone(),
                count,
            });
            record_pruned_feature_fanout(diagnostics, policy, &facts.origin, plan, count);
            continue;
        }
        let Some(handles) = context.postings.get(&(index, plan.key.clone())) else {
            continue;
        };
        for handle in handles {
            let score = plan.base_weight * rarity_weight(facts.declaration_count, count);
            add_contribution(
                CandidateId::External {
                    index,
                    handle: handle.clone(),
                },
                policy,
                plan,
                score,
                accumulators,
            );
        }
    }
}

fn add_contribution(
    id: CandidateId,
    policy: CandidateGenerationPolicy,
    plan: &PlannedKey,
    score: f64,
    accumulators: &mut HashMap<CandidateId, CandidateAccumulator>,
) {
    if !plan.admits_candidate && !accumulators.contains_key(&id) {
        return;
    }
    let accumulator = accumulators.entry(id.clone()).or_insert_with(|| CandidateAccumulator {
        id,
        policy,
        score: 0.0,
        admitted: false,
        contributions: BTreeMap::new(),
    });
    accumulator.score += score;
    accumulator.admitted |= plan.admits_candidate;
    let mut contribution = plan.contribution.clone();
    contribution.score = score;
    accumulator
        .contributions
        .entry(contribution_sort_key(&contribution))
        .and_modify(|existing| existing.score += score)
        .or_insert(contribution);
}

fn select_top(
    anchor_declaration_id: &str,
    accumulators: HashMap<CandidateId, CandidateAccumulator>,
    diagnostics: &mut RetrievalDiagnostics,
) -> Vec<SelectedCandidate> {
    let candidates = accumulators
        .into_values()
        .filter(|candidate| candidate.admitted)
        .collect::<Vec<_>>();
    diagnostics.generated_candidate_count += candidates.len();
    for candidate in &candidates {
        *diagnostics
            .candidate_count_by_generation_policy
            .entry(candidate.policy.label().to_owned())
            .or_default() += 1;
    }
    if candidates.len() > TOP_K_PER_WORKSPACE_DECLARATION {
        diagnostics.heap_truncations.push(HeapTruncation {
            anchor_declaration_id: anchor_declaration_id.to_owned(),
            dropped_count: candidates.len() - TOP_K_PER_WORKSPACE_DECLARATION,
        });
        *diagnostics
            .top_k_saturation_by_source_id
            .entry("symbolic-retrieval".to_owned())
            .or_default() += 1;
    }

    let symbolic_selection = select_by_score(&candidates, TOP_K_PER_WORKSPACE_DECLARATION, |candidate| {
        candidate.score
    });
    let symbolic_saturated = candidates.len() > TOP_K_PER_WORKSPACE_DECLARATION;
    let mut source_evidence_by_index = BTreeMap::<usize, Vec<CandidateSourceEvidence>>::new();
    for (rank, candidate_index) in symbolic_selection.iter().enumerate() {
        let candidate = &candidates[*candidate_index];
        source_evidence_by_index
            .entry(*candidate_index)
            .or_default()
            .push(selected_source_evidence(
                "symbolic-retrieval",
                CandidateSourceFamily::Symbolic,
                rank + 1,
                symbolic_saturated,
                candidate.contributions.values(),
            ));
    }
    for lane in SemanticCandidateLane::all() {
        let lane_candidates = candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| semantic_lane_score(candidate, lane) > 0.0)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if lane_candidates.is_empty() {
            continue;
        }
        let lane_saturated = lane_candidates.len() > SEMANTIC_LANE_TOP_K_PER_WORKSPACE_DECLARATION;
        if lane_saturated {
            *diagnostics
                .top_k_saturation_by_source_id
                .entry(lane.source_id().to_owned())
                .or_default() += 1;
        }
        let lane_selection = select_subset_by_score(
            &candidates,
            &lane_candidates,
            SEMANTIC_LANE_TOP_K_PER_WORKSPACE_DECLARATION,
            |candidate| semantic_lane_score(candidate, lane),
        );
        for (rank, candidate_index) in lane_selection.iter().enumerate() {
            let candidate = &candidates[*candidate_index];
            source_evidence_by_index
                .entry(*candidate_index)
                .or_default()
                .push(selected_source_evidence(
                    lane.source_id(),
                    CandidateSourceFamily::LeanSemantic,
                    rank + 1,
                    lane_saturated,
                    candidate
                        .contributions
                        .values()
                        .filter(|contribution| lane.accepts(contribution)),
                ));
        }
    }

    let mut selected = source_evidence_by_index
        .into_iter()
        .map(|(candidate_index, source_evidence)| {
            let candidate = candidates[candidate_index].clone();
            SelectedCandidate {
                id: candidate.id,
                score: candidate.score,
                contributions: candidate.contributions.into_values().collect(),
                source_evidence,
            }
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| candidate_sort_key(&left.id).cmp(&candidate_sort_key(&right.id)))
    });
    diagnostics.ranked_candidate_count += selected.len();
    selected
}

fn select_by_score(
    candidates: &[CandidateAccumulator],
    limit: usize,
    score: impl Fn(&CandidateAccumulator) -> f64,
) -> Vec<usize> {
    let indices = (0..candidates.len()).collect::<Vec<_>>();
    select_subset_by_score(candidates, &indices, limit, score)
}

fn select_subset_by_score(
    candidates: &[CandidateAccumulator],
    candidate_indices: &[usize],
    limit: usize,
    score: impl Fn(&CandidateAccumulator) -> f64,
) -> Vec<usize> {
    let mut heap: BinaryHeap<Reverse<HeapEntry>> = BinaryHeap::new();
    for candidate_index in candidate_indices {
        let candidate = &candidates[*candidate_index];
        let entry = HeapEntry {
            score_micros: (score(candidate) * 1_000_000.0).round() as i64,
            tie_breaker: candidate_sort_key(&candidate.id),
            candidate_index: *candidate_index,
        };
        if heap.len() < limit {
            heap.push(Reverse(entry));
        } else if let Some(mut smallest) = heap.peek_mut()
            && entry > smallest.0
        {
            *smallest = Reverse(entry);
        }
    }
    let mut selected = heap
        .into_iter()
        .map(|Reverse(entry)| entry.candidate_index)
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        score(&candidates[*right])
            .partial_cmp(&score(&candidates[*left]))
            .unwrap_or(Ordering::Equal)
            .then_with(|| candidate_sort_key(&candidates[*left].id).cmp(&candidate_sort_key(&candidates[*right].id)))
    });
    selected
}

fn semantic_lane_score(candidate: &CandidateAccumulator, lane: SemanticCandidateLane) -> f64 {
    candidate
        .contributions
        .values()
        .filter(|contribution| lane.accepts(contribution))
        .map(|contribution| contribution.score)
        .sum()
}

fn selected_source_evidence<'a>(
    source_id: &str,
    source_family: CandidateSourceFamily,
    generation_rank: usize,
    top_k_saturated: bool,
    contributions: impl Iterator<Item = &'a KeyContribution>,
) -> CandidateSourceEvidence {
    CandidateSourceEvidence {
        source_id: source_id.to_owned(),
        source_family,
        generation_rank: Some(generation_rank),
        top_k_status: CandidateTopKStatus::Selected,
        top_k_saturated,
        feature_families: feature_families_from_contributions(contributions),
    }
}

fn generated_pair_source_evidence<'a>(
    contributions: impl Iterator<Item = &'a KeyContribution> + Clone,
) -> Vec<CandidateSourceEvidence> {
    let mut source_evidence = vec![CandidateSourceEvidence {
        source_id: "symbolic-retrieval".to_owned(),
        source_family: CandidateSourceFamily::Symbolic,
        generation_rank: None,
        top_k_status: CandidateTopKStatus::GeneratedNotSelected,
        top_k_saturated: false,
        feature_families: feature_families_from_contributions(contributions.clone()),
    }];
    for lane in SemanticCandidateLane::all() {
        let feature_families = feature_families_from_contributions(
            contributions.clone().filter(|contribution| lane.accepts(contribution)),
        );
        if !feature_families.is_empty() {
            source_evidence.push(CandidateSourceEvidence {
                source_id: lane.source_id().to_owned(),
                source_family: CandidateSourceFamily::LeanSemantic,
                generation_rank: None,
                top_k_status: CandidateTopKStatus::GeneratedNotSelected,
                top_k_saturated: false,
                feature_families,
            });
        }
    }
    source_evidence
}

fn feature_families_from_contributions<'a>(contributions: impl Iterator<Item = &'a KeyContribution>) -> Vec<String> {
    let mut families = contributions
        .map(KeyContribution::feature_family)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    families.sort();
    families
}

fn policy_for_external(facts: &lean_dup_index::OpenedIndexFacts) -> CandidateGenerationPolicy {
    if facts.origin == "mathlib" {
        CandidateGenerationPolicy::MathlibComparison
    } else {
        match facts.provenance.kind {
            lean_dup_index::IndexProvenanceKind::Static => CandidateGenerationPolicy::StaticExternalComparison,
            lean_dup_index::IndexProvenanceKind::SourceBacked => {
                CandidateGenerationPolicy::SourceBackedExternalComparison
            }
        }
    }
}

fn record_pruned_feature_fanout(
    diagnostics: &mut RetrievalDiagnostics,
    policy: CandidateGenerationPolicy,
    source: &str,
    plan: &PlannedKey,
    count: usize,
) {
    diagnostics.pruned_feature_fanouts.push(PrunedFeatureFanout {
        policy: policy.label().to_owned(),
        source: source.to_owned(),
        reason: prune_reason_for_plan(plan).to_owned(),
        feature_family: plan.contribution.feature_family(),
        count,
    });
}

fn hydrate_external(
    indexes: &[OpenedIndex],
    external_needed: BTreeMap<usize, Vec<DeclarationHandle>>,
    diagnostics: &mut RetrievalDiagnostics,
) -> Result<HashMap<(usize, DeclarationHandle), HydratedDeclaration>> {
    let mut hydrated = HashMap::default();
    for (index, mut handles) in external_needed {
        handles.sort();
        handles.dedup();
        diagnostics.hydrated_external_count += handles.len();
        for declaration in indexes[index].hydrate(&handles)? {
            hydrated.insert((index, declaration.handle.clone()), declaration);
        }
    }
    Ok(hydrated)
}

fn pair_id(anchor: &HydratedDeclaration, candidate: &HydratedDeclaration) -> String {
    format!("{}::{}", anchor.declaration_id, candidate.declaration_id)
}

fn rarity_weight(total_documents: usize, document_frequency: usize) -> f64 {
    if document_frequency == 0 || total_documents == 0 {
        return 1.0;
    }
    let total = total_documents as f64 + 1.0;
    let frequency = document_frequency as f64 + 1.0;
    (1.0 + (total / frequency).ln()).max(1.0)
}

fn posting_limit(plan: &PlannedKey) -> usize {
    if plan.broad_head {
        BROAD_HEAD_POSTING_LIMIT
    } else if matches!(plan.key, SemanticFeatureKey::RoleFeature(_)) {
        ROLE_POSTING_LIMIT
    } else {
        usize::MAX
    }
}

fn posting_limit_for_key(workspace_plans: &[Vec<PlannedKey>], key: &SemanticFeatureKey) -> usize {
    workspace_plans
        .iter()
        .flat_map(|plans| plans.iter())
        .filter(|plan| &plan.key == key)
        .map(posting_limit)
        .min()
        .unwrap_or(usize::MAX)
}

fn prune_reason_for_plan(plan: &PlannedKey) -> &'static str {
    if plan.broad_head {
        "broad-posting"
    } else {
        "overwide-posting"
    }
}

fn contribution_sort_key(contribution: &KeyContribution) -> String {
    format!(
        "{}:{}:{}",
        contribution.kind,
        contribution.role.as_deref().unwrap_or(""),
        contribution.key
    )
}

fn candidate_sort_key(candidate: &CandidateId) -> String {
    match candidate {
        CandidateId::Workspace(index) => format!("workspace:{index:010}"),
        CandidateId::External { index, handle } => {
            format!("external:{index:010}:{}", handle.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Instant;

    use tempfile::TempDir;

    use super::retrieve_candidates;
    use lean_dup_diagnostics::progress::Reporter;
    use lean_dup_index::{IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore};
    use lean_dup_project::{WorkspaceRequest, resolve};
    use lean_dup_worker::{Fingerprints, WorkerClient};

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .to_path_buf()
    }

    fn build_index(
        cache: &TempDir,
        fixture: &str,
        module_root: &str,
        label: &str,
        origin: &str,
        kind: IndexBuildKind,
    ) -> (IndexStore, lean_dup_index::OpenedIndex) {
        let fixture_root = repo_root().join("tests/fixtures").join(fixture);
        let output = Command::new("lake")
            .arg("build")
            .current_dir(&fixture_root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let workspace = resolve(
            WorkspaceRequest {
                requested_root: fixture_root,
                module_root: Some(module_root.to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let store = IndexStore::new(cache.path().to_path_buf());
        store
            .build_or_reuse(
                IndexBuildRequest {
                    workspace,
                    execution_root: None,
                    label: label.to_owned(),
                    module_root: module_root.to_owned(),
                    origin: origin.to_owned(),
                    include_private: true,
                    include_generated: false,
                    require_oleans: false,
                    force: false,
                    kind,
                    max_heartbeats: None,
                },
                &WorkerClient::new(),
                &mut Reporter::new(false, false),
            )
            .unwrap();
        let opened = store.resolve(IndexReference::Label(label.to_owned())).unwrap();
        (store, opened)
    }

    fn hydrated_workspace(cache: &TempDir) -> Vec<lean_dup_index::HydratedDeclaration> {
        let (_store, opened) = build_index(cache, "tiny", "Tiny", "workspace", "workspace", IndexBuildKind::Local);
        let handles = opened.all_handles().unwrap();
        opened.hydrate(&handles).unwrap()
    }

    #[test]
    fn retrieves_fixture_exact_permutation_and_connective_hits() {
        let cache = TempDir::new().unwrap();
        let workspace = hydrated_workspace(&cache);
        let output = retrieve_candidates(&workspace, &[]).unwrap();

        let same = candidate(&output, "Tiny.same_left", "Tiny.same_right").expect("same statement candidate");
        assert!(has_contribution(same, "statement-fingerprint"));

        let permuted = candidate(&output, "Tiny.independent_arrow_left", "Tiny.independent_arrow_right")
            .expect("safe permutation candidate");
        assert!(has_contribution(permuted, "safe-permutation-fingerprint"));

        let connective =
            candidate(&output, "Tiny.connective_and_left", "Tiny.connective_and_right").expect("connective candidate");
        assert!(has_contribution(connective, "connective-fingerprint"));
    }

    #[test]
    fn broad_head_only_rows_do_not_create_candidates() {
        let cache = TempDir::new().unwrap();
        let workspace = hydrated_workspace(&cache);
        let seed = workspace
            .iter()
            .find(|row| row.qualified_name == "Tiny.broad_eq_only")
            .unwrap();
        let mut rows = Vec::new();
        for index in 0..70 {
            let mut row = seed.clone();
            row.declaration_id = format!("synthetic:broad:{index}");
            row.qualified_name = format!("Synthetic.broad_{index}");
            row.display_name = format!("broad_{index}");
            row.fingerprints = Fingerprints {
                statement: format!("synthetic.statement.{index}"),
                safe_binder_permutation: format!("synthetic.permutation.{index}"),
                connective_shape: format!("synthetic.connective.{index}"),
                conclusion_shape: format!("synthetic.conclusion.{index}"),
            };
            row.role_features
                .retain(|feature| feature.role == "conclusion_head" && feature.display.as_deref() == Some("Eq"));
            row.low_signal_markers = vec!["broad_head:Eq".to_owned()];
            rows.push(row);
        }

        let output = retrieve_candidates(&rows, &[]).unwrap();

        assert!(output.candidate_sets.is_empty());
        assert!(
            output
                .diagnostics
                .pruned_postings
                .iter()
                .any(|posting| posting.reason == "broad-posting" && posting.display.as_deref() == Some("Eq"))
        );
        assert!(
            output
                .diagnostics
                .pruned_feature_fanouts
                .iter()
                .any(|fanout| fanout.reason == "broad-posting" && fanout.feature_family == "role_head")
        );
    }

    #[test]
    fn generated_candidates_are_counted_before_top_k_selection() {
        let cache = TempDir::new().unwrap();
        let workspace = hydrated_workspace(&cache);
        let seed = workspace
            .iter()
            .find(|row| row.qualified_name == "Tiny.same_left")
            .unwrap();
        let mut rows = Vec::new();
        for index in 0..100 {
            let mut row = seed.clone();
            row.declaration_id = format!("synthetic:generated:{index}");
            row.qualified_name = format!("Synthetic.generated_{index}");
            row.display_name = format!("generated_{index}");
            row.handle = lean_dup_index::DeclarationHandle::from_fixture_id(row.declaration_id.clone());
            rows.push(row);
        }

        let output = retrieve_candidates(&rows, &[]).unwrap();

        assert!(output.diagnostics.generated_candidate_count > output.diagnostics.candidate_count);
        assert_eq!(
            output.diagnostics.ranked_candidate_count,
            output.diagnostics.candidate_count
        );
        assert_eq!(
            output
                .diagnostics
                .candidate_count_by_generation_policy
                .get("local_duplicate_audit"),
            Some(&output.diagnostics.generated_candidate_count)
        );
    }

    #[test]
    fn external_fixture_hydrates_only_surviving_candidates() {
        let cache = TempDir::new().unwrap();
        let workspace = hydrated_workspace(&cache);
        let (_external_store, external) = build_index(
            &cache,
            "external",
            "External",
            "fixture",
            "external:fixture",
            IndexBuildKind::External,
        );
        let external_count = external.facts().unwrap().declaration_count;

        let output = retrieve_candidates(&workspace, &[external]).unwrap();

        let external_match =
            candidate(&output, "Tiny.same_left", "External.same_as_tiny").expect("workspace/external exact candidate");
        assert!(has_contribution(external_match, "statement-fingerprint"));
        assert!(output.diagnostics.hydrated_external_count > 0);
        assert!(output.diagnostics.hydrated_external_count < external_count);
        assert!(output.candidate_sets.iter().all(|set| set.anchor.origin == "workspace"));
        assert!(output.candidate_sets.iter().all(|set| {
            set.candidates
                .iter()
                .all(|candidate| !(set.anchor.origin != "workspace" && candidate.declaration.origin != "workspace"))
        }));
    }

    #[test]
    #[ignore = "manual benchmark: records retrieval timing and candidate counts"]
    fn retrieval_benchmark_records_counts_and_time() {
        let cache = TempDir::new().unwrap();
        let workspace = hydrated_workspace(&cache);
        let seed = workspace
            .iter()
            .find(|row| row.qualified_name == "Tiny.same_left")
            .unwrap();
        let mut rows = Vec::new();
        for index in 0..400 {
            let mut row = seed.clone();
            row.declaration_id = format!("synthetic:medium:{index}");
            row.qualified_name = format!("Synthetic.medium_{index}");
            row.display_name = format!("medium_{index}");
            if index % 8 != 0 {
                row.fingerprints.statement = format!("synthetic.statement.{index}");
            }
            rows.push(row);
        }

        let started = Instant::now();
        let output = retrieve_candidates(&rows, &[]).unwrap();
        let elapsed = started.elapsed();

        eprintln!(
            "retrieval_benchmark candidate_count={} hydrated_external_count={} pruned_postings={} elapsed_ms={}",
            output.diagnostics.candidate_count,
            output.diagnostics.hydrated_external_count,
            output.diagnostics.pruned_postings.len(),
            elapsed.as_millis()
        );
        assert!(output.diagnostics.candidate_count > 0);
    }

    fn candidate<'a>(
        output: &'a super::RetrievalOutput,
        anchor: &str,
        candidate: &str,
    ) -> Option<&'a super::RetrievedCandidate> {
        output
            .candidate_sets
            .iter()
            .find(|set| set.anchor.qualified_name == anchor)
            .and_then(|set| {
                set.candidates
                    .iter()
                    .find(|item| item.declaration.qualified_name == candidate)
            })
    }

    fn has_contribution(candidate: &super::RetrievedCandidate, kind: &str) -> bool {
        candidate
            .explanation
            .contributions
            .iter()
            .any(|contribution| contribution.kind == kind)
    }
}
