use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::{Mutex, MutexGuard, OnceLock};

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;

fn worker_cli_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("worker CLI test lock poisoned")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

#[cfg(unix)]
fn write_test_extension(directory: &Path, executable: &str, script: &str) -> PathBuf {
    let path = directory.join(executable);
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn path_with_extension_dir(directory: &Path) -> std::ffi::OsString {
    let mut paths = vec![directory.to_path_buf()];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(paths).unwrap()
}

#[test]
fn help_lists_foundation_commands() {
    let assert = Command::cargo_bin("lean-dup").unwrap().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for command in [
        "doctor",
        "cache-cleanup",
        "index",
        "index-mathlib",
        "audit",
        "eval",
        "show",
        "diff",
        "baseline",
    ] {
        assert!(stdout.contains(command), "missing {command} in help:\n{stdout}");
    }
    assert!(
        !stdout.contains("perf"),
        "hidden perf command leaked into help:\n{stdout}"
    );
    assert!(
        !stdout.contains("embedding"),
        "hidden embedding command leaked into help:\n{stdout}"
    );
    assert!(stdout.contains("--list"));
    assert!(
        !stdout.contains("vector"),
        "external vector command should not be hardcoded into static help:\n{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn cargo_style_external_extension_dispatches_unknown_command() {
    let temp = tempfile::TempDir::new().unwrap();
    write_test_extension(
        temp.path(),
        "lean-dup-vector",
        "#!/bin/sh\necho \"OUT:$*\"\necho \"ERR:$*\" >&2\nexit 23\n",
    );

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", path_with_extension_dir(temp.path()))
        .args(["vector", "validate", "--flag"])
        .assert()
        .code(23)
        .stdout(predicate::str::contains("OUT:validate --flag"))
        .stderr(predicate::str::contains("ERR:validate --flag"));
}

#[cfg(unix)]
#[test]
fn external_extension_receives_global_flags_before_extension_args() {
    let temp = tempfile::TempDir::new().unwrap();
    write_test_extension(
        temp.path(),
        "lean-dup-vector",
        "#!/bin/sh\necho \"OUT:$*\"\necho \"ERR:$*\" >&2\nexit 23\n",
    );

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", path_with_extension_dir(temp.path()))
        .args(["--progress", "--profile", "vector", "validate"])
        .assert()
        .code(23)
        .stdout(predicate::str::contains("OUT:--progress --profile validate"))
        .stderr(predicate::str::contains("ERR:--progress --profile validate"));
}

#[cfg(unix)]
#[test]
fn external_extension_help_dispatches_to_installed_tool() {
    let temp = tempfile::TempDir::new().unwrap();
    write_test_extension(
        temp.path(),
        "lean-dup-vector",
        "#!/bin/sh\necho \"VECTOR HELP:$*\"\nexit 0\n",
    );

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", path_with_extension_dir(temp.path()))
        .args(["vector", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("VECTOR HELP:--help"));
}

#[cfg(unix)]
#[test]
fn unknown_command_suggests_nearest_built_in() {
    let temp = tempfile::TempDir::new().unwrap();
    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", temp.path())
        .arg("audot")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown command `audot`"))
        .stderr(predicate::str::contains("did you mean `audit`"));
}

#[cfg(unix)]
#[test]
fn missing_vector_extension_reports_install_hint() {
    let temp = tempfile::TempDir::new().unwrap();

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", temp.path())
        .args(["vector", "validate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lean-dup-vector"))
        .stderr(predicate::str::contains("cargo install lean-dup-vector-search"));
}

#[cfg(unix)]
#[test]
fn invalid_external_names_are_rejected_without_execution() {
    let temp = tempfile::TempDir::new().unwrap();
    write_test_extension(
        temp.path(),
        "lean-dup-vector",
        "#!/bin/sh\necho SHOULD_NOT_RUN\nexit 23\n",
    );

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", path_with_extension_dir(temp.path()))
        .arg("./vector")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid external command name"))
        .stdout(predicate::str::is_empty());
}

#[cfg(unix)]
#[test]
fn built_in_commands_shadow_external_extensions() {
    let temp = tempfile::TempDir::new().unwrap();
    write_test_extension(temp.path(), "lean-dup-audit", "#!/bin/sh\necho SHADOW_AUDIT\nexit 77\n");

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", path_with_extension_dir(temp.path()))
        .args(["audit", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: lean-dup audit"))
        .stdout(predicate::str::contains("--workspace"))
        .stdout(predicate::str::contains("SHADOW_AUDIT").not());
}

#[cfg(unix)]
#[test]
fn list_reports_installed_external_extensions_without_hardcoding_them() {
    let temp = tempfile::TempDir::new().unwrap();
    write_test_extension(temp.path(), "lean-dup-vector", "#!/bin/sh\nexit 0\n");
    write_test_extension(temp.path(), "lean-dup-audit", "#!/bin/sh\nexit 0\n");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("PATH", path_with_extension_dir(temp.path()))
        .arg("--list")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("lean-dup commands:"));
    assert!(stdout.contains("  audit"));
    assert!(stdout.contains("installed extensions:"));
    assert!(stdout.contains("  vector"));
    assert!(!stdout.contains("lean-dup-vector"));
    assert!(!stdout.contains("lean-dup-audit"));
}

#[test]
fn index_mathlib_help_has_no_standalone_mathlib_default() {
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["index-mathlib", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("--workspace"));
    // Confirm there is no hardcoded mathlib path baked into the help text.
    assert!(!stdout.contains("/Users/"));
}

#[test]
fn audit_help_omits_removed_noop_flags_and_rejects_them() {
    let help = Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["audit", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(help.get_output().stdout.clone()).unwrap();
    for present in [
        "--show-private-actionable",
        "--low-priority",
        "--diagnostics",
        "--visibility",
    ] {
        assert!(
            stdout.contains(present),
            "audit help did not mention visibility flag {present}"
        );
    }
    for removed_visibility in ["--public-only", "--include-private", "--no-include-private"] {
        assert!(
            !stdout.contains(removed_visibility),
            "collapsed visibility flag {removed_visibility} still appears in audit help"
        );
    }
    for removed in [
        "--threshold",
        "--include-imports",
        "--import-root",
        "--min-priority",
        "--replacement-hints",
        "--review-profile",
        "--show-noise",
    ] {
        assert!(
            !stdout.contains(removed),
            "removed flag {removed} appeared in audit help"
        );
    }

    let tiny = repo_root().join("tests/fixtures/tiny");
    Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["audit", "--workspace"])
        .arg(tiny)
        .args(["--threshold", "0.8"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["audit", "--workspace"])
        .arg(repo_root().join("tests/fixtures/tiny"))
        .args(["--review-profile", "noise"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn hidden_perf_fixture_workload_emits_json_metrics() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["perf", "--workload", "fixture-audit", "--cache-root"])
        .arg(cache.path())
        .args(["--output"])
        .arg(output.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "perf");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["report"]["workload"], "fixture-audit");
    assert!(payload["report"]["elapsed_ms"].as_u64().unwrap() > 0);
    assert!(payload["report"]["events"].as_array().unwrap().len() > 1);
    assert!(output.path().exists());
}

#[test]
fn version_reports_release_identity_without_workspace() {
    Command::cargo_bin("lean-dup")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("lean-dup 0.1.0"))
        .stdout(predicate::str::contains("git revision:"))
        .stdout(predicate::str::contains("build profile:"))
        .stdout(predicate::str::contains("report schema: lean-dup.report.v3"))
        .stdout(predicate::str::contains("index schema: lean-dup.index.v2"))
        .stdout(predicate::str::contains("cache key: rust-cli-cache.v1"))
        .stdout(predicate::str::contains("doctor --workspace <workspace>"));
}

#[test]
fn doctor_json_reports_cache_lifecycle_diagnostics() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["doctor", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "doctor");
    assert_eq!(payload["report_schema_version"], "lean-dup.report.v3");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["release"]["version"], "0.1.0");
    assert_eq!(payload["release"]["report_schema_version"], "lean-dup.report.v3");
    assert_eq!(payload["release"]["index_schema_version"], "lean-dup.index.v2");
    assert_eq!(payload["release"]["cache_key_version"], "rust-cli-cache.v1");
    assert_eq!(payload["worker"]["protocol_version"], "lean-dup.worker.v1");
    assert_eq!(payload["worker"]["worker_version"], "0.1.0");
    assert!(
        payload["worker"]["supported_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "version")
    );
    assert_eq!(payload["cache"]["cache_root"]["kind"], "cache-root");
    assert!(
        payload["cache"]["cache_root"]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(!stdout.contains(cache.path().to_string_lossy().as_ref()));
    assert!(!stdout.contains("index.sqlite"));
    let labels = payload["cache"]["labels"].as_array().unwrap();
    assert!(labels.iter().any(|label| label["label"] == "audit-workspace"));
    let audit_workspace = labels.iter().find(|label| label["label"] == "audit-workspace").unwrap();
    assert_eq!(audit_workspace["latest"]["status"], "missing");
    assert!(
        audit_workspace["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["expected_current"] == true && entry["status"] == "missing" })
    );
}

#[test]
fn hidden_cache_cleanup_dry_run_and_execute_preserve_latest_entry() {
    let cache = tempfile::TempDir::new().unwrap();
    let label_dir = cache.path().join("indexes/fixture");
    let active = label_dir.join("active");
    let stale = label_dir.join("stale");
    fs::create_dir_all(&active).unwrap();
    fs::create_dir_all(&stale).unwrap();
    fs::write(active.join("index.sqlite"), "active").unwrap();
    fs::write(stale.join("index.sqlite"), "stale").unwrap();
    fs::write(
        label_dir.join("latest.json"),
        serde_json::to_string(&serde_json::json!({ "index_dir": &active })).unwrap(),
    )
    .unwrap();

    let dry_run = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["cache-cleanup", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(dry_run.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "cache-cleanup");
    assert_eq!(payload["executed"], false);
    assert_eq!(payload["removable_count"], 1);
    assert!(active.exists());
    assert!(stale.exists());

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["cache-cleanup", "--execute"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed"))
        .stdout(predicate::str::contains("1 entries to remove"));

    assert!(active.exists());
    assert!(!stale.exists());
}

#[test]
fn eval_default_prints_compact_metrics_table() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
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
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "default", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "eval");
    assert_eq!(payload["report_schema_version"], "lean-dup.report.v3");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["scorer_version"], "lean-dup.symbolic-scorer.v2");
    assert_eq!(payload["metrics"]["suite"], "default");
    assert!(
        payload["metrics"]["recall"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recall| recall["k"] == 10 && recall["found"].as_u64() == recall["total"].as_u64())
    );
    assert_eq!(payload["metrics"]["hard_negative_hits"]["found"], 0);
    assert!(payload["metrics"]["stage_metrics"].is_object());
    assert_eq!(
        payload["metrics"]["stage_metrics"]["candidate_generation_recall"]["total"],
        payload["metrics"]["recall"]
            .as_array()
            .unwrap()
            .iter()
            .find(|recall| recall["k"] == 10)
            .unwrap()["total"]
    );
    assert!(
        payload["metrics"]["stage_metrics"]["candidate_count_by_feature_family"]
            .as_object()
            .unwrap()
            .contains_key("statement_fingerprint")
    );
    assert!(
        payload["metrics"]["stage_metrics"]["generated_candidate_count_by_policy"]
            .as_object()
            .unwrap()
            .contains_key("local_duplicate_audit")
    );
}

#[test]
fn eval_hard_negatives_json_reports_positive_and_hard_negative_denominators() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "hard-negatives", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "eval");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["metrics"]["suite"], "hard-negatives");
    assert!(
        payload["metrics"]["recall"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recall| recall["total"].as_u64().unwrap() > 0)
    );
    assert!(payload["metrics"]["hard_negative_hits"]["total"].as_u64().unwrap() > 0);
    assert_eq!(payload["metrics"]["hard_negative_hits"]["found"], 0);
    assert!(
        payload["metrics"]["stage_metrics"]["hard_negative_survival"]["candidate_generation"]["total"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        payload["metrics"]["stage_metrics"]["hard_negative_survival"]["visible_queue"]["found"],
        0
    );
}

#[test]
fn eval_output_writes_artifact_and_keeps_stdout_valid() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "default", "--format", "json", "--output"])
        .arg(output.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stdout_payload: Value = serde_json::from_str(&stdout).unwrap();
    let artifact_payload: Value = serde_json::from_str(&fs::read_to_string(output.path()).unwrap()).unwrap();
    assert_eq!(stdout_payload["command"], "eval");
    assert_eq!(artifact_payload["command"], "eval");
    let output_path = output.path().to_string_lossy();
    assert_eq!(stdout_payload["artifact_path"].as_str().unwrap(), output_path.as_ref());
    assert_eq!(
        artifact_payload["artifact_path"].as_str().unwrap(),
        output_path.as_ref()
    );
    assert_eq!(stdout_payload["metrics"]["suite"], artifact_payload["metrics"]["suite"]);
}

