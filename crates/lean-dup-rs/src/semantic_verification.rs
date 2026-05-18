use std::collections::BTreeMap;
use std::time::Duration;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cli::{ProbePolicy, ReviewProfile};
use crate::error::Result;
use crate::external_provenance::ComparisonEvidencePolicy;
use crate::index::{HydratedDeclaration, OpenedIndex, ProbeCacheEntry};
use crate::progress::Reporter;
use crate::ranking::{ConfidenceTier, RankedReview, ReviewAction, ReviewPriority, ReviewRelation};
use crate::retrieval::{CandidateSet, RetrievedCandidate};
use crate::worker::{ModuleDescriptor, ProbeBatch, ProbePair, ProbeResult, WorkerClient, WorkerError};
use crate::workspace::ResolvedWorkspace;

const PROBE_CACHE_VERSION: &str = "semantic-probe-cache.v3";
const PROBE_POLICY_VERSION: &str = "semantic-probe-policy.v2";
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
    pub(crate) comparison_policy: &'a ComparisonEvidencePolicy,
    pub(crate) enabled: bool,
    pub(crate) include_private: bool,
    pub(crate) include_generated: bool,
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
    pub(crate) evidence: BTreeMap<String, SemanticEvidence>,
    pub(crate) diagnostics: ProbeDiagnostics,
}

/// Verified or intentionally unavailable semantic evidence for one review pair.
///
/// Ranking consumes this type instead of worker probe rows. That keeps worker
/// status strings, JSONL fields, cache keys, and Lean recovery policy inside
/// semantic verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SemanticEvidence {
    pub(crate) pair_id: String,
    pub(crate) kind: EvidenceKind,
    pub(crate) status: EvidenceStatus,
    pub(crate) summary: Option<String>,
}

impl SemanticEvidence {
    pub(crate) fn proof_grade(&self) -> bool {
        self.status == EvidenceStatus::Verified
    }

    fn rejected(pair_id: String, kind: EvidenceKind, summary: impl Into<String>) -> Self {
        Self {
            pair_id,
            kind,
            status: EvidenceStatus::Rejected,
            summary: Some(summary.into()),
        }
    }
}

/// User-meaningful semantic finding kinds. These are not worker protocol fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EvidenceKind {
    ExactTheorem,
    PermutedTheorem,
    Replacement,
    ReducibleDefinition,
    Specialization,
    LocalDuplicate,
    Unavailable,
}

/// Whether a planned proof obligation produced usable evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EvidenceStatus {
    Verified,
    Unavailable,
    Rejected,
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
    pub(crate) cheap_summary_rejects: usize,
    pub(crate) planned_exact_theorem: usize,
    pub(crate) planned_permuted_theorem: usize,
    pub(crate) planned_replacement: usize,
    pub(crate) planned_reducible_definition: usize,
    pub(crate) planned_specialization: usize,
    pub(crate) planned_local_duplicate: usize,
    pub(crate) cached_hits: usize,
    pub(crate) worker_pairs: usize,
    pub(crate) worker_batches: usize,
    pub(crate) recovered_failures: usize,
    pub(crate) unavailable_results: usize,
    pub(crate) unavailable_unsupported: usize,
    pub(crate) unavailable_missing: usize,
    pub(crate) unavailable_timeout: usize,
    pub(crate) unavailable_internal: usize,
    pub(crate) unavailable_by_reason: BTreeMap<String, usize>,
    pub(crate) unavailable_by_obligation: BTreeMap<String, usize>,
    pub(crate) unavailable_by_module: BTreeMap<String, usize>,
    pub(crate) unavailable_by_origin: BTreeMap<String, usize>,
    pub(crate) verified_results: usize,
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
            cheap_summary_rejects: 0,
            planned_exact_theorem: 0,
            planned_permuted_theorem: 0,
            planned_replacement: 0,
            planned_reducible_definition: 0,
            planned_specialization: 0,
            planned_local_duplicate: 0,
            cached_hits: 0,
            worker_pairs: 0,
            worker_batches: 0,
            recovered_failures: 0,
            unavailable_results: 0,
            unavailable_unsupported: 0,
            unavailable_missing: 0,
            unavailable_timeout: 0,
            unavailable_internal: 0,
            unavailable_by_reason: BTreeMap::new(),
            unavailable_by_obligation: BTreeMap::new(),
            unavailable_by_module: BTreeMap::new(),
            unavailable_by_origin: BTreeMap::new(),
            verified_results: 0,
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
    if show_noise || review_profile != ReviewProfile::Mathlib {
        return candidate_sets.to_vec();
    }

    candidate_sets
        .iter()
        .filter_map(|set| {
            let candidates = set
                .candidates
                .iter()
                .filter(|candidate| !compare_mathlib || candidate.declaration.origin == "mathlib")
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
            evidence: BTreeMap::new(),
            diagnostics,
        });
    }

    let plan = plan_probes(&input, &mut diagnostics);
    reporter.event(
        "semantic.probe.plan",
        Some(plan.planned.len() as u64),
        Some(input.settings.budget as u64),
        format!("planned {} semantic probe pairs", plan.planned.len()),
    );

    let mut evidence = plan.preflight_evidence;
    let mut missing = Vec::new();
    for planned_probe in plan.planned {
        if let Some(cached) = input.local_index.index.cached_probe_result(&planned_probe.cache_key)? {
            diagnostics.cached_hits += 1;
            let semantic = semantic_evidence(&planned_probe, &cached);
            record_evidence_diagnostic(&semantic, &planned_probe, &mut diagnostics);
            evidence.insert(semantic.pair_id.clone(), semantic);
        } else {
            missing.push(planned_probe);
        }
    }
    if missing.is_empty() {
        return Ok(ProbeVerification { evidence, diagnostics });
    }

    let worker = WorkerClient::with_timeout(PROBE_TIMEOUT);
    for chunk in missing.chunks(input.settings.chunk_size) {
        run_probe_chunk(chunk, &input, &worker, reporter, &mut evidence, &mut diagnostics)?;
    }
    Ok(ProbeVerification { evidence, diagnostics })
}

