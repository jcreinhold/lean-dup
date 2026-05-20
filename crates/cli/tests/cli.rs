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
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

#[test]
fn help_lists_foundation_commands() {
    let assert = Command::cargo_bin("lean-dup").unwrap().arg("--help").assert().success();
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
    assert!(
        !stdout.contains("embedding"),
        "hidden embedding command leaked into help:\n{stdout}"
    );
}

#[test]
fn hidden_embedding_prepare_cache_only_reports_not_prepared() {
    let cache = tempfile::TempDir::new().unwrap();
    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .args([
            "embedding",
            "prepare",
            "--policy",
            "cache-only",
            "--format",
            "json",
            "--cache-root",
        ])
        .arg(cache.path())
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["command"], "embedding-prepare");
    assert_eq!(payload["status"], "warning");
    assert_eq!(payload["model_id"], "BAAI/bge-small-en-v1.5");
    assert_eq!(payload["profile_id"], "bge-small-en-v1.5");
    assert_eq!(payload["backend_family"], "fastembed");
    assert_eq!(payload["acquisition_policy"], "cache-only");
    assert_eq!(payload["cache_status"], "not-prepared");
    let required_files = payload["required_files"].as_array().unwrap();
    assert!(required_files.iter().any(|file| file["role"] == "runtime-model"));
    assert!(required_files.iter().any(|file| file["role"] == "tokenizer"));
    assert!(!stdout.contains("snapshots"));
    assert!(!stdout.contains("blobs"));
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
    for removed in [
        "--threshold",
        "--include-imports",
        "--import-root",
        "--min-priority",
        "--replacement-hints",
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
        .stdout(predicate::str::contains("executed: true"))
        .stdout(predicate::str::contains("removable entries: 1"));

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
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["scorer_version"], "lean-dup.symbolic-scorer.v1");
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
    assert_eq!(stdout_payload["metrics"]["suite"], artifact_payload["metrics"]["suite"]);
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
    assert_eq!(dataset["scoring"]["version"], "lean-dup.symbolic-scorer.v1");
    assert_eq!(dataset["scoring"]["variant"], "symbolic-only");
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
fn eval_hidden_vector_search_mode_writes_skipped_artifact_without_model() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let model_cache = tempfile::TempDir::new().unwrap();
    let text_cache = tempfile::TempDir::new().unwrap();
    let corpus_cache = tempfile::TempDir::new().unwrap();
    let artifact = repo_root().join("target/search-quality/default-vector-search.json");
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
            "--write-vector-search",
            "--vector-acquisition",
            "cache-only",
            "--vector-eligibility",
            "actionable-public-statement",
            "--vector-profile-id",
            "bge-small-en-v1.5",
            "--vector-input-format",
            "asymmetric-query-document",
            "--vector-model-cache-root",
        ])
        .arg(model_cache.path())
        .arg("--vector-text-cache-root")
        .arg(text_cache.path())
        .arg("--vector-corpus-cache-root")
        .arg(corpus_cache.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["vector_search_status"], "skipped");
    assert_eq!(
        payload["vector_search_artifact"],
        "target/search-quality/default-vector-search.json"
    );

    let vector: Value = serde_json::from_str(&fs::read_to_string(&artifact).unwrap()).unwrap();
    assert_eq!(vector["schema_version"], "lean-dup.vector-search.v2");
    assert_eq!(vector["suite"], "default");
    assert_eq!(vector["status"], "skipped");
    assert_eq!(vector["reason"], "vector-model-not-prepared");
    assert_eq!(vector["vector_candidates"]["status"], "skipped");
    assert_eq!(vector["vector_candidates"]["acquisition_policy"], "cache-only");
    assert_eq!(vector["vector_candidates"]["model_id"], "BAAI/bge-small-en-v1.5");
    assert_eq!(vector["vector_candidates"]["model_profile_id"], "bge-small-en-v1.5");
    assert_eq!(
        vector["vector_candidates"]["input_format_id"],
        "asymmetric-query-document"
    );
    assert_eq!(
        vector["vector_candidates"]["input_format_version"],
        "lean-dup.embedding-input-format.v1"
    );
    assert_eq!(
        vector["vector_candidates"]["query_eligibility"]["policy_id"],
        "actionable-public-statement"
    );
    assert_eq!(
        vector["vector_candidates"]["corpus_eligibility"]["policy_id"],
        "actionable-public-statement"
    );
    assert_eq!(vector["vector_candidates"]["top_k"], 32);
    assert!(vector["vector_candidates"]["eligible_corpus_size"].is_number());
    assert!(vector["vector_candidates"]["top_k_saturated"].is_boolean());
    assert!(vector["vector_stage_metrics"]["vector_top_k_recall"].is_object());
    assert!(vector["vector_stage_metrics"]["vector_top_k_precision"].is_object());
    assert!(vector["vector_stage_metrics"]["top_k_saturation"].is_object());
    assert!(vector["vector_stage_metrics"]["vector_only_positives"].is_object());
    assert!(vector["symbolic_baseline"]["stage_metrics"].is_object());
    assert_eq!(vector["validation_bounds"]["max_declarations"], 5000);
    assert_eq!(vector["validation_bounds"]["max_queries"], 1000);
    assert!(vector["validation_cost"]["phase_runtimes"].is_array());
    assert!(
        vector["validation_cost"]["peak_rss_bytes"].is_number()
            || vector["validation_cost"]["peak_rss_bytes"].is_null()
    );
    assert!(vector["validation_cost"]["model_cache_bytes"].is_number());
    assert!(vector["validation_cost"]["text_vector_cache_bytes"].is_number());
    assert!(vector["validation_cost"]["vector_corpus_bytes"].is_number());
    assert_eq!(vector["validation_cost"]["top_k"], 32);

    let raw = fs::read_to_string(artifact).unwrap();
    let forbidden_artifact_text = [
        "/Users/",
        "statement_text",
        "model.safetensors",
        "snapshot",
        "sqlite",
        "posting",
        "worker JSONL",
        "tensor",
        "FastEmbed",
        "LanceDB",
        "Qdrant",
        "sqlite-vec",
        "HNSW",
        "ANN",
        "\"table\"",
        "\"row\"",
        "graph",
        "layer",
        "neighbor",
        "lancedb",
        "table_name",
        "FeatureMatch",
        "IndexQuery",
    ];
    for forbidden in forbidden_artifact_text
        .into_iter()
        .map(str::to_owned)
        .chain([["query", ":"].concat(), ["passage", ":"].concat()])
    {
        assert!(!raw.contains(&forbidden), "vector search artifact leaked {forbidden}");
    }
}

