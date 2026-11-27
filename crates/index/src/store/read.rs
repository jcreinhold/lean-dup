//! The read path: a read-only connection that answers the [`Corpus`] seam.
//!
//! [`Store::open`] validates the schema and metadata and caches the document
//! total once, so [`Corpus`] methods are total and `document_total` is a field
//! read. The connection is opened read-only and single-threaded; nothing here
//! loads the corpus into memory. `fanout` answers all requested keys in one
//! batched `COUNT … GROUP BY` (chunked under `SQLite`'s bind-parameter ceiling);
//! `postings` is a per-key indexed scan bounded by the caller's limit;
//! `declaration_row` deserializes one stored row. None hydrate display data —
//! the store holds none.

use std::path::Path;

use lean_semantic_search_contract::{DeclarationFeatureRow, OpaqueFeatureKey};
use lean_semantic_search_retrieval::Corpus;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};

use super::StoreError;
use super::schema::{
    META_CORPUS_TOKEN, META_POLICY_VERSION, META_SCHEMA_VERSION, META_TOTAL_DOCUMENTS, STORE_SCHEMA_VERSION,
};

/// `SQLite` binds at most 999 parameters per statement by default; stay well
/// under it so a fanout over many keys chunks rather than fails.
const FANOUT_CHUNK: usize = 900;

/// A persisted corpus opened read-only.
///
/// Implements [`Corpus`] over `SQLite` so retrieval ranks against a large,
/// on-disk index without materializing it.
pub(crate) struct Store {
    connection: Connection,
    total_documents: usize,
}

impl Store {
    /// Open a persisted corpus read-only and cache its document total.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the file cannot be opened,
    /// [`StoreError::MissingMetadata`] if a required metadata fact is absent,
    /// [`StoreError::SchemaMismatch`] if the stored schema version is not the
    /// one this build understands, or [`StoreError::Corrupt`] if the document
    /// total is unparseable.
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(path, flags)?;
        connection.execute_batch("PRAGMA query_only = true;")?;

        let schema_version = read_metadata(&connection, META_SCHEMA_VERSION)?;
        if schema_version != STORE_SCHEMA_VERSION {
            return Err(StoreError::SchemaMismatch {
                found: schema_version,
                expected: STORE_SCHEMA_VERSION,
            });
        }
        // Publish always records these two facts. Nothing reads them back, but a
        // file without them is not a corpus this module published.
        read_metadata(&connection, META_POLICY_VERSION)?;
        read_metadata(&connection, META_CORPUS_TOKEN)?;
        let total_documents = read_metadata(&connection, META_TOTAL_DOCUMENTS)?
            .parse::<usize>()
            .map_err(|_| StoreError::Corrupt(META_TOTAL_DOCUMENTS))?;

        Ok(Self {
            connection,
            total_documents,
        })
    }
}

impl Corpus for Store {
    fn document_total(&self) -> usize {
        self.total_documents
    }

    fn fanout(&self, keys: &[OpaqueFeatureKey]) -> Vec<usize> {
        if keys.is_empty() {
            return Vec::new();
        }
        let mut distinct: Vec<&str> = keys.iter().map(OpaqueFeatureKey::as_str).collect();
        distinct.sort_unstable();
        distinct.dedup();

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for chunk in distinct.chunks(FANOUT_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT key, COUNT(*) FROM postings WHERE key IN ({placeholders}) GROUP BY key");
            let Ok(mut statement) = self.connection.prepare_cached(&sql) else {
                continue;
            };
            let Ok(rows) = statement.query_map(params_from_iter(chunk.iter().copied()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }) else {
                continue;
            };
            for (key, count) in rows.flatten() {
                counts.insert(key, usize::try_from(count).unwrap_or(0));
            }
        }
        keys.iter()
            .map(|key| counts.get(key.as_str()).copied().unwrap_or(0))
            .collect()
    }

