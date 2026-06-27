//! Packaged `LeanDup` capability source and its per-toolchain build.
//!
//! This crate hides one decision: *how the `LeanDup` shared-facet capability
//! dylib is produced from packaged Lean source for a given toolchain*. The Lean
//! source (`lean/LeanDup.lean` + `lean/LeanDup/*.lean`) ships inside the crate so
//! it survives a crates.io publish/unpack, exactly as
//! [`lean_semantic_search_runtime`] ships its own runtime payload. Callers pass a
//! target directory, a toolchain label, and the matching Lean sysroot, and
//! receive the built dylib and its artifact manifest.
//!
//! The build was previously a `build.rs` in `lean-dup-worker` that baked an
//! `OUT_DIR` manifest path at compile time. That broke `cargo install`: the
//! published worker crate did not contain `<repo>/lean/`, and `OUT_DIR` is
//! deleted after install. Lifting the build here — invoked at *install-worker*
//! time on the user's machine — keeps `cargo install lean-dup` pure Rust and
//! produces the dylib at a stable absolute path under the worker install dir.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lean_rs_interop_shims::LeanRsInteropShimsSourcePackageRequest;
use lean_rs_worker_protocol::worker_exports::{json_command_signature, streaming_command_signature};
use lean_semantic_search_runtime::{
    SemanticSearchRuntimeBuild, SemanticSearchRuntimeProvenance, SemanticSearchSourcePackageRequest,
};
use lean_toolchain::{
    CargoLeanCapability, GeneratedSourceFile, SourcePackageManifestPolicy, SourcePackageMaterializationRequest,
};
use sha2::{Digest, Sha256};

/// Packaged `LeanDup` Lean source root, resolved at runtime to the crate's
/// unpacked location (the registry cache after `cargo install`, or the checkout
/// during development). Mirrors `lean-semantic-search-runtime`'s
/// `RUNTIME_SOURCE_ROOT`: cargo preserves the unpacked registry source, so the
/// compile-time `CARGO_MANIFEST_DIR` resolves to a directory that still holds the
/// `lean/` payload when `install-worker` runs.
const LEAN_DUP_SOURCE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/lean");

/// The five capability export symbols advertised in the artifact manifest. These
/// are the capability ABI; the worker's command dispatch references the same
/// names (kept in sync there, asserted at install time by the smoke test).
const VERSION_EXPORT: &str = "lean_dup_capability_version";
const EXTRACT_EXPORT: &str = "lean_dup_capability_extract";
const FEATURES_EXPORT: &str = "lean_dup_capability_features";
const PROBE_EXPORT: &str = "lean_dup_capability_probe";
const INDEX_EXPORT: &str = "lean_dup_capability_index";

/// The built `LeanDup` capability.
///
/// Carries the artifact manifest the worker parent loads and the shared-facet
/// dylib it points at. Both are absolute paths under the `install_dir` passed to
/// [`build_capability_into`], so they survive the build process and resolve at
/// worker-spawn time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltCapability {
    /// Path to the JSON artifact manifest (consumed by `LeanBuiltCapability`).
    pub manifest_path: PathBuf,
    /// Path to the built `LeanDup` shared-facet dylib.
    pub dylib_path: PathBuf,
}

