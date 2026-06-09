use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lean_rs_interop_shims::LeanRsInteropShimsSourcePackageRequest;
use lean_semantic_search_runtime::{
    SemanticSearchRuntimeBuild, SemanticSearchRuntimeProvenance, SemanticSearchSourcePackageRequest,
};

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
    let build_root = out_dir.join("lean-dup-capability-root");
    materialize_lean_dup_build_root(
        &lean_root,
        &build_root,
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
    build_root: &Path,
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
    toolchain_label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    remove_path_if_exists(build_root)?;
    fs::create_dir_all(build_root)?;
    fs::copy(source_root.join("LeanDup.lean"), build_root.join("LeanDup.lean"))?;
    copy_dir_recursive(&source_root.join("LeanDup"), &build_root.join("LeanDup"))?;
    fs::write(build_root.join("lean-toolchain"), format!("{toolchain_label}\n"))?;
    write_generated_lakefile(build_root, semantic_root, semantic_provenance, interop_root)?;
    write_generated_manifest(build_root, semantic_root, semantic_provenance, interop_root)?;
    Ok(())
}

fn write_generated_lakefile(
    build_root: &Path,
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let semantic_root = fs::canonicalize(semantic_root)?;
    let interop_root = fs::canonicalize(interop_root)?;
    let text = format!(
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
    );
    fs::write(build_root.join("lakefile.lean"), text)?;
    Ok(())
}

fn write_generated_manifest(
    build_root: &Path,
    semantic_root: &Path,
    semantic_provenance: &SemanticSearchRuntimeProvenance,
    interop_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
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
    fs::write(
        build_root.join("lake-manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

fn lean_string_literal(path: &Path) -> String {
    serde_json::to_string(&path.display().to_string()).unwrap_or_else(|_| "\"\"".to_owned())
}

fn copy_dir_recursive(source: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &dest_path)?;
        } else if source_path.is_file() {
            fs::copy(&source_path, &dest_path)?;
        }
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
