//! Shared semantic-probe verdict store, scoped independently of the index lifecycle.
//!
//! Probe verdicts are the most expensive artifact lean-dup produces, and a
//! verdict for a pair `(A, B)` is determined by the two declarations' content,
//! the prover semantics, and the transitive import closures of the two
//! declarations' modules — *not* by the whole workspace corpus. Storing verdicts
//! inside each per-`cache_id` index coupled them to the index lifecycle: any
//! source edit, even to a file appearing in no probed pair's closure, discarded
//! every cached verdict (see `docs/architecture/probe-cache-scoping.md`).
//!
//! This module owns the shared store at `<cache_root>/probes/<label>.sqlite`.
//! It sits outside every index directory, so it survives index rebuilds;
//! callers key entries with content + import-closure digests so unrelated edits
//! reuse verdicts and closure edits invalidate exactly the affected pairs.

use std::path::{Path, PathBuf};

use lean_dup_diagnostics::perf::{self, CostClass};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use lean_dup_worker::ProbeResult;

/// A shared, index-lifecycle-independent store of semantic probe verdicts.
///
/// One store per cache label lives at `<cache_root>/probes/<label>.sqlite`.
/// Keys are opaque to this module: the caller (semantic verification) owns the
/// content + closure digest recipe.
#[derive(Debug)]
pub struct ProbeStore {
    path: PathBuf,
}

impl ProbeStore {
    /// Open (creating when needed) the shared probe store for `label` under
    /// `cache_root`.
    pub fn open(cache_root: &Path, label: &str) -> Result<Self> {
        let dir = cache_root.join("probes");
        std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
            message: "could not create probe store directory",
            path: dir.clone(),
            source,
        })?;
        let path = dir.join(format!("{}.sqlite", safe_probe_label(label)));
        let store = Self { path };
        store.initialize()?;
        Ok(store)
    }

    /// The on-disk path of the store, for diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn initialize(&self) -> Result<()> {
        let connection = Connection::open(&self.path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS probe_cache (
                probe_key TEXT PRIMARY KEY,
                payload_json TEXT NOT NULL
            );
            -- Schema: lean-dup.probes.sqlite.v1
            INSERT OR REPLACE INTO metadata VALUES ('schema', 'lean-dup.probes.sqlite.v1');",
        )?;
        Ok(())
    }

    /// Persist verdicts under their probe keys.
    pub fn store_results(&self, entries: &[(String, ProbeResult)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        perf::record_count(CostClass::SqliteIndex, "probes.store.rows", entries.len() as u64);
        perf::measure_result(CostClass::SqliteIndex, "probes.store.write", || {
            let mut connection = Connection::open(&self.path)?;
            let transaction = connection.transaction()?;
            for (key, result) in entries {
                let payload = serde_json::to_string(result).map_err(Error::from)?;
                transaction.execute(
                    "INSERT OR REPLACE INTO probe_cache VALUES (?1, ?2)",
                    params![key.as_str(), payload],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    /// Look up one verdict by probe key.
    pub fn cached_result(&self, probe_key: &str) -> Result<Option<ProbeResult>> {
        let connection = Connection::open(&self.path)?;
        let payload = connection
            .query_row(
                "SELECT payload_json FROM probe_cache WHERE probe_key = ?1",
                params![probe_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Error::from))
            .transpose()
    }

    /// Row count and on-disk size, for `doctor` diagnostics.
    pub fn facts(&self) -> Result<ProbeStoreFacts> {
        let connection = Connection::open(&self.path)?;
        let rows: i64 = connection.query_row("SELECT COUNT(*) FROM probe_cache", [], |row| row.get(0))?;
        let rows = u64::try_from(rows).unwrap_or(0);
        let bytes = std::fs::metadata(&self.path).map(|metadata| metadata.len()).unwrap_or(0);
        Ok(ProbeStoreFacts { rows, bytes })
    }
}

/// Size facts about the shared probe store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeStoreFacts {
    pub rows: u64,
    pub bytes: u64,
}

/// Labels are caller-controlled; keep the store filename inside the probes dir.
fn safe_probe_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ProbeStore;
    use lean_dup_worker::ProbeResult;
    use tempfile::TempDir;

    fn result(pair_id: &str) -> ProbeResult {
        serde_json::from_value(serde_json::json!({
            "pair_id": pair_id,
            "left_declaration_id": "l",
            "right_declaration_id": "r",
            "status": "ok",
            "same_statement": true,
            "same_up_to_safe_reordering": false,
            "connective_equivalent": false,
            "specializes_left_to_right": false,
            "specializes_right_to_left": false,
            "mutual_implication_shape": false,
            "same_reducible_definition": false,
            "message": null
        }))
        .expect("probe result json")
    }

    #[test]
    fn store_roundtrips_verdicts_across_opens() {
        let cache = TempDir::new().unwrap();
        let store = ProbeStore::open(cache.path(), "audit-workspace").unwrap();
        store.store_results(&[("key-a".to_owned(), result("pair-a"))]).unwrap();
        drop(store);

        let reopened = ProbeStore::open(cache.path(), "audit-workspace").unwrap();
        let cached = reopened.cached_result("key-a").unwrap().expect("cached verdict");
        assert_eq!(cached.pair_id, "pair-a");
        assert!(reopened.cached_result("missing").unwrap().is_none());
        let facts = reopened.facts().unwrap();
        assert_eq!(facts.rows, 1);
        assert!(facts.bytes > 0);
    }

    #[test]
    fn labels_are_confined_to_the_probes_dir() {
        let cache = TempDir::new().unwrap();
        let store = ProbeStore::open(cache.path(), "../escape").unwrap();
        assert!(store.path().starts_with(cache.path().join("probes")));
    }
}
