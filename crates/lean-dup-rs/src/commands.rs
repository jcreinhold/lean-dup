use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::cache::{self, CacheFacts};
use crate::cli::{
    AuditArgs, Cli, Command, DiffArgs, DoctorArgs, EvalArgs, EvalFormat, IndexArgs,
    IndexMathlibArgs, OutputFormat, ShowArgs,
};
use crate::error::Result;
use crate::eval::{EvalRequest, EvaluationReport};
use crate::index::{
    CacheStatus, IndexBuildKind, IndexBuildRequest, IndexReference, IndexStore, IndexSummary,
    OpenedIndex, ProbeCacheEntry,
};
use crate::progress::Reporter;
use crate::ranking::{
    RankedReview, RankingInput, RankingProfile, ReviewFilter, ReviewPriority as RankedPriority,
    rank_candidates,
};
use crate::replacement_hints::{ReplacementHintProfile, attach_replacement_hints};
use crate::retrieval::{RetrievalDiagnostics, RetrievalOutput, retrieve_candidates};
use crate::source_refs::{SourceFactInput, collect_source_facts};
use crate::worker::WorkerClient;
use crate::worker::{ModuleDescriptor, ProbeBatch, ProbePair, ProbeResult};
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
    Show(SkeletonReport),
    Diff(SkeletonReport),
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
pub(crate) struct SkeletonReport {
    pub(crate) status: &'static str,
    pub(crate) requested_workspace: PathBuf,
    pub(crate) lake_root: PathBuf,
    pub(crate) selected_roots: Vec<String>,
    pub(crate) source_count: usize,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_fingerprint: String,
    pub(crate) message: &'static str,
    pub(crate) label: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) baseline: Option<PathBuf>,
    pub(crate) force: bool,
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
    pub(crate) retrieval: RetrievalDiagnostics,
    pub(crate) review: RankedReview,
    pub(crate) visible_group_count: usize,
    pub(crate) message: &'static str,
}

struct Foundation {
    workspace: ResolvedWorkspace,
    cache: CacheFacts,
}