#[test]
fn eval_hidden_vector_fixture_writes_non_saturated_artifact_and_reuses_corpus() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let model_cache = tempfile::TempDir::new().unwrap();
    let text_cache = tempfile::TempDir::new().unwrap();
    let corpus_cache = tempfile::TempDir::new().unwrap();
    let artifact = repo_root().join("target/search-quality/vector-fixture-vector-search.json");
    let _ = fs::remove_file(&artifact);

    for expected_corpus_status in ["built", "reused"] {
        let assert = Command::cargo_bin("lean-dup")
            .unwrap()
            .env("LEAN_DUP_CACHE_DIR", cache.path())
            .args([
                "eval",
                "--suite",
                "vector-fixture",
                "--format",
                "json",
                "--write-vector-search",
                "--vector-acquisition",
                "cache-only",
                "--vector-eligibility",
                "actionable-public-statement",
                "--vector-profile-id",
                "fixture-deterministic-v1",
                "--vector-input-format",
                "symmetric-document",
                "--vector-model-cache-root",
            ])
            .arg(model_cache.path())
            .arg("--vector-text-cache-root")
            .arg(text_cache.path())
            .arg("--vector-corpus-cache-root")
            .arg(corpus_cache.path())
            .assert()
            .success();

        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let payload: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(payload["suite"], "vector-fixture");
        assert_eq!(payload["vector_search_status"], "ok");
        assert_eq!(
            payload["vector_search_artifact"],
            "target/search-quality/vector-fixture-vector-search.json"
        );

        let vector: Value = serde_json::from_str(&fs::read_to_string(&artifact).unwrap()).unwrap();
        assert_eq!(vector["schema_version"], "lean-dup.vector-search.v2");
        assert_eq!(vector["suite"], "vector-fixture");
        assert_eq!(vector["status"], "ok");
        assert_eq!(vector["vector_candidates"]["status"], "ok");
        assert_eq!(vector["vector_candidates"]["corpus_status"], expected_corpus_status);
        assert_eq!(vector["vector_candidates"]["top_k"], 32);
        assert_eq!(vector["vector_candidates"]["eligible_corpus_size"], 72);
        assert_eq!(vector["vector_candidates"]["query_declaration_count"], 72);
        assert_eq!(vector["vector_candidates"]["top_k_saturated"], false);
        assert_eq!(vector["validation_bounds"]["max_declarations"], 5000);
        assert_eq!(vector["validation_bounds"]["max_queries"], 1000);
        assert_eq!(vector["validation_cost"]["eligible_corpus_size"], 72);
        assert_eq!(vector["validation_cost"]["query_count"], 72);
        assert_eq!(vector["validation_cost"]["top_k"], 32);
        assert_eq!(vector["validation_cost"]["top_k_saturated"], false);
        assert_eq!(vector["validation_cost"]["corpus_status"], expected_corpus_status);
        assert!(
            vector["validation_cost"]["phase_runtimes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|phase| { phase["phase"] == "total-vector-validation" && phase["elapsed_ms"].is_number() })
        );
        assert!(vector["validation_cost"]["warm_open_query_ms"].is_number());
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["generated"],
            1
        );
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["private"],
            1
        );
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["synthetic"],
            1
        );
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["low-signal"],
            1
        );
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["missing-statement"],
            1
        );
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["not-actionable"],
            1
        );
        assert_eq!(
            vector["vector_candidates"]["query_eligibility"]["skipped_by_reason"]["unsupported-kind"],
            1
        );
        assert_eq!(vector["vector_stage_metrics"]["top_k_saturation"]["found"], 0);
        assert_eq!(vector["vector_stage_metrics"]["top_k_saturation"]["total"], 72);
        assert_eq!(vector["vector_stage_metrics"]["vector_only_positives"]["found"], 1);
        assert_eq!(vector["vector_stage_metrics"]["symbolic_only_positives"]["found"], 1);
        assert_eq!(vector["vector_stage_metrics"]["vector_only_hard_negatives"]["found"], 1);
        assert!(vector["vector_stage_metrics"]["vector_top_k_recall"]["total"].is_number());
        assert!(vector["vector_stage_metrics"]["vector_top_k_precision"]["total"].is_number());

        let pairs = vector["pairs"].as_array().unwrap();
        let unique_pairs = pairs
            .iter()
            .map(|pair| {
                let mut names = [
                    pair["left"].as_str().unwrap().to_owned(),
                    pair["right"].as_str().unwrap().to_owned(),
                ];
                names.sort();
                names
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            pairs.len(),
            unique_pairs.len(),
            "artifact pair rows are not deduplicated"
        );
        assert!(pairs.iter().any(|pair| {
            pair["label_status"] == "positive"
                && pair["vector_generated"] == true
                && pair["symbolic_generated"] == false
        }));
        assert!(pairs.iter().any(|pair| {
            pair["label_status"] == "positive"
                && pair["symbolic_generated"] == true
                && pair["vector_generated"] == false
        }));
        assert!(
            pairs
                .iter()
                .any(|pair| { pair["label_status"] == "hard-negative" && pair["vector_generated"] == true })
        );
        assert!(
            pairs
                .iter()
                .flat_map(|pair| pair["label_facts"].as_array().unwrap())
                .any(|fact| fact["status"] == "expanded-positive")
        );
        assert!(
            pairs
                .iter()
                .flat_map(|pair| pair["label_facts"].as_array().unwrap())
                .any(|fact| fact["status"] == "expanded-hard-negative")
        );

        let raw = fs::read_to_string(&artifact).unwrap();
        let forbidden_artifact_text = [
            "/Users/",
            "statement_text",
            "model.safetensors",
            "snapshot",
            "sqlite",
            "posting",
            "worker JSONL",
            "tensor",
            "FastEmbed",
            "LanceDB",
            "Qdrant",
            "sqlite-vec",
            "HNSW",
            "ANN",
            "\"table\"",
            "\"row\"",
            "graph",
            "layer",
            "neighbor",
            "lancedb",
            "table_name",
            "FeatureMatch",
            "IndexQuery",
        ];
        for forbidden in forbidden_artifact_text
            .into_iter()
            .map(str::to_owned)
            .chain([["query", ":"].concat(), ["passage", ":"].concat()])
        {
            assert!(!raw.contains(&forbidden), "vector fixture artifact leaked {forbidden}");
        }
    }
}

