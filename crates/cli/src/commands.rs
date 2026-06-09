use std::path::PathBuf;
use std::time::Duration;

use crate::cli::{
    AuditArgs, BaselineAction, BaselineArgs, BaselineCommonArgs, CacheCleanupArgs, Cli, Command, DiffArgs, DoctorArgs,
    EvalArgs, EvalFormat, IndexArgs, IndexMathlibArgs, OutputFormat, ShowArgs,
};
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_eval::EvalRequest;
use lean_dup_index::CleanupPolicy;
use lean_dup_index::{self, CacheFacts};
use lean_dup_index::{IndexBuildKind, IndexBuildRequest, IndexStore, IndexSummary};
use lean_dup_project::{ResolvedWorkspace, WorkspaceRequest, resolve, resolve_project_mathlib};
use lean_dup_report::{
    AuditReport, BaselineReport, BaselineSummaryReport, CacheCleanupReportDto, DiffReport, DoctorReport, IndexReport,
    RenderOptions, Report, ShowReport,
};
use lean_dup_search::{
    AuditRequest, BaselineSnapshot, DiffOutput, baseline_name_is_valid, baseline_path, baselines_dir, diff_snapshots,
    load_last_audit_detail, load_last_audit_snapshot, load_named_baseline, run_audit, run_diff, run_show,
};
use lean_dup_worker::{WorkerClient, WorkerVersion};

use crate::error::{AppError, Result};

#[derive(Debug)]
pub struct Outcome {
    pub report: Report,
    pub output_format: OutputFormat,
    pub output_path: Option<PathBuf>,
    pub reporter: Reporter,
    pub render_options: RenderOptions,
}

struct Foundation {
    workspace: ResolvedWorkspace,
    cache: CacheFacts,
}

pub fn run(cli: Cli) -> Result<Outcome> {
    let progress = !cli.no_progress && (cli.progress || std::io::IsTerminal::is_terminal(&std::io::stderr()));
    let mut reporter = Reporter::new_live(progress, cli.profile);
    let command = cli.command.ok_or_else(|| AppError::Cli {
        message: "missing command; run `lean-dup --help`".to_owned(),
    })?;
    let mut render_options = RenderOptions::default();
    let (report, output_format, output_path) = match command {
        Command::Doctor(args) => {
            let format = args.format;
            render_options.verbose = args.verbose;
            (Report::Doctor(doctor(args, &mut reporter)?), format, None)
        }
        Command::CacheCleanup(args) => {
            let format = args.format;
            render_options.verbose = args.verbose;
            (Report::CacheCleanup(cache_cleanup(args, &mut reporter)?), format, None)
        }
        Command::Index(args) => {
            let format = args.format;
            (Report::Index(index(args, &mut reporter)?), format, None)
        }
        Command::IndexMathlib(args) => {
            let format = args.format;
            (Report::IndexMathlib(index_mathlib(args, &mut reporter)?), format, None)
        }
        Command::Audit(args) => {
            let format = args.format;
            render_options.verbose = render_options.verbose || args.verbose;
            render_options.audit_limit = args.limit;
            (Report::Audit(Box::new(audit(args, &mut reporter)?)), format, None)
        }
        Command::Eval(args) => {
            let format = if args.format == EvalFormat::Json {
                OutputFormat::Json
            } else {
                OutputFormat::Text
            };
            let output_path = args.output.clone();
            (Report::Eval(Box::new(eval(args, &mut reporter)?)), format, output_path)
        }
        Command::Perf(args) => {
            let _format = args.format;
            (Report::Perf(crate::perf::run(args)?), OutputFormat::Json, None)
        }
        Command::Show(args) => {
            let format = args.format;
            render_options.verbose = args.verbose;
            (Report::Show(Box::new(show(args, &mut reporter)?)), format, None)
        }
        Command::Diff(args) => {
            let format = args.format;
            render_options.verbose = args.verbose;
            (Report::Diff(diff(args, &mut reporter)?), format, None)
        }
        Command::Baseline(args) => {
            let common = baseline_common(&args.action);
            let format = common.format;
            render_options.verbose = common.verbose;
            (Report::Baseline(baseline(args, &mut reporter)?), format, None)
        }
        Command::External(_) => {
            return Err(AppError::Cli {
                message: "external command dispatch must happen before built-in command execution".to_owned(),
            });
        }
    };

    Ok(Outcome {
        report,
        output_format,
        output_path,
        reporter,
        render_options,
    })
}

