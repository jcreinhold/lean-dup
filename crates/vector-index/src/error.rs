use std::io;

use crate::VectorCorpusStatus;

pub type Result<T> = std::result::Result<T, VectorIndexError>;

#[derive(Debug, thiserror::Error)]
pub enum VectorIndexError {
    #[error("invalid vector corpus request: {reason}")]
    InvalidRequest { reason: String },
    #[error("vector corpus is {status:?}: {reason}")]
    CorpusUnavailable { status: VectorCorpusStatus, reason: String },
    #[error("vector corpus storage failed: {reason}")]
    Storage { reason: String },
    #[error("vector corpus manifest failed: {source}")]
    Manifest { source: serde_json::Error },
    #[error("vector corpus I/O failed: {source}")]
    Io { source: io::Error },
}

impl VectorIndexError {
    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        Self::InvalidRequest { reason: reason.into() }
    }

    pub(crate) fn storage(reason: impl Into<String>) -> Self {
        Self::Storage { reason: reason.into() }
    }

    pub(crate) fn unavailable(status: VectorCorpusStatus, reason: impl Into<String>) -> Self {
        Self::CorpusUnavailable {
            status,
            reason: reason.into(),
        }
    }
}

impl From<io::Error> for VectorIndexError {
    fn from(source: io::Error) -> Self {
        Self::Io { source }
    }
}

impl From<serde_json::Error> for VectorIndexError {
    fn from(source: serde_json::Error) -> Self {
        Self::Manifest { source }
    }
}
