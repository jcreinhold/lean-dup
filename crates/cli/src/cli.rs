use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use lean_dup_embedding::EmbeddingAcquisitionPolicy;
use lean_dup_eval::EvalSuite;
use lean_dup_search::{
    ProbePolicy, ReviewProfile, SearchEmbeddingDocumentPolicy, SearchVectorAcquisitionPolicy,
    SearchVectorEligibilityPolicy,
};
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
    Embedding(EmbeddingArgs),
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

    #[arg(long = "compare-index")]
    pub compare_indexes: Vec<String>,

    #[arg(long)]
    pub compare_mathlib: bool,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    #[arg(long)]
    pub include_generated: bool,

    #[arg(long)]
    pub show_noise: bool,

    #[arg(long = "review-profile", value_enum, default_value_t = CliReviewProfile::Mathlib)]
    pub review_profile: CliReviewProfile,

    #[arg(long = "save-baseline")]
    pub save_baseline: Option<String>,

    #[arg(long = "no-semantic-probes", action = clap::ArgAction::SetFalse)]
    pub semantic_probes: bool,

    #[arg(long = "probe-budget", hide = true, default_value_t = 500)]
    pub probe_budget: usize,

    #[arg(long = "probe-policy", hide = true, value_enum, default_value_t = CliProbePolicy::Actionable)]
    pub probe_policy: CliProbePolicy,

    #[arg(long = "probe-chunk-size", hide = true, default_value_t = 16)]
    pub probe_chunk_size: usize,
}

#[derive(Debug, Clone, clap::Args)]
pub struct EvalArgs {
    #[arg(long, value_enum, default_value_t = CliEvalSuite::Default)]
    pub suite: CliEvalSuite,

    #[arg(long, value_enum, default_value_t = EvalFormat::Table)]
    pub format: EvalFormat,

    #[arg(long)]
    pub workspace: Option<PathBuf>,

    #[arg(long)]
    pub mathlib_workspace: Option<PathBuf>,

    /// Lean module root for the manual suites, when running with `--workspace`.
    /// Defaults to `Workspace`; operators with a private corpus pass their actual root.
    #[arg(long)]
    pub manual_module: Option<String>,

    #[arg(long = "k", value_delimiter = ',', default_value = "1,5,10")]
    pub k_values: Vec<usize>,

    #[arg(long)]
    pub output: Option<PathBuf>,

    #[arg(long, hide = true)]
    pub write_search_dataset: bool,

    #[arg(long, hide = true)]
    pub write_scorer_ablations: bool,

    #[arg(long, hide = true)]
    pub write_embedding_rerank: bool,

    #[arg(long = "embedding-acquisition", hide = true, value_enum, default_value_t = CliEmbeddingAcquisitionPolicy::CacheOnly)]
    pub embedding_acquisition: CliEmbeddingAcquisitionPolicy,

    #[arg(long = "embedding-model-id", hide = true, default_value = "BAAI/bge-small-en-v1.5")]
    pub embedding_model_id: String,

    #[arg(long = "embedding-revision", hide = true)]
    pub embedding_revision: Option<String>,

    #[arg(long = "embedding-cache-root", hide = true)]
    pub embedding_cache_root: Option<PathBuf>,

    #[arg(long = "embedding-vector-cache-root", hide = true)]
    pub embedding_vector_cache_root: Option<PathBuf>,

    #[arg(long, hide = true)]
    pub write_vector_search: bool,

    #[arg(long = "vector-acquisition", hide = true, value_enum, default_value_t = CliEmbeddingAcquisitionPolicy::CacheOnly)]
    pub vector_acquisition: CliEmbeddingAcquisitionPolicy,

    #[arg(long = "vector-model-id", hide = true, default_value = "BAAI/bge-small-en-v1.5")]
    pub vector_model_id: String,

    #[arg(long = "vector-revision", hide = true)]
    pub vector_revision: Option<String>,

    #[arg(long = "vector-model-cache-root", hide = true)]
    pub vector_model_cache_root: Option<PathBuf>,

