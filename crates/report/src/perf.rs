use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Runtime cost classes used by internal diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostClass {
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
pub struct PerfEvent {
    pub cost_class: CostClass,
    pub name: String,
    pub elapsed_ms: Option<u128>,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerfSnapshot {
    pub events: Vec<PerfEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerfSummary {
    pub elapsed_ms_by_class: BTreeMap<CostClass, u128>,
    pub counts_by_name: BTreeMap<String, u64>,
}

thread_local! {
    static COLLECTOR: RefCell<Option<PerfSnapshot>> = const { RefCell::new(None) };
}

pub fn measure<T>(class: CostClass, name: impl Into<String>, work: impl FnOnce() -> T) -> T {
    let name = name.into();
    let started = Instant::now();
    let result = work();
    record_duration(class, name, started.elapsed());
    result
}

pub fn measure_result<T, E>(
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

pub fn record_duration(class: CostClass, name: impl Into<String>, duration: Duration) {
    record_event(PerfEvent {
        cost_class: class,
        name: name.into(),
        elapsed_ms: Some(duration.as_millis()),
        count: None,
    });
}

pub fn record_count(class: CostClass, name: impl Into<String>, count: u64) {
    record_event(PerfEvent {
        cost_class: class,
        name: name.into(),
        elapsed_ms: None,
        count: Some(count),
    });
}

pub fn record_worker_event(phase: &str, elapsed_ms: Option<u64>, current: Option<u64>) {
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

pub fn with_collection<T>(work: impl FnOnce() -> T) -> (T, PerfSnapshot) {
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

pub fn summarize(snapshot: &PerfSnapshot) -> PerfSummary {
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
