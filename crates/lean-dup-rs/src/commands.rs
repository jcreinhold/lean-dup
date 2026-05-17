use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::baseline::{self, BaselineDiff};
use crate::cache::{self, CacheFacts};
use crate::cli::{
    AuditArgs, Cli, Command, DiffArgs, DoctorArgs, EvalArgs, EvalFormat, IndexArgs, IndexMathlibArgs, OutputFormat,
    ReviewProfile, ShowArgs,
};
use crate::error::Result;
use crate::eval::{EvalRequest, EvaluationReport};
use crate::index::{
    CacheStatus, IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, IndexSummary, OpenedIndex,
};
use crate::perf::{self, CostClass};
use crate::progress::Reporter;
use crate::ranking::{
    RankedGroup, RankedReview, RankingInput, RankingProfile, ReviewFilter, ReviewPriority as RankedPriority,
    rank_candidates,
};
use crate::replacement_hints::{ReplacementHintProfile, attach_replacement_hints};
use crate::retrieval::{RetrievalDiagnostics, retrieve_candidates};
use crate::semantic_verification::{
    ProbeDiagnostics, ProbeSettings, SemanticVerificationInput, VerificationIndex, candidate_sets_for_review,
    verify_candidate_probes,
};
use crate::source_refs::{SourceFactInput, collect_source_facts};
use crate::worker::WorkerClient;
use crate::workspace::{self, ResolvedWorkspace, WorkspaceRequest};

#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) report: Report,
    pub(crate) output_format: OutputFormat,
    pub(crate) reporter: Reporter,
}

