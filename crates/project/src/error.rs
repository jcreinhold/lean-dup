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

    #[error("{}", format_no_source_files(.root, .selected_roots, .available_roots))]
    NoSourceFiles {
        root: PathBuf,
        selected_roots: Vec<String>,
        available_roots: Vec<String>,
    },

    #[error("invalid lakefile TOML: {path}")]
    LakefileToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

fn format_no_source_files(root: &PathBuf, selected: &[String], available: &[String]) -> String {
    let selected_label = if selected.is_empty() {
        "(none)".to_owned()
    } else {
        selected.join(", ")
    };
    let mut message = format!(
        "no Lean source files found for selected module roots ({selected_label}) in {}",
        root.display()
    );
    if !available.is_empty() && available != selected {
        message.push_str(&format!("\nhelp: available module roots: {}", available.join(", ")));
    }
    message
}