    fn postings(&self, key: &OpaqueFeatureKey, limit: usize) -> Vec<String> {
        // `usize::MAX` (used for never-pruned fingerprints) cannot exceed the
        // SQL limit; clamp to i64 so the bind is valid. Safe because the caller
        // only reaches here when fanout already fits the limit.
        let bound = i64::try_from(limit).unwrap_or(i64::MAX);
        let Ok(mut statement) = self
            .connection
            .prepare_cached("SELECT declaration_id FROM postings WHERE key = ?1 ORDER BY declaration_id LIMIT ?2")
        else {
            return Vec::new();
        };
        let Ok(rows) = statement.query_map(params![key.as_str(), bound], |row| row.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    fn declaration_row(&self, declaration_id: &str) -> Option<DeclarationFeatureRow> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT row_json FROM feature_rows WHERE declaration_id = ?1")
            .ok()?;
        let row_json: String = statement.query_row(params![declaration_id], |row| row.get(0)).ok()?;
        serde_json::from_str(&row_json).ok()
    }
}

fn read_metadata(connection: &Connection, key: &str) -> Result<String, StoreError> {
    connection
        .prepare_cached("SELECT value FROM metadata WHERE key = ?1")?
        .query_row(params![key], |row| row.get::<_, String>(0))
        .optional()?
        .ok_or_else(|| StoreError::MissingMetadata(key.to_owned()))
}

#[cfg(test)]
mod tests {
    use lean_semantic_search_contract::{DeclarationFeatureRow, ProofGoalFeatureRow, SEMANTIC_FEATURE_VERSION};
    use lean_semantic_search_retrieval::{Anchor, Corpus, SemanticIndex, retrieve_across};

    use crate::store::fixtures::{
        build_store, declaration, fingerprints, key, rich_declaration, role, sample, temp_path,
    };

    #[test]
    fn fanout_counts_match_direct_inspection() -> Result<(), String> {
        let path = temp_path("fanout")?;
        let store = build_store(&path, "tok", &sample())?;
        let keys = vec![
            key("canon-a-stmt"),   // carried by d1, d2
            key("shared-role"),    // carried by d1, d2
            key("canon-b-stmt"),   // carried by d3 only
            key("does-not-exist"), // carried by nobody
        ];
        assert_eq!(store.fanout(&keys), vec![2, 2, 1, 0]);
        assert_eq!(store.document_total(), 3);
        Ok(())
    }

    #[test]
    fn postings_return_matching_id_sets() -> Result<(), String> {
        let path = temp_path("postings")?;
        let store = build_store(&path, "tok", &sample())?;

        let mut shared = store.postings(&key("canon-a-stmt"), 100);
        shared.sort();
        assert_eq!(shared, vec!["d1".to_owned(), "d2".to_owned()]);

        assert_eq!(store.postings(&key("canon-b-stmt"), 100), vec!["d3".to_owned()]);
        assert_eq!(store.postings(&key("does-not-exist"), 100), Vec::<String>::new());

        // The caller's limit bounds the scan.
        assert_eq!(store.postings(&key("shared-role"), 1).len(), 1);
        Ok(())
    }

    #[test]
    fn declaration_row_round_trips_exactly() -> Result<(), String> {
        let row = rich_declaration("rich");
        let path = temp_path("roundtrip")?;
        let store = build_store(&path, "tok", std::slice::from_ref(&row))?;

        assert_eq!(store.declaration_row("rich"), Some(row));
        assert_eq!(store.declaration_row("absent"), None);
        Ok(())
    }

    // The `SQLite` `Store` and the in-memory `SemanticIndex` must rank
    // identically. The tests below run the same rows through both backends via
    // `retrieve_across` and assert the full `Retrieval` (candidate ids, ranks,
    // family explanations, and diagnostics) is equal — including the
    // multi-lane recall guarantee and multi-corpus fan-out.