fn doctor(args: DoctorArgs, reporter: &mut Reporter) -> Result<DoctorReport> {
    let foundation = foundation(
        workspace_or_cwd(pick_workspace(args.workspace, args.workspace_positional)),
        args.module_root,
        reporter,
    )?;
    let worker_identity = reporter.measure("worker.version", |_| {
        WorkerClient::with_timeout(Duration::from_secs(60)).worker_identity(foundation.workspace.root.clone())
    })?;
    let worker_identity = worker_identity
        .rows
        .into_iter()
        .next()
        .expect("worker version returns one version row");
    let worker_version = worker_identity.semantic.clone();
    let store = IndexStore::new(foundation.cache.root.clone());
    let current_index = store.expected_entry(
        &IndexBuildRequest {
            workspace: foundation.workspace.clone(),
            execution_root: None,
            label: "audit-workspace".to_owned(),
            module_root: foundation.workspace.selected_roots.join(","),
            origin: "workspace".to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans: false,
            force: false,
            kind: IndexBuildKind::Local,
            max_heartbeats: None,
        },
        &worker_identity,
    )?;
    let cache_diagnostics = lean_dup_report::cache_diagnostics_report(lean_dup_index::diagnose_cache(
        foundation.cache.root.clone(),
        &[current_index],
        &store,
    )?);
    let missing_oleans = if args.require_oleans {
        missing_oleans(&foundation.workspace)
    } else {
        Vec::new()
    };
    let cache_has_problems = cache_diagnostics.labels.iter().any(|label| {
        // `missing` is the natural state of a fresh or never-audited cache;
        // only flag genuinely broken pointers or damaged entries.
        matches!(label.latest.status.as_str(), "corrupt-pointer" | "target-missing")
            || label.entries.iter().any(|entry| {
                (entry.active_latest || entry.expected_current) && matches!(entry.status.as_str(), "corrupt" | "stale")
            })
    });

    Ok(DoctorReport {
        report_schema_version: lean_dup_report::REPORT_SCHEMA_VERSION,
        release: crate::release::identity(),
        status: if missing_oleans.is_empty() && !cache_has_problems {
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
        cache: cache_diagnostics,
        worker: worker_diagnostics(&worker_version),
        lean_version: worker_version
            .lean_version
            .unwrap_or_else(|| "unknown Lean version".to_owned()),
        require_oleans: args.require_oleans,
        missing_oleans,
    })
}

fn worker_diagnostics(version: &WorkerVersion) -> lean_dup_report::WorkerDiagnosticsReport {
    lean_dup_report::WorkerDiagnosticsReport {
        protocol_version: version.protocol_version.clone(),
        worker_version: version.worker_version.clone(),
        lean_version: version
            .lean_version
            .clone()
            .unwrap_or_else(|| "unknown Lean version".to_owned()),
        extract_version: version.extract_version.clone(),
        features_version: version.features_version.clone(),
        probe_version: version.probe_version.clone(),
        supported_commands: version.supported_commands.clone(),
        supported_capabilities: version.supported_capabilities.clone(),
    }
}

fn cache_cleanup(args: CacheCleanupArgs, reporter: &mut Reporter) -> Result<CacheCleanupReportDto> {
    let cache_root = args.cache_root.unwrap_or_else(lean_dup_index::cache_root);
    let store = IndexStore::new(cache_root.clone());
    let mut protected_fingerprints: Vec<String> = Vec::new();
    let expected_entries = if let Some(workspace_root) = pick_workspace(args.workspace, args.workspace_positional) {
        let workspace = resolve(
            WorkspaceRequest {
                requested_root: workspace_root,
                module_root: args.module_root,
            },
            reporter,
        )?;
        if let Ok(cache) = lean_dup_index::resolve_cache(&workspace) {
            protected_fingerprints.push(cache.fingerprint);
        }
        let version_call =
            WorkerClient::with_timeout(Duration::from_secs(60)).worker_identity(workspace.root.clone())?;
        let worker_identity = version_call.rows.into_iter().next().ok_or_else(|| AppError::Cli {
            message: "worker version returned no rows".to_owned(),
        })?;
        vec![store.expected_entry(
            &IndexBuildRequest {
                workspace: workspace.clone(),
                execution_root: None,
                label: "audit-workspace".to_owned(),
                module_root: workspace.selected_roots.join(","),
                origin: "workspace".to_owned(),
                include_private: true,
                include_generated: false,
                require_oleans: false,
                force: false,
                kind: IndexBuildKind::Local,
                max_heartbeats: None,
            },
            &worker_identity,
        )?]
    } else {
        Vec::new()
    };
    let policy = CleanupPolicy { execute: args.execute };
    let workspace_files = lean_dup_search::cleanup_stale_workspace_files(&cache_root, &protected_fingerprints, policy)?;
    let index_report = lean_dup_index::cleanup_cache(cache_root, &expected_entries, policy)?;
    Ok(lean_dup_report::cache_cleanup_report(
        index_report,
        Some(workspace_files),
    ))
}

fn index(args: IndexArgs, reporter: &mut Reporter) -> Result<IndexReport> {
    let module_root = args.module_root.clone();
    let label = args.label.clone();
    let force = args.force;
    let require_oleans = args.require_oleans;
    let foundation = foundation(
        workspace_or_cwd(pick_workspace(args.workspace, args.workspace_positional)),
        Some(module_root.clone()),
        reporter,
    )?;
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
                max_heartbeats: args.max_heartbeats,
            },
            &WorkerClient::for_indexing(),
            reporter,
        )
    })?;
    Ok(index_report(foundation, summary, force))
}

