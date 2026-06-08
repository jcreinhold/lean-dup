//! Private engine seam behind `WorkerClient`.
//!
//! `WorkerClient` drives Lean through one concrete engine — never a trait
//! object. [`WorkerEngine::Pool`] is the production path over the
//! `lean-rs-worker-parent` pool; the test-only [`WorkerEngine::Fake`] serves
//! canned results so the worker crate's unit tests run without a Lean runtime.
//! The seam is private to this crate: no engine type appears on the public API.
//!
//! The pool engine pairs the [`runtime::LeanDupCapabilityRuntime`] (which owns
//! *how the `LeanDup` capability is built and loaded*) with the command calls
//! (which own *what each command sends and decodes*). Keeping those two
//! concerns in separate modules means the steady-state packaging change — moving
//! capability production behind a package-owned runtime crate — touches only
//! `runtime`, not the command path.

mod payload;
mod pool;
mod runtime;

#[cfg(test)]
mod fake;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use lean_rs_worker_parent::{LeanWorkerCancellationToken, LeanWorkerError as ParentWorkerError};

use super::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, IndexBatch, IndexStreamItem, ProbeBatch, ProbeResult,
    WorkerCall, WorkerDiagnostic, WorkerError, WorkerIdentity,
};

#[cfg(test)]
use fake::FakeEngine;
use pool::PoolEngine;

/// The single, private engine `WorkerClient` dispatches through.
#[derive(Debug)]
pub(super) enum WorkerEngine {
    Pool(Box<PoolEngine>),
    #[cfg(test)]
    Fake(FakeEngine),
}

impl WorkerEngine {
    /// The production engine over the `lean-rs-worker-parent` pool.
    pub(super) fn pool() -> Self {
        Self::Pool(Box::new(PoolEngine::new()))
    }

    /// A test engine that records request payloads and returns no rows, for
    /// unit tests that run without a Lean runtime.
    #[cfg(test)]
    pub(super) fn fake(requests: Arc<std::sync::Mutex<Vec<serde_json::Value>>>) -> Self {
        Self::Fake(FakeEngine::capturing(requests))
    }

    pub(super) fn identity(
        &self,
        workspace_root: PathBuf,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<WorkerIdentity>, WorkerError> {
        match self {
            Self::Pool(engine) => engine.identity(workspace_root, timeout, cancelled),
            #[cfg(test)]
            Self::Fake(engine) => engine.identity(workspace_root, timeout, cancelled),
        }
    }

    pub(super) fn extract(
        &self,
        batch: ExtractBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<DeclarationRow>, WorkerError> {
        match self {
            Self::Pool(engine) => engine.extract(batch, timeout, cancelled),
            #[cfg(test)]
            Self::Fake(engine) => engine.extract(batch, timeout, cancelled),
        }
    }

    pub(super) fn features(
        &self,
        batch: FeaturesBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<FeatureRow>, WorkerError> {
        match self {
            Self::Pool(engine) => engine.features(batch, timeout, cancelled),
            #[cfg(test)]
            Self::Fake(engine) => engine.features(batch, timeout, cancelled),
        }
    }

    pub(super) fn probe(
        &self,
        batch: ProbeBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<ProbeResult>, WorkerError> {
        match self {
            Self::Pool(engine) => engine.probe(batch, timeout, cancelled),
            #[cfg(test)]
            Self::Fake(engine) => engine.probe(batch, timeout, cancelled),
        }
    }

    pub(super) fn index_stream(
        &self,
        batch: IndexBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
        sink: &mut dyn FnMut(IndexStreamItem) -> Result<(), WorkerError>,
    ) -> Result<WorkerCall<()>, WorkerError> {
        match self {
            Self::Pool(engine) => engine.index_stream(batch, timeout, cancelled, sink),
            #[cfg(test)]
            Self::Fake(engine) => engine.index_stream(batch, timeout, cancelled, sink),
        }
    }
}

/// Map a pool error onto the worker crate's error type, preserving the
/// recoverable timeout/cancel distinction probe chunk-splitting depends on and
/// collapsing the rest to fatal diagnostics.
pub(super) fn map_parent_error(error: ParentWorkerError) -> WorkerError {
    match error {
        ParentWorkerError::Timeout { duration, .. } => WorkerError::Timeout { timeout: duration },
        ParentWorkerError::Cancelled { .. } => WorkerError::Cancelled,
        ParentWorkerError::CapabilityBuild { diagnostic } => WorkerError::BuildFailed {
            status: 1,
            diagnostic: diagnostic.to_string(),
        },
        ParentWorkerError::ChildExited { exit } | ParentWorkerError::ChildPanicOrAbort { exit } => {
            WorkerError::NonZeroExit {
                status: exit.code.unwrap_or(1),
                stderr: exit.diagnostics,
            }
        }
        ParentWorkerError::StreamExportFailed { status } => WorkerError::WorkerDiagnostic {
            diagnostics: vec![WorkerDiagnostic {
                code: "capability.failed".to_owned(),
                message: format!("capability export returned status {status}"),
                fatal: true,
                details: None,
            }],
        },
        ParentWorkerError::Worker { code, message } => WorkerError::WorkerDiagnostic {
            diagnostics: vec![WorkerDiagnostic {
                code,
                message,
                fatal: true,
                details: None,
            }],
        },
        other => WorkerError::Protocol {
            message: other.to_string(),
        },
    }
}

/// Bridge the client's cooperative `AtomicBool` cancellation to the pool's
/// cancellation token: a short-lived watcher thread cancels the token when the
/// flag is set, and is stopped when the command returns.
pub(super) struct CancellationBridge {
    done: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CancellationBridge {
    pub(super) fn spawn(cancelled: Arc<AtomicBool>, token: LeanWorkerCancellationToken) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let thread_done = done.clone();
        let handle = thread::spawn(move || {
            while !thread_done.load(Ordering::Relaxed) {
                if cancelled.load(Ordering::Relaxed) {
                    token.cancel();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        Self {
            done,
            handle: Some(handle),
        }
    }

    pub(super) fn stop(mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            drop(handle.join());
        }
    }
}

impl Drop for CancellationBridge {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            drop(handle.join());
        }
    }
}
