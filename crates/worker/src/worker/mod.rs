mod protocol;
mod subprocess;
mod transport;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::perf::{self, CostClass};

use self::protocol::{Command, ProtocolItem, Request, Row};
use self::subprocess::SubprocessTransport;
use self::transport::{CallControl, WorkerTransport};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const DEFAULT_WORKER_TIMEOUT: Duration = Duration::from_secs(60);
const INDEX_WORKER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Client for Lean semantic worker capabilities.
///
/// Callers ask for semantic facts by capability: worker version, declaration
/// extraction, feature extraction, and semantic probes. The client returns typed
/// rows, progress events, and diagnostics; callers do not receive process
/// handles, protocol envelopes, transport frames, or stderr as machine data.
pub struct WorkerClient {
    transport: Box<dyn WorkerTransport + Send + Sync>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
}

impl fmt::Debug for WorkerClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerClient")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Default for WorkerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerClient {
    /// Create a worker client with the default timeout policy.
    pub fn new() -> Self {
        Self {
            transport: Box::new(SubprocessTransport::new()),
            timeout: DEFAULT_WORKER_TIMEOUT,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a worker client for index builds over large imported environments.
    ///
    /// Indexing can legitimately spend minutes importing and streaming rows for
    /// mathlib-sized workspaces. Callers use this policy for index construction
    /// without learning transport timeout constants or adding user-facing knobs.
    pub fn for_indexing() -> Self {
        Self::with_timeout(INDEX_WORKER_TIMEOUT)
    }

    /// Create a worker client with an explicit per-call timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            transport: Box::new(SubprocessTransport::new()),
            timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Request cancellation for calls started from this client.
    ///
    /// Cancellation is cooperative at the Rust transport boundary: a running
    /// worker subprocess is terminated and the call returns a structured
    /// cancellation error.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Return the worker and semantic algorithm versions for a Lake workspace.
    pub fn version(&self, workspace_root: PathBuf) -> Result<WorkerCall<WorkerVersion>, WorkerError> {
        let payload = serde_json::json!({ "workspace_root": workspace_root });
        self.call(Request::new(request_id(), Command::Version, payload))
    }

    /// Extract typed declaration rows for a batch of Lean modules.
    pub fn extract_batch(&self, batch: ExtractBatch) -> Result<WorkerCall<DeclarationRow>, WorkerError> {
        let mut payload = protocol::modules_payload(&batch.workspace_root_string(), &batch.modules);
        payload["include_private"] = Value::Bool(batch.include_private);
        payload["include_generated"] = Value::Bool(batch.include_generated);
        self.call(Request::new(request_id(), Command::Extract, payload))
    }

    /// Compute Lean-owned semantic feature rows for a module batch.
    pub fn features_batch(&self, batch: FeaturesBatch) -> Result<WorkerCall<FeatureRow>, WorkerError> {
        let mut payload = protocol::modules_payload(&batch.workspace_root_string(), &batch.modules);
        payload["include_private"] = Value::Bool(batch.include_private);
        payload["include_generated"] = Value::Bool(batch.include_generated);
        if let Some(declaration_ids) = batch.declaration_ids {
            payload["declaration_ids"] = serde_json::json!(declaration_ids);
        }
        self.call(Request::new(request_id(), Command::Features, payload))
    }

    /// Stream declaration and feature rows from one import-once index command.
    ///
    /// The caller receives semantic rows and progress events as they arrive.
    /// The worker client still owns JSONL framing, request ids, subprocess
    /// lifetime, and structured diagnostic handling.
    pub fn index_stream(
        &self,
        batch: IndexBatch,
        sink: &mut dyn FnMut(IndexStreamItem) -> Result<(), WorkerError>,
    ) -> Result<WorkerCall<()>, WorkerError> {
        let mut payload = protocol::modules_payload(&batch.workspace_root_string(), &batch.modules);
        payload["include_private"] = Value::Bool(batch.include_private);
        payload["include_generated"] = Value::Bool(batch.include_generated);
        payload["declaration_chunk_size"] = serde_json::json!(batch.declaration_chunk_size);
        payload["declaration_parallelism"] = serde_json::json!(batch.declaration_parallelism);
        let request = Request::new(request_id(), Command::Index, payload);
        let mut adapter = |item: ProtocolItem| match item {
            ProtocolItem::Row(Row::Declaration(row)) => sink(IndexStreamItem::Declaration(row)),
            ProtocolItem::Row(Row::Feature(row)) => sink(IndexStreamItem::Feature(row)),
            ProtocolItem::Row(_) => Err(WorkerError::Protocol {
                message: "worker returned non-index row for index call".to_owned(),
            }),
            ProtocolItem::Event(event) => sink(IndexStreamItem::Event(event)),
            ProtocolItem::Diagnostic(diagnostic) => sink(IndexStreamItem::Diagnostic(diagnostic)),
            ProtocolItem::Complete => Ok(()),
        };
        let output = self.transport.call_stream(
            request,
            CallControl {
                timeout: self.timeout,
                cancelled: self.cancelled.clone(),
            },
            &mut adapter,
        )?;
        for event in &output.events {
            perf::record_worker_event(&event.phase, event.elapsed_ms, event.current);
        }
        Ok(WorkerCall {
            rows: Vec::new(),
            events: output.events,
            diagnostics: output.diagnostics,
        })
    }

    /// Run bounded semantic probes for candidate declaration pairs.
    pub fn probe_batch(&self, batch: ProbeBatch) -> Result<WorkerCall<ProbeResult>, WorkerError> {
        perf::record_count(CostClass::LeanSemantic, "worker.probe.batch", 1);
        perf::record_count(CostClass::LeanSemantic, "worker.probe.pairs", batch.pairs.len() as u64);
        let mut payload = protocol::modules_payload(&batch.workspace_root_string(), &batch.modules);
        payload["include_private"] = Value::Bool(batch.include_private);
        payload["include_generated"] = Value::Bool(batch.include_generated);
        payload["pairs"] = serde_json::json!(batch.pairs);
        if let Some(max_pairs) = batch.max_pairs {
            payload["max_pairs"] = serde_json::json!(max_pairs);
        }
        self.call(Request::new(request_id(), Command::Probe, payload))
    }

    fn call<T>(&self, request: Request) -> Result<WorkerCall<T>, WorkerError>
    where
        T: TryFrom<Row, Error = WorkerError>,
    {
        let output = self.transport.call(
            request,
            CallControl {
                timeout: self.timeout,
                cancelled: self.cancelled.clone(),
            },
        )?;
        for event in &output.events {
            perf::record_worker_event(&event.phase, event.elapsed_ms, event.current);
        }
        let rows = output
            .rows
            .into_iter()
            .map(T::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(WorkerCall {
            rows,
            events: output.events,
            diagnostics: output.diagnostics,
        })
    }
}

/// Worker result rows plus non-row events and diagnostics from a committed call.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerCall<T> {
    pub rows: Vec<T>,
    pub events: Vec<WorkerEvent>,
    pub diagnostics: Vec<WorkerDiagnostic>,
}

/// Input for import-once declaration and feature indexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexBatch {
    pub workspace_root: PathBuf,
    pub modules: Vec<ModuleDescriptor>,
    pub include_private: bool,
    pub include_generated: bool,
    pub declaration_chunk_size: usize,
    pub declaration_parallelism: usize,
}

