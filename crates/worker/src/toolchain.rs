//! Per-toolchain worker provisioning and runtime resolution.
//!
//! `lean-dup` ships the parent CLI Lean-free; the toolchain-specific artifacts —
//! the `lean-dup-worker-child` binary, the `LeanDup` capability dylib, and its
//! dependency dylibs — are built on the user's machine by `lean-dup
//! install-worker` into `<install_root>/<toolchain-id>/`. This module owns that
//! install layout and the two operations over it:
//!
//! - **resolution** ([`resolve_installed_worker`]): given an audited workspace,
//!   read its `lean-toolchain` pin and return the matching [`InstalledWorker`],
//!   or a [`ProvisionError`] whose message names the exact `install-worker`
//!   command that produces it. The audited project's `.olean` files are loaded
//!   by the worker's Lean runtime, so the worker must match *that project's*
//!   toolchain — not a single global pin.
//! - **provenance** ([`WorkerSidecar`]): the `worker.json` record `install-worker`
//!   writes beside the artifacts and resolution reads back, so header drift
//!   (the toolchain's `lean.h` changing under a built worker) and a failed
//!   post-build smoke test become refuse-with-rebuild verdicts.
//!
//! The install side (building the artifacts) lives in the CLI's `install-worker`
//! command; this module provides the primitives it records through.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `lean-dup`'s development-pinned toolchain: what `<repo>/lean` builds against
/// and the default when an audited workspace has no readable `lean-toolchain`.
/// A drift test asserts it equals `<repo>/lean/lean-toolchain`.
pub const PINNED_TOOLCHAIN: &str = "leanprover/lean4:v4.32.0-rc1";

/// File name of the per-toolchain worker-child binary inside an install dir.
pub const WORKER_FILE_NAME: &str = "lean-dup-worker-child";

/// Developer/CI override pointing the parent at an install dir outside the
/// standard `<data_local>/lean-dup/workers` layout.
pub const WORKERS_DIR_ENV: &str = "LEAN_DUP_WORKERS_DIR";

/// Provenance sidecar file name inside an install dir.
const SIDECAR_FILE_NAME: &str = "worker.json";

/// Canonical short form of a Lean toolchain pin (e.g. `v4.32.0-rc1`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolchainId(String);

impl ToolchainId {
    /// Parse a `lean-toolchain` line: either the elan-style
    /// `leanprover/lean4:<id>` or the bare `<id>` short form.
    ///
    /// # Errors
    ///
    /// [`ToolchainError::Unparseable`] if empty, whitespace-bearing, or naming a
    /// Lean fork we do not understand.
    pub fn parse(raw: &str) -> Result<Self, ToolchainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ToolchainError::Unparseable(raw.to_owned()));
        }
        let short = if let Some(rest) = trimmed.strip_prefix("leanprover/lean4:") {
            rest
        } else if trimmed.contains(':') || trimmed.contains('/') {
            return Err(ToolchainError::Unparseable(raw.to_owned()));
        } else {
            trimmed
        };
        if short.is_empty() || short.chars().any(char::is_whitespace) {
            return Err(ToolchainError::Unparseable(raw.to_owned()));
        }
        Ok(Self(short.to_owned()))
    }

    /// Read `<root>/lean-toolchain` and parse it.
    ///
    /// # Errors
    ///
    /// [`ToolchainError::FileMissing`] if absent/unreadable; forwards
    /// [`Self::parse`] otherwise.
    pub fn from_lake_root(root: &Path) -> Result<Self, ToolchainError> {
        let path = root.join("lean-toolchain");
        let contents = std::fs::read_to_string(&path).map_err(|_| ToolchainError::FileMissing(path.clone()))?;
        Self::parse(&contents)
    }

    /// `lean-dup`'s development pin, used as the resolution fallback. The const
    /// is under our control, so a parse failure is impossible in practice; the
    /// `unwrap_or_else` keeps the function total without a panic.
    #[must_use]
    pub fn pinned() -> Self {
        Self::parse(PINNED_TOOLCHAIN).unwrap_or_else(|_| Self("v4.32.0-rc1".to_owned()))
    }

    /// Resolved path to the elan toolchain root
    /// (`~/.elan/toolchains/leanprover--lean4---<id>`).
    ///
    /// # Errors
    ///
    /// [`ToolchainError::ElanMissing`] if the directory is absent.
    pub fn elan_dir(&self) -> Result<PathBuf, ToolchainError> {
        let dir = self.elan_dir_path()?;
        if dir.is_dir() {
            Ok(dir)
        } else {
            Err(ToolchainError::ElanMissing {
                toolchain: self.clone(),
                elan_dir: dir,
            })
        }
    }

    fn elan_dir_path(&self) -> Result<PathBuf, ToolchainError> {
        let home = dirs::home_dir().ok_or_else(|| ToolchainError::ElanMissing {
            toolchain: self.clone(),
            elan_dir: PathBuf::from(format!("~/.elan/toolchains/leanprover--lean4---{}", self.0)),
        })?;
        Ok(home
            .join(".elan")
            .join("toolchains")
            .join(format!("leanprover--lean4---{}", self.0)))
    }

    /// The elan-style label (`leanprover/lean4:<id>`) Lake and the toolchain
    /// materializers expect.
    #[must_use]
    pub fn elan_label(&self) -> String {
        format!("leanprover/lean4:{}", self.0)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolchainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `<data_local>/lean-dup/workers` — the per-toolchain install root.
///
/// Falls back to the current directory if no data dir can be located; callers
/// then fail soon after with a concrete [`ProvisionError::NotInstalled`].
#[must_use]
pub fn install_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lean-dup")
        .join("workers")
}