#[test]
fn eval_production_gate_json_reports_manual_prerequisite_blockers() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "production-gate", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["status"], "incomplete");
    let manual = payload["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["suite"] == "manual-internal")
        .expect("manual internal run");
    assert_eq!(manual["status"], "skipped");
    assert_eq!(manual["manual_prerequisites"]["workspace"]["status"], "missing");
    assert_eq!(manual["manual_prerequisites"]["labels"]["status"], "ok");
    assert!(
        manual["manual_prerequisites"]["next_command"]
            .as_str()
            .unwrap()
            .contains("--workspace <manual-workspace>")
    );
    assert!(
        manual["reason"]
            .as_str()
            .unwrap()
            .contains("missing required --workspace")
    );
}

#[test]
fn eval_manual_suite_without_workspace_reports_structured_skip() {
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["eval", "--suite", "manual-internal", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["status"], "skipped");
    assert_eq!(payload["manual_prerequisites"]["workspace"]["status"], "missing");
    assert_eq!(payload["manual_prerequisites"]["labels"]["status"], "ok");
    assert_eq!(payload["runs"][0]["status"], "skipped");
    assert!(
        payload["runs"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("missing required --workspace")
    );
}

#[test]
fn eval_hidden_search_dataset_mode_writes_feature_artifact() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let artifact = repo_root().join("target/search-quality/default-dataset.json");
    let _ = fs::remove_file(&artifact);

    let help = Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["eval", "--help"])
        .assert()
        .success();
    let help_stdout = String::from_utf8(help.get_output().stdout.clone()).unwrap();
    assert!(!help_stdout.contains("--write-search-dataset"));
    assert!(!help_stdout.contains("--write-scorer-ablations"));
    assert!(!help_stdout.contains("--write-vector-search"));
    assert!(!help_stdout.contains("--vector-acquisition"));
    assert!(!help_stdout.contains("--vector-profile-id"));
    assert!(!help_stdout.contains("--vector-input-format"));
    assert!(!help_stdout.contains("--vector-document-policy"));
    assert!(!help_stdout.contains("--vector-eligibility"));
    assert!(!help_stdout.contains("--vector-max-declarations"));
    assert!(!help_stdout.contains("--vector-max-queries"));
    assert!(!help_stdout.contains("--vector-max-runtime-ms"));
    assert!(!help_stdout.contains("--vector-max-rss-bytes"));
    assert!(!help_stdout.contains("vector-fixture"));

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args([
            "eval",
            "--suite",
            "default",
            "--format",
            "json",
            "--write-search-dataset",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        payload["search_dataset_artifact"],
        "target/search-quality/default-dataset.json"
    );

    let dataset: Value = serde_json::from_str(&fs::read_to_string(&artifact).unwrap()).unwrap();
    assert_eq!(dataset["schema_version"], "lean-dup.search-dataset.v1");
    assert_eq!(dataset["suite"], "default");
    assert_eq!(dataset["scoring"]["version"], "lean-dup.symbolic-scorer.v2");
    assert_eq!(dataset["scoring"]["variant"], "all-features");
    assert_eq!(
        dataset["review_policy"]["version"],
        "lean-dup.symbolic-review-policy.v2"
    );
    assert_eq!(
        dataset["semantic_reranking"]["version"],
        "lean-dup.semantic-reranking.v1"
    );
    assert!(dataset["semantic_obligation_yield"].is_array());
    let pairs = dataset["pairs"].as_array().unwrap();
    assert!(!pairs.is_empty());
    assert!(pairs.iter().any(|pair| pair["label"].is_object()));
    assert!(pairs.iter().any(|pair| pair["label_status"] == "unlabeled"));
    let first = &pairs[0];
    assert!(first["stage_position"].is_object());
    assert!(first["stage_position"]["generated"].is_boolean());
    assert!(first["stage_position"]["ranked"].is_boolean());
    assert!(first["final_visibility"].is_object());
    assert!(first["features"]["retrieval_feature_families"].is_array());
    assert_eq!(
        first["features"]["semantic_reranking"]["version"],
        "lean-dup.semantic-reranking.v1"
    );
    assert!(first["features"]["semantic_evidence_state"].is_string());
    assert!(first["features"]["semantic_obligations"].is_array());

    let raw = fs::read_to_string(artifact).unwrap();
    for forbidden in ["/Users/", "statement_text", "IndexQuery", "FeatureMatch", "sqlite"] {
        assert!(!raw.contains(forbidden), "dataset leaked {forbidden}");
    }
}