const INDEX_WORKER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(crate) fn run(cli: Cli) -> Result<Outcome> {
    let mut reporter = Reporter::new(cli.progress, cli.profile);
    let (report, output_format) = match cli.command {
        Command::Doctor(args) => (
            Report::Doctor(doctor(args, &mut reporter)?),
            OutputFormat::Text,
        ),
        Command::Index(args) => (
            Report::Index(index(args, &mut reporter)?),
            OutputFormat::Text,
        ),
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
        WorkerClient::with_timeout(Duration::from_secs(60))
            .version(foundation.workspace.root.clone())
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
        status: if missing_oleans.is_empty() {
            "ok"
        } else {
            "warning"
        },
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
    let foundation = foundation(args.workspace, Some("Mathlib".to_owned()), reporter)?;
    let store = IndexStore::new(foundation.cache.root.clone());
    let summary = reporter.measure("index.build_or_reuse", |reporter| {
        store.build_or_reuse(
            IndexBuildRequest {
                workspace: foundation.workspace.clone(),
                label: "mathlib".to_owned(),
                module_root: "Mathlib".to_owned(),
                origin: "mathlib".to_owned(),
                include_private: true,
                include_generated: false,
                require_oleans: true,
                force,
                kind: IndexBuildKind::External,
            },
            &WorkerClient::with_timeout(INDEX_WORKER_TIMEOUT),
            reporter,
        )
    })?;
    Ok(index_report(foundation, summary, force))
}

fn audit(args: AuditArgs, reporter: &mut Reporter) -> Result<AuditReport> {
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
    let compare_indexes = open_compare_indexes(&args, &store, reporter)?;
    let retrieval_output = reporter.measure("retrieval", |_| {
        retrieve_candidates(&workspace_rows, &compare_indexes)
    })?;
    let probe_results = collect_probe_results(
        &retrieval_output,
        &local_index,
        &foundation.workspace,
        args.semantic_probes,
        reporter,
    )?;
    let source_facts = collect_source_facts(SourceFactInput::new(&workspace_rows));
    let review = rank_candidates(RankingInput {
        candidate_sets: &retrieval_output.candidate_sets,
        probe_results: &probe_results,
        source_facts: &source_facts,
        profile: RankingProfile::default(),
    });
    let review = attach_replacement_hints(review, &source_facts, ReplacementHintProfile::default());
    let min_priority = ranked_priority(args.min_priority);
    let filter = ReviewFilter {
        include_generated: args.include_generated,
        show_noise: args.show_noise,
        min_priority,
    };
    let visible_group_count = review.visible_groups(filter).len();
    Ok(AuditReport {
        status: "ok",
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        selected_roots: foundation.workspace.selected_roots,
        source_count: foundation.workspace.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        include_private,
        include_imports: args.include_imports,
        import_roots: args.import_roots,
        compare_indexes: args.compare_indexes,
        compare_mathlib: args.compare_mathlib,
        threshold: args.threshold,
        include_generated: args.include_generated,
        show_noise: args.show_noise,
        min_priority,
        retrieval: retrieval_output.diagnostics,
        review,
        visible_group_count,
        message: "audit ranking queue generated",
    })
}

fn open_compare_indexes(
    args: &AuditArgs,
    store: &IndexStore,
    reporter: &mut Reporter,
) -> Result<Vec<OpenedIndex>> {
    let mut indexes = Vec::new();
    for label in &args.compare_indexes {
        indexes.push(store.resolve(IndexReference::Label(label.clone()))?);
    }
    if args.compare_mathlib {
        if let Some(mathlib_workspace) = &args.mathlib_workspace {
            let mathlib = workspace::resolve(
                WorkspaceRequest {
                    requested_root: mathlib_workspace.clone(),
                    module_root: Some("Mathlib".to_owned()),
                },
                reporter,
            )?;
            store.build_or_reuse(
                IndexBuildRequest {
                    workspace: mathlib,
                    label: "mathlib".to_owned(),
                    module_root: "Mathlib".to_owned(),
                    origin: "mathlib".to_owned(),
                    include_private: true,
                    include_generated: false,
                    require_oleans: true,
                    force: false,
                    kind: IndexBuildKind::External,
                },
                &WorkerClient::with_timeout(INDEX_WORKER_TIMEOUT),
                reporter,
            )?;
        }
        indexes.push(store.resolve(IndexReference::Label("mathlib".to_owned()))?);
    }
    Ok(indexes)
}

fn collect_probe_results(
    output: &RetrievalOutput,
    local_index: &OpenedIndex,
    workspace: &ResolvedWorkspace,
    enabled: bool,
    reporter: &mut Reporter,
) -> Result<std::collections::BTreeMap<String, ProbeResult>> {
    let mut results = std::collections::BTreeMap::new();
    if !enabled {
        return Ok(results);
    }
    let mut missing_pairs = Vec::new();
    for set in &output.candidate_sets {
        for candidate in &set.candidates {
            if candidate.declaration.origin != "workspace" {
                continue;
            }
            let pair = ProbePair {
                pair_id: candidate.pair_id.clone(),
                left_declaration_id: set.anchor.declaration_id.clone(),
                right_declaration_id: candidate.declaration.declaration_id.clone(),
            };
            if let Some(cached) = local_index.cached_probe_result(&pair)? {
                results.insert(candidate.pair_id.clone(), cached);
            } else {
                missing_pairs.push(pair);
            }
        }
    }
    if missing_pairs.is_empty() {
        return Ok(results);
    }
    let modules = workspace
        .selected_roots
        .iter()
        .map(|module| ModuleDescriptor {
            module: module.clone(),
            origin: "workspace".to_owned(),
        })
        .collect::<Vec<_>>();
    let call = reporter.measure("worker.probe", |_| {
        WorkerClient::with_timeout(Duration::from_secs(5 * 60)).probe_batch(ProbeBatch {
            workspace_root: workspace.root.clone(),
            modules,
            pairs: missing_pairs.clone(),
            max_pairs: Some(missing_pairs.len() as u64),
        })
    })?;
    let pairs_by_id = missing_pairs
        .into_iter()
        .map(|pair| (pair.pair_id.clone(), pair))
        .collect::<std::collections::BTreeMap<_, _>>();
    let entries = call
        .rows
        .iter()
        .filter_map(|result| {
            pairs_by_id
                .get(&result.pair_id)
                .cloned()
                .map(|pair| ProbeCacheEntry {
                    pair,
                    result: result.clone(),
                })
        })
        .collect::<Vec<_>>();
    local_index.cache_probe_results(&entries)?;
    for result in call.rows {
        results.insert(result.pair_id.clone(), result);
    }
    Ok(results)
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

fn show(args: ShowArgs, reporter: &mut Reporter) -> Result<SkeletonReport> {
    let foundation = foundation(args.workspace, None, reporter)?;
    Ok(skeleton(
        foundation,
        "show is stubbed until report persistence is implemented in prompt 14",
        None,
        Some(args.group),
        None,
        false,
    ))
}

fn diff(args: DiffArgs, reporter: &mut Reporter) -> Result<SkeletonReport> {
    let foundation = foundation(args.workspace, None, reporter)?;
    Ok(skeleton(
        foundation,
        "diff is stubbed until baseline reports are implemented in prompt 14",
        None,
        None,
        Some(args.baseline),
        false,
    ))
}

fn foundation(
    requested_root: PathBuf,
    module_root: Option<String>,
    reporter: &mut Reporter,
) -> Result<Foundation> {
    reporter.measure("workspace.resolve", |reporter| {
        let workspace = workspace::resolve(
            WorkspaceRequest {
                requested_root,
                module_root,
            },
            reporter,
        )?;
        let cache = cache::resolve_cache(&workspace)?;
        reporter.event(
            "cache",
            None,
            None,
            format!("cache root {}", cache.root.display()),
        );
        Ok(Foundation { workspace, cache })
    })
}

fn skeleton(
    foundation: Foundation,
    message: &'static str,
    label: Option<String>,
    group: Option<String>,
    baseline: Option<PathBuf>,
    force: bool,
) -> SkeletonReport {
    SkeletonReport {
        status: "stub",
        requested_workspace: foundation.workspace.requested_root,
        lake_root: foundation.workspace.root,
        selected_roots: foundation.workspace.selected_roots,
        source_count: foundation.workspace.source_files.len(),
        cache_root: foundation.cache.root,
        cache_fingerprint: foundation.cache.fingerprint,
        message,
        label,
        group,
        baseline,
        force,
    }
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

fn missing_oleans(workspace: &ResolvedWorkspace) -> Vec<String> {
    workspace
        .source_files
        .iter()
        .filter(|source| !workspace::olean_exists(&workspace.root, &source.module))
        .map(|source| source.module.clone())
        .collect()
}