/// Install directory for one toolchain, honoring [`WORKERS_DIR_ENV`].
///
/// The override points directly at a single-toolchain install dir (bare layout),
/// used by dev/CI to redirect provisioning out of the user data dir; otherwise
/// the per-toolchain subdir under [`install_root`] is used. Both `install-worker`
/// (write) and [`resolve_installed_worker`] (read) go through here, so they never
/// disagree on where a toolchain's artifacts live.
#[must_use]
pub fn install_dir(id: &ToolchainId) -> PathBuf {
    std::env::var_os(WORKERS_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| install_root().join(id.as_str()))
}

/// `lean-dup install-worker --toolchain <id>` — the command that produces a
/// missing or stale worker for `id`.
fn install_cmd(id: &ToolchainId) -> String {
    format!("lean-dup install-worker --toolchain {}", id.as_str())
}

/// The resolved, ready-to-spawn worker artifacts for one toolchain.
#[derive(Clone, Debug)]
pub struct InstalledWorker {
    /// The `lean-dup-worker-child` binary that links `libleanshared`.
    pub worker_child: PathBuf,
    /// The `LeanDup` capability artifact manifest the parent loads.
    pub capability_manifest: PathBuf,
    /// The Lean sysroot the child is spawned with (`LEAN_SYSROOT`).
    pub lean_sysroot: PathBuf,
}

/// Resolve the installed worker for the toolchain `workspace_root` pins.
///
/// Reads `<workspace_root>/lean-toolchain` (falling back to [`PINNED_TOOLCHAIN`]
/// when absent), then resolves `<install_root>/<id>/` — or the
/// [`WORKERS_DIR_ENV`] override. A missing install, header drift, or a recorded
/// smoke failure each produce a [`ProvisionError`] whose message names the
/// `install-worker` command that fixes it.
///
/// # Errors
///
/// [`ProvisionError`] when no usable worker is installed for the pin.
pub fn resolve_installed_worker(workspace_root: &Path) -> Result<InstalledWorker, ProvisionError> {
    let id = ToolchainId::from_lake_root(workspace_root).unwrap_or_else(|_| ToolchainId::pinned());
    resolve_in(&install_dir(&id), &id)
}