fn index_mathlib(args: IndexMathlibArgs, reporter: &mut Reporter) -> Result<IndexReport> {
    let force = args.force;
    let requested_workspace = workspace_or_cwd(args.workspace);
    let project_mathlib = resolve_project_mathlib(requested_workspace, args.mathlib_workspace, reporter)?;
    let project_workspace = project_mathlib.project.clone();
    let mathlib_source = project_mathlib.source.clone();
    let cache = lean_dup_index::resolve_cache(&project_workspace)?;
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
                max_heartbeats: args.max_heartbeats,
            },
            &WorkerClient::for_indexing(),
            reporter,
        )
    })?;
    Ok(mathlib_index_report(foundation, &mathlib_source, summary, force))
}

fn audit(args: AuditArgs, reporter: &mut Reporter) -> Result<AuditReport> {
    let output = run_audit(audit_request(args), reporter)?;
    Ok(lean_dup_report::audit_report(output))
}

fn audit_request(args: AuditArgs) -> AuditRequest {
    let include_private = args.effective_include_private();
    let visibility = args.visibility_options();
    AuditRequest {
        workspace: workspace_or_cwd(pick_workspace(args.workspace, args.workspace_positional)),
        module_root: args.module_root,
        include_private,
        compare_indexes: args.compare_indexes,
        compare_mathlib: args.compare_mathlib,
        mathlib_workspace: args.mathlib_workspace,
        include_generated: args.include_generated,
        visibility,
        save_baseline: args.save_baseline,
        semantic_probes: args.semantic_probes,
        probe_budget: args.probe_budget,
        probe_policy: args.probe_policy.into(),
        probe_chunk_size: args.probe_chunk_size,
        max_heartbeats: args.max_heartbeats,
    }
}

