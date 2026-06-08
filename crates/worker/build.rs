use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir.parent().and_then(std::path::Path::parent).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "worker crate must live under repo/crates/worker",
        )
    })?;
    let lean_root = repo_root.join("lean");

    emit_rerun_inputs(&lean_root)?;
    use lean_rs_worker_protocol::worker_exports::{json_command_signature, streaming_command_signature};
    let built = lean_toolchain::CargoLeanCapability::new(&lean_root, "LeanDup")
        .package("lean_dup_worker")
        .module("LeanDup")
        .export_signature(json_command_signature("lean_dup_capability_version"))
        .export_signature(streaming_command_signature("lean_dup_capability_extract"))
        .export_signature(streaming_command_signature("lean_dup_capability_features"))
        .export_signature(streaming_command_signature("lean_dup_capability_probe"))
        .export_signature(streaming_command_signature("lean_dup_capability_index"))
        .build_quiet()?;
    add_semantic_search_dependency(&lean_root, built.manifest_path())?;
    println!(
        "cargo:rustc-env={}={}",
        built.manifest_env_var(),
        built.manifest_path().display()
    );

    Ok(())
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

fn add_semantic_search_dependency(lean_root: &Path, manifest_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let semantic_root = semantic_search_root(lean_root)?;
    let dylib_path = semantic_search_dylib(&semantic_root)?;
    let mut manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let dependencies = manifest
        .get_mut("dependencies")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "capability manifest has no dependencies array",
            )
        })?;

    let dylib_string = dylib_path.display().to_string();
    let already_present = dependencies
        .iter()
        .filter_map(|dependency| dependency.get("dylib_path"))
        .filter_map(Value::as_str)
        .any(|existing| existing == dylib_string);
    if !already_present {
        dependencies.push(serde_json::json!({
            "name": "lean-semantic-search",
            "dylib_path": dylib_string,
            "export_symbols_for_dependents": true
        }));
    }
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn semantic_search_root(lean_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let lake_manifest: Value = serde_json::from_slice(&fs::read(lean_root.join("lake-manifest.json"))?)?;
    let packages = lake_manifest
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "lake-manifest.json has no packages array"))?;
    let package = packages
        .iter()
        .find(|package| {
            package
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == "«lean-semantic-search»")
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "lean-semantic-search package is missing"))?;
    let dir = package
        .get("dir")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "lean-semantic-search package has no dir"))?;
    Ok(fs::canonicalize(lean_root.join(dir))?)
}

fn semantic_search_dylib(semantic_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let extension = match std::env::var("CARGO_CFG_TARGET_OS")?.as_str() {
        "macos" => "dylib",
        "linux" => "so",
        other => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("unsupported target_os `{other}`; only macos and linux are tested"),
            )
            .into());
        }
    };
    let dylib = semantic_root
        .join(".lake")
        .join("build")
        .join("lib")
        .join(format!("liblean_x2dsemantic_x2dsearch_LeanSemanticSearch.{extension}"));
    if !dylib.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "missing lean-semantic-search shared library {}; run `cd lean && lake build LeanDup`",
                dylib.display()
            ),
        )
        .into());
    }
    Ok(dylib)
}