/// Build the `LeanDup` capability for `toolchain_label` against `lean_sysroot`,
/// persisting every artifact under `install_dir`.
///
/// `install_dir` receives three subtrees, all at stable absolute paths so the
/// worker resolves them later without rebuilding:
/// - `deps/semantic-search-runtime` and `deps/interop-shims` — the materialized
///   Lean dependency packages and their cached dylibs;
/// - `build/lean-dup-capability-root` — the generated Lake root and the built
///   `LeanDup` dylib + manifest.
///
/// `lean_sysroot` is the Lean prefix containing `include/lean/lean.h` and
/// `bin/lake`; it is passed only to the spawned Lake command (never mutated into
/// the process environment).
///
/// # Errors
///
/// Returns [`BuildError`] if any dependency cannot be materialized or built, the
/// generated Lake root cannot be written, or Lake fails to build the capability.
pub fn build_capability_into(
    install_dir: &Path,
    toolchain_label: &str,
    lean_sysroot: &Path,
) -> Result<BuiltCapability, BuildError> {
    let source_root = PathBuf::from(LEAN_DUP_SOURCE_ROOT);
    let deps_root = install_dir.join("deps");
    let semantic_cache_root = deps_root.join("semantic-search-runtime");
    let interop_cache_root = deps_root.join("interop-shims");
    let build_cache_root = install_dir.join("build").join("lean-dup-capability-root");

    let semantic_source =
        lean_semantic_search_runtime::materialize_source_package(SemanticSearchSourcePackageRequest {
            cache_root: semantic_cache_root.clone(),
            toolchain_label: toolchain_label.to_owned(),
        })
        .map_err(|error| BuildError::context("materialize semantic-search source", error))?;
    let semantic_runtime = lean_semantic_search_runtime::build_cached(SemanticSearchRuntimeBuild {
        cache_root: semantic_cache_root,
        toolchain_label: toolchain_label.to_owned(),
        lean_sysroot: lean_sysroot.to_path_buf(),
    })
    .map_err(|error| BuildError::context("build semantic-search runtime", error))?;
    let semantic_dependency = semantic_runtime
        .dependency()
        .map_err(|error| BuildError::context("resolve semantic-search dependency", error))?;
    let interop_source = lean_rs_interop_shims::materialize_source_package(LeanRsInteropShimsSourcePackageRequest {
        cache_root: interop_cache_root,
        toolchain_label: toolchain_label.to_owned(),
    })
    .map_err(|error| BuildError::context("materialize interop-shims source", error))?;

    let build_root = materialize_lean_dup_build_root(
        &source_root,
        &build_cache_root,
        &semantic_source.project_root,
        &semantic_runtime.provenance,
        &interop_source.project_root,
        toolchain_label,
    )?;

    let built = CargoLeanCapability::new(&build_root, "LeanDup")
        .package("lean_dup_worker")
        .module("LeanDup")
        .lean_sysroot(lean_sysroot.to_path_buf())
        .dependency(semantic_dependency)
        .export_signature(json_command_signature(VERSION_EXPORT))
        .export_signature(streaming_command_signature(EXTRACT_EXPORT))
        .export_signature(streaming_command_signature(FEATURES_EXPORT))
        .export_signature(streaming_command_signature(PROBE_EXPORT))
        .export_signature(streaming_command_signature(INDEX_EXPORT))
        .build_quiet()
        .map_err(|error| BuildError::context("build LeanDup capability", error))?;

    Ok(BuiltCapability {
        manifest_path: built.manifest_path().to_path_buf(),
        dylib_path: built.dylib_path().to_path_buf(),
    })
}

fn materialize_lean_dup_build_root(
    source_root: &Path,
    cache_root: &Path,
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
    toolchain_label: &str,
) -> Result<PathBuf, BuildError> {
    let lakefile = generated_lakefile_text(semantic_root, semantic_provenance, interop_root)?;
    let manifest = generated_manifest_bytes(semantic_root, semantic_provenance, interop_root)?;
    let source_digest = lean_dup_source_digest(source_root, &lakefile, &manifest)?;
    let materialized = lean_toolchain::materialize_source_package(&SourcePackageMaterializationRequest {
        source_root: source_root.to_path_buf(),
        cache_root: cache_root.to_path_buf(),
        package_name: "lean_dup_worker".to_owned(),
        materialized_package_name: "lean_dup_worker".to_owned(),
        library_name: "LeanDup".to_owned(),
        source_digest,
        source_revision: env!("CARGO_PKG_VERSION").to_owned(),
        crate_name: env!("CARGO_PKG_NAME").to_owned(),
        crate_version: env!("CARGO_PKG_VERSION").to_owned(),
        toolchain_label: toolchain_label.to_owned(),
        include_paths: vec![PathBuf::from("LeanDup.lean"), PathBuf::from("LeanDup")],
        generated_files: vec![
            GeneratedSourceFile {
                relative_path: PathBuf::from("lakefile.lean"),
                contents: lakefile.into_bytes(),
            },
            GeneratedSourceFile {
                relative_path: PathBuf::from("lake-manifest.json"),
                contents: manifest,
            },
        ],
        sentinel_files: vec![PathBuf::from("LeanDup/Capability.lean")],
        manifest_policy: SourcePackageManifestPolicy::AllowPackages,
    })
    .map_err(|error| BuildError::context("materialize LeanDup build root", error))?;
    Ok(materialized.project_root)
}

fn generated_lakefile_text(
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
) -> Result<String, BuildError> {
    let semantic_root =
        fs::canonicalize(semantic_root).map_err(|error| BuildError::context("canonicalize semantic root", error))?;
    let interop_root =
        fs::canonicalize(interop_root).map_err(|error| BuildError::context("canonicalize interop root", error))?;
    Ok(format!(
        r#"import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

require {} from {}
require «lean_rs_interop_shims» from {}

@[default_target]
lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]
  defaultFacets := #[LeanLib.sharedFacet]
"#,
        semantic_provenance.materialized_package.as_str(),
        lean_string_literal(&semantic_root),
        lean_string_literal(&interop_root)
    ))
}

