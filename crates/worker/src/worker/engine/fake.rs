//! Test engine: serves canned worker results without a Lean runtime.
//!
//! `FakeEngine` lets the worker crate's unit tests exercise `WorkerClient`
//! request shaping and result plumbing without building the Lean capability or
//! the `lean-dup-worker-child` binary. It records every request payload so tests
//! can assert on what the client sent.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::payload;
use crate::worker::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, IndexBatch, IndexStreamItem, ProbeBatch, ProbeResult,
    WorkerCall, WorkerError, WorkerIdentity, WorkerSubstrateFacts, WorkerVersion,
};

/// Canned engine for unit tests. Captures request payloads and returns empty
/// row sets so client-side request shaping can be asserted without Lean.
#[derive(Debug, Default)]
pub(in crate::worker) struct FakeEngine {
    requests: Arc<Mutex<Vec<Value>>>,
}

impl FakeEngine {
    /// Build an engine that records request payloads into the shared buffer.
    pub(super) fn capturing(requests: Arc<Mutex<Vec<Value>>>) -> Self {
        Self { requests }
    }

    fn record(&self, request: Value) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
    }

    pub(super) fn identity(
        &self,
        _workspace_root: PathBuf,
        _timeout: Duration,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<WorkerIdentity>, WorkerError> {
        Ok(WorkerCall {
            rows: vec![WorkerIdentity {
                semantic: WorkerVersion {
                    protocol_version: "lean-dup.worker.v1".to_owned(),
                    worker_version: "0.0.0-fake".to_owned(),
                    lean_version: None,
                    extract_version: "fake".to_owned(),
                    features_version: "fake".to_owned(),
                    probe_version: "fake".to_owned(),
                    supported_commands: vec![
                        "version".to_owned(),
                        "extract".to_owned(),
                        "features".to_owned(),
                        "index".to_owned(),
                        "probe".to_owned(),
                    ],
                    supported_capabilities: Vec::new(),
                },
                substrate: WorkerSubstrateFacts {
                    protocol_version: 0,
                    worker_version: "0.0.0-fake".to_owned(),
                },
            }],
            events: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    pub(super) fn extract(
        &self,
        batch: ExtractBatch,
        _timeout: Duration,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<DeclarationRow>, WorkerError> {
        self.record(payload::extract_request(&batch));
        Ok(empty_call())
    }

    pub(super) fn features(
        &self,
        batch: FeaturesBatch,
        _timeout: Duration,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<FeatureRow>, WorkerError> {
        self.record(payload::features_request(&batch));
        Ok(empty_call())
    }

    pub(super) fn probe(
        &self,
        batch: ProbeBatch,
        _timeout: Duration,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<ProbeResult>, WorkerError> {
        self.record(payload::probe_request(&batch));
        Ok(empty_call())
    }

    pub(super) fn index_stream(
        &self,
        batch: IndexBatch,
        _timeout: Duration,
        _cancelled: Arc<AtomicBool>,
        _sink: &mut dyn FnMut(IndexStreamItem) -> Result<(), WorkerError>,
    ) -> Result<WorkerCall<()>, WorkerError> {
        self.record(payload::index_request(&batch));
        Ok(empty_call())
    }
}

fn empty_call<Row>() -> WorkerCall<Row> {
    WorkerCall {
        rows: Vec::new(),
        events: Vec::new(),
        diagnostics: Vec::new(),
    }
}
