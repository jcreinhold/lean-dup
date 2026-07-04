use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use lean_dup_eval::EvalSuite;
use lean_dup_search::{AuditVisibilityOptions, ProbePolicy};

#[derive(Debug, Parser)]
#[command(name = "lean-dup")]
#[command(about = "Find and triage duplicate declarations in Lean 4 Lake workspaces")]
#[command(arg_required_else_help = true)]
#[command(disable_version_flag = true)]
#[command(after_help = ENV_VAR_HELP)]
pub struct Cli {
    #[arg(long, global = true, help = "Force progress on (default: on when stderr is a TTY)")]
    pub progress: bool,

    #[arg(
        long = "no-progress",
        global = true,
        help = "Suppress phase-by-phase progress events on stderr"
    )]
    pub no_progress: bool,

    #[arg(
        long,
        global = true,
        help = "Print per-phase timings to stderr after the command finishes"
    )]
    pub profile: bool,

    #[arg(
        long,
        help = "List built-in subcommands and installed external `lean-dup-*` extensions"
    )]
    pub list: bool,

    #[arg(long, help = "Print release identity, schema versions, and build info")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

const ENV_VAR_HELP: &str = "\
Environment variables:
  LEAN_DUP_CACHE_DIR                    Override the on-disk cache root (default: platform user cache dir)
  LEAN_DUP_DISABLE_WORKER_BUILD_CACHE   Set to any value to disable the Lean worker capability build cache
  LEAN_DUP_GIT_REVISION                 Build-time only; embeds the git revision shown by --version
";

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Diagnose workspace, cache, and worker health.
    Doctor(DoctorArgs),
    /// Remove cache entries no workspace points to anymore.
    #[command(name = "cache-cleanup")]
    CacheCleanup(CacheCleanupArgs),
    /// Build or refresh a labelled index for a workspace.
    Index(IndexArgs),
    /// Build or refresh the project's mathlib index.
    #[command(name = "index-mathlib")]
    IndexMathlib(IndexMathlibArgs),
    /// Find duplicate declarations across the selected workspace.
    Audit(AuditArgs),
    /// Run the recall/precision evaluation suites.
    Eval(EvalArgs),
    /// Print the full evidence for one duplicate group.
    Show(ShowArgs),
    /// Compare current findings against a saved baseline.
    Diff(DiffArgs),
    /// List, inspect, or delete saved baselines (see `audit --save-baseline`).
    Baseline(BaselineArgs),
    /// Build the per-toolchain Lean worker on this machine (run once per toolchain you audit).
    #[command(name = "install-worker")]
    InstallWorker(InstallWorkerArgs),
    #[command(hide = true)]
    Perf(PerfArgs),
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

pub(crate) const VISIBLE_BUILT_IN_COMMANDS: &[&str] = &[
    "doctor",
    "cache-cleanup",
    "index",
    "index-mathlib",
    "audit",
    "eval",
    "show",
    "diff",
    "baseline",
    "install-worker",
];

pub(crate) const ALL_BUILT_IN_COMMANDS: &[&str] = &[
    "doctor",
    "cache-cleanup",
    "index",
    "index-mathlib",
    "audit",
    "eval",
    "show",
    "diff",
    "baseline",
    "install-worker",
    "perf",
];

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup doctor                       Check the current workspace
  lean-dup doctor --verbose             Include every cache entry
  lean-dup doctor --format json         Machine-readable diagnostics