/// Resolution core over a concrete install dir, factored out so tests drive the
/// not-installed/stale/unusable verdicts without mutating the environment.
fn resolve_in(install_dir: &Path, id: &ToolchainId) -> Result<InstalledWorker, ProvisionError> {
    let worker_child = install_dir.join(WORKER_FILE_NAME);
    let Some(sidecar) = WorkerSidecar::load(install_dir) else {
        return Err(ProvisionError::NotInstalled {
            toolchain: id.clone(),
            install_cmd: install_cmd(id),
        });
    };
    if !worker_child.is_file() {
        return Err(ProvisionError::NotInstalled {
            toolchain: id.clone(),
            install_cmd: install_cmd(id),
        });
    }
    let lean_sysroot = PathBuf::from(&sidecar.lean_sysroot);
    // Header drift trumps everything: if the toolchain's lean.h moved under the
    // worker, a rebuild is the right move. When the header can't be read (the
    // elan toolchain was removed), skip the check — the child spawn surfaces the
    // real failure with its own diagnostics.
    if let Ok(current) = hash_lean_header(&lean_sysroot)
        && !sidecar.header_matches(&current)
    {
        return Err(ProvisionError::Stale {
            toolchain: id.clone(),
            install_cmd: install_cmd(id),
        });
    }
    if let Some(SmokeOutcome::Failed { detail }) = sidecar.smoke() {
        return Err(ProvisionError::Unusable {
            toolchain: id.clone(),
            detail: detail.clone(),
            install_cmd: install_cmd(id),
        });
    }
    Ok(InstalledWorker {
        worker_child,
        capability_manifest: PathBuf::from(&sidecar.capability_manifest),
        lean_sysroot,
    })
}

/// Full SHA-256 (lowercase hex) of `<lean_sysroot>/include/lean/lean.h` — the
/// robust toolchain-identity check (a version string can lie; the header digest
/// cannot).
///
/// # Errors
///
/// Forwards the read error if the header is absent or unreadable.
pub fn hash_lean_header(lean_sysroot: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let path = lean_sysroot.join("include").join("lean").join("lean.h");
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(digest.len().saturating_mul(2));
    for byte in &digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// Outcome of `install-worker`'s post-build runtime smoke test, recorded in the
/// sidecar. A header-digest match does not imply ABI compatibility — the
/// toolchain's `libleanshared` can still crash the worker — so the recorded run
/// result is the sound "can it actually load" signal.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum SmokeOutcome {
    /// The built worker loaded the capability and answered `version`.
    Passed,
    /// The worker built but crashed/failed at load. `detail` is the failure
    /// (e.g. `signal: 11 (SIGSEGV)`).
    Failed { detail: String },
}

/// Provenance record written beside an installed worker and read back at
/// resolution time. Records what the worker was built against (for header-drift
/// detection) and where its artifacts landed (absolute paths, so resolution does
/// not recompute the build layout). Fields stay private behind query methods.
#[derive(Serialize, Deserialize, Debug)]
pub struct WorkerSidecar {
    toolchain: String,
    /// SHA-256 of the `lean.h` the worker was built against.
    header_digest: String,
    /// `lean_toolchain::LEAN_VERSION` the host was built against.
    built_against_lean_version: String,
    /// `lean-dup` version (`CARGO_PKG_VERSION`) that built this worker. `""`
    /// (serde default) for a sidecar predating the field.
    #[serde(default)]
    built_by_host_version: String,
    /// Absolute Lean sysroot the worker-child is spawned with.
    lean_sysroot: String,
    /// Absolute path to the `LeanDup` capability artifact manifest.
    capability_manifest: String,
    /// Absolute path to the built `LeanDup` capability dylib (recorded for
    /// diagnostics; the manifest already points at it).
    capability_dylib: String,
    /// Post-build runtime smoke outcome. `None` for a sidecar predating it.
    #[serde(default)]
    smoke: Option<SmokeOutcome>,
}

