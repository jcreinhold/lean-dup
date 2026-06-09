use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lean_rs_interop_shims::LeanRsInteropShimsSourcePackageRequest;
use lean_semantic_search_runtime::{
    SemanticSearchRuntimeBuild, SemanticSearchRuntimeProvenance, SemanticSearchSourcePackageRequest,
};
use lean_toolchain::{GeneratedSourceFile, SourcePackageManifestPolicy, SourcePackageMaterializationRequest};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir.parent().and_then(std::path::Path::parent).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "worker crate must live under repo/crates/worker",
        )
    })?;
    let lean_root = repo_root.join("lean");
    let toolchain_label = read_toolchain_label(&lean_root)?;
    let lean_sysroot = resolve_lean_sysroot(&toolchain_label, &lean_root)?;
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);

    emit_rerun_inputs(&lean_root)?;
    println!("cargo:rerun-if-env-changed=LEAN_SYSROOT");
    println!("cargo:rerun-if-env-changed=ELAN_HOME");

    let semantic_cache_root = out_dir.join("semantic-search-runtime-cache");
    let semantic_source =
        lean_semantic_search_runtime::materialize_source_package(SemanticSearchSourcePackageRequest {
            cache_root: semantic_cache_root.clone(),
            toolchain_label: toolchain_label.clone(),
        })?;
    let semantic_runtime = lean_semantic_search_runtime::build_cached(SemanticSearchRuntimeBuild {
        cache_root: semantic_cache_root,
        toolchain_label: toolchain_label.clone(),
        lean_sysroot: lean_sysroot.clone(),
    })?;
    let semantic_dependency = semantic_runtime.dependency()?;
    let interop_source = lean_rs_interop_shims::materialize_source_package(LeanRsInteropShimsSourcePackageRequest {
        cache_root: out_dir.join("lean-rs-interop-shims-cache"),
        toolchain_label: toolchain_label.clone(),
    })?;
    let interop_root = interop_source.project_root;
    let build_root = materialize_lean_dup_build_root(
        &lean_root,
        &out_dir.join("lean-dup-capability-root-cache"),
        &semantic_source.project_root,
        &semantic_runtime.provenance,
        &interop_root,
        &toolchain_label,
    )?;

    use lean_rs_worker_protocol::worker_exports::{json_command_signature, streaming_command_signature};
    let built = lean_toolchain::CargoLeanCapability::new(&build_root, "LeanDup")
        .package("lean_dup_worker")
        .module("LeanDup")
        .lean_sysroot(lean_sysroot)
        .dependency(semantic_dependency)
        .export_signature(json_command_signature("lean_dup_capability_version"))
        .export_signature(streaming_command_signature("lean_dup_capability_extract"))
        .export_signature(streaming_command_signature("lean_dup_capability_features"))
        .export_signature(streaming_command_signature("lean_dup_capability_probe"))
        .export_signature(streaming_command_signature("lean_dup_capability_index"))
        .build_quiet()?;
    println!(
        "cargo:rustc-env={}={}",
        built.manifest_env_var(),
        built.manifest_path().display()
    );

    Ok(())
}

fn read_toolchain_label(lean_root: &Path) -> io::Result<String> {
    let path = lean_root.join("lean-toolchain");
    let label = fs::read_to_string(&path)?;
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is empty", path.display()),
        ));
    }
    Ok(trimmed.to_owned())
}

fn resolve_lean_sysroot(toolchain_label: &str, lean_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(sysroot) = std::env::var_os("LEAN_SYSROOT").map(PathBuf::from)
        && lean_header_exists(&sysroot)
    {
        return Ok(sysroot);
    }
    if let Some(sysroot) = elan_toolchain_dir(toolchain_label)
        && lean_header_exists(&sysroot)
    {
        return Ok(sysroot);
    }
    let info = lean_toolchain::discover_toolchain(&lean_toolchain::DiscoverOptions {
        toolchain_file: Some(lean_root.join("lean-toolchain")),
        ..lean_toolchain::DiscoverOptions::default()
    })?;
    Ok(info.prefix)
}

fn lean_header_exists(sysroot: &Path) -> bool {
    sysroot.join("include").join("lean").join("lean.h").is_file()
}

fn elan_toolchain_dir(toolchain_label: &str) -> Option<PathBuf> {
    let elan_home = std::env::var_os("ELAN_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".elan")))?;
    Some(elan_home.join("toolchains").join(elan_directory_name(toolchain_label)))
}

fn elan_directory_name(toolchain_label: &str) -> String {
    toolchain_label.replace('/', "--").replace(':', "---")
}

fn emit_rerun_inputs(lean_root: &Path) -> io::Result<()> {
    for file_name in ["lakefile.lean", "lean-toolchain", "lake-manifest.json"] {
        println!("cargo:rerun-if-changed={}", lean_root.join(file_name).display());
    }
    emit_lean_source_reruns(lean_root)
}

fn emit_lean_source_reruns(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name() == Some(OsStr::new(".lake")) {
            continue;
        }
        if path.is_dir() {
            emit_lean_source_reruns(&path)?;
        } else if path.extension() == Some(OsStr::new("lean")) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    Ok(())
}

fn materialize_lean_dup_build_root(
    source_root: &Path,
    cache_root: &Path,
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
    toolchain_label: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
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
    })?;
    Ok(materialized.project_root)
}

fn generated_lakefile_text(
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let semantic_root = fs::canonicalize(semantic_root)?;
    let interop_root = fs::canonicalize(interop_root)?;
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
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
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
                "dir": fs::canonicalize(semantic_root)?.display().to_string(),
                "configFile": "lakefile.lean"
            },
            {
                "type": "path",
                "scope": "",
                "name": "lean_rs_interop_shims",
                "manifestFile": "lake-manifest.json",
                "inherited": false,
                "dir": fs::canonicalize(interop_root)?.display().to_string(),
                "configFile": "lakefile.lean"
            }
        ],
        "name": "lean_dup_worker",
        "lakeDir": ".lake",
        "fixedToolchain": false
    });
    Ok(serde_json::to_vec_pretty(&manifest)?)
}

fn lean_string_literal(path: &Path) -> String {
    serde_json::to_string(&path.display().to_string()).unwrap_or_else(|_| "\"\"".to_owned())
}

fn lean_dup_source_digest(
    source_root: &Path,
    lakefile: &str,
    manifest: &[u8],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    collect_digest_entries(source_root, Path::new("LeanDup.lean"), &mut entries)?;
    collect_digest_entries(source_root, Path::new("LeanDup"), &mut entries)?;
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