fn eval(args: EvalArgs, reporter: &mut Reporter) -> Result<lean_dup_report::EvalReportDto> {
    let output_path = args.output.clone();
    let mut report = lean_dup_report::eval_report(lean_dup_eval::run(
        EvalRequest {
            suite: args.suite.into(),
            workspace: args.workspace,
            mathlib_workspace: args.mathlib_workspace,
            manual_module: args.manual_module,
            k_values: args.k_values,
            write_search_dataset: args.write_search_dataset,
            write_scorer_ablations: args.write_scorer_ablations,
        },
        reporter,
    )?);
    report.artifact_path = output_path;
    Ok(report)
}

fn show(args: ShowArgs, reporter: &mut Reporter) -> Result<ShowReport> {
    let workspace = pick_workspace(args.workspace.clone(), args.workspace_positional.clone());
    let module_root = args.module_root.clone();
    let snapshot = if args.no_cache {
        None
    } else {
        load_snapshot_for(workspace.clone(), module_root.clone(), reporter)
    };
    let requested_group = match snapshot.as_ref() {
        Some(snapshot) => match resolve_group(snapshot, &args.group) {
            ResolveOutcome::Exact(id) | ResolveOutcome::Unique(id) => id,
            ResolveOutcome::Ambiguous(matches) => {
                return Err(AppError::Cli {
                    message: format!("ambiguous group `{}` — matches: {}", args.group, matches.join(", ")),
                });
            }
            ResolveOutcome::TooShort => {
                return Err(AppError::Cli {
                    message: format!(
                        "group `{}` is too short — provide at least 6 characters for prefix/suffix matching",
                        args.group
                    ),
                });
            }
            ResolveOutcome::None => {
                return Err(unknown_group_error(&args.group, snapshot));
            }
        },
        None => args.group.clone(),
    };
    if !args.no_cache
        && let Some(report) = try_show_from_detail(workspace.clone(), module_root.clone(), &requested_group, reporter)
    {
        return Ok(report);
    }
    match run_show(
        audit_request(default_audit_args(workspace, module_root)),
        &requested_group,
        reporter,
    ) {
        Ok(output) => Ok(lean_dup_report::show_report(output)),
        Err(error) => Err(decorate_unknown_group(error, &requested_group, snapshot.as_ref())),
    }
}

/// Try to render `show` from the persisted per-audit detail snapshot, skipping
/// the full audit pipeline. Returns `None` on any miss (no workspace, no
/// cache facts, no detail file, schema mismatch, or group not present), so
/// the caller falls through to the slow path.
fn try_show_from_detail(
    workspace: Option<PathBuf>,
    module_root: Option<String>,
    requested_group: &str,
    reporter: &mut Reporter,
) -> Option<ShowReport> {
    let resolved = resolve(
        WorkspaceRequest {
            requested_root: workspace_or_cwd(workspace),
            module_root,
        },
        reporter,
    )
    .ok()?;
    let cache = lean_dup_index::resolve_cache(&resolved).ok()?;
    let detail = load_last_audit_detail(&cache.root, &cache.fingerprint)?;
    let group = detail.resolve(requested_group)?;
    Some(lean_dup_report::show_report_from_detail(&detail, group))
}

/// If `run_show` reports the group is unknown but the snapshot we loaded does
/// contain it, the snapshot is stale: source files changed between the last
/// `audit` and now. Steer the user toward `lean-dup audit` rather than
/// repeating the bare error.
fn decorate_unknown_group(
    error: lean_dup_search::Error,
    requested: &str,
    snapshot: Option<&BaselineSnapshot>,
) -> AppError {
    let message = error.to_string();
    if !message.contains("unknown audit group") {
        return AppError::Search(error);
    }
    let Some(snapshot) = snapshot else {
        return AppError::Search(error);
    };
    if matches!(
        resolve_group(snapshot, requested),
        ResolveOutcome::None | ResolveOutcome::TooShort
    ) {
        return AppError::Search(error);
    }
    AppError::Cli {
        message: format!(
            "{message}\nhelp: this group was in the last audit snapshot but is missing from current findings — \
             source files may have changed; run `lean-dup audit` again to refresh"
        ),
    }
}

