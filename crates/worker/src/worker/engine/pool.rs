//! Production engine: drives Lean through the `lean-rs-worker-parent` pool.
//!
//! One warm worker session per audited workspace serves every command. The pool
//! session key embeds the workspace import root, so distinct audited workspaces
//! never alias a warm session; the registered export set does not affect the
//! key, so all six commands reuse the same child. Only the `lean-dup-worker-child`
//! binary links `libleanshared`; this crate and the CLI do not.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lean_rs_worker_parent::{
    LeanWorkerCancellationToken, LeanWorkerDiagnosticEvent, LeanWorkerDiagnosticSink, LeanWorkerJsonCommand,
    LeanWorkerPool, LeanWorkerPoolConfig, LeanWorkerProgressEvent, LeanWorkerProgressSink, LeanWorkerStreamingCommand,
    LeanWorkerTypedDataRow, LeanWorkerTypedDataSink,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::payload::{self, CapabilityStreamSummary, WorkerVersionPayload};
use super::runtime::{
    EXTRACT_EXPORT, FEATURES_EXPORT, INDEX_EXPORT, LeanDupCapabilityRuntime, PROBE_EXPORT, VERSION_EXPORT,
};
use super::{CancellationBridge, map_parent_error};
use crate::worker::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, IndexBatch, IndexStreamItem, ProbeBatch, ProbeResult,
    WorkerCall, WorkerDiagnostic, WorkerError, WorkerEvent, WorkerIdentity, WorkerSubstrateFacts, WorkerVersion,
};

/// Pool-backed Lean engine. The pool is sized to one worker because audits run a
/// single workspace at a time; the warm session is reused across commands. The
/// `runtime` owns how the capability is built and loaded.
#[derive(Debug)]
pub(in crate::worker) struct PoolEngine {
    pool: Mutex<LeanWorkerPool>,
    runtime: LeanDupCapabilityRuntime,
}

impl PoolEngine {
    pub(super) fn new() -> Self {
        Self {
            pool: Mutex::new(LeanWorkerPool::new(LeanWorkerPoolConfig::new(1))),
            runtime: LeanDupCapabilityRuntime::from_build_manifest(),
        }
    }