impl IndexBatch {
    fn workspace_root_string(&self) -> String {
        self.workspace_root.to_string_lossy().into_owned()
    }
}

/// One streamed event from an import-once index command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum IndexStreamItem {
    Declaration(DeclarationRow),
    Feature(FeatureRow),
    Event(WorkerEvent),
    Diagnostic(WorkerDiagnostic),
}

/// Version and compatibility facts reported by the Lean worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerVersion {
    pub protocol_version: String,
    pub worker_version: String,
    pub lean_version: Option<String>,
    pub extract_version: String,
    pub features_version: String,
    pub probe_version: String,
    pub supported_commands: Vec<String>,
    pub supported_capabilities: Vec<String>,
}

impl WorkerVersion {
    /// Validate that this worker advertises every capability required by a
    /// caller before starting a semantic command that depends on them.
    pub fn require_capabilities(&self, required: &[String]) -> Result<(), WorkerError> {
        let missing = required
            .iter()
            .filter(|capability| !self.supported_capabilities.contains(capability))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(WorkerError::Protocol {
                message: format!("worker is missing required capabilities: {}", missing.join(", ")),
            })
        }
    }
}

/// One Lean declaration accepted by extraction filters.
///
/// Display fields are safe to show to users. Semantic comparison must use
/// feature rows, not `statement_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeclarationRow {
    pub declaration_id: String,
    pub origin: String,
    pub module: String,
    pub qualified_name: String,
    pub display_name: String,
    pub kind: String,
    pub visibility: String,
    pub modifiers: Vec<String>,
    pub source_span: Option<SourceSpan>,
    pub statement_text: String,
    pub docstring_text: Option<String>,
    pub definition_body_summary: Option<String>,
    pub status_flags: Vec<String>,
}

