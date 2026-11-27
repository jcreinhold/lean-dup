//! The on-disk shape: three tables, the build-time pragmas, and the metadata
//! keys. Everything here is opaque equality tokens and the rows behind them —
//! nothing a caller would render.

use rusqlite::Connection;

use super::StoreError;

/// The store's own on-disk schema identity, stored in `metadata` and checked on
/// open. Bump it when the table layout changes.
///
/// It keeps the name of the crate this module came from, because corpora
/// already in caches carry it. Cache reuse checks only the index beside a
/// corpus, so changing this value without bumping
/// [`INDEX_SCHEMA_VERSION`](crate::INDEX_SCHEMA_VERSION) would make cached
/// entries fail to open instead of rebuilding.
pub(super) const STORE_SCHEMA_VERSION: &str = "lean-semantic-search.store.sqlite.v1";

/// Metadata row keys. Publish writes all four and open requires all four. The
/// store checks the schema version and reads the document total; it never
/// interprets the policy version or the corpus token.
pub(super) const META_SCHEMA_VERSION: &str = "schema_version";
pub(super) const META_POLICY_VERSION: &str = "policy_version";
pub(super) const META_CORPUS_TOKEN: &str = "corpus_token";
pub(super) const META_TOTAL_DOCUMENTS: &str = "total_documents";

/// Data-definition for a fresh build. One inverted-index table over opaque
/// keys, one JSON-backed feature-row table for anchor reconstruction, and a
/// metadata key/value table.
///
/// A single `postings` table is correct because fingerprint keys and role keys
/// each carry their own version prefix owned by the Lean feature package, so
/// the two key spaces cannot collide; its composite primary key doubles as the
/// lookup index — `COUNT` over a key prefix answers fanout, a range scan answers
/// postings. The declaration id is already opaque and stable, so it keys both
/// tables directly with no internal handle indirection.
const SCHEMA: &str = "\
CREATE TABLE metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE feature_rows (
    declaration_id  TEXT PRIMARY KEY,
    feature_version TEXT NOT NULL,
    row_json        TEXT NOT NULL
);

CREATE TABLE postings (
    key            TEXT NOT NULL,
    declaration_id TEXT NOT NULL,
    PRIMARY KEY (key, declaration_id)
) WITHOUT ROWID;
";

/// Build-time pragmas. The store is a build-once cache artifact whose crash
/// safety comes from the temp-file-then-rename publish, not from the journal,
/// so the build connection turns both off for throughput. This is deliberately
/// the opposite of a long-lived read-write database.
const BUILD_PRAGMAS: &str = "\
PRAGMA journal_mode = OFF;
PRAGMA synchronous  = OFF;
";

/// Apply the build pragmas and create the empty schema on a fresh connection.
///
/// # Errors
///
/// Returns [`StoreError::Sqlite`] if the pragmas or table creation fail.
pub(super) fn initialize(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(BUILD_PRAGMAS)?;
    connection.execute_batch(SCHEMA)?;
    Ok(())
}