#[test]
fn eval_hidden_scorer_ablation_mode_writes_variant_artifact() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let artifact = repo_root().join("target/search-quality/default-scorer-ablations.json");
    let _ = fs::remove_file(&artifact);

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args([
            "eval",
            "--suite",
            "default",
            "--format",
            "json",
            "--write-scorer-ablations",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        payload["scorer_ablation_artifact"],
        "target/search-quality/default-scorer-ablations.json"
    );

    let ablations: Value = serde_json::from_str(&fs::read_to_string(&artifact).unwrap()).unwrap();
    assert_eq!(ablations["schema_version"], "lean-dup.scorer-ablation.v1");
    assert_eq!(ablations["scorer_version"], "lean-dup.symbolic-scorer.v2");
    assert_eq!(ablations["review_policy_version"], "lean-dup.symbolic-review-policy.v2");
    assert_eq!(
        ablations["semantic_reranking"]["version"],
        "lean-dup.semantic-reranking.v1"
    );
    assert!(ablations["semantic_obligation_yield"].is_array());
    let variants = ablations["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 6);
    for expected in [
        "all-features",
        "no-role-features",
        "no-connective-conclusion-features",
        "no-source-module-features",
        "no-static-evidence-features",
        "semantic-evidence-only-rerank",
    ] {
        assert!(
            variants.iter().any(|variant| variant["variant"] == expected),
            "missing {expected} in {variants:?}"
        );
    }
    for variant in variants {
        assert_eq!(
            variant["semantic_reranking"]["version"],
            "lean-dup.semantic-reranking.v1"
        );
        assert!(variant["semantic_obligation_yield"].is_array());
    }
}

#[test]
fn doctor_reports_workspace_facts_from_repo_root() {
    let _worker = worker_cli_lock();
    let root = repo_root();
    // Default view: triaged summary. Headline, workspace+lean line, cache root,
    // problems section, totals line with cleanup hint.
    Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["doctor", "--workspace"])
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("lean-dup doctor — status: "))
        .stdout(predicate::str::contains("workspace: workspace-root sha256:"))
        .stdout(predicate::str::contains("cache root: cache-root sha256:"))
        .stdout(predicate::str::contains("cache:"))
        .stdout(predicate::str::contains("totals:"))
        .stdout(predicate::str::contains("lean:"));

    // --verbose adds the per-entry detail plus the previously-headlined facts
    // (schema versions, worker protocol, module roots, cache fingerprint).
    Command::cargo_bin("lean-dup")
        .unwrap()
        .args(["doctor", "--workspace"])
        .arg(&root)
        .arg("--verbose")
        .assert()
        .success()
        .stdout(predicate::str::contains("verbose detail:"))
        .stdout(predicate::str::contains("report schema: lean-dup.report.v3"))
        .stdout(predicate::str::contains("module roots: LeanDup"))
        .stdout(predicate::str::contains("worker commands:"))
        .stdout(predicate::str::contains("cache fingerprint: rust-cli-cache.v1:"));
}

