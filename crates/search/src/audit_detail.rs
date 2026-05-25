use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audit::AuditGroup;
use crate::{Error, Result};

const SCHEMA_VERSION: &str = "lean-dup.audit-detail.v1";

/// On-disk record of one audit's per-group detail, written next to the
/// lean baseline snapshot. Backs the `show` fast path so re-deriving the
/// pipeline isn't needed for a group that the previous `audit` already
/// computed. Keyed by `workspace_fingerprint` (the same key
/// `BaselineSnapshot` uses); a fingerprint mismatch on load is treated as
/// "no cache" and the caller falls through to the slow path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditDetailSnapshot {
    pub schema_version: String,
    pub workspace_fingerprint: String,
    pub requested_workspace: PathBuf,
    pub lake_root: PathBuf,
    pub selected_roots: Vec<String>,
    pub source_count: usize,
    pub cache_root: PathBuf,
    pub groups: Vec<AuditGroup>,
    pub suppressed_groups: Vec<AuditGroup>,
}

impl AuditDetailSnapshot {
    /// Resolve `requested` against any of the surfaces `run_show` accepts:
    /// group id, family id, pair id, pair ids, or pair-evidence id. Returns
    /// a cloned group with its `id` rewritten to the matched key (mirroring
    /// `run_show`'s mutation), so the fast path's output matches the slow
    /// path's output byte-for-byte.
    pub fn resolve(&self, requested: &str) -> Option<AuditGroup> {
        let group = self
            .groups
            .iter()
            .chain(self.suppressed_groups.iter())
            .find(|group| {
                group.id == requested
                    || group.family_id == requested
                    || group.pair_id == requested
                    || group.pair_ids.iter().any(|pair_id| pair_id == requested)
                    || group.pair_evidence.iter().any(|pair| pair.id == requested)
            })?;
        let mut cloned = group.clone();
        if cloned.id != requested
            && (cloned.pair_id == requested
                || cloned.pair_ids.iter().any(|pair_id| pair_id == requested)
                || cloned.pair_evidence.iter().any(|pair| pair.id == requested))
        {
            cloned.id = requested.to_owned();
        }
        Some(cloned)
    }
}

/// Directory holding per-audit detail snapshots, one per workspace
/// fingerprint. Exposed crate-private so the workspace cleanup module can
/// walk it without duplicating the path constant.
pub(crate) fn last_audit_detail_dir(cache_root: &Path) -> PathBuf {
    cache_root.join("last-audit-detail")
}

fn snapshot_path(cache_root: &Path, fingerprint: &str) -> PathBuf {
    last_audit_detail_dir(cache_root).join(format!("{fingerprint}.json"))
}

/// Persist the detail snapshot atomically. Best-effort: failures degrade
/// `show` to the slow path on the next call but never block `audit` itself.
pub fn save_last(cache_root: &Path, snapshot: &AuditDetailSnapshot) -> Result<PathBuf> {
    let path = snapshot_path(cache_root, &snapshot.workspace_fingerprint);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            message: "could not create last-audit-detail directory",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec(snapshot)?;
    std::fs::write(&tmp, &body).map_err(|source| Error::Io {
        message: "could not write last-audit-detail",
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| Error::Io {
        message: "could not finalize last-audit-detail",
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Load the detail snapshot if one is on disk for `fingerprint`, parses
/// cleanly, matches `SCHEMA_VERSION`, and reports the same fingerprint.
/// Any failure returns `None`; callers fall through to the slow path.
pub fn load_last(cache_root: &Path, fingerprint: &str) -> Option<AuditDetailSnapshot> {
    let path = snapshot_path(cache_root, fingerprint);
    let body = std::fs::read(&path).ok()?;
    let snapshot: AuditDetailSnapshot = serde_json::from_slice(&body).ok()?;
    if snapshot.schema_version != SCHEMA_VERSION || snapshot.workspace_fingerprint != fingerprint {
        return None;
    }
    Some(snapshot)
}

pub(crate) fn build(
    workspace_fingerprint: String,
    requested_workspace: PathBuf,
    lake_root: PathBuf,
    selected_roots: Vec<String>,
    source_count: usize,
    cache_root: PathBuf,
    groups: Vec<AuditGroup>,
    suppressed_groups: Vec<AuditGroup>,
) -> AuditDetailSnapshot {
    AuditDetailSnapshot {
        schema_version: SCHEMA_VERSION.to_owned(),
        workspace_fingerprint,
        requested_workspace,
        lake_root,
        selected_roots,
        source_count,
        cache_root,
        groups,
        suppressed_groups,
    }
}
