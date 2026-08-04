//! Production engine: drives the native `lean-dup-worker` executable over JSONL.
//!
//! One child process per audited workspace serves every command; the warm child
//! holds the imported Lean environment (Lean-side session cache), so an audit
//! imports at most once per module signature. The transport is line-framed
//! JSON: one request envelope per stdin line (`{"command", "request"}`), framed
//! response lines on stdout (`row` / `progress` / `diagnostic` / `metadata` /
//! `result`). Cancellation and timeouts kill the child — process exit is the
//! only sound interrupt for Lean elaboration — and the next command respawns.
//! Only the spawned executable links `libleanshared`; this crate and the CLI do
//! not.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::debug;

use super::payload::{self, CapabilityStreamSummary, WorkerVersionPayload};
use crate::toolchain::resolve_installed_worker;
use crate::worker::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, IndexBatch, IndexStreamItem, ProbeBatch, ProbeResult,
    WorkerCall, WorkerDiagnostic, WorkerError, WorkerEvent, WorkerIdentity, WorkerSubstrateFacts, WorkerVersion,
};

/// Bound on the reader-thread channel; the parent drains continuously, so this
/// only smooths bursts (chunked index rows arrive in batches).
const FRAME_CHANNEL_BOUND: usize = 1024;

/// Slice of the per-call timeout slept between cancellation polls.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Transport framing protocol version reported as the worker substrate fact.
/// `1` was the retired `lean-rs-worker` pool transport; `2` is the native
/// JSONL subprocess. The index cache key folds this in, so the bump re-warms
/// caches once across the transport swap.
const TRANSPORT_PROTOCOL_VERSION: u16 = 2;

/// Session map entry: one shared, command-serialized child per workspace.
type SharedSession = Arc<Mutex<Session>>;

/// JSONL subprocess engine. Sessions are keyed by workspace root; each is
/// locked for the duration of a command, so commands stay serialized per child
/// (matching the single warm session the Lean server caches against).
#[derive(Debug)]
pub(in crate::worker) struct SubprocessEngine {
    sessions: Mutex<HashMap<PathBuf, SharedSession>>,
}

/// One framed line from the worker.
#[derive(Debug)]
enum Frame {
    Row {
        stream: String,
        payload: Value,
    },
    Progress {
        phase: String,
        current: u64,
        total: Option<u64>,
    },
    Diagnostic {
        code: String,
        message: String,
    },
    Metadata(Value),
    Result(Value),
}

/// A live worker child: its stdin, the reader-thread frame stream, and its
/// captured stderr (surfaced in crash diagnostics).
#[derive(Debug)]
struct Session {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<Frame>,
    stderr: Arc<Mutex<String>>,
}

