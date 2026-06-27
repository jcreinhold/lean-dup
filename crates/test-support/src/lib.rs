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
/// artifacts (worker-child binary + `LeanDup` capability dylib) are built on the
/// user's machine by `lean-dup install-worker`. Tests reproduce that exactly: a
/// debug worker-child plus the capability, installed under
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

    let built_child = build_worker_child();
    let installed_child = dir.join(WORKER_FILE_NAME);
    std::fs::copy(&built_child, &installed_child).expect("copy worker-child into install dir");

    let lean_sysroot = id
        .elan_dir()
        .expect("elan toolchain for the pinned toolchain must be installed");
    let built = lean_dup_capability_source::build_capability_into(&dir, &id.elan_label(), &lean_sysroot)
        .expect("build LeanDup capability");

    let header = hash_lean_header(&lean_sysroot).expect("hash toolchain lean.h");
    WorkerSidecar::new(
        &id,
        header,
        &lean_sysroot,
        &built.manifest_path,
        &built.dylib_path,
        SmokeOutcome::Passed,
    )
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
fn build_worker_child() -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "lean-dup-worker-child", "--locked", "--jobs", "2"])
        .current_dir(repo_root())
        .status()
        .expect("spawn cargo build for lean-dup-worker-child");
    assert!(status.success(), "failed to build lean-dup-worker-child");
    repo_root().join("target").join("debug").join(WORKER_FILE_NAME)
}
