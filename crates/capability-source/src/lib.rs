//! Packaged `lean-dup-worker` Lean source and its per-toolchain build.
//!
//! This crate hides one decision: *how the native `lean-dup-worker` executable
//! is produced from packaged Lean source for a given toolchain*. The Lean
//! source (`lean/lakefile.lean`, `lean/Main.lean`, `lean/LeanDup.lean`, and
//! `lean/LeanDup/*.lean`) ships inside the crate so it survives a crates.io
//! publish/unpack. Callers pass a target directory, a toolchain label, and the
//! matching Lean sysroot, and receive the built executable.
//!
//! The build was previously a capability dylib produced through the
//! `lean-toolchain`/`lean-rs` crates and loaded through a dlopen worker pool.
//! The native transport builds a plain executable with the toolchain's own
//! `lake` — no dlopen, no FFI, no artifact manifest — and the parent speaks
//! JSONL to it (see `LeanDup.Server`).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path,PathBuf};
use std::process::Command;

use lean_semantic_search_runtime::{
    SemanticSearchRuntimeProvenance, SemanticSearchSourcePackageRequest,
};

/// Packaged `lean-dup-worker` Lean source root, resolved at runtime to the
/// crate's unpacked location (the registry cache after `cargo install`, or the
/// checkout during development). Cargo preserves the unpacked registry source,
/// so the compile-time `CARGO_MANIFEST_DIR` resolves to a directory that still
/// holds the `lean/` payload when `install-worker` runs.
const LEAN_DUP_SOURCE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/lean");

/// Files and directories copied from the packaged source into the materialized
/// build root.
const SOURCE_ENTRIES: &[&str] = &["lakefile.lean", "Main.lean", "LeanDup.lean", "LeanDup"];

/// The built `lean-dup-worker` executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltWorker {
    /// Absolute path to the built executable (inside the materialized build
    /// root's `.lake/build/bin`).
    pub exe_path: PathBuf,
}

/// Build the `lean-dup-worker` executable for `toolchain_label` against
/// `lean_sysroot`, persisting every artifact under `install_dir`.
///
/// `install_dir` receives two subtrees, at stable absolute paths:
/// - `deps/semantic-search-runtime` — the materialized Lean dependency package
///   (network-free: the source ships inside `lean-semantic-search-runtime`);
/// - `build/lean-dup-worker-root` — the copied build root whose
///   `.lake/build/bin/lean-dup-worker` is the product.
///
/// The build runs the toolchain's own `bin/lake`, so the executable matches the
/// audited project's `.olean` format by construction.
///
/// # Errors
///
/// Returns [`BuildError`] if the dependency cannot be materialized, the build
/// root cannot be written, or Lake fails to build the executable.
pub fn build_worker_into(
    install_dir: &Path,
    toolchain_label: &str,
    lean_sysroot: &Path,
) -> Result<BuiltWorker, BuildError> {
    tracing::info!(toolchain = toolchain_label, "building lean-dup-worker executable");
    let source_root = PathBuf::from(LEAN_DUP_SOURCE_ROOT);
    let deps_root = install_dir.join("deps");
    let semantic_cache_root = deps_root.join("semantic-search-runtime");
    let build_root = install_dir.join("build").join("lean-dup-worker-root");

    let semantic_source =
        lean_semantic_search_runtime::materialize_source_package(SemanticSearchSourcePackageRequest {
            cache_root: semantic_cache_root,
            toolchain_label: toolchain_label.to_owned(),
        })
        .map_err(|error| BuildError::context("materialize semantic-search source", error))?;

    materialize_build_root(&source_root, &build_root, &semantic_source.project_root, &semantic_source.provenance)?;

    let lake = lean_sysroot.join("bin").join("lake");
    let status = Command::new(&lake)
        .args(["build", "lean-dup-worker"])
        .current_dir(&build_root)
        .status()
        .map_err(|error| BuildError::context("spawn lake build", error))?;
    if !status.success() {
        return Err(BuildError::context(
            "lake build lean-dup-worker",
            format_args!("exited with status {status}"),
        ));
    }
    let exe_path = build_root.join(".lake").join("build").join("bin").join("lean-dup-worker");
    if !exe_path.is_file() {
        return Err(BuildError::context(
            "lake build lean-dup-worker",
            format_args!("expected executable at {} but found none", exe_path.display()),
        ));
    }
    Ok(BuiltWorker { exe_path })
}

