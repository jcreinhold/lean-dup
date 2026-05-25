use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use lean_dup_diagnostics::perf::{self, CostClass};
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_index::{self, CacheFacts};
use lean_dup_index::{ComparisonProvenance, ComparisonProvenanceReport};
use lean_dup_index::{IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex};
use lean_dup_project::{ResolvedWorkspace, WorkspaceRequest, resolve, resolve_workspace_mathlib};
use lean_dup_worker::WorkerClient;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::baseline;
use std::path::Path;
pub use crate::baseline::{BaselineGroup, BaselineSnapshot};

/// Load the most recent persisted audit snapshot for a workspace fingerprint,
/// if one is on disk. Used by `show`/`diff` to validate group IDs without
/// re-running the full audit pipeline.
pub fn load_last_audit_snapshot(cache_root: &Path, fingerprint: &str) -> Option<BaselineSnapshot> {
    baseline::load_last(cache_root, fingerprint)
}
use crate::ranking::{
    ConfidenceTier, RankedGroup, RankedReview, RankingDiagnostics, RankingInput, RankingProfile, ReviewAction,
    ReviewEvidence, ReviewEvidenceMode, ReviewFilter, ReviewMember, ReviewPriority, ReviewRelation, rank_candidates,
};
use crate::replacement_hints::{
    CallerImpact, ReplacementHint, ReplacementHintProfile, attach_replacement_hints, reference_declarations_for_hints,
};
use crate::retrieval::RetrievalDiagnostics;
use crate::retrieval::retrieve_candidates;
use crate::review_policy::{self, SearchReviewPolicySummary};
use crate::scorer::{SearchScoringSummary, default_summary};
use crate::semantic_reranking::{
    SearchSemanticObligationFact, SearchSemanticObligationYield, SearchSemanticRerankingSummary, sorted_yield,
    summary as semantic_reranking_summary,
};
use crate::semantic_verification::{
    ProbeDiagnostics, ProbeSettings, SemanticVerificationInput, VerificationIndex, candidate_sets_for_review,
    verify_candidate_probes,
};
use crate::source_refs::{ImportStatus, SourceFactInput, collect_source_facts};
use crate::{ProbePolicy, ProbeStatusBreakdown, Result};

const DEFAULT_VISIBLE_GROUP_LIMIT: usize = 500;
const ORDINARY_PAIR_SUMMARY_LIMIT: usize = 8;

/// Request for a complete duplicate-audit computation.
///
/// The search crate owns the phase ordering from local index reuse through
/// retrieval, semantic evidence, ranking, source impact, and optional baseline
/// persistence. Callers provide user intent; they do not sequence internal
/// search phases.
#[derive(Debug, Clone)]
pub struct AuditRequest {
    pub workspace: PathBuf,
    pub module_root: Option<String>,
    pub include_private: bool,
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub mathlib_workspace: Option<PathBuf>,
    pub include_generated: bool,
    pub visibility: AuditVisibilityOptions,
    pub save_baseline: Option<String>,
    pub semantic_probes: bool,
    pub probe_budget: usize,
    pub probe_policy: ProbePolicy,
    pub probe_chunk_size: usize,
}

/// Result of a complete audit computation before report projection.
#[derive(Debug)]
pub struct AuditOutput {
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub include_private: bool,
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub include_generated: bool,
    pub visibility: AuditVisibilityOptions,
    pub scoring: SearchScoringSummary,
    pub review_policy: SearchReviewPolicySummary,
    pub retrieval: AuditRetrievalSummary,
    pub comparison_provenance: Vec<ComparisonProvenanceReport>,
    pub semantic_verification: AuditProbeSummary,
    pub queue_counts: AuditQueueCounts,
    pub queue_summary: AuditQueueSummary,
    pub review: AuditReview,
    pub visible_groups: Vec<AuditGroup>,
    pub visible_group_count: usize,
    pub visible_group_limit: usize,
    pub visible_groups_truncated: bool,
    pub saved_baseline: Option<PathBuf>,
    pub saved_baseline_name: Option<String>,
    pub saved_baseline_group_count: usize,
}

/// Result of resolving one audit group through the search workflow.
#[derive(Debug)]
pub struct ShowOutput {
    pub audit: AuditOutput,
    pub group: AuditGroup,
}