    /// A corpus where a fingerprint cohort competes with a candidate that matches
    /// only through a rare, selective role key — the multi-lane recall case.
    fn parity_fixture() -> Vec<DeclarationFeatureRow> {
        let mut rows = Vec::new();
        for tag in ["fp1", "fp2", "fp3", "fp4", "fp5"] {
            rows.push(declaration(tag, fingerprints("shared"), Vec::new()));
        }
        rows.push(declaration(
            "role-only",
            fingerprints("unique"),
            vec![role("conclusion_const", "rare-role-key", "Rare")],
        ));
        rows
    }

    fn anchor_row() -> DeclarationFeatureRow {
        declaration(
            "anchor",
            fingerprints("shared"),
            vec![role("conclusion_const", "rare-role-key", "Rare")],
        )
    }

    #[test]
    fn single_corpus_ranking_matches_in_memory() -> Result<(), String> {
        let rows = parity_fixture();
        let anchor = Anchor::from_declaration(&anchor_row());

        let index = SemanticIndex::from_declarations(&rows);
        let path = temp_path("parity-single")?;
        let store = build_store(&path, "token-a", &rows)?;

        let from_index = retrieve_across(&[&index], &anchor, 3);
        let from_store = retrieve_across(&[&store], &anchor, 3);

        assert_eq!(from_index, from_store);

        // The fixture must actually exercise the lane it claims to: the role-only
        // candidate survives a fingerprint cohort that fills the limit.
        let ids: Vec<&str> = from_store
            .candidates
            .iter()
            .map(|c| c.declaration_id.as_str())
            .collect();
        assert!(
            ids.contains(&"role-only"),
            "multi-lane guarantee did not surface role-only: {ids:?}"
        );
        assert!(
            ids.len() > 3,
            "expected the role lane to push the union past the limit: {ids:?}"
        );
        Ok(())
    }

    #[test]
    fn multi_corpus_fan_out_matches_in_memory() -> Result<(), String> {
        // Split the cohort across two corpora, with role-only present in both so
        // the merge-by-declaration-id path is exercised.
        let left = vec![
            declaration("fp1", fingerprints("shared"), Vec::new()),
            declaration("fp2", fingerprints("shared"), Vec::new()),
            declaration(
                "role-only",
                fingerprints("unique"),
                vec![role("conclusion_const", "rare-role-key", "Rare")],
            ),
        ];
        let right = vec![
            declaration("fp3", fingerprints("shared"), Vec::new()),
            declaration("fp4", fingerprints("shared"), Vec::new()),
            declaration(
                "role-only",
                fingerprints("unique"),
                vec![role("conclusion_const", "rare-role-key", "Rare")],
            ),
        ];
        let anchor = Anchor::from_declaration(&anchor_row());

        let index_left = SemanticIndex::from_declarations(&left);
        let index_right = SemanticIndex::from_declarations(&right);
        let path_left = temp_path("parity-left")?;
        let path_right = temp_path("parity-right")?;
        let store_left = build_store(&path_left, "token-l", &left)?;
        let store_right = build_store(&path_right, "token-r", &right)?;

        let from_index = retrieve_across(&[&index_left, &index_right], &anchor, 5);
        let from_store = retrieve_across(&[&store_left, &store_right], &anchor, 5);

        assert_eq!(from_index, from_store);
        Ok(())
    }

    #[test]
    fn proof_goal_anchor_matches_in_memory() -> Result<(), String> {
        let rows = parity_fixture();
        let goal = ProofGoalFeatureRow {
            goal_id: "goal".to_owned(),
            feature_version: SEMANTIC_FEATURE_VERSION.to_owned(),
            fingerprints: fingerprints("shared"),
            role_features: vec![role("conclusion_const", "rare-role-key", "Rare")],
            low_signal_markers: Vec::new(),
        };
        let anchor = Anchor::from_proof_goal(&goal);

        let index = SemanticIndex::from_declarations(&rows);
        let path = temp_path("parity-goal")?;
        let store = build_store(&path, "token-g", &rows)?;

        assert_eq!(
            retrieve_across(&[&index], &anchor, 4),
            retrieve_across(&[&store], &anchor, 4)
        );
        Ok(())
    }
}
