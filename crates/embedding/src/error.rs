use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("embedding runtime is not implemented until Prompt 35C")]
    UnsupportedUntilRuntimePrompt,

    #[error("embedding model id must not be empty")]
    EmptyModelId,

    #[error("embedding model revision must not be empty when provided")]
    EmptyRevision,
}

pub type Result<T> = std::result::Result<T, Error>;
