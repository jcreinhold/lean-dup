use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate lives under repo/crates/lean-dup-rs")
        .to_path_buf()
}

#[test]
fn help_lists_foundation_commands() {
    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for command in [
        "doctor",
        "index",
        "index-mathlib",
        "audit",
        "eval",
        "show",
        "diff",
    ] {
        assert!(
            stdout.contains(command),
            "missing {command} in help:\n{stdout}"
        );
    }
}

#[test]
fn eval_default_prints_compact_metrics_table() {
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "default", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "suite\trecall@1\trecall@5\trecall@10\tqueue_precision",
        ))
        .stdout(predicate::str::contains("default\t"));

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("\thard_negatives\t"));
    assert!(stdout.contains("\t0/"));
}

#[test]
fn eval_default_json_contains_raw_metric_counts() {
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "default", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "eval");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["metrics"]["suite"], "default");
    assert!(
        payload["metrics"]["recall"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recall| recall["k"] == 10
                && recall["found"].as_u64() == recall["total"].as_u64())
    );
    assert_eq!(payload["metrics"]["hard_negative_hits"]["found"], 0);
}

#[test]
fn doctor_reports_workspace_facts_from_repo_root() {
    let root = repo_root();
    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .args(["doctor", "--workspace"])
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("command: doctor"))
        .stdout(predicate::str::contains(format!(
            "requested workspace: {}",
            root.display()
        )))
        .stdout(predicate::str::contains(format!(
            "resolved Lake root: {}",
            root.join("lean").display()
        )))
        .stdout(predicate::str::contains("module roots: LeanDup"))
        .stdout(predicate::str::contains("source files:"))
        .stdout(predicate::str::contains("cache root:"))
        .stdout(predicate::str::contains("lean:"))
        .stdout(predicate::str::contains(
            "cache fingerprint: rust-cli-cache.v1:",
        ));
}

#[test]
fn doctor_respects_cache_dir_override() {
    let temp = tempfile::TempDir::new().unwrap();
    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", temp.path())
        .args(["doctor", "--workspace"])
        .arg(repo_root())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "cache root: {}",
            temp.path().display()
        )));
}

#[test]
fn audit_json_keeps_progress_and_profile_off_stdout() {
    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .args(["--progress", "--profile", "audit", "--workspace"])
        .arg(repo_root())
        .args(["--format", "json"])
        .assert()
        .success()
        .stderr(predicate::str::contains("progress.workspace"))
        .stderr(predicate::str::contains("profile.workspace.resolve"));

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "audit");
    assert_eq!(payload["status"], "ok");
    assert!(payload["review"]["groups"].is_array());
    assert!(payload.get("kind").is_none());
    assert!(!stdout.contains("feature_row"));
    assert!(!stdout.contains("declaration_row"));
    assert!(!stdout.contains("probe_result"));
}

#[test]
fn audit_fixture_mathlib_label_produces_actionable_hints() {
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let external = root.join("tests/fixtures/external");
    let tiny = root.join("tests/fixtures/tiny");

    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&external)
        .args(["--module", "External", "--label", "mathlib"])
        .assert()
        .success();

    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--compare-index",
            "mathlib",
            "--no-semantic-probes",
            "--format",
            "json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let groups = payload["review"]["groups"].as_array().unwrap();
    let exact = groups
        .iter()
        .find(|group| {
            group["recommended_action"] == "already-in-mathlib"
                && group["replacement_hint"]["target_decl"] == "External.same_as_tiny"
        })
        .expect("mathlib exact duplicate group");

    assert_eq!(exact["review_priority"], "high");
    assert_eq!(
        exact["replacement_hint"]["target_decl"],
        "External.same_as_tiny"
    );
    assert_eq!(exact["replacement_hint"]["import_status"], "missing");
    assert!(exact["replacement_hint"]["caller_count"].as_u64().unwrap() > 0);
    assert!(
        !exact["replacement_hint"]["displayed_callers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn audit_default_text_hides_noise_blockers() {
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("command: audit"));
    assert!(!stdout.contains("generated-declaration"));
    assert!(!stdout.contains("broad-head-only"));
}

#[test]
fn skeleton_commands_return_stub_status_without_worker_rows() {
    let root = repo_root();
    for args in [
        vec![
            "show",
            "--workspace",
            root.to_str().unwrap(),
            "--group",
            "g1",
        ],
        vec![
            "diff",
            "--workspace",
            root.to_str().unwrap(),
            "--baseline",
            "baseline.json",
        ],
    ] {
        let assert = Command::cargo_bin("lean-dup-rs")
            .unwrap()
            .args(args)
            .assert()
            .success()
            .stdout(predicate::str::contains("status: stub"));
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(!stdout.contains("feature_row"));
        assert!(!stdout.contains("declaration_row"));
        assert!(!stdout.contains("probe_result"));
    }
}

#[test]
fn index_builds_canonical_sqlite_and_reuses_cache() {
    let cache = tempfile::TempDir::new().unwrap();
    let external = repo_root().join("tests/fixtures/external");

    let first = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&external)
        .args(["--module", "External", "--label", "fixture"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: ok"))
        .stdout(predicate::str::contains("cache: miss"))
        .stdout(predicate::str::contains("index path:"))
        .stdout(predicate::str::contains("declarations:"));
    let first_stdout = String::from_utf8(first.get_output().stdout.clone()).unwrap();
    let index_path = line_value(&first_stdout, "index path: ");
    let index_dir = line_value(&first_stdout, "index dir: ");

    assert!(PathBuf::from(&index_path).ends_with("index.sqlite"));
    assert!(PathBuf::from(&index_path).exists());
    assert_eq!(
        PathBuf::from(&index_path).parent().unwrap(),
        PathBuf::from(&index_dir)
    );
    assert!(
        !PathBuf::from(&index_dir)
            .join("declarations.jsonl.gz")
            .exists()
    );
    assert!(!PathBuf::from(&index_dir).join("buckets.sqlite").exists());
    assert!(
        !PathBuf::from(&index_dir)
            .join("fixture.metadata.json")
            .exists()
    );

    let latest = fs::read_to_string(cache.path().join("indexes/fixture/latest.json")).unwrap();
    assert!(latest.contains(&index_dir));

    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&external)
        .args(["--module", "External", "--label", "fixture"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: ok"))
        .stdout(predicate::str::contains("cache: hit"))
        .stdout(predicate::str::contains(format!(
            "index path: {index_path}"
        )));
}

fn line_value(text: &str, prefix: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing `{prefix}` in:\n{text}"))
        .to_owned()
}
