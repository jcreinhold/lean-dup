//! `LeanDupCapabilityRuntime` — the one seam that owns *how the installed
//! `LeanDup` capability is located and loaded*: the per-toolchain artifact
//! manifest, the worker-child binary, the Lean sysroot, and the command export
//! declarations.
//!
//! Steady-state note (see `docs/architecture/shared-search-adoption.md`): the
//! capability dylib and worker-child are no longer built at crate-compile time.
//! `cargo install lean-dup` ships the parent Lean-free; `lean-dup install-worker`
//! builds the toolchain-specific artifacts on the user's machine into
//! `<install_root>/<toolchain-id>/`. This module resolves them per audited
//! workspace through [`crate::toolchain::resolve_installed_worker`] — the audited
//! project's `.olean` files dictate which toolchain's worker must load them — so
//! the command path in `pool.rs` stays independent of where artifacts live.

use std::path::PathBuf;
use std::time::Duration;

use lean_rs_worker_parent::{LeanWorkerCapabilityBuilder, LeanWorkerChild};
use lean_toolchain::LeanBuiltCapability;

use super::map_parent_error;
use crate::toolchain::resolve_installed_worker;
use crate::worker::WorkerError;

/// Export symbols advertised by the `LeanDup` capability. These names are the
/// capability ABI; they live here, beside the runtime that declares them to the
/// builder, and are referenced by the command calls in `pool.rs`.
pub(super) const VERSION_EXPORT: &str = "lean_dup_capability_version";
pub(super) const EXTRACT_EXPORT: &str = "lean_dup_capability_extract";
pub(super) const FEATURES_EXPORT: &str = "lean_dup_capability_features";
pub(super) const PROBE_EXPORT: &str = "lean_dup_capability_probe";
pub(super) const INDEX_EXPORT: &str = "lean_dup_capability_index";

/// Override for the negotiated worker frame cap (bytes). The parent clamps the
/// value to the protocol's `[MIN_FRAME_BYTES, MAX_FRAME_BYTES_HARD_CAP]` window.
const MAX_FRAME_BYTES_ENV: &str = "LEAN_DUP_MAX_FRAME_BYTES";

/// Default negotiated frame cap: headroom above the protocol's 1 MiB default.
/// The per-request module manifest is the dominant frame, and `modules_payload`
/// already hoists its repeated constants so a mathlib-scale request is well under
/// 1 MiB — this margin covers far larger corpora without re-breaking, while
/// staying finite (never the 256 MiB hard cap) so the parent's largest single
/// `read_frame` allocation stays bounded.
const DEFAULT_MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Resolves and loads the installed `LeanDup` worker for each audited workspace.
///
/// Stateless: the artifacts live per-toolchain on disk (provisioned by
/// `install-worker`), and the toolchain is decided by the audited workspace, so
/// there is nothing to cache across workspaces. Resolution happens in
/// [`Self::builder`].
#[derive(Debug)]
pub(super) struct LeanDupCapabilityRuntime;

impl LeanDupCapabilityRuntime {
    /// Construct the runtime. Resolution is deferred to [`Self::builder`], which
    /// is where the audited workspace (and thus its toolchain) is known.
    pub(super) fn installed() -> Self {
        Self
    }

    /// Produce a capability builder for one audited workspace and timeout, with
    /// every `LeanDup` command export registered so the warm session serves all
    /// of them. The pool session key embeds `import_workspace_root` (not the
    /// export set), so registering all exports keeps the warm session shared
    /// across commands while distinct workspaces stay isolated.
    ///
    /// Resolves the per-toolchain worker for `workspace_root` first; a missing,
    /// stale, or unusable install surfaces as [`WorkerError::NotProvisioned`]
    /// whose message names the `install-worker` command that fixes it.
    pub(super) fn builder(
        &self,
        workspace_root: PathBuf,
        timeout: Duration,
    ) -> Result<LeanWorkerCapabilityBuilder, WorkerError> {
        let installed = resolve_installed_worker(&workspace_root).map_err(|error| WorkerError::NotProvisioned {
            message: error.to_string(),
        })?;
        let built = LeanBuiltCapability::manifest_path(installed.capability_manifest);
        let max_frame_bytes = std::env::var(MAX_FRAME_BYTES_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(DEFAULT_MAX_FRAME_BYTES);
        LeanWorkerCapabilityBuilder::from_built_capability(&built, Vec::<String>::new())
            .map(|builder| {
                builder
                    .worker_child(LeanWorkerChild::for_toolchain(
                        installed.worker_child,
                        installed.lean_sysroot,
                    ))
                    .json_command_export(VERSION_EXPORT)
                    .streaming_command_export(EXTRACT_EXPORT)
                    .streaming_command_export(FEATURES_EXPORT)
                    .streaming_command_export(PROBE_EXPORT)
                    .streaming_command_export(INDEX_EXPORT)
                    .request_timeout(timeout)
                    .import_workspace_root(workspace_root)
                    .max_frame_bytes(max_frame_bytes)
            })
            .map_err(map_parent_error)
    }
}
