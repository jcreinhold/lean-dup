//! `lean-dup install-worker` — build the toolchain-specific worker on this
//! machine.
//!
//! `cargo install lean-dup` ships the parent Lean-free. The artifacts that link
//! Lean — the `lean-dup-worker-child` binary and the `LeanDup` capability dylib
//! plus its dependency dylibs — are built here, into
//! `<install_root>/<toolchain-id>/`, and resolved at audit time from the audited
//! project's `lean-toolchain` pin. One toolchain is built per invocation: the one
//! named by `--toolchain`, or the current directory's `lean-toolchain`, or
//! lean-dup's development pin.
//!
//! The build has two halves. The worker-child comes from `cargo` — a local
//! checkout (`cargo build -p lean-dup-worker-child`) when this binary was built
//! from one, otherwise the published crate (`cargo install
//! lean-dup-worker-child`), in both cases with `LEAN_SYSROOT` pointed at the
//! target toolchain's elan dir so it links that toolchain's `libleanshared`. The
//! capability comes from `lean-dup-capability-source`, whose packaged Lean source
//! survives a crates.io unpack. After both build, a post-build smoke test spawns
//! the worker and runs the `version` export through the real dlopen chain — a
//! matching header digest does not imply ABI compatibility, so this is the sound
//! "can it actually load" signal — and the outcome is recorded in the sidecar.

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

    // The elan toolchain has to exist before the worker-child and capability can
    // link against it; surface the actionable `elan toolchain install` hint up
    // front rather than after a failed build.
    let lean_sysroot = id.elan_dir().map_err(|error| error.to_string())?;

    let staged = build_worker_child(args, &id, &lean_sysroot)?;
    std::fs::create_dir_all(&dest).map_err(|error| format!("create install dir {}: {error}", dest.display()))?;
    let installed_child = dest.join(WORKER_FILE_NAME);
    move_into_place(&staged.binary, &installed_child)?;

    eprintln!("==> building LeanDup capability for {id}");
    let built = lean_dup_capability_source::build_capability_into(&dest, &id.elan_label(), &lean_sysroot)
        .map_err(|error| format!("build LeanDup capability: {error}"))?;

    let header_digest = hash_lean_header(&lean_sysroot).map_err(|error| format!("hash toolchain lean.h: {error}"))?;

    // Write the sidecar optimistically so the smoke test's resolution finds the
    // freshly built artifacts; the smoke outcome is then recorded for real below.
    write_sidecar(&dest, &id, &header_digest, &lean_sysroot, &built, SmokeOutcome::Passed)?;

    eprintln!("==> smoke test: load the capability and run `version` for {id}");
    if let Err(detail) = smoke_test(&id) {
        write_sidecar(
            &dest,
            &id,
            &header_digest,
            &lean_sysroot,
            &built,
            SmokeOutcome::Failed { detail: detail.clone() },
        )?;
        return Err(format!(
            "worker for {id} built but FAILED its smoke test ({detail}); this toolchain's libleanshared is \
             likely ABI-incompatible with lean-dup's lean-rs build. The worker is recorded as unusable and will \
             not be served — audit a project on a toolchain lean-dup supports, or rebuild against a different pin"
        ));
    }

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

/// A freshly built worker-child binary, plus the temp dir that owns it for the
/// registry build (kept alive until the binary is relocated).
struct StagedWorker {
    binary: PathBuf,
    _tmp: Option<tempfile::TempDir>,
}

/// Build `lean-dup-worker-child` for `id`. A `--source-dir` or an in-checkout
/// build uses `cargo build`; a registry-installed parent uses `cargo install` of
/// the published worker-child at this binary's exact version (they share the
/// workspace version and are ABI-coupled in lockstep). `LEAN_SYSROOT` pins the
/// link target toolchain.
fn build_worker_child(args: &InstallWorkerArgs, id: &ToolchainId, lean_sysroot: &Path) -> Result<StagedWorker, String> {
    if let Some(workspace) = workspace_source(args) {
        eprintln!("==> building {WORKER_FILE_NAME} for {id} (workspace source)");
        let status = Command::new("cargo")
            .args(["build", "--release", "-p", WORKER_FILE_NAME, "--locked"])
            .current_dir(&workspace)
            .env("LEAN_SYSROOT", lean_sysroot)
            .status()
            .map_err(|error| format!("spawn cargo build: {error}"))?;
        if !status.success() {
            return Err(format!(
                "cargo build -p {WORKER_FILE_NAME} (toolchain {id}) failed with status {status}"
            ));
        }
        let binary = workspace.join("target").join("release").join(WORKER_FILE_NAME);
        if !binary.is_file() {
            return Err(format!("expected worker binary at {} but found none", binary.display()));
        }
        return Ok(StagedWorker { binary, _tmp: None });
    }

    let version = env!("CARGO_PKG_VERSION");
    eprintln!("==> installing {WORKER_FILE_NAME} {version} for {id} (crates.io)");
    let tmp = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
    let status = Command::new("cargo")
        .args(["install", WORKER_FILE_NAME, "--version"])
        .arg(format!("={version}"))
        .args(["--root"])
        .arg(tmp.path())
        .arg("--locked")
        .env("LEAN_SYSROOT", lean_sysroot)
        .status()
        .map_err(|error| format!("spawn cargo install: {error}"))?;
    if !status.success() {
        return Err(format!(
            "cargo install {WORKER_FILE_NAME}@={version} (toolchain {id}) failed with status {status}; \
             a Rust toolchain and network access are required"
        ));
    }
    let binary = tmp.path().join("bin").join(WORKER_FILE_NAME);
    if !binary.is_file() {
        return Err(format!(
            "cargo install did not produce a worker binary at {}",
            binary.display()
        ));
    }
    Ok(StagedWorker {
        binary,
        _tmp: Some(tmp),
    })
}

/// The checkout to build the worker-child from: `--source-dir` if given, else
/// this binary's own workspace when it was built from a checkout (the
/// worker-child crate sits beside it), else `None` (registry build).
fn workspace_source(args: &InstallWorkerArgs) -> Option<PathBuf> {
    if let Some(dir) = &args.source_dir {
        return Some(dir.clone());
    }
    // `CARGO_MANIFEST_DIR` is `<repo>/crates/cli` for a checkout build; for a
    // registry-installed binary it points into `~/.cargo/registry/...` with no
    // worker-child crate beside it. Probe for that crate specifically.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)?;
    repo.join("crates")
        .join("worker-child")
        .join("Cargo.toml")
        .is_file()
        .then_some(repo)
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
    std::fs::copy(from, to).map_err(|error| format!("copy worker binary to {}: {error}", to.display()))?;
    Ok(())
}

fn write_sidecar(
    dest: &Path,
    id: &ToolchainId,
    header_digest: &str,
    lean_sysroot: &Path,
    built: &lean_dup_capability_source::BuiltCapability,
    smoke: SmokeOutcome,
) -> Result<(), String> {
    WorkerSidecar::new(
        id,
        header_digest.to_owned(),
        lean_sysroot,
        &built.manifest_path,
        &built.dylib_path,
        smoke,
    )
    .write(dest)
    .map_err(|error| format!("write worker sidecar: {error}"))
}

/// Spawn the just-built worker and run the `version` export through the real
/// dlopen chain. A temp workspace pins the toolchain so resolution selects the
/// install dir we just wrote; the `version` export ignores its request payload,
/// so no `.olean` files are needed — loading the capability is the test.
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