    fn lock_pool(&self) -> Result<MutexGuard<'_, LeanWorkerPool>, WorkerError> {
        self.pool.lock().map_err(|_| WorkerError::Protocol {
            message: "worker pool mutex poisoned".to_owned(),
        })
    }

    /// Report the semantic version facts (from the Lean `version` export) plus
    /// the worker substrate facts (from the pool handshake) for a workspace.
    pub(super) fn identity(
        &self,
        workspace_root: PathBuf,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<WorkerIdentity>, WorkerError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(WorkerError::Cancelled);
        }
        let request = payload::version_request(&workspace_root.to_string_lossy());
        let token = LeanWorkerCancellationToken::new();
        let bridge = CancellationBridge::spawn(cancelled, token.clone());
        let builder = self.runtime.builder(workspace_root, timeout)?;
        let command = LeanWorkerJsonCommand::<Value, WorkerVersionPayload>::new(VERSION_EXPORT);
        let result = {
            let mut pool = self.lock_pool()?;
            let mut lease = pool.acquire_lease(builder).map_err(map_parent_error)?;
            let version = lease
                .run_json_command(&command, &request, Some(&token), None)
                .map_err(map_parent_error)?;
            let runtime = lease.runtime_metadata();
            let substrate = WorkerSubstrateFacts {
                protocol_version: runtime.protocol_version,
                worker_version: runtime.worker_version,
            };
            (version, substrate)
        };
        bridge.stop();
        if token.is_cancelled() {
            return Err(WorkerError::Cancelled);
        }
        let identity = WorkerIdentity {
            semantic: WorkerVersion::from(result.0),
            substrate: result.1,
        };
        Ok(WorkerCall {
            rows: vec![identity],
            events: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    /// Run a bounded streaming command and collect its rows. Used for `extract`,
    /// `features`, and `probe`, whose outputs are bounded per request.
    fn collect_rows<Row: DeserializeOwned + Send>(
        &self,
        export: &str,
        command_name: &str,
        request: Value,
        workspace_root: PathBuf,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<Row>, WorkerError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(WorkerError::Cancelled);
        }
        let token = LeanWorkerCancellationToken::new();
        let bridge = CancellationBridge::spawn(cancelled, token.clone());
        let builder = self.runtime.builder(workspace_root, timeout)?;
        let command = LeanWorkerStreamingCommand::<Value, Row, CapabilityStreamSummary>::new(export);
        let sink = VecSink::<Row>::default();
        let summary = {
            let mut pool = self.lock_pool()?;
            let mut lease = pool.acquire_lease(builder).map_err(map_parent_error)?;
            lease
                .run_streaming_command(&command, &request, &sink, None, Some(&token), None)
                .map_err(map_parent_error)?
        };
        bridge.stop();
        if token.is_cancelled() {
            return Err(WorkerError::Cancelled);
        }
        if let Some(metadata) = summary.metadata
            && !metadata.ok
        {
            return Err(WorkerError::WorkerDiagnostic {
                diagnostics: vec![WorkerDiagnostic {
                    code: format!("{command_name}.failed"),
                    message: metadata
                        .message
                        .unwrap_or_else(|| format!("{command_name} capability failed")),
                    fatal: true,
                    details: None,
                }],
            });
        }
        sink.into_rows()
    }

    pub(super) fn extract(
        &self,
        batch: ExtractBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<DeclarationRow>, WorkerError> {
        let request = payload::extract_request(&batch);
        let rows = self.collect_rows::<DeclarationRow>(
            EXTRACT_EXPORT,
            "extract",
            request,
            batch.workspace_root,
            timeout,
            cancelled,
        )?;
        Ok(WorkerCall {
            rows,
            events: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    pub(super) fn features(
        &self,
        batch: FeaturesBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<FeatureRow>, WorkerError> {
        let request = payload::features_request(&batch);
        let rows = self.collect_rows::<FeatureRow>(
            FEATURES_EXPORT,
            "features",
            request,
            batch.workspace_root,
            timeout,
            cancelled,
        )?;
        Ok(WorkerCall {
            rows,
            events: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    pub(super) fn probe(
        &self,
        batch: ProbeBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<ProbeResult>, WorkerError> {
        let request = payload::probe_request(&batch);
        let rows =
            self.collect_rows::<ProbeResult>(PROBE_EXPORT, "probe", request, batch.workspace_root, timeout, cancelled)?;
        Ok(WorkerCall {
            rows,
            events: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    /// Stream the import-once index, forwarding declaration rows, feature rows,
    /// and progress to the caller live so the consumer can write to its store
    /// incrementally (bounded memory). The streaming command runs on a scoped
    /// thread; the caller's sink is driven from this thread as rows arrive.
    pub(super) fn index_stream(
        &self,
        batch: IndexBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
        sink: &mut dyn FnMut(IndexStreamItem) -> Result<(), WorkerError>,
    ) -> Result<WorkerCall<()>, WorkerError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(WorkerError::Cancelled);
        }
        let request = payload::index_request(&batch);
        let workspace_root = batch.workspace_root;
        let token = LeanWorkerCancellationToken::new();
        let stream_token = token.clone();
        let bridge = CancellationBridge::spawn(cancelled, token.clone());
        let builder = self.runtime.builder(workspace_root, timeout)?;
        let command = LeanWorkerStreamingCommand::<Value, Value, CapabilityStreamSummary>::new(INDEX_EXPORT);

        let (tx, rx) = sync_channel::<StreamMsg>(256);
        let diagnostics: Mutex<Vec<WorkerDiagnostic>> = Mutex::new(Vec::new());
        let mut forward_err: Option<WorkerError> = None;

        let summary_result = std::thread::scope(|scope| {
            let stream_tx = tx.clone();
            let diagnostics = &diagnostics;
            let command = &command;
            let request = &request;
            let stream_token = &stream_token;
            let handle = scope.spawn(move || -> Result<_, WorkerError> {
                let row_sink = ChannelRowSink { tx: stream_tx.clone() };
                let progress_sink = ChannelProgressSink { tx: stream_tx };
                let diag_sink = CapturingDiagnosticSink { diagnostics };
                let mut pool = self.lock_pool()?;
                let mut lease = pool.acquire_lease(builder).map_err(map_parent_error)?;
                lease
                    .run_streaming_command(
                        command,
                        request,
                        &row_sink,
                        Some(&diag_sink),
                        Some(stream_token),
                        Some(&progress_sink),
                    )
                    .map_err(map_parent_error)
            });
            drop(tx);
            for msg in rx {
                if forward_err.is_some() {
                    continue;
                }
                let item = match decode_stream_item(msg) {
                    Ok(item) => item,
                    Err(error) => {
                        token.cancel();
                        forward_err = Some(error);
                        continue;
                    }
                };
                if let Err(error) = sink(item) {
                    token.cancel();
                    forward_err = Some(error);
                }
            }
            handle.join().unwrap_or_else(|_| {
                Err(WorkerError::Protocol {
                    message: "index streaming thread panicked".to_owned(),
                })
            })
        });

        bridge.stop();
        if let Some(error) = forward_err {
            return Err(error);
        }
        let summary = summary_result?;
        if token.is_cancelled() {
            return Err(WorkerError::Cancelled);
        }
        if let Some(metadata) = summary.metadata
            && !metadata.ok
        {
            let mut captured = diagnostics.into_inner().unwrap_or_default();
            if captured.is_empty() {
                captured.push(WorkerDiagnostic {
                    code: "index.failed".to_owned(),
                    message: metadata.message.unwrap_or_else(|| "index capability failed".to_owned()),
                    fatal: true,
                    details: None,
                });
            }
            return Err(WorkerError::WorkerDiagnostic { diagnostics: captured });
        }
        Ok(WorkerCall {
            rows: Vec::new(),
            events: Vec::new(),
            diagnostics: Vec::new(),
        })
    }
}

/// Decode one streamed message into the public index item, routing rows by their
/// caller-defined stream name.
fn decode_stream_item(msg: StreamMsg) -> Result<IndexStreamItem, WorkerError> {
    match msg {
        StreamMsg::Row { stream, payload } => match stream.as_str() {
            "declarations" => serde_json::from_value::<DeclarationRow>(payload)
                .map(IndexStreamItem::Declaration)
                .map_err(|error| WorkerError::Protocol {
                    message: format!("could not decode streamed declaration row: {error}"),
                }),
            "features" => serde_json::from_value::<FeatureRow>(payload)
                .map(IndexStreamItem::Feature)
                .map_err(|error| WorkerError::Protocol {
                    message: format!("could not decode streamed feature row: {error}"),
                }),
            other => Err(WorkerError::Protocol {
                message: format!("index stream produced unknown stream `{other}`"),
            }),
        },
        StreamMsg::Progress(event) => Ok(IndexStreamItem::Event(event)),
    }
}

/// Message carried from the streaming worker thread to the draining caller.
enum StreamMsg {
    Row { stream: String, payload: Value },
    Progress(WorkerEvent),
}

/// Bounded-command sink: collect every reported row.
struct VecSink<Row> {
    rows: Mutex<Vec<Row>>,
}

impl<Row> Default for VecSink<Row> {
    fn default() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
        }
    }
}

impl<Row> VecSink<Row> {
    fn into_rows(self) -> Result<Vec<Row>, WorkerError> {
        self.rows.into_inner().map_err(|_| WorkerError::Protocol {
            message: "worker row sink mutex poisoned".to_owned(),
        })
    }
}

impl<Row: Send> LeanWorkerTypedDataSink<Row> for VecSink<Row> {
    fn report(&self, row: LeanWorkerTypedDataRow<Row>) {
        if let Ok(mut rows) = self.rows.lock() {
            rows.push(row.payload);
        }
    }
}

/// Streaming-index row sink: forward each row through the bounded channel.
struct ChannelRowSink {
    tx: SyncSender<StreamMsg>,
}

impl LeanWorkerTypedDataSink<Value> for ChannelRowSink {
    fn report(&self, row: LeanWorkerTypedDataRow<Value>) {
        let _ = self.tx.send(StreamMsg::Row {
            stream: row.stream,
            payload: row.payload,
        });
    }
}

struct ChannelProgressSink {
    tx: SyncSender<StreamMsg>,
}

impl LeanWorkerProgressSink for ChannelProgressSink {
    fn report(&self, event: LeanWorkerProgressEvent) {
        let _ = self.tx.send(StreamMsg::Progress(WorkerEvent {
            phase: event.phase,
            current: Some(event.current),
            total: event.total,
            module: None,
            declaration: None,
            elapsed_ms: Some(u64::try_from(event.elapsed.as_millis()).unwrap_or(u64::MAX)),
            message: String::new(),
        }));
    }
}

struct CapturingDiagnosticSink<'a> {
    diagnostics: &'a Mutex<Vec<WorkerDiagnostic>>,
}

impl LeanWorkerDiagnosticSink for CapturingDiagnosticSink<'_> {
    fn report(&self, diagnostic: LeanWorkerDiagnosticEvent) {
        if let Ok(mut diagnostics) = self.diagnostics.lock() {
            diagnostics.push(WorkerDiagnostic {
                code: diagnostic.code,
                message: diagnostic.message,
                fatal: true,
                details: None,
            });
        }
    }
}
