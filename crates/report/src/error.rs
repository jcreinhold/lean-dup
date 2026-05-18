use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Diagnostics(#[from] lean_dup_diagnostics::Error),

    #[error("{0}")]
    Search(#[from] lean_dup_search::Error),

    #[error("report error: {message}")]
    Report { message: String },

    #[error("could not render JSON output")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