#[test]
fn doctor_respects_cache_dir_override() {
    let _worker = worker_cli_lock();
    let temp = tempfile::TempDir::new().unwrap();
    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", temp.path())
        .args(["doctor", "--workspace"])
        .arg(repo_root())
        .assert()
        .success()
        .stdout(predicate::str::contains("cache root: cache-root sha256:"))
        .stdout(predicate::str::contains(temp.path().to_string_lossy().as_ref()).not());
}

#[test]
fn audit_json_keeps_progress_and_profile_off_stdout() {
    let _worker = worker_cli_lock();
    let assert = Command::cargo_bin("lean-dup")
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
    assert!(payload["review"]["groups"].is_null());
    assert!(payload["review"]["group_count"].as_u64().unwrap() >= payload["visible_groups_emitted"].as_u64().unwrap());
    assert!(payload["visible_groups"].is_array());
    assert!(!stdout.contains(repo_root().to_string_lossy().as_ref()));
    assert!(!stdout.contains("pruned_postings"));
    assert_eq!(
        payload["review_policy"]["version"],
        "lean-dup.symbolic-review-policy.v2"
    );
    if let Some(first_group) = payload["visible_groups"].as_array().unwrap().first() {
        assert!(first_group["id"].as_str().unwrap().contains('-'));
        assert!(!first_group["id"].as_str().unwrap().starts_with("review-"));
        assert_eq!(first_group["family_id"], first_group["id"]);
        assert!(first_group["pair_count"].as_u64().unwrap() >= 1);
        assert!(first_group["pair_ids"].is_array());
        assert!(first_group["pair_evidence"].is_array());
        assert!(first_group["pair_evidence_truncated"].is_boolean());
        assert!(first_group["evidence"].is_array());
        if let Some(first_member) = first_group["members"].as_array().unwrap().first()
            && !first_member["source_span"].is_null()
        {
            assert_eq!(first_member["source_span"]["file"]["kind"], "workspace-root");
            assert!(
                first_member["source_span"]["file"]["fingerprint"]
                    .as_str()
                    .unwrap()
                    .starts_with("sha256:")
            );
        }
    } else {
        assert_eq!(payload["visible_group_count"], 0);
        assert!(
            payload["explanations"]["visible_queue"]["reason"]
                .as_str()
                .unwrap()
                .contains("No ranked groups pass")
        );
    }
    assert!(payload.get("kind").is_none());
    assert!(!stdout.contains("feature_row"));
    assert!(!stdout.contains("declaration_row"));
    assert!(!stdout.contains("probe_result"));
}