/// One Lean-owned feature row for retrieval and ranking.
///
/// Fingerprints and role keys are opaque equality keys. Callers may store and
/// compare them but must not parse or reconstruct them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureRow {
    pub declaration_id: String,
    pub feature_version: String,
    pub fingerprints: Fingerprints,
    pub role_features: Vec<RoleFeature>,
    pub binder_count: u64,
    pub low_signal_markers: Vec<String>,
}

/// Opaque semantic fingerprints for one declaration statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprints {
    pub statement: String,
    pub safe_binder_permutation: String,
    pub connective_shape: String,
    pub conclusion_shape: String,
}

/// One role-aware semantic feature key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleFeature {
    pub role: String,
    pub key: String,
    pub display: Option<String>,
}

/// Result of one bounded semantic probe pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub pair_id: String,
    pub left_declaration_id: String,
    pub right_declaration_id: String,
    pub status: String,
    pub same_statement: bool,
    pub same_up_to_safe_reordering: bool,
    pub connective_equivalent: bool,
    pub specializes_left_to_right: bool,
    pub specializes_right_to_left: bool,
    pub mutual_implication_shape: bool,
    pub same_reducible_definition: bool,
    pub message: Option<String>,
}

/// Progress event emitted by the worker during a committed call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerEvent {
    pub phase: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub module: Option<String>,
    pub declaration: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub message: String,
}

/// Structured diagnostic emitted by the worker or adapter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkerDiagnostic {
    pub code: String,
    pub message: String,
    pub fatal: bool,
    pub details: Option<Value>,
}

/// One module requested from a Lean worker capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleDescriptor {
    pub module: String,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_root: Option<PathBuf>,
}

/// Input for declaration extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractBatch {
    pub workspace_root: PathBuf,
    pub modules: Vec<ModuleDescriptor>,
    pub include_private: bool,
    pub include_generated: bool,
}

impl ExtractBatch {
    fn workspace_root_string(&self) -> String {
        self.workspace_root.to_string_lossy().into_owned()
    }
}

/// Input for semantic feature extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeaturesBatch {
    pub workspace_root: PathBuf,
    pub modules: Vec<ModuleDescriptor>,
    pub declaration_ids: Option<Vec<String>>,
    pub include_private: bool,
    pub include_generated: bool,
}

impl FeaturesBatch {
    fn workspace_root_string(&self) -> String {
        self.workspace_root.to_string_lossy().into_owned()
    }
}

/// Input for semantic pair probes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeBatch {
    pub workspace_root: PathBuf,
    pub modules: Vec<ModuleDescriptor>,
    pub include_private: bool,
    pub include_generated: bool,
    pub pairs: Vec<ProbePair>,
    pub max_pairs: Option<u64>,
}

impl ProbeBatch {
    fn workspace_root_string(&self) -> String {
        self.workspace_root.to_string_lossy().into_owned()
    }
}

/// One candidate pair to check with Lean-owned semantic probes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbePair {
    pub pair_id: String,
    pub left_declaration_id: String,
    pub right_declaration_id: String,
}

/// 1-based source span reported by Lean when available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub file: String,
    pub start: SourcePoint,
    pub end: SourcePoint,
}

