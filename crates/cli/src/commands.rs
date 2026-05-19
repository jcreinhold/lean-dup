use std::path::PathBuf;
use std::time::Duration;

use crate::cli::{
    AuditArgs, CacheCleanupArgs, Cli, Command, DiffArgs, DoctorArgs, EmbeddingCommand, EmbeddingPrepareArgs, EvalArgs,
    EvalFormat, IndexArgs, IndexMathlibArgs, OutputFormat, ShowArgs,
};
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_embedding::{
    EmbeddingAcquisitionPolicy, EmbeddingCacheStatus, EmbeddingModelFileRole, EmbeddingModelFileState,
    EmbeddingModelSpec, EmbeddingPrepareRequest, EmbeddingPrepareResult, prepare_embedding_model,
};
use lean_dup_eval::{EmbeddingRerankRequest, EvalRequest};
use lean_dup_index::CleanupPolicy;
use lean_dup_index::{self, CacheFacts};
use lean_dup_index::{IndexBuildKind, IndexBuildRequest, IndexStore, IndexSummary};
use lean_dup_project::{ResolvedWorkspace, WorkspaceRequest, resolve, resolve_project_mathlib};
use lean_dup_report::{
    AuditReport, CacheCleanupReportDto, DiffReport, DoctorReport, EmbeddingPrepareReportDto,
    EmbeddingRequiredFileReportDto, IndexReport, Report, ShowReport,
};
use lean_dup_search::{AuditRequest, run_audit, run_diff, run_show};
use lean_dup_worker::WorkerClient;

use crate::error::{AppError, Result};

#[derive(Debug)]
pub struct Outcome {
    pub report: Report,
    pub output_format: OutputFormat,
    pub output_path: Option<PathBuf>,
    pub reporter: Reporter,
}

struct Foundation {
    workspace: ResolvedWorkspace,
    cache: CacheFacts,
}

pub fn run(cli: Cli) -> Result<Outcome> {
    let mut reporter = Reporter::new_live(cli.progress, cli.profile);
    let (report, output_format, output_path) = match cli.command {
        Command::Doctor(args) => {
            let format = args.format;
            (Report::Doctor(doctor(args, &mut reporter)?), format, None)
        }
        Command::CacheCleanup(args) => {
            let format = args.format;
            (Report::CacheCleanup(cache_cleanup(args, &mut reporter)?), format, None)
        }
        Command::Index(args) => (Report::Index(index(args, &mut reporter)?), OutputFormat::Text, None),
        Command::IndexMathlib(args) => (
            Report::IndexMathlib(index_mathlib(args, &mut reporter)?),
            OutputFormat::Text,
            None,
        ),
        Command::Audit(args) => {
            let format = args.format;
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
        Command::Embedding(args) => match args.command {
            EmbeddingCommand::Prepare(prepare_args) => {
                let format = prepare_args.format;
                (
                    Report::EmbeddingPrepare(embedding_prepare(prepare_args, &mut reporter)?),
                    format,
                    None,
                )
            }
        },
        Command::Show(args) => (
            Report::Show(Box::new(show(args, &mut reporter)?)),
            OutputFormat::Text,
            None,
        ),
        Command::Diff(args) => (Report::Diff(diff(args, &mut reporter)?), OutputFormat::Text, None),
    };

    Ok(Outcome {
        report,
        output_format,
        output_path,
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
        },
        &worker_version,
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
        cache: cache_diagnostics,
        lean_version: worker_version
            .lean_version
            .unwrap_or_else(|| "unknown Lean version".to_owned()),
        require_oleans: args.require_oleans,
        missing_oleans,
    })
}

fn cache_cleanup(args: CacheCleanupArgs, reporter: &mut Reporter) -> Result<CacheCleanupReportDto> {
    let cache_root = args.cache_root.unwrap_or_else(lean_dup_index::cache_root);
    let store = IndexStore::new(cache_root.clone());
    let expected_entries = if let Some(workspace_root) = args.workspace {
        let workspace = resolve(
            WorkspaceRequest {
                requested_root: workspace_root,
                module_root: args.module_root,
            },
            reporter,
        )?;
        let version_call = WorkerClient::with_timeout(Duration::from_secs(60)).version(workspace.root.clone())?;
        let worker_version = version_call.rows.into_iter().next().ok_or_else(|| AppError::Cli {
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
            },
            &worker_version,
        )?]
    } else {
        Vec::new()
    };
    Ok(lean_dup_report::cache_cleanup_report(lean_dup_index::cleanup_cache(
        cache_root,
        &expected_entries,
        CleanupPolicy { execute: args.execute },
    )?))
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
            &WorkerClient::for_indexing(),
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
    AuditRequest {
        workspace: args.workspace,
        module_root: args.module_root,
        include_private,
        compare_indexes: args.compare_indexes,
        compare_mathlib: args.compare_mathlib,
        mathlib_workspace: args.mathlib_workspace,
        include_generated: args.include_generated,
        show_noise: args.show_noise,
        review_profile: args.review_profile.into(),
        save_baseline: args.save_baseline,
        semantic_probes: args.semantic_probes,
        probe_budget: args.probe_budget,
        probe_policy: args.probe_policy.into(),
        probe_chunk_size: args.probe_chunk_size,
    }
}

fn eval(args: EvalArgs, reporter: &mut Reporter) -> Result<lean_dup_report::EvalReportDto> {
    let embedding_rerank = embedding_rerank_request(&args);
    Ok(lean_dup_report::eval_report(lean_dup_eval::run(
        EvalRequest {
            suite: args.suite.into(),
            workspace: args.workspace,
            mathlib_workspace: args.mathlib_workspace,
            manual_module: args.manual_module,
            k_values: args.k_values,
            write_search_dataset: args.write_search_dataset,
            write_scorer_ablations: args.write_scorer_ablations,
            embedding_rerank,
        },
        reporter,
    )?))
}

