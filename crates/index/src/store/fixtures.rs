//! Rows, temp paths, and corpus builds shared by the store's tests.

use std::path::{Path, PathBuf};

use lean_semantic_search_contract::{
    DeclarationFeatureRow, Fingerprints, OpaqueFeatureKey, RoleFeature, SEMANTIC_FEATURE_VERSION, SourcePosition,
    SourceSpan,
};
use rusqlite::{Connection, OpenFlags};

use super::{Ingest, Store, StoreBuilder};

pub(super) fn key(value: &str) -> OpaqueFeatureKey {
    OpaqueFeatureKey::new(value)
}

pub(super) fn fingerprints(prefix: &str) -> Fingerprints {
    Fingerprints {
        statement: key(&format!("canon-{prefix}-stmt")),
        safe_binder_permutation: key(&format!("canon-{prefix}-safe")),
        connective_shape: key(&format!("canon-{prefix}-conn")),
        conclusion_shape: key(&format!("canon-{prefix}-concl")),
    }
}

pub(super) fn role(role: &str, key_value: &str, display: &str) -> RoleFeature {
    RoleFeature {
        role: role.to_owned(),
        key: key(key_value),
        display: Some(display.to_owned()),
    }
}

pub(super) fn declaration(
    id: &str,
    fingerprints: Fingerprints,
    role_features: Vec<RoleFeature>,
) -> DeclarationFeatureRow {
    DeclarationFeatureRow {
        declaration_id: id.to_owned(),
        feature_version: SEMANTIC_FEATURE_VERSION.to_owned(),
        fingerprints,
        role_features,
        binder_count: 0,
        low_signal_markers: Vec::new(),
        source: None,
    }
}

/// A row exercising every field — markers and a source span — so a round-trip
/// has something non-trivial to preserve.
pub(super) fn rich_declaration(id: &str) -> DeclarationFeatureRow {
    DeclarationFeatureRow {
        declaration_id: id.to_owned(),
        feature_version: SEMANTIC_FEATURE_VERSION.to_owned(),
        fingerprints: fingerprints(id),
        role_features: vec![
            role("conclusion_const", &format!("rk-{id}-c"), "Foo"),
            role("hypothesis_head", &format!("rk-{id}-h"), "Eq"),
        ],
        binder_count: 3,
        low_signal_markers: vec!["broad_head:Eq".to_owned()],
        source: Some(SourceSpan {
            start: SourcePosition { line: 12, column: 3 },
            end: SourcePosition { line: 14, column: 8 },
        }),
    }
}

/// Three rows: `d1` and `d2` share fingerprints and a role key, and `d3` stands
/// alone.
pub(super) fn sample() -> Vec<DeclarationFeatureRow> {
    vec![
        declaration(
            "d1",
            fingerprints("a"),
            vec![role("conclusion_const", "shared-role", "S")],
        ),
        declaration(
            "d2",
            fingerprints("a"),
            vec![role("conclusion_const", "shared-role", "S")],
        ),
        declaration("d3", fingerprints("b"), Vec::new()),
    ]
}

/// A store path inside its own temp directory. Dropping it removes the
/// directory, so keep it alive while a builder or store uses the path.
pub(super) struct TempStorePath {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl std::ops::Deref for TempStorePath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempStorePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A fresh store path for one test, in a directory of its own, so parallel
/// tests never collide and a finished test leaves nothing behind.
///
/// # Errors
///
/// Returns the stringified I/O error if the temp directory cannot be created.
pub(super) fn temp_path(tag: &str) -> Result<TempStorePath, String> {
    let dir = tempfile::Builder::new()
        .prefix(&format!("lean-dup-store-{tag}-"))
        .tempdir()
        .map_err(|error| error.to_string())?;
    let path = dir.path().join(format!("{tag}.sqlite"));
    Ok(TempStorePath { _dir: dir, path })
}

/// Start a build and ingest rows (declaration announced, then featured),
/// leaving it unpublished.
///
/// # Errors
///
/// Returns the stringified store error if creating the build or ingesting fails.
pub(super) fn start_build(
    path: &Path,
    corpus_token: &str,
    rows: &[DeclarationFeatureRow],
) -> Result<StoreBuilder, String> {
    let mut builder = StoreBuilder::create(path, corpus_token).map_err(|error| error.to_string())?;
    for row in rows {
        builder
            .accept(Ingest::Declaration(row.declaration_id.clone()))
            .map_err(|error| error.to_string())?;
        builder
            .accept(Ingest::Feature(row.clone()))
            .map_err(|error| error.to_string())?;
    }
    Ok(builder)
}

/// Build and publish a corpus from rows, then open it read-only.
///
/// # Errors
///
/// Returns the stringified store error if building, publishing, or opening fails.
pub(super) fn build_store(path: &Path, corpus_token: &str, rows: &[DeclarationFeatureRow]) -> Result<Store, String> {
    let published = start_build(path, corpus_token, rows)?
        .publish()
        .map_err(|error| error.to_string())?;
    Store::open(published).map_err(|error| error.to_string())
}

/// Every entry in the directory holding `path` other than `path` itself, such
/// as a temp file a build left behind.
///
/// # Errors
///
/// Returns the stringified I/O error if the directory cannot be read.
pub(super) fn stray_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let directory = path.parent().ok_or("store path has no parent directory")?;
    let mut stray = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry_path = entry.map_err(|error| error.to_string())?.path();
        if entry_path != path {
            stray.push(entry_path);
        }
    }
    Ok(stray)
}

/// The value a published corpus records under metadata `key`, read straight
/// from the file, since the store reads some recorded facts only to check that
/// they exist.
///
/// # Errors
///
/// Returns the stringified `SQLite` error if the file cannot be opened or holds
/// no such key.
pub(super) fn stored_metadata(path: &Path, key: &str) -> Result<String, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    connection
        .query_row("SELECT value FROM metadata WHERE key = ?1", [key], |row| row.get(0))
        .map_err(|e| e.to_string())
}
