//! Dev-only test helpers shared across the lean-dup workspace.
//!
//! Not published (`publish = false`). It exists to keep a single copy of the
//! worker-provisioning logic that both the `lean-dup-worker` unit tests and the
//! `lean-dup-cli` integration tests depend on, rather than duplicating it per
//! crate.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Workspace root — the directory that contains `crates/`.
#[must_use]
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

/// Guarantee `lean-dup-worker-child` exists in `target/debug/` so the parent's
/// sibling resolution finds it when a test spawns the worker.
///
/// `cargo test` does not force-build a cross-crate binary that an integration
/// test only spawns at runtime, so without this the worker bootstrap fails with
/// `child_unresolved`. Memoized; a no-op when `LEAN_DUP_WORKER_CHILD` already
/// points at a built binary (the path CI provisions up front). Capped at
/// `--jobs 2` so parallel test binaries do not each launch a full rustc swarm.
pub fn ensure_worker_child_built() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        if let Some(path) = std::env::var_os("LEAN_DUP_WORKER_CHILD")
            && PathBuf::from(&path).is_file()
        {
            return;
        }
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "lean-dup-worker-child", "--locked", "--jobs", "2"])
            .current_dir(repo_root())
            .status()
            .expect("spawn cargo build for lean-dup-worker-child");
        assert!(status.success(), "failed to build lean-dup-worker-child");
    });
}