/// Copy the packaged Lean source into `build_root`, rewriting the lakefile's
/// git dependency on `lean-semantic-search` to the materialized path package
/// and pinning a manifest that points at it — so the build needs no network.
fn materialize_build_root(
    source_root: &Path,
    build_root: &Path,
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
) -> Result<(), BuildError> {
    for entry in SOURCE_ENTRIES {
        let source = source_root.join(entry);
        let dest = build_root.join(entry);
        if source.is_dir() {
            copy_dir(&source, &dest).map_err(|error| BuildError::context("copy Lean source tree", error))?;
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|error| BuildError::context("create build root", error))?;
            }
            fs::copy(&source, &dest).map_err(|error| BuildError::context("copy Lean source", error))?;
        }
    }
    let semantic_dir = fs::canonicalize(semantic_root)
        .map_err(|error| BuildError::context("canonicalize semantic-search root", error))?;
    let lakefile = format!(
        r#"import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

require {} from {:?}

@[default_target]
lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]

lean_exe «lean-dup-worker» where
  root := `Main
  -- `importModules` loads compiled module extensions through the interpreter.
  supportInterpreter := true
"#,
        semantic_provenance.materialized_package.as_str(),
        semantic_dir.display().to_string(),
    );
    fs::write(build_root.join("lakefile.lean"), lakefile)
        .map_err(|error| BuildError::context("write generated lakefile", error))?;
    let manifest = serde_json::json!({
        "version": "1.2.0",
        "packagesDir": ".lake/packages",
        "packages": [
            {
                "type": "path",
                "scope": "",
                "name": semantic_provenance.materialized_package,
                "manifestFile": "lake-manifest.json",
                "inherited": false,
                "dir": semantic_dir.display().to_string(),
                "configFile": "lakefile.lean"
            }
        ],
        "name": "lean_dup_worker",
        "lakeDir": ".lake",
        "fixedToolchain": false
    });
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|error| BuildError::context("serialize lake manifest", error))?;
    fs::write(build_root.join("lake-manifest.json"), manifest_bytes)
        .map_err(|error| BuildError::context("write lake manifest", error))?;
    Ok(())
}

fn copy_dir(source: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// A failure while building the `lean-dup-worker` executable. Carries a
/// human-readable context label plus the underlying error's message;
/// `install-worker` surfaces it verbatim with an actionable hint.
#[derive(Debug)]
pub struct BuildError {
    context: &'static str,
    message: String,
}

impl BuildError {
    fn context(context: &'static str, error: impl fmt::Display) -> Self {
        Self {
            context,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for BuildError {}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code uses unwrap/expect/panic to surface failure paths concisely"
)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn collect_digest_entries(
        source_root: &Path,
        relative: &Path,
        entries: &mut Vec<(String, String)>,
    ) -> io::Result<()> {
        let source = source_root.join(relative);
        let metadata = fs::symlink_metadata(&source)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            for entry in fs::read_dir(&source)? {
                let entry = entry?;
                collect_digest_entries(source_root, &relative.join(entry.file_name()), entries)?;
            }
        } else if metadata.is_file() {
            let mut hasher = Sha256::new();
            hasher.update(fs::read(&source)?);
            let digest = hasher.finalize();
            entries.push((
                relative.to_string_lossy().into_owned(),
                digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            ));
        }
        Ok(())
    }

    /// The packaged Lean source must stay byte-identical to the editable dev
    /// project under `<repo>/lean/`. The dev project is what `lake build` and
    /// the bump-toolchain skill operate on; this vendored copy is what ships to
    /// crates.io and gets built on the user's machine. They must not drift.
    #[test]
    fn vendored_lean_source_matches_dev_project() {
        let vendored = PathBuf::from(LEAN_DUP_SOURCE_ROOT);
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("capability-source crate lives under <repo>/crates/capability-source")
            .join("lean");
        let tracked = ["lakefile.lean", "Main.lean", "LeanDup.lean", "LeanDup"];
        let digest_of = |root: &Path| {
            let mut entries = Vec::new();
            for relative in tracked {
                collect_digest_entries(root, Path::new(relative), &mut entries).unwrap();
            }
            entries.sort();
            entries
        };
        assert_eq!(
            digest_of(&vendored),
            digest_of(&dev),
            "vendored crates/capability-source/lean/ has drifted from <repo>/lean/; \
             re-sync with: cp lean/lakefile.lean lean/Main.lean lean/LeanDup.lean crates/capability-source/lean/ && \
             rm -rf crates/capability-source/lean/LeanDup && cp -R lean/LeanDup crates/capability-source/lean/LeanDup"
        );
    }
}