impl Session {
    /// Terminate the child. Best-effort: a wedged Lean process answers SIGKILL,
    /// not protocol.
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn stderr_text(&self) -> String {
        self.stderr.lock().map(|text| text.clone()).unwrap_or_default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

impl SubprocessEngine {
    pub(super) fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn lock_sessions(&self) -> Result<MutexGuard<'_, HashMap<PathBuf, SharedSession>>, WorkerError> {
        self.sessions.lock().map_err(|_| WorkerError::Protocol {
            message: "worker session map mutex poisoned".to_owned(),
        })
    }

    /// The session for `workspace_root`, spawning the child on first use.
    fn session(&self, workspace_root: &std::path::Path) -> Result<SharedSession, WorkerError> {
        let mut sessions = self.lock_sessions()?;
        if let Some(session) = sessions.get(workspace_root) {
            return Ok(session.clone());
        }
        let session = Arc::new(Mutex::new(spawn_session(workspace_root)?));
        sessions.insert(workspace_root.to_path_buf(), session.clone());
        Ok(session)
    }

    /// Drop the session for `workspace_root` after a fatal transport failure
    /// (timeout, crash): the child's in-flight state is unknowable, so the
    /// next command starts a fresh process. The Lean session cache means this
    /// costs one re-import, not unbounded growth.
    fn retire(&self, workspace_root: &std::path::Path) {
        if let Ok(mut sessions) = self.lock_sessions() {
            sessions.remove(workspace_root);
        }
    }

    /// Report the semantic version facts (from the `version` command) plus the
    /// worker substrate facts for a workspace. The substrate facts come from
    /// the same payload: the transport change does not move cache keys.
    pub(super) fn identity(
        &self,
        workspace_root: PathBuf,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<WorkerIdentity>, WorkerError> {
        let request = payload::version_request(&workspace_root.to_string_lossy());
        let outcome = self.run_command(&workspace_root, "version", &request, timeout, &cancelled, &mut |_| {})?;
        let Terminal::Result(payload) = outcome.terminal else {
            return Err(WorkerError::Protocol {
                message: "version command did not produce a result frame".to_owned(),
            });
        };
        let semantic = WorkerVersion::from(serde_json::from_value::<WorkerVersionPayload>(payload).map_err(
            |error| WorkerError::Protocol {
                message: format!("could not decode version payload: {error}"),
            },
        )?);
        let substrate = WorkerSubstrateFacts {
            protocol_version: TRANSPORT_PROTOCOL_VERSION,
            worker_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let identity = WorkerIdentity { semantic, substrate };
        Ok(WorkerCall {
            rows: vec![identity],
            events: Vec::new(),
            diagnostics: Vec::new(),
            skipped: 0,
        })
    }

    /// Run a bounded streaming command and collect its rows. Used for
    /// `extract`, `features`, and `probe`, whose outputs are bounded per
    /// request.
    fn collect_rows<Row: DeserializeOwned>(
        &self,
        command_name: &str,
        request: Value,
        workspace_root: PathBuf,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(Vec<Row>, u64), WorkerError> {
        debug!(command = command_name, "dispatching streaming worker command");
        let mut rows: Vec<Row> = Vec::new();
        let mut decode_error: Option<WorkerError> = None;
        let outcome = self.run_command(
            &workspace_root,
            command_name,
            &request,
            timeout,
            &cancelled,
            &mut |frame| {
                if decode_error.is_some() {
                    return;
                }
                let Frame::Row { payload, .. } = frame else {
                    return;
                };
                match serde_json::from_value::<Row>(payload.clone()) {
                    Ok(row) => rows.push(row),
                    Err(error) => {
                        decode_error = Some(WorkerError::Protocol {
                            message: format!("could not decode streamed {command_name} row: {error}"),
                        });
                    }
                }
            },
        )?;
        if let Some(error) = decode_error {
            return Err(error);
        }
        let skipped = outcome.skipped;
        if skipped > 0 {
            debug!(
                command = command_name,
                skipped, "worker skipped declarations under budget"
            );
        }
        Ok((rows, skipped))
    }

    pub(super) fn extract(
        &self,
        batch: ExtractBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<DeclarationRow>, WorkerError> {
        let request = payload::extract_request(&batch);
        let (rows, skipped) = self.collect_rows("extract", request, batch.workspace_root, timeout, cancelled)?;
        Ok(WorkerCall {
            rows,
            events: Vec::new(),
            diagnostics: Vec::new(),
            skipped,
        })
    }

    pub(super) fn features(
        &self,
        batch: FeaturesBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<FeatureRow>, WorkerError> {
        let request = payload::features_request(&batch);
        let (rows, skipped) = self.collect_rows("features", request, batch.workspace_root, timeout, cancelled)?;
        Ok(WorkerCall {
            rows,
            events: Vec::new(),
            diagnostics: Vec::new(),
            skipped,
        })
    }

    pub(super) fn probe(
        &self,
        batch: ProbeBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Result<WorkerCall<ProbeResult>, WorkerError> {
        let request = payload::probe_request(&batch);
        let (rows, skipped) = self.collect_rows("probe", request, batch.workspace_root, timeout, cancelled)?;
        Ok(WorkerCall {
            rows,
            events: Vec::new(),
            diagnostics: Vec::new(),
            skipped,
        })
    }

    /// Stream the import-once index, forwarding declaration rows, feature rows,
    /// and progress to the caller live so the consumer can write to its store
    /// incrementally (bounded memory).
    pub(super) fn index_stream(
        &self,
        batch: IndexBatch,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
        sink: &mut dyn FnMut(IndexStreamItem) -> Result<(), WorkerError>,
    ) -> Result<WorkerCall<()>, WorkerError> {
        let request = payload::index_request(&batch);
        let workspace_root = batch.workspace_root;
        let mut forward_err: Option<WorkerError> = None;
        let outcome = self.run_command(&workspace_root, "index", &request, timeout, &cancelled, &mut |frame| {
            if forward_err.is_some() {
                return;
            }
            if let Frame::Progress { phase, current, total } = frame {
                if let Err(error) = sink(IndexStreamItem::Event(WorkerEvent {
                    phase: phase.clone(),
                    current: Some(*current),
                    total: *total,
                    module: None,
                    declaration: None,
                    elapsed_ms: None,
                    message: String::new(),
                })) {
                    forward_err = Some(error);
                }
                return;
            }
            let Frame::Row { stream, payload } = frame else {
                return;
            };
            let payload = payload.clone();
            let item = match stream.as_str() {
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
            };
            match item {
                Ok(item) => {
                    if let Err(error) = sink(item) {
                        forward_err = Some(error);
                    }
                }
                Err(error) => forward_err = Some(error),
            }
        })?;
        if let Some(error) = forward_err {
            return Err(error);
        }
        Ok(WorkerCall {
            rows: Vec::new(),
            events: Vec::new(),
            diagnostics: outcome.diagnostics,
            skipped: outcome.skipped,
        })
    }

    /// Execute one command against the workspace's warm child: write the
    /// request envelope, then drain frames into the row/progress callbacks
    /// until the terminal frame (`metadata` for streaming commands, `result`
    /// for `version`). Handles the three fatal outcomes uniformly: timeout and
    /// cancellation kill and retire the child; a dead child retires itself.
    fn run_command(
        &self,
        workspace_root: &std::path::Path,
        command: &str,
        request: &Value,
        timeout: Duration,
        cancelled: &Arc<AtomicBool>,
        on_frame: &mut dyn FnMut(&Frame),
    ) -> Result<CommandOutcome, WorkerError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(WorkerError::Cancelled);
        }
        let session = self.session(workspace_root)?;
        let command_result = {
            let mut session = session.lock().map_err(|_| WorkerError::Protocol {
                message: "worker session mutex poisoned".to_owned(),
            })?;
            drive_command(&mut session, command, request, timeout, cancelled, on_frame)
        };
        match command_result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                if error.retires_session() {
                    self.retire(workspace_root);
                }
                Err(error)
            }
        }
    }
}

/// The frame that ended a command.
enum Terminal {
    /// Streaming command finished (metadata summary; `skipped` already read).
    Metadata,
    /// Request/response command finished: the result payload.
    Result(Value),
}

/// What a finished command produced beyond its rows.
struct CommandOutcome {
    terminal: Terminal,
    skipped: u64,
    diagnostics: Vec<WorkerDiagnostic>,
}

/// Drive one command on a locked session. See [`SubprocessEngine::run_command`]
/// for the contract.
fn drive_command(
    session: &mut Session,
    command: &str,
    request: &Value,
    timeout: Duration,
    cancelled: &Arc<AtomicBool>,
    on_frame: &mut dyn FnMut(&Frame),
) -> Result<CommandOutcome, WorkerError> {
    let envelope = serde_json::json!({ "command": command, "request": request });
    writeln!(session.stdin, "{envelope}")
        .and_then(|()| session.stdin.flush())
        .map_err(|error| WorkerError::NonZeroExit {
            status: 1,
            stderr: format!(
                "worker stdin closed while sending `{command}`: {error}; {}",
                session.stderr_text()
            ),
        })?;

    let deadline = Instant::now() + timeout;
    let mut diagnostics: Vec<WorkerDiagnostic> = Vec::new();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            session.kill();
            return Err(WorkerError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            session.kill();
            return Err(WorkerError::Timeout { timeout });
        }
        match session.frames.recv_timeout(remaining.min(CANCEL_POLL)) {
            Ok(frame) => match frame {
                Frame::Diagnostic { code, message } => {
                    diagnostics.push(WorkerDiagnostic {
                        code,
                        message,
                        fatal: true,
                        details: None,
                    });
                }
                Frame::Progress { .. } => on_frame(&frame),
                Frame::Result(payload) => {
                    return Ok(CommandOutcome {
                        terminal: Terminal::Result(payload),
                        skipped: 0,
                        diagnostics,
                    });
                }
                Frame::Metadata(summary) => {
                    return finish_streaming(command, summary, diagnostics);
                }
                Frame::Row { .. } => on_frame(&frame),
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(WorkerError::NonZeroExit {
                    status: session
                        .child
                        .try_wait()
                        .ok()
                        .flatten()
                        .and_then(|status| status.code())
                        .unwrap_or(1),
                    stderr: session.stderr_text(),
                });
            }
        }
    }
}

/// Interpret the terminal metadata summary of a streaming command: a failed
/// summary (`ok: false`) maps to the fatal diagnostic the worker reported,
/// with any earlier diagnostic frames attached.
fn finish_streaming(
    command: &str,
    summary: Value,
    mut diagnostics: Vec<WorkerDiagnostic>,
) -> Result<CommandOutcome, WorkerError> {
    let parsed: CapabilityStreamSummary = serde_json::from_value(summary.clone()).unwrap_or_default();
    if parsed.ok {
        return Ok(CommandOutcome {
            terminal: Terminal::Metadata,
            skipped: parsed.skipped,
            diagnostics,
        });
    }
    if diagnostics.is_empty() {
        diagnostics.push(WorkerDiagnostic {
            code: format!("{command}.failed"),
            message: parsed.message.unwrap_or_else(|| format!("{command} command failed")),
            fatal: true,
            details: None,
        });
    }
    Err(WorkerError::WorkerDiagnostic { diagnostics })
}

impl WorkerError {
    /// Whether this failure poisons the child: timeouts and crashes leave
    /// in-flight state unknowable, so the session is retired and the next
    /// command respawns. Protocol-level worker diagnostics leave the child
    /// usable (the Lean server framed the failure and is ready for more).
    fn retires_session(&self) -> bool {
        matches!(
            self,
            WorkerError::Timeout { .. } | WorkerError::Cancelled | WorkerError::NonZeroExit { .. }
        )
    }
}

/// Spawn the `lean-dup-worker` executable for one workspace. Inside a Lake
/// package the child runs under `lake env`, which installs the workspace's
/// `.olean` search path; outside one (the `install-worker` smoke workspace,
/// which only exercises `version`) the executable runs bare.
fn spawn_session(workspace_root: &std::path::Path) -> Result<Session, WorkerError> {
    let installed = resolve_installed_worker(workspace_root).map_err(|error| WorkerError::NotProvisioned {
        message: error.to_string(),
    })?;
    let in_lake_package =
        workspace_root.join("lakefile.lean").is_file() || workspace_root.join("lakefile.toml").is_file();
    let mut command = if in_lake_package {
        let mut command = Command::new("lake");
        command.arg("env").arg(&installed.worker_exe);
        command
    } else {
        Command::new(&installed.worker_exe)
    };
    let mut child = command
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| WorkerError::NotProvisioned {
            message: format!(
                "could not spawn lean-dup worker at {}: {error}",
                installed.worker_exe.display()
            ),
        })?;
    let stdin = child.stdin.take().ok_or_else(|| WorkerError::Protocol {
        message: "worker child stdin was not piped".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| WorkerError::Protocol {
        message: "worker child stdout was not piped".to_owned(),
    })?;
    let stderr_pipe = child.stderr.take().ok_or_else(|| WorkerError::Protocol {
        message: "worker child stderr was not piped".to_owned(),
    })?;
    let (tx, rx) = sync_channel::<Frame>(FRAME_CHANNEL_BOUND);
    std::thread::spawn(move || read_frames(stdout, tx));
    let stderr: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_sink = stderr.clone();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::BufReader::new(stderr_pipe).read_to_string(&mut buf);
        if let Ok(mut text) = stderr_sink.lock() {
            *text = buf;
        }
    });
    Ok(Session {
        child,
        stdin,
        frames: rx,
        stderr,
    })
}