#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub(crate) enum Report {
    Doctor(DoctorReport),
    Index(IndexReport),
    IndexMathlib(IndexReport),
    Audit(AuditReport),
    Eval(EvaluationReport),
    Perf(crate::perf::PerfReport),
    Show(ShowReport),
    Diff(DiffReport),
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    pub(crate) status: &'static str,
    pub(crate) requested_workspace: PathBuf,
    pub(crate) lake_root: PathBuf,
    pub(crate) lakefile: PathBuf,
    pub(crate) module_roots: Vec<String>,
    pub(crate) selected_roots: Vec<String>,
    pub(crate) source_count: usize,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_fingerprint: String,
    pub(crate) lean_version: String,
    pub(crate) require_oleans: bool,
    pub(crate) missing_oleans: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct IndexReport {
    pub(crate) status: &'static str,
    pub(crate) requested_workspace: PathBuf,
    pub(crate) lake_root: PathBuf,
    pub(crate) selected_roots: Vec<String>,
    pub(crate) source_count: usize,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_fingerprint: String,
    pub(crate) label: String,
    pub(crate) cache_status: CacheStatus,
    pub(crate) index_path: PathBuf,
    pub(crate) index_dir: PathBuf,
    pub(crate) declaration_count: usize,
    pub(crate) diagnostics: Vec<String>,
    pub(crate) force: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuditReport {
    pub(crate) status: &'static str,
    pub(crate) requested_workspace: PathBuf,
    pub(crate) lake_root: PathBuf,
    pub(crate) selected_roots: Vec<String>,
    pub(crate) source_count: usize,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_fingerprint: String,
    pub(crate) include_private: bool,
    pub(crate) include_imports: bool,
    pub(crate) import_roots: Vec<String>,
    pub(crate) compare_indexes: Vec<String>,
    pub(crate) compare_mathlib: bool,
    pub(crate) threshold: f64,
    pub(crate) include_generated: bool,
    pub(crate) show_noise: bool,
    pub(crate) min_priority: RankedPriority,
    pub(crate) review_profile: ReviewProfile,
    pub(crate) profile_counts: ReviewProfileCounts,
    pub(crate) retrieval: RetrievalDiagnostics,
    pub(crate) semantic_verification: ProbeDiagnostics,
    pub(crate) review: RankedReview,
    pub(crate) visible_groups: Vec<RankedGroup>,
    pub(crate) visible_group_count: usize,
    pub(crate) saved_baseline: Option<PathBuf>,
    pub(crate) message: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReviewProfileCounts {
    pub(crate) mathlib: usize,
    pub(crate) internal: usize,
    pub(crate) api_design: usize,
    pub(crate) noise: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShowReport {
    pub(crate) status: &'static str,
    pub(crate) requested_workspace: PathBuf,
    pub(crate) lake_root: PathBuf,
    pub(crate) selected_roots: Vec<String>,
    pub(crate) source_count: usize,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_fingerprint: String,
    pub(crate) group: RankedGroup,
}

#[derive(Debug, Serialize)]
pub(crate) struct DiffReport {
    pub(crate) status: &'static str,
    pub(crate) requested_workspace: PathBuf,
    pub(crate) lake_root: PathBuf,
    pub(crate) selected_roots: Vec<String>,
    pub(crate) source_count: usize,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_fingerprint: String,
    pub(crate) diff: BaselineDiff,
}

struct Foundation {
    workspace: ResolvedWorkspace,
    cache: CacheFacts,
}

struct AuditComputation {
    foundation: Foundation,
    include_private: bool,
    include_imports: bool,
    import_roots: Vec<String>,
    compare_indexes: Vec<String>,
    compare_mathlib: bool,
    threshold: f64,
    include_generated: bool,
    show_noise: bool,
    min_priority: RankedPriority,
    review_profile: ReviewProfile,
    retrieval: RetrievalDiagnostics,
    semantic_verification: ProbeDiagnostics,
    review: RankedReview,
}

const INDEX_WORKER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(crate) fn run(cli: Cli) -> Result<Outcome> {
    let mut reporter = Reporter::new_live(cli.progress, cli.profile);
    let (report, output_format) = match cli.command {
        Command::Doctor(args) => (Report::Doctor(doctor(args, &mut reporter)?), OutputFormat::Text),
        Command::Index(args) => (Report::Index(index(args, &mut reporter)?), OutputFormat::Text),
        Command::IndexMathlib(args) => (
            Report::IndexMathlib(index_mathlib(args, &mut reporter)?),
            OutputFormat::Text,
        ),
        Command::Audit(args) => {
            let format = args.format;
            (Report::Audit(audit(args, &mut reporter)?), format)
        }
        Command::Eval(args) => {
            let format = if args.format == EvalFormat::Json {
                OutputFormat::Json
            } else {
                OutputFormat::Text
            };
            (Report::Eval(eval(args, &mut reporter)?), format)
        }
        Command::Perf(args) => {
            let _format = args.format;
            (Report::Perf(crate::perf::run(args)?), OutputFormat::Json)
        }
        Command::Show(args) => (Report::Show(show(args, &mut reporter)?), OutputFormat::Text),
        Command::Diff(args) => (Report::Diff(diff(args, &mut reporter)?), OutputFormat::Text),
    };

    Ok(Outcome {
        report,
        output_format,
        reporter,
    })
}

fn doctor(args: DoctorArgs, reporter: &mut Reporter) -> Result<DoctorReport> {
    let foundation = foundation(args.workspace, args.module_root, reporter)?;
    let worker_version = reporter.measure("worker.version", |_| {
        WorkerClient::with_timeout(Duration::from_secs(60)).version(foundation.workspace.root.clone())
    })?;
    let worker_version = worker_version
        .rows
        .into_iter()
        .next()
        .expect("worker version returns one version row");
    let missing_oleans = if args.require_oleans {
        missing_oleans(&foundation.workspace)
    } else {
        Vec::new()
    };

    Ok(DoctorReport {
        status: if missing_oleans.is_empty() { "ok" } else { "warning" },
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        lakefile: foundation.workspace.lakefile,
        module_roots: foundation.workspace.module_roots,
        selected_roots: foundation.workspace.selected_roots,
        source_count: foundation.workspace.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        lean_version: worker_version
            .lean_version
            .unwrap_or_else(|| "unknown Lean version".to_owned()),
        require_oleans: args.require_oleans,
        missing_oleans,
    })
}

fn index(args: IndexArgs, reporter: &mut Reporter) -> Result<IndexReport> {
    let module_root = args.module_root.clone();
    let label = args.label.clone();
    let force = args.force;
    let require_oleans = args.require_oleans;
    let foundation = foundation(args.workspace, Some(module_root.clone()), reporter)?;
    let store = IndexStore::new(foundation.cache.root.clone());
    let summary = reporter.measure("index.build_or_reuse", |reporter| {
        store.build_or_reuse(
            IndexBuildRequest {
                workspace: foundation.workspace.clone(),
                execution_root: None,
                label,
                module_root,
                origin: origin_for_label(&args.label),
                include_private: true,
                include_generated: false,
                require_oleans,
                force,
                kind: IndexBuildKind::External,
            },
            &WorkerClient::with_timeout(INDEX_WORKER_TIMEOUT),
            reporter,
        )
    })?;
    Ok(index_report(foundation, summary, force))
}

fn index_mathlib(args: IndexMathlibArgs, reporter: &mut Reporter) -> Result<IndexReport> {
    let force = args.force;
    let requested_workspace = args
        .workspace
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let project_mathlib = crate::mathlib::resolve_project(requested_workspace, args.mathlib_workspace, reporter)?;
    let project_workspace = project_mathlib.project.clone();
    let mathlib_source = project_mathlib.source.clone();
    let cache = cache::resolve_cache(&project_workspace)?;
    let foundation = Foundation {
        workspace: project_workspace.clone(),
        cache,
    };
    let store = IndexStore::new(foundation.cache.root.clone());
    let summary = reporter.measure("index.build_or_reuse", |reporter| {
        store.build_or_reuse(
            IndexBuildRequest {
                workspace: mathlib_source.clone(),
                execution_root: Some(project_mathlib.execution_root()),
                label: "mathlib".to_owned(),
                module_root: "Mathlib".to_owned(),
                origin: "mathlib".to_owned(),
                include_private: true,
                include_generated: false,
                require_oleans: true,
                force,
                kind: IndexBuildKind::ProjectMathlib,
            },
            &WorkerClient::with_timeout(INDEX_WORKER_TIMEOUT),
            reporter,
        )
    })?;
    Ok(mathlib_index_report(foundation, &mathlib_source, summary, force))
}

fn audit(args: AuditArgs, reporter: &mut Reporter) -> Result<AuditReport> {
    let save_baseline = args.save_baseline.clone();
    let computation = compute_audit(args, reporter)?;
    let filter = profile_filter(
        computation.review_profile,
        computation.include_generated,
        computation.show_noise,
        computation.min_priority,
    );
    let visible_groups = computation
        .review
        .visible_groups(filter)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let visible_group_count = visible_groups.len();
    let profile_counts = profile_counts(&computation.review);
    let saved_baseline = if let Some(name) = save_baseline {
        let snapshot = baseline::snapshot(&computation.review, computation.foundation.cache.fingerprint.clone());
        Some(baseline::save(&computation.foundation.cache.root, &name, &snapshot)?)
    } else {
        None
    };
    Ok(AuditReport {
        status: "ok",
        requested_workspace: computation.foundation.workspace.requested_root,
        lake_root: computation.foundation.workspace.root,
        selected_roots: computation.foundation.workspace.selected_roots,
        source_count: computation.foundation.workspace.source_files.len(),
        cache_root: computation.foundation.cache.root,
        cache_fingerprint: computation.foundation.cache.fingerprint,
        include_private: computation.include_private,
        include_imports: computation.include_imports,
        import_roots: computation.import_roots,
        compare_indexes: computation.compare_indexes,
        compare_mathlib: computation.compare_mathlib,
        threshold: computation.threshold,
        include_generated: computation.include_generated,
        show_noise: computation.show_noise,
        min_priority: computation.min_priority,
        review_profile: computation.review_profile,
        profile_counts,
        retrieval: computation.retrieval,
        semantic_verification: computation.semantic_verification,
        review: computation.review,
        visible_groups,
        visible_group_count,
        saved_baseline,
        message: "audit ranking queue generated",
    })
}

fn compute_audit(args: AuditArgs, reporter: &mut Reporter) -> Result<AuditComputation> {
    let include_private = args.effective_include_private();
    let module_root = args.module_root.clone();
    let force = false;
    let foundation = foundation(args.workspace.clone(), module_root.clone(), reporter)?;
    let store = IndexStore::new(foundation.cache.root.clone());
    let local_label = "audit-workspace".to_owned();
    let local_module_root = module_root
        .clone()
        .unwrap_or_else(|| foundation.workspace.selected_roots.join(","));
    reporter.measure("index.local", |reporter| {
        store.build_or_reuse(
            IndexBuildRequest {
                workspace: foundation.workspace.clone(),
                execution_root: None,
                label: local_label.clone(),
                module_root: local_module_root,
                origin: "workspace".to_owned(),
                include_private,
                include_generated: args.include_generated,
                require_oleans: false,
                force,
                kind: IndexBuildKind::Local,
            },
            &WorkerClient::with_timeout(INDEX_WORKER_TIMEOUT),
            reporter,
        )
    })?;
    let local_index = store.resolve(IndexReference::Label(local_label))?;
    let local_handles = local_index.all_handles()?;
    let workspace_rows = local_index.hydrate(&local_handles)?;
    let compare = open_compare_indexes(&args, &store, &foundation.workspace, reporter)?;
    let retrieval_output = reporter.measure("retrieval", |_| retrieve_candidates(&workspace_rows, &compare.indexes))?;
    let review_candidate_sets = perf::measure(CostClass::RetrievalRanking, "ranking.candidate_shaping", || {
        candidate_sets_for_review(
            &retrieval_output.candidate_sets,
            args.compare_mathlib,
            args.review_profile,
            args.show_noise,
        )
    });
    let source_fact_rows = source_fact_declarations(
        &workspace_rows,
        &review_candidate_sets,
        args.compare_mathlib,
        args.review_profile,
        args.show_noise,
    );
    let source_facts = perf::measure(CostClass::RetrievalRanking, "source_refs.collect", || {
        collect_source_facts(SourceFactInput::new(&source_fact_rows))
    });
    let cheap_review = perf::measure(CostClass::RetrievalRanking, "ranking.rank_candidates.initial", || {
        rank_candidates(RankingInput {
            candidate_sets: &review_candidate_sets,
            semantic_evidence: &std::collections::BTreeMap::new(),
            source_facts: &source_facts,
            profile: RankingProfile::default(),
            require_mathlib_semantic_evidence: args.compare_mathlib,
        })
    });
    let verification = verify_candidate_probes(
        SemanticVerificationInput {
            candidate_sets: &review_candidate_sets,
            cheap_review: &cheap_review,
            local_index: VerificationIndex::new(&local_index),
            workspace: &foundation.workspace,
            mathlib_source: compare.mathlib_source.as_ref(),
            enabled: args.semantic_probes,
            settings: ProbeSettings {
                policy: args.probe_policy,
                budget: args.probe_budget,
                per_declaration_cap: 2,
                chunk_size: args.probe_chunk_size,
            },
        },
        reporter,
    )?;
    let review = perf::measure(CostClass::RetrievalRanking, "ranking.rank_candidates.final", || {
        rank_candidates(RankingInput {
            candidate_sets: &review_candidate_sets,
            semantic_evidence: &verification.evidence,
            source_facts: &source_facts,
            profile: RankingProfile::default(),
            require_mathlib_semantic_evidence: args.compare_mathlib,
        })
    });
    let review = perf::measure(CostClass::RetrievalRanking, "ranking.replacement_hints", || {
        attach_replacement_hints(review, &source_facts, ReplacementHintProfile::default())
    });
    Ok(AuditComputation {
        foundation,
        include_private,
        include_imports: args.include_imports,
        import_roots: args.import_roots,
        compare_indexes: args.compare_indexes,
        compare_mathlib: args.compare_mathlib,
        threshold: args.threshold,
        include_generated: args.include_generated,
        show_noise: args.show_noise,
        min_priority: ranked_priority(args.min_priority),
        review_profile: args.review_profile,
        retrieval: retrieval_output.diagnostics,
        semantic_verification: verification.diagnostics,
        review,
    })
}

struct CompareIndexes {
    indexes: Vec<OpenedIndex>,
    mathlib_source: Option<ResolvedWorkspace>,
}

fn open_compare_indexes(
    args: &AuditArgs,
    store: &IndexStore,
    project_workspace: &ResolvedWorkspace,
    reporter: &mut Reporter,
) -> Result<CompareIndexes> {
    let mut indexes = Vec::new();
    let mut mathlib_source = None;
    for label in &args.compare_indexes {
        indexes.push(store.resolve(IndexReference::Label(label.clone()))?);
    }
    if args.compare_mathlib {
        let mathlib =
            crate::mathlib::resolve_for_workspace(project_workspace.clone(), args.mathlib_workspace.clone(), reporter)?;
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
            &WorkerClient::with_timeout(INDEX_WORKER_TIMEOUT),
            reporter,
        )?;
        mathlib_source = Some(mathlib.source);
        indexes.push(store.resolve(IndexReference::Label("mathlib".to_owned()))?);
    }
    Ok(CompareIndexes {
        indexes,
        mathlib_source,
    })
}

fn source_fact_declarations(
    workspace_rows: &[crate::index::HydratedDeclaration],
    candidate_sets: &[crate::retrieval::CandidateSet],
    compare_mathlib: bool,
    review_profile: ReviewProfile,
    show_noise: bool,
) -> Vec<crate::index::HydratedDeclaration> {
    if !compare_mathlib || show_noise || review_profile != ReviewProfile::Mathlib {
        return workspace_rows.to_vec();
    }

    let by_id = workspace_rows
        .iter()
        .map(|declaration| (declaration.declaration_id.as_str(), declaration))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut selected = std::collections::BTreeMap::new();
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

fn eval(args: EvalArgs, reporter: &mut Reporter) -> Result<EvaluationReport> {
    crate::eval::run(
        EvalRequest {
            suite: args.suite,
            workspace: args.workspace,
            mathlib_workspace: args.mathlib_workspace,
            k_values: args.k_values,
        },
        reporter,
    )
}

fn show(args: ShowArgs, reporter: &mut Reporter) -> Result<ShowReport> {
    let requested_group = args.group.clone();
    let computation = compute_audit(default_audit_args(args.workspace, args.module_root), reporter)?;
    let group = computation
        .review
        .groups
        .iter()
        .find(|group| group.id == requested_group)
        .cloned()
        .ok_or_else(|| crate::error::Error::Index {
            message: format!("unknown audit group: {requested_group}"),
        })?;
    Ok(ShowReport {
        status: "ok",
        requested_workspace: computation.foundation.workspace.requested_root,
        lake_root: computation.foundation.workspace.root,
        selected_roots: computation.foundation.workspace.selected_roots,
        source_count: computation.foundation.workspace.source_files.len(),
        cache_root: computation.foundation.cache.root,
        cache_fingerprint: computation.foundation.cache.fingerprint,
        group,
    })
}

fn diff(args: DiffArgs, reporter: &mut Reporter) -> Result<DiffReport> {
    let baseline_name = args.baseline.clone();
    let computation = compute_audit(default_audit_args(args.workspace, args.module_root), reporter)?;
    let (baseline_path, saved) = baseline::load(&computation.foundation.cache.root, &baseline_name)?;
    let current = baseline::snapshot(&computation.review, computation.foundation.cache.fingerprint.clone());
    let diff = baseline::diff(baseline_name, baseline_path, saved, current);
    Ok(DiffReport {
        status: "ok",
        requested_workspace: computation.foundation.workspace.requested_root,
        lake_root: computation.foundation.workspace.root,
        selected_roots: computation.foundation.workspace.selected_roots,
        source_count: computation.foundation.workspace.source_files.len(),
        cache_root: computation.foundation.cache.root,
        cache_fingerprint: computation.foundation.cache.fingerprint,
        diff,
    })
}

fn foundation(requested_root: PathBuf, module_root: Option<String>, reporter: &mut Reporter) -> Result<Foundation> {
    reporter.measure("workspace.resolve", |reporter| {
        let workspace = workspace::resolve(
            WorkspaceRequest {
                requested_root,
                module_root,
            },
            reporter,
        )?;
        let cache = cache::resolve_cache(&workspace)?;
        reporter.event("cache", None, None, format!("cache root {}", cache.root.display()));
        Ok(Foundation { workspace, cache })
    })
}

fn index_report(foundation: Foundation, summary: IndexSummary, force: bool) -> IndexReport {
    IndexReport {
        status: "ok",
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        selected_roots: foundation.workspace.selected_roots,
        source_count: foundation.workspace.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        label: summary.label,
        cache_status: summary.cache_status,
        index_path: summary.path,
        index_dir: summary.index_dir,
        declaration_count: summary.declaration_count,
        diagnostics: summary.diagnostics,
        force,
    }
}

fn mathlib_index_report(
    foundation: Foundation,
    mathlib_source: &ResolvedWorkspace,
    summary: IndexSummary,
    force: bool,
) -> IndexReport {
    IndexReport {
        status: "ok",
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        selected_roots: mathlib_source.selected_roots.clone(),
        source_count: mathlib_source.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        label: summary.label,
        cache_status: summary.cache_status,
        index_path: summary.path,
        index_dir: summary.index_dir,
        declaration_count: summary.declaration_count,
        diagnostics: summary.diagnostics,
        force,
    }
}

fn origin_for_label(label: &str) -> String {
    if label == "mathlib" {
        "mathlib".to_owned()
    } else if label == "workspace" {
        "workspace".to_owned()
    } else {
        format!("external:{label}")
    }
}

fn ranked_priority(priority: crate::cli::ReviewPriority) -> RankedPriority {
    match priority {
        crate::cli::ReviewPriority::High => RankedPriority::High,
        crate::cli::ReviewPriority::Medium => RankedPriority::Medium,
        crate::cli::ReviewPriority::Low => RankedPriority::Low,
        crate::cli::ReviewPriority::Noise => RankedPriority::Noise,
    }
}

fn profile_filter(
    profile: ReviewProfile,
    include_generated: bool,
    show_noise: bool,
    _min_priority: RankedPriority,
) -> ReviewFilter {
    let profile_filter = match profile {
        ReviewProfile::Mathlib => ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: RankedPriority::Medium,
        },
        ReviewProfile::Internal => ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: RankedPriority::Medium,
        },
        ReviewProfile::ApiDesign => ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: RankedPriority::Low,
        },
        ReviewProfile::Noise => ReviewFilter {
            include_generated: true,
            show_noise: true,
            min_priority: RankedPriority::Noise,
        },
    };
    ReviewFilter {
        include_generated: include_generated || profile_filter.include_generated,
        show_noise: show_noise || profile_filter.show_noise,
        min_priority: profile_filter.min_priority,
    }
}