#[test]
fn composable_visibility_flags_filter_ranked_audit_results() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--low-priority",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let counts = &payload["queue_counts"];

    assert_eq!(payload["options"]["visibility"]["include_low_priority"], true);
    assert_eq!(payload["options"]["visibility"]["include_private"], false);
    assert_eq!(payload["options"]["visibility"]["diagnostics"], false);
    assert_eq!(payload["visible_group_count"], counts["with_low_priority"]);
    assert!(counts["cleanup"].as_u64().unwrap() <= counts["with_private"].as_u64().unwrap());
    assert!(counts["cleanup"].as_u64().unwrap() <= counts["with_low_priority"].as_u64().unwrap());
    assert!(counts["with_low_priority"].as_u64().unwrap() <= counts["diagnostics"].as_u64().unwrap());
    assert!(payload["review"]["groups"].is_null());
    assert!(payload["review"]["group_count"].is_u64());
    assert_eq!(
        payload["visible_groups_emitted"].as_u64().unwrap(),
        payload["visible_groups"].as_array().unwrap().len() as u64
    );
    assert!(payload["visible_groups_emitted"].as_u64().unwrap() <= payload["visible_group_limit"].as_u64().unwrap());
}

#[test]
fn audit_visibility_flags_compose_in_json_options() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--show-private-actionable",
            "--low-priority",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["options"]["visibility"]["include_private"], true);
    assert_eq!(payload["options"]["visibility"]["include_low_priority"], true);
    assert_eq!(payload["options"]["visibility"]["diagnostics"], false);

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--diagnostics",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["options"]["visibility"]["include_private"], false);
    assert_eq!(payload["options"]["visibility"]["include_low_priority"], false);
    assert_eq!(payload["options"]["visibility"]["diagnostics"], true);
    assert_eq!(payload["visible_group_count"], payload["queue_counts"]["diagnostics"]);
}

#[test]
fn audit_visibility_public_excludes_private_corpus() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let all = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--format", "json"])
        .assert()
        .success();
    let all_payload: Value = serde_json::from_str(&String::from_utf8(all.get_output().stdout.clone()).unwrap()).unwrap();
    assert_eq!(all_payload["options"]["include_private"], true);

    let public = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--visibility",
            "public",
        ])
        .assert()
        .success();
    let public_payload: Value =
        serde_json::from_str(&String::from_utf8(public.get_output().stdout.clone()).unwrap()).unwrap();
    assert_eq!(public_payload["options"]["include_private"], false);
}

