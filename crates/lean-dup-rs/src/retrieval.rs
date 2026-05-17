use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::Serialize;

use crate::error::Result;
use crate::index::{
    DeclarationHandle, FingerprintKind, FingerprintQuery, HydratedDeclaration, OpenedIndex, PostingKey,
    RoleFeatureQuery,
};
use crate::perf::{self, CostClass};

const TOP_K_PER_WORKSPACE_DECLARATION: usize = 80;
const ROLE_POSTING_LIMIT: usize = 512;
const BROAD_HEAD_POSTING_LIMIT: usize = 64;

/// Candidate retrieval results for a workspace corpus.
///
/// The output is grouped by workspace declaration. Each group contains a
/// bounded, score-ordered set of declarations worth sending to later ranking
/// or semantic-probe stages, plus diagnostics that explain pruning and
/// hydration decisions.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub(crate) struct RetrievalOutput {
    pub(crate) candidate_sets: Vec<CandidateSet>,
    pub(crate) diagnostics: RetrievalDiagnostics,
}

/// Retrieved candidates for one workspace declaration.
///
/// The anchor is always a workspace declaration supplied by the caller.
/// Candidates may come from the same workspace corpus or from one of the
/// opened comparison indexes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub(crate) struct CandidateSet {
    pub(crate) anchor: HydratedDeclaration,
    pub(crate) candidates: Vec<RetrievedCandidate>,
}

/// One candidate declaration selected for a workspace declaration.
///
/// `score` is only an ordering signal for retrieval. Later stages decide
/// review priority, replacement actions, and report wording.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub(crate) struct RetrievedCandidate {
    pub(crate) pair_id: String,
    pub(crate) declaration: HydratedDeclaration,
    pub(crate) score: f64,
    pub(crate) explanation: CandidateExplanation,
}

/// Why retrieval selected a candidate.
///
/// The contributions are semantic keys emitted by Lean and matched through an
/// index. They are evidence for retrieval only, not proof certificates.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub(crate) struct CandidateExplanation {
    pub(crate) contributions: Vec<KeyContribution>,
}

/// Retrieval-level counters and pruning records.
///
/// Diagnostics are intended for audit logs and later `show` output. They
/// explain how much work was avoided without exposing the index storage shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[allow(dead_code)]
pub(crate) struct RetrievalDiagnostics {
    pub(crate) candidate_count: usize,
    pub(crate) hydrated_external_count: usize,
    pub(crate) pruned_postings: Vec<PrunedPosting>,
    pub(crate) heap_truncations: Vec<HeapTruncation>,
}

/// One semantic key contribution to a retrieved candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(dead_code)]
pub(crate) struct KeyContribution {
    pub(crate) kind: String,
    pub(crate) role: Option<String>,
    pub(crate) display: Option<String>,
    pub(crate) key: String,
    pub(crate) score: f64,
}

/// A semantic posting that retrieval chose not to expand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub(crate) struct PrunedPosting {
    pub(crate) anchor_declaration_id: String,
    pub(crate) source: String,
    pub(crate) reason: String,
    pub(crate) kind: String,
    pub(crate) role: Option<String>,
    pub(crate) display: Option<String>,
    pub(crate) count: usize,
}

/// Per-anchor count of lower-scoring candidates dropped from the bounded set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub(crate) struct HeapTruncation {
    pub(crate) anchor_declaration_id: String,
    pub(crate) dropped_count: usize,
}

/// Retrieve bounded candidate declarations for each workspace declaration.
///
/// Callers provide already-hydrated workspace declarations and any opened
/// comparison indexes. Retrieval returns semantic candidates and explanations;
/// it does not perform ranking, probing, reporting, or replacement-hint policy.
#[allow(dead_code)]
pub(crate) fn retrieve_candidates(
    workspace: &[HydratedDeclaration],
    indexes: &[OpenedIndex],
) -> Result<RetrievalOutput> {
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
    let index_facts = indexes.iter().map(OpenedIndex::facts).collect::<Result<Vec<_>>>()?;
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
    key: PostingKey,
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
    score: f64,
    admitted: bool,
    contributions: BTreeMap<String, KeyContribution>,
}

