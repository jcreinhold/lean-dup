use std::collections::BTreeMap;
use std::time::Duration;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cli::{ProbePolicy, ReviewProfile};
use crate::error::Result;
use crate::index::{HydratedDeclaration, OpenedIndex, ProbeCacheEntry};
use crate::progress::Reporter;
use crate::ranking::{ConfidenceTier, RankedReview, ReviewAction, ReviewPriority, ReviewRelation};
use crate::retrieval::{CandidateSet, RetrievedCandidate};
use crate::worker::{ModuleDescriptor, ProbeBatch, ProbePair, ProbeResult, WorkerClient, WorkerError};
use crate::workspace::ResolvedWorkspace;

const PROBE_CACHE_VERSION: &str = "semantic-probe-cache.v2";
const PROBE_POLICY_VERSION: &str = "semantic-probe-policy.v1";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// User-independent settings for bounded semantic verification.
///
/// The settings describe review-budget policy, not worker transport, Lean
/// reduction strategy, SQLite layout, or cache-key construction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeSettings {
    pub(crate) policy: ProbePolicy,
    pub(crate) budget: usize,
    pub(crate) per_declaration_cap: usize,
    pub(crate) chunk_size: usize,
}

/// Input for turning a cheap review queue into recoverable Lean probes.
///
/// Callers supply candidate and workspace facts. This module owns pair
/// selection, cache identity, worker chunking, heartbeat recovery, and
/// diagnostics.
pub(crate) struct SemanticVerificationInput<'a> {
    pub(crate) candidate_sets: &'a [CandidateSet],
    pub(crate) cheap_review: &'a RankedReview,
    pub(crate) local_index: VerificationIndex<'a>,
    pub(crate) workspace: &'a ResolvedWorkspace,
    pub(crate) mathlib_source: Option<&'a ResolvedWorkspace>,
    pub(crate) enabled: bool,
    pub(crate) settings: ProbeSettings,
}

/// Narrow index capability used by semantic verification.
///
/// The verifier needs an opaque probe cache and nothing about SQLite paths,
/// table names, declaration handles, or index construction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VerificationIndex<'a> {
    index: &'a OpenedIndex,
}

impl<'a> VerificationIndex<'a> {
    pub(crate) fn new(index: &'a OpenedIndex) -> Self {
        Self { index }
    }
}

/// Semantic verification output for ranking and diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ProbeVerification {
    pub(crate) results: BTreeMap<String, ProbeResult>,
    pub(crate) diagnostics: ProbeDiagnostics,
}

/// Counters that explain semantic-probe cost and pruning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ProbeDiagnostics {
    pub(crate) enabled: bool,
    pub(crate) policy: String,
    pub(crate) budget: usize,
    pub(crate) per_declaration_cap: usize,
    pub(crate) chunk_size: usize,
    pub(crate) candidates_considered: usize,
    pub(crate) planned_pairs: usize,
    pub(crate) skipped_by_policy: usize,
    pub(crate) skipped_by_budget: usize,
    pub(crate) cached_hits: usize,
    pub(crate) worker_pairs: usize,
    pub(crate) worker_batches: usize,
    pub(crate) recovered_failures: usize,
    pub(crate) unavailable_results: usize,
}

impl Default for ProbeDiagnostics {
    fn default() -> Self {
        Self {
            enabled: false,
            policy: "actionable".to_owned(),
            budget: 0,
            per_declaration_cap: 0,
            chunk_size: 0,
            candidates_considered: 0,
            planned_pairs: 0,
            skipped_by_policy: 0,
            skipped_by_budget: 0,
            cached_hits: 0,
            worker_pairs: 0,
            worker_batches: 0,
            recovered_failures: 0,
            unavailable_results: 0,
        }
    }
}