#[test]
fn audit_json_includes_stable_report_explanations() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(payload["report_schema_version"], "lean-dup.report.v3");
    assert_eq!(
        payload["explanations"]["visible_queue"]["visible"],
        payload["visible_group_count"]
    );
    assert_eq!(
        payload["explanations"]["visible_queue"]["emitted"],
        payload["visible_groups_emitted"]
    );
    assert!(
        payload["explanations"]["visible_queue"]["reason"]
            .as_str()
            .unwrap()
            .contains("groups match")
    );
    assert!(payload["explanations"]["hidden_groups"]["total"].is_u64());
    assert_eq!(
        payload["explanations"]["semantic_probes"]["summary"],
        "semantic probes disabled"
    );
    assert_eq!(
        payload["explanations"]["comparison_provenance"]["summary"],
        "no comparison indexes"
    );
}

#[test]
fn audit_text_reports_queue_probe_and_provenance_explanations() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    // Default (triaged) view: header + groups table; no provenance section.
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("lean-dup audit — status:"));
    assert!(stdout.contains("review queue:"));
    assert!(!stdout.contains("report schema:"), "verbose-only line in default output:\n{stdout}");
    assert!(!stdout.contains("probe summary:"), "verbose-only line in default output:\n{stdout}");
    assert!(!stdout.contains("comparison provenance:"), "verbose-only line in default output:\n{stdout}");
    assert!(!stdout.contains("feature_row"));
    assert!(!stdout.contains("declaration_row"));
    assert!(!stdout.contains("probe_result"));

    // --verbose: full provenance + per-group detail (strict superset of default).
    let verbose = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--verbose"])
        .assert()
        .success();
    let verbose_stdout = String::from_utf8(verbose.get_output().stdout.clone()).unwrap();
    assert!(verbose_stdout.contains("verbose detail:"));
    assert!(verbose_stdout.contains("report schema: lean-dup.report.v3"));
    assert!(verbose_stdout.contains("visible queue:"));
    assert!(verbose_stdout.contains("hidden groups: total="));
    assert!(verbose_stdout.contains("probe summary: semantic probes disabled"));
    assert!(verbose_stdout.contains("comparison provenance: no comparison indexes"));
}

#[test]
fn json_stdout_stays_clean_with_progress_and_profile() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["--progress", "--profile", "audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(payload["command"], "audit");
    assert!(stderr.contains("profile."));
    assert!(!stdout.contains("profile."));
    assert!(!stdout.contains("progress."));
}

#[test]
fn audit_fixture_mathlib_label_produces_actionable_hints() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let external = root.join("tests/fixtures/external");
    let tiny = root.join("tests/fixtures/tiny");

    let index = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&external)
        .args(["--module", "External", "--label", "mathlib"])
        .assert()
        .success();
    let index_stdout = String::from_utf8(index.get_output().stdout.clone()).unwrap();
    let index_path = line_value(&index_stdout, "index path: ");
    let connection = Connection::open(index_path).unwrap();
    connection
        .execute("DELETE FROM metadata WHERE key = 'provenance_json'", [])
        .unwrap();
    drop(connection);

    let assert = Command::cargo_bin("lean-dup")
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
    assert_eq!(payload["comparison_provenance"][0]["evidence_mode"], "static");
    let groups = payload["visible_groups"].as_array().unwrap();
    let exact = groups
        .iter()
        .find(|group| {
            group["recommended_action"] == "already-in-mathlib"
                && group["replacement_hint"]["target_decl"] == "External.same_as_tiny"
        })
        .expect("mathlib exact duplicate group");

    assert_eq!(exact["review_priority"], "high");
    assert_eq!(exact["evidence_mode"], "static");
    assert_eq!(exact["replacement_hint"]["target_decl"], "External.same_as_tiny");
    assert_eq!(exact["replacement_hint"]["import_status"], "missing");
    assert!(exact["replacement_hint"]["caller_count"].is_u64());
    assert!(
        exact["replacement_hint"]["displayed_callers"].as_array().unwrap().len()
            <= exact["replacement_hint"]["caller_count"].as_u64().unwrap() as usize
    );
}

#[test]
fn source_backed_external_index_gets_proof_grade_probe_evidence() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let source_backed = repo_root().join("tests/fixtures/source-backed");
    let _ = fs::remove_dir_all(source_backed.join(".lake"));
    let lake = ProcessCommand::new("lake")
        .arg("build")
        .current_dir(&source_backed)
        .output()
        .unwrap();
    assert!(
        lake.status.success(),
        "source-backed fixture lake build failed:\n{}{}",
        String::from_utf8_lossy(&lake.stderr),
        String::from_utf8_lossy(&lake.stdout)
    );

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&source_backed)
        .args(["--module", "External", "--label", "linked"])
        .assert()
        .success();

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&source_backed)
        .args([
            "--module",
            "Tiny",
            "--compare-index",
            "linked",
            "--format",
            "json",
            "--low-priority",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(payload["comparison_provenance"][0]["evidence_mode"], "proof-grade");
    assert!(payload["semantic_verification"]["planned_pairs"].as_u64().unwrap() > 0);
    assert!(payload["semantic_verification"]["verified_results"].as_u64().unwrap() > 0);
    let groups = payload["visible_groups"].as_array().unwrap();
    let exact = groups
        .iter()
        .find(|group| group["target_decl"] == "External.same_as_tiny")
        .expect("source-backed exact duplicate group");
    assert_eq!(exact["evidence_mode"], "proof-grade");
    assert!(
        exact["signals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|signal| signal == "probe:verified:exact-theorem")
    );
}

#[test]
fn audit_default_text_hides_noise_blockers() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("lean-dup audit — status:"));
    assert!(!stdout.contains("generated-declaration"));
    assert!(!stdout.contains("broad-head-only"));
}

