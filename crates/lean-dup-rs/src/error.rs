use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("{message}: {path}")]
    Io {
        message: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("workspace does not exist: {0}")]
    WorkspaceMissing(PathBuf),

    #[error("not a Lake workspace: {0}")]
    NotLakeWorkspace(PathBuf),

    #[error("could not infer Lean module roots in {0}; pass --module")]
    NoModuleRoots(PathBuf),

    #[error("no Lean source files found for selected module roots in {0}")]
    NoSourceFiles(PathBuf),

    #[error("invalid lakefile TOML: {path}")]
    LakefileToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("{0}")]
    Worker(#[from] crate::worker::WorkerError),

    #[error("could not render JSON output")]
    Json(#[from] serde_json::Error),

    #[error("could not write CLI output")]
    Write(#[from] std::io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) fn read_to_string(path: PathBuf) -> Result<String> {
    std::fs::read_to_string(&path).map_err(|source| Error::Io {
        message: "could not read file",
        path,
        source,
    })
}

pub(crate) fn read(path: PathBuf) -> Result<Vec<u8>> {
    std::fs::read(&path).map_err(|source| Error::Io {
        message: "could not read file",
        path,
        source,
    })
}
