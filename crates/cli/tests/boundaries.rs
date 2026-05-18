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

fn rust_files_under(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).expect("read directory") {
            let entry = entry.expect("read entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files
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
            for forbidden in dependencies
                .iter()
                .filter(|dependency| dependency.starts_with("lean-dup-"))
            {
                assert!(
                    forbidden == "lean-dup-diagnostics",
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
        "pub mod audit",
        "pub mod observation",
        "pub mod retrieval",
        "pub mod ranking",
        "pub mod semantic_verification",
        "pub mod source_refs",
        "pub mod replacement_hints",
        "pub use retrieval",
        "pub use semantic_verification",
        "pub use baseline",
        "retrieve_candidates",
        "rank_candidates",
        "CandidateExplanation",
        "KeyContribution",
        "RetrievalOutput",
        "ScorerConfig",
        "ScorerWeights",
        "ScorerThresholds",
    ] {
        assert!(!search_lib.contains(forbidden), "search exposes {forbidden}");
    }
    let search_audit = fs::read_to_string(root.join("crates/search/src/audit.rs")).expect("read search audit facade");
    for forbidden in [
        "pub use crate::ranking",
        "pub use crate::retrieval",
        "pub use crate::semantic_verification",
        "pub group: RankedGroup",
        "pub review: RankedReview",
        "pub retrieval: RetrievalDiagnostics",
        "pub semantic_verification: ProbeDiagnostics",
    ] {
        assert!(
            !search_audit.contains(forbidden),
            "search audit facade exposes {forbidden}"
        );
    }
    for allowed_public_name in [
        "pub struct AuditReview",
        "pub struct AuditGroup",
        "pub struct AuditRetrievalSummary",
        "pub struct AuditProbeSummary",
        "pub struct SearchBaselineDiff",
    ] {
        assert!(
            search_audit.contains(allowed_public_name),
            "search audit facade is missing {allowed_public_name}"
        );
    }

    let index_lib = fs::read_to_string(root.join("crates/index/src/lib.rs")).expect("read index lib");
    for forbidden in [
        "pub mod index",
        "pub mod cache",
        "pub mod cache_lifecycle",
        "pub mod external_provenance",
        "pub use index::*",
        "Posting",
        "IndexQuery",
        "FingerprintQuery",
        "RoleFeatureQuery",
        "FeatureMatch,",
        "FeatureMatchCount",
    ] {
        assert!(!index_lib.contains(forbidden), "index exposes {forbidden}");
    }

    let eval_lib = fs::read_to_string(root.join("crates/eval/src/lib.rs")).expect("read eval lib");
    assert!(!eval_lib.contains("pub mod eval"), "eval exposes old eval module");
    assert!(
        !eval_lib.contains("peak_rss_bytes"),
        "eval exposes runtime memory measurement"
    );

    let worker_lib = fs::read_to_string(root.join("crates/worker/src/lib.rs")).expect("read worker lib");
    assert!(
        !worker_lib.contains("pub use worker::*"),
        "worker exposes a wildcard facade"
    );
    for allowed_stable_dto in ["IndexStreamItem", "WorkerEvent", "WorkerDiagnostic"] {
        assert!(
            worker_lib.contains(allowed_stable_dto),
            "worker stable DTO allowlist is missing {allowed_stable_dto}"
        );
    }

    let project_lib = fs::read_to_string(root.join("crates/project/src/lib.rs")).expect("read project lib");
    for forbidden in ["pub mod workspace", "pub mod mathlib"] {
        assert!(!project_lib.contains(forbidden), "project exposes {forbidden}");
    }

    let report_lib = fs::read_to_string(root.join("crates/report/src/lib.rs")).expect("read report lib");
    for forbidden in ["pub mod render", "pub mod reports"] {
        assert!(!report_lib.contains(forbidden), "report exposes {forbidden}");
    }
    assert!(
        !report_lib.contains("pub mod report_contract"),
        "report exposes report_contract as an implementation module"
    );

    let reports = fs::read_to_string(root.join("crates/report/src/reports.rs")).expect("read report DTOs");
    for forbidden in [
        "RankedGroup",
        "RankedReview",
        "ReviewFilter",
        "RetrievalDiagnostics",
        "ProbeDiagnostics",
        "EvaluationReport",
        "pub retrieval: RetrievalDiagnostics",
        "pub semantic_verification: ProbeDiagnostics",
        "pub review: RankedReview",
        "pub group: RankedGroup",
        "pub cache: CacheDiagnostics,",
        "CacheCleanup(CacheCleanupReport)",
        "Eval(EvalOutput)",
    ] {
        assert!(!reports.contains(forbidden), "report DTO exposes {forbidden}");
    }
    let report_contract =
        fs::read_to_string(root.join("crates/report/src/report_contract.rs")).expect("read report contract");
    for forbidden in [
        "RankedGroup",
        "RankedReview",
        "ReviewFilter",
        "RetrievalDiagnostics",
        "ProbeDiagnostics",
    ] {
        assert!(
            !report_contract.contains(forbidden),
            "report contract depends on search internals: {forbidden}"
        );
    }

    for path in rust_files_under(&root.join("crates")) {
        if path.ends_with("crates/cli/tests/boundaries.rs") {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("read rust file");
        let display = path.display();
        if !path.starts_with(root.join("crates/project")) {
            for forbidden in ["lean_dup_project::workspace", "lean_dup_project::mathlib"] {
                assert!(!contents.contains(forbidden), "{display} imports {forbidden}");
            }
        }
        if !path.starts_with(root.join("crates/report")) {
            for forbidden in ["lean_dup_report::render::", "lean_dup_report::reports::"] {
                assert!(!contents.contains(forbidden), "{display} imports {forbidden}");
            }
        }
        if !path.starts_with(root.join("crates/search")) {
            for forbidden in ["lean_dup_search::audit::", "lean_dup_search::observation::"] {
                assert!(!contents.contains(forbidden), "{display} imports {forbidden}");
            }
        }
    }
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
