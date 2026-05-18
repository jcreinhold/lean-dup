//! Lean worker process boundary.
//!
//! This crate owns the protocol, subprocess transport, worker version policy,
//! and request/response data exchanged with Lean. Callers should not know the
//! JSONL framing, stdout/stderr parsing, or timeout mechanics.

mod worker;

pub use worker::*;

mod perf {
    #[derive(Debug, Clone, Copy)]
    pub enum CostClass {
        WorkerStartup,
        Transport,
        LeanSemantic,
    }

    pub fn measure_result<T, E>(
        _class: CostClass,
        _name: impl Into<String>,
        work: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        work()
    }

    pub fn record_count(_class: CostClass, _name: impl Into<String>, _count: u64) {}

    pub fn record_worker_event(_phase: &str, _elapsed_ms: Option<u64>, _current: Option<u64>) {}
}