")]
pub struct DoctorArgs {
    /// Workspace root to inspect. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Positional form of `--workspace`.
    #[arg(value_name = "WORKSPACE", conflicts_with = "workspace")]
    pub workspace_positional: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`). Defaults to the lakefile's first root.
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-triaged; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Warn if Lean's compiled `.olean` outputs are missing; some semantic checks need them.
    #[arg(long)]
    pub require_oleans: bool,

    /// Print every cache entry (sha256, status, schema) in addition to the summary.
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup cache-cleanup                Dry run: list what would be removed
  lean-dup cache-cleanup --execute      Actually delete the entries
")]
pub struct CacheCleanupArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Override the cache root to inspect. Defaults to the workspace's resolved cache.
    #[arg(long)]
    pub cache_root: Option<PathBuf>,

    /// Workspace whose live cache pointer and last-audit snapshots should be protected.
    /// Without this flag, every per-workspace snapshot file in the cache is treated as stale.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Positional form of `--workspace`.
    #[arg(value_name = "WORKSPACE", conflicts_with = "workspace")]
    pub workspace_positional: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Actually delete the entries. Without this flag the command is a dry run.
    #[arg(long)]
    pub execute: bool,

    /// Include per-entry detail (sha256, reasons) in the text output.
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup index --module MyLib --label local
                                        Index the workspace's `MyLib` root under label `local`
  lean-dup index --module MyLib --label local --force
                                        Rebuild even if the cache is hot
")]
pub struct IndexArgs {
    /// Workspace root to index. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Positional form of `--workspace`.
    #[arg(value_name = "WORKSPACE", conflicts_with = "workspace")]
    pub workspace_positional: Option<PathBuf>,

    /// Lean module root to index (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: String,

    /// Cache label to store this index under. Reused across runs with matching inputs.
    #[arg(long)]
    pub label: String,

    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Rebuild the index even if a cached entry would have been reused.
    #[arg(long)]
    pub force: bool,

    /// Fail if Lean's compiled `.olean` outputs are missing for the selected module root.
    #[arg(long)]
    pub require_oleans: bool,

    /// Per-declaration Lean elaboration heartbeat budget (worker default 200000; 0 = unlimited).
    /// Declarations whose elaboration exceeds it are skipped, with the count reported.
    #[arg(long = "max-heartbeats")]
    pub max_heartbeats: Option<u64>,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup index-mathlib                Index the project's resolved mathlib
  lean-dup index-mathlib --force        Rebuild the mathlib index from scratch
")]
pub struct IndexMathlibArgs {
    /// Project workspace whose mathlib dependency should be indexed. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Override the resolved mathlib workspace root (rare; for non-standard layouts).
    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Rebuild the mathlib index even if a cached entry would have been reused.
    #[arg(long)]
    pub force: bool,

    /// Per-declaration Lean elaboration heartbeat budget (worker default 200000; 0 = unlimited).
    /// Declarations whose elaboration exceeds it are skipped, with the count reported.
    #[arg(long = "max-heartbeats")]
    pub max_heartbeats: Option<u64>,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup audit                        Audit the current workspace (top groups, terse)
  lean-dup audit --verbose              Include provenance and per-group detail
  lean-dup audit --visibility public    Skip private declarations in the corpus
  lean-dup audit --save-baseline v1     Save these findings as baseline `v1`
                                        (replay later with `lean-dup diff --baseline v1`)
")]
pub struct AuditArgs {
    /// Workspace root to audit. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Positional form of `--workspace`.
    #[arg(value_name = "WORKSPACE", conflicts_with = "workspace")]
    pub workspace_positional: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`). Defaults to the lakefile's first root.
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-triaged; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Which corpus to audit: `all` (default), `public`, or `private`.
    #[arg(long, value_enum, default_value_t = Visibility::default())]
    pub visibility: Visibility,

    /// Add an external named index to the comparison set. May be repeated.
    #[arg(long = "compare-index")]
    pub compare_indexes: Vec<String>,

    /// Also compare against the project's mathlib index.
    #[arg(long)]
    pub compare_mathlib: bool,

    /// Override the resolved mathlib workspace root (rare; for non-standard layouts).
    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    /// Include generated declarations (synthesized by macros, deriving, etc.).
    #[arg(long)]
    pub include_generated: bool,

    /// Surface actionable findings about private helpers (otherwise suppressed
    /// because users typically cannot act on someone else's private decl).
    /// Independent of `--visibility`, which controls the audit *corpus*.
    #[arg(
        long = "show-private-actionable",
        help = "Surface actionable findings about private helpers"
    )]
    pub show_private: bool,

    #[arg(long = "low-priority", help = "Include lower-priority structural findings")]
    pub low_priority: bool,

    #[arg(long, help = "Show broad diagnostic findings")]
    pub diagnostics: bool,

    /// Save the post-audit baseline under this name for later `lean-dup diff`.
    #[arg(long = "save-baseline")]
    pub save_baseline: Option<String>,

    /// Skip semantic probes (faster, less precise; useful for quick triage).
    #[arg(long = "no-semantic-probes", action = clap::ArgAction::SetFalse)]
    pub semantic_probes: bool,

    /// Print full provenance, semantic-probe stats, and per-group detail in addition to the summary.
    #[arg(long)]
    pub verbose: bool,

    /// Show at most N groups in the text table (default: 20). Has no effect on JSON output.
    #[arg(long)]
    pub limit: Option<usize>,

    /// Per-declaration Lean elaboration heartbeat budget (worker default 200000; 0 = unlimited).
    /// Declarations whose elaboration exceeds it are skipped, with the count reported.
    #[arg(long = "max-heartbeats")]
    pub max_heartbeats: Option<u64>,

    #[arg(long = "probe-budget", hide = true, default_value_t = 500)]
    pub probe_budget: usize,

    #[arg(long = "probe-policy", hide = true, value_enum, default_value_t = CliProbePolicy::Actionable)]
    pub probe_policy: CliProbePolicy,

    #[arg(long = "probe-chunk-size", hide = true, default_value_t = 16)]
    pub probe_chunk_size: usize,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup eval                         Run the default suite, print a TSV table
  lean-dup eval --suite hard-negatives --format json
                                        Hard-negatives suite, machine-readable
  lean-dup eval --k 1,5                 Override the recall-at-k cutoffs