#[test]
fn eval_hidden_vector_fixture_budget_exceeded_writes_partial_artifact() {
    let _worker = worker_cli_lock();
    let cache = tempfile::TempDir::new().unwrap();
    let model_cache = tempfile::TempDir::new().unwrap();
    let text_cache = tempfile::TempDir::new().unwrap();
    let corpus_cache = tempfile::TempDir::new().unwrap();
    let artifact = repo_root().join("target/search-quality/vector-fixture-vector-search.json");
    let _ = fs::remove_file(&artifact);

    let assert = Command::cargo_bin("lean-dup")
        .unwrap()
        .env("LEAN_DUP_CACHE_DIR", cache.path())
        .args([
            "eval",
            "--suite",
            "vector-fixture",
            "--format",
            "json",
            "--write-vector-search",
            "--vector-acquisition",
            "cache-only",
            "--vector-profile-id",
            "fixture-deterministic-v1",
            "--vector-input-format",
            "symmetric-document",
            "--vector-max-declarations",
            "10",
            "--vector-model-cache-root",
        ])
        .arg(model_cache.path())
        .arg("--vector-text-cache-root")
        .arg(text_cache.path())
        .arg("--vector-corpus-cache-root")
        .arg(corpus_cache.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["vector_search_status"], "budget-exceeded");
    assert_eq!(
        payload["vector_search_artifact"],
        "target/search-quality/vector-fixture-vector-search.json"
    );

    let vector: Value = serde_json::from_str(&fs::read_to_string(&artifact).unwrap()).unwrap();
    assert_eq!(vector["schema_version"], "lean-dup.vector-search.v2");
    assert_eq!(vector["status"], "budget-exceeded");
    assert!(
        vector["reason"]
            .as_str()
            .unwrap()
            .starts_with("vector-validation-budget-exceeded:max-declarations")
    );
    assert_eq!(vector["validation_bounds"]["max_declarations"], 10);
    assert_eq!(vector["validation_cost"]["query_count"], 79);
    assert_eq!(vector["validation_cost"]["eligible_corpus_size"], 79);
    assert_eq!(vector["pairs"].as_array().unwrap().len(), 0);

    let raw = fs::read_to_string(&artifact).unwrap();
    for forbidden in [
        "/Users/".to_owned(),
        ["query", ":"].concat(),
        ["passage", ":"].concat(),
        "FastEmbed".to_owned(),
        "LanceDB".to_owned(),
        "\"table\"".to_owned(),
        "\"row\"".to_owned(),
    ] {
        assert!(!raw.contains(&forbidden), "partial vector artifact leaked {forbidden}");
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
    assert_eq!(ablations["scorer_version"], "lean-dup.symbolic-scorer.v1");
    assert_eq!(
        ablations["semantic_reranking"]["version"],
        "lean-dup.semantic-reranking.v1"
    );
    assert!(ablations["semantic_obligation_yield"].is_array());
    let variants = ablations["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 8);
    for expected in [
        "symbolic-only",
        "vector-evidence-only",
        "symbolic-plus-vector",
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
    Command::cargo_bin("lean-dup")
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
    Command::cargo_bin("lean-dup")
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
    let assert = Command::cargo_bin("lean-dup")
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

    let assert = Command::cargo_bin("lean-dup")
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

    let assert = Command::cargo_bin("lean-dup")
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
            "--review-profile",
            "api-design",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(audit.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    let group_id = payload["review"]["groups"][0]["id"].as_str().unwrap();

    let show = Command::cargo_bin("lean-dup")
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

fn line_value(text: &str, prefix: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing `{prefix}` in:\n{text}"))
        .to_owned()
}
