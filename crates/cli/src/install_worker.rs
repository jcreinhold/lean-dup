//! `lean-dup install-worker` — build the toolchain-specific worker on this
//! machine.
//!
//! `cargo install lean-dup` ships the parent Lean-free. The artifact that links
//! Lean — the native `lean-dup-worker` executable — is built here, into
//! `<install_root>/<toolchain-id>/`, and resolved at audit time from the audited
//! project's `lean-toolchain` pin. One toolchain is built per invocation: the
//! one named by `--toolchain`, or the current directory's `lean-toolchain`, or
//! lean-dup's development pin.
//!
//! The build is one `lake build lean-dup-worker` with the target toolchain's own
//! Lake, either in a checkout's `lean/` project (`--source-dir`) or from the
//! packaged Lean source in `lean-dup-capability-source` (which survives a
//! crates.io unpack). After the build, a smoke test spawns the executable and
//! runs the `version` command over the real JSONL transport, and the outcome is
//! recorded in the sidecar.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lean_dup_worker::WorkerClient;
use lean_dup_worker::toolchain::{
    SmokeOutcome, ToolchainId, WORKER_FILE_NAME, WorkerSidecar, hash_lean_header, install_dir,
};

use crate::cli::InstallWorkerArgs;

/// Build and install the worker for the requested toolchain, returning a process
/// exit code. Progress goes to stderr; the final install path goes to stdout.
pub(crate) fn run(args: &InstallWorkerArgs) -> i32 {
    match install(args) {
        Ok(dir) => {
            println!("{}", dir.display());
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

fn install(args: &InstallWorkerArgs) -> Result<PathBuf, String> {
    let id = resolve_toolchain(args)?;
    let dest = install_dir(&id);

    if !args.force && worker_is_current(&dest, &id) {
        eprintln!(
            "==> worker for {id} is already installed and current at {} (use --force to rebuild)",
            dest.display()
        );
        return Ok(dest);
    }

    // The elan toolchain has to exist before the worker can build against it;
    // surface the actionable `elan toolchain install` hint up front rather than
    // after a failed build.
    let lean_sysroot = id.elan_dir().map_err(|error| error.to_string())?;

    eprintln!("==> building lean-dup-worker executable for {id}");
    let built_exe = build_worker_exe(args, &id, &lean_sysroot)?;
    std::fs::create_dir_all(&dest).map_err(|error| format!("create install dir {}: {error}", dest.display()))?;
    let installed_exe = dest.join(WORKER_FILE_NAME);
    move_into_place(&built_exe, &installed_exe)?;

    let header_digest = hash_lean_header(&lean_sysroot).map_err(|error| format!("hash toolchain lean.h: {error}"))?;

    // The smoke test resolves the worker through the parent's runtime path,
    // which refuses to serve a worker without a sidecar — write a pending one
    // first so a fresh install dir can pass its own smoke test.
    WorkerSidecar::pending(&id, header_digest.clone(), &lean_sysroot)
        .write(&dest)
        .map_err(|error| format!("write pending worker sidecar: {error}"))?;

    eprintln!("==> smoke test: spawn the worker and run `version` for {id}");
    if let Err(detail) = smoke_test(&id) {
        write_sidecar(
            &dest,
            &id,
            &header_digest,
            &lean_sysroot,
            SmokeOutcome::Failed { detail: detail.clone() },
        )?;
        return Err(format!(
            "worker for {id} built but FAILED its smoke test ({detail}); the worker is recorded as unusable and \
             will not be served — audit a project on a toolchain lean-dup supports, or rebuild against a \
             different pin"
        ));
    }
    write_sidecar(&dest, &id, &header_digest, &lean_sysroot, SmokeOutcome::Passed)?;

    eprintln!("==> installed worker for {id} at {}", dest.display());
    Ok(dest)
}

/// The toolchain to build for: `--toolchain` wins, else the current directory's
/// `lean-toolchain`, else lean-dup's development pin.
fn resolve_toolchain(args: &InstallWorkerArgs) -> Result<ToolchainId, String> {
    if let Some(raw) = &args.toolchain {
        return ToolchainId::parse(raw).map_err(|error| error.to_string());
    }
    let cwd = std::env::current_dir().map_err(|error| format!("read current directory: {error}"))?;
    Ok(ToolchainId::from_lake_root(&cwd).unwrap_or_else(|_| ToolchainId::pinned()))
}

/// Whether `dest` already holds a usable, header-fresh, smoke-passing worker for
/// `id` — mirrors the parent's runtime resolution so a skipped rebuild is one
/// the parent would accept.
fn worker_is_current(dest: &Path, id: &ToolchainId) -> bool {
    let Some(sidecar) = WorkerSidecar::load(dest) else {
        return false;
    };
    if !dest.join(WORKER_FILE_NAME).is_file() || !matches!(sidecar.smoke(), Some(SmokeOutcome::Passed)) {
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

/// Build the `lean-dup-worker` executable for `id` and return its path.
///
/// `--source-dir` builds the checkout's `lean/` project in place (its
/// dependencies are already fetched); otherwise the packaged Lean source in
/// `lean-dup-capability-source` is materialized under the install dir and built
/// there, so a registry-installed parent needs no checkout and no network. Both
/// paths run the target toolchain's own `lake`.
fn build_worker_exe(args: &InstallWorkerArgs, id: &ToolchainId, lean_sysroot: &Path) -> Result<PathBuf, String> {
    if let Some(source_dir) = &args.source_dir {
        let lean_project = source_dir.join("lean");
        if !lean_project.join("lakefile.lean").is_file() {
            return Err(format!(
                "--source-dir {} does not look like a lean-dup checkout (no lean/lakefile.lean)",
                source_dir.display()
            ));
        }
        eprintln!("==> building {WORKER_FILE_NAME} for {id} (checkout source)");
        let lake = lean_sysroot.join("bin").join("lake");
        let status = Command::new(&lake)
            .args(["build", "lean-dup-worker"])
            .current_dir(&lean_project)
            .status()
            .map_err(|error| format!("spawn lake build: {error}"))?;
        if !status.success() {
            return Err(format!(
                "lake build lean-dup-worker (toolchain {id}) failed with status {status}"
            ));
        }
        let exe = lean_project
            .join(".lake")
            .join("build")
            .join("bin")
            .join(WORKER_FILE_NAME);
        if !exe.is_file() {
            return Err(format!(
                "expected worker executable at {} but found none",
                exe.display()
            ));
        }
        return Ok(exe);
    }

    let built = lean_dup_capability_source::build_worker_into(&install_dir(id), &id.elan_label(), lean_sysroot)
        .map_err(|error| format!("build lean-dup-worker: {error}"))?;
    Ok(built.exe_path)
}

/// Relocate `from` to `to`, replacing any existing file. Falls back to copy when
/// a cross-device rename fails.
fn move_into_place(from: &Path, to: &Path) -> Result<(), String> {
    if to.is_file() {
        std::fs::remove_file(to).map_err(|error| format!("remove stale {}: {error}", to.display()))?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|error| format!("copy worker executable to {}: {error}", to.display()))?;
    Ok(())
}

fn write_sidecar(
    dest: &Path,
    id: &ToolchainId,
    header_digest: &str,
    lean_sysroot: &Path,
    smoke: SmokeOutcome,
) -> Result<(), String> {
    WorkerSidecar::new(id, header_digest.to_owned(), lean_sysroot, smoke)
        .write(dest)
        .map_err(|error| format!("write worker sidecar: {error}"))
}

/// Spawn the just-built worker and run the `version` command over the real
/// JSONL transport. A temp workspace pins the toolchain so resolution selects
/// the install dir we just wrote; `version` needs no `.olean` files — spawning
/// and answering is the test.
fn smoke_test(id: &ToolchainId) -> Result<(), String> {
    let workspace = tempfile::tempdir().map_err(|error| format!("create smoke workspace: {error}"))?;
    std::fs::write(
        workspace.path().join("lean-toolchain"),
        format!("{}\n", id.elan_label()),
    )
    .map_err(|error| format!("write smoke lean-toolchain: {error}"))?;
    WorkerClient::with_timeout(Duration::from_secs(120))
        .version(workspace.path().to_path_buf())
        .map(|_| ())
        .map_err(|error| error.to_string())
}
