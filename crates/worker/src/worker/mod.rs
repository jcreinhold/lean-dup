mod engine;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::perf::{self, CostClass};

use self::engine::WorkerEngine;

const DEFAULT_WORKER_TIMEOUT: Duration = Duration::from_secs(60);
const INDEX_WORKER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Client for Lean semantic worker capabilities.
///
/// Callers ask for semantic facts by capability: worker version, declaration
/// extraction, feature extraction, and semantic probes. The client returns typed
/// rows, progress events, and diagnostics; callers do not receive lifecycle
/// handles, protocol envelopes, transport frames, or child diagnostics as
/// machine data.
pub struct WorkerClient {
    engine: WorkerEngine,
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
            engine: WorkerEngine::pool(),
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
            engine: WorkerEngine::pool(),
            timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Request cancellation for calls started from this client.
    ///
    /// Cancellation is cooperative at the Rust worker boundary: a running
    /// request is interrupted and the call returns a structured cancellation
    /// error.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Return the worker and semantic algorithm versions for a Lake workspace.
    pub fn version(&self, workspace_root: PathBuf) -> Result<WorkerCall<WorkerVersion>, WorkerError> {
        let call = self
            .engine
            .identity(workspace_root, self.timeout, self.cancelled.clone())?;
        Ok(WorkerCall {
            rows: call.rows.into_iter().map(|identity| identity.semantic).collect(),
            events: call.events,
            diagnostics: call.diagnostics,
        })
    }

    /// Return the worker version facts together with the worker substrate facts
    /// that legitimately affect cached results.
    ///
    /// The index cache layer folds the substrate facts into its cache key;
    /// callers that only display version facts use [`WorkerClient::version`].
    pub fn worker_identity(&self, workspace_root: PathBuf) -> Result<WorkerCall<WorkerIdentity>, WorkerError> {
        self.engine
            .identity(workspace_root, self.timeout, self.cancelled.clone())
    }

    /// Extract typed declaration rows for a batch of Lean modules.
    pub fn extract_batch(&self, batch: ExtractBatch) -> Result<WorkerCall<DeclarationRow>, WorkerError> {
        self.engine.extract(batch, self.timeout, self.cancelled.clone())
    }

    /// Compute Lean-owned semantic feature rows for a module batch.
    pub fn features_batch(&self, batch: FeaturesBatch) -> Result<WorkerCall<FeatureRow>, WorkerError> {
        self.engine.features(batch, self.timeout, self.cancelled.clone())
    }

    /// Stream declaration and feature rows from one import-once index command.
    ///
    /// The caller receives semantic rows and progress events as they arrive.
    /// The worker client still owns worker lifetime, cancellation, and
    /// structured diagnostic handling.
    pub fn index_stream(
        &self,
        batch: IndexBatch,
        sink: &mut dyn FnMut(IndexStreamItem) -> Result<(), WorkerError>,
    ) -> Result<WorkerCall<()>, WorkerError> {
        self.engine
            .index_stream(batch, self.timeout, self.cancelled.clone(), sink)
    }

    /// Run bounded semantic probes for candidate declaration pairs.
    pub fn probe_batch(&self, batch: ProbeBatch) -> Result<WorkerCall<ProbeResult>, WorkerError> {
        perf::record_count(CostClass::LeanSemantic, "worker.probe.batch", 1);
        perf::record_count(CostClass::LeanSemantic, "worker.probe.pairs", batch.pairs.len() as u64);
        self.engine.probe(batch, self.timeout, self.cancelled.clone())
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

/// Worker substrate facts that legitimately affect cached results.
///
/// These describe the worker/runtime contract carried by the
/// `lean-rs-worker-parent` pool handshake — not semantic algorithm versions and
/// not ephemeral pool state. The index cache key folds them in so a change to
/// the worker transport substrate invalidates stale entries; pool ids, pids, and
/// queue counters are deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerSubstrateFacts {
    /// The `lean-rs-worker` transport framing protocol version (not the
    /// `lean-dup.worker.v1` schema string).
    pub protocol_version: u16,
    /// The pooled worker runtime version reported at handshake.
    pub worker_version: String,
}

/// Worker identity: the semantic version facts plus the worker substrate facts.
///
/// `semantic` is the unchanged [`WorkerVersion`] DTO callers display; `substrate`
/// carries the pool runtime facts the index cache key depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerIdentity {
    pub semantic: WorkerVersion,
    pub substrate: WorkerSubstrateFacts,
}

/// One Lean declaration accepted by extraction filters.
///
/// Display fields are safe to show to users. Semantic comparison must use
/// feature rows, not `statement_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Input for semantic feature extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeaturesBatch {
    pub workspace_root: PathBuf,
    pub modules: Vec<ModuleDescriptor>,
    pub declaration_ids: Option<Vec<String>>,
    pub include_private: bool,
    pub include_generated: bool,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command as ProcessCommand;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use super::engine::WorkerEngine;
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

    fn tiny_extract_batch() -> ExtractBatch {
        ExtractBatch {
            workspace_root: tiny_root(),
            modules: tiny_basic(),
            include_private: true,
            include_generated: false,
        }
    }

    fn ensure_worker_child_built() {
        static BUILT: OnceLock<()> = OnceLock::new();
        BUILT.get_or_init(|| {
            let status = ProcessCommand::new("cargo")
                .args(["build", "-p", "lean-dup-worker-child", "--locked"])
                .current_dir(repo_root())
                .status()
                .unwrap();
            assert!(status.success(), "failed to build lean-dup-worker-child");
        });
    }

    #[test]
    fn public_client_version_returns_typed_version() {
        ensure_worker_child_built();
        let client = WorkerClient::new();
        let call = client.version(tiny_root()).unwrap();
        let version = call.rows.first().unwrap();
        assert_eq!(version.protocol_version, "lean-dup.worker.v1");
        for command in ["extract", "features", "probe", "doctor", "version"] {
            assert!(version.supported_commands.iter().any(|value| value == command));
        }
    }

    #[test]
    fn worker_identity_reports_substrate_facts() {
        ensure_worker_child_built();
        let call = WorkerClient::new().worker_identity(tiny_root()).unwrap();
        let identity = call.rows.first().unwrap();
        assert_eq!(identity.semantic.protocol_version, "lean-dup.worker.v1");
        // The substrate protocol version is the pool transport framing version,
        // distinct from the `lean-dup.worker.v1` schema string.
        assert!(!identity.substrate.worker_version.is_empty());
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
        ensure_worker_child_built();
        let client = WorkerClient::new();
        let call = client.extract_batch(tiny_extract_batch()).unwrap();
        assert!(call.rows.iter().any(|row| row.qualified_name == "Tiny.same_left"));
    }

    #[test]
    fn public_client_features_returns_typed_feature_rows() {
        ensure_worker_child_built();
        let client = WorkerClient::new();
        let declarations = client.extract_batch(tiny_extract_batch()).unwrap();
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
        ensure_worker_child_built();
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
            engine: WorkerEngine::fake(requests.clone()),
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
}
