//! Private engine seam behind `WorkerClient`.
//!
//! `WorkerClient` drives Lean through one concrete engine — never a trait
//! object. [`WorkerEngine::Subprocess`] is the production path over the native
//! `lean-dup-worker` executable; the test-only [`WorkerEngine::Fake`] serves
//! canned results so the worker crate's unit tests run without a Lean runtime.
//! The seam is private to this crate: no engine type appears on the public API.

mod payload;
mod subprocess;

#[cfg(test)]
mod fake;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[cfg(test)]
use fake::FakeEngine;
use subprocess::SubprocessEngine;

use super::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, IndexBatch, IndexStreamItem, ProbeBatch, ProbeResult,
    WorkerCall, WorkerError, WorkerIdentity,
};

/// The single, private engine `WorkerClient` dispatches through.
#[derive(Debug)]
pub(super) enum WorkerEngine {
    Subprocess(SubprocessEngine),
    #[cfg(test)]
    Fake(FakeEngine),
}

impl WorkerEngine {
    /// The production engine over the native worker executable.
    pub(super) fn pool() -> Self {
        Self::Subprocess(SubprocessEngine::new())
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
            Self::Subprocess(engine) => engine.identity(workspace_root, timeout, cancelled),
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
            Self::Subprocess(engine) => engine.extract(batch, timeout, cancelled),
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
            Self::Subprocess(engine) => engine.features(batch, timeout, cancelled),
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
            Self::Subprocess(engine) => engine.probe(batch, timeout, cancelled),
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
            Self::Subprocess(engine) => engine.index_stream(batch, timeout, cancelled, sink),
            #[cfg(test)]
            Self::Fake(engine) => engine.index_stream(batch, timeout, cancelled, sink),
        }
    }
}