/// 1-based source position reported by Lean when available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePoint {
    pub line: u64,
    pub column: u64,
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("worker protocol violation: {message}")]
    Protocol { message: String },

    #[error("worker returned a fatal diagnostic: {}", format_worker_diagnostics(.diagnostics))]
    WorkerDiagnostic { diagnostics: Vec<WorkerDiagnostic> },

    #[error("worker timed out after {timeout:?}")]
    Timeout { timeout: Duration },

    #[error("worker call was cancelled")]
    Cancelled,

    #[error("worker exited with status {status}")]
    NonZeroExit { status: i32, stderr: String },

    #[error("worker ended before complete: {}", format_worker_diagnostics(.diagnostics))]
    EofBeforeComplete { diagnostics: Vec<WorkerDiagnostic> },

    #[error("invalid worker JSON on line {line}")]
    InvalidJsonLine {
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("{message}")]
    Io {
        message: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not build Lean worker; status {status}: {diagnostic}")]
    BuildFailed { status: i32, diagnostic: String },
}

fn format_worker_diagnostics(diagnostics: &[WorkerDiagnostic]) -> String {
    if diagnostics.is_empty() {
        return "no structured diagnostic payload".to_owned();
    }
    diagnostics
        .iter()
        .map(|diagnostic| {
            let fatal = if diagnostic.fatal { " fatal" } else { "" };
            format!("{}{}: {}", diagnostic.code, fatal, diagnostic.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

impl TryFrom<Row> for WorkerVersion {
    type Error = WorkerError;

    fn try_from(row: Row) -> Result<Self, Self::Error> {
        match row {
            Row::Version(row) => Ok(row),
            _ => Err(WorkerError::Protocol {
                message: "worker returned non-version row for version call".to_owned(),
            }),
        }
    }
}

impl TryFrom<Row> for DeclarationRow {
    type Error = WorkerError;

    fn try_from(row: Row) -> Result<Self, Self::Error> {
        match row {
            Row::Declaration(row) => Ok(row),
            _ => Err(WorkerError::Protocol {
                message: "worker returned non-declaration row for extract call".to_owned(),
            }),
        }
    }
}

impl TryFrom<Row> for FeatureRow {
    type Error = WorkerError;

    fn try_from(row: Row) -> Result<Self, Self::Error> {
        match row {
            Row::Feature(row) => Ok(row),
            _ => Err(WorkerError::Protocol {
                message: "worker returned non-feature row for features call".to_owned(),
            }),
        }
    }
}

impl TryFrom<Row> for ProbeResult {
    type Error = WorkerError;

    fn try_from(row: Row) -> Result<Self, Self::Error> {
        match row {
            Row::Probe(row) => Ok(row),
            _ => Err(WorkerError::Protocol {
                message: "worker returned non-probe row for probe call".to_owned(),
            }),
        }
    }
}

fn request_id() -> String {
    let id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    format!("rust-worker-{id}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::protocol::ProtocolOutput;
    use super::transport::{CallControl, WorkerTransport};
    use super::{ExtractBatch, FeaturesBatch, ModuleDescriptor, ProbeBatch, ProbePair, WorkerClient};

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .to_path_buf()
    }

    fn tiny_root() -> PathBuf {
        repo_root().join("tests/fixtures/tiny")
    }

    fn tiny_basic() -> Vec<ModuleDescriptor> {
        vec![ModuleDescriptor {
            module: "Tiny.Basic".to_owned(),
            origin: "workspace".to_owned(),
            source_root: None,
        }]
    }

    #[test]
    fn public_client_version_returns_typed_version() {
        let client = WorkerClient::new();
        let call = client.version(tiny_root()).unwrap();
        let version = call.rows.first().unwrap();
        assert_eq!(version.protocol_version, "lean-dup.worker.v1");
        for command in ["extract", "features", "probe", "doctor", "version"] {
            assert!(version.supported_commands.iter().any(|value| value == command));
        }
    }

    #[test]
    fn indexing_client_uses_long_timeout_policy() {
        assert_eq!(WorkerClient::new().timeout, super::DEFAULT_WORKER_TIMEOUT);
        assert_eq!(WorkerClient::for_indexing().timeout, super::INDEX_WORKER_TIMEOUT);
        assert!(WorkerClient::for_indexing().timeout > WorkerClient::new().timeout);
    }

    #[test]
    fn unknown_required_capability_is_rejected_before_command_use() {
        let version = super::WorkerVersion {
            protocol_version: "lean-dup.worker.v1".to_owned(),
            worker_version: "0.1.0".to_owned(),
            lean_version: None,
            extract_version: "e".to_owned(),
            features_version: "f".to_owned(),
            probe_version: "p".to_owned(),
            supported_commands: vec!["version".to_owned()],
            supported_capabilities: vec![],
        };
        let required = vec!["future-capability".to_owned()];
        assert!(matches!(
            version.require_capabilities(&required),
            Err(super::WorkerError::Protocol { .. })
        ));
    }

    #[test]
    fn public_client_extract_returns_typed_declarations() {
        let client = WorkerClient::new();
        let call = client
            .extract_batch(ExtractBatch {
                workspace_root: tiny_root(),
                modules: tiny_basic(),
                include_private: true,
                include_generated: false,
            })
            .unwrap();
        assert!(call.rows.iter().any(|row| row.qualified_name == "Tiny.same_left"));
    }

    #[test]
    fn public_client_features_returns_typed_feature_rows() {
        let client = WorkerClient::new();
        let declarations = client
            .extract_batch(ExtractBatch {
                workspace_root: tiny_root(),
                modules: tiny_basic(),
                include_private: true,
                include_generated: false,
            })
            .unwrap();
        let ids = declarations
            .rows
            .iter()
            .filter(|row| row.qualified_name == "Tiny.same_left")
            .map(|row| row.declaration_id.clone())
            .collect();
        let call = client
            .features_batch(FeaturesBatch {
                workspace_root: tiny_root(),
                modules: tiny_basic(),
                declaration_ids: Some(ids),
                include_private: true,
                include_generated: false,
            })
            .unwrap();
        assert_eq!(call.rows.len(), 1);
        assert_eq!(call.rows[0].feature_version, "features.roles.v3");
    }

    #[test]
    fn public_client_probe_returns_typed_probe_results() {
        let client = WorkerClient::new();
        let left = "workspace:Tiny.Basic:Tiny.same_left".to_owned();
        let right = "workspace:Tiny.Basic:Tiny.same_right".to_owned();
        let call = client
            .probe_batch(ProbeBatch {
                workspace_root: tiny_root(),
                modules: tiny_basic(),
                include_private: true,
                include_generated: false,
                pairs: vec![ProbePair {
                    pair_id: "p1".to_owned(),
                    left_declaration_id: left,
                    right_declaration_id: right,
                }],
                max_pairs: Some(1),
            })
            .unwrap();
        assert_eq!(call.rows.len(), 1);
        assert_eq!(call.rows[0].pair_id, "p1");
    }

    #[test]
    fn probe_batch_serializes_extraction_filters() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = WorkerClient {
            transport: Box::new(CapturingTransport {
                requests: requests.clone(),
            }),
            timeout: Duration::from_secs(1),
            cancelled: Arc::new(AtomicBool::new(false)),
        };

        let call = client
            .probe_batch(ProbeBatch {
                workspace_root: tiny_root(),
                modules: tiny_basic(),
                include_private: true,
                include_generated: false,
                pairs: vec![ProbePair {
                    pair_id: "p1".to_owned(),
                    left_declaration_id: "left".to_owned(),
                    right_declaration_id: "right".to_owned(),
                }],
                max_pairs: Some(1),
            })
            .unwrap();

        assert!(call.rows.is_empty());
        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0]["include_private"], true);
        assert_eq!(captured[0]["include_generated"], false);
    }

    struct CapturingTransport {
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    impl WorkerTransport for CapturingTransport {
        fn call(
            &self,
            request: super::protocol::Request,
            _control: CallControl,
        ) -> Result<ProtocolOutput, super::WorkerError> {
            self.requests.lock().unwrap().push(request.to_json());
            Ok(ProtocolOutput {
                rows: Vec::new(),
                events: Vec::new(),
                diagnostics: Vec::new(),
            })
        }

        fn call_stream(
            &self,
            request: super::protocol::Request,
            _control: CallControl,
            _sink: &mut dyn FnMut(super::protocol::ProtocolItem) -> Result<(), super::WorkerError>,
        ) -> Result<ProtocolOutput, super::WorkerError> {
            self.requests.lock().unwrap().push(request.to_json());
            Ok(ProtocolOutput {
                rows: Vec::new(),
                events: Vec::new(),
                diagnostics: Vec::new(),
            })
        }
    }
}