#[test]
fn show_renders_evidence_blockers_probe_hint_and_callers_for_group() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");
    let audit = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--low-priority",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(audit.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let group_id = payload["visible_groups"][0]["id"].as_str().unwrap();

    let show = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["show", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--group", group_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("lean-dup show — status:"))
        .stdout(predicate::str::contains(format!("group: {group_id}")))
        .stdout(predicate::str::contains("evidence:"))
        .stdout(predicate::str::contains("explanation:"))
        .stdout(predicate::str::contains("why visible or hidden:"))
        .stdout(predicate::str::contains("static/proof evidence:"))
        .stdout(predicate::str::contains("semantic evidence:"))
        .stdout(predicate::str::contains("blockers:"))
        .stdout(predicate::str::contains("probe:"))
        .stdout(predicate::str::contains("replacement:"))
        .stdout(predicate::str::contains("replacement/import/callers:"))
        .stdout(predicate::str::contains("callers:"));
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains("feature_row"));
    assert!(!stdout.contains("declaration_row"));
    assert!(!stdout.contains("probe_result"));
    assert!(
        stdout.contains(tiny.to_string_lossy().as_ref()),
        "show should print absolute member paths so they can be opened directly: {stdout}"
    );
    assert!(stdout.contains(".lean:"), "show member lines should end with `.lean:<line>`");
}

#[test]
fn show_fast_fails_on_unknown_group_after_recent_audit() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");
    let audit = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--low-priority",
        ])
        .assert()
        .success();
    let payload: Value = serde_json::from_str(&String::from_utf8(audit.get_output().stdout.clone()).unwrap()).unwrap();
    let real_id = payload["visible_groups"][0]["id"].as_str().unwrap().to_owned();
    let suggestion_target = real_id
        .chars()
        .enumerate()
        .map(|(i, ch)| if i == real_id.len() - 1 { 'x' } else { ch })
        .collect::<String>();

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["show", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--group", &suggestion_target])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown audit group"))
        .stderr(predicate::str::contains(&real_id));
}

#[test]
fn baseline_diff_reports_appeared_disappeared_and_changed_groups() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    let audit = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--format", "json", "--save-baseline", "before"])
        .assert()
        .success();
    let stdout = String::from_utf8(audit.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let baseline_path = PathBuf::from(payload["saved_baseline"].as_str().unwrap());
    let mut baseline: Value = serde_json::from_str(&fs::read_to_string(&baseline_path).unwrap()).unwrap();
    let groups = baseline["groups"].as_array_mut().unwrap();
    assert!(groups.len() > 3);
    let mut fake_disappeared = groups[0].clone();
    fake_disappeared["id"] = Value::String("exact-statement-disappeared".to_owned());
    groups.remove(0);
    groups[0]["evidence_digest"] = Value::String("changed-in-test".to_owned());
    groups.push(fake_disappeared);
    fs::write(&baseline_path, serde_json::to_string_pretty(&baseline).unwrap()).unwrap();

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["diff", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--baseline", "before"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lean-dup diff — status:"))
        .stdout(predicate::str::contains("appeared: 1"))
        .stdout(predicate::str::contains("disappeared: 1"))
        .stdout(predicate::str::contains("changed: 1"));
}

#[test]
fn baseline_subcommand_lists_shows_and_deletes() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    for name in ["alpha", "beta"] {
        Command::cargo_bin("lean-dup")
            .unwrap()
            .env("LEAN_DUP_CACHE_DIR", cache.path())
            .args(["audit", "--workspace"])
            .arg(&tiny)
            .args(["--module", "Tiny", "--no-semantic-probes", "--format", "json", "--save-baseline", name])
            .assert()
            .success();
    }

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["baseline", "list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha"))
        .stdout(predicate::str::contains("beta"));

    let show = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["baseline", "show", "alpha", "--format", "json"])
        .assert()
        .success();
    let payload: Value = serde_json::from_str(&String::from_utf8(show.get_output().stdout.clone()).unwrap()).unwrap();
    assert_eq!(payload["command"], "baseline");
    assert_eq!(payload["action"], "show");
    let summaries = payload["baselines"].as_array().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0]["name"], "alpha");
    assert!(summaries[0]["group_count"].as_u64().unwrap() > 0);

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["baseline", "delete", "alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted baseline 'alpha'"));

    // Re-saving a baseline reports it as replacing the existing one.
    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--save-baseline", "beta"])
        .assert()
        .success()
        .stdout(predicate::str::contains("replaced existing"));

    // `baseline show` must dedup group IDs in its listing.
    let show = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["baseline", "show", "beta", "--verbose"])
        .assert()
        .success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.contains('-')
            && !trimmed.starts_with("lean-dup")
            && !trimmed.starts_with("name")
            && !trimmed.contains(' ')
            && trimmed.len() > 6
        {
            assert!(
                seen.insert(trimmed),
                "duplicate group id {trimmed:?} in baseline show output:\n{stdout}"
            );
        }
    }

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["baseline", "show", "alpha"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("baseline not found"));
}