impl WorkerSidecar {
    /// Build a sidecar stamped with this host's build-time context.
    #[must_use]
    pub fn new(
        id: &ToolchainId,
        header_digest: String,
        lean_sysroot: &Path,
        capability_manifest: &Path,
        capability_dylib: &Path,
        smoke: SmokeOutcome,
    ) -> Self {
        Self {
            toolchain: id.as_str().to_owned(),
            header_digest,
            built_against_lean_version: lean_toolchain::LEAN_VERSION.to_owned(),
            built_by_host_version: env!("CARGO_PKG_VERSION").to_owned(),
            lean_sysroot: lean_sysroot.display().to_string(),
            capability_manifest: capability_manifest.display().to_string(),
            capability_dylib: capability_dylib.display().to_string(),
            smoke: Some(smoke),
        }
    }

    /// Write `<install_dir>/worker.json`, overwriting any existing record.
    ///
    /// # Errors
    ///
    /// Forwards serialization or write failures.
    pub fn write(&self, install_dir: &Path) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(install_dir.join(SIDECAR_FILE_NAME), json)
    }

    /// Load `<install_dir>/worker.json`. `None` when absent or unparseable —
    /// unknown provenance, not an error.
    #[must_use]
    pub fn load(install_dir: &Path) -> Option<Self> {
        let bytes = std::fs::read(install_dir.join(SIDECAR_FILE_NAME)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Whether the recorded build-time digest still matches the toolchain.
    #[must_use]
    pub fn header_matches(&self, current_digest: &str) -> bool {
        self.header_digest == current_digest
    }

    /// The recorded post-build smoke outcome, if any.
    #[must_use]
    pub fn smoke(&self) -> Option<&SmokeOutcome> {
        self.smoke.as_ref()
    }

    /// The `lean-dup` version that built this worker, or `""` if the sidecar
    /// predates host-version provenance.
    #[must_use]
    pub fn host_version(&self) -> &str {
        &self.built_by_host_version
    }
}

/// Typed failures while parsing or locating a toolchain.
#[derive(Debug)]
pub enum ToolchainError {
    Unparseable(String),
    FileMissing(PathBuf),
    ElanMissing { toolchain: ToolchainId, elan_dir: PathBuf },
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unparseable(raw) => write!(f, "could not parse lean-toolchain string: {raw:?}"),
            Self::FileMissing(path) => write!(f, "lean-toolchain file not found at {}", path.display()),
            Self::ElanMissing { toolchain, elan_dir } => write!(
                f,
                "elan toolchain {toolchain} is not installed (expected {}); install it with: \
                 elan toolchain install {}",
                elan_dir.display(),
                toolchain.elan_label()
            ),
        }
    }
}

impl std::error::Error for ToolchainError {}

/// Why a usable worker could not be resolved for an audited workspace. Every
/// variant's [`Display`] names the `install-worker` command that fixes it.
#[derive(Debug)]
pub enum ProvisionError {
    /// No worker is installed for this toolchain.
    NotInstalled {
        toolchain: ToolchainId,
        install_cmd: String,
    },
    /// The toolchain's `lean.h` changed since the worker was built: rebuild it.
    Stale {
        toolchain: ToolchainId,
        install_cmd: String,
    },
    /// The worker built and matched its header digest but failed its post-build
    /// smoke test — its toolchain's `libleanshared` is ABI-incompatible and the
    /// worker crashes on load.
    Unusable {
        toolchain: ToolchainId,
        detail: String,
        install_cmd: String,
    },
}

impl fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled { toolchain, install_cmd } => write!(
                f,
                "no lean-dup worker is installed for toolchain {toolchain}; run: {install_cmd}"
            ),
            Self::Stale { toolchain, install_cmd } => write!(
                f,
                "the lean-dup worker for toolchain {toolchain} is stale (its lean.h changed since it was built); \
                 rebuild it: {install_cmd}"
            ),
            Self::Unusable {
                toolchain,
                detail,
                install_cmd,
            } => write!(
                f,
                "the lean-dup worker for toolchain {toolchain} failed its smoke test ({detail}); rebuild it: {install_cmd}"
            ),
        }
    }
}