#[derive(Debug, Clone)]
struct PlannedProbe {
    pair: ProbePair,
    cache_key: String,
    obligation: ProbeObligation,
    right_module: String,
    right_origin: String,
    left_module: String,
    left_origin: String,
    include_private: bool,
    include_generated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProbeObligation {
    ExactTheorem,
    PermutedTheorem,
    Replacement,
    ReducibleDefinition,
    Specialization,
    LocalDuplicate,
}

#[derive(Debug, Clone, Default)]
struct ProbePlan {
    planned: Vec<PlannedProbe>,
    preflight_evidence: BTreeMap<String, SemanticEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeDecision {
    Worker(ProbeObligation),
    Unavailable(UnavailableReason),
}

impl ProbeDecision {
    fn order_key(self) -> ProbeObligation {
        match self {
            Self::Worker(obligation) => obligation,
            Self::Unavailable(_) => ProbeObligation::LocalDuplicate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnavailableReason {
    MissingDeclaration,
    Unsupported,
    OpaqueOrUnreducible,
    Timeout,
    InternalError,
}

impl UnavailableReason {
    fn label(self) -> &'static str {
        match self {
            Self::MissingDeclaration => "missing-declaration",
            Self::Unsupported => "unsupported",
            Self::OpaqueOrUnreducible => "opaque-or-unreducible",
            Self::Timeout => "timeout",
            Self::InternalError => "internal-error",
        }
    }
}

#[derive(Debug, Clone)]
struct DeclarationProbeSummary {
    origin: String,
    module: String,
    kind: String,
    visibility: String,
    generated: bool,
    importable: bool,
}

impl DeclarationProbeSummary {
    fn from_declaration(declaration: &HydratedDeclaration, policy: &ComparisonEvidencePolicy) -> Self {
        Self {
            origin: declaration.origin.clone(),
            module: declaration.module.clone(),
            kind: declaration.kind.clone(),
            visibility: declaration.visibility.clone(),
            generated: declaration.status_flags.iter().any(|flag| flag == "generated"),
            importable: probe_supported_origin(declaration, policy),
        }
    }

    fn theorem_like(&self) -> bool {
        matches!(self.kind.as_str(), "theorem" | "axiom")
    }

    fn definition_like(&self) -> bool {
        matches!(self.kind.as_str(), "def" | "abbrev")
    }

    fn probe_supported_kind(&self) -> bool {
        matches!(self.kind.as_str(), "theorem" | "axiom" | "def" | "abbrev")
    }

    fn needs_private_filter(&self) -> bool {
        self.visibility == "private"
    }
}

fn plan_probes(input: &SemanticVerificationInput<'_>, diagnostics: &mut ProbeDiagnostics) -> ProbePlan {
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
            if !eligible_for_policy(input.settings.policy, candidate, group) {
                diagnostics.skipped_by_policy += 1;
                continue;
            }
            let left = DeclarationProbeSummary::from_declaration(&set.anchor, input.comparison_policy);
            let right = DeclarationProbeSummary::from_declaration(&candidate.declaration, input.comparison_policy);
            if !left.importable || !right.importable {
                candidates.push((
                    set,
                    candidate,
                    *group,
                    ProbeDecision::Unavailable(UnavailableReason::MissingDeclaration),
                    left,
                    right,
                ));
                continue;
            }
            if !left.probe_supported_kind() || !right.probe_supported_kind() {
                candidates.push((
                    set,
                    candidate,
                    *group,
                    ProbeDecision::Unavailable(UnavailableReason::Unsupported),
                    left,
                    right,
                ));
                continue;
            }
            let Some(obligation) = probe_obligation(input.settings.policy, &left, &right, candidate, group) else {
                diagnostics.cheap_summary_rejects += 1;
                continue;
            };
            candidates.push((set, candidate, *group, ProbeDecision::Worker(obligation), left, right));
        }
    }

