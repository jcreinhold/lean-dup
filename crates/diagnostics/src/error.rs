use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{message}: {path}")]
    Io {
        message: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not render JSON output")]
    Json(#[from] serde_json::Error),

    #[error("could not write CLI output")]
    Write(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn read_to_string(path: PathBuf) -> Result<String> {
    std::fs::read_to_string(&path).map_err(|source| Error::Io {
        message: "could not read file",
        path,
        source,
    })
}

pub fn read(path: PathBuf) -> Result<Vec<u8>> {
    std::fs::read(&path).map_err(|source| Error::Io {
        message: "could not read file",
        path,
        source,
    })
}
