use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use lean_dup_diagnostics::read_to_string;

use crate::index::{ExpectedIndexEntry, INDEX_SCHEMA_VERSION, IndexProvenance, IndexProvenanceKind, IndexStore};
use crate::{Error, Result};

/// Cache diagnostics for all known labels under one cache root.
///
/// The lifecycle layer reports cache validity and cleanup facts without
/// exposing cache-key JSON, SQLite table layout, or latest-pointer storage to
/// audit and index callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheDiagnostics {
    pub cache_root: PathBuf,
    pub total_disk_bytes: u64,
    pub labels: Vec<CacheLabelDiagnostics>,
    pub probe_stores: Vec<ProbeStoreDiagnostics>,
}

/// Size facts for one shared probe-verdict store (`<cache_root>/probes/<label>.sqlite`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeStoreDiagnostics {
    pub label: String,
    pub path: PathBuf,
    pub rows: u64,
    pub disk_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheLabelDiagnostics {
    pub label: String,
    pub label_dir: PathBuf,
    pub disk_bytes: u64,
    pub latest: CacheLatestDiagnostics,
    pub entries: Vec<CacheEntryDiagnostics>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheLatestDiagnostics {
    pub pointer_path: PathBuf,
    pub status: CacheLatestStatus,
    pub index_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheLatestStatus {
    Ok,
    Missing,
    TargetMissing,
    CorruptPointer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheEntryDiagnostics {
    pub index_dir: PathBuf,
    pub index_path: PathBuf,
    pub status: CacheEntryStatus,
    pub active_latest: bool,
    pub expected_current: bool,
    pub schema_version: Option<String>,
    pub provenance_kind: IndexProvenanceKind,
    pub declaration_count: Option<usize>,
    pub disk_bytes: u64,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheEntryStatus {
    Current,
    Stale,
    Corrupt,
    Missing,
    Unchecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupPolicy {
    pub execute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleanupReport {
    pub status: &'static str,
    pub cache_root: PathBuf,
    pub executed: bool,
    pub removable_count: usize,
    pub protected_count: usize,
    pub bytes_to_remove: u64,
    pub bytes_removed: u64,
    pub removed_entries: Vec<CacheCleanupEntry>,
    pub protected_entries: Vec<CacheCleanupEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleanupEntry {
    pub label: String,
    pub index_dir: PathBuf,
    pub disk_bytes: u64,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
struct LatestPointer {
    index_dir: PathBuf,
}

#[derive(Debug)]
struct IndexMetadata {
    schema_version: Option<String>,
    provenance_kind: IndexProvenanceKind,
    declaration_count: Option<usize>,
}

pub fn diagnose_cache(
    cache_root: PathBuf,
    expected_entries: &[ExpectedIndexEntry],
    store: &IndexStore,
) -> Result<CacheDiagnostics> {
    let indexes_root = cache_root.join("indexes");
    let mut labels = label_dirs(&indexes_root)?;
    for expected in expected_entries {
        labels
            .entry(expected.label.clone())
            .or_insert_with(|| expected.index_dir.parent().unwrap_or(&indexes_root).to_path_buf());
    }

    let expected_by_dir = expected_entries
        .iter()
        .map(|entry| (entry.index_dir.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    let mut reports = Vec::with_capacity(labels.len());
    for (label, label_dir) in labels {
        reports.push(diagnose_label(&label, label_dir, &expected_by_dir, store)?);
    }
    let probe_stores = diagnose_probe_stores(&cache_root);
    let total_disk_bytes = reports.iter().map(|label| label.disk_bytes).sum::<u64>()
        + probe_stores.iter().map(|store| store.disk_bytes).sum::<u64>();
    Ok(CacheDiagnostics {
        cache_root,
        total_disk_bytes,
        labels: reports,
        probe_stores,
    })
}

/// Shared probe stores are managed artifacts (not orphans): report their size
/// alongside the per-label index facts. Unreadable stores are skipped — doctor
/// reports what it can see, it does not repair.
fn diagnose_probe_stores(cache_root: &Path) -> Vec<ProbeStoreDiagnostics> {
    let probes_dir = cache_root.join("probes");
    let Ok(entries) = std::fs::read_dir(&probes_dir) else {
        return Vec::new();
    };
    let mut stores = Vec::new();
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("sqlite") {
            continue;
        }
        let Ok(store) = crate::probe_store::ProbeStore::open(cache_root, stem) else {
            continue;
        };
        let Ok(facts) = store.facts() else {
            continue;
        };
        stores.push(ProbeStoreDiagnostics {
            label: stem.to_owned(),
            path,
            rows: facts.rows,
            disk_bytes: facts.bytes,
        });
    }
    stores.sort_by(|left, right| left.label.cmp(&right.label));
    stores
}

pub fn cleanup_cache(
    cache_root: PathBuf,
    expected_entries: &[ExpectedIndexEntry],
    policy: CleanupPolicy,
) -> Result<CacheCleanupReport> {
    let indexes_root = cache_root.join("indexes");
    let labels = label_dirs(&indexes_root)?;
    let protected = protected_dirs(&labels, expected_entries);
    let expected_dirs = expected_entries
        .iter()
        .map(|entry| entry.index_dir.clone())
        .collect::<BTreeSet<_>>();
    let mut removable = Vec::new();
    let mut protected_entries = Vec::new();

    for (label, label_dir) in labels {
        for index_dir in index_dirs(&label_dir)? {
            let disk_bytes = disk_bytes(&index_dir);
            if protected.contains(&index_dir) {
                let reason = if expected_dirs.contains(&index_dir) {
                    "current expected index".to_owned()
                } else {
                    "active latest pointer".to_owned()
                };
                protected_entries.push(CacheCleanupEntry {
                    label: label.clone(),
                    index_dir,
                    disk_bytes,
                    reason,
                });
            } else {
                removable.push(CacheCleanupEntry {
                    label: label.clone(),
                    index_dir,
                    disk_bytes,
                    reason: "not referenced by latest pointer or current request".to_owned(),
                });
            }
        }
    }

    let bytes_to_remove = removable.iter().map(|entry| entry.disk_bytes).sum();
    let mut bytes_removed = 0;
    if policy.execute {
        for entry in &removable {
            bytes_removed += entry.disk_bytes;
            std::fs::remove_dir_all(&entry.index_dir).map_err(|source| Error::Io {
                message: "could not remove cache index directory",
                path: entry.index_dir.clone(),
                source,
            })?;
        }
    }

    Ok(CacheCleanupReport {
        status: "ok",
        cache_root,
        executed: policy.execute,
        removable_count: removable.len(),
        protected_count: protected_entries.len(),
        bytes_to_remove,
        bytes_removed,
        removed_entries: removable,
        protected_entries,
    })
}

fn diagnose_label(
    label: &str,
    label_dir: PathBuf,
    expected_by_dir: &BTreeMap<PathBuf, &ExpectedIndexEntry>,
    store: &IndexStore,
) -> Result<CacheLabelDiagnostics> {
    let latest = latest_diagnostics(&label_dir)?;
    let active_dirs = latest.index_dir.iter().cloned().collect::<BTreeSet<PathBuf>>();
    let mut dirs = index_dirs(&label_dir)?.into_iter().collect::<BTreeSet<_>>();
    for expected in expected_by_dir.values() {
        if expected.label == label {
            dirs.insert(expected.index_dir.clone());
        }
    }

    let mut entries = Vec::with_capacity(dirs.len());
    for index_dir in dirs {
        let expected = expected_by_dir.get(&index_dir).copied();
        entries.push(diagnose_entry(index_dir, &active_dirs, expected, store)?);
    }
    let disk_bytes = disk_bytes(&label_dir);
    Ok(CacheLabelDiagnostics {
        label: label.to_owned(),
        label_dir,
        disk_bytes,
        latest,
        entries,
    })
}

fn diagnose_entry(
    index_dir: PathBuf,
    active_dirs: &BTreeSet<PathBuf>,
    expected: Option<&ExpectedIndexEntry>,
    store: &IndexStore,
) -> Result<CacheEntryDiagnostics> {
    let index_path = index_dir.join("index.sqlite");
    let active_latest = active_dirs.contains(&index_dir);
    let expected_current = expected.is_some();
    let disk_bytes = disk_bytes(&index_dir);
    let mut reasons = Vec::new();

    if !index_path.exists() {
        reasons.push("missing cache store".to_owned());
        return Ok(CacheEntryDiagnostics {
            index_dir,
            index_path,
            status: CacheEntryStatus::Missing,
            active_latest,
            expected_current,
            schema_version: None,
            provenance_kind: IndexProvenanceKind::Static,
            declaration_count: None,
            disk_bytes,
            reasons,
        });
    }

    let metadata = match read_index_metadata(&index_path) {
        Ok(metadata) => metadata,
        Err(_) => {
            reasons.push("cache store metadata is unreadable".to_owned());
            return Ok(CacheEntryDiagnostics {
                index_dir,
                index_path,
                status: CacheEntryStatus::Corrupt,
                active_latest,
                expected_current,
                schema_version: None,
                provenance_kind: IndexProvenanceKind::Static,
                declaration_count: None,
                disk_bytes,
                reasons,
            });
        }
    };

    let status = if metadata.schema_version.as_deref() != Some(INDEX_SCHEMA_VERSION) {
        reasons.push("schema version differs from current binary".to_owned());
        CacheEntryStatus::Stale
    } else if let Some(expected) = expected {
        if store.cache_entry_is_current(expected)? {
            CacheEntryStatus::Current
        } else {
            reasons.push("cache key differs from current workspace inputs".to_owned());
            CacheEntryStatus::Stale
        }
    } else {
        reasons.push("no current request was provided for source freshness".to_owned());
        CacheEntryStatus::Unchecked
    };

    Ok(CacheEntryDiagnostics {
        index_dir,
        index_path,
        status,
        active_latest,
        expected_current,
        schema_version: metadata.schema_version,
        provenance_kind: metadata.provenance_kind,
        declaration_count: metadata.declaration_count,
        disk_bytes,
        reasons,
    })
}

fn label_dirs(indexes_root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut labels = BTreeMap::new();
    if !indexes_root.exists() {
        return Ok(labels);
    }
    for entry in std::fs::read_dir(indexes_root).map_err(|source| Error::Io {
        message: "could not read cache indexes directory",
        path: indexes_root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            message: "could not read cache label entry",
            path: indexes_root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(label) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        labels.insert(label.to_owned(), path);
    }
    Ok(labels)
}

fn index_dirs(label_dir: &Path) -> Result<Vec<PathBuf>> {
    if !label_dir.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(label_dir).map_err(|source| Error::Io {
        message: "could not read cache label directory",
        path: label_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            message: "could not read cache label entry",
            path: label_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn latest_diagnostics(label_dir: &Path) -> Result<CacheLatestDiagnostics> {
    let pointer_path = label_dir.join("latest.json");
    if !pointer_path.exists() {
        return Ok(CacheLatestDiagnostics {
            pointer_path,
            status: CacheLatestStatus::Missing,
            index_dir: None,
        });
    }
    let pointer = match read_to_string(pointer_path.clone())
        .map_err(Error::from)
        .and_then(|text| serde_json::from_str::<LatestPointer>(&text).map_err(Error::from))
    {
        Ok(pointer) => pointer,
        Err(_) => {
            return Ok(CacheLatestDiagnostics {
                pointer_path,
                status: CacheLatestStatus::CorruptPointer,
                index_dir: None,
            });
        }
    };
    let status = if pointer.index_dir.join("index.sqlite").exists() {
        CacheLatestStatus::Ok
    } else {
        CacheLatestStatus::TargetMissing
    };
    Ok(CacheLatestDiagnostics {
        pointer_path,
        status,
        index_dir: Some(pointer.index_dir),
    })
}

fn protected_dirs(labels: &BTreeMap<String, PathBuf>, expected_entries: &[ExpectedIndexEntry]) -> BTreeSet<PathBuf> {
    let mut protected = expected_entries
        .iter()
        .map(|entry| entry.index_dir.clone())
        .collect::<BTreeSet<_>>();
    for label_dir in labels.values() {
        if let Ok(latest) = latest_diagnostics(label_dir)
            && let Some(index_dir) = latest.index_dir
        {
            protected.insert(index_dir);
        }
    }
    protected
}

fn read_index_metadata(index_path: &Path) -> std::result::Result<IndexMetadata, String> {
    let connection = open_readonly(index_path).map_err(|error| error.to_string())?;
    let schema_version = metadata_value(&connection, "schema_version").map_err(|error| error.to_string())?;
    let provenance_kind = match metadata_value(&connection, "provenance_json").map_err(|error| error.to_string())? {
        Some(json) => serde_json::from_str::<IndexProvenance>(&json)
            .map(|provenance| provenance.kind)
            .map_err(|error| format!("invalid provenance metadata: {error}"))?,
        None => IndexProvenanceKind::Static,
    };
    let declaration_count = declaration_count(&connection).ok();
    Ok(IndexMetadata {
        schema_version,
        provenance_kind,
        declaration_count,
    })
}

fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
    )
}

fn metadata_value(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row("SELECT value FROM metadata WHERE key = ?1", params![key], |row| {
            row.get(0)
        })
        .optional()
}

fn declaration_count(connection: &Connection) -> rusqlite::Result<usize> {
    let count = connection.query_row("SELECT COUNT(*) FROM declarations", [], |row| row.get::<_, i64>(0))?;
    Ok(count as usize)
}

fn disk_bytes(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{CacheEntryStatus, CacheLatestStatus, CleanupPolicy, cleanup_cache, diagnose_cache};
    use crate::index::{ExpectedIndexEntry, INDEX_SCHEMA_VERSION, IndexProvenance, IndexProvenanceKind, IndexStore};

    #[test]
    fn diagnostics_classify_current_stale_missing_corrupt_and_provenance() {
        let temp = TempDir::new().unwrap();
        let cache_root = temp.path().to_path_buf();
        let label_dir = cache_root.join("indexes/fixture");
        let current_dir = label_dir.join("current");
        let stale_dir = label_dir.join("stale");
        let missing_dir = label_dir.join("missing");
        let corrupt_dir = label_dir.join("corrupt");
        let static_dir = label_dir.join("static");
        write_index(
            &current_dir,
            INDEX_SCHEMA_VERSION,
            "same",
            Some(IndexProvenanceKind::SourceBacked),
        );
        write_index(&stale_dir, "old-schema", "old", Some(IndexProvenanceKind::SourceBacked));
        fs::create_dir_all(&missing_dir).unwrap();
        fs::create_dir_all(&corrupt_dir).unwrap();
        fs::write(corrupt_dir.join("index.sqlite"), "not sqlite").unwrap();
        write_index(&static_dir, INDEX_SCHEMA_VERSION, "static", None);
        write_latest(&label_dir, &current_dir);

        let expected = ExpectedIndexEntry::for_test("fixture", current_dir.clone(), "same");
        let diagnostics = diagnose_cache(cache_root, &[expected], &IndexStore::new(temp.path().to_path_buf())).unwrap();
        let label = diagnostics
            .labels
            .iter()
            .find(|label| label.label == "fixture")
            .unwrap();
        assert_eq!(label.latest.status, CacheLatestStatus::Ok);

        let status_for = |dir: &std::path::Path| {
            label
                .entries
                .iter()
                .find(|entry| entry.index_dir == dir)
                .map(|entry| (entry.status, entry.provenance_kind))
                .unwrap()
        };
        assert_eq!(status_for(&current_dir).0, CacheEntryStatus::Current);
        assert_eq!(status_for(&stale_dir).0, CacheEntryStatus::Stale);
        assert_eq!(status_for(&missing_dir).0, CacheEntryStatus::Missing);
        assert_eq!(status_for(&corrupt_dir).0, CacheEntryStatus::Corrupt);
        assert_eq!(
            status_for(&static_dir),
            (CacheEntryStatus::Unchecked, IndexProvenanceKind::Static)
        );
    }

    #[test]
    fn cleanup_protects_latest_and_current_expected_entries() {
        let temp = TempDir::new().unwrap();
        let cache_root = temp.path().to_path_buf();
        let label_dir = cache_root.join("indexes/fixture");
        let active_dir = label_dir.join("active");
        let expected_dir = label_dir.join("expected");
        let stale_dir = label_dir.join("stale");
        write_index(
            &active_dir,
            INDEX_SCHEMA_VERSION,
            "active",
            Some(IndexProvenanceKind::SourceBacked),
        );
        write_index(
            &expected_dir,
            INDEX_SCHEMA_VERSION,
            "expected",
            Some(IndexProvenanceKind::SourceBacked),
        );
        write_index(
            &stale_dir,
            INDEX_SCHEMA_VERSION,
            "stale",
            Some(IndexProvenanceKind::SourceBacked),
        );
        write_latest(&label_dir, &active_dir);
        let expected = ExpectedIndexEntry::for_test("fixture", expected_dir.clone(), "expected");

        let dry_run = cleanup_cache(
            cache_root.clone(),
            std::slice::from_ref(&expected),
            CleanupPolicy { execute: false },
        )
        .unwrap();
        assert_eq!(dry_run.removable_count, 1);
        assert!(stale_dir.exists());

        let executed = cleanup_cache(cache_root, &[expected], CleanupPolicy { execute: true }).unwrap();
        assert_eq!(executed.removable_count, 1);
        assert!(active_dir.exists());
        assert!(expected_dir.exists());
        assert!(!stale_dir.exists());
    }

    fn write_latest(label_dir: &std::path::Path, index_dir: &std::path::Path) {
        fs::create_dir_all(label_dir).unwrap();
        fs::write(
            label_dir.join("latest.json"),
            serde_json::to_string(&serde_json::json!({ "index_dir": index_dir })).unwrap(),
        )
        .unwrap();
    }

    fn write_index(
        index_dir: &std::path::Path,
        schema_version: &str,
        cache_key: &str,
        provenance_kind: Option<IndexProvenanceKind>,
    ) {
        fs::create_dir_all(index_dir).unwrap();
        let index_path = index_dir.join("index.sqlite");
        let connection = rusqlite::Connection::open(index_path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE declarations (declaration_handle TEXT PRIMARY KEY);
                "#,
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO metadata VALUES ('schema_version', ?1)",
                rusqlite::params![schema_version],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO metadata VALUES ('cache_key', ?1)",
                rusqlite::params![cache_key],
            )
            .unwrap();
        if let Some(kind) = provenance_kind {
            let mut provenance = IndexProvenance::static_index("Fixture");
            provenance.kind = kind;
            connection
                .execute(
                    "INSERT INTO metadata VALUES ('provenance_json', ?1)",
                    rusqlite::params![serde_json::to_string(&provenance).unwrap()],
                )
                .unwrap();
        }
    }
}