    candidates.sort_by(|left, right| {
        left.3
            .order_key()
            .cmp(&right.3.order_key())
            .then_with(|| left.2.review_priority.cmp(&right.2.review_priority))
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
    let mut preflight_evidence = BTreeMap::new();
    let mut per_declaration = HashMap::<String, usize>::default();
    for (set, candidate, _, decision, left, right) in candidates {
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
        match decision {
            ProbeDecision::Worker(obligation) => {
                record_planned_obligation(obligation, diagnostics);
                let include_private =
                    input.include_private || left.needs_private_filter() || right.needs_private_filter();
                let include_generated = input.include_generated || left.generated || right.generated;
                planned.push(PlannedProbe {
                    cache_key: probe_cache_key(
                        &pair,
                        &set.anchor,
                        &candidate.declaration,
                        input.settings.policy,
                        obligation,
                    ),
                    pair,
                    obligation,
                    right_module: right.module,
                    right_origin: right.origin,
                    left_module: left.module,
                    left_origin: left.origin,
                    include_private,
                    include_generated,
                });
            }
            ProbeDecision::Unavailable(reason) => {
                let synthetic = PlannedProbe {
                    cache_key: String::new(),
                    pair: pair.clone(),
                    obligation: ProbeObligation::LocalDuplicate,
                    right_module: right.module,
                    right_origin: right.origin,
                    left_module: left.module,
                    left_origin: left.origin,
                    include_private: false,
                    include_generated: false,
                };
                let semantic = SemanticEvidence {
                    pair_id: pair.pair_id,
                    kind: EvidenceKind::Unavailable,
                    status: EvidenceStatus::Unavailable,
                    summary: Some(reason.label().to_owned()),
                };
                record_unavailable_reason(reason, &synthetic, diagnostics);
                preflight_evidence.insert(semantic.pair_id.clone(), semantic);
            }
        }
    }
    diagnostics.planned_pairs = planned.len();
    ProbePlan {
        planned,
        preflight_evidence,
    }
}

fn run_probe_chunk(
    chunk: &[PlannedProbe],
    input: &SemanticVerificationInput<'_>,
    worker: &WorkerClient,
    reporter: &mut Reporter,
    evidence: &mut BTreeMap<String, SemanticEvidence>,
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
    let modules = probe_modules_for(input.workspace, input.comparison_policy, chunk);
    match worker.probe_batch(ProbeBatch {
        workspace_root: input.workspace.root.clone(),
        modules,
        include_private: input.include_private || chunk.iter().any(|planned| planned.include_private),
        include_generated: input.include_generated || chunk.iter().any(|planned| planned.include_generated),
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
                if let Some(planned) = by_pair.get(result.pair_id.as_str()) {
                    let semantic = semantic_evidence(planned, &result);
                    record_evidence_diagnostic(&semantic, planned, diagnostics);
                    evidence.insert(semantic.pair_id.clone(), semantic);
                }
            }
            Ok(())
        }
        Err(error) if recoverable_probe_error(&error) && chunk.len() > 1 => {
            diagnostics.recovered_failures += 1;
            let midpoint = chunk.len() / 2;
            run_probe_chunk(&chunk[..midpoint], input, worker, reporter, evidence, diagnostics)?;
            run_probe_chunk(&chunk[midpoint..], input, worker, reporter, evidence, diagnostics)
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
            let semantic = semantic_evidence(planned, &result);
            record_evidence_diagnostic(&semantic, planned, diagnostics);
            evidence.insert(semantic.pair_id.clone(), semantic);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn probe_modules_for(
    workspace: &ResolvedWorkspace,
    comparison_policy: &ComparisonEvidencePolicy,
    chunk: &[PlannedProbe],
) -> Vec<ModuleDescriptor> {
    let workspace_modules = workspace
        .source_files
        .iter()
        .map(|source| (source.module.as_str(), source))
        .collect::<HashMap<_, _>>();

    let mut modules = Vec::new();
    let mut seen = HashSet::default();
    for planned in chunk {
        for (origin, module) in [
            (planned.left_origin.as_str(), planned.left_module.as_str()),
            (planned.right_origin.as_str(), planned.right_module.as_str()),
        ] {
            if !seen.insert((origin.to_owned(), module.to_owned())) {
                continue;
            }
            if origin == "workspace" {
                if workspace_modules.contains_key(module) {
                    modules.push(ModuleDescriptor {
                        module: module.to_owned(),
                        origin: "workspace".to_owned(),
                        source_root: None,
                    });
                }
            } else if let Some(descriptor) = comparison_policy.probe_module(origin, module) {
                modules.push(descriptor);
            }
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

fn probe_obligation(
    policy: ProbePolicy,
    anchor: &DeclarationProbeSummary,
    right: &DeclarationProbeSummary,
    candidate: &RetrievedCandidate,
    group: &crate::ranking::RankedGroup,
) -> Option<ProbeObligation> {
    if anchor.theorem_like() && right.theorem_like() {
        if has_contribution(candidate, "statement-fingerprint") {
            return Some(ProbeObligation::ExactTheorem);
        }
        if has_contribution(candidate, "safe-permutation-fingerprint") {
            return Some(ProbeObligation::PermutedTheorem);
        }
        if matches!(group.relation, ReviewRelation::Specialization)
            || has_contribution(candidate, "conclusion-fingerprint")
        {
            return Some(ProbeObligation::Specialization);
        }
        if has_contribution(candidate, "connective-fingerprint") {
            return Some(ProbeObligation::Replacement);
        }
    }
    if anchor.definition_like() && right.definition_like() && strong_static_evidence(candidate) {
        return Some(ProbeObligation::ReducibleDefinition);
    }
    (policy == ProbePolicy::Broad).then_some(ProbeObligation::LocalDuplicate)
}

fn strong_static_evidence(candidate: &RetrievedCandidate) -> bool {
    if !matches!(
        candidate.declaration.kind.as_str(),
        "theorem" | "axiom" | "def" | "abbrev"
    ) {
        return false;
    }
    candidate.explanation.contributions.iter().any(|contribution| {
        matches!(
            contribution.kind.as_str(),
            "statement-fingerprint" | "safe-permutation-fingerprint" | "connective-fingerprint"
        )
    })
}

fn probe_supported_origin(declaration: &HydratedDeclaration, policy: &ComparisonEvidencePolicy) -> bool {
    declaration.origin == "workspace" || policy.probe_module(&declaration.origin, &declaration.module).is_some()
}

fn has_contribution(candidate: &RetrievedCandidate, kind: &str) -> bool {
    candidate
        .explanation
        .contributions
        .iter()
        .any(|contribution| contribution.kind == kind)
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
        message: Some(format!(
            "{}: probe isolated after worker failure: {error}",
            worker_error_reason(error).label()
        )),
    }
}

fn worker_error_reason(error: &WorkerError) -> UnavailableReason {
    let text = error.to_string();
    if text.contains("heartbeat") || text.contains("timeout") || text.contains("maximum number of heartbeats") {
        UnavailableReason::Timeout
    } else if text.contains("not available") || text.contains("missing") {
        UnavailableReason::MissingDeclaration
    } else {
        UnavailableReason::InternalError
    }
}

fn semantic_evidence(planned: &PlannedProbe, result: &ProbeResult) -> SemanticEvidence {
    if result.status != "ok" {
        return SemanticEvidence {
            pair_id: result.pair_id.clone(),
            kind: EvidenceKind::Unavailable,
            status: EvidenceStatus::Unavailable,
            summary: result
                .message
                .clone()
                .or_else(|| Some(format!("probe status {}", result.status))),
        };
    }
    match planned.obligation {
        ProbeObligation::ExactTheorem if result.same_statement => verified(result, EvidenceKind::ExactTheorem),
        ProbeObligation::PermutedTheorem if result.same_up_to_safe_reordering || result.same_statement => {
            verified(result, EvidenceKind::PermutedTheorem)
        }
        ProbeObligation::Replacement
            if result.connective_equivalent || result.mutual_implication_shape || result.same_statement =>
        {
            verified(result, EvidenceKind::Replacement)
        }
        ProbeObligation::ReducibleDefinition if result.same_reducible_definition => {
            verified(result, EvidenceKind::ReducibleDefinition)
        }
        ProbeObligation::Specialization
            if result.specializes_left_to_right || result.specializes_right_to_left || result.same_statement =>
        {
            verified(result, EvidenceKind::Specialization)
        }
        ProbeObligation::LocalDuplicate
            if result.same_statement
                || result.same_up_to_safe_reordering
                || result.same_reducible_definition
                || result.mutual_implication_shape =>
        {
            verified(result, EvidenceKind::LocalDuplicate)
        }
        _ => SemanticEvidence::rejected(
            result.pair_id.clone(),
            obligation_evidence_kind(planned.obligation),
            "Lean probe did not verify the planned obligation",
        ),
    }
}

fn verified(result: &ProbeResult, kind: EvidenceKind) -> SemanticEvidence {
    SemanticEvidence {
        pair_id: result.pair_id.clone(),
        kind,
        status: EvidenceStatus::Verified,
        summary: result.message.clone(),
    }
}

fn obligation_evidence_kind(obligation: ProbeObligation) -> EvidenceKind {
    match obligation {
        ProbeObligation::ExactTheorem => EvidenceKind::ExactTheorem,
        ProbeObligation::PermutedTheorem => EvidenceKind::PermutedTheorem,
        ProbeObligation::Replacement => EvidenceKind::Replacement,
        ProbeObligation::ReducibleDefinition => EvidenceKind::ReducibleDefinition,
        ProbeObligation::Specialization => EvidenceKind::Specialization,
        ProbeObligation::LocalDuplicate => EvidenceKind::LocalDuplicate,
    }
}

fn record_planned_obligation(obligation: ProbeObligation, diagnostics: &mut ProbeDiagnostics) {
    match obligation {
        ProbeObligation::ExactTheorem => diagnostics.planned_exact_theorem += 1,
        ProbeObligation::PermutedTheorem => diagnostics.planned_permuted_theorem += 1,
        ProbeObligation::Replacement => diagnostics.planned_replacement += 1,
        ProbeObligation::ReducibleDefinition => diagnostics.planned_reducible_definition += 1,
        ProbeObligation::Specialization => diagnostics.planned_specialization += 1,
        ProbeObligation::LocalDuplicate => diagnostics.planned_local_duplicate += 1,
    }
}

fn record_evidence_diagnostic(evidence: &SemanticEvidence, planned: &PlannedProbe, diagnostics: &mut ProbeDiagnostics) {
    match evidence.status {
        EvidenceStatus::Verified => diagnostics.verified_results += 1,
        EvidenceStatus::Rejected => {}
        EvidenceStatus::Unavailable => {
            let summary = evidence.summary.as_deref().unwrap_or_default();
            let reason = if summary.contains("missing-declaration")
                || summary.contains("not available")
                || summary.contains("missing")
            {
                UnavailableReason::MissingDeclaration
            } else if summary.contains("heartbeat") || summary.contains("timeout") {
                UnavailableReason::Timeout
            } else if summary.contains("supports")
                || summary.contains("opaque")
                || summary.contains("unavailable")
                || summary.contains("reducible probe guard")
                || summary.contains("definition body")
            {
                UnavailableReason::OpaqueOrUnreducible
            } else {
                UnavailableReason::InternalError
            };
            record_unavailable_reason(reason, planned, diagnostics);
        }
    }
}

fn record_unavailable_reason(reason: UnavailableReason, planned: &PlannedProbe, diagnostics: &mut ProbeDiagnostics) {
    diagnostics.unavailable_results += 1;
    match reason {
        UnavailableReason::MissingDeclaration => diagnostics.unavailable_missing += 1,
        UnavailableReason::Unsupported | UnavailableReason::OpaqueOrUnreducible => {
            diagnostics.unavailable_unsupported += 1;
        }
        UnavailableReason::Timeout => diagnostics.unavailable_timeout += 1,
        UnavailableReason::InternalError => diagnostics.unavailable_internal += 1,
    }
    increment(&mut diagnostics.unavailable_by_reason, reason.label());
    increment(
        &mut diagnostics.unavailable_by_obligation,
        obligation_label(planned.obligation),
    );
    increment(
        &mut diagnostics.unavailable_by_module,
        &format!("{}:{}", planned.left_origin, planned.left_module),
    );
    increment(
        &mut diagnostics.unavailable_by_module,
        &format!("{}:{}", planned.right_origin, planned.right_module),
    );
    increment(&mut diagnostics.unavailable_by_origin, &planned.left_origin);
    increment(&mut diagnostics.unavailable_by_origin, &planned.right_origin);
}

fn increment(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_owned()).or_insert(0) += 1;
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
    obligation: ProbeObligation,
) -> String {
    let payload = serde_json::json!({
        "cache_version": PROBE_CACHE_VERSION,
        "policy_version": PROBE_POLICY_VERSION,
        "policy": probe_policy_label(policy),
        "obligation": obligation_label(obligation),
        "pair": pair,
        "left": declaration_cache_facts(left),
        "right": declaration_cache_facts(right),
    });
    let encoded = serde_json::to_vec(&payload).expect("probe cache key ingredients serialize");
    let digest = Sha256::digest(encoded);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn obligation_label(obligation: ProbeObligation) -> &'static str {
    match obligation {
        ProbeObligation::ExactTheorem => "exact-theorem",
        ProbeObligation::PermutedTheorem => "permuted-theorem",
        ProbeObligation::Replacement => "replacement",
        ProbeObligation::ReducibleDefinition => "reducible-definition",
        ProbeObligation::Specialization => "specialization",
        ProbeObligation::LocalDuplicate => "local-duplicate",
    }
}

fn declaration_cache_facts(declaration: &HydratedDeclaration) -> serde_json::Value {
    serde_json::json!({
        "declaration_id": declaration.declaration_id,
        "feature_version": declaration.feature_version,
        "fingerprints": declaration.fingerprints,
        "binder_count": declaration.binder_count,
        "kind": declaration.kind,
    })
}

#[cfg(test)]
mod tests {
    use super::{ProbeSettings, SemanticVerificationInput, VerificationIndex, candidate_sets_for_review, plan_probes};
    use crate::cli::{ProbePolicy, ReviewProfile};
    use crate::external_provenance::{ComparisonEvidenceMode, ComparisonEvidencePolicy};
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
    fn default_review_shape_drops_internal_feature_only_candidates() {
        let anchor = declaration("workspace:Tiny:Tiny.local", "workspace", "Tiny.local");
        let exact = candidate(
            declaration("workspace:Tiny:Tiny.exact", "workspace", "Tiny.exact"),
            "statement-fingerprint",
            100.0,
        );
        let broad = candidate(
            declaration("workspace:Tiny:Tiny.broad", "workspace", "Tiny.broad"),
            "role-feature",
            12.0,
        );
        let shaped = candidate_sets_for_review(
            &[CandidateSet {
                anchor,
                candidates: vec![exact.clone(), broad],
            }],
            false,
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
        let policy = proof_grade_mathlib_policy();
        let review = rank_candidates(RankingInput {
            candidate_sets: &candidate_sets,
            semantic_evidence: &std::collections::BTreeMap::new(),
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
            comparison_policy: &policy,
        });
        let mut diagnostics = super::ProbeDiagnostics::default();
        let index = empty_index();
        let input = SemanticVerificationInput {
            candidate_sets: &candidate_sets,
            cheap_review: &review,
            local_index: VerificationIndex::new(&index),
            workspace: &workspace(),
            comparison_policy: &policy,
            enabled: true,
            include_private: true,
            include_generated: false,
            settings: ProbeSettings {
                policy: ProbePolicy::Broad,
                budget: 10,
                per_declaration_cap: 2,
                chunk_size: 16,
            },
        };

        assert_eq!(plan_probes(&input, &mut diagnostics).planned.len(), 1);
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
        let policy = proof_grade_mathlib_policy();
        let review = rank_candidates(RankingInput {
            candidate_sets: &candidate_sets,
            semantic_evidence: &std::collections::BTreeMap::new(),
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
            comparison_policy: &policy,
        });
        let mut diagnostics = super::ProbeDiagnostics::default();
        let index = empty_index();
        let input = SemanticVerificationInput {
            candidate_sets: &candidate_sets,
            cheap_review: &review,
            local_index: VerificationIndex::new(&index),
            workspace: &workspace(),
            comparison_policy: &policy,
            enabled: true,
            include_private: true,
            include_generated: false,
            settings: ProbeSettings {
                policy: ProbePolicy::Actionable,
                budget: 10,
                per_declaration_cap: 2,
                chunk_size: 16,
            },
        };

        assert_eq!(plan_probes(&input, &mut diagnostics).planned.len(), 2);
        assert_eq!(diagnostics.skipped_by_budget, 1);
    }

    #[test]
    fn planned_private_pair_requests_private_probe_filter() {
        let anchor = declaration("workspace:Tiny:Tiny.local", "workspace", "Tiny.local");
        let mut private = declaration(
            "mathlib:Mathlib:_private.Mathlib.0.hidden",
            "mathlib",
            "_private.Mathlib.0.hidden",
        );
        private.visibility = "private".to_owned();
        let candidate = candidate(private, "statement-fingerprint", 100.0);
        let candidate_sets = vec![CandidateSet {
            anchor,
            candidates: vec![candidate],
        }];
        let policy = proof_grade_mathlib_policy();
        let review = rank_candidates(RankingInput {
            candidate_sets: &candidate_sets,
            semantic_evidence: &std::collections::BTreeMap::new(),
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
            comparison_policy: &policy,
        });
        let mut diagnostics = super::ProbeDiagnostics::default();
        let index = empty_index();
        let input = SemanticVerificationInput {
            candidate_sets: &candidate_sets,
            cheap_review: &review,
            local_index: VerificationIndex::new(&index),
            workspace: &workspace(),
            comparison_policy: &policy,
            enabled: true,
            include_private: false,
            include_generated: false,
            settings: ProbeSettings {
                policy: ProbePolicy::Actionable,
                budget: 10,
                per_declaration_cap: 2,
                chunk_size: 16,
            },
        };

        let plan = plan_probes(&input, &mut diagnostics);

        assert_eq!(plan.planned.len(), 1);
        assert!(plan.planned[0].include_private);
    }

    #[test]
    fn unsupported_pairs_are_classified_before_worker_calls() {
        let mut anchor = declaration("workspace:Tiny:Tiny.local", "workspace", "Tiny.local");
        anchor.kind = "inductive".to_owned();
        let candidate = candidate(
            declaration("mathlib:Mathlib:Mathlib.exact", "mathlib", "Mathlib.exact"),
            "statement-fingerprint",
            100.0,
        );
        let candidate_sets = vec![CandidateSet {
            anchor,
            candidates: vec![candidate],
        }];
        let policy = proof_grade_mathlib_policy();
        let review = rank_candidates(RankingInput {
            candidate_sets: &candidate_sets,
            semantic_evidence: &std::collections::BTreeMap::new(),
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
            comparison_policy: &policy,
        });
        let mut diagnostics = super::ProbeDiagnostics::default();
        let index = empty_index();
        let input = SemanticVerificationInput {
            candidate_sets: &candidate_sets,
            cheap_review: &review,
            local_index: VerificationIndex::new(&index),
            workspace: &workspace(),
            comparison_policy: &policy,
            enabled: true,
            include_private: true,
            include_generated: false,
            settings: ProbeSettings {
                policy: ProbePolicy::Broad,
                budget: 10,
                per_declaration_cap: 2,
                chunk_size: 16,
            },
        };

        let plan = plan_probes(&input, &mut diagnostics);

        assert!(plan.planned.is_empty());
        assert_eq!(plan.preflight_evidence.len(), 1);
        assert_eq!(diagnostics.unavailable_unsupported, 1);
        assert_eq!(diagnostics.unavailable_by_reason.get("unsupported"), Some(&1));
    }

    fn empty_index() -> crate::index::OpenedIndex {
        crate::index::OpenedIndex::for_test(std::path::PathBuf::from("/tmp/nonexistent/index.sqlite"))
    }

    fn proof_grade_mathlib_policy() -> ComparisonEvidencePolicy {
        ComparisonEvidencePolicy::for_origin("mathlib", ComparisonEvidenceMode::ProofGrade)
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
