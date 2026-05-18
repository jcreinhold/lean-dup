use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Diagnostics(#[from] lean_dup_diagnostics::Error),

    #[error("{0}")]
    Eval(#[from] lean_dup_eval::Error),

    #[error("{0}")]
    Index(#[from] lean_dup_index::Error),

    #[error("{0}")]
    Project(#[from] lean_dup_project::Error),

    #[error("{0}")]
    Report(#[from] lean_dup_report::Error),

    #[error("{0}")]
    Search(#[from] lean_dup_search::Error),

    #[error("{0}")]
    Worker(#[from] lean_dup_worker::WorkerError),

    #[error("{message}: {path}")]
    Io {
        message: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("CLI error: {message}")]
    Cli { message: String },

    #[error("could not render JSON output")]
    Json(#[from] serde_json::Error),

    #[error("could not write CLI output")]
    Write(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AppError>;
