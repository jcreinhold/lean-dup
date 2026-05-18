use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
pub use lean_dup_eval::EvalSuite;
pub use lean_dup_search::{ProbePolicy, ReviewProfile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(name = "lean-dup")]
#[command(about = "Rust foundation CLI for Lean duplicate audits")]
pub struct Cli {
    #[arg(long, global = true, help = "Render typed progress events on stderr")]
    pub progress: bool,

    #[arg(long, global = true, help = "Render phase timings on stderr")]
    pub profile: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Doctor(DoctorArgs),
    #[command(name = "cache-cleanup", hide = true)]
    CacheCleanup(CacheCleanupArgs),
    Index(IndexArgs),
    #[command(name = "index-mathlib")]
    IndexMathlib(IndexMathlibArgs),
    Audit(AuditArgs),
    Eval(EvalArgs),
    Show(ShowArgs),
    Diff(DiffArgs),
    #[command(hide = true)]
    Perf(PerfArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub workspace: PathBuf,

    #[arg(long = "module")]
    pub module_root: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    #[arg(long)]
    pub require_oleans: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct CacheCleanupArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    #[arg(long)]
    pub cache_root: Option<PathBuf>,

    #[arg(long)]
    pub workspace: Option<PathBuf>,

    #[arg(long = "module")]
    pub module_root: Option<String>,

    #[arg(long)]
    pub execute: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct IndexArgs {
    #[arg(long)]
    pub workspace: PathBuf,

    #[arg(long = "module")]
    pub module_root: String,

    #[arg(long)]
    pub label: String,

    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub require_oleans: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct IndexMathlibArgs {
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct AuditArgs {
    #[arg(long)]
    pub workspace: PathBuf,

    #[arg(long = "module")]
    pub module_root: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    #[arg(long)]
    pub public_only: bool,

    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue)]
    pub include_private: bool,

    #[arg(long = "no-include-private", action = clap::ArgAction::SetFalse)]
    pub no_include_private: bool,

    #[arg(long)]
    pub include_imports: bool,

    #[arg(long = "import-root")]
    pub import_roots: Vec<String>,

    #[arg(long = "compare-index")]
    pub compare_indexes: Vec<String>,

    #[arg(long)]
    pub compare_mathlib: bool,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    #[arg(long, default_value_t = 0.78)]
    pub threshold: f64,

    #[arg(long)]
    pub include_generated: bool,

    #[arg(long)]
    pub show_noise: bool,

    #[arg(long, value_enum, default_value_t = ReviewPriority::Low)]
    pub min_priority: ReviewPriority,

    #[arg(long = "review-profile", value_enum, default_value_t = ReviewProfile::Mathlib)]
    pub review_profile: ReviewProfile,

    #[arg(long = "save-baseline")]
    pub save_baseline: Option<String>,

    #[arg(long = "no-semantic-probes", action = clap::ArgAction::SetFalse)]
    pub semantic_probes: bool,

    #[arg(long = "probe-budget", hide = true, default_value_t = 500)]
    pub probe_budget: usize,

    #[arg(long = "probe-policy", hide = true, value_enum, default_value_t = ProbePolicy::Actionable)]
    pub probe_policy: ProbePolicy,

    #[arg(long = "probe-chunk-size", hide = true, default_value_t = 16)]
    pub probe_chunk_size: usize,

    #[arg(long = "no-replacement-hints", action = clap::ArgAction::SetFalse)]
    pub replacement_hints: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct EvalArgs {
    #[arg(long, value_enum, default_value_t = EvalSuite::Default)]
    pub suite: EvalSuite,

    #[arg(long, value_enum, default_value_t = EvalFormat::Table)]
    pub format: EvalFormat,

    #[arg(long)]
    pub workspace: Option<PathBuf>,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    #[arg(long = "k", value_delimiter = ',', default_value = "1,5,10")]
    pub k_values: Vec<usize>,

    #[arg(long)]
    pub output: Option<PathBuf>,
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
    pub kanproofs_workspace: Option<PathBuf>,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ShowArgs {
    #[arg(long)]
    pub workspace: PathBuf,

    #[arg(long = "module")]
    pub module_root: Option<String>,

    #[arg(long)]
    pub group: String,
}

#[derive(Debug, Clone, clap::Args)]
pub struct DiffArgs {
    #[arg(long)]
    pub workspace: PathBuf,

    #[arg(long = "module")]
    pub module_root: Option<String>,

    #[arg(long)]
    pub baseline: String,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewPriority {
    High,
    Medium,
    Low,
    Noise,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvalFormat {
    Table,
    Json,
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
    KanproofsTargetedMathlib,
    KanproofsFullNoMathlib,
    KanproofsFullMathlibNoProbes,
    KanproofsFullMathlib,
    FixtureAudit,
}

impl AuditArgs {
    pub fn effective_include_private(&self) -> bool {
        self.include_private && self.no_include_private && !self.public_only
    }
}
