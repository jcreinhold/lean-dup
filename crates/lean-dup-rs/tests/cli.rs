use std::fs;
use std::path::PathBuf;
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

    for command in ["doctor", "index", "index-mathlib", "audit", "eval", "show", "diff"] {
        assert!(stdout.contains(command), "missing {command} in help:\n{stdout}");
    }
    assert!(
        !stdout.contains("perf"),
        "hidden perf command leaked into help:\n{stdout}"
    );
    assert!(
        !stdout.contains("cache-cleanup"),
        "hidden cache cleanup command leaked into help:\n{stdout}"
    );
}

#[test]
fn index_mathlib_help_has_no_standalone_mathlib_default() {
    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .args(["index-mathlib", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("--workspace"));
    assert!(!stdout.contains("/Users/jcreinhold/Code/mathlib4"));
}

#[test]
fn hidden_perf_fixture_workload_emits_json_metrics() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let assert = Command::cargo_bin("lean-dup-rs")
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
fn doctor_json_reports_cache_lifecycle_diagnostics() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup-rs")
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
    assert_eq!(payload["status"], "ok");
    assert_eq!(
        payload["cache"]["cache_root"].as_str().unwrap(),
        cache.path().to_string_lossy()
    );
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

    let dry_run = Command::cargo_bin("lean-dup-rs")
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

    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["cache-cleanup", "--execute"])
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: true"))
        .stdout(predicate::str::contains("removable entries: 1"));

    assert!(active.exists());
    assert!(!stale.exists());
}

#[test]
fn eval_default_prints_compact_metrics_table() {
    let _worker = worker_cli_lock();
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
    let _worker = worker_cli_lock();
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
            .any(|recall| recall["k"] == 10 && recall["found"].as_u64() == recall["total"].as_u64())
    );
    assert_eq!(payload["metrics"]["hard_negative_hits"]["found"], 0);
}

#[test]
fn eval_hard_negatives_json_reports_positive_and_hard_negative_denominators() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup-rs")
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
}

#[test]
fn eval_output_writes_artifact_and_keeps_stdout_valid() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let assert = Command::cargo_bin("lean-dup-rs")
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
    assert_eq!(stdout_payload["metrics"]["suite"], artifact_payload["metrics"]["suite"]);
}

#[test]
fn doctor_reports_workspace_facts_from_repo_root() {
    let _worker = worker_cli_lock();
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
        .stdout(predicate::str::contains("cache fingerprint: rust-cli-cache.v1:"));
}

#[test]
fn doctor_respects_cache_dir_override() {
    let _worker = worker_cli_lock();
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
    let _worker = worker_cli_lock();
    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .args(["--progress", "--profile", "audit", "--workspace"])
        .arg(repo_root())
        .args(["--format", "json"])
        .assert()
        .success()
        .stderr(predicate::str::contains("["))
        .stderr(predicate::str::contains("workspace"))
        .stderr(predicate::str::contains("profile.workspace.resolve"));

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "audit");
    assert_eq!(payload["status"], "ok");
    assert!(payload["review"]["groups"].is_array());
    let first_group = payload["review"]["groups"].as_array().unwrap().first().unwrap();
    assert!(first_group["id"].as_str().unwrap().contains('-'));
    assert!(!first_group["id"].as_str().unwrap().starts_with("review-"));
    assert!(first_group["evidence"].is_array());
    assert!(payload.get("kind").is_none());
    assert!(!stdout.contains("feature_row"));
    assert!(!stdout.contains("declaration_row"));
    assert!(!stdout.contains("probe_result"));
}

