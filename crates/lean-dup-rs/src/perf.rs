use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cli::{PerfArgs, PerfWorkload};
use crate::error::{Error, Result};
use crate::eval::memory;

/// Runtime cost classes used by the internal performance harness.
///
/// Callers see stable cost categories, not the worker transport, SQLite schema,
/// cache layout, or retrieval data structures that currently produce them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CostClass {
    WorkerStartup,
    LeanImport,
    Transport,
    LeanSemantic,
    SqliteIndex,
    RetrievalRanking,
    Reporting,
    Harness,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PerfEvent {
    pub(crate) cost_class: CostClass,
    pub(crate) name: String,
    pub(crate) elapsed_ms: Option<u128>,
    pub(crate) count: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PerfSnapshot {
    pub(crate) events: Vec<PerfEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PerfSummary {
    pub(crate) elapsed_ms_by_class: BTreeMap<CostClass, u128>,
    pub(crate) counts_by_name: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PerfWorkloadReport {
    pub(crate) workload: PerfWorkload,
    pub(crate) command: Vec<String>,
    pub(crate) cache_state: String,
    pub(crate) exit_code: i32,
    pub(crate) elapsed_ms: u128,
    pub(crate) peak_memory_bytes: Option<u64>,
    pub(crate) stdout_bytes: usize,
    pub(crate) stderr_bytes: usize,
    pub(crate) stdout_tail: Option<String>,
    pub(crate) stderr_tail: Option<String>,
    pub(crate) candidate_count: Option<u64>,
    pub(crate) hydrated_declarations: Option<u64>,
    pub(crate) probe_batches: Option<u64>,
    pub(crate) probe_pairs: Option<u64>,
    pub(crate) events: Vec<PerfEvent>,
    pub(crate) summary: PerfSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PerfReport {
    pub(crate) status: &'static str,
    pub(crate) workload: PerfWorkload,
    pub(crate) cache_root: PathBuf,
    pub(crate) report: PerfWorkloadReport,
}

thread_local! {
    static COLLECTOR: RefCell<Option<PerfSnapshot>> = const { RefCell::new(None) };
}

pub(crate) fn measure<T>(class: CostClass, name: impl Into<String>, work: impl FnOnce() -> T) -> T {
    let name = name.into();
    let started = Instant::now();
    let result = work();
    record_duration(class, name, started.elapsed());
    result
}

pub(crate) fn measure_result<T, E>(
    class: CostClass,
    name: impl Into<String>,
    work: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let name = name.into();
    let started = Instant::now();
    let result = work();
    record_duration(class, name, started.elapsed());
    result
}

pub(crate) fn record_duration(class: CostClass, name: impl Into<String>, duration: Duration) {
    record_event(PerfEvent {
        cost_class: class,
        name: name.into(),
        elapsed_ms: Some(duration.as_millis()),
        count: None,
    });
}

pub(crate) fn record_count(class: CostClass, name: impl Into<String>, count: u64) {
    record_event(PerfEvent {
        cost_class: class,
        name: name.into(),
        elapsed_ms: None,
        count: Some(count),
    });
}

pub(crate) fn record_worker_event(phase: &str, elapsed_ms: Option<u64>, current: Option<u64>) {
    let class = match phase {
        phase if phase.contains("import") || phase.contains("setup") => CostClass::LeanImport,
        phase
            if phase.contains("semantic")
                || phase.contains("extract")
                || phase.contains("feature")
                || phase.contains("probe") =>
        {
            CostClass::LeanSemantic
        }
        _ => CostClass::Transport,
    };
    if let Some(elapsed_ms) = elapsed_ms {
        record_event(PerfEvent {
            cost_class: class,
            name: format!("worker.{phase}"),
            elapsed_ms: Some(elapsed_ms as u128),
            count: None,
        });
    }
    if let Some(current) = current {
        record_count(class, format!("worker.{phase}.count"), current);
    }
}

pub(crate) fn with_collection<T>(work: impl FnOnce() -> T) -> (T, PerfSnapshot) {
    COLLECTOR.with(|collector| {
        *collector.borrow_mut() = Some(PerfSnapshot::default());
    });
    let result = work();
    let snapshot = COLLECTOR.with(|collector| collector.borrow_mut().take().unwrap_or_default());
    (result, snapshot)
}

fn record_event(event: PerfEvent) {
    COLLECTOR.with(|collector| {
        if let Some(snapshot) = collector.borrow_mut().as_mut() {
            snapshot.events.push(event);
        }
    });
}

pub(crate) fn summarize(snapshot: &PerfSnapshot) -> PerfSummary {
    let mut summary = PerfSummary::default();
    for event in &snapshot.events {
        if let Some(elapsed_ms) = event.elapsed_ms {
            *summary.elapsed_ms_by_class.entry(event.cost_class).or_default() += elapsed_ms;
        }
        if let Some(count) = event.count {
            *summary.counts_by_name.entry(event.name.clone()).or_default() += count;
        }
    }
    summary
}

pub(crate) fn run(args: PerfArgs) -> Result<PerfReport> {
    let cache_root = args
        .cache_root
        .clone()
        .unwrap_or_else(|| repo_root().join("target/lean-dup-perf/cache"));
    let output_path = args.output.clone();
    let report = run_workload(args, &cache_root)?;
    let response = PerfReport {
        status: "ok",
        workload: report.workload,
        cache_root,
        report,
    };
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                message: "could not create directory",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&path, serde_json::to_string_pretty(&response)?).map_err(|source| Error::Io {
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
    let candidate_count = parsed
        .as_ref()
        .and_then(|payload| payload.pointer("/retrieval/candidate_count"))
        .and_then(serde_json::Value::as_u64);
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

    Ok(PerfWorkloadReport {
        workload: args.workload,
        command,
        cache_state,
        exit_code,
        elapsed_ms,
        peak_memory_bytes: memory::peak_rss_bytes(),
        stdout_bytes: stdout.len(),
        stderr_bytes: stderr.len(),
        stdout_tail: text_tail(&stdout),
        stderr_tail: text_tail(&stderr),
        candidate_count,
        hydrated_declarations,
        probe_batches: (probe_batches > 0).then_some(probe_batches),
        probe_pairs: (probe_pairs > 0).then_some(probe_pairs),
        events: snapshot.events,
        summary,
    })
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
        fs::remove_dir_all(cache_root).map_err(|source| Error::Io {
            message: "could not remove directory",
            path: cache_root.to_path_buf(),
            source,
        })?;
    }
    fs::create_dir_all(cache_root).map_err(|source| Error::Io {
        message: "could not create directory",
        path: cache_root.to_path_buf(),
        source,
    })
}

fn cache_state(workload: PerfWorkload) -> &'static str {
    match workload {
        PerfWorkload::ColdMathlibIndex => "cold",
        PerfWorkload::WarmMathlibIndex => "warm",
        PerfWorkload::KanproofsTargetedMathlib
        | PerfWorkload::KanproofsFullNoMathlib
        | PerfWorkload::KanproofsFullMathlib
        | PerfWorkload::FixtureAudit => "reuse-or-build",
    }
}

fn workload_command(args: &PerfArgs) -> Vec<String> {
    let kanproofs = args
        .kanproofs_workspace
        .clone()
        .unwrap_or_else(|| PathBuf::from("/Users/jcreinhold/Code/kan-proofs"));
    let repo = repo_root();
    let mut command = vec!["lean-dup-rs".to_owned(), "--profile".to_owned()];
    match args.workload {
        PerfWorkload::ColdMathlibIndex | PerfWorkload::WarmMathlibIndex => {
            command.extend([
                "index-mathlib".to_owned(),
                "--workspace".to_owned(),
                kanproofs.display().to_string(),
            ]);
            if let Some(mathlib) = &args.mathlib_workspace {
                command.extend(["--mathlib-workspace".to_owned(), mathlib.display().to_string()]);
            }
            if args.workload == PerfWorkload::ColdMathlibIndex {
                command.push("--force".to_owned());
            }
        }
        PerfWorkload::KanproofsTargetedMathlib => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                kanproofs.display().to_string(),
                "--module".to_owned(),
                "KanProofs.Mathlib4Backports".to_owned(),
                "--compare-mathlib".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);
            if let Some(mathlib) = &args.mathlib_workspace {
                command.extend(["--mathlib-workspace".to_owned(), mathlib.display().to_string()]);
            }
        }
        PerfWorkload::KanproofsFullNoMathlib => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                kanproofs.display().to_string(),
                "--module".to_owned(),
                "KanProofs".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);
        }
        PerfWorkload::KanproofsFullMathlib => {
            command.extend([
                "audit".to_owned(),
                "--workspace".to_owned(),
                kanproofs.display().to_string(),
                "--module".to_owned(),
                "KanProofs".to_owned(),
                "--compare-mathlib".to_owned(),
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
        .expect("crate lives under repo/crates/lean-dup-rs")
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
    use super::{CostClass, PerfSnapshot, measure, record_count, summarize};

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
}
