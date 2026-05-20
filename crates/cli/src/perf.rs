use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cli::{PerfArgs, PerfWorkload};
use crate::error::{AppError, Result};
use lean_dup_diagnostics::perf::{summarize, with_collection};
use lean_dup_report::{PerfReport, PerfWorkloadReport};

pub fn run(args: PerfArgs) -> Result<PerfReport> {
    let cache_root = args
        .cache_root
        .clone()
        .unwrap_or_else(|| repo_root().join("target/lean-dup-perf/cache"));
    let output_path = args.output.clone();
    let report = run_workload(args, &cache_root)?;
    let response = PerfReport {
        status: "ok",
        workload: report.workload.clone(),
        cache_root,
        report,
    };
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| AppError::Io {
                message: "could not create directory",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&path, serde_json::to_string_pretty(&response)?).map_err(|source| AppError::Io {
            message: "could not write file",
            path,
            source,
        })?;
    }
    Ok(response)
}

fn run_workload(args: PerfArgs, cache_root: &Path) -> Result<PerfWorkloadReport> {
    prepare_cache(args.workload, cache_root)?;
    let command = workload_command(&args);
    let cache_state = cache_state(args.workload).to_owned();
    let _guard = EnvGuard::set("LEAN_DUP_CACHE_DIR", cache_root.as_os_str().to_owned());
    let started = Instant::now();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let (exit_code, snapshot) =
        with_collection(|| crate::run(command.iter().map(String::as_str), &mut stdout, &mut stderr));
    let elapsed_ms = started.elapsed().as_millis();
    let stdout_text = String::from_utf8_lossy(&stdout);
    let parsed = serde_json::from_str::<serde_json::Value>(&stdout_text).ok();
    let candidate_count = json_u64(parsed.as_ref(), "/retrieval/candidate_count");
    let review_groups = json_u64(parsed.as_ref(), "/review/group_count");
    let visible_groups = json_u64(parsed.as_ref(), "/visible_group_count");
    let semantic_planned_pairs = json_u64(parsed.as_ref(), "/semantic_verification/planned_pairs");
    let semantic_cached_hits = json_u64(parsed.as_ref(), "/semantic_verification/cached_hits");
    let semantic_worker_pairs = json_u64(parsed.as_ref(), "/semantic_verification/worker_pairs");
    let semantic_unavailable_results = json_u64(parsed.as_ref(), "/semantic_verification/unavailable_results");
    let hydrated_declarations = snapshot
        .events
        .iter()
        .filter(|event| event.name == "sqlite.hydrate.declarations")
        .filter_map(|event| event.count)
        .sum::<u64>();
    let hydrated_declarations = (hydrated_declarations > 0).then_some(hydrated_declarations);
    let probe_batches = snapshot
        .events
        .iter()
        .filter(|event| event.name == "worker.probe.batch")
        .filter_map(|event| event.count)
        .sum::<u64>();
    let probe_pairs = snapshot
        .events
        .iter()
        .filter(|event| event.name == "worker.probe.pairs")
        .filter_map(|event| event.count)
        .sum::<u64>();
    let summary = summarize(&snapshot);
    let profile_timings_ms = profile_timings(&stderr);

    Ok(PerfWorkloadReport {
        workload: args.workload.as_str().to_owned(),
        command,
        cache_state,
        exit_code,
        elapsed_ms,
        peak_memory_bytes: lean_dup_diagnostics::perf::peak_rss_bytes(),
        stdout_bytes: stdout.len(),
        stderr_bytes: stderr.len(),
        stdout_tail: text_tail(&stdout),
        stderr_tail: text_tail(&stderr),
        candidate_count,
        hydrated_declarations,
        review_groups,
        visible_groups,
        semantic_planned_pairs,
        semantic_cached_hits,
        semantic_worker_pairs,
        semantic_unavailable_results,
        probe_batches: (probe_batches > 0).then_some(probe_batches),
        probe_pairs: (probe_pairs > 0).then_some(probe_pairs),
        profile_timings_ms,
        events: snapshot.events,
        summary,
    })
}

fn json_u64(payload: Option<&serde_json::Value>, pointer: &str) -> Option<u64> {
    payload
        .and_then(|payload| payload.pointer(pointer))
        .and_then(serde_json::Value::as_u64)
}

fn profile_timings(stderr: &[u8]) -> BTreeMap<String, u128> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("profile.")?;
            let (phase, value) = rest.split_once('=')?;
            let millis = value.strip_suffix("ms")?.parse().ok()?;
            Some((phase.to_owned(), millis))
        })
        .collect()
}