#[test]
fn review_profiles_filter_one_ranked_audit_result() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup-rs")
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
            "--review-profile",
            "api-design",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let counts = &payload["profile_counts"];

    assert_eq!(payload["review_profile"], "api-design");
    assert_eq!(payload["visible_group_count"], counts["api_design"]);
    assert!(counts["mathlib"].as_u64().unwrap() <= counts["internal"].as_u64().unwrap());
    assert!(counts["internal"].as_u64().unwrap() <= counts["api_design"].as_u64().unwrap());
    assert!(counts["api_design"].as_u64().unwrap() <= counts["noise"].as_u64().unwrap());
    assert_eq!(
        payload["review"]["diagnostics"]["emitted_groups"].as_u64().unwrap(),
        payload["review"]["groups"].as_array().unwrap().len() as u64
    );
}

#[test]
fn audit_json_includes_stable_report_explanations() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(payload["report_schema_version"], "lean-dup.report.v1");
    assert_eq!(
        payload["explanations"]["visible_queue"]["visible"],
        payload["visible_group_count"]
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

    let assert = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["audit", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--no-semantic-probes"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("report schema: lean-dup.report.v1"));
    assert!(stdout.contains("visible queue:"));
    assert!(stdout.contains("hidden groups: total="));
    assert!(stdout.contains("probe summary: semantic probes disabled"));
    assert!(stdout.contains("comparison provenance: no comparison indexes"));
    assert!(!stdout.contains("feature_row"));
    assert!(!stdout.contains("declaration_row"));
    assert!(!stdout.contains("probe_result"));
}

#[test]
fn json_stdout_stays_clean_with_progress_and_profile() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let tiny = repo_root().join("tests/fixtures/tiny");

    let assert = Command::cargo_bin("lean-dup-rs")
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

    let index = Command::cargo_bin("lean-dup-rs")
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
    assert_eq!(payload["comparison_provenance"][0]["evidence_mode"], "static");
    let groups = payload["review"]["groups"].as_array().unwrap();
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

    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["index", "--workspace"])
        .arg(&source_backed)
        .args(["--module", "External", "--label", "linked"])
        .assert()
        .success();

    let assert = Command::cargo_bin("lean-dup-rs")
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
            "--review-profile",
            "api-design",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(payload["comparison_provenance"][0]["evidence_mode"], "proof-grade");
    assert!(payload["semantic_verification"]["planned_pairs"].as_u64().unwrap() > 0);
    assert!(payload["semantic_verification"]["verified_results"].as_u64().unwrap() > 0);
    let groups = payload["review"]["groups"].as_array().unwrap();
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
fn show_renders_evidence_blockers_probe_hint_and_callers_for_group() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");
    let audit = Command::cargo_bin("lean-dup-rs")
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
            "--review-profile",
            "api-design",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(audit.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let group_id = payload["review"]["groups"][0]["id"].as_str().unwrap();

    let show = Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["show", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--group", group_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("command: show"))
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
}

#[test]
fn baseline_diff_reports_appeared_disappeared_and_changed_groups() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let root = repo_root();
    let tiny = root.join("tests/fixtures/tiny");

    let audit = Command::cargo_bin("lean-dup-rs")
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

    Command::cargo_bin("lean-dup-rs")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args(["diff", "--workspace"])
        .arg(&tiny)
        .args(["--module", "Tiny", "--baseline", "before"])
        .assert()
        .success()
        .stdout(predicate::str::contains("command: diff"))
        .stdout(predicate::str::contains("appeared: 1"))
        .stdout(predicate::str::contains("disappeared: 1"))
        .stdout(predicate::str::contains("changed: 1"));
}

#[test]
fn index_builds_canonical_sqlite_and_reuses_cache() {
    let _worker = worker_cli_lock();
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
    assert_eq!(PathBuf::from(&index_path).parent().unwrap(), PathBuf::from(&index_dir));
    assert!(!PathBuf::from(&index_dir).join("declarations.jsonl.gz").exists());
    assert!(!PathBuf::from(&index_dir).join("buckets.sqlite").exists());
    assert!(!PathBuf::from(&index_dir).join("fixture.metadata.json").exists());

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
        .stdout(predicate::str::contains(format!("index path: {index_path}")));
}

fn line_value(text: &str, prefix: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing `{prefix}` in:\n{text}"))
        .to_owned()
}
