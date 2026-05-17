use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::error::{Error, Result};
use crate::progress::Reporter;

#[derive(Debug, Clone)]
pub(crate) struct LeanVersion {
    pub(crate) text: String,
}

pub(crate) fn lean_version(root: &Path, reporter: &mut Reporter) -> Result<LeanVersion> {
    reporter.event("lake", None, None, "running lake env lean --version");
    let started = Instant::now();
    let output = Command::new("lake")
        .args(["env", "lean", "--version"])
        .current_dir(root)
        .output()
        .map_err(|source| Error::Io {
            message: "could not run lake",
            path: root.to_path_buf(),
            source,
        })?;
    reporter.timing("lake.lean_version", started.elapsed());

    if !output.status.success() {
        return Err(Error::LakeCommand {
            command: "lake env lean --version".to_owned(),
            diagnostic: bounded_diagnostic(&output.stdout, &output.stderr),
        });
    }

    Ok(LeanVersion {
        text: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    })
}

fn bounded_diagnostic(stdout: &[u8], stderr: &[u8]) -> String {
    let combined = [stdout, stderr]
        .into_iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if combined.chars().count() <= 2_000 {
        return combined;
    }
    combined.chars().take(2_000).collect::<String>()
}
