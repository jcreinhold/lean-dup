//! The streaming, order-agnostic build path.
//!
//! A [`StoreBuilder`] ingests two kinds of [`Ingest`] item — a declaration
//! identity announcement and a feature row — that may arrive in any order or
//! interleaved. A declaration is written only once *both* halves have arrived,
//! paired by `declaration_id`; the unpaired halves wait in two small maps and
//! are evicted the moment they pair, so the resident set during a build is
//! bounded by the number of concurrently unpaired declarations, never by the
//! corpus. `SQLite` holds the rows already written.
//!
//! The whole build runs in one transaction against a temp file with aggressive
//! write pragmas; on [`publish`](StoreBuilder::publish) the transaction commits,
//! the connection closes, and the temp file is renamed into place atomically. A
//! builder dropped before publishing removes its temp file, so an interrupted
//! build leaves any prior corpus at the destination untouched and the new path
//! absent.
//!
//! Each build gets its own randomly named temp file in the destination's
//! directory, so concurrent builds of one destination never share a file: each
//! publishes a whole corpus, and the last rename wins. A build killed before it
//! publishes or drops leaves its temp file behind. No later build removes it,
//! because it cannot tell a dead build's file from a live one's.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use lean_semantic_search_contract::DeclarationFeatureRow;
use lean_semantic_search_retrieval::RETRIEVAL_POLICY_VERSION;
use rusqlite::{Connection, params};
use tempfile::TempPath;

use super::StoreError;
use super::schema::{
    self, META_CORPUS_TOKEN, META_POLICY_VERSION, META_SCHEMA_VERSION, META_TOTAL_DOCUMENTS, STORE_SCHEMA_VERSION,
};

/// One item in the build stream.
///
/// The two variants are paired by `declaration_id`: a declaration enters the
/// corpus only when its identity has been announced *and* its feature row has
/// arrived. The store keeps only the feature row; the announcement carries no
/// data of its own — it records that the extractor accepted the declaration,
/// so a feature row for a later-filtered declaration is never indexed alone.
pub(crate) enum Ingest {
    /// A declaration the extractor accepted, identified by its opaque id.
    Declaration(String),
    /// A declaration's feature row.
    Feature(DeclarationFeatureRow),
}

/// Builds a persisted corpus, then publishes it atomically.
pub(crate) struct StoreBuilder {
    connection: Option<Connection>,
    temp_path: Option<TempPath>,
    final_path: PathBuf,
    corpus_token: String,
    announced: HashSet<String>,
    awaiting: HashMap<String, DeclarationFeatureRow>,
    total_documents: usize,
}

