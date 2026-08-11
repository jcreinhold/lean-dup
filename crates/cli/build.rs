use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    emit_git_rerun_paths();

    let revision = std::env::var("GITHUB_SHA")
        .ok()
        .map(|sha| sha.chars().take(12).collect::<String>())
        .or_else(git_revision)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=LEAN_DUP_GIT_REVISION={revision}");
}

fn emit_git_rerun_paths() {
    let Some(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR").ok() else {
        return;
    };
    for git_path in ["HEAD", "packed-refs"] {
        if let Some(path) = git_text(&manifest_dir, &["rev-parse", "--git-path", git_path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(head_ref) = git_text(&manifest_dir, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git_text(&manifest_dir, &["rev-parse", "--git-path", &head_ref])
    {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn git_revision() -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    git_text(&manifest_dir, &["rev-parse", "--short=12", "HEAD"])
}

fn git_text(manifest_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let revision = String::from_utf8(output.stdout).ok()?;
    let revision = revision.trim();
    if revision.is_empty() {
        None
    } else {
        Some(revision.to_owned())
    }
}