fn generated_manifest_bytes(
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
) -> Result<Vec<u8>, BuildError> {
    let semantic_dir = fs::canonicalize(semantic_root)
        .map_err(|error| BuildError::context("canonicalize semantic root", error))?
        .display()
        .to_string();
    let interop_dir = fs::canonicalize(interop_root)
        .map_err(|error| BuildError::context("canonicalize interop root", error))?
        .display()
        .to_string();
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
                "dir": semantic_dir,
                "configFile": "lakefile.lean"
            },
            {
                "type": "path",
                "scope": "",
                "name": "lean_rs_interop_shims",
                "manifestFile": "lake-manifest.json",
                "inherited": false,
                "dir": interop_dir,
                "configFile": "lakefile.lean"
            }
        ],
        "name": "lean_dup_worker",
        "lakeDir": ".lake",
        "fixedToolchain": false
    });
    serde_json::to_vec_pretty(&manifest).map_err(|error| BuildError::context("serialize lake manifest", error))
}

fn lean_string_literal(path: &Path) -> String {
    serde_json::to_string(&path.display().to_string()).unwrap_or_else(|_| "\"\"".to_owned())
}

fn lean_dup_source_digest(source_root: &Path, lakefile: &str, manifest: &[u8]) -> Result<String, BuildError> {
    let mut entries = Vec::new();
    collect_digest_entries(source_root, Path::new("LeanDup.lean"), &mut entries)
        .map_err(|error| BuildError::context("digest LeanDup.lean", error))?;
    collect_digest_entries(source_root, Path::new("LeanDup"), &mut entries)
        .map_err(|error| BuildError::context("digest LeanDup directory", error))?;
    entries.push(("lakefile.lean".to_owned(), sha256_hex(lakefile.as_bytes())));
    entries.push(("lake-manifest.json".to_owned(), sha256_hex(manifest)));
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut outer = Sha256::new();
    for (canonical_path, digest) in entries {
        outer.update(digest.as_bytes());
        outer.update(b"  ");
        outer.update(canonical_path.as_bytes());
        outer.update(b"\n");
    }
    Ok(hex_lower(&outer.finalize()))
}

fn collect_digest_entries(source_root: &Path, relative: &Path, entries: &mut Vec<(String, String)>) -> io::Result<()> {
    let source = source_root.join(relative);
    let metadata = fs::symlink_metadata(&source)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            collect_digest_entries(source_root, &relative.join(entry.file_name()), entries)?;
        }
    } else if metadata.is_file() {
        entries.push((relative.to_string_lossy().into_owned(), sha256_hex(&fs::read(source)?)));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "hex encoding indexes a fixed 16-byte table with masked nibbles"
)]
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[(byte >> 4) as usize]));
        out.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    out
}

/// A failure while building the `LeanDup` capability. Carries a human-readable
/// context label plus the underlying error's message; `install-worker` surfaces
/// it verbatim with an actionable hint.
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

    /// The packaged Lean source must stay byte-identical to the editable dev
    /// project under `<repo>/lean/`. The dev project is what `lake build LeanDup`
    /// and the bump-toolchain skill operate on; this vendored copy is what ships
    /// to crates.io and gets built on the user's machine. They must not drift.
    #[test]
    fn vendored_lean_source_matches_dev_project() {
        let vendored = PathBuf::from(LEAN_DUP_SOURCE_ROOT);
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("capability-source crate lives under <repo>/crates/capability-source")
            .join("lean");
        let mut vendored_entries = Vec::new();
        collect_digest_entries(&vendored, Path::new("LeanDup.lean"), &mut vendored_entries).unwrap();
        collect_digest_entries(&vendored, Path::new("LeanDup"), &mut vendored_entries).unwrap();
        let mut dev_entries = Vec::new();
        collect_digest_entries(&dev, Path::new("LeanDup.lean"), &mut dev_entries).unwrap();
        collect_digest_entries(&dev, Path::new("LeanDup"), &mut dev_entries).unwrap();
        vendored_entries.sort();
        dev_entries.sort();
        assert_eq!(
            vendored_entries, dev_entries,
            "vendored crates/capability-source/lean/ has drifted from <repo>/lean/; \
             re-sync with: cp lean/LeanDup.lean crates/capability-source/lean/ && \
             rm -rf crates/capability-source/lean/LeanDup && cp -R lean/LeanDup crates/capability-source/lean/LeanDup"
        );
    }
}