/// Return the candidate sets worth ranking for the requested default queue.
///
/// In the mathlib profile, feature-only mathlib overlaps are intentionally not
/// ranked unless the user asks for a broad/noise-oriented profile. This keeps
/// the default report actionable without changing retrieval's diagnostic
/// counters or index behavior.
pub(crate) fn candidate_sets_for_review(
    candidate_sets: &[CandidateSet],
    compare_mathlib: bool,
    review_profile: ReviewProfile,
    show_noise: bool,
) -> Vec<CandidateSet> {
    if !compare_mathlib || show_noise || review_profile != ReviewProfile::Mathlib {
        return candidate_sets.to_vec();
    }

    candidate_sets
        .iter()
        .filter_map(|set| {
            let candidates = set
                .candidates
                .iter()
                .filter(|candidate| candidate.declaration.origin == "mathlib")
                .filter(|candidate| strong_static_evidence(candidate))
                .cloned()
                .collect::<Vec<_>>();
            (!candidates.is_empty()).then(|| CandidateSet {
                anchor: set.anchor.clone(),
                candidates,
            })
        })
        .collect()
}

pub(crate) fn verify_candidate_probes(
    input: SemanticVerificationInput<'_>,
    reporter: &mut Reporter,
) -> Result<ProbeVerification> {
    let mut diagnostics = ProbeDiagnostics {
        enabled: input.enabled,
        policy: probe_policy_label(input.settings.policy).to_owned(),
        budget: input.settings.budget,
        per_declaration_cap: input.settings.per_declaration_cap,
        chunk_size: input.settings.chunk_size,
        ..ProbeDiagnostics::default()
    };
    if !input.enabled || input.settings.budget == 0 || input.settings.chunk_size == 0 {
        return Ok(ProbeVerification {
            results: BTreeMap::new(),
            diagnostics,
        });
    }

    let planned = plan_probes(&input, &mut diagnostics);
    reporter.event(
        "semantic.probe.plan",
        Some(planned.len() as u64),
        Some(input.settings.budget as u64),
        format!("planned {} semantic probe pairs", planned.len()),
    );

    let mut results = BTreeMap::new();
    let mut missing = Vec::new();
    for planned_probe in planned {
        if let Some(cached) = input.local_index.index.cached_probe_result(&planned_probe.cache_key)? {
            diagnostics.cached_hits += 1;
            results.insert(cached.pair_id.clone(), cached);
        } else {
            missing.push(planned_probe);
        }
    }
    if missing.is_empty() {
        diagnostics.unavailable_results = results.values().filter(|result| result.status != "ok").count();
        return Ok(ProbeVerification { results, diagnostics });
    }

    let worker = WorkerClient::with_timeout(PROBE_TIMEOUT);
    for chunk in missing.chunks(input.settings.chunk_size) {
        run_probe_chunk(chunk, &input, &worker, reporter, &mut results, &mut diagnostics)?;
    }
    diagnostics.unavailable_results = results.values().filter(|result| result.status != "ok").count();
    Ok(ProbeVerification { results, diagnostics })
}

#[derive(Debug, Clone)]
struct PlannedProbe {
    pair: ProbePair,
    cache_key: String,
    right_module: String,
    right_origin: String,
}

