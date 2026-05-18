use std::ffi::OsString;
use std::path::PathBuf;

use serde::Serialize;
use sha2::{Digest, Sha256};

use lean_dup_diagnostics::{Result, read, read_to_string};
use lean_dup_project::workspace::ResolvedWorkspace;

pub const CACHE_KEY_VERSION: &str = "rust-cli-cache.v1";

#[derive(Debug, Clone, Serialize)]
pub struct CacheFacts {
    pub root: PathBuf,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
struct WorkspaceCacheKey {
    version: &'static str,
    lake_root: String,
    lakefile: Option<FileDigest>,
    lean_toolchain: Option<String>,
    lake_manifest: Option<FileDigest>,
    selected_roots: Vec<String>,
    sources: Vec<SourceDigest>,
}

#[derive(Debug, Clone, Serialize)]
struct SourceDigest {
    module: String,
    path: String,
    digest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct FileDigest {
    path: String,
    digest: String,
}

pub fn resolve_cache(workspace: &ResolvedWorkspace) -> Result<CacheFacts> {
    let root = cache_root();
    let fingerprint = workspace_fingerprint(workspace)?;
    Ok(CacheFacts { root, fingerprint })
}

pub fn cache_root() -> PathBuf {
    cache_root_from(|key| std::env::var_os(key), std::env::var_os("HOME").map(PathBuf::from))
}

fn cache_root_from(env: impl Fn(&str) -> Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(configured) = env("LEAN_DUP_CACHE_DIR") {
        return PathBuf::from(configured);
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("lean-dup")
}

pub fn workspace_fingerprint(workspace: &ResolvedWorkspace) -> Result<String> {
    let key = workspace_cache_key(workspace)?;
    let encoded = serde_json::to_vec(&key)?;
    Ok(format!("{CACHE_KEY_VERSION}:{}", hex_digest(&encoded)))
}

fn workspace_cache_key(workspace: &ResolvedWorkspace) -> Result<WorkspaceCacheKey> {
    let sources = workspace
        .source_files
        .iter()
        .map(|source| {
            Ok(SourceDigest {
                module: source.module.clone(),
                path: source.path.display().to_string(),
                digest: optional_hash(source.path.clone())?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(WorkspaceCacheKey {
        version: CACHE_KEY_VERSION,
        lake_root: workspace.root.display().to_string(),
        lakefile: file_digest(workspace.lakefile.clone())?,
        lean_toolchain: optional_text(workspace.lean_toolchain_path())?,
        lake_manifest: file_digest(workspace.manifest_path())?,
        selected_roots: workspace.selected_roots.clone(),
        sources,
    })
}

fn file_digest(path: PathBuf) -> Result<Option<FileDigest>> {
    Ok(optional_hash(path.clone())?.map(|digest| FileDigest {
        path: path.display().to_string(),
        digest,
    }))
}

fn optional_text(path: PathBuf) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_to_string(path)?.trim().to_owned()))
}

fn optional_hash(path: PathBuf) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(hex_digest(&read(path)?)))
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{cache_root_from, workspace_fingerprint};
    use lean_dup_diagnostics::progress::Reporter;
    use lean_dup_project::workspace::{WorkspaceRequest, resolve};

    #[test]
    fn cache_root_uses_env_override() {
        let root = cache_root_from(
            |key| (key == "LEAN_DUP_CACHE_DIR").then(|| OsString::from("/tmp/lean-dup-cache")),
            Some(PathBuf::from("/home/example")),
        );

        assert_eq!(root, PathBuf::from("/tmp/lean-dup-cache"));
    }

    #[test]
    fn cache_root_defaults_to_user_cache_dir() {
        let root = cache_root_from(|_| None, Some(PathBuf::from("/home/example")));

        assert_eq!(root, PathBuf::from("/home/example/.cache/lean-dup"));
    }

    #[test]
    fn workspace_fingerprint_changes_with_source_content() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.toml"), "[[lean_lib]]\nname = \"Demo\"\n").unwrap();
        fs::write(temp.path().join("lean-toolchain"), "leanprover/lean4:v4.25.0\n").unwrap();
        fs::write(temp.path().join("Demo.lean"), "#check Nat\n").unwrap();

        let workspace = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: None,
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let first = workspace_fingerprint(&workspace).unwrap();

        fs::write(temp.path().join("Demo.lean"), "#check Bool\n").unwrap();
        let workspace = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: None,
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let second = workspace_fingerprint(&workspace).unwrap();

        assert_ne!(first, second);
    }
}