#[test]
fn baseline_list_filters_by_current_workspace() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--no-semantic-probes", "--save-baseline", "tiny-one"])
        .assert()
        .success();

    // Plant a baseline file claiming a different workspace fingerprint, so
    // the filter has something to exclude.
    let planted_path = cache.path().join("baselines").join("from-elsewhere.json");
    let real = std::fs::read_to_string(cache.path().join("baselines").join("tiny-one.json")).unwrap();
    let mut payload: Value = serde_json::from_str(&real).unwrap();
    payload["workspace_fingerprint"] = Value::String("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned());
    std::fs::write(&planted_path, serde_json::to_string(&payload).unwrap()).unwrap();

    // Default `list` (filter by cwd workspace) hides the planted entry;
    // `--all` shows both.
    let scoped = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .current_dir(&tiny)
        .args(["baseline", "list", "--format", "json"])
        .assert()
        .success();
    let payload: Value =
        serde_json::from_str(&String::from_utf8(scoped.get_output().stdout.clone()).unwrap()).unwrap();
    let names: Vec<&str> = payload["baselines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"tiny-one"), "expected tiny-one in scoped list, got {names:?}");
    assert!(!names.contains(&"from-elsewhere"), "unexpected from-elsewhere in scoped list: {names:?}");

    let all = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .current_dir(&tiny)
        .args(["baseline", "list", "--all", "--format", "json"])
        .assert()
        .success();
    let payload: Value = serde_json::from_str(&String::from_utf8(all.get_output().stdout.clone()).unwrap()).unwrap();
    let names: Vec<&str> = payload["baselines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"tiny-one") && names.contains(&"from-elsewhere"), "expected both with --all, got {names:?}");
}

#[test]
fn diff_fast_path_skips_audit_pipeline_when_snapshot_is_fresh() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--format", "json", "--save-baseline", "fp"])
        .assert()
        .success();

    let fast = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["--progress", "diff", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--baseline", "fp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lean-dup diff — status:"));
    let fast_stderr = String::from_utf8(fast.get_output().stderr.clone()).unwrap();
    assert!(
        !fast_stderr.contains("progress.retrieval") && !fast_stderr.contains("progress.semantic"),
        "fast path should skip audit-pipeline phases; got stderr: {fast_stderr}"
    );

    let slow = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["--progress", "diff", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--baseline", "fp", "--no-cache"])
        .assert()
        .success();
    let slow_stderr = String::from_utf8(slow.get_output().stderr.clone()).unwrap();
    assert!(
        slow_stderr.contains("progress.retrieval") || slow_stderr.contains("progress.index"),
        "slow path should still emit audit-pipeline phases; got stderr: {slow_stderr}"
    );
}

#[test]
fn index_builds_canonical_sqlite_and_reuses_cache() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let external = repo_root().join("tests/fixtures/external");

    let first = Command::cargo_bin("lean-dup")
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
    assert_eq!(PathBuf::from(&index_path).parent().unwrap(), PathBuf::from(&index_dir));
    assert!(!PathBuf::from(&index_dir).join("declarations.jsonl.gz").exists());
    assert!(!PathBuf::from(&index_dir).join("buckets.sqlite").exists());
    assert!(!PathBuf::from(&index_dir).join("fixture.metadata.json").exists());

    let latest = fs::read_to_string(cache.path().join("indexes/fixture/latest.json")).unwrap();
    assert!(latest.contains(&index_dir));

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&external)
        .args(["--module", "External", "--label", "fixture"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: ok"))
        .stdout(predicate::str::contains("cache: hit"))
        .stdout(predicate::str::contains(format!("index path: {index_path}")));
}

#[test]
fn index_json_emits_canonical_payload() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let external = repo_root().join("tests/fixtures/external");

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&external)
        .args(["--module", "External", "--label", "fixture", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "index");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["label"], "fixture");
    assert!(payload["declaration_count"].as_u64().is_some());
}

#[test]
fn show_json_emits_canonical_payload() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let audit = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--low-priority",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(audit.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let group_id = payload["visible_groups"][0]["id"].as_str().unwrap().to_owned();

    let show = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["show", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--group", &group_id, "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "show");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["group"]["id"], group_id);
}

#[test]
fn diff_json_emits_canonical_payload() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args([
            "--module",
            "Tiny",
            "--no-semantic-probes",
            "--format",
            "json",
            "--save-baseline",
            "before",
        ])
        .assert()
        .success();

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["diff", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--baseline", "before", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "diff");
    assert_eq!(payload["status"], "ok");
    assert!(payload["diff"]["baseline"].is_string());
}

fn line_value(text: &str, prefix: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing `{prefix}` in:\n{text}"))
        .to_owned()
}