fn text_tail(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let start = bytes.len().saturating_sub(2000);
    Some(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

fn prepare_cache(workload: PerfWorkload, cache_root: &Path) -> Result<()> {
    if matches!(workload, PerfWorkload::ColdMathlibIndex) && cache_root.exists() {
        fs::remove_dir_all(cache_root).map_err(|source| AppError::Io {
            message: "could not remove directory",
            path: cache_root.to_path_buf(),
            source,
        })?;
    }
    fs::create_dir_all(cache_root).map_err(|source| AppError::Io {
        message: "could not create directory",
        path: cache_root.to_path_buf(),
        source,
    })
}

fn cache_state(workload: PerfWorkload) -> &'static str {
    match workload {
        PerfWorkload::ColdMathlibIndex => "cold",
        PerfWorkload::WarmMathlibIndex => "warm",
        PerfWorkload::ManualTargetedMathlib
        | PerfWorkload::ManualFullNoMathlib
        | PerfWorkload::ManualFullMathlibNoProbes
        | PerfWorkload::ManualFullMathlib
        | PerfWorkload::FixtureAudit => "reuse-or-build",
    }
}

fn workload_command(args: &PerfArgs) -> Vec<String> {
    let manual = args
        .manual_workspace
        .clone()
        .unwrap_or_else(|| PathBuf::from("MANUAL-WORKSPACE-NOT-SET"));
    let manual_module = args.manual_module.clone().unwrap_or_else(|| "Workspace".to_owned());
    let manual_module_targeted = args
        .manual_module
        .clone()
        .unwrap_or_else(|| "Workspace.Targeted".to_owned());
    let repo = repo_root();
    let mut command = vec!["lean-dup".to_owned(), "--profile".to_owned()];
    match args.workload {
        PerfWorkload::ColdMathlibIndex | PerfWorkload::WarmMathlibIndex => {
            command.extend([
                "index-mathlib".to_owned(),
                "--workspace".to_owned(),
                manual.display().to_string(),
            ]);
            if let Some(mathlib) = &args.mathlib_workspace {
                command.extend(["--mathlib-workspace".to_owned(), mathlib.display().to_string()]);
            }
            if args.workload == PerfWorkload::ColdMathlibIndex {
                command.push("--force".to_owned());
            }
        }
        PerfWorkload::ManualTargetedMathlib => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                manual.display().to_string(),
                "--module".to_owned(),
                manual_module_targeted,
                "--compare-mathlib".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);
            if let Some(mathlib) = &args.mathlib_workspace {
                command.extend(["--mathlib-workspace".to_owned(), mathlib.display().to_string()]);
            }
        }
        PerfWorkload::ManualFullNoMathlib => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                manual.display().to_string(),
                "--module".to_owned(),
                manual_module.clone(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);
        }
        PerfWorkload::ManualFullMathlib => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                manual.display().to_string(),
                "--module".to_owned(),
                manual_module.clone(),
                "--compare-mathlib".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);
            if let Some(mathlib) = &args.mathlib_workspace {
                command.extend(["--mathlib-workspace".to_owned(), mathlib.display().to_string()]);
            }
        }
        PerfWorkload::ManualFullMathlibNoProbes => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                manual.display().to_string(),
                "--module".to_owned(),
                manual_module,
                "--compare-mathlib".to_owned(),
                "--no-semantic-probes".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);
            if let Some(mathlib) = &args.mathlib_workspace {
                command.extend(["--mathlib-workspace".to_owned(), mathlib.display().to_string()]);
            }
        }
        PerfWorkload::FixtureAudit => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                repo.join("tests/fixtures/tiny").display().to_string(),
                "--module".to_owned(),
                "Tiny".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
                "--no-semantic-probes".to_owned(),
            ]);
        }
    }
    command
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::{PerfArgs, PerfFormat, PerfWorkload};

    use lean_dup_diagnostics::perf::{CostClass, PerfSnapshot, measure, record_count, summarize};

    use super::{profile_timings, workload_command};

    #[test]
    fn summarizes_duration_and_counts_by_stable_names() {
        let (_, snapshot) = super::with_collection(|| {
            measure(CostClass::SqliteIndex, "sqlite.test", || ());
            record_count(CostClass::SqliteIndex, "sqlite.rows", 3);
            record_count(CostClass::SqliteIndex, "sqlite.rows", 4);
        });
        let summary = summarize(&snapshot);

        assert!(summary.elapsed_ms_by_class.contains_key(&CostClass::SqliteIndex));
        assert_eq!(summary.counts_by_name["sqlite.rows"], 7);
    }

    #[test]
    fn empty_snapshot_has_empty_summary() {
        let summary = summarize(&PerfSnapshot::default());

        assert!(summary.elapsed_ms_by_class.is_empty());
        assert!(summary.counts_by_name.is_empty());
    }

    #[test]
    fn full_mathlib_no_probes_workload_disables_semantic_probes() {
        let command = workload_command(&PerfArgs {
            workload: PerfWorkload::ManualFullMathlibNoProbes,
            format: PerfFormat::Json,
            output: None,
            cache_root: None,
            manual_workspace: Some(std::path::PathBuf::from("/tmp/manual")),
            manual_module: None,
            mathlib_workspace: None,
        });

        assert!(command.contains(&"--compare-mathlib".to_owned()));
        assert!(command.contains(&"--no-semantic-probes".to_owned()));
        assert!(
            command
                .windows(2)
                .any(|window| window[0] == "--module" && window[1] == "Workspace")
        );
    }

    #[test]
    fn parses_profile_timings_from_stderr() {
        let timings = profile_timings(b"profile.retrieval=123ms\nignored\nprofile.report.render=4ms\n");

        assert_eq!(timings["retrieval"], 123);
        assert_eq!(timings["report.render"], 4);
    }
}