/// Result of comparing the current audit queue against a saved baseline.
#[derive(Debug)]
pub struct DiffOutput {
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub cache_fingerprint: String,
    pub diff: SearchBaselineDiff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchBaselineDiff {
    pub baseline: String,
    pub baseline_path: PathBuf,
    pub appeared: Vec<SearchBaselineGroup>,
    pub disappeared: Vec<SearchBaselineGroup>,
    pub changed: Vec<SearchBaselineChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchBaselineGroup {
    pub id: String,
    pub relation: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub member_ids: Vec<String>,
    pub evidence_summary: Vec<String>,
    pub evidence_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchBaselineChange {
    pub id: String,
    pub before: SearchBaselineGroup,
    pub after: SearchBaselineGroup,
}

/// Stable retrieval counters exposed by audit workflows.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRetrievalSummary {
    pub fanout_policy_id: String,
    pub candidate_count: usize,
    pub hydrated_external_count: usize,
    pub pruned_feature_fanout_count: usize,
    pub heap_truncations: usize,
    pub top_k_saturation_by_source_id: BTreeMap<String, usize>,
    pub pruned_feature_fanout_by_family: BTreeMap<String, usize>,
}

/// Stable semantic-probe counters exposed by audit workflows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditProbeSummary {
    pub semantic_reranking: SearchSemanticRerankingSummary,
    pub enabled: bool,
    pub policy: String,
    pub budget: usize,
    pub per_declaration_cap: usize,
    pub chunk_size: usize,
    pub candidates_considered: usize,
    pub planned_pairs: usize,
    pub skipped_by_policy: usize,
    pub skipped_by_budget: usize,
    pub cheap_summary_rejects: usize,
    pub planned_exact_theorem: usize,
    pub planned_permuted_theorem: usize,
    pub planned_replacement: usize,
    pub planned_reducible_definition: usize,
    pub planned_specialization: usize,
    pub planned_local_duplicate: usize,
    pub cached_hits: usize,
    pub worker_pairs: usize,
    pub worker_batches: usize,
    pub recovered_failures: usize,
    pub unavailable_results: usize,
    pub unavailable_unsupported: usize,
    pub unavailable_missing: usize,
    pub unavailable_timeout: usize,
    pub unavailable_internal: usize,
    pub unavailable_by_reason: BTreeMap<String, usize>,
    pub unavailable_by_obligation: BTreeMap<String, usize>,
    pub unavailable_by_module: BTreeMap<String, usize>,
    pub unavailable_by_origin: BTreeMap<String, usize>,
    pub status_by_source: BTreeMap<String, ProbeStatusBreakdown>,
    pub status_by_match_class: BTreeMap<String, ProbeStatusBreakdown>,
    pub verified_results: usize,
    pub rejected_results: usize,
    pub obligation_yield: Vec<SearchSemanticObligationYield>,
}

/// Review queue facts computed by the search workflow.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditReview {
    pub groups: Vec<AuditGroup>,
    pub group_count: usize,
    pub suppressed_count: usize,
    pub diagnostics: AuditReviewDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditReviewDiagnostics {
    pub candidate_pairs: usize,
    pub emitted_groups: usize,
    pub suppressed_groups: usize,
}

/// One audit group in the stable search workflow output.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditGroup {
    pub family_id: String,
    pub id: String,
    pub pair_id: String,
    pub pair_count: usize,
    pub pair_ids: Vec<String>,
    pub pair_evidence: Vec<AuditPairEvidence>,
    pub pair_evidence_truncated: bool,
    pub relation: String,
    pub members: Vec<AuditMember>,
    pub evidence: Vec<AuditEvidence>,
    pub signals: Vec<String>,
    pub blockers: Vec<String>,
    pub confidence: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub target_decl: Option<String>,
    pub target_module: Option<String>,
    pub evidence_mode: String,
    pub probe_summary: Option<String>,
    pub semantic_obligations: Vec<SearchSemanticObligationFact>,
    pub local_caller_count: usize,
    pub replacement_hint: Option<AuditReplacementHint>,
    pub visibility: AuditVisibility,
}

