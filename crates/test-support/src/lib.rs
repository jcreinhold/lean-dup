//! Dev-only test helpers shared across the lean-dup workspace.
//!
//! Not published (`publish = false`). It keeps a single copy of the
//! worker-provisioning logic that both the `lean-dup-worker` unit tests and the
//! `lean-dup-cli` integration tests depend on, rather than duplicating it per
//! crate.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use lean_dup_worker::toolchain::{
    SmokeOutcome, ToolchainId, WORKER_FILE_NAME, WorkerSidecar, hash_lean_header, install_dir,
};

/// Workspace root — the directory that contains `crates/`.
#[must_use]
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

/// Provision a per-toolchain worker for `lean-dup`'s pinned toolchain so tests
/// can spawn the worker, and return the install directory.
///
/// `cargo install lean-dup` ships the parent Lean-free; the toolchain-specific
/// artifact (the native `lean-dup-worker` executable) is built on the user's
/// machine by `lean-dup install-worker`. Tests reproduce that exactly: the
/// executable built from the checkout's `lean/` project, installed under
/// `<install_root>/<pinned-id>` (or `LEAN_DUP_WORKERS_DIR` when CI sets it). The
/// default location is what the parent's runtime resolution consults with no
/// environment override, so both the in-process worker tests and the
/// `assert_cmd` subprocess CLI tests find it.
///
/// Memoized per process and short-circuited across processes: an install whose
/// sidecar records a passing smoke result and matching `lean.h` digest is reused
/// untouched, so only the first run on a machine pays the Lake build.
pub fn ensure_worker_provisioned() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(provision).clone()
}

fn provision() -> PathBuf {
    let id = ToolchainId::pinned();
    let dir = install_dir(&id);
    if worker_is_current(&dir, &id) {
        return dir;
    }
    std::fs::create_dir_all(&dir).expect("create worker install dir");

    let lean_sysroot = id
        .elan_dir()
        .expect("elan toolchain for the pinned toolchain must be installed");
    let built_exe = build_worker_exe(&lean_sysroot);
    let installed_exe = dir.join(WORKER_FILE_NAME);
    std::fs::copy(&built_exe, &installed_exe).expect("copy lean-dup-worker into install dir");

    let header = hash_lean_header(&lean_sysroot).expect("hash toolchain lean.h");
    WorkerSidecar::new(&id, header, &lean_sysroot, SmokeOutcome::Passed)
        .write(&dir)
        .expect("write worker sidecar");
    dir
}

/// Whether `dir` already holds a usable, header-fresh worker for `id`. Mirrors
/// the parent's runtime resolution so a reused install is one the parent would
/// accept.
fn worker_is_current(dir: &Path, id: &ToolchainId) -> bool {
    let Some(sidecar) = WorkerSidecar::load(dir) else {
        return false;
    };
    if !dir.join(WORKER_FILE_NAME).is_file() || !matches!(sidecar.smoke(), Some(SmokeOutcome::Passed)) {
        return false;
    }
    let Ok(sysroot) = id.elan_dir() else {
        return false;
    };
    let Ok(current) = hash_lean_header(&sysroot) else {
        return false;
    };
    sidecar.header_matches(&current)
}

/// Build the debug `lean-dup-worker-child` binary and return its path. `cargo
/// test` does not force-build a cross-crate binary that a test only spawns at
/// runtime, so without this the worker bootstrap fails. Capped at `--jobs 2` so
/// parallel test binaries do not each launch a full rustc swarm.
/// Build the native worker executable from the checkout's `lean/` project with
/// the pinned toolchain's own Lake (its dependencies are already fetched).
fn build_worker_exe(lean_sysroot: &Path) -> PathBuf {
    let lean_project = repo_root().join("lean");
    let lake = lean_sysroot.join("bin").join("lake");
    let status = Command::new(lake)
        .args(["build", "lean-dup-worker"])
        .current_dir(&lean_project)
        .status()
        .expect("spawn lake build for lean-dup-worker");
    assert!(status.success(), "failed to build lean-dup-worker");
    lean_project
        .join(".lake")
        .join("build")
        .join("bin")
        .join(WORKER_FILE_NAME)
}