/// Reader loop: decode each stdout line into a frame and forward it. Unknown
/// or unparseable lines are skipped (the server writes only protocol frames,
/// but a stray Lean trace on stdout must not desynchronize the transport).
fn read_frames(stdout: impl Read, tx: SyncSender<Frame>) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let frame = decode_frame(&value);
        let Some(frame) = frame else { continue };
        if tx.send(frame).is_err() {
            break;
        }
    }
}

fn decode_frame(value: &Value) -> Option<Frame> {
    let object = value.as_object()?;
    if let Some(payload) = object.get("result") {
        return Some(Frame::Result(payload.clone()));
    }
    if let Some(metadata) = object.get("metadata") {
        return Some(Frame::Metadata(metadata.clone()));
    }
    if let Some(diagnostic) = object.get("diagnostic") {
        return Some(Frame::Diagnostic {
            code: diagnostic
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("worker.diagnostic")
                .to_owned(),
            message: diagnostic
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    if let Some(progress) = object.get("progress") {
        return Some(Frame::Progress {
            phase: progress
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            current: progress.get("current").and_then(Value::as_u64).unwrap_or(0),
            total: progress.get("total").and_then(Value::as_u64),
        });
    }
    if let (Some(stream), Some(payload)) = (object.get("stream"), object.get("payload")) {
        return Some(Frame::Row {
            stream: stream.as_str()?.to_owned(),
            payload: payload.clone(),
        });
    }
    None
}
