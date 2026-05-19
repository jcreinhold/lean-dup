use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("embedding runtime is not implemented until Prompt 35C")]
    UnsupportedUntilRuntimePrompt,
}

pub type Result<T> = std::result::Result<T, Error>;
