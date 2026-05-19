use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("embedding runtime is not implemented until Prompt 35C")]
    UnsupportedUntilRuntimePrompt,

    #[error("embedding model id must not be empty")]
    EmptyModelId,

    #[error("embedding model revision must not be empty when provided")]
    EmptyRevision,

    #[error("embedding model is not prepared: {reason}")]
    ModelNotPrepared { reason: String },

    #[error("embedding model is unsupported: {reason}")]
    UnsupportedModel { reason: String },

    #[error("failed to read embedding runtime file during {operation}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse embedding runtime JSON for {artifact}")]
    Json {
        artifact: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("embedding tokenizer failed: {reason}")]
    Tokenizer { reason: String },

    #[error("embedding runtime failed: {reason}")]
    Runtime { reason: String },

    #[error("embedding vector cache failed during {operation}")]
    VectorCache {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("embedding vector is invalid: {reason}")]
    InvalidVector { reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;