fn embedding_rerank_request(args: &EvalArgs) -> Option<EmbeddingRerankRequest> {
    args.write_embedding_rerank.then(|| EmbeddingRerankRequest {
        model: EmbeddingModelSpec {
            id: args.embedding_model_id.clone(),
            revision: args.embedding_revision.clone(),
        },
        acquisition_policy: args.embedding_acquisition.into(),
        model_cache_root: args.embedding_cache_root.clone(),
        vector_cache_root: args.embedding_vector_cache_root.clone(),
    })
}

fn embedding_prepare(args: EmbeddingPrepareArgs, reporter: &mut Reporter) -> Result<EmbeddingPrepareReportDto> {
    let model = EmbeddingModelSpec {
        id: args.model_id,
        revision: args.revision,
    };
    let acquisition_policy = args.policy.into();
    let result = reporter.measure("embedding.prepare", |_| {
        prepare_embedding_model(EmbeddingPrepareRequest {
            model,
            acquisition_policy,
            cache_root: args.cache_root,
        })
    })?;
    Ok(embedding_prepare_report(result))
}

fn embedding_prepare_report(result: EmbeddingPrepareResult) -> EmbeddingPrepareReportDto {
    EmbeddingPrepareReportDto {
        status: if result.cache.status == EmbeddingCacheStatus::Prepared {
            "ok".to_owned()
        } else {
            "warning".to_owned()
        },
        model_id: result.model.id,
        revision: result.model.revision,
        profile_id: result.model.profile_id,
        backend_family: result.model.backend_family,
        dimension: result.model.dimension,
        input_roles: result.model.input_roles,
        acquisition_policy: acquisition_policy_name(result.acquisition_policy).to_owned(),
        cache_status: cache_status_name(result.cache.status).to_owned(),
        cache_root: result.cache.cache_label.map(PathBuf::from),
        elapsed_ms: result.elapsed_ms,
        total_bytes: result.total_bytes,
        required_files: result
            .required_files
            .into_iter()
            .map(|file| EmbeddingRequiredFileReportDto {
                role: file_role_name(file.role).to_owned(),
                state: file_state_name(file.state).to_owned(),
                bytes: file.bytes,
                reason: file.reason,
            })
            .collect(),
        reasons: result.reasons,
    }
}

fn acquisition_policy_name(policy: EmbeddingAcquisitionPolicy) -> &'static str {
    match policy {
        EmbeddingAcquisitionPolicy::CacheOnly => "cache-only",
        EmbeddingAcquisitionPolicy::DownloadIfMissing => "download-if-missing",
    }
}

fn cache_status_name(status: EmbeddingCacheStatus) -> &'static str {
    match status {
        EmbeddingCacheStatus::NotPrepared => "not-prepared",
        EmbeddingCacheStatus::Prepared => "prepared",
        EmbeddingCacheStatus::Unusable => "unusable",
        EmbeddingCacheStatus::Skipped => "skipped",
    }
}

fn file_role_name(role: EmbeddingModelFileRole) -> &'static str {
    match role {
        EmbeddingModelFileRole::Config => "config",
        EmbeddingModelFileRole::Tokenizer => "tokenizer",
        EmbeddingModelFileRole::TokenizerConfig => "tokenizer-config",
        EmbeddingModelFileRole::SpecialTokens => "special-tokens",
        EmbeddingModelFileRole::RuntimeModel => "runtime-model",
    }
}

fn file_state_name(state: EmbeddingModelFileState) -> &'static str {
    match state {
        EmbeddingModelFileState::Present => "present",
        EmbeddingModelFileState::Downloaded => "downloaded",
        EmbeddingModelFileState::Missing => "missing",
        EmbeddingModelFileState::Unavailable => "unavailable",
    }
}

fn show(args: ShowArgs, reporter: &mut Reporter) -> Result<ShowReport> {
    let requested_group = args.group.clone();
    let output = run_show(
        audit_request(default_audit_args(args.workspace, args.module_root)),
        &requested_group,
        reporter,
    )?;
    Ok(lean_dup_report::show_report(output))
}

fn diff(args: DiffArgs, reporter: &mut Reporter) -> Result<DiffReport> {
    let baseline_name = args.baseline.clone();
    let output = run_diff(
        audit_request(default_audit_args(args.workspace, args.module_root)),
        baseline_name,
        reporter,
    )?;
    Ok(lean_dup_report::diff_report(output))
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

fn default_audit_args(workspace: PathBuf, module_root: Option<String>) -> AuditArgs {
    AuditArgs {
        workspace,
        module_root,
        format: OutputFormat::Text,
        public_only: false,
        include_private: true,
        no_include_private: true,
        compare_indexes: Vec::new(),
        compare_mathlib: false,
        mathlib_workspace: None,
        include_generated: false,
        show_noise: false,
        review_profile: crate::cli::CliReviewProfile::Mathlib,
        save_baseline: None,
        semantic_probes: true,
        probe_budget: 500,
        probe_policy: crate::cli::CliProbePolicy::Actionable,
        probe_chunk_size: 16,
    }
}

fn missing_oleans(workspace: &ResolvedWorkspace) -> Vec<String> {
    workspace
        .missing_olean_sources(&workspace.root, &workspace.source_files)
        .into_iter()
        .map(|source| source.module.clone())
        .collect()
}
