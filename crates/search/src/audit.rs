use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use lean_dup_diagnostics::Result;
use lean_dup_diagnostics::perf::{self, CostClass};
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_index::{self, CacheFacts};
use lean_dup_index::{ComparisonProvenance, ComparisonProvenanceReport};
use lean_dup_index::{IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, OpenedIndex};
use lean_dup_project::workspace::{ResolvedWorkspace, WorkspaceRequest};
use lean_dup_worker::WorkerClient;

use crate::baseline;
use crate::ranking::{RankedReview, RankingInput, RankingProfile, ReviewFilter, ReviewPriority, rank_candidates};
use crate::replacement_hints::{ReplacementHintProfile, attach_replacement_hints, reference_declarations_for_hints};
use crate::retrieval::{RetrievalDiagnostics, retrieve_candidates};
use crate::semantic_verification::{
    ProbeDiagnostics, ProbeSettings, SemanticVerificationInput, VerificationIndex, candidate_sets_for_review,
    verify_candidate_probes,
};
use crate::source_refs::{SourceFactInput, collect_source_facts};
use crate::{ProbePolicy, ReviewProfile};

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
    pub include_imports: bool,
    pub import_roots: Vec<String>,
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub mathlib_workspace: Option<PathBuf>,
    pub threshold: f64,
    pub include_generated: bool,
    pub show_noise: bool,
    pub min_priority: ReviewPriority,
    pub review_profile: ReviewProfile,
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
    pub include_imports: bool,
    pub import_roots: Vec<String>,
    pub compare_indexes: Vec<String>,
    pub compare_mathlib: bool,
    pub threshold: f64,
    pub include_generated: bool,
    pub show_noise: bool,
    pub min_priority: ReviewPriority,
    pub review_profile: ReviewProfile,
    pub retrieval: RetrievalDiagnostics,
    pub comparison_provenance: Vec<ComparisonProvenanceReport>,
    pub semantic_verification: ProbeDiagnostics,
    pub review: RankedReview,
    pub saved_baseline: Option<PathBuf>,
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
            request.review_profile,
            request.show_noise,
        )
    });
    let source_fact_rows = source_fact_declarations(
        &workspace_rows,
        &review_candidate_sets,
        request.compare_mathlib,
        request.review_profile,
        request.show_noise,
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
    let filter = review_filter(
        request.review_profile,
        request.include_generated,
        request.show_noise,
        request.min_priority,
    );
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
    let saved_baseline = if let Some(name) = request.save_baseline {
        let snapshot = baseline::snapshot(&review, foundation.cache.fingerprint.clone());
        Some(baseline::save(&foundation.cache.root, &name, &snapshot)?)
    } else {
        None
    };

    Ok(AuditOutput {
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        selected_roots: foundation.workspace.selected_roots,
        source_count: foundation.workspace.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        include_private: request.include_private,
        include_imports: request.include_imports,
        import_roots: request.import_roots,
        compare_indexes: request.compare_indexes,
        compare_mathlib: request.compare_mathlib,
        threshold: request.threshold,
        include_generated: request.include_generated,
        show_noise: request.show_noise,
        min_priority: request.min_priority,
        review_profile: request.review_profile,
        retrieval: retrieval_output.diagnostics,
        comparison_provenance: compare.provenance.reports,
        semantic_verification: verification.diagnostics,
        review,
        saved_baseline,
    })
}

pub fn review_filter(
    profile: ReviewProfile,
    include_generated: bool,
    show_noise: bool,
    _min_priority: ReviewPriority,
) -> ReviewFilter {
    let profile_filter = match profile {
        ReviewProfile::Mathlib => ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: ReviewPriority::Medium,
        },
        ReviewProfile::Internal => ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: ReviewPriority::Medium,
        },
        ReviewProfile::ApiDesign => ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: ReviewPriority::Low,
        },
        ReviewProfile::Noise => ReviewFilter {
            include_generated: true,
            show_noise: true,
            min_priority: ReviewPriority::Noise,
        },
    };
    ReviewFilter {
        include_generated: include_generated || profile_filter.include_generated,
        show_noise: show_noise || profile_filter.show_noise,
        min_priority: profile_filter.min_priority,
    }
}

fn foundation(requested_root: PathBuf, module_root: Option<String>, reporter: &mut Reporter) -> Result<Foundation> {
    reporter.measure("workspace.resolve", |reporter| {
        let workspace = lean_dup_project::workspace::resolve(
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
        let mathlib = lean_dup_project::mathlib::resolve_for_workspace(
            project_workspace.clone(),
            request.mathlib_workspace.clone(),
            reporter,
        )?;
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
    compare_mathlib: bool,
    review_profile: ReviewProfile,
    show_noise: bool,
) -> Vec<lean_dup_index::HydratedDeclaration> {
    if !compare_mathlib || show_noise || review_profile != ReviewProfile::Mathlib {
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
