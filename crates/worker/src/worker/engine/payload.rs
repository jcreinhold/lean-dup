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
///
/// A request covers one audited corpus, so every descriptor carries the same
/// `origin` and `source_root`. Emitting those per module repeats two constant
/// strings once per entry — for a mathlib-scale request (8k+ modules) that
/// repetition is ~61% of the frame and pushes it past the transport cap. When
/// the descriptors are uniform (the universal case) we hoist `origin` and
/// `source_root` to the top level and stream the modules as bare name strings;
/// the Lean parsers read the hoisted defaults and still accept per-entry objects
/// for any future non-uniform caller.
fn modules_payload(workspace_root: &str, modules: &[ModuleDescriptor]) -> Value {
    let mut payload = json!({ "workspace_root": workspace_root });
    let uniform = modules.first().map(|first| {
        modules
            .iter()
            .all(|m| m.origin == first.origin && m.source_root == first.source_root)
    });
    match (uniform, modules.first()) {
        (Some(true), Some(first)) => {
            payload["modules_origin"] = json!(first.origin);
            if let Some(source_root) = &first.source_root {
                payload["modules_source_root"] = json!(source_root);
            }
            payload["modules"] = Value::Array(modules.iter().map(|m| json!(m.module)).collect());
        }
        _ => {
            payload["modules"] = json!(modules);
        }
    }
    payload
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn module(name: &str, origin: &str, source_root: Option<&str>) -> ModuleDescriptor {
        ModuleDescriptor {
            module: name.to_owned(),
            origin: origin.to_owned(),
            source_root: source_root.map(PathBuf::from),
        }
    }

    #[test]
    fn uniform_modules_hoist_constants_and_stream_names() {
        // The mathlib shape: every descriptor shares origin + source_root. The
        // constants must be hoisted out of the per-module entries (which are then
        // bare name strings), so the request frame does not repeat them per module.
        let modules = vec![
            module("Mathlib.A", "mathlib", Some("/pkgs/mathlib")),
            module("Mathlib.B", "mathlib", Some("/pkgs/mathlib")),
        ];
        let payload = modules_payload("/ws", &modules);
        assert_eq!(payload["modules_origin"], "mathlib");
        assert_eq!(payload["modules_source_root"], "/pkgs/mathlib");
        assert_eq!(payload["modules"], json!(["Mathlib.A", "Mathlib.B"]));

        // The hoisted form must be strictly smaller than the per-object form would
        // be — that shrinkage is the whole point (it keeps mathlib under the cap).
        let per_object = serde_json::to_string(&json!({ "workspace_root": "/ws", "modules": modules })).unwrap();
        let hoisted = serde_json::to_string(&payload).unwrap();
        assert!(
            hoisted.len() < per_object.len(),
            "hoisted {} !< per-object {}",
            hoisted.len(),
            per_object.len()
        );
    }

    #[test]
    fn workspace_modules_without_source_root_omit_the_hoisted_key() {
        let modules = vec![module("Tiny.Basic", "workspace", None)];
        let payload = modules_payload("/ws", &modules);
        assert_eq!(payload["modules_origin"], "workspace");
        assert!(payload.get("modules_source_root").is_none());
        assert_eq!(payload["modules"], json!(["Tiny.Basic"]));
    }

    #[test]
    fn mixed_descriptors_fall_back_to_per_object_entries() {
        // No current caller is non-uniform, but the encoder must not silently drop
        // a differing origin/source_root: it emits full objects instead of names.
        let modules = vec![
            module("A", "mathlib", Some("/pkgs/mathlib")),
            module("B", "workspace", None),
        ];
        let payload = modules_payload("/ws", &modules);
        assert!(payload.get("modules_origin").is_none());
        assert_eq!(payload["modules"][0]["origin"], "mathlib");
        assert_eq!(payload["modules"][1]["origin"], "workspace");
    }
}