fn plan_probes(input: &SemanticVerificationInput<'_>, diagnostics: &mut ProbeDiagnostics) -> Vec<PlannedProbe> {
    let groups = input
        .cheap_review
        .groups
        .iter()
        .map(|group| (group.pair_id.as_str(), group))
        .collect::<HashMap<_, _>>();
    let mut candidates = Vec::new();
    for set in input.candidate_sets {
        for candidate in &set.candidates {
            diagnostics.candidates_considered += 1;
            let Some(group) = groups.get(candidate.pair_id.as_str()) else {
                diagnostics.skipped_by_policy += 1;
                continue;
            };
            if !probe_supported_origin(&candidate.declaration) || !probe_supported_origin(&set.anchor) {
                diagnostics.skipped_by_policy += 1;
                continue;
            }
            if !probe_supported_kind(&candidate.declaration) || !probe_supported_kind(&set.anchor) {
                diagnostics.skipped_by_policy += 1;
                continue;
            }
            if !eligible_for_policy(input.settings.policy, candidate, group) {
                diagnostics.skipped_by_policy += 1;
                continue;
            }
            candidates.push((set, candidate, *group));
        }
    }

    candidates.sort_by(|left, right| {
        left.2
            .review_priority
            .cmp(&right.2.review_priority)
            .then_with(|| left.2.confidence.cmp(&right.2.confidence))
            .then_with(|| {
                right
                    .1
                    .score
                    .partial_cmp(&left.1.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.1.pair_id.cmp(&right.1.pair_id))
    });

    let mut planned = Vec::new();
    let mut per_declaration = HashMap::<String, usize>::default();
    for (set, candidate, _) in candidates {
        if planned.len() >= input.settings.budget {
            diagnostics.skipped_by_budget += 1;
            continue;
        }
        let count = per_declaration.entry(set.anchor.declaration_id.clone()).or_default();
        if *count >= input.settings.per_declaration_cap {
            diagnostics.skipped_by_budget += 1;
            continue;
        }
        *count += 1;
        let pair = ProbePair {
            pair_id: candidate.pair_id.clone(),
            left_declaration_id: set.anchor.declaration_id.clone(),
            right_declaration_id: candidate.declaration.declaration_id.clone(),
        };
        planned.push(PlannedProbe {
            cache_key: probe_cache_key(&pair, &set.anchor, &candidate.declaration, input.settings.policy),
            pair,
            right_module: candidate.declaration.module.clone(),
            right_origin: candidate.declaration.origin.clone(),
        });
    }
    diagnostics.planned_pairs = planned.len();
    planned
}

fn run_probe_chunk(
    chunk: &[PlannedProbe],
    input: &SemanticVerificationInput<'_>,
    worker: &WorkerClient,
    reporter: &mut Reporter,
    results: &mut BTreeMap<String, ProbeResult>,
    diagnostics: &mut ProbeDiagnostics,
) -> Result<()> {
    diagnostics.worker_batches += 1;
    diagnostics.worker_pairs += chunk.len();
    reporter.event(
        "semantic.probe.chunk",
        Some(diagnostics.worker_pairs as u64),
        Some(diagnostics.planned_pairs as u64),
        format!("probing {} candidate pairs", chunk.len()),
    );
    let pairs = chunk.iter().map(|planned| planned.pair.clone()).collect::<Vec<_>>();
    let modules = probe_modules_for(input.workspace, input.mathlib_source, chunk);
    match worker.probe_batch(ProbeBatch {
        workspace_root: input.workspace.root.clone(),
        modules,
        pairs,
        max_pairs: Some(chunk.len() as u64),
    }) {
        Ok(call) => {
            let by_pair = chunk
                .iter()
                .map(|planned| (planned.pair.pair_id.as_str(), planned))
                .collect::<HashMap<_, _>>();
            let entries = call
                .rows
                .iter()
                .filter_map(|result| {
                    by_pair.get(result.pair_id.as_str()).map(|planned| ProbeCacheEntry {
                        cache_key: planned.cache_key.clone(),
                        pair: planned.pair.clone(),
                        result: result.clone(),
                    })
                })
                .collect::<Vec<_>>();
            input.local_index.index.cache_probe_results(&entries)?;
            for result in call.rows {
                results.insert(result.pair_id.clone(), result);
            }
            Ok(())
        }
        Err(error) if recoverable_probe_error(&error) && chunk.len() > 1 => {
            diagnostics.recovered_failures += 1;
            let midpoint = chunk.len() / 2;
            run_probe_chunk(&chunk[..midpoint], input, worker, reporter, results, diagnostics)?;
            run_probe_chunk(&chunk[midpoint..], input, worker, reporter, results, diagnostics)
        }
        Err(error) if recoverable_probe_error(&error) => {
            diagnostics.recovered_failures += 1;
            let planned = &chunk[0];
            let result = unavailable_probe_result(&planned.pair, &error);
            input.local_index.index.cache_probe_results(&[ProbeCacheEntry {
                cache_key: planned.cache_key.clone(),
                pair: planned.pair.clone(),
                result: result.clone(),
            }])?;
            results.insert(result.pair_id.clone(), result);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn probe_modules_for(
    workspace: &ResolvedWorkspace,
    mathlib_source: Option<&ResolvedWorkspace>,
    chunk: &[PlannedProbe],
) -> Vec<ModuleDescriptor> {
    let mut modules = workspace
        .source_files
        .iter()
        .map(|source| ModuleDescriptor {
            module: source.module.clone(),
            origin: "workspace".to_owned(),
            source_root: None,
        })
        .collect::<Vec<_>>();

    let Some(mathlib_source) = mathlib_source else {
        return modules;
    };
    let mut seen = HashSet::default();
    for planned in chunk {
        if planned.right_origin == "mathlib" && seen.insert(planned.right_module.clone()) {
            modules.push(ModuleDescriptor {
                module: planned.right_module.clone(),
                origin: "mathlib".to_owned(),
                source_root: Some(mathlib_source.root.clone()),
            });
        }
    }
    modules.sort_by(|left, right| {
        left.origin
            .cmp(&right.origin)
            .then_with(|| left.module.cmp(&right.module))
    });
    modules
}

fn eligible_for_policy(
    policy: ProbePolicy,
    candidate: &RetrievedCandidate,
    group: &crate::ranking::RankedGroup,
) -> bool {
    if policy == ProbePolicy::Broad {
        return true;
    }
    if group.blockers.iter().any(|blocker| {
        matches!(
            blocker.as_str(),
            "generated-declaration" | "broad-head-only" | "weak-feature-overlap"
        )
    }) {
        return false;
    }
    if strong_static_evidence(candidate) {
        return true;
    }
    if matches!(
        group.relation,
        ReviewRelation::ExactStatement | ReviewRelation::PermutedStatement | ReviewRelation::ConnectiveEquivalent
    ) {
        return true;
    }
    matches!(
        group.recommended_action,
        ReviewAction::AlreadyInMathlib | ReviewAction::ReplaceLocalUses | ReviewAction::LocalAlias
    ) && matches!(group.confidence, ConfidenceTier::High | ConfidenceTier::Medium)
        && group.review_priority <= ReviewPriority::Medium
}

fn strong_static_evidence(candidate: &RetrievedCandidate) -> bool {
    if !probe_supported_kind(&candidate.declaration) {
        return false;
    }
    candidate.explanation.contributions.iter().any(|contribution| {
        matches!(
            contribution.kind.as_str(),
            "statement-fingerprint" | "safe-permutation-fingerprint" | "connective-fingerprint"
        )
    })
}

fn probe_supported_origin(declaration: &HydratedDeclaration) -> bool {
    matches!(declaration.origin.as_str(), "workspace" | "mathlib")
}

fn probe_supported_kind(declaration: &HydratedDeclaration) -> bool {
    matches!(declaration.kind.as_str(), "theorem" | "axiom" | "def" | "abbrev")
}

fn recoverable_probe_error(error: &WorkerError) -> bool {
    match error {
        WorkerError::WorkerDiagnostic { diagnostics } | WorkerError::EofBeforeComplete { diagnostics } => {
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code.contains("internal")
                    || diagnostic.message.contains("declaration processing failed")
                    || diagnostic.message.contains("heartbeat")
                    || diagnostic.message.contains("timeout")
                    || diagnostic.message.contains("maximum number of heartbeats")
            }) && diagnostics.iter().all(|diagnostic| !diagnostic.code.contains("import"))
        }
        WorkerError::Timeout { .. } | WorkerError::NonZeroExit { .. } => true,
        WorkerError::Protocol { .. } | WorkerError::Cancelled | WorkerError::InvalidJsonLine { .. } => false,
        WorkerError::Io { .. } | WorkerError::BuildFailed { .. } => false,
    }
}

fn unavailable_probe_result(pair: &ProbePair, error: &WorkerError) -> ProbeResult {
    ProbeResult {
        pair_id: pair.pair_id.clone(),
        left_declaration_id: pair.left_declaration_id.clone(),
        right_declaration_id: pair.right_declaration_id.clone(),
        status: "unavailable".to_owned(),
        same_statement: false,
        same_up_to_safe_reordering: false,
        connective_equivalent: false,
        specializes_left_to_right: false,
        specializes_right_to_left: false,
        mutual_implication_shape: false,
        same_reducible_definition: false,
        message: Some(format!("probe isolated after worker failure: {error}")),
    }
}

fn probe_policy_label(policy: ProbePolicy) -> &'static str {
    match policy {
        ProbePolicy::Actionable => "actionable",
        ProbePolicy::Broad => "broad",
    }
}

fn probe_cache_key(
    pair: &ProbePair,
    left: &HydratedDeclaration,
    right: &HydratedDeclaration,
    policy: ProbePolicy,
) -> String {
    let payload = serde_json::json!({
        "cache_version": PROBE_CACHE_VERSION,
        "policy_version": PROBE_POLICY_VERSION,
        "policy": probe_policy_label(policy),
        "pair": pair,
        "left": declaration_cache_facts(left),
        "right": declaration_cache_facts(right),
    });
    let encoded = serde_json::to_vec(&payload).expect("probe cache key ingredients serialize");
    let digest = Sha256::digest(encoded);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn declaration_cache_facts(declaration: &HydratedDeclaration) -> serde_json::Value {
    serde_json::json!({
        "declaration_id": declaration.declaration_id,
        "feature_version": declaration.feature_version,
        "fingerprints": declaration.fingerprints,
        "binder_count": declaration.binder_count,
    })
}

#[cfg(test)]
mod tests {
    use super::{ProbeSettings, SemanticVerificationInput, VerificationIndex, candidate_sets_for_review, plan_probes};
    use crate::cli::{ProbePolicy, ReviewProfile};
    use crate::index::{DeclarationHandle, HydratedDeclaration};
    use crate::ranking::{RankingInput, RankingProfile, rank_candidates};
    use crate::retrieval::{CandidateExplanation, CandidateSet, KeyContribution, RetrievedCandidate};
    use crate::source_refs::SourceFacts;
    use crate::worker::Fingerprints;
    use crate::workspace::ResolvedWorkspace;

    #[test]
    fn mathlib_review_shape_drops_feature_only_candidates() {
        let anchor = declaration("workspace:Tiny:Tiny.local", "workspace", "Tiny.local");
        let exact = candidate(
            declaration("mathlib:Mathlib:Mathlib.exact", "mathlib", "Mathlib.exact"),
            "statement-fingerprint",
            100.0,
        );
        let broad = candidate(
            declaration("mathlib:Mathlib:Mathlib.broad", "mathlib", "Mathlib.broad"),
            "role-feature",
            12.0,
        );
        let shaped = candidate_sets_for_review(
            &[CandidateSet {
                anchor,
                candidates: vec![exact.clone(), broad],
            }],
            true,
            ReviewProfile::Mathlib,
            false,
        );

        assert_eq!(shaped[0].candidates, vec![exact]);
    }

    #[test]
    fn broad_policy_restores_feature_only_probe_candidates() {
        let anchor = declaration("workspace:Tiny:Tiny.local", "workspace", "Tiny.local");
        let candidate = candidate(
            declaration("mathlib:Mathlib:Mathlib.broad", "mathlib", "Mathlib.broad"),
            "role-feature",
            40.0,
        );
        let candidate_sets = vec![CandidateSet {
            anchor,
            candidates: vec![candidate],
        }];
        let review = rank_candidates(RankingInput {
            candidate_sets: &candidate_sets,
            probe_results: &std::collections::BTreeMap::new(),
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
        });
        let mut diagnostics = super::ProbeDiagnostics::default();
        let index = empty_index();
        let input = SemanticVerificationInput {
            candidate_sets: &candidate_sets,
            cheap_review: &review,
            local_index: VerificationIndex::new(&index),
            workspace: &workspace(),
            mathlib_source: None,
            enabled: true,
            settings: ProbeSettings {
                policy: ProbePolicy::Broad,
                budget: 10,
                per_declaration_cap: 2,
                chunk_size: 16,
            },
        };

        assert_eq!(plan_probes(&input, &mut diagnostics).len(), 1);
    }

    #[test]
    fn actionable_policy_enforces_budget_and_per_declaration_cap() {
        let anchor = declaration("workspace:Tiny:Tiny.local", "workspace", "Tiny.local");
        let candidates = (0..3)
            .map(|index| {
                candidate(
                    declaration(
                        &format!("mathlib:Mathlib:Mathlib.exact{index}"),
                        "mathlib",
                        &format!("Mathlib.exact{index}"),
                    ),
                    "statement-fingerprint",
                    100.0 - index as f64,
                )
            })
            .collect::<Vec<_>>();
        let candidate_sets = vec![CandidateSet { anchor, candidates }];
        let review = rank_candidates(RankingInput {
            candidate_sets: &candidate_sets,
            probe_results: &std::collections::BTreeMap::new(),
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
        });
        let mut diagnostics = super::ProbeDiagnostics::default();
        let index = empty_index();
        let input = SemanticVerificationInput {
            candidate_sets: &candidate_sets,
            cheap_review: &review,
            local_index: VerificationIndex::new(&index),
            workspace: &workspace(),
            mathlib_source: None,
            enabled: true,
            settings: ProbeSettings {
                policy: ProbePolicy::Actionable,
                budget: 10,
                per_declaration_cap: 2,
                chunk_size: 16,
            },
        };

        assert_eq!(plan_probes(&input, &mut diagnostics).len(), 2);
        assert_eq!(diagnostics.skipped_by_budget, 1);
    }

    fn empty_index() -> crate::index::OpenedIndex {
        crate::index::OpenedIndex::for_test(std::path::PathBuf::from("/tmp/nonexistent/index.sqlite"))
    }

    fn workspace() -> ResolvedWorkspace {
        ResolvedWorkspace {
            requested_root: std::path::PathBuf::from("/tmp/project"),
            root: std::path::PathBuf::from("/tmp/project"),
            lakefile: std::path::PathBuf::from("/tmp/project/lakefile.toml"),
            module_roots: vec!["Tiny".to_owned()],
            selected_roots: vec!["Tiny".to_owned()],
            source_files: Vec::new(),
        }
    }

    fn candidate(declaration: HydratedDeclaration, contribution_kind: &str, score: f64) -> RetrievedCandidate {
        RetrievedCandidate {
            pair_id: format!("workspace:Tiny:Tiny.local::{}", declaration.declaration_id),
            declaration,
            score,
            explanation: CandidateExplanation {
                contributions: vec![KeyContribution {
                    kind: contribution_kind.to_owned(),
                    role: Some("conclusion_head".to_owned()),
                    display: Some("Eq".to_owned()),
                    key: contribution_kind.to_owned(),
                    score,
                }],
            },
        }
    }

    fn declaration(id: &str, origin: &str, name: &str) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::for_test(id),
            declaration_id: id.to_owned(),
            origin: origin.to_owned(),
            module: name.rsplit_once('.').map(|(module, _)| module).unwrap_or("").to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap().to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: "theorem".to_owned(),
            status_flags: Vec::new(),
            feature_version: "features.roles.v1".to_owned(),
            fingerprints: Fingerprints {
                statement: "statement".to_owned(),
                safe_binder_permutation: "permutation".to_owned(),
                connective_shape: "connective".to_owned(),
                conclusion_shape: "conclusion".to_owned(),
            },
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: Vec::new(),
        }
    }
}
