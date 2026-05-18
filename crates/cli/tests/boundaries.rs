#![allow(clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli crate lives under crates/cli")
        .to_path_buf()
}

fn crate_manifests() -> BTreeMap<String, toml::Value> {
    let root = repo_root().join("crates");
    let mut manifests = BTreeMap::new();
    for entry in fs::read_dir(root).expect("read crates directory") {
        let entry = entry.expect("read crates entry");
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.exists() {
            continue;
        }
        let contents = fs::read_to_string(&manifest_path).expect("read manifest");
        let parsed = toml::from_str::<toml::Value>(&contents).expect("parse manifest");
        let name = parsed["package"]["name"].as_str().expect("package name").to_owned();
        manifests.insert(name, parsed);
    }
    manifests
}

fn dependency_names(manifest: &toml::Value) -> Vec<String> {
    manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn only_cli_depends_on_clap() {
    for (name, manifest) in crate_manifests() {
        let has_clap = dependency_names(&manifest)
            .iter()
            .any(|dependency| dependency == "clap");
        assert!(!has_clap || name == "lean-dup-cli", "{name} must not depend on clap");
    }
}

#[test]
fn dependency_direction_keeps_lower_crates_out_of_report_and_cli() {
    for (name, manifest) in crate_manifests() {
        let dependencies = dependency_names(&manifest);
        if name != "lean-dup-cli" {
            assert!(
                !dependencies.iter().any(|dependency| dependency == "lean-dup-cli"),
                "{name} must not depend on lean-dup-cli"
            );
        }
        if matches!(
            name.as_str(),
            "lean-dup-project" | "lean-dup-index" | "lean-dup-search" | "lean-dup-eval"
        ) {
            assert!(
                !dependencies.iter().any(|dependency| dependency == "lean-dup-report"),
                "{name} must not depend on lean-dup-report"
            );
        }
        if name == "lean-dup-worker" {
            assert!(
                !dependencies
                    .iter()
                    .any(|dependency| dependency.starts_with("lean-dup-")),
                "worker must not depend on another lean-dup crate"
            );
        }
        if name == "lean-dup-diagnostics" {
            for forbidden in [
                "lean-dup-project",
                "lean-dup-index",
                "lean-dup-search",
                "lean-dup-eval",
                "lean-dup-report",
                "lean-dup-cli",
            ] {
                assert!(
                    !dependencies.iter().any(|dependency| dependency == forbidden),
                    "diagnostics must not depend on {forbidden}"
                );
            }
        }
    }
}

#[test]
fn old_file_shaped_modules_are_not_public_api() {
    let root = repo_root();
    let search_lib = fs::read_to_string(root.join("crates/search/src/lib.rs")).expect("read search lib");
    for forbidden in [
        "pub mod retrieval",
        "pub mod ranking",
        "pub mod semantic_verification",
        "pub mod source_refs",
        "pub mod replacement_hints",
    ] {
        assert!(!search_lib.contains(forbidden), "search exposes {forbidden}");
    }

    let index_lib = fs::read_to_string(root.join("crates/index/src/lib.rs")).expect("read index lib");
    for forbidden in [
        "pub mod index",
        "pub mod cache",
        "pub mod cache_lifecycle",
        "pub mod external_provenance",
    ] {
        assert!(!index_lib.contains(forbidden), "index exposes {forbidden}");
    }

    let eval_lib = fs::read_to_string(root.join("crates/eval/src/lib.rs")).expect("read eval lib");
    assert!(!eval_lib.contains("pub mod eval"), "eval exposes old eval module");
}

#[test]
fn removed_rs_suffix_does_not_return() {
    let root = repo_root();
    let mut stack = vec![root.clone()];
    let stale_binary = ["lean-dup", "-rs"].concat();
    let stale_crate = ["lean_dup", "_rs"].concat();
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).expect("read directory") {
            let entry = entry.expect("read entry");
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git" || name == "target") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("rs" | "toml" | "md" | "json")
            ) {
                continue;
            }
            let contents = fs::read_to_string(&path).expect("read text file");
            assert!(
                !contents.contains(&stale_binary) && !contents.contains(&stale_crate),
                "stale -rs reference in {}",
                path.display()
            );
        }
    }
}