")]
pub struct EvalArgs {
    /// Evaluation suite to run.
    #[arg(long, value_enum, default_value_t = CliEvalSuite::Default)]
    pub suite: CliEvalSuite,

    /// Output format. `table` is the human-readable TSV; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = EvalFormat::Table)]
    pub format: EvalFormat,

    /// Workspace root for workspace-backed suites. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Override the resolved mathlib workspace root (rare; for non-standard layouts).
    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    /// Lean module root for the manual suites, when running with `--workspace`.
    /// Defaults to `Workspace`; operators with a private corpus pass their actual root.
    #[arg(long)]
    pub manual_module: Option<String>,

    /// Recall-at-k cutoffs to report, comma-separated. Default reports k=1, 5, and 10.
    #[arg(
        long = "k-values",
        visible_alias = "k",
        value_delimiter = ',',
        default_value = "1,5,10"
    )]
    pub k_values: Vec<usize>,

    /// Write the rendered report to this path in addition to stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,

    #[arg(long, hide = true)]
    pub write_search_dataset: bool,

    #[arg(long, hide = true)]
    pub write_scorer_ablations: bool,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup install-worker               Build the worker for the current project's toolchain
  lean-dup install-worker --toolchain v4.32.0-rc1
                                        Build the worker for a specific toolchain
  lean-dup install-worker --force       Rebuild even if a current worker is installed
")]
pub struct InstallWorkerArgs {
    /// Toolchain to build for (e.g. `v4.32.0-rc1` or `leanprover/lean4:v4.32.0-rc1`).
    /// Defaults to the current directory's `lean-toolchain`, or lean-dup's dev pin.
    #[arg(long)]
    pub toolchain: Option<String>,

    /// Rebuild even if a current, smoke-passing worker is already installed.
    #[arg(long)]
    pub force: bool,

