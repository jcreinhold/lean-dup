use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Diagnostics(#[from] lean_dup_diagnostics::Error),

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
}

pub type Result<T> = std::result::Result<T, Error>;
