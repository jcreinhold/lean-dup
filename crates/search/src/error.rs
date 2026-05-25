use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Diagnostics(#[from] lean_dup_diagnostics::Error),

    #[error("{0}")]
    Index(#[from] lean_dup_index::Error),

    #[error("{0}")]
    Project(#[from] lean_dup_project::Error),

    #[error("{0}")]
    Worker(#[from] lean_dup_worker::WorkerError),

    #[error("{message}: {path}")]
    Io {
        message: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{message}")]
    Search { message: String },

    #[error("could not render JSON output")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