    /// Build the worker-child from this checkout instead of the published crate.
    /// Defaults to lean-dup's own checkout when run from one, else crates.io.
    #[arg(long = "source-dir")]
    pub source_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct PerfArgs {
    #[arg(long, value_enum)]
    pub workload: PerfWorkload,

    #[arg(long, value_enum, default_value_t = PerfFormat::Json)]
    pub format: PerfFormat,

    #[arg(long)]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub cache_root: Option<PathBuf>,

    #[arg(long)]
    pub manual_workspace: Option<PathBuf>,

    /// Lean module root for the manual workloads. Defaults to `Workspace` (or
    /// `Workspace.Targeted` for targeted variants); operators override.
    #[arg(long)]
    pub manual_module: Option<String>,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup show --group exact-statement-0f780280dc04
                                        Print evidence for one group (ID from `audit`)
  lean-dup show --group <id> --format json
                                        Same, as JSON
")]
pub struct ShowArgs {
    /// Workspace root to inspect. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Positional form of `--workspace`.
    #[arg(value_name = "WORKSPACE", conflicts_with = "workspace")]
    pub workspace_positional: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Group or pair ID. Obtain one from the `lean-dup audit` output table.
    #[arg(long)]
    pub group: String,

    /// Skip the last-audit snapshot fast-fail. Always run the full pipeline.
    #[arg(long = "no-cache")]
    pub no_cache: bool,

    /// Include workspace and cache provenance lines in the text output.
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup diff --baseline v1           Compare current findings to baseline `v1`
                                        (save one first with `audit --save-baseline v1`)
")]
pub struct DiffArgs {
    /// Workspace root to diff against the baseline. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Positional form of `--workspace`.
    #[arg(value_name = "WORKSPACE", conflicts_with = "workspace")]
    pub workspace_positional: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Name of a previously-saved baseline (see `lean-dup audit --save-baseline`).
    #[arg(long)]
    pub baseline: String,

    /// Include workspace and cache provenance lines in the text output.
    #[arg(long)]
    pub verbose: bool,

    /// Skip the last-audit snapshot fast-path. Always re-run the full audit.
    #[arg(long = "no-cache")]
    pub no_cache: bool,
}

#[derive(Debug, Clone, clap::Args)]
#[command(after_help = "\
Examples:
  lean-dup baseline list                List baselines for the current workspace
  lean-dup baseline list --all          List every baseline under the cache root
  lean-dup baseline show v1             Inspect baseline `v1`
  lean-dup baseline show v1 --format json
                                        Same, as JSON
  lean-dup baseline delete v1           Remove baseline `v1`
")]
pub struct BaselineArgs {
    #[command(subcommand)]
    pub action: BaselineAction,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BaselineCommonArgs {
    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Override the cache root. Defaults to the platform user cache dir (or `$LEAN_DUP_CACHE_DIR`).
    #[arg(long)]
    pub cache_root: Option<PathBuf>,

    /// Include the full group ID list (otherwise the first ~20 are shown).
    #[arg(long)]
    pub verbose: bool,

    /// Filter `list` to baselines saved for this workspace's cache fingerprint.
    /// Defaults to the current directory; pass `--all` to list every saved
    /// baseline regardless of workspace.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// List every saved baseline under the cache root (overrides the cwd filter).
    #[arg(long, conflicts_with = "workspace")]
    pub all: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BaselineAction {
    /// List saved baselines.
    List(BaselineCommonArgs),
    /// Print the contents of one baseline.
    Show {
        /// Baseline name (as passed to `audit --save-baseline`).
        name: String,
        #[command(flatten)]
        common: BaselineCommonArgs,
    },
    /// Remove a saved baseline.
    Delete {
        /// Baseline name.
        name: String,
        #[command(flatten)]
        common: BaselineCommonArgs,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvalFormat {
    Table,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CliEvalSuite {
    Default,
    HardNegatives,
    ManualInternal,
    ManualMathlib,
    ProductionGate,
}

impl From<CliEvalSuite> for EvalSuite {
    fn from(value: CliEvalSuite) -> Self {
        match value {
            CliEvalSuite::Default => Self::Default,
            CliEvalSuite::HardNegatives => Self::HardNegatives,
            CliEvalSuite::ManualInternal => Self::ManualInternal,
            CliEvalSuite::ManualMathlib => Self::ManualMathlib,
            CliEvalSuite::ProductionGate => Self::ProductionGate,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    /// Include both public and private declarations (default).
    #[default]
    All,
    /// Public declarations only.
    Public,
    /// Private declarations only (rare; for `private`-helper audits).
    Private,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CliProbePolicy {
    Actionable,
    Broad,
}

impl From<CliProbePolicy> for ProbePolicy {
    fn from(value: CliProbePolicy) -> Self {
        match value {
            CliProbePolicy::Actionable => Self::Actionable,
            CliProbePolicy::Broad => Self::Broad,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PerfFormat {
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PerfWorkload {
    ColdMathlibIndex,
    WarmMathlibIndex,
    ManualTargetedMathlib,
    ManualFullNoMathlib,
    ManualFullMathlibNoProbes,
    ManualFullMathlib,
    FixtureAudit,
}

impl PerfWorkload {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ColdMathlibIndex => "cold-mathlib-index",
            Self::WarmMathlibIndex => "warm-mathlib-index",
            Self::ManualTargetedMathlib => "manual-targeted-mathlib",
            Self::ManualFullNoMathlib => "manual-full-no-mathlib",
            Self::ManualFullMathlibNoProbes => "manual-full-mathlib-no-probes",
            Self::ManualFullMathlib => "manual-full-mathlib",
            Self::FixtureAudit => "fixture-audit",
        }
    }
}

impl AuditArgs {
    /// Whether the audit *corpus* includes private declarations.
    /// Controlled by `--visibility`; default is `all`.
    pub fn effective_include_private(&self) -> bool {
        matches!(self.visibility, Visibility::All | Visibility::Private)
    }

    pub fn visibility_options(&self) -> AuditVisibilityOptions {
        AuditVisibilityOptions {
            include_private: self.show_private,
            include_low_priority: self.low_priority,
            diagnostics: self.diagnostics,
        }
    }
}
