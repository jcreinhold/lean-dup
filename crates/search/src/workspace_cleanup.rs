use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lean_dup_index::CleanupPolicy;

use crate::{Error, Result, audit_detail, baseline};

/// Result of sweeping `last-snapshot/` and `last-audit-detail/` for files
/// belonging to workspace fingerprints other than the protected set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileCleanupReport {
    pub removable_count: usize,
    pub protected_count: usize,
    pub bytes_to_remove: u64,
    pub bytes_removed: u64,
    pub removed: Vec<WorkspaceFileCleanupEntry>,
    pub protected: Vec<WorkspaceFileCleanupEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileCleanupEntry {
    /// Subdirectory name (`"last-snapshot"` or `"last-audit-detail"`).
    pub kind: &'static str,
    /// File stem — i.e. the workspace fingerprint the file belongs to.
    pub fingerprint: String,
    pub path: PathBuf,
    pub disk_bytes: u64,
}

const KIND_LAST_SNAPSHOT: &str = "last-snapshot";
const KIND_LAST_AUDIT_DETAIL: &str = "last-audit-detail";

/// Walk the per-fingerprint snapshot directories and remove (or, in
/// dry-run mode, just count) any file whose fingerprint is not in
/// `protected_fingerprints`. An empty `protected_fingerprints` means
/// every file is stale — the caller (the CLI) is responsible for
/// surfacing that to the user, just as `lean_dup_index::cleanup_cache`
/// does for indexes with no expected entries.
pub fn cleanup_stale_workspace_files(
    cache_root: &Path,
    protected_fingerprints: &[String],
    policy: CleanupPolicy,
) -> Result<WorkspaceFileCleanupReport> {
    let protected: HashSet<&str> = protected_fingerprints.iter().map(String::as_str).collect();
    let mut removed = Vec::new();
    let mut protected_entries = Vec::new();
    let mut bytes_to_remove: u64 = 0;
    let mut bytes_removed: u64 = 0;

    for (dir, kind) in [
        (baseline::last_snapshot_dir(cache_root), KIND_LAST_SNAPSHOT),
        (audit_detail::last_audit_detail_dir(cache_root), KIND_LAST_AUDIT_DETAIL),
    ] {
        sweep_directory(
            dir,
            kind,
            &protected,
            policy.execute,
            &mut removed,
            &mut protected_entries,
            &mut bytes_to_remove,
            &mut bytes_removed,
        )?;
    }

    Ok(WorkspaceFileCleanupReport {
        removable_count: removed.len(),
        protected_count: protected_entries.len(),
        bytes_to_remove,
        bytes_removed,
        removed,
        protected: protected_entries,
    })
}

fn sweep_directory(
    dir: PathBuf,
    kind: &'static str,
    protected: &HashSet<&str>,
    execute: bool,
    removed: &mut Vec<WorkspaceFileCleanupEntry>,
    protected_entries: &mut Vec<WorkspaceFileCleanupEntry>,
    bytes_to_remove: &mut u64,
    bytes_removed: &mut u64,
) -> Result<()> {
    let read_dir = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        // A missing directory just means no files of this kind were ever
        // written; that's the normal state for a fresh cache.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(Error::Io {
                message: "could not read workspace-snapshot directory",
                path: dir,
                source,
            });
        }
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let disk_bytes = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
        let summary = WorkspaceFileCleanupEntry {
            kind,
            fingerprint: name.to_owned(),
            path: path.clone(),
            disk_bytes,
        };
        if protected.contains(name) {
            protected_entries.push(summary);
            continue;
        }
        *bytes_to_remove += disk_bytes;
        if execute {
            std::fs::remove_file(&path).map_err(|source| Error::Io {
                message: "could not remove stale workspace snapshot",
                path: path.clone(),
                source,
            })?;
            *bytes_removed += disk_bytes;
        }
        removed.push(summary);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plant(dir: &Path, fingerprint: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{fingerprint}.json")), body).unwrap();
    }

    #[test]
    fn keeps_protected_fingerprints_and_removes_others() {
        let cache = tempfile::TempDir::new().unwrap();
        let snap = baseline::last_snapshot_dir(cache.path());
        let detail = audit_detail::last_audit_detail_dir(cache.path());
        for fp in ["alive", "stale-1", "stale-2"] {
            plant(&snap, fp, "{}");
            plant(&detail, fp, "{}");
        }

        let report = cleanup_stale_workspace_files(
            cache.path(),
            &["alive".to_owned()],
            CleanupPolicy { execute: true },
        )
        .unwrap();

        // Two protected files (one snapshot + one detail), four stale.
        assert_eq!(report.protected_count, 2);
        assert_eq!(report.removable_count, 4);
        assert!(report.bytes_removed > 0);
        assert_eq!(report.bytes_removed, report.bytes_to_remove);
        assert!(snap.join("alive.json").exists());
        assert!(detail.join("alive.json").exists());
        assert!(!snap.join("stale-1.json").exists());
        assert!(!detail.join("stale-2.json").exists());
    }

    #[test]
    fn dry_run_reports_without_removing() {
        let cache = tempfile::TempDir::new().unwrap();
        let snap = baseline::last_snapshot_dir(cache.path());
        plant(&snap, "stale", "{}");

        let report = cleanup_stale_workspace_files(cache.path(), &[], CleanupPolicy { execute: false }).unwrap();

        assert_eq!(report.removable_count, 1);
        assert_eq!(report.bytes_removed, 0);
        assert!(report.bytes_to_remove > 0);
        assert!(snap.join("stale.json").exists());
    }

    #[test]
    fn missing_directories_are_not_an_error() {
        let cache = tempfile::TempDir::new().unwrap();
        let report =
            cleanup_stale_workspace_files(cache.path(), &["any".to_owned()], CleanupPolicy { execute: true }).unwrap();
        assert_eq!(report.removable_count, 0);
        assert_eq!(report.protected_count, 0);
    }
}