impl StoreBuilder {
    /// Open a fresh build in a temp file of its own beside `path`, ready to
    /// ingest. Concurrent builds of one `path` each get their own temp file.
    ///
    /// `corpus_token` is an opaque, caller-supplied content identity stored
    /// verbatim; the store never interprets it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the destination directory or the temp file
    /// cannot be created, or [`StoreError::Sqlite`] if the connection, pragmas,
    /// schema, or opening transaction fail.
    pub(crate) fn create(path: impl AsRef<Path>, corpus_token: impl Into<String>) -> Result<Self, StoreError> {
        let final_path = path.as_ref().to_path_buf();
        // Callers often build into a new directory, so create a missing parent.
        // A bare filename has an empty parent and builds in the current directory.
        let directory = match final_path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                std::fs::create_dir_all(parent)?;
                parent
            }
            _ => Path::new("."),
        };
        let temp_path = building_file(directory, &final_path)?;
        let connection = Connection::open(&temp_path)?;
        schema::initialize(&connection)?;
        // One transaction for the whole build; commit happens at publish.
        connection.execute_batch("BEGIN")?;
        Ok(Self {
            connection: Some(connection),
            temp_path: Some(temp_path),
            final_path,
            corpus_token: corpus_token.into(),
            announced: HashSet::new(),
            awaiting: HashMap::new(),
            total_documents: 0,
        })
    }

    /// Ingest one stream item, writing a declaration as soon as both its halves
    /// have arrived.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Json`] if a feature row cannot be serialized, or
    /// [`StoreError::Sqlite`] if an insert fails. [`StoreError::Closed`] if the
    /// builder has already been published.
    pub(crate) fn accept(&mut self, item: Ingest) -> Result<(), StoreError> {
        match item {
            Ingest::Declaration(declaration_id) => {
                if let Some(row) = self.awaiting.remove(&declaration_id) {
                    self.write_row(&row)?;
                } else {
                    self.announced.insert(declaration_id);
                }
            }
            Ingest::Feature(row) => {
                if self.announced.remove(&row.declaration_id) {
                    self.write_row(&row)?;
                } else {
                    self.awaiting.insert(row.declaration_id.clone(), row);
                }
            }
        }
        Ok(())
    }

    /// Finalize: write metadata, commit, close, and rename the temp file into
    /// place. Returns the published path.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the metadata write or commit fails, or
    /// [`StoreError::Io`] if closing or renaming fails. On any error the temp
    /// build is discarded and the destination is left untouched.
    pub(crate) fn publish(mut self) -> Result<PathBuf, StoreError> {
        let connection = self.connection.take().ok_or(StoreError::Closed)?;
        write_metadata(&connection, META_SCHEMA_VERSION, STORE_SCHEMA_VERSION)?;
        write_metadata(&connection, META_POLICY_VERSION, RETRIEVAL_POLICY_VERSION)?;
        write_metadata(&connection, META_CORPUS_TOKEN, &self.corpus_token)?;
        write_metadata(&connection, META_TOTAL_DOCUMENTS, &self.total_documents.to_string())?;
        connection.execute_batch("COMMIT")?;
        connection.close().map_err(|(_, error)| StoreError::Sqlite(error))?;
        // Take the temp file only now: on an earlier error, drop closes the
        // connection before it deletes the file.
        let temp_path = self.temp_path.take().ok_or(StoreError::Closed)?;
        temp_path.persist(&self.final_path).map_err(std::io::Error::from)?;
        Ok(self.final_path.clone())
    }

    fn write_row(&mut self, row: &DeclarationFeatureRow) -> Result<(), StoreError> {
        let connection = self.connection.as_ref().ok_or(StoreError::Closed)?;
        let row_json = serde_json::to_string(row)?;
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO feature_rows(declaration_id, feature_version, row_json) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![row.declaration_id, row.feature_version, row_json])?;
        let mut posting =
            connection.prepare_cached("INSERT OR IGNORE INTO postings(key, declaration_id) VALUES (?1, ?2)")?;
        for key in posting_keys(row) {
            posting.execute(params![key, row.declaration_id])?;
        }
        self.total_documents = self.total_documents.saturating_add(1);
        Ok(())
    }
}

impl Drop for StoreBuilder {
    fn drop(&mut self) {
        // Close any still-open build connection so its rollback completes and
        // the handle is released before the temp file is touched.
        if let Some(connection) = self.connection.take() {
            connection.close().ok();
        }
        // Dropping the temp path deletes an unpublished build. Publish takes it
        // first, so a renamed build is never touched.
        drop(self.temp_path.take());
    }
}

/// The unique opaque keys a row contributes to the inverted index: its four
/// fingerprints and every role-feature key, deduplicated exactly as the
/// in-memory reference index does so fanout counts match.
fn posting_keys(row: &DeclarationFeatureRow) -> HashSet<&str> {
    let mut keys: HashSet<&str> = HashSet::with_capacity(row.role_features.len().saturating_add(4));
    keys.insert(row.fingerprints.statement.as_str());
    keys.insert(row.fingerprints.safe_binder_permutation.as_str());
    keys.insert(row.fingerprints.connective_shape.as_str());
    keys.insert(row.fingerprints.conclusion_shape.as_str());
    for feature in &row.role_features {
        keys.insert(feature.key.as_str());
    }
    keys
}

fn write_metadata(connection: &Connection, key: &str, value: &str) -> Result<(), StoreError> {
    connection
        .prepare_cached("INSERT OR REPLACE INTO metadata(key, value) VALUES (?1, ?2)")?
        .execute(params![key, value])?;
    Ok(())
}

