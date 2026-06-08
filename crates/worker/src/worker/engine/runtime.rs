//! `LeanDupCapabilityRuntime` — the one seam that owns *how the `LeanDup`
//! capability is produced and loaded*: the built-capability manifest, the
//! worker-child binary, and the command export declarations.
//!
//! Steady-state note (see `docs/architecture/worker-migration-spike.md`): today
//! the manifest is built from a sibling Lake checkout, and its semantic-search
//! dylib dependency is injected by `crates/worker/build.rs`. That packaging
//! coupling is deliberately confined to this module. The intended end state is a
//! package-owned runtime crate that ships/materializes/builds the Lean payload
//! and returns a `LeanBuiltCapability`; adopting it would replace only
//! [`LeanDupCapabilityRuntime::from_build_manifest`] with a `build_cached(...)`
//! call. The command path in `pool.rs` does not change.

use std::path::PathBuf;
use std::time::Duration;

use lean_rs_worker_parent::{LeanWorkerCapabilityBuilder, LeanWorkerChild};
use lean_toolchain::LeanBuiltCapability;

use super::map_parent_error;
use crate::worker::WorkerError;

/// Export symbols advertised by the `LeanDup` capability. These names are the
/// capability ABI; they live here, beside the runtime that declares them to the
/// builder, and are referenced by the command calls in `pool.rs`.
pub(super) const VERSION_EXPORT: &str = "lean_dup_capability_version";
pub(super) const EXTRACT_EXPORT: &str = "lean_dup_capability_extract";
pub(super) const FEATURES_EXPORT: &str = "lean_dup_capability_features";
pub(super) const PROBE_EXPORT: &str = "lean_dup_capability_probe";
pub(super) const INDEX_EXPORT: &str = "lean_dup_capability_index";

/// The sibling binary that links `libleanshared` and hosts the capability.
const WORKER_CHILD: &str = "lean-dup-worker-child";

/// Owns capability production for the `LeanDup` worker.
#[derive(Debug)]
pub(super) struct LeanDupCapabilityRuntime {
    manifest_path: &'static str,
}

impl LeanDupCapabilityRuntime {
    /// Load the runtime from the capability manifest emitted by the worker
    /// crate's build script.
    pub(super) fn from_build_manifest() -> Self {
        Self {
            manifest_path: env!("LEAN_RS_CAPABILITY_LEAN_DUP_MANIFEST"),
        }
    }

    /// Produce a capability builder for one audited workspace and timeout, with
    /// every `LeanDup` command export registered so the warm session serves all
    /// of them. The pool session key embeds `import_workspace_root` (not the
    /// export set), so registering all exports keeps the warm session shared
    /// across commands while distinct workspaces stay isolated.
    pub(super) fn builder(
        &self,
        workspace_root: PathBuf,
        timeout: Duration,
    ) -> Result<LeanWorkerCapabilityBuilder, WorkerError> {
        let built = LeanBuiltCapability::manifest_path(self.manifest_path);
        LeanWorkerCapabilityBuilder::from_built_capability(&built, Vec::<String>::new())
            .map(|builder| {
                builder
                    .worker_child(LeanWorkerChild::sibling(WORKER_CHILD))
                    .json_command_export(VERSION_EXPORT)
                    .streaming_command_export(EXTRACT_EXPORT)
                    .streaming_command_export(FEATURES_EXPORT)
                    .streaming_command_export(PROBE_EXPORT)
                    .streaming_command_export(INDEX_EXPORT)
                    .request_timeout(timeout)
                    .import_workspace_root(workspace_root)
            })
            .map_err(map_parent_error)
    }
}