impl std::error::Error for ProvisionError {}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code uses unwrap/expect/panic to surface failure paths concisely"
)]
mod tests {
    use std::fs;

    use super::*;

    fn sidecar(id: &ToolchainId, digest: &str, sysroot: &Path, smoke: SmokeOutcome) -> WorkerSidecar {
        WorkerSidecar::new(
            id,
            digest.to_owned(),
            sysroot,
            &sysroot.join("manifest.json"),
            &sysroot.join("LeanDup.dylib"),
            smoke,
        )
    }

    #[test]
    fn parse_accepts_elan_prefix_and_bare_short_form() {
        assert_eq!(
            ToolchainId::parse("leanprover/lean4:v4.32.0-rc1").unwrap().as_str(),
            "v4.32.0-rc1"
        );
        assert_eq!(ToolchainId::parse("  v4.31.0\n").unwrap().as_str(), "v4.31.0");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(matches!(ToolchainId::parse(""), Err(ToolchainError::Unparseable(_))));
        assert!(matches!(
            ToolchainId::parse("acme/lean5:v6"),
            Err(ToolchainError::Unparseable(_))
        ));
    }

    #[test]
    fn pinned_matches_the_constant() {
        assert_eq!(ToolchainId::pinned().as_str(), "v4.32.0-rc1");
        assert_eq!(ToolchainId::pinned().elan_label(), PINNED_TOOLCHAIN);
    }

    #[test]
    fn from_lake_root_reads_workspace_pin() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("lean-toolchain"), "leanprover/lean4:v4.31.0\n").unwrap();
        assert_eq!(ToolchainId::from_lake_root(tmp.path()).unwrap().as_str(), "v4.31.0");
    }

    #[test]
    fn sidecar_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ToolchainId::pinned();
        sidecar(&id, "abc123", tmp.path(), SmokeOutcome::Passed)
            .write(tmp.path())
            .unwrap();
        let loaded = WorkerSidecar::load(tmp.path()).expect("sidecar loads");
        assert!(loaded.header_matches("abc123"));
        assert!(!loaded.header_matches("other"));
        assert_eq!(loaded.smoke(), Some(&SmokeOutcome::Passed));
        assert_eq!(loaded.host_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn resolve_missing_sidecar_is_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ToolchainId::pinned();
        let err = resolve_in(tmp.path(), &id).unwrap_err();
        let ProvisionError::NotInstalled { install_cmd, .. } = err else {
            panic!("expected NotInstalled, got {err:?}");
        };
        assert!(install_cmd.contains("install-worker --toolchain v4.32.0-rc1"));
    }

    #[test]
    fn resolve_failed_smoke_is_unusable() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ToolchainId::pinned();
        fs::write(tmp.path().join(WORKER_FILE_NAME), b"#!/bin/sh\n").unwrap();
        // A digest the resolver cannot recompute (no real lean.h under tmp), so
        // drift is skipped and the smoke verdict is what bites.
        sidecar(
            &id,
            "digest",
            tmp.path(),
            SmokeOutcome::Failed {
                detail: "SIGSEGV".to_owned(),
            },
        )
        .write(tmp.path())
        .unwrap();
        let err = resolve_in(tmp.path(), &id).unwrap_err();
        let ProvisionError::Unusable { detail, .. } = err else {
            panic!("expected Unusable, got {err:?}");
        };
        assert!(detail.contains("SIGSEGV"));
    }

    #[test]
    fn resolve_ready_returns_artifacts() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ToolchainId::pinned();
        fs::write(tmp.path().join(WORKER_FILE_NAME), b"#!/bin/sh\n").unwrap();
        sidecar(&id, "digest", tmp.path(), SmokeOutcome::Passed)
            .write(tmp.path())
            .unwrap();
        let installed = resolve_in(tmp.path(), &id).expect("resolves");
        assert_eq!(installed.worker_child, tmp.path().join(WORKER_FILE_NAME));
        assert_eq!(installed.capability_manifest, tmp.path().join("manifest.json"));
        assert_eq!(installed.lean_sysroot, tmp.path());
    }
}
