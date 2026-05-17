use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "lean-dup-rs")]
#[command(about = "Rust foundation CLI for Lean duplicate audits")]
pub(crate) struct Cli {
    #[arg(long, global = true, help = "Render typed progress events on stderr")]
    pub(crate) progress: bool,

    #[arg(long, global = true, help = "Render phase timings on stderr")]
    pub(crate) profile: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Doctor(DoctorArgs),
    Index(IndexArgs),
    #[command(name = "index-mathlib")]
    IndexMathlib(IndexMathlibArgs),
    Audit(AuditArgs),
    Show(ShowArgs),
    Diff(DiffArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct DoctorArgs {
    #[arg(long)]
    pub(crate) workspace: PathBuf,

    #[arg(long = "module")]
    pub(crate) module_root: Option<String>,

    #[arg(long)]
    pub(crate) require_oleans: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct IndexArgs {
    #[arg(long)]
    pub(crate) workspace: PathBuf,

    #[arg(long = "module")]
    pub(crate) module_root: String,

    #[arg(long)]
    pub(crate) label: String,

    #[arg(long)]
    pub(crate) force: bool,

    #[arg(long)]
    pub(crate) require_oleans: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct IndexMathlibArgs {
    #[arg(long, default_value = "/Users/jcreinhold/Code/mathlib4")]
    pub(crate) workspace: PathBuf,

    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct AuditArgs {
    #[arg(long)]
    pub(crate) workspace: PathBuf,

    #[arg(long = "module")]
    pub(crate) module_root: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub(crate) format: OutputFormat,

    #[arg(long)]
    pub(crate) public_only: bool,

    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue)]
    pub(crate) include_private: bool,

    #[arg(long = "no-include-private", action = clap::ArgAction::SetFalse)]
    pub(crate) no_include_private: bool,

    #[arg(long)]
    pub(crate) include_imports: bool,

    #[arg(long = "import-root")]
    pub(crate) import_roots: Vec<String>,

    #[arg(long = "compare-index")]
    pub(crate) compare_indexes: Vec<String>,

    #[arg(long)]
    pub(crate) compare_mathlib: bool,

    #[arg(long)]
    pub(crate) mathlib_workspace: Option<PathBuf>,

    #[arg(long, default_value_t = 0.78)]
    pub(crate) threshold: f64,

    #[arg(long)]
    pub(crate) include_generated: bool,

    #[arg(long)]
    pub(crate) show_noise: bool,

    #[arg(long, value_enum, default_value_t = ReviewPriority::Low)]
    pub(crate) min_priority: ReviewPriority,

    #[arg(long = "no-semantic-probes", action = clap::ArgAction::SetFalse)]
    pub(crate) semantic_probes: bool,

    #[arg(long = "no-replacement-hints", action = clap::ArgAction::SetFalse)]
    pub(crate) replacement_hints: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct ShowArgs {
    #[arg(long)]
    pub(crate) workspace: PathBuf,

    #[arg(long)]
    pub(crate) group: String,
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct DiffArgs {
    #[arg(long)]
    pub(crate) workspace: PathBuf,

    #[arg(long)]
    pub(crate) baseline: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReviewPriority {
    High,
    Medium,
    Low,
    Noise,
}

impl AuditArgs {
    pub(crate) fn effective_include_private(&self) -> bool {
        self.include_private && self.no_include_private && !self.public_only
    }
}