fn profile_counts(review: &RankedReview) -> ReviewProfileCounts {
    ReviewProfileCounts {
        mathlib: review
            .visible_groups(profile_filter(
                ReviewProfile::Mathlib,
                false,
                false,
                RankedPriority::Low,
            ))
            .len(),
        internal: review
            .visible_groups(profile_filter(
                ReviewProfile::Internal,
                false,
                false,
                RankedPriority::Low,
            ))
            .len(),
        api_design: review
            .visible_groups(profile_filter(
                ReviewProfile::ApiDesign,
                false,
                false,
                RankedPriority::Low,
            ))
            .len(),
        noise: review
            .visible_groups(profile_filter(ReviewProfile::Noise, false, false, RankedPriority::Low))
            .len(),
    }
}

fn default_audit_args(workspace: PathBuf, module_root: Option<String>) -> AuditArgs {
    AuditArgs {
        workspace,
        module_root,
        format: OutputFormat::Text,
        public_only: false,
        include_private: true,
        no_include_private: true,
        include_imports: false,
        import_roots: Vec::new(),
        compare_indexes: Vec::new(),
        compare_mathlib: false,
        mathlib_workspace: None,
        threshold: 0.78,
        include_generated: false,
        show_noise: false,
        min_priority: crate::cli::ReviewPriority::Low,
        review_profile: ReviewProfile::Mathlib,
        save_baseline: None,
        semantic_probes: true,
        probe_budget: 500,
        probe_policy: crate::cli::ProbePolicy::Actionable,
        probe_chunk_size: 16,
        replacement_hints: true,
    }
}

fn missing_oleans(workspace: &ResolvedWorkspace) -> Vec<String> {
    workspace
        .source_files
        .iter()
        .filter(|source| !workspace::olean_exists(&workspace.root, &source.module))
        .map(|source| source.module.clone())
        .collect()
}
