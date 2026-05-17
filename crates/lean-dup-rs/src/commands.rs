use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::cache::{self, CacheFacts};
use crate::cli::{
    AuditArgs, Cli, Command, DiffArgs, DoctorArgs, IndexArgs, IndexMathlibArgs, OutputFormat,
    ShowArgs,
};
use crate::error::Result;
use crate::progress::Reporter;
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
    Index(SkeletonReport),
    IndexMathlib(SkeletonReport),
    Audit(AuditReport),
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
    pub(crate) message: &'static str,
}

struct Foundation {
    workspace: ResolvedWorkspace,
    cache: CacheFacts,
}

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

fn index(args: IndexArgs, reporter: &mut Reporter) -> Result<SkeletonReport> {
    let foundation = foundation(args.workspace, Some(args.module_root), reporter)?;
    let missing = if args.require_oleans {
        missing_oleans(&foundation.workspace)
    } else {
        Vec::new()
    };
    let message = if missing.is_empty() {
        "index planning complete; persistence is implemented in prompt 10"
    } else {
        "index planning complete with missing oleans; persistence is implemented in prompt 10"
    };
    Ok(skeleton(
        foundation,
        message,
        Some(args.label),
        None,
        None,
        args.force,
    ))
}

fn index_mathlib(args: IndexMathlibArgs, reporter: &mut Reporter) -> Result<SkeletonReport> {
    let foundation = foundation(args.workspace, Some("Mathlib".to_owned()), reporter)?;
    Ok(skeleton(
        foundation,
        "mathlib index planning complete; persistence is implemented in prompt 10",
        Some("mathlib".to_owned()),
        None,
        None,
        args.force,
    ))
}

fn audit(args: AuditArgs, reporter: &mut Reporter) -> Result<AuditReport> {
    let include_private = args.effective_include_private();
    let foundation = foundation(args.workspace.clone(), args.module_root.clone(), reporter)?;
    Ok(AuditReport {
        status: "stub",
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
        message: "audit orchestration is stubbed until worker protocol, indexes, retrieval, and ranking are implemented",
    })
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

fn missing_oleans(workspace: &ResolvedWorkspace) -> Vec<String> {
    workspace
        .source_files
        .iter()
        .filter(|source| !workspace::olean_exists(&workspace.root, &source.module))
        .map(|source| source.module.clone())
        .collect()
}