    #[arg(long = "vector-text-cache-root", hide = true)]
    pub vector_text_cache_root: Option<PathBuf>,

    #[arg(long = "vector-corpus-cache-root", hide = true)]
    pub vector_corpus_cache_root: Option<PathBuf>,

    #[arg(long = "vector-document-policy", hide = true, value_enum, default_value_t = CliVectorDocumentPolicy::NameAndFormalStatement)]
    pub vector_document_policy: CliVectorDocumentPolicy,

    #[arg(long = "vector-eligibility", hide = true, value_enum, default_value_t = CliVectorEligibilityPolicy::ActionablePublicStatement)]
    pub vector_eligibility: CliVectorEligibilityPolicy,
}

#[derive(Debug, Clone, clap::Args)]
pub struct EmbeddingArgs {
    #[command(subcommand)]
    pub command: EmbeddingCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum EmbeddingCommand {
    Prepare(EmbeddingPrepareArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct EmbeddingPrepareArgs {
    #[arg(long, value_enum, default_value_t = CliEmbeddingAcquisitionPolicy::DownloadIfMissing)]
    pub policy: CliEmbeddingAcquisitionPolicy,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    #[arg(long = "model-id", default_value = "BAAI/bge-small-en-v1.5")]
    pub model_id: String,

    #[arg(long)]
    pub revision: Option<String>,

    #[arg(long = "cache-root", hide = true)]
    pub cache_root: Option<PathBuf>,
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

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvalFormat {
    Table,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CliEmbeddingAcquisitionPolicy {
    CacheOnly,
    DownloadIfMissing,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliVectorDocumentPolicy {
    FormalStatement,
    NameAndFormalStatement,
    InformalOrFormal,
    LegacyRerankV1,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliVectorEligibilityPolicy {
    ActionablePublicStatement,
    Broad,
}

impl From<CliVectorDocumentPolicy> for SearchEmbeddingDocumentPolicy {
    fn from(value: CliVectorDocumentPolicy) -> Self {
        match value {
            CliVectorDocumentPolicy::FormalStatement => Self::FormalStatement,
            CliVectorDocumentPolicy::NameAndFormalStatement => Self::NameAndFormalStatement,
            CliVectorDocumentPolicy::InformalOrFormal => Self::InformalOrFormal,
            CliVectorDocumentPolicy::LegacyRerankV1 => Self::LegacyRerankV1,
        }
    }
}

impl From<CliVectorEligibilityPolicy> for SearchVectorEligibilityPolicy {
    fn from(value: CliVectorEligibilityPolicy) -> Self {
        match value {
            CliVectorEligibilityPolicy::ActionablePublicStatement => Self::ActionablePublicStatement,
            CliVectorEligibilityPolicy::Broad => Self::Broad,
        }
    }
}

impl From<CliEmbeddingAcquisitionPolicy> for EmbeddingAcquisitionPolicy {
    fn from(value: CliEmbeddingAcquisitionPolicy) -> Self {
        match value {
            CliEmbeddingAcquisitionPolicy::CacheOnly => Self::CacheOnly,
            CliEmbeddingAcquisitionPolicy::DownloadIfMissing => Self::DownloadIfMissing,
        }
    }
}

impl From<CliEmbeddingAcquisitionPolicy> for SearchVectorAcquisitionPolicy {
    fn from(value: CliEmbeddingAcquisitionPolicy) -> Self {
        match value {
            CliEmbeddingAcquisitionPolicy::CacheOnly => Self::CacheOnly,
            CliEmbeddingAcquisitionPolicy::DownloadIfMissing => Self::DownloadIfMissing,
        }
    }
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
pub enum CliReviewProfile {
    Mathlib,
    Internal,
    ApiDesign,
    Noise,
}

impl From<CliReviewProfile> for ReviewProfile {
    fn from(value: CliReviewProfile) -> Self {
        match value {
            CliReviewProfile::Mathlib => Self::Mathlib,
            CliReviewProfile::Internal => Self::Internal,
            CliReviewProfile::ApiDesign => Self::ApiDesign,
            CliReviewProfile::Noise => Self::Noise,
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
}