/// Resolve the workspace just enough to look up the cache root and fingerprint,
/// then load the persisted "last audit" snapshot for that workspace if one
/// exists. Returns `None` on any error so callers fall through to the slow
/// audit pipeline (which produces its own diagnostics).
fn load_snapshot_for(
    workspace: Option<PathBuf>,
    module_root: Option<String>,
    reporter: &mut Reporter,
) -> Option<BaselineSnapshot> {
    let resolved = resolve(
        WorkspaceRequest {
            requested_root: workspace_or_cwd(workspace),
            module_root,
        },
        reporter,
    )
    .ok()?;
    let cache = lean_dup_index::resolve_cache(&resolved).ok()?;
    load_last_audit_snapshot(&cache.root, &cache.fingerprint)
}

enum ResolveOutcome {
    Exact(String),
    Unique(String),
    Ambiguous(Vec<String>),
    TooShort,
    None,
}

const GROUP_PREFIX_MIN: usize = 6;

/// Match the user's group ID against the snapshot. Exact match wins
/// outright; otherwise (provided the input is at least `GROUP_PREFIX_MIN`
/// characters) try prefix then suffix match against group and member IDs.
fn resolve_group(snapshot: &BaselineSnapshot, requested: &str) -> ResolveOutcome {
    let mut all_ids: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for group in &snapshot.groups {
        all_ids.insert(group.id.as_str());
        for member in &group.member_ids {
            all_ids.insert(member.as_str());
        }
    }
    if all_ids.contains(requested) {
        return ResolveOutcome::Exact(requested.to_owned());
    }
    if requested.len() < GROUP_PREFIX_MIN {
        return ResolveOutcome::TooShort;
    }
    let mut hits: Vec<String> = all_ids
        .iter()
        .filter(|id| id.starts_with(requested))
        .map(|id| (*id).to_owned())
        .collect();
    if hits.is_empty() {
        hits = all_ids
            .iter()
            .filter(|id| id.ends_with(requested))
            .map(|id| (*id).to_owned())
            .collect();
    }
    match hits.len() {
        0 => ResolveOutcome::None,
        1 => ResolveOutcome::Unique(hits.into_iter().next().unwrap()),
        _ => ResolveOutcome::Ambiguous(hits),
    }
}

fn unknown_group_error(requested: &str, snapshot: &BaselineSnapshot) -> AppError {
    let ids: Vec<&str> = snapshot.groups.iter().map(|group| group.id.as_str()).collect();
    let hint = crate::extensions::nearest_match(requested, ids.iter().copied(), 4)
        .map(|suggestion| format!(" — did you mean `{suggestion}`?"))
        .unwrap_or_default();
    AppError::Cli {
        message: format!("unknown audit group: {requested}{hint}"),
    }
}

fn diff(args: DiffArgs, reporter: &mut Reporter) -> Result<DiffReport> {
    let baseline_name = args.baseline.clone();
    if !args.no_cache
        && let Some(output) = try_diff_from_snapshot(&args, &baseline_name, reporter)
    {
        return Ok(lean_dup_report::diff_report(output));
    }
    let output = run_diff(
        audit_request(default_audit_args(
            pick_workspace(args.workspace, args.workspace_positional),
            args.module_root,
        )),
        baseline_name,
        reporter,
    )?;
    Ok(lean_dup_report::diff_report(output))
}

