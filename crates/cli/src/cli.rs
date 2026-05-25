use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use lean_dup_eval::EvalSuite;
use lean_dup_search::{AuditVisibilityOptions, ProbePolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(name = "lean-dup")]
#[command(about = "Find and triage duplicate declarations in Lean 4 Lake workspaces")]
#[command(arg_required_else_help = true)]
#[command(disable_version_flag = true)]
#[command(after_help = ENV_VAR_HELP)]
pub struct Cli {
    #[arg(long, global = true, help = "Stream phase-by-phase progress events to stderr (useful for long audits)")]
    pub progress: bool,

    #[arg(long, global = true, help = "Print per-phase timings to stderr after the command finishes")]
    pub profile: bool,

    #[arg(long, help = "List built-in subcommands and installed external `lean-dup-*` extensions")]
    pub list: bool,

    #[arg(long, help = "Print release identity, schema versions, and build info")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

const ENV_VAR_HELP: &str = "\
Environment variables:
  LEAN_DUP_CACHE_DIR                    Override the on-disk cache root (default: platform user cache dir)
  LEAN_DUP_DISABLE_WORKER_BUILD_CACHE   Set to any value to disable the Lean worker subprocess build cache
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
    #[command(hide = true)]
    Perf(PerfArgs),
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

pub(crate) const VISIBLE_BUILT_IN_COMMANDS: &[&str] =
    &["doctor", "cache-cleanup", "index", "index-mathlib", "audit", "eval", "show", "diff"];

pub(crate) const ALL_BUILT_IN_COMMANDS: &[&str] = &[
    "doctor",
    "cache-cleanup",
    "index",
    "index-mathlib",
    "audit",
    "eval",
    "show",
    "diff",
    "perf",
];

#[derive(Debug, Clone, clap::Args)]
pub struct DoctorArgs {
    /// Workspace root to inspect. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

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
pub struct CacheCleanupArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Override the cache root to inspect. Defaults to the workspace's resolved cache.
    #[arg(long)]
    pub cache_root: Option<PathBuf>,

    /// Workspace whose live cache pointer should be protected. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Actually delete the entries. Without this flag the command is a dry run.
    #[arg(long)]
    pub execute: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct IndexArgs {
    /// Workspace root to index. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

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
}

#[derive(Debug, Clone, clap::Args)]
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
}

#[derive(Debug, Clone, clap::Args)]
pub struct AuditArgs {
    /// Workspace root to audit. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`). Defaults to the lakefile's first root.
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-triaged; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Exclude private declarations from the audit corpus (equivalent to --no-include-private).
    #[arg(long)]
    pub public_only: bool,

    /// Include private declarations in the audit corpus (default).
    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue)]
    pub include_private: bool,

    /// Exclude private declarations from the audit corpus.
    #[arg(long = "no-include-private", action = clap::ArgAction::SetFalse)]
    pub no_include_private: bool,

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

    #[arg(long = "private", help = "Show otherwise-actionable private helper findings")]
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

    #[arg(long = "probe-budget", hide = true, default_value_t = 500)]
    pub probe_budget: usize,

    #[arg(long = "probe-policy", hide = true, value_enum, default_value_t = CliProbePolicy::Actionable)]
    pub probe_policy: CliProbePolicy,

    #[arg(long = "probe-chunk-size", hide = true, default_value_t = 16)]
    pub probe_chunk_size: usize,
}

#[derive(Debug, Clone, clap::Args)]
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
    #[arg(long = "k-values", visible_alias = "k", value_delimiter = ',', default_value = "1,5,10")]
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
pub struct ShowArgs {
    /// Workspace root to inspect. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Group or pair ID. Obtain one from the `lean-dup audit` output table.
    #[arg(long)]
    pub group: String,
}

#[derive(Debug, Clone, clap::Args)]
pub struct DiffArgs {
    /// Workspace root to diff against the baseline. Defaults to the current directory.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Lean module root inside the workspace (e.g. `Mathlib`).
    #[arg(long = "module")]
    pub module_root: Option<String>,

    /// Output format. `text` is human-readable; `json` is the stable wire schema.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Name of a previously-saved baseline (see `lean-dup audit --save-baseline`).
    #[arg(long)]
    pub baseline: String,
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
    pub fn effective_include_private(&self) -> bool {
        self.include_private && self.no_include_private && !self.public_only
    }

    pub fn visibility_options(&self) -> AuditVisibilityOptions {
        AuditVisibilityOptions {
            include_private: self.show_private,
            include_low_priority: self.low_priority,
            diagnostics: self.diagnostics,
        }
    }
}
