//! Capability request payloads and typed response/summary structs.
//!
//! The pool engine drives Lean through named `@[export]` capabilities, so a
//! request is just the command payload object the Lean side parses (modules,
//! filters, pairs, chunk sizes) — there is no transport envelope, request id, or
//! command name to inject. The typed `version` response and the shared streaming
//! `metadata` summary are decoded here.

use serde::Deserialize;
use serde_json::{Value, json};

use super::super::{ExtractBatch, FeaturesBatch, IndexBatch, ModuleDescriptor, ProbeBatch, WorkerVersion};

/// Build the base payload shared by every command: the audited workspace root
/// and the requested module descriptors.
fn modules_payload(workspace_root: &str, modules: &[ModuleDescriptor]) -> Value {
    json!({
        "workspace_root": workspace_root,
        "modules": modules,
    })
}

/// The `version` command takes no inputs the Lean side reads; the workspace root
/// is carried only so the host session keys the warm worker to this workspace.
pub(super) fn version_request(workspace_root: &str) -> Value {
    json!({ "workspace_root": workspace_root })
}

/// Attach the optional per-declaration heartbeat budget when the caller set one.
/// Omitted entirely when `None`, so the worker keeps its default; an additive
/// `lean-dup.worker.v1` payload field.
fn set_max_heartbeats(payload: &mut Value, max_heartbeats: Option<u64>) {
    if let Some(budget) = max_heartbeats {
        payload["max_heartbeats"] = json!(budget);
    }
}

pub(super) fn extract_request(batch: &ExtractBatch) -> Value {
    let mut payload = modules_payload(&batch.workspace_root.to_string_lossy(), &batch.modules);
    payload["include_private"] = Value::Bool(batch.include_private);
    payload["include_generated"] = Value::Bool(batch.include_generated);
    set_max_heartbeats(&mut payload, batch.max_heartbeats);
    payload
}

pub(super) fn features_request(batch: &FeaturesBatch) -> Value {
    let mut payload = modules_payload(&batch.workspace_root.to_string_lossy(), &batch.modules);
    payload["include_private"] = Value::Bool(batch.include_private);
    payload["include_generated"] = Value::Bool(batch.include_generated);
    if let Some(declaration_ids) = &batch.declaration_ids {
        payload["declaration_ids"] = json!(declaration_ids);
    }
    set_max_heartbeats(&mut payload, batch.max_heartbeats);
    payload
}

pub(super) fn probe_request(batch: &ProbeBatch) -> Value {
    let mut payload = modules_payload(&batch.workspace_root.to_string_lossy(), &batch.modules);
    payload["include_private"] = Value::Bool(batch.include_private);
    payload["include_generated"] = Value::Bool(batch.include_generated);
    payload["pairs"] = json!(batch.pairs);
    if let Some(max_pairs) = batch.max_pairs {
        payload["max_pairs"] = json!(max_pairs);
    }
    set_max_heartbeats(&mut payload, batch.max_heartbeats);
    payload
}

pub(super) fn index_request(batch: &IndexBatch) -> Value {
    let mut payload = modules_payload(&batch.workspace_root.to_string_lossy(), &batch.modules);
    payload["include_private"] = Value::Bool(batch.include_private);
    payload["include_generated"] = Value::Bool(batch.include_generated);
    payload["declaration_chunk_size"] = json!(batch.declaration_chunk_size);
    payload["declaration_parallelism"] = json!(batch.declaration_parallelism);
    set_max_heartbeats(&mut payload, batch.max_heartbeats);
    payload
}

/// Typed decode of the `version_result` payload returned by the `version`
/// capability. The nested `semantic_versions` object is flattened into the
/// public [`WorkerVersion`] DTO.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerVersionPayload {
    protocol_version: String,
    worker_version: String,
    lean_version: Option<String>,
    semantic_versions: SemanticVersionsPayload,
    supported_commands: Vec<String>,
    supported_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticVersionsPayload {
    extract: String,
    features: String,
    probe: String,
}

impl From<WorkerVersionPayload> for WorkerVersion {
    fn from(payload: WorkerVersionPayload) -> Self {
        Self {
            protocol_version: payload.protocol_version,
            worker_version: payload.worker_version,
            lean_version: payload.lean_version,
            extract_version: payload.semantic_versions.extract,
            features_version: payload.semantic_versions.features,
            probe_version: payload.semantic_versions.probe,
            supported_commands: payload.supported_commands,
            supported_capabilities: payload.supported_capabilities,
        }
    }
}

/// Terminal `metadata` summary frame every streaming capability emits. The
/// engine reads `ok`/`message` to map a logical failure to a fatal worker
/// diagnostic; other summary fields are advisory and ignored.
#[derive(Debug, Default, Deserialize)]
pub(super) struct CapabilityStreamSummary {
    #[serde(default)]
    pub(super) ok: bool,
    #[serde(default)]
    pub(super) message: Option<String>,
    /// Declarations the worker skipped because their elaboration exceeded the
    /// heartbeat budget. Advisory; surfaced so the count is never silently lost.
    #[serde(default)]
    pub(super) skipped: u64,
}