/// Create this build's own temp file, `<name>.<random>.building`, in the
/// destination's directory, so publish can rename it in one step on the same
/// filesystem.
fn building_file(directory: &Path, final_path: &Path) -> Result<TempPath, StoreError> {
    let mut prefix = final_path.file_name().map_or_else(OsString::new, OsString::from);
    prefix.push(".");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix).suffix(".building");
    // `tempfile` makes owner-only files. Ask for the mode `SQLite` gives a new
    // database (0644, less the umask), so a published store reads as before.
    #[cfg(unix)]
    {
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o644));
    }
    Ok(builder.tempfile_in(directory)?.into_temp_path())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Barrier;

    use lean_semantic_search_contract::DeclarationFeatureRow;
    use lean_semantic_search_retrieval::Corpus;
    use rusqlite::{Connection, OpenFlags};

    use super::{Ingest, StoreBuilder};
    use crate::store::Store;
    use crate::store::fixtures::{
        build_store, declaration, fingerprints, key, rich_declaration, sample, start_build, stored_metadata,
        stray_files, temp_path,
    };

    /// Build a store at `path` feeding the items in exactly the given order, then
    /// open it.
    fn build_in_order(path: &Path, items: Vec<Ingest>) -> Result<Store, String> {
        let mut builder = StoreBuilder::create(path, "tok").map_err(|e| e.to_string())?;
        for item in items {
            builder.accept(item).map_err(|e| e.to_string())?;
        }
        let published = builder.publish().map_err(|e| e.to_string())?;
        Store::open(published).map_err(|e| e.to_string())
    }

    #[test]
    fn pairing_is_order_agnostic() -> Result<(), String> {
        let rows = sample();

        // Interleaved: declaration then feature, per row.
        let interleaved: Vec<Ingest> = rows
            .iter()
            .flat_map(|row| {
                [
                    Ingest::Declaration(row.declaration_id.clone()),
                    Ingest::Feature(row.clone()),
                ]
            })
            .collect();

        // Reversed halves: every feature first, then every announcement.
        let mut reversed: Vec<Ingest> = rows.iter().map(|row| Ingest::Feature(row.clone())).collect();
        reversed.extend(rows.iter().map(|row| Ingest::Declaration(row.declaration_id.clone())));

        let path_a = temp_path("order-a")?;
        let path_b = temp_path("order-b")?;
        let a = build_in_order(&path_a, interleaved)?;
        let b = build_in_order(&path_b, reversed)?;

        assert_eq!(a.document_total(), b.document_total());
        for row in &rows {
            let id = &row.declaration_id;
            assert_eq!(
                a.declaration_row(id),
                b.declaration_row(id),
                "row {id} differs by arrival order"
            );
        }
        let probe = vec![key("canon-a-stmt"), key("shared-role"), key("canon-b-stmt")];
        assert_eq!(a.fanout(&probe), b.fanout(&probe));
        Ok(())
    }

    #[test]
    fn unpaired_halves_are_not_indexed() -> Result<(), String> {
        // A feature with no announcement, and an announcement with no feature: both
        // are incomplete and must not enter the corpus.
        let path = temp_path("unpaired")?;
        let store = build_in_order(
            &path,
            vec![
                Ingest::Feature(declaration("featured-only", fingerprints("x"), Vec::new())),
                Ingest::Declaration("announced-only".to_owned()),
            ],
        )?;
        assert_eq!(store.document_total(), 0);
        assert_eq!(store.declaration_row("featured-only"), None);
        Ok(())
    }

    #[test]
    fn dropped_build_leaves_no_final_path() -> Result<(), String> {
        let path = temp_path("dropped")?;
        {
            let mut builder = StoreBuilder::create(&path, "tok").map_err(|e| e.to_string())?;
            builder
                .accept(Ingest::Declaration("d1".to_owned()))
                .map_err(|e| e.to_string())?;
            builder
                .accept(Ingest::Feature(declaration("d1", fingerprints("a"), Vec::new())))
                .map_err(|e| e.to_string())?;
            // Drop without publishing.
        }
        assert!(!path.exists(), "an unpublished build must not create the final path");
        let stray = stray_files(&path)?;
        assert!(
            stray.is_empty(),
            "an unpublished build must clean up its temp file: {stray:?}"
        );
        Ok(())
    }

    #[test]
    fn interrupted_rebuild_leaves_prior_corpus_intact() -> Result<(), String> {
        let path = temp_path("rebuild")?;

        // Publish an original corpus.
        let original = vec![declaration("orig", fingerprints("a"), Vec::new())];
        let store = build_store(&path, "v1", &original)?;
        assert_eq!(store.document_total(), 1);
        drop(store);

        // Start a replacement build at the same destination, then abandon it.
        {
            let mut builder = StoreBuilder::create(&path, "v2").map_err(|e| e.to_string())?;
            builder
                .accept(Ingest::Declaration("new".to_owned()))
                .map_err(|e| e.to_string())?;
            builder
                .accept(Ingest::Feature(declaration("new", fingerprints("b"), Vec::new())))
                .map_err(|e| e.to_string())?;
        }

        let stray = stray_files(&path)?;
        assert!(
            stray.is_empty(),
            "abandoned rebuild must clean up its temp file: {stray:?}"
        );
        assert_eq!(
            stored_metadata(&path, "corpus_token")?,
            "v1",
            "prior corpus must survive an interrupted rebuild"
        );
        let reopened = Store::open(&path).map_err(|e| e.to_string())?;
        assert_eq!(
            reopened.declaration_row("orig").map(|r| r.declaration_id),
            Some("orig".to_owned())
        );
        assert_eq!(reopened.declaration_row("new"), None);
        Ok(())
    }

    #[test]
    fn concurrent_builds_of_one_path_both_publish() -> Result<(), String> {
        let path = temp_path("concurrent")?;
        let left: Vec<DeclarationFeatureRow> = (0..3)
            .map(|n| declaration(&format!("left-{n}"), fingerprints("l"), Vec::new()))
            .collect();
        let right: Vec<DeclarationFeatureRow> = (0..5)
            .map(|n| declaration(&format!("right-{n}"), fingerprints("r"), Vec::new()))
            .collect();

        // Hold both builds open at once: each ingests, waits for the other, then
        // publishes. Builds that shared a temp file would break each other here.
        let both_open = Barrier::new(2);
        let build = |corpus_token: &str, rows: &[DeclarationFeatureRow]| -> Result<PathBuf, String> {
            let builder = start_build(&path, corpus_token, rows);
            both_open.wait();
            builder?.publish().map_err(|error| error.to_string())
        };
        let results = std::thread::scope(|scope| {
            let left_build = scope.spawn(|| build("left", &left));
            let right_build = scope.spawn(|| build("right", &right));
            [left_build.join(), right_build.join()]
        });
        for result in results {
            let published = result.map_err(|_| "a build thread panicked".to_owned())??;
            assert_eq!(published.as_path(), &*path);
        }
        let stray = stray_files(&path)?;
        assert!(
            stray.is_empty(),
            "both builds must clean up their temp files: {stray:?}"
        );

        // The last rename wins, and what it leaves is one whole corpus, not a mix.
        let store = Store::open(&path).map_err(|e| e.to_string())?;
        let (expected, absent) = match stored_metadata(&path, "corpus_token")?.as_str() {
            "left" => (&left, &right),
            "right" => (&right, &left),
            other => return Err(format!("unexpected corpus token {other}")),
        };
        assert_eq!(store.document_total(), expected.len());
        for row in expected {
            assert_eq!(store.declaration_row(&row.declaration_id).as_ref(), Some(row));
        }
        for row in absent {
            assert_eq!(store.declaration_row(&row.declaration_id), None);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn published_store_has_the_mode_sqlite_gives_a_new_database() -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;

        // A database `SQLite` creates itself in the same directory shows the mode
        // a published store should have under the current umask.
        let path = temp_path("mode")?;
        drop(build_store(&path, "tok", &sample())?);
        let reference = path.with_file_name("reference.sqlite");
        Connection::open(&reference)
            .and_then(|connection| connection.execute_batch("CREATE TABLE t (x)"))
            .map_err(|e| e.to_string())?;
        let mode = |file: &Path| -> Result<u32, String> {
            let metadata = std::fs::metadata(file).map_err(|e| e.to_string())?;
            Ok(metadata.permissions().mode() & 0o777)
        };
        assert_eq!(mode(&path)?, mode(&reference)?);
        Ok(())
    }

    #[test]
    fn metadata_keys_are_exactly_the_recorded_facts() -> Result<(), String> {
        let path = temp_path("metadata")?;
        let _store = build_store(&path, "tok", std::slice::from_ref(&rich_declaration("rich")))?;

        let connection =
            Connection::open_with_flags(&*path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        let mut query = connection
            .prepare("SELECT key FROM metadata ORDER BY key")
            .map_err(|e| e.to_string())?;
        let keys: Vec<String> = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();

        assert_eq!(
            keys,
            vec![
                "corpus_token".to_owned(),
                "policy_version".to_owned(),
                "schema_version".to_owned(),
                "total_documents".to_owned(),
            ]
        );
        Ok(())
    }
}