#[derive(Debug, Clone)]
struct SelectedCandidate {
    id: CandidateId,
    score: f64,
    contributions: Vec<KeyContribution>,
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
            key: PostingKey::RoleFeature(RoleFeatureQuery {
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

fn fingerprint_plan(kind: FingerprintKind, key: &str, label: &'static str, base_weight: f64) -> PlannedKey {
    PlannedKey {
        key: PostingKey::Fingerprint(FingerprintQuery {
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

fn local_postings(plans_by_decl: &[Vec<PlannedKey>]) -> HashMap<PostingKey, Vec<usize>> {
    let mut postings: HashMap<PostingKey, Vec<usize>> = HashMap::default();
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

fn local_counts(postings: &HashMap<PostingKey, Vec<usize>>) -> HashMap<PostingKey, usize> {
    postings
        .iter()
        .map(|(key, handles)| (key.clone(), handles.len()))
        .collect()
}

fn external_counts(
    indexes: &[OpenedIndex],
    workspace_plans: &[Vec<PlannedKey>],
) -> Result<HashMap<(usize, PostingKey), usize>> {
    let keys = unique_keys(workspace_plans);
    let mut counts = HashMap::default();
    for (index, opened) in indexes.iter().enumerate() {
        for count in opened.posting_counts(&keys)? {
            counts.insert((index, count.key), count.count);
        }
    }
    Ok(counts)
}

fn external_postings(
    indexes: &[OpenedIndex],
    workspace_plans: &[Vec<PlannedKey>],
    external_counts: &HashMap<(usize, PostingKey), usize>,
) -> Result<HashMap<(usize, PostingKey), Vec<DeclarationHandle>>> {
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
        for posting in opened.matched_postings(&selected)? {
            postings
                .entry((index, posting.key))
                .or_insert_with(Vec::new)
                .push(posting.handle);
        }
    }
    Ok(postings)
}

fn unique_keys(workspace_plans: &[Vec<PlannedKey>]) -> Vec<PostingKey> {
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
    local_postings: &HashMap<PostingKey, Vec<usize>>,
    local_counts: &HashMap<PostingKey, usize>,
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
        return;
    }
    for candidate_index in matches {
        if *candidate_index == anchor_index {
            continue;
        }
        let score = plan.base_weight * rarity_weight(workspace.len(), count);
        add_contribution(CandidateId::Workspace(*candidate_index), plan, score, accumulators);
    }
}

struct ExternalMatchContext<'a> {
    indexes: &'a [OpenedIndex],
    index_facts: &'a [crate::index::OpenedIndexFacts],
    counts: &'a HashMap<(usize, PostingKey), usize>,
    postings: &'a HashMap<(usize, PostingKey), Vec<DeclarationHandle>>,
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
                plan,
                score,
                accumulators,
            );
        }
    }
}

fn add_contribution(
    id: CandidateId,
    plan: &PlannedKey,
    score: f64,
    accumulators: &mut HashMap<CandidateId, CandidateAccumulator>,
) {
    if !plan.admits_candidate && !accumulators.contains_key(&id) {
        return;
    }
    let accumulator = accumulators.entry(id.clone()).or_insert_with(|| CandidateAccumulator {
        id,
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
    if candidates.len() > TOP_K_PER_WORKSPACE_DECLARATION {
        diagnostics.heap_truncations.push(HeapTruncation {
            anchor_declaration_id: anchor_declaration_id.to_owned(),
            dropped_count: candidates.len() - TOP_K_PER_WORKSPACE_DECLARATION,
        });
    }

    let mut heap: BinaryHeap<Reverse<HeapEntry>> = BinaryHeap::new();
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        let entry = HeapEntry {
            score_micros: (candidate.score * 1_000_000.0).round() as i64,
            tie_breaker: candidate_sort_key(&candidate.id),
            candidate_index,
        };
        if heap.len() < TOP_K_PER_WORKSPACE_DECLARATION {
            heap.push(Reverse(entry));
        } else if let Some(mut smallest) = heap.peek_mut()
            && entry > smallest.0
        {
            *smallest = Reverse(entry);
        }
    }

    let mut selected = heap
        .into_iter()
        .map(|Reverse(entry)| {
            let candidate = candidates[entry.candidate_index].clone();
            SelectedCandidate {
                id: candidate.id,
                score: candidate.score,
                contributions: candidate.contributions.into_values().collect(),
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
    selected
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
    } else if matches!(plan.key, PostingKey::RoleFeature(_)) {
        ROLE_POSTING_LIMIT
    } else {
        usize::MAX
    }
}

fn posting_limit_for_key(workspace_plans: &[Vec<PlannedKey>], key: &PostingKey) -> usize {
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
    use crate::index::{IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore};
    use crate::progress::Reporter;
    use crate::worker::{Fingerprints, WorkerClient};
    use crate::workspace::{WorkspaceRequest, resolve};

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
    ) -> (IndexStore, crate::index::OpenedIndex) {
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
                },
                &WorkerClient::new(),
                &mut Reporter::new(false, false),
            )
            .unwrap();
        let opened = store.resolve(IndexReference::Label(label.to_owned())).unwrap();
        (store, opened)
    }

    fn hydrated_workspace(cache: &TempDir) -> Vec<crate::index::HydratedDeclaration> {
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
    #[ignore = "records retrieval timing and candidate counts for prompt-11 performance checks"]
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
