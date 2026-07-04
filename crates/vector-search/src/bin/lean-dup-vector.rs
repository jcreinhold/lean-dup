use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use lean_dup_diagnostics::progress::Reporter;
use lean_dup_embedding::{
    EmbeddingAcquisitionPolicy, EmbeddingModelSpec, EmbeddingPrepareRequest, prepare_embedding_model,
};
use lean_dup_vector_search::{
    VectorAcquisitionPolicy, VectorValidationBounds, VectorValidationRequest, run_vector_validation,
};

#[derive(Debug, Parser)]
#[command(name = "lean-dup-vector")]
#[command(about = "Hidden semantic/vector validation operator tool")]
struct Cli {
    #[arg(long, global = true)]
    progress: bool,

    #[arg(long, global = true)]
    profile: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Validate(Box<ValidateArgs>),
    #[command(name = "prepare-model")]
    PrepareModel(PrepareModelArgs),
}

#[derive(Debug, clap::Args)]
struct ValidateArgs {
    #[arg(long, default_value = "vector-fixture")]
    suite: String,
    #[arg(long)]
    workspace: Option<PathBuf>,
    #[arg(long)]
    mathlib_workspace: Option<PathBuf>,
    #[arg(long)]
    manual_module: Option<String>,
    #[arg(long, default_value = "bge-small-en-v1.5")]
    profile_id: String,
    #[arg(long)]
    revision: Option<String>,
    #[arg(long, value_enum, default_value_t = CliAcquisitionPolicy::CacheOnly)]
    acquisition: CliAcquisitionPolicy,
    #[arg(long, default_value = "asymmetric-query-document")]
    input_format: String,
    #[arg(long, default_value = "name-and-statement")]
    document_policy: String,
    #[arg(long, default_value = "actionable-public-statement")]
    eligibility_policy: String,
    #[arg(long)]
    model_cache_root: Option<PathBuf>,
    #[arg(long)]
    text_vector_cache_root: Option<PathBuf>,
    #[arg(long)]
    corpus_cache_root: PathBuf,
    #[arg(long)]
    artifact_root: Option<PathBuf>,
    #[arg(long = "k", value_delimiter = ',', default_value = "1,5,10")]
    k_values: Vec<usize>,
    #[arg(long, default_value_t = 5_000)]
    max_declarations: usize,
    #[arg(long, default_value_t = 1_000)]
    max_queries: usize,
    #[arg(long, default_value_t = 900_000)]
    max_runtime_ms: u128,
    #[arg(long, default_value_t = 8 * 1024 * 1024 * 1024)]
    max_rss_bytes: u64,
}

#[derive(Debug, clap::Args)]
struct PrepareModelArgs {
    #[arg(long, default_value = "BAAI/bge-small-en-v1.5")]
    model_id: String,
    #[arg(long)]
    revision: Option<String>,
    #[arg(long, value_enum, default_value_t = CliAcquisitionPolicy::DownloadIfMissing)]
    acquisition: CliAcquisitionPolicy,
    #[arg(long)]
    cache_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliAcquisitionPolicy {
    CacheOnly,
    DownloadIfMissing,
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    lean_dup_diagnostics::install_tracing("warn");
    let cli = Cli::parse();
    tracing::debug!("lean-dup-vector starting");
    let mut reporter = Reporter::new_live(cli.progress, cli.profile);
    match cli.command {
        Command::Validate(args) => {
            let outcome = run_vector_validation(validate_request(*args), &mut reporter)?;
            println!("{}", serde_json::to_string_pretty(&outcome).expect("serialize outcome"));
        }
        Command::PrepareModel(args) => {
            let result = prepare_embedding_model(EmbeddingPrepareRequest {
                model: EmbeddingModelSpec {
                    id: args.model_id,
                    revision: args.revision,
                },
                acquisition_policy: args.acquisition.into_embedding(),
                cache_root: args.cache_root,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize prepare result")
            );
        }
    }
    Ok(())
}

fn validate_request(args: ValidateArgs) -> VectorValidationRequest {
    VectorValidationRequest::new(args.suite, args.corpus_cache_root)
        .with_workspace(args.workspace)
        .with_mathlib_workspace(args.mathlib_workspace)
        .with_manual_module(args.manual_module)
        .with_profile(args.profile_id, args.revision)
        .with_acquisition_policy(args.acquisition.into_vector())
        .with_input_format(args.input_format)
        .with_document_policy(args.document_policy)
        .with_eligibility_policy(args.eligibility_policy)
        .with_model_cache_root(args.model_cache_root)
        .with_text_vector_cache_root(args.text_vector_cache_root)
        .with_artifact_root(args.artifact_root)
        .with_k_values(args.k_values)
        .with_bounds(VectorValidationBounds {
            max_declarations: args.max_declarations,
            max_queries: args.max_queries,
            max_runtime_ms: args.max_runtime_ms,
            max_rss_bytes: args.max_rss_bytes,
        })
}

impl CliAcquisitionPolicy {
    fn into_vector(self) -> VectorAcquisitionPolicy {
        match self {
            Self::CacheOnly => VectorAcquisitionPolicy::CacheOnly,
            Self::DownloadIfMissing => VectorAcquisitionPolicy::DownloadIfMissing,
        }
    }

    fn into_embedding(self) -> EmbeddingAcquisitionPolicy {
        match self {
            Self::CacheOnly => EmbeddingAcquisitionPolicy::CacheOnly,
            Self::DownloadIfMissing => EmbeddingAcquisitionPolicy::DownloadIfMissing,
        }
    }
}