/// Build a `DiffOutput` from on-disk snapshots without running the audit
/// pipeline. Returns `None` (so the caller falls through to the slow path)
/// if any of the inputs is missing, unparseable, or fingerprint-mismatched.
fn try_diff_from_snapshot(args: &DiffArgs, baseline_name: &str, reporter: &mut Reporter) -> Option<DiffOutput> {
    let resolved = resolve(
        WorkspaceRequest {
            requested_root: workspace_or_cwd(pick_workspace(
                args.workspace.clone(),
                args.workspace_positional.clone(),
            )),
            module_root: args.module_root.clone(),
        },
        reporter,
    )
    .ok()?;
    let cache = lean_dup_index::resolve_cache(&resolved).ok()?;
    let current = load_last_audit_snapshot(&cache.root, &cache.fingerprint)?;
    if current.workspace_fingerprint != cache.fingerprint {
        return None;
    }
    let (baseline_path, baseline) = load_named_baseline(&cache.root, baseline_name).ok()?;
    let diff = diff_snapshots(baseline_name.to_owned(), baseline_path, baseline, current);
    Some(DiffOutput {
        requested_workspace: resolved.requested_root,
        lake_root: resolved.root,
        selected_roots: resolved.selected_roots,
        source_count: resolved.source_files.len(),
        cache_root: cache.root,
        cache_fingerprint: cache.fingerprint,
        diff,
    })
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

fn default_audit_args(workspace: Option<PathBuf>, module_root: Option<String>) -> AuditArgs {
    AuditArgs {
        workspace,
        workspace_positional: None,
        module_root,
        format: OutputFormat::Text,
        visibility: crate::cli::Visibility::All,
        compare_indexes: Vec::new(),
        compare_mathlib: false,
        mathlib_workspace: None,
        include_generated: false,
        show_private: false,
        low_priority: false,
        diagnostics: false,
        save_baseline: None,
        semantic_probes: true,
        verbose: false,
        limit: None,
        max_heartbeats: None,
        probe_budget: 500,
        probe_policy: crate::cli::CliProbePolicy::Actionable,
        probe_chunk_size: 16,
    }
}

/// Resolve a user-supplied `--workspace` argument, defaulting to the current
/// working directory. Used by every subcommand so the flag is optional.
fn workspace_or_cwd(workspace: Option<PathBuf>) -> PathBuf {
    workspace.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Combine a `--workspace` flag with the optional positional form. Clap rejects
/// "both given" via `conflicts_with`, so at most one is `Some`.
fn pick_workspace(flag: Option<PathBuf>, positional: Option<PathBuf>) -> Option<PathBuf> {
    flag.or(positional)
}

fn missing_oleans(workspace: &ResolvedWorkspace) -> Vec<String> {
    workspace
        .missing_olean_sources(&workspace.root, &workspace.source_files)
        .into_iter()
        .map(|source| source.module.clone())
        .collect()
}

fn baseline_common(action: &BaselineAction) -> &BaselineCommonArgs {
    match action {
        BaselineAction::List(common) => common,
        BaselineAction::Show { common, .. } => common,
        BaselineAction::Delete { common, .. } => common,
    }
}

fn baseline(args: BaselineArgs, reporter: &mut Reporter) -> Result<BaselineReport> {
    match args.action {
        BaselineAction::List(common) => {
            let cache_root = common.cache_root.clone().unwrap_or_else(lean_dup_index::cache_root);
            baseline_list(cache_root, common, reporter)
        }
        BaselineAction::Show { name, common } => {
            let cache_root = common.cache_root.unwrap_or_else(lean_dup_index::cache_root);
            baseline_show(cache_root, name)
        }
        BaselineAction::Delete { name, common } => {
            let cache_root = common.cache_root.unwrap_or_else(lean_dup_index::cache_root);
            baseline_delete(cache_root, name)
        }
    }
}

/// Resolve the current workspace's cache fingerprint cheaply, for filtering
/// `baseline list` to entries that belong to this workspace. Returns `None`
/// (with a note on stderr) when there is no workspace to resolve, so the
/// caller falls back to listing everything.
fn current_workspace_fingerprint(workspace: Option<PathBuf>, reporter: &mut Reporter) -> Option<String> {
    let resolved = resolve(
        WorkspaceRequest {
            requested_root: workspace_or_cwd(workspace),
            module_root: None,
        },
        reporter,
    )
    .ok()?;
    lean_dup_index::resolve_cache(&resolved)
        .ok()
        .map(|cache| cache.fingerprint)
}

fn baseline_list(cache_root: PathBuf, common: BaselineCommonArgs, reporter: &mut Reporter) -> Result<BaselineReport> {
    let dir = baselines_dir(&cache_root);
    let mut entries: Vec<BaselineSummaryReport> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(&dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Some(summary) = baseline_summary(&cache_root, name, false) {
                entries.push(summary);
            }
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let total_before_filter = entries.len();
    let mut filtered = false;

    if !common.all {
        match current_workspace_fingerprint(common.workspace.clone(), reporter) {
            Some(fp) => {
                entries.retain(|entry| entry.workspace_fingerprint == fp);
                filtered = true;
            }
            None => {
                eprintln!(
                    "note: could not resolve a workspace to filter by; showing all baselines (pass --all to silence)"
                );
            }
        }
    }

    Ok(BaselineReport {
        status: "ok",
        cache_root,
        action: "list",
        baselines: entries,
        deleted: None,
        total_before_filter: filtered.then_some(total_before_filter),
    })
}

fn baseline_show(cache_root: PathBuf, name: String) -> Result<BaselineReport> {
    if !baseline_name_is_valid(&name) {
        return Err(AppError::Cli {
            message: format!("invalid baseline name: {name}"),
        });
    }
    let summary = baseline_summary(&cache_root, &name, true).ok_or_else(|| AppError::Cli {
        message: format!("baseline not found: {name}"),
    })?;
    Ok(BaselineReport {
        status: "ok",
        cache_root,
        action: "show",
        baselines: vec![summary],
        deleted: None,
        total_before_filter: None,
    })
}

fn baseline_delete(cache_root: PathBuf, name: String) -> Result<BaselineReport> {
    let path = baseline_path(&cache_root, &name).map_err(|_| AppError::Cli {
        message: format!("invalid baseline name: {name}"),
    })?;
    if !path.exists() {
        return Err(AppError::Cli {
            message: format!("baseline not found: {name}"),
        });
    }
    std::fs::remove_file(&path).map_err(|source| AppError::Io {
        message: "could not delete baseline",
        path: path.clone(),
        source,
    })?;
    Ok(BaselineReport {
        status: "ok",
        cache_root,
        action: "delete",
        baselines: Vec::new(),
        deleted: Some(name),
        total_before_filter: None,
    })
}

/// Load summary metadata for one baseline. Returns `None` if the name is
/// invalid or the file can't be opened/parsed — callers filter these out
/// silently in list mode and raise a friendly error in show mode.
fn baseline_summary(cache_root: &std::path::Path, name: &str, include_ids: bool) -> Option<BaselineSummaryReport> {
    if !baseline_name_is_valid(name) {
        return None;
    }
    let (path, snapshot) = load_named_baseline(cache_root, name).ok()?;
    let disk_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let (group_ids, unique_group_count) = if include_ids {
        let mut seen = std::collections::BTreeSet::new();
        let mut ordered: Vec<String> = Vec::new();
        for group in &snapshot.groups {
            if seen.insert(group.id.clone()) {
                ordered.push(group.id.clone());
            }
        }
        let unique = ordered.len();
        (ordered, Some(unique))
    } else {
        (Vec::new(), None)
    };
    Some(BaselineSummaryReport {
        name: name.to_owned(),
        path,
        workspace_fingerprint: snapshot.workspace_fingerprint,
        group_count: snapshot.groups.len(),
        unique_group_count,
        disk_bytes,
        group_ids,
    })
}