/// Bounded pair evidence carried by a review family.
///
/// Ordinary audit output uses these summaries to show why a family exists
/// without exposing retrieval rows or forcing users to reconstruct the family
/// from unbounded pair-shaped findings. `show` may request the full selected
/// family evidence through the same stable shape.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditPairEvidence {
    pub id: String,
    pub pair_id: String,
    pub relation: String,
    pub members: Vec<AuditMember>,
    pub evidence: Vec<AuditEvidence>,
    pub signals: Vec<String>,
    pub blockers: Vec<String>,
    pub confidence: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub evidence_mode: String,
    pub probe_summary: Option<String>,
    pub semantic_obligations: Vec<SearchSemanticObligationFact>,
    pub local_caller_count: usize,
    pub replacement_hint: Option<AuditReplacementHint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditMember {
    pub declaration_id: String,
    pub origin: String,
    pub module: String,
    pub qualified_name: String,
    pub display_name: String,
    pub kind: String,
    pub visibility: String,
    pub source_span: Option<lean_dup_worker::SourceSpan>,
    pub status_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEvidence {
    pub kind: String,
    pub role: Option<String>,
    pub display: Option<String>,
    pub score: f64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditReplacementHint {
    pub target_decl: String,
    pub target_module: String,
    pub import_status: String,
    pub caller_impact: String,
    pub caller_count: usize,
    pub displayed_callers: Vec<AuditSourceReference>,
    pub callers_truncated: bool,
    pub notes: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSourceReference {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditVisibility {
    pub visible: bool,
    pub reason: String,
    pub hidden_reason: Option<AuditHiddenReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditHiddenReason {
    Generated,
    UnverifiedProofGrade,
    UnavailableProbe,
    VisibilityOrNoise,
    OtherBlocker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQueueSummary {
    pub visible: usize,
    pub emitted: usize,
    pub limit: usize,
    pub truncated: bool,
    pub total: usize,
    pub hidden: AuditHiddenGroupCounts,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditHiddenGroupCounts {
    pub total: usize,
    pub visibility_or_noise: usize,
    pub generated: usize,
    pub unverified_proof_grade: usize,
    pub unavailable_probe: usize,
    pub other_blockers: usize,
}

/// Composable visibility options for audit review queues.
///
/// Callers describe which classes of findings they want to include. Search
/// owns the priority thresholds, blocker predicates, and diagnostic details.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AuditVisibilityOptions {
    pub include_private: bool,
    pub include_low_priority: bool,
    pub diagnostics: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditQueueCounts {
    pub cleanup: usize,
    pub with_private: usize,
    pub with_low_priority: usize,
    pub diagnostics: usize,
}

struct WorkflowOutput {
    requested_workspace: PathBuf,
    lake_root: PathBuf,
    selected_roots: Vec<String>,
    source_count: usize,
    cache_root: PathBuf,
    cache_fingerprint: String,
    include_private: bool,
    compare_indexes: Vec<String>,
    compare_mathlib: bool,
    include_generated: bool,
    visibility: AuditVisibilityOptions,
    retrieval: RetrievalDiagnostics,
    comparison_provenance: Vec<ComparisonProvenanceReport>,
    semantic_verification: ProbeDiagnostics,
    review: RankedReview,
    saved_baseline: Option<PathBuf>,
    saved_baseline_name: Option<String>,
    saved_baseline_group_count: usize,
}

struct Foundation {
    workspace: ResolvedWorkspace,
    cache: CacheFacts,
}

struct CompareIndexes {
    indexes: Vec<OpenedIndex>,
    provenance: ComparisonProvenance,
}

/// Run the complete audit workflow.
pub fn run_audit(request: AuditRequest, reporter: &mut Reporter) -> Result<AuditOutput> {
    Ok(project_audit_output(run_audit_workflow(request, reporter)?))
}

fn run_audit_workflow(request: AuditRequest, reporter: &mut Reporter) -> Result<WorkflowOutput> {
    let module_root = request.module_root.clone();
    let foundation = foundation(request.workspace.clone(), module_root.clone(), reporter)?;
    let store = IndexStore::new(foundation.cache.root.clone());
    let local_label = "audit-workspace".to_owned();
    let local_module_root = module_root.unwrap_or_else(|| foundation.workspace.selected_roots.join(","));
    reporter.measure("index.local", |reporter| {
        store.build_or_reuse(
            IndexBuildRequest {
                workspace: foundation.workspace.clone(),
                execution_root: None,
                label: local_label.clone(),
                module_root: local_module_root,
                origin: "workspace".to_owned(),
                include_private: request.include_private,
                include_generated: request.include_generated,
                require_oleans: false,
                force: false,
                kind: IndexBuildKind::Local,
            },
            &WorkerClient::for_indexing(),
            reporter,
        )
    })?;
    let local_index = store.resolve(IndexReference::Label(local_label))?;
    let local_handles = local_index.all_handles()?;
    let workspace_rows = local_index.hydrate(&local_handles)?;
    let compare = open_compare_indexes(&request, &store, &foundation.workspace, reporter)?;
    let retrieval_output = reporter.measure("retrieval", |_| retrieve_candidates(&workspace_rows, &compare.indexes))?;
    let review_candidate_sets = perf::measure(CostClass::RetrievalRanking, "ranking.candidate_shaping", || {
        candidate_sets_for_review(
            &retrieval_output.candidate_sets,
            request.compare_mathlib,
            request.visibility.diagnostics,
        )
    });
    let source_fact_rows = source_fact_declarations(
        &workspace_rows,
        &review_candidate_sets,
        request.compare_mathlib,
        request.visibility.diagnostics,
    );
    let mut source_facts = perf::measure(CostClass::RetrievalRanking, "source_refs.collect.initial", || {
        collect_source_facts(SourceFactInput::new(&source_fact_rows).without_references())
    });
    let cheap_review = perf::measure(CostClass::RetrievalRanking, "ranking.rank_candidates.initial", || {
        rank_candidates(RankingInput {
            candidate_sets: &review_candidate_sets,
            semantic_evidence: &std::collections::BTreeMap::new(),
            source_facts: &source_facts,
            profile: RankingProfile::default(),
            comparison_policy: &compare.provenance.policy,
        })
    });
    let verification = verify_candidate_probes(
        SemanticVerificationInput {
            candidate_sets: &review_candidate_sets,
            cheap_review: &cheap_review,
            local_index: VerificationIndex::new(&local_index),
            workspace: &foundation.workspace,
            comparison_policy: &compare.provenance.policy,
            enabled: request.semantic_probes,
            include_private: request.include_private,
            include_generated: request.include_generated,
            settings: ProbeSettings {
                policy: request.probe_policy,
                budget: request.probe_budget,
                per_declaration_cap: 2,
                chunk_size: request.probe_chunk_size,
            },
        },
        reporter,
    )?;
    let review_without_references = perf::measure(CostClass::RetrievalRanking, "ranking.rank_candidates.final", || {
        rank_candidates(RankingInput {
            candidate_sets: &review_candidate_sets,
            semantic_evidence: &verification.evidence,
            source_facts: &source_facts,
            profile: RankingProfile::default(),
            comparison_policy: &compare.provenance.policy,
        })
    });
    let filter = review_filter(request.visibility, request.include_generated);
    let reference_ids = reference_declarations_for_hints(&review_without_references, filter)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let review = if reference_ids.is_empty() {
        review_without_references
    } else {
        source_facts = perf::measure(CostClass::RetrievalRanking, "source_refs.collect.references", || {
            collect_source_facts(SourceFactInput::new(&source_fact_rows).with_reference_declarations(reference_ids))
        });
        perf::measure(
            CostClass::RetrievalRanking,
            "ranking.rank_candidates.with_references",
            || {
                rank_candidates(RankingInput {
                    candidate_sets: &review_candidate_sets,
                    semantic_evidence: &verification.evidence,
                    source_facts: &source_facts,
                    profile: RankingProfile::default(),
                    comparison_policy: &compare.provenance.policy,
                })
            },
        )
    };
    let review = perf::measure(CostClass::RetrievalRanking, "ranking.replacement_hints", || {
        attach_replacement_hints(review, &source_facts, ReplacementHintProfile::default())
    });
    let snapshot = baseline::snapshot(&review, foundation.cache.fingerprint.clone());
    if let Err(error) = baseline::save_last(&foundation.cache.root, &snapshot) {
        reporter.event(
            "baseline.last",
            None,
            None,
            format!("could not persist last-snapshot: {error}"),
        );
    }
    let snapshot_group_count = snapshot.groups.len();
    let (saved_baseline, saved_baseline_name) = match request.save_baseline {
        Some(name) => {
            let path = baseline::save(&foundation.cache.root, &name, &snapshot)?;
            (Some(path), Some(name))
        }
        None => (None, None),
    };

    Ok(WorkflowOutput {
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        selected_roots: foundation.workspace.selected_roots,
        source_count: foundation.workspace.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        include_private: request.include_private,
        compare_indexes: request.compare_indexes,
        compare_mathlib: request.compare_mathlib,
        include_generated: request.include_generated,
        visibility: request.visibility,
        retrieval: retrieval_output.diagnostics,
        comparison_provenance: compare.provenance.reports,
        semantic_verification: verification.diagnostics,
        review,
        saved_baseline,
        saved_baseline_name,
        saved_baseline_group_count: snapshot_group_count,
    })
}

/// Run an audit and return one ranked group by stable group id.
pub fn run_show(request: AuditRequest, requested_group: &str, reporter: &mut Reporter) -> Result<ShowOutput> {
    let workflow = run_audit_workflow(request, reporter)?;
    let filter = review_filter(workflow.visibility, workflow.include_generated);
    let mut group = audit_families(workflow.review.groups.iter(), filter, usize::MAX)
        .into_iter()
        .find(|group| {
            group.id == requested_group
                || group.family_id == requested_group
                || group.pair_id == requested_group
                || group.pair_ids.iter().any(|pair_id| pair_id == requested_group)
                || group.pair_evidence.iter().any(|pair| pair.id == requested_group)
        })
        .ok_or_else(|| crate::Error::Search {
            message: format!("unknown audit group: {requested_group}"),
        })?;
    if group.id != requested_group
        && (group.pair_id == requested_group
            || group.pair_ids.iter().any(|pair_id| pair_id == requested_group)
            || group.pair_evidence.iter().any(|pair| pair.id == requested_group))
    {
        group.id = requested_group.to_owned();
    }
    let audit = project_audit_output(workflow);
    Ok(ShowOutput { audit, group })
}

/// Run an audit and compare it with a named saved baseline.
pub fn run_diff(request: AuditRequest, baseline_name: String, reporter: &mut Reporter) -> Result<DiffOutput> {
    let audit = run_audit_workflow(request, reporter)?;
    let (baseline_path, saved) = baseline::load(&audit.cache_root, &baseline_name)?;
    let current = baseline::snapshot(&audit.review, audit.cache_fingerprint.clone());
    let diff = search_baseline_diff(baseline::diff(baseline_name, baseline_path, saved, current));
    Ok(DiffOutput {
        requested_workspace: audit.requested_workspace,
        lake_root: audit.lake_root,
        selected_roots: audit.selected_roots,
        source_count: audit.source_count,
        cache_root: audit.cache_root,
        cache_fingerprint: audit.cache_fingerprint,
        diff,
    })
}

fn search_baseline_diff(diff: baseline::BaselineDiff) -> SearchBaselineDiff {
    SearchBaselineDiff {
        baseline: diff.baseline,
        baseline_path: diff.baseline_path,
        appeared: diff.appeared.into_iter().map(search_baseline_group).collect(),
        disappeared: diff.disappeared.into_iter().map(search_baseline_group).collect(),
        changed: diff
            .changed
            .into_iter()
            .map(|change| SearchBaselineChange {
                id: change.id,
                before: search_baseline_group(change.before),
                after: search_baseline_group(change.after),
            })
            .collect(),
    }
}

fn search_baseline_group(group: baseline::BaselineGroup) -> SearchBaselineGroup {
    SearchBaselineGroup {
        id: group.id,
        relation: group.relation,
        review_priority: group.review_priority,
        recommended_action: group.recommended_action,
        member_ids: group.member_ids,
        evidence_summary: group.evidence_summary,
        evidence_digest: group.evidence_digest,
    }
}

fn project_audit_output(workflow: WorkflowOutput) -> AuditOutput {
    let filter = review_filter(workflow.visibility, workflow.include_generated);
    let visible_families = audit_families(
        workflow.review.visible_groups(filter),
        filter,
        ORDINARY_PAIR_SUMMARY_LIMIT,
    );
    let visible_group_count = visible_families.len();
    let visible_group_limit = DEFAULT_VISIBLE_GROUP_LIMIT;
    let visible_groups_truncated = visible_group_count > visible_group_limit;
    let visible_groups = visible_families
        .into_iter()
        .take(visible_group_limit)
        .collect::<Vec<_>>();
    let queue_summary = audit_queue_summary(&workflow.review, filter, visible_groups.len(), visible_group_limit);
    let queue_counts = audit_queue_counts(&workflow.review);
    let review = audit_review(&workflow.review, filter);
    AuditOutput {
        requested_workspace: workflow.requested_workspace,
        lake_root: workflow.lake_root,
        selected_roots: workflow.selected_roots,
        source_count: workflow.source_count,
        cache_root: workflow.cache_root,
        cache_fingerprint: workflow.cache_fingerprint,
        include_private: workflow.include_private,
        compare_indexes: workflow.compare_indexes,
        compare_mathlib: workflow.compare_mathlib,
        include_generated: workflow.include_generated,
        visibility: workflow.visibility,
        scoring: default_summary(),
        review_policy: review_policy::summary(),
        retrieval: audit_retrieval_summary(&workflow.retrieval),
        comparison_provenance: workflow.comparison_provenance,
        semantic_verification: audit_probe_summary(&workflow.semantic_verification),
        queue_counts,
        queue_summary,
        review,
        visible_groups,
        visible_group_count,
        visible_group_limit,
        visible_groups_truncated,
        saved_baseline: workflow.saved_baseline,
        saved_baseline_name: workflow.saved_baseline_name,
        saved_baseline_group_count: workflow.saved_baseline_group_count,
    }
}

fn audit_retrieval_summary(diagnostics: &RetrievalDiagnostics) -> AuditRetrievalSummary {
    AuditRetrievalSummary {
        fanout_policy_id: diagnostics.fanout_policy.policy_id.clone(),
        candidate_count: diagnostics.candidate_count,
        hydrated_external_count: diagnostics.hydrated_external_count,
        pruned_feature_fanout_count: diagnostics.pruned_feature_fanouts.len(),
        heap_truncations: diagnostics.heap_truncations.len(),
        top_k_saturation_by_source_id: diagnostics.top_k_saturation_by_source_id.clone(),
        pruned_feature_fanout_by_family: pruned_feature_fanout_by_family(diagnostics),
    }
}

fn pruned_feature_fanout_by_family(diagnostics: &RetrievalDiagnostics) -> BTreeMap<String, usize> {
    let mut by_family = BTreeMap::new();
    for item in &diagnostics.pruned_feature_fanouts {
        *by_family.entry(item.feature_family.clone()).or_insert(0) += item.count;
    }
    by_family
}

fn audit_probe_summary(diagnostics: &ProbeDiagnostics) -> AuditProbeSummary {
    AuditProbeSummary {
        semantic_reranking: semantic_reranking_summary(),
        enabled: diagnostics.enabled,
        policy: diagnostics.policy.clone(),
        budget: diagnostics.budget,
        per_declaration_cap: diagnostics.per_declaration_cap,
        chunk_size: diagnostics.chunk_size,
        candidates_considered: diagnostics.candidates_considered,
        planned_pairs: diagnostics.planned_pairs,
        skipped_by_policy: diagnostics.skipped_by_policy,
        skipped_by_budget: diagnostics.skipped_by_budget,
        cheap_summary_rejects: diagnostics.cheap_summary_rejects,
        planned_exact_theorem: diagnostics.planned_exact_theorem,
        planned_permuted_theorem: diagnostics.planned_permuted_theorem,
        planned_replacement: diagnostics.planned_replacement,
        planned_reducible_definition: diagnostics.planned_reducible_definition,
        planned_specialization: diagnostics.planned_specialization,
        planned_local_duplicate: diagnostics.planned_local_duplicate,
        cached_hits: diagnostics.cached_hits,
        worker_pairs: diagnostics.worker_pairs,
        worker_batches: diagnostics.worker_batches,
        recovered_failures: diagnostics.recovered_failures,
        unavailable_results: diagnostics.unavailable_results,
        unavailable_unsupported: diagnostics.unavailable_unsupported,
        unavailable_missing: diagnostics.unavailable_missing,
        unavailable_timeout: diagnostics.unavailable_timeout,
        unavailable_internal: diagnostics.unavailable_internal,
        unavailable_by_reason: diagnostics.unavailable_by_reason.clone(),
        unavailable_by_obligation: diagnostics.unavailable_by_obligation.clone(),
        unavailable_by_module: diagnostics.unavailable_by_module.clone(),
        unavailable_by_origin: diagnostics.unavailable_by_origin.clone(),
        status_by_source: diagnostics.status_by_source.clone(),
        status_by_match_class: diagnostics.status_by_match_class.clone(),
        verified_results: diagnostics.verified_results,
        rejected_results: diagnostics.rejected_results,
        obligation_yield: sorted_yield(diagnostics.obligation_yield.clone()),
    }
}

fn audit_review(review: &RankedReview, _filter: ReviewFilter) -> AuditReview {
    AuditReview {
        groups: Vec::new(),
        group_count: review.groups.len(),
        suppressed_count: review.suppressed.len(),
        diagnostics: audit_review_diagnostics(&review.diagnostics),
    }
}

fn audit_review_diagnostics(diagnostics: &RankingDiagnostics) -> AuditReviewDiagnostics {
    AuditReviewDiagnostics {
        candidate_pairs: diagnostics.candidate_pairs,
        emitted_groups: diagnostics.emitted_groups,
        suppressed_groups: diagnostics.suppressed_groups,
    }
}

fn audit_families<'a, I>(groups: I, filter: ReviewFilter, pair_summary_limit: usize) -> Vec<AuditGroup>
where
    I: IntoIterator<Item = &'a RankedGroup>,
{
    let mut families: Vec<(String, Vec<&RankedGroup>)> = Vec::new();
    for group in groups {
        let key = family_key(group);
        if let Some((_, grouped)) = families.iter_mut().find(|(existing, _)| existing == &key) {
            grouped.push(group);
        } else {
            families.push((key, vec![group]));
        }
    }

    families
        .into_iter()
        .map(|(_key, groups)| audit_family(groups, filter, pair_summary_limit))
        .collect()
}

fn audit_family(groups: Vec<&RankedGroup>, filter: ReviewFilter, pair_summary_limit: usize) -> AuditGroup {
    let representative = groups
        .first()
        .copied()
        .expect("audit families are built from non-empty groups");
    let family_id = family_id(&groups);
    let pair_count = groups.len();
    let pair_ids = groups.iter().map(|group| group.pair_id.clone()).collect::<Vec<_>>();
    let pair_evidence_truncated = pair_summary_limit != usize::MAX && pair_count > pair_summary_limit;
    let pair_evidence = if pair_count > 1 {
        groups
            .iter()
            .take(pair_summary_limit)
            .map(|group| audit_pair_evidence(group))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let members = family_members(&groups);
    let signals = family_strings(&groups, |group| &group.signals);
    let blockers = family_strings(&groups, |group| &group.blockers);
    let local_caller_count = groups.iter().map(|group| group.local_caller_count).sum();

    AuditGroup {
        family_id: family_id.clone(),
        id: family_id,
        pair_id: representative.pair_id.clone(),
        pair_count,
        pair_ids,
        pair_evidence,
        pair_evidence_truncated,
        relation: relation_label(representative.relation).to_owned(),
        members,
        evidence: representative.evidence.iter().map(audit_evidence).collect(),
        signals,
        blockers,
        confidence: confidence_label(representative.confidence).to_owned(),
        review_priority: priority_label(representative.review_priority).to_owned(),
        recommended_action: action_label(representative.recommended_action).to_owned(),
        target_decl: representative.target_decl.clone(),
        target_module: representative.target_module.clone(),
        evidence_mode: evidence_mode_label(representative.evidence_mode).to_owned(),
        probe_summary: representative.probe_summary.clone(),
        semantic_obligations: representative.semantic_obligations.clone(),
        local_caller_count,
        replacement_hint: representative.replacement_hint.as_ref().map(audit_replacement_hint),
        visibility: audit_visibility(representative, filter),
    }
}

fn audit_pair_evidence(group: &RankedGroup) -> AuditPairEvidence {
    AuditPairEvidence {
        id: group.id.clone(),
        pair_id: group.pair_id.clone(),
        relation: relation_label(group.relation).to_owned(),
        members: group.members.iter().map(audit_member).collect(),
        evidence: group.evidence.iter().map(audit_evidence).collect(),
        signals: group.signals.clone(),
        blockers: group.blockers.clone(),
        confidence: confidence_label(group.confidence).to_owned(),
        review_priority: priority_label(group.review_priority).to_owned(),
        recommended_action: action_label(group.recommended_action).to_owned(),
        evidence_mode: evidence_mode_label(group.evidence_mode).to_owned(),
        probe_summary: group.probe_summary.clone(),
        semantic_obligations: group.semantic_obligations.clone(),
        local_caller_count: group.local_caller_count,
        replacement_hint: group.replacement_hint.as_ref().map(audit_replacement_hint),
    }
}

fn family_key(group: &RankedGroup) -> String {
    if coherent_family_action(group.recommended_action)
        && let Some(target_decl) = group.target_decl.as_deref().filter(|target| !target.is_empty())
    {
        return format!("action:{}:target:{target_decl}", action_label(group.recommended_action));
    }
    format!("single:{}", group.id)
}

fn coherent_family_action(action: ReviewAction) -> bool {
    matches!(
        action,
        ReviewAction::AlreadyInMathlib
            | ReviewAction::LocalAlias
            | ReviewAction::ReplaceLocalUses
            | ReviewAction::InlinePrivateHelper
    )
}

fn family_id(groups: &[&RankedGroup]) -> String {
    if groups.len() == 1 {
        return groups[0].id.clone();
    }
    let representative = groups[0];
    let mut parts = vec![format!("action={}", action_label(representative.recommended_action))];
    if let Some(target) = &representative.target_decl {
        parts.push(format!("target={target}"));
    }
    parts.extend(groups.iter().map(|group| format!("pair={}", group.pair_id)));
    parts.sort();
    let encoded = serde_json::to_vec(&parts).expect("family id parts serialize");
    let digest = Sha256::digest(&encoded);
    let suffix = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("family-{}-{suffix}", action_label(representative.recommended_action))
}

fn family_members(groups: &[&RankedGroup]) -> Vec<AuditMember> {
    let mut seen = BTreeSet::new();
    let mut members = Vec::new();
    for group in groups {
        for member in &group.members {
            if seen.insert(member.declaration_id.clone()) {
                members.push(audit_member(member));
            }
        }
    }
    members
}

fn family_strings<F>(groups: &[&RankedGroup], extract: F) -> Vec<String>
where
    F: Fn(&RankedGroup) -> &Vec<String>,
{
    let mut values = groups
        .iter()
        .flat_map(|group| extract(group).iter().cloned())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn audit_member(member: &ReviewMember) -> AuditMember {
    AuditMember {
        declaration_id: member.declaration_id.clone(),
        origin: member.origin.clone(),
        module: member.module.clone(),
        qualified_name: member.qualified_name.clone(),
        display_name: member.display_name.clone(),
        kind: member.kind.clone(),
        visibility: member.visibility.clone(),
        source_span: member.source_span.clone(),
        status_flags: member.status_flags.clone(),
    }
}

fn audit_evidence(evidence: &ReviewEvidence) -> AuditEvidence {
    AuditEvidence {
        kind: evidence.kind.clone(),
        role: evidence.role.clone(),
        display: evidence.display.clone(),
        score: evidence.score,
        summary: evidence.summary(),
    }
}

fn audit_replacement_hint(hint: &ReplacementHint) -> AuditReplacementHint {
    AuditReplacementHint {
        target_decl: hint.target_decl.clone(),
        target_module: hint.target_module.clone(),
        import_status: import_status_label(hint.import_status).to_owned(),
        caller_impact: caller_impact_label(hint.caller_impact).to_owned(),
        caller_count: hint.caller_count,
        displayed_callers: hint
            .displayed_callers
            .iter()
            .map(|caller| AuditSourceReference {
                file: caller.file.clone(),
                line: caller.line,
                column: caller.column,
                text: caller.text.clone(),
            })
            .collect(),
        callers_truncated: hint.callers_truncated,
        notes: hint.notes.clone(),
        blockers: hint.blockers.clone(),
    }
}

fn import_status_label(status: ImportStatus) -> &'static str {
    match status {
        ImportStatus::Direct => "direct",
        ImportStatus::Missing => "missing",
        ImportStatus::Unknown => "unknown",
        ImportStatus::SourceBackedNotImportable => "source-backed-not-importable",
    }
}

fn caller_impact_label(impact: CallerImpact) -> &'static str {
    match impact {
        CallerImpact::NoCallers => "no-callers",
        CallerImpact::WrapperOnly => "wrapper-only",
        CallerImpact::BoundedCallers => "bounded-callers",
        CallerImpact::TruncatedCallers => "truncated-callers",
        CallerImpact::UnknownCallers => "unknown-callers",
        CallerImpact::MissingSource => "missing-source",
    }
}

fn audit_visibility(group: &RankedGroup, filter: ReviewFilter) -> AuditVisibility {
    if filter.includes(group) {
        return AuditVisibility {
            visible: true,
            reason: "included by the active audit visibility options".to_owned(),
            hidden_reason: None,
        };
    }
    let hidden_reason = audit_hidden_reason(group, filter).unwrap_or(AuditHiddenReason::OtherBlocker);
    AuditVisibility {
        visible: false,
        reason: hidden_reason_sentence(hidden_reason),
        hidden_reason: Some(hidden_reason),
    }
}

fn audit_queue_summary(
    review: &RankedReview,
    filter: ReviewFilter,
    visible_groups_emitted: usize,
    visible_group_limit: usize,
) -> AuditQueueSummary {
    let mut hidden = AuditHiddenGroupCounts::default();
    for group in &review.groups {
        if filter.includes(group) {
            continue;
        }
        hidden.total += 1;
        match audit_hidden_reason(group, filter).unwrap_or(AuditHiddenReason::OtherBlocker) {
            AuditHiddenReason::Generated => hidden.generated += 1,
            AuditHiddenReason::UnverifiedProofGrade => hidden.unverified_proof_grade += 1,
            AuditHiddenReason::UnavailableProbe => hidden.unavailable_probe += 1,
            AuditHiddenReason::VisibilityOrNoise => hidden.visibility_or_noise += 1,
            AuditHiddenReason::OtherBlocker => hidden.other_blockers += 1,
        }
    }
    let visible = audit_families(review.visible_groups(filter), filter, 0).len();
    AuditQueueSummary {
        visible,
        emitted: visible_groups_emitted,
        limit: visible_group_limit,
        truncated: visible_groups_emitted < visible,
        total: review.groups.len(),
        hidden,
    }
}

fn audit_queue_counts(review: &RankedReview) -> AuditQueueCounts {
    AuditQueueCounts {
        cleanup: {
            let filter = review_filter(AuditVisibilityOptions::default(), false);
            audit_families(review.visible_groups(filter), filter, 0).len()
        },
        with_private: {
            let filter = review_filter(
                AuditVisibilityOptions {
                    include_private: true,
                    ..AuditVisibilityOptions::default()
                },
                false,
            );
            audit_families(review.visible_groups(filter), filter, 0).len()
        },
        with_low_priority: {
            let filter = review_filter(
                AuditVisibilityOptions {
                    include_low_priority: true,
                    ..AuditVisibilityOptions::default()
                },
                false,
            );
            audit_families(review.visible_groups(filter), filter, 0).len()
        },
        diagnostics: {
            let filter = review_filter(
                AuditVisibilityOptions {
                    include_private: true,
                    include_low_priority: true,
                    diagnostics: true,
                },
                true,
            );
            audit_families(review.visible_groups(filter), filter, 0).len()
        },
    }
}

fn audit_hidden_reason(group: &RankedGroup, filter: ReviewFilter) -> Option<AuditHiddenReason> {
    if filter.includes(group) {
        return None;
    }
    if !filter.include_generated && has_blocker(group, "generated-declaration") {
        return Some(AuditHiddenReason::Generated);
    }
    if has_blocker(group, "unverified-proof-grade-evidence") {
        return Some(AuditHiddenReason::UnverifiedProofGrade);
    }
    if has_blocker(group, "lean-probe-unavailable") {
        return Some(AuditHiddenReason::UnavailableProbe);
    }
    if group.review_priority == ReviewPriority::Noise
        || group.review_priority > filter.min_priority
        || !filter.include_diagnostics
    {
        return Some(AuditHiddenReason::VisibilityOrNoise);
    }
    if !group.blockers.is_empty() {
        return Some(AuditHiddenReason::OtherBlocker);
    }
    Some(AuditHiddenReason::OtherBlocker)
}

fn has_blocker(group: &RankedGroup, blocker: &str) -> bool {
    group.blockers.iter().any(|item| item == blocker)
}

fn hidden_reason_sentence(reason: AuditHiddenReason) -> String {
    match reason {
        AuditHiddenReason::Generated => "hidden because generated declarations are excluded".to_owned(),
        AuditHiddenReason::UnverifiedProofGrade => {
            "hidden because proof-grade comparison evidence was required but not verified".to_owned()
        }
        AuditHiddenReason::UnavailableProbe => {
            "hidden because the required Lean semantic probe is unavailable".to_owned()
        }
        AuditHiddenReason::VisibilityOrNoise => "hidden by the active audit visibility options".to_owned(),
        AuditHiddenReason::OtherBlocker => "hidden by blockers or output filters".to_owned(),
    }
}

fn evidence_mode_label(mode: ReviewEvidenceMode) -> &'static str {
    match mode {
        ReviewEvidenceMode::Static => "static",
        ReviewEvidenceMode::SourceBackedNotImportable => "source-backed-not-importable",
        ReviewEvidenceMode::ProofGrade => "proof-grade",
    }
}

fn relation_label(relation: ReviewRelation) -> &'static str {
    match relation {
        ReviewRelation::ExactStatement => "exact-statement",
        ReviewRelation::PermutedStatement => "permuted-statement",
        ReviewRelation::ConnectiveEquivalent => "connective-equivalent",
        ReviewRelation::Specialization => "specialization",
        ReviewRelation::SourceClone => "source-clone",
        ReviewRelation::SubsumptionCandidate => "subsumption-candidate",
        ReviewRelation::NearStatement => "near-statement",
    }
}

fn action_label(action: ReviewAction) -> &'static str {
    match action {
        ReviewAction::AlreadyInMathlib => "already-in-mathlib",
        ReviewAction::LocalAlias => "local-alias",
        ReviewAction::ReplaceLocalUses => "replace-local-uses",
        ReviewAction::InlinePrivateHelper => "inline-private-helper",
        ReviewAction::MergeGeneralization => "merge-generalization",
        ReviewAction::SpecializationOf => "specialization-of",
        ReviewAction::ProbableSourceClone => "probable-source-clone",
        ReviewAction::ManualReview => "manual-review",
    }
}

fn confidence_label(confidence: ConfidenceTier) -> &'static str {
    match confidence {
        ConfidenceTier::High => "high",
        ConfidenceTier::Medium => "medium",
        ConfidenceTier::Low => "low",
        ConfidenceTier::Noise => "noise",
    }
}

fn priority_label(priority: ReviewPriority) -> &'static str {
    match priority {
        ReviewPriority::High => "high",
        ReviewPriority::Medium => "medium",
        ReviewPriority::Low => "low",
        ReviewPriority::Noise => "noise",
    }
}

fn review_filter(visibility: AuditVisibilityOptions, include_generated: bool) -> ReviewFilter {
    ReviewFilter {
        include_generated: include_generated || visibility.diagnostics,
        include_private: visibility.include_private || visibility.diagnostics,
        include_diagnostics: visibility.diagnostics,
        min_priority: if visibility.diagnostics {
            ReviewPriority::Noise
        } else if visibility.include_low_priority {
            ReviewPriority::Low
        } else {
            ReviewPriority::Medium
        },
    }
}

fn foundation(requested_root: PathBuf, module_root: Option<String>, reporter: &mut Reporter) -> Result<Foundation> {
    reporter.measure("workspace.resolve", |reporter| {
        let workspace = resolve(
            WorkspaceRequest {
                requested_root,
                module_root,
            },
            reporter,
        )?;
        let cache = lean_dup_index::resolve_cache(&workspace)?;
        reporter.event("cache", None, None, format!("cache root {}", cache.root.display()));
        Ok(Foundation { workspace, cache })
    })
}

fn open_compare_indexes(
    request: &AuditRequest,
    store: &IndexStore,
    project_workspace: &ResolvedWorkspace,
    reporter: &mut Reporter,
) -> Result<CompareIndexes> {
    let mut indexes = Vec::new();
    for label in &request.compare_indexes {
        indexes.push(store.resolve(IndexReference::Label(label.clone()))?);
    }
    if request.compare_mathlib {
        let mathlib =
            resolve_workspace_mathlib(project_workspace.clone(), request.mathlib_workspace.clone(), reporter)?;
        let execution_root = mathlib.execution_root();
        store.build_or_reuse(
            IndexBuildRequest {
                workspace: mathlib.source.clone(),
                execution_root: Some(execution_root),
                label: "mathlib".to_owned(),
                module_root: "Mathlib".to_owned(),
                origin: "mathlib".to_owned(),
                include_private: true,
                include_generated: false,
                require_oleans: true,
                force: false,
                kind: IndexBuildKind::ProjectMathlib,
            },
            &WorkerClient::for_indexing(),
            reporter,
        )?;
        indexes.push(store.resolve(IndexReference::Label("mathlib".to_owned()))?);
    }
    let provenance = lean_dup_index::resolve_comparison_provenance(&indexes, project_workspace)?;
    Ok(CompareIndexes { indexes, provenance })
}

fn source_fact_declarations(
    workspace_rows: &[lean_dup_index::HydratedDeclaration],
    candidate_sets: &[crate::retrieval::CandidateSet],
    _compare_mathlib: bool,
    diagnostics: bool,
) -> Vec<lean_dup_index::HydratedDeclaration> {
    if diagnostics {
        return workspace_rows.to_vec();
    }

    let by_id = workspace_rows
        .iter()
        .map(|declaration| (declaration.declaration_id.as_str(), declaration))
        .collect::<BTreeMap<_, _>>();
    let mut selected = BTreeMap::new();
    for set in candidate_sets {
        if let Some(anchor) = by_id.get(set.anchor.declaration_id.as_str()) {
            selected.insert(anchor.declaration_id.clone(), (*anchor).clone());
        }
        for candidate in &set.candidates {
            if candidate.declaration.origin == "workspace" {
                selected.insert(
                    candidate.declaration.declaration_id.clone(),
                    candidate.declaration.clone(),
                );
            }
        }
    }
    selected.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_cluster_only_when_one_target_action_is_coherent() {
        let filter = review_filter(AuditVisibilityOptions::default(), false);
        let groups = [
            ranked_group("g1", "p1", ReviewAction::ReplaceLocalUses, Some("Tiny.target")),
            ranked_group("g2", "p2", ReviewAction::ReplaceLocalUses, Some("Tiny.target")),
            ranked_group("g3", "p3", ReviewAction::LocalAlias, Some("Tiny.target")),
        ];

        let families = audit_families(groups.iter(), filter, ORDINARY_PAIR_SUMMARY_LIMIT);

        assert_eq!(families.len(), 2);
        let clustered = families
            .iter()
            .find(|family| family.recommended_action == "replace-local-uses")
            .unwrap();
        assert_eq!(clustered.pair_count, 2);
        assert_eq!(clustered.pair_ids, vec!["p1", "p2"]);
        assert!(clustered.id.starts_with("family-replace-local-uses-"));

        let single = families
            .iter()
            .find(|family| family.recommended_action == "local-alias")
            .unwrap();
        assert_eq!(single.id, "g3");
        assert_eq!(single.pair_count, 1);
    }

    #[test]
    fn ordinary_family_pair_summaries_are_bounded() {
        let filter = review_filter(AuditVisibilityOptions::default(), false);
        let groups = (0..10)
            .map(|index| {
                ranked_group(
                    &format!("g{index}"),
                    &format!("p{index}"),
                    ReviewAction::ReplaceLocalUses,
                    Some("Tiny.target"),
                )
            })
            .collect::<Vec<_>>();

        let families = audit_families(groups.iter(), filter, 3);

        assert_eq!(families.len(), 1);
        assert_eq!(families[0].pair_count, 10);
        assert_eq!(families[0].pair_evidence.len(), 3);
        assert!(families[0].pair_evidence_truncated);
    }

    fn ranked_group(id: &str, pair_id: &str, action: ReviewAction, target_decl: Option<&str>) -> RankedGroup {
        RankedGroup {
            id: id.to_owned(),
            pair_id: pair_id.to_owned(),
            relation: ReviewRelation::ExactStatement,
            members: vec![
                member(&format!("{pair_id}:left"), "Tiny.left"),
                member(&format!("{pair_id}:right"), "Tiny.right"),
            ],
            evidence: Vec::new(),
            signals: Vec::new(),
            blockers: Vec::new(),
            confidence: ConfidenceTier::High,
            review_priority: ReviewPriority::High,
            recommended_action: action,
            target_decl: target_decl.map(str::to_owned),
            target_module: Some("Tiny".to_owned()),
            evidence_mode: ReviewEvidenceMode::Static,
            probe_summary: None,
            semantic_obligations: Vec::new(),
            local_caller_count: 0,
            replacement_hint: None,
        }
    }

    fn member(id: &str, name: &str) -> ReviewMember {
        ReviewMember {
            declaration_id: id.to_owned(),
            origin: "workspace".to_owned(),
            module: "Tiny".to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            source_span: None,
            status_flags: Vec::new(),
        }
    }
}
