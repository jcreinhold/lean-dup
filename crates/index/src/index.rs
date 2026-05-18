use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use rustc_hash::FxHashMap as HashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use lean_dup_diagnostics::perf::{self, CostClass};
use lean_dup_diagnostics::progress::Reporter;
use lean_dup_diagnostics::{read, read_to_string};
use lean_dup_project::workspace::{self, ResolvedWorkspace};
use lean_dup_worker::{
    DeclarationRow, ExtractBatch, FeatureRow, FeaturesBatch, Fingerprints, IndexBatch, IndexStreamItem,
    ModuleDescriptor, ProbePair, ProbeResult, RoleFeature, SourceSpan, WorkerClient, WorkerDiagnostic, WorkerError,
    WorkerEvent, WorkerVersion,
};

use crate::{Error, Result};

pub const INDEX_SCHEMA_VERSION: &str = "lean-dup.index.sqlite.v1";
const INDEX_PROVENANCE_VERSION: &str = "lean-dup.index.provenance.v1";
const MATHLIB_DECLARATION_CHUNK_SIZE: usize = 32;
const MAX_MATHLIB_INDEX_THREADS: usize = 2;

/// Builds, resolves, and opens persisted declaration indexes.
///
/// Callers provide typed worker rows and workspace facts. They receive cache
/// status, declaration handles, hydrated declarations, and diagnostics. The
/// store does not expose SQL, table names, storage row ids, insertion order, or
/// cache pointer format.
#[derive(Debug, Clone)]
pub struct IndexStore {
    cache_root: PathBuf,
}

/// Request to build or reuse one declaration index for a resolved workspace.
///
/// The request names the semantic origin and filtering policy. Callers do not
/// provide storage paths or cache ids; those are derived from the cache key
/// ingredients owned by the index store.
#[derive(Debug, Clone)]
pub struct IndexBuildRequest {
    pub workspace: ResolvedWorkspace,
    pub execution_root: Option<PathBuf>,
    pub label: String,
    pub module_root: String,
    pub origin: String,
    pub include_private: bool,
    pub include_generated: bool,
    pub require_oleans: bool,
    pub force: bool,
    pub kind: IndexBuildKind,
}

impl IndexBuildRequest {
    fn execution_root(&self) -> PathBuf {
        self.execution_root
            .clone()
            .unwrap_or_else(|| self.workspace.root.clone())
    }
}

/// Distinguishes local and external cache-key policy without changing the
/// storage interface used by retrieval and audit callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum IndexBuildKind {
    Local,
    External,
    ProjectMathlib,
}

/// Result of building or reusing a persisted index.
///
/// The summary is suitable for CLI reports and later orchestration. It names
/// the resolved index path and cache status, but it does not expose any SQLite
/// identity or schema fact.
#[derive(Debug, Clone, Serialize)]
pub struct IndexSummary {
    pub label: String,
    pub path: PathBuf,
    pub index_dir: PathBuf,
    pub cache_status: CacheStatus,
    pub declaration_count: usize,
    pub diagnostics: Vec<String>,
}

/// Whether an index build reused an existing cache entry or wrote a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheStatus {
    Hit,
    Miss,
}

/// The cache entry an index build request would publish if it were current.
///
/// Cache lifecycle callers use this to compare active cache entries with the
/// current workspace/toolchain/source fingerprint. They do not receive the
/// cache key JSON or any SQLite storage detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpectedIndexEntry {
    pub label: String,
    pub index_dir: PathBuf,
    pub index_path: PathBuf,
    cache_key_json: String,
}

#[cfg(test)]
impl ExpectedIndexEntry {
    pub fn for_test(label: impl Into<String>, index_dir: PathBuf, cache_key_json: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            index_path: index_dir.join("index.sqlite"),
            index_dir,
            cache_key_json: cache_key_json.into(),
        }
    }
}

/// User-facing ways to locate an existing index.
///
/// Labels resolve through the cache root. Paths may point either at an index
/// directory or directly at its SQLite file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum IndexReference {
    Label(String),
    Path(PathBuf),
}

/// An opened declaration index.
///
/// Callers can ask semantic questions in terms of opaque keys and declaration
/// handles. The opened index keeps storage handles private and hydrates only
/// the requested declarations.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OpenedIndex {
    path: PathBuf,
}

/// Stable facts about an opened index needed by orchestration layers.
///
/// These facts identify the index for diagnostics and origin-aware pairing.
/// They do not describe how declarations or postings are stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct OpenedIndexFacts {
    pub origin: String,
    pub label: Option<String>,
    pub declaration_count: usize,
    pub path: PathBuf,
    pub provenance: IndexProvenance,
}

/// Source provenance carried by an index.
///
/// Callers may use this to decide whether an index can support proof-grade
/// comparison in the current audit environment. They must not depend on the
/// metadata storage key or SQLite layout used to persist it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexProvenance {
    pub version: String,
    pub kind: IndexProvenanceKind,
    pub source_root: Option<PathBuf>,
    pub execution_root: Option<PathBuf>,
    pub execution_policy: String,
    pub module_root: String,
    pub protocol_version: Option<String>,
    pub worker_version: Option<String>,
    pub extract_version: Option<String>,
    pub features_version: Option<String>,
    pub probe_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IndexProvenanceKind {
    Static,
    SourceBacked,
}

impl IndexProvenance {
    pub fn static_index(module_root: impl Into<String>) -> Self {
        Self {
            version: INDEX_PROVENANCE_VERSION.to_owned(),
            kind: IndexProvenanceKind::Static,
            source_root: None,
            execution_root: None,
            execution_policy: "static-index".to_owned(),
            module_root: module_root.into(),
            protocol_version: None,
            worker_version: None,
            extract_version: None,
            features_version: None,
            probe_version: None,
        }
    }
}

/// Opaque declaration identity returned by index queries.
///
/// Handles are stable within one index cache context and are the only
/// declaration identities accepted by hydration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeclarationHandle(String);

impl DeclarationHandle {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn for_test(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

/// Query over Lean-owned opaque semantic keys.
///
/// Callers may compare and store the keys Lean emitted, but must not parse
/// them or reconstruct them from display text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SemanticFeatureQuery {
    pub fingerprints: Vec<SemanticFingerprintFeature>,
    pub role_features: Vec<SemanticRoleFeature>,
}

/// One requested opaque fingerprint key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(dead_code)]
pub struct SemanticFingerprintFeature {
    pub kind: FingerprintKind,
    pub key: String,
}

/// Supported Lean-owned fingerprint classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(dead_code)]
pub enum FingerprintKind {
    Statement,
    SafeBinderPermutation,
    ConnectiveShape,
    ConclusionShape,
}

/// One requested opaque role-feature key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(dead_code)]
pub struct SemanticRoleFeature {
    pub role: String,
    pub key: String,
}

/// One opaque semantic key that can contribute retrieval evidence.
///
/// Callers may pass keys emitted by Lean and compare matching handles. The key
/// value remains opaque and display text must not be used as a replacement key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(dead_code)]
pub enum SemanticFeatureKey {
    Fingerprint(SemanticFingerprintFeature),
    RoleFeature(SemanticRoleFeature),
}

/// The number of declarations matched by one requested semantic key.
///
/// Retrieval uses this to distinguish selective evidence from broad evidence
/// without hydrating declarations first.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct FeatureMatchCount {
    pub key: SemanticFeatureKey,
    pub count: usize,
}

/// One declaration handle matched by one requested semantic key.
///
/// The matched key is included so retrieval can explain why a candidate was
/// returned without reopening the declaration or inspecting storage.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct FeatureMatch {
    pub key: SemanticFeatureKey,
    pub handle: DeclarationHandle,
}

/// A declaration hydrated from an index by handle.
///
/// The row contains only semantic facts and display/source metadata needed by
/// later retrieval, ranking, and reporting layers. It does not reveal storage
/// identity or table layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct HydratedDeclaration {
    pub handle: DeclarationHandle,
    pub declaration_id: String,
    pub origin: String,
    pub module: String,
    pub qualified_name: String,
    pub display_name: String,
    pub kind: String,
    pub visibility: String,
    pub modifiers: Vec<String>,
    pub source_span: Option<SourceSpan>,
    pub statement_text: String,
    pub status_flags: Vec<String>,
    pub feature_version: String,
    pub fingerprints: Fingerprints,
    pub role_features: Vec<RoleFeature>,
    pub binder_count: u64,
    pub low_signal_markers: Vec<String>,
}

/// One probe-cache update for a candidate pair.
///
/// Probe cache callers use worker-domain pair and result values. The index
/// derives the cache key and persists the payload without exposing storage
/// shape or JSON fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub struct ProbeCacheEntry {
    pub cache_key: String,
    pub pair: ProbePair,
    pub result: ProbeResult,
}

#[derive(Debug, Clone, Serialize)]
struct IndexCacheKey {
    index_schema_version: &'static str,
    index_provenance_version: &'static str,
    protocol_version: String,
    worker_version: String,
    lean_version: Option<String>,
    extract_version: String,
    features_version: String,
    probe_version: String,
    worker_source_digest: Option<String>,
    label: String,
    kind: IndexBuildKind,
    origin: String,
    workspace_root: String,
    module_root: String,
    selected_roots: Vec<String>,
    include_private: bool,
    include_generated: bool,
    require_oleans: bool,
    lean_toolchain: Option<String>,
    lakefile: Option<FileDigest>,
    lake_manifest: Option<FileDigest>,
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

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LatestPointer {
    index_dir: PathBuf,
}

impl IndexStore {
    pub fn new(cache_root: PathBuf) -> Self {
        Self { cache_root }
    }

    pub fn build_or_reuse(
        &self,
        request: IndexBuildRequest,
        worker: &WorkerClient,
        reporter: &mut Reporter,
    ) -> Result<IndexSummary> {
        require_oleans_if_requested(&request)?;

        let version_call = reporter.measure("worker.version", |_| worker.version(request.execution_root()))?;
        record_worker_events(reporter, &version_call.events);
        let version = version_call.rows.into_iter().next().ok_or_else(|| Error::Index {
            message: "worker version returned no rows".to_owned(),
        })?;

        let expected = self.expected_entry(&request, &version)?;
        let index_dir = expected.index_dir.clone();
        let index_path = expected.index_path.clone();

        if index_path.exists() && !request.force && sqlite_cache_is_current(&index_path, &expected.cache_key_json)? {
            let declaration_count = declaration_count(&index_path)?;
            self.write_latest(&request.label, &index_dir)?;
            reporter.event(
                "index",
                Some(declaration_count as u64),
                Some(declaration_count as u64),
                format!("reused index {}", index_path.display()),
            );
            return Ok(IndexSummary {
                label: request.label,
                path: index_path,
                index_dir,
                cache_status: CacheStatus::Hit,
                declaration_count,
                diagnostics: diagnostics_to_strings(version_call.diagnostics),
            });
        }

        let mut diagnostics = diagnostics_to_strings(version_call.diagnostics);
        if request.kind == IndexBuildKind::ProjectMathlib {
            let build = write_batched_sqlite_index(
                &index_path,
                &expected.cache_key_json,
                &request,
                &version,
                worker,
                reporter,
            )?;
            diagnostics.extend(build.diagnostics);
        } else {
            let modules = modules_for(&request);
            let declarations = reporter.measure("worker.extract", |_| {
                worker.extract_batch(ExtractBatch {
                    workspace_root: request.execution_root(),
                    modules: modules.clone(),
                    include_private: request.include_private,
                    include_generated: request.include_generated,
                })
            })?;
            record_worker_events(reporter, &declarations.events);

            let features = reporter.measure("worker.features", |_| {
                worker.features_batch(FeaturesBatch {
                    workspace_root: request.execution_root(),
                    modules,
                    declaration_ids: None,
                    include_private: request.include_private,
                    include_generated: request.include_generated,
                })
            })?;
            record_worker_events(reporter, &features.events);

            write_sqlite_index(
                &index_path,
                &expected.cache_key_json,
                &request,
                &version,
                declarations.rows,
                features.rows,
            )?;
            diagnostics.extend(diagnostics_to_strings(declarations.diagnostics));
            diagnostics.extend(diagnostics_to_strings(features.diagnostics));
        }
        self.write_latest(&request.label, &index_dir)?;

        let declaration_count = declaration_count(&index_path)?;
        reporter.event(
            "index",
            Some(declaration_count as u64),
            Some(declaration_count as u64),
            format!("built index {}", index_path.display()),
        );

        Ok(IndexSummary {
            label: request.label,
            path: index_path,
            index_dir,
            cache_status: CacheStatus::Miss,
            declaration_count,
            diagnostics,
        })
    }

    pub fn expected_entry(&self, request: &IndexBuildRequest, version: &WorkerVersion) -> Result<ExpectedIndexEntry> {
        let cache_key = index_cache_key(request, version)?;
        let cache_key_json = serde_json::to_string(&cache_key)?;
        let cache_id = hex_digest(cache_key_json.as_bytes());
        let index_dir = self.label_dir(&request.label).join(cache_id);
        Ok(ExpectedIndexEntry {
            label: request.label.clone(),
            index_path: index_dir.join("index.sqlite"),
            index_dir,
            cache_key_json,
        })
    }

    pub fn cache_entry_is_current(&self, entry: &ExpectedIndexEntry) -> Result<bool> {
        sqlite_cache_is_current(&entry.index_path, &entry.cache_key_json)
    }

    #[allow(dead_code)]
    pub fn resolve(&self, reference: IndexReference) -> Result<OpenedIndex> {
        let path = match reference {
            IndexReference::Label(label) => {
                let pointer_path = self.label_dir(&label).join("latest.json");
                let pointer: LatestPointer = serde_json::from_str(&read_to_string(pointer_path.clone())?)?;
                pointer.index_dir.join("index.sqlite")
            }
            IndexReference::Path(path) => {
                let expanded = expand_home(&path);
                if expanded.is_dir() {
                    expanded.join("index.sqlite")
                } else {
                    expanded
                }
            }
        };
        if path.file_name().and_then(|name| name.to_str()) != Some("index.sqlite") {
            return Err(Error::Index {
                message: format!("index path must resolve to index.sqlite: {}", path.display()),
            });
        }
        validate_index_schema(&path)?;
        Ok(OpenedIndex { path })
    }

    fn label_dir(&self, label: &str) -> PathBuf {
        self.cache_root.join("indexes").join(safe_label(label))
    }

    fn write_latest(&self, label: &str, index_dir: &Path) -> Result<()> {
        let pointer = self.label_dir(label).join("latest.json");
        if let Some(parent) = pointer.parent() {
            create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&serde_json::json!({
            "index_dir": index_dir,
        }))?;
        write_text(&pointer, &body)
    }
}

#[allow(dead_code)]
impl OpenedIndex {
    pub fn for_test(path: PathBuf) -> Self {
        Self { path }
    }

    /// Return stable index facts needed for diagnostics and origin-aware pairing.
    pub fn facts(&self) -> Result<OpenedIndexFacts> {
        let connection = open_readonly(&self.path)?;
        let origin = metadata_value(&connection, "origin")?.ok_or_else(|| Error::Index {
            message: "index metadata is missing origin".to_owned(),
        })?;
        let label = metadata_value(&connection, "label")?;
        let module_root = metadata_value(&connection, "module_root")?.unwrap_or_default();
        let provenance = metadata_value(&connection, "provenance_json")?
            .map(|json| serde_json::from_str(&json))
            .transpose()?
            .unwrap_or_else(|| IndexProvenance::static_index(module_root));
        Ok(OpenedIndexFacts {
            origin,
            label,
            declaration_count: declaration_count(&self.path)?,
            path: self.path.clone(),
            provenance,
        })
    }

    /// Count matches for each requested semantic key without hydrating rows.
    pub fn feature_match_counts(&self, keys: &[SemanticFeatureKey]) -> Result<Vec<FeatureMatchCount>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        perf::record_count(CostClass::SqliteIndex, "sqlite.posting_count.keys", keys.len() as u64);
        perf::measure_result(CostClass::SqliteIndex, "sqlite.feature_match_counts", || {
            let connection = open_readonly(&self.path)?;
            let mut counts = Vec::with_capacity(keys.len());
            for key in keys {
                if key.is_empty() {
                    continue;
                }
                counts.push(FeatureMatchCount {
                    key: key.clone(),
                    count: feature_match_count(&connection, key)?,
                });
            }
            Ok(counts)
        })
    }

    /// Return handles matched by each requested semantic key.
    ///
    /// The returned handles remain opaque; callers hydrate only the handles
    /// they decide to keep.
    pub fn matched_feature_handles(&self, keys: &[SemanticFeatureKey]) -> Result<Vec<FeatureMatch>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        perf::record_count(CostClass::SqliteIndex, "sqlite.matched_posting.keys", keys.len() as u64);
        perf::measure_result(CostClass::SqliteIndex, "sqlite.matched_feature_handles", || {
            let connection = open_readonly(&self.path)?;
            let mut postings = Vec::new();
            for key in keys {
                if key.is_empty() {
                    continue;
                }
                postings.extend(
                    feature_match_handles(&connection, key)?
                        .into_iter()
                        .map(|handle| FeatureMatch {
                            key: key.clone(),
                            handle,
                        }),
                );
            }
            perf::record_count(
                CostClass::SqliteIndex,
                "sqlite.matched_posting.rows",
                postings.len() as u64,
            );
            Ok(postings)
        })
    }

    /// Return every declaration handle in deterministic order.
    pub fn all_handles(&self) -> Result<Vec<DeclarationHandle>> {
        let connection = open_readonly(&self.path)?;
        let mut statement =
            connection.prepare("SELECT handle FROM declarations ORDER BY qualified_name, declaration_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut handles = Vec::new();
        for row in rows {
            handles.push(DeclarationHandle(row?));
        }
        Ok(handles)
    }

    fn matching_handles(&self, query: SemanticFeatureQuery) -> Result<Vec<DeclarationHandle>> {
        let connection = open_readonly(&self.path)?;
        let mut handles = BTreeSet::new();
        for fingerprint in query.fingerprints {
            if fingerprint.key.is_empty() {
                continue;
            }
            let mut statement = connection
                .prepare("SELECT declaration_handle FROM fingerprint_postings WHERE kind = ?1 AND key = ?2")?;
            let rows = statement.query_map(params![fingerprint.kind.as_str(), fingerprint.key], |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                handles.insert(DeclarationHandle(row?));
            }
        }
        for role_feature in query.role_features {
            if role_feature.key.is_empty() {
                continue;
            }
            let mut statement = connection
                .prepare("SELECT declaration_handle FROM role_feature_postings WHERE role = ?1 AND key = ?2")?;
            let rows = statement.query_map(params![role_feature.role, role_feature.key], |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                handles.insert(DeclarationHandle(row?));
            }
        }
        Ok(handles.into_iter().collect())
    }

    pub fn hydrate(&self, handles: &[DeclarationHandle]) -> Result<Vec<HydratedDeclaration>> {
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        perf::record_count(
            CostClass::SqliteIndex,
            "sqlite.hydrate.declarations",
            handles.len() as u64,
        );
        perf::measure_result(CostClass::SqliteIndex, "sqlite.hydrate", || {
            let connection = open_readonly(&self.path)?;
            let mut hydrated = Vec::with_capacity(handles.len());
            for handle in handles {
                let declaration = load_declaration(&connection, handle)?;
                hydrated.push(declaration);
            }
            Ok(hydrated)
        })
    }

    pub fn cache_probe_results(&self, entries: &[ProbeCacheEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        perf::record_count(CostClass::SqliteIndex, "sqlite.probe_cache.rows", entries.len() as u64);
        perf::measure_result(CostClass::SqliteIndex, "sqlite.probe_cache.write", || {
            let mut connection = Connection::open(&self.path)?;
            let transaction = connection.transaction()?;
            for entry in entries {
                transaction.execute(
                    "INSERT OR REPLACE INTO probe_cache VALUES (?1, ?2)",
                    params![entry.cache_key.as_str(), probe_cache_payload(&entry.result)],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    pub fn cached_probe_result(&self, cache_key: &str) -> Result<Option<ProbeResult>> {
        let connection = open_readonly(&self.path)?;
        let payload = connection
            .query_row(
                "SELECT payload_json FROM probe_cache WHERE pair_key = ?1",
                params![cache_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Error::from))
            .transpose()
    }
}

impl SemanticFeatureKey {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Fingerprint(query) => query.key.is_empty(),
            Self::RoleFeature(query) => query.key.is_empty(),
        }
    }
}

impl FingerprintKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::SafeBinderPermutation => "safe_binder_permutation",
            Self::ConnectiveShape => "connective_shape",
            Self::ConclusionShape => "conclusion_shape",
        }
    }
}

fn require_oleans_if_requested(request: &IndexBuildRequest) -> Result<()> {
    if !request.require_oleans {
        return Ok(());
    }
    let olean_root = if request.kind == IndexBuildKind::ProjectMathlib {
        request.workspace.root.clone()
    } else {
        request.execution_root()
    };
    let missing = request
        .workspace
        .source_files
        .iter()
        .filter(|source| !workspace::olean_exists(&olean_root, &source.module))
        .map(|source| source.module.clone())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        let sample = missing.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        Err(Error::Index {
            message: format!(
                "missing compiled oleans for index ({} missing; sample: {sample})",
                missing.len()
            ),
        })
    }
}

fn modules_for(request: &IndexBuildRequest) -> Vec<ModuleDescriptor> {
    let source_root = (request.execution_root().as_path() != request.workspace.root.as_path())
        .then(|| request.workspace.root.clone());
    request
        .workspace
        .source_files
        .iter()
        .map(|source| ModuleDescriptor {
            module: source.module.clone(),
            origin: request.origin.clone(),
            source_root: source_root.clone(),
        })
        .collect()
}

fn index_cache_key(request: &IndexBuildRequest, version: &WorkerVersion) -> Result<IndexCacheKey> {
    let shared_mathlib = request.kind == IndexBuildKind::ProjectMathlib;
    let execution_root = request.execution_root();
    Ok(IndexCacheKey {
        index_schema_version: INDEX_SCHEMA_VERSION,
        index_provenance_version: INDEX_PROVENANCE_VERSION,
        protocol_version: version.protocol_version.clone(),
        worker_version: version.worker_version.clone(),
        lean_version: version.lean_version.clone(),
        extract_version: version.extract_version.clone(),
        features_version: version.features_version.clone(),
        probe_version: version.probe_version.clone(),
        worker_source_digest: worker_source_digest()?,
        label: request.label.clone(),
        kind: request.kind,
        origin: request.origin.clone(),
        workspace_root: if shared_mathlib {
            "project-pinned-mathlib".to_owned()
        } else {
            request.workspace.root.display().to_string()
        },
        module_root: request.module_root.clone(),
        selected_roots: request.workspace.selected_roots.clone(),
        include_private: request.include_private,
        include_generated: request.include_generated,
        require_oleans: request.require_oleans,
        lean_toolchain: optional_text(if shared_mathlib {
            execution_root.join("lean-toolchain")
        } else {
            request.workspace.lean_toolchain_path()
        })?,
        lakefile: if shared_mathlib {
            file_digest_relative(&request.workspace.root, request.workspace.lakefile.clone())?
        } else {
            file_digest(request.workspace.lakefile.clone())?
        },
        lake_manifest: if shared_mathlib {
            file_digest_relative(&request.workspace.root, request.workspace.manifest_path())?
        } else {
            file_digest(request.workspace.manifest_path())?
        },
        sources: request
            .workspace
            .source_files
            .iter()
            .map(|source| {
                Ok(SourceDigest {
                    module: source.module.clone(),
                    path: if shared_mathlib {
                        relative_path(&request.workspace.root, &source.path)
                    } else {
                        source.path.display().to_string()
                    },
                    digest: optional_hash(source.path.clone())?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

fn write_sqlite_index(
    index_path: &Path,
    cache_key_json: &str,
    request: &IndexBuildRequest,
    version: &WorkerVersion,
    declarations: Vec<DeclarationRow>,
    features: Vec<FeatureRow>,
) -> Result<()> {
    let Some(index_dir) = index_path.parent() else {
        return Err(Error::Index {
            message: format!("index path has no parent: {}", index_path.display()),
        });
    };
    create_dir_all(index_dir)?;
    let temp_path = index_path.with_extension("tmp.sqlite");
    if temp_path.exists() {
        remove_file(&temp_path)?;
    }

    perf::record_count(
        CostClass::SqliteIndex,
        "sqlite.index.declarations",
        declarations.len() as u64,
    );
    perf::record_count(CostClass::SqliteIndex, "sqlite.index.features", features.len() as u64);
    perf::measure_result(CostClass::SqliteIndex, "sqlite.index.write", || {
        let features_by_id = features
            .into_iter()
            .map(|feature| (feature.declaration_id.clone(), feature))
            .collect::<HashMap<_, _>>();
        let mut connection = Connection::open(&temp_path)?;
        initialize_schema(&connection)?;
        let transaction = connection.transaction()?;
        write_metadata(&transaction, cache_key_json, request, version)?;
        for declaration in declarations {
            let Some(feature) = features_by_id.get(&declaration.declaration_id) else {
                return Err(Error::Index {
                    message: format!(
                        "worker emitted declaration without feature row: {}",
                        declaration.qualified_name
                    ),
                });
            };
            insert_declaration(&transaction, &declaration, feature)?;
        }
        transaction.commit()?;
        replace_file(&temp_path, index_path)?;
        Ok(())
    })
}

#[derive(Debug)]
struct BatchedIndexBuild {
    diagnostics: Vec<String>,
}

struct StreamingIndexWriter {
    pending_declarations: HashMap<String, DeclarationRow>,
    pending_features: HashMap<String, FeatureRow>,
    written: usize,
}

fn write_batched_sqlite_index(
    index_path: &Path,
    cache_key_json: &str,
    request: &IndexBuildRequest,
    version: &WorkerVersion,
    worker: &WorkerClient,
    reporter: &mut Reporter,
) -> Result<BatchedIndexBuild> {
    let Some(index_dir) = index_path.parent() else {
        return Err(Error::Index {
            message: format!("index path has no parent: {}", index_path.display()),
        });
    };
    create_dir_all(index_dir)?;
    let temp_path = index_path.with_extension("tmp.sqlite");
    if temp_path.exists() {
        remove_file(&temp_path)?;
    }
    let connection = Connection::open(&temp_path)?;
    initialize_schema(&connection)?;
    write_metadata(&connection, cache_key_json, request, version)?;

    let modules = modules_for(request);
    let total_modules = modules.len();
    let index_threads = mathlib_index_threads();
    let mut writer = StreamingIndexWriter {
        pending_declarations: HashMap::default(),
        pending_features: HashMap::default(),
        written: 0,
    };
    reporter.event(
        "index.mathlib",
        Some(0),
        Some(total_modules as u64),
        format!("streaming mathlib index with {index_threads} Lean task(s) over {total_modules} modules"),
    );
    connection.execute_batch("BEGIN IMMEDIATE")?;
    let mut diagnostics = Vec::new();
    let mut insertion_error = None;
    let worker_result = reporter.measure("worker.index", |measure_reporter| {
        worker
            .index_stream(
                IndexBatch {
                    workspace_root: request.execution_root(),
                    modules,
                    include_private: request.include_private,
                    include_generated: request.include_generated,
                    declaration_chunk_size: MATHLIB_DECLARATION_CHUNK_SIZE,
                    declaration_parallelism: index_threads,
                },
                &mut |item| {
                    match item {
                        IndexStreamItem::Declaration(row) => {
                            if let Err(error) = writer.accept_declaration(&connection, row) {
                                insertion_error = Some(error);
                                return Err(WorkerError::Protocol {
                                    message: "could not insert streamed declaration row".to_owned(),
                                });
                            }
                        }
                        IndexStreamItem::Feature(row) => {
                            if let Err(error) = writer.accept_feature(&connection, row) {
                                insertion_error = Some(error);
                                return Err(WorkerError::Protocol {
                                    message: "could not insert streamed feature row".to_owned(),
                                });
                            }
                        }
                        IndexStreamItem::Event(event) => {
                            measure_reporter.event(
                                format!("worker.{}", event.phase),
                                event.current,
                                event.total,
                                event.message,
                            );
                        }
                        IndexStreamItem::Diagnostic(diagnostic) => {
                            diagnostics.push(format!("{}: {}", diagnostic.code, diagnostic.message));
                        }
                    }
                    Ok(())
                },
            )
            .map(|call| call.diagnostics)
    });
    match worker_result {
        Ok(worker_diagnostics) => {
            diagnostics.extend(diagnostics_to_strings(worker_diagnostics));
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            if let Some(error) = insertion_error {
                return Err(error);
            }
            return Err(error.into());
        }
    }
    if let Some(error) = insertion_error {
        let _ = connection.execute_batch("ROLLBACK");
        return Err(error);
    }
    writer.finish()?;
    connection.execute_batch("COMMIT")?;
    replace_file(&temp_path, index_path)?;
    Ok(BatchedIndexBuild { diagnostics })
}

fn mathlib_index_threads() -> usize {
    std::env::var("LEAN_NUM_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|jobs| *jobs > 0)
        .map(|jobs| jobs.min(MAX_MATHLIB_INDEX_THREADS))
        .unwrap_or(1)
}

impl StreamingIndexWriter {
    fn accept_declaration(&mut self, connection: &Connection, declaration: DeclarationRow) -> Result<()> {
        let id = declaration.declaration_id.clone();
        if let Some(feature) = self.pending_features.remove(&id) {
            self.insert_pair(connection, &declaration, &feature)
        } else {
            self.pending_declarations.insert(id, declaration);
            Ok(())
        }
    }

    fn accept_feature(&mut self, connection: &Connection, feature: FeatureRow) -> Result<()> {
        let id = feature.declaration_id.clone();
        if let Some(declaration) = self.pending_declarations.remove(&id) {
            self.insert_pair(connection, &declaration, &feature)
        } else {
            self.pending_features.insert(id, feature);
            Ok(())
        }
    }

    fn insert_pair(
        &mut self,
        connection: &Connection,
        declaration: &DeclarationRow,
        feature: &FeatureRow,
    ) -> Result<()> {
        insert_declaration(connection, declaration, feature)?;
        self.written += 1;
        if self.written.is_multiple_of(1000) {
            perf::record_count(CostClass::SqliteIndex, "sqlite.index.declarations", self.written as u64);
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        if !self.pending_declarations.is_empty() || !self.pending_features.is_empty() {
            return Err(Error::Index {
                message: format!(
                    "worker index stream ended with {} declarations and {} features unmatched",
                    self.pending_declarations.len(),
                    self.pending_features.len()
                ),
            });
        }
        perf::record_count(CostClass::SqliteIndex, "sqlite.index.declarations", self.written as u64);
        perf::record_count(CostClass::SqliteIndex, "sqlite.index.features", self.written as u64);
        Ok(())
    }
}

fn initialize_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "
        PRAGMA journal_mode = OFF;
        PRAGMA synchronous = OFF;

        CREATE TABLE metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE declarations (
            handle TEXT PRIMARY KEY,
            declaration_id TEXT NOT NULL UNIQUE,
            origin TEXT NOT NULL,
            module TEXT NOT NULL,
            qualified_name TEXT NOT NULL,
            display_name TEXT NOT NULL,
            kind TEXT NOT NULL,
            visibility TEXT NOT NULL,
            modifiers_json TEXT NOT NULL,
            source_span_json TEXT,
            statement_text TEXT NOT NULL,
            status_flags_json TEXT NOT NULL
        );

        CREATE TABLE declaration_features (
            declaration_handle TEXT PRIMARY KEY,
            feature_version TEXT NOT NULL,
            fingerprints_json TEXT NOT NULL,
            role_features_json TEXT NOT NULL,
            binder_count INTEGER NOT NULL,
            low_signal_markers_json TEXT NOT NULL
        );

        CREATE TABLE fingerprint_postings (
            kind TEXT NOT NULL,
            key TEXT NOT NULL,
            declaration_handle TEXT NOT NULL
        );

        CREATE TABLE role_feature_postings (
            role TEXT NOT NULL,
            key TEXT NOT NULL,
            display TEXT,
            declaration_handle TEXT NOT NULL
        );

        CREATE TABLE probe_cache (
            pair_key TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );

        CREATE INDEX fingerprint_postings_key ON fingerprint_postings(kind, key);
        CREATE INDEX role_feature_postings_key ON role_feature_postings(role, key);
        CREATE INDEX declarations_name ON declarations(qualified_name);
        ",
    )?;
    Ok(())
}

fn write_metadata(
    connection: &Connection,
    cache_key_json: &str,
    request: &IndexBuildRequest,
    version: &WorkerVersion,
) -> Result<()> {
    let provenance = index_provenance(request, version)?;
    let values = [
        ("schema_version", INDEX_SCHEMA_VERSION.to_owned()),
        ("cache_key", cache_key_json.to_owned()),
        ("label", request.label.clone()),
        ("module_root", request.module_root.clone()),
        ("origin", request.origin.clone()),
        ("kind", serde_json::to_string(&request.kind)?),
        ("provenance_json", serde_json::to_string(&provenance)?),
    ];
    for (key, value) in values {
        connection.execute("INSERT INTO metadata (key, value) VALUES (?1, ?2)", params![key, value])?;
    }
    Ok(())
}

fn index_provenance(request: &IndexBuildRequest, version: &WorkerVersion) -> Result<IndexProvenance> {
    let execution_policy = match request.kind {
        IndexBuildKind::Local => "workspace-lake-environment",
        IndexBuildKind::External => "indexed-workspace-lake-environment",
        IndexBuildKind::ProjectMathlib => "project-pinned-mathlib-lake-environment",
    };
    Ok(IndexProvenance {
        version: INDEX_PROVENANCE_VERSION.to_owned(),
        kind: IndexProvenanceKind::SourceBacked,
        source_root: Some(request.workspace.root.clone()),
        execution_root: Some(request.execution_root()),
        execution_policy: execution_policy.to_owned(),
        module_root: request.module_root.clone(),
        protocol_version: Some(version.protocol_version.clone()),
        worker_version: Some(version.worker_version.clone()),
        extract_version: Some(version.extract_version.clone()),
        features_version: Some(version.features_version.clone()),
        probe_version: Some(version.probe_version.clone()),
    })
}

fn insert_declaration(connection: &Connection, declaration: &DeclarationRow, feature: &FeatureRow) -> Result<()> {
    let handle = DeclarationHandle(handle_for(&declaration.declaration_id));
    connection.execute(
        "INSERT INTO declarations VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            handle.0,
            declaration.declaration_id,
            declaration.origin,
            declaration.module,
            declaration.qualified_name,
            declaration.display_name,
            declaration.kind,
            declaration.visibility,
            serde_json::to_string(&declaration.modifiers)?,
            optional_json(&declaration.source_span)?,
            declaration.statement_text,
            serde_json::to_string(&declaration.status_flags)?,
        ],
    )?;
    connection.execute(
        "INSERT INTO declaration_features VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            handle.0,
            feature.feature_version,
            serde_json::to_string(&feature.fingerprints)?,
            serde_json::to_string(&feature.role_features)?,
            i64::try_from(feature.binder_count).map_err(|_| Error::Index {
                message: format!("binder count exceeds sqlite integer range: {}", feature.binder_count),
            })?,
            serde_json::to_string(&feature.low_signal_markers)?,
        ],
    )?;
    insert_fingerprint(
        connection,
        FingerprintKind::Statement,
        &feature.fingerprints.statement,
        &handle,
    )?;
    insert_fingerprint(
        connection,
        FingerprintKind::SafeBinderPermutation,
        &feature.fingerprints.safe_binder_permutation,
        &handle,
    )?;
    insert_fingerprint(
        connection,
        FingerprintKind::ConnectiveShape,
        &feature.fingerprints.connective_shape,
        &handle,
    )?;
    insert_fingerprint(
        connection,
        FingerprintKind::ConclusionShape,
        &feature.fingerprints.conclusion_shape,
        &handle,
    )?;
    for role_feature in &feature.role_features {
        connection.execute(
            "INSERT INTO role_feature_postings VALUES (?1, ?2, ?3, ?4)",
            params![role_feature.role, role_feature.key, role_feature.display, handle.0,],
        )?;
    }
    Ok(())
}

fn insert_fingerprint(
    connection: &Connection,
    kind: FingerprintKind,
    key: &str,
    handle: &DeclarationHandle,
) -> Result<()> {
    if key.is_empty() {
        return Ok(());
    }
    connection.execute(
        "INSERT INTO fingerprint_postings VALUES (?1, ?2, ?3)",
        params![kind.as_str(), key, handle.0],
    )?;
    Ok(())
}

fn optional_json<T: Serialize>(value: &Option<T>) -> Result<Option<String>> {
    Ok(match value {
        Some(value) => Some(serde_json::to_string(value)?),
        None => None,
    })
}

fn sqlite_cache_is_current(index_path: &Path, cache_key_json: &str) -> Result<bool> {
    perf::measure_result(CostClass::SqliteIndex, "sqlite.cache_check", || {
        let connection = match open_readonly(index_path) {
            Ok(connection) => connection,
            Err(_) => return Ok(false),
        };
        let schema_version = match metadata_value(&connection, "schema_version") {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let cache_key = match metadata_value(&connection, "cache_key") {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        Ok(schema_version.as_deref() == Some(INDEX_SCHEMA_VERSION) && cache_key.as_deref() == Some(cache_key_json))
    })
}

#[allow(dead_code)]
fn validate_index_schema(index_path: &Path) -> Result<()> {
    let connection = open_readonly(index_path)?;
    if metadata_value(&connection, "schema_version")?.as_deref() == Some(INDEX_SCHEMA_VERSION) {
        Ok(())
    } else {
        Err(Error::Index {
            message: format!("unsupported index schema in {}", index_path.display()),
        })
    }
}

fn metadata_value(connection: &Connection, key: &str) -> Result<Option<String>> {
    Ok(connection
        .query_row("SELECT value FROM metadata WHERE key = ?1", params![key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn feature_match_count(connection: &Connection, key: &SemanticFeatureKey) -> Result<usize> {
    let count = match key {
        SemanticFeatureKey::Fingerprint(query) => connection.query_row(
            "SELECT COUNT(*) FROM fingerprint_postings WHERE kind = ?1 AND key = ?2",
            params![query.kind.as_str(), query.key],
            |row| row.get::<_, i64>(0),
        )?,
        SemanticFeatureKey::RoleFeature(query) => connection.query_row(
            "SELECT COUNT(*) FROM role_feature_postings WHERE role = ?1 AND key = ?2",
            params![query.role, query.key],
            |row| row.get::<_, i64>(0),
        )?,
    };
    Ok(count as usize)
}

fn feature_match_handles(connection: &Connection, key: &SemanticFeatureKey) -> Result<Vec<DeclarationHandle>> {
    let mut handles = Vec::new();
    match key {
        SemanticFeatureKey::Fingerprint(query) => {
            let mut statement = connection
                .prepare("SELECT declaration_handle FROM fingerprint_postings WHERE kind = ?1 AND key = ?2")?;
            let rows = statement.query_map(params![query.kind.as_str(), query.key], |row| row.get::<_, String>(0))?;
            for row in rows {
                handles.push(DeclarationHandle(row?));
            }
        }
        SemanticFeatureKey::RoleFeature(query) => {
            let mut statement = connection
                .prepare("SELECT declaration_handle FROM role_feature_postings WHERE role = ?1 AND key = ?2")?;
            let rows = statement.query_map(params![query.role, query.key], |row| row.get::<_, String>(0))?;
            for row in rows {
                handles.push(DeclarationHandle(row?));
            }
        }
    }
    Ok(handles)
}

fn declaration_count(index_path: &Path) -> Result<usize> {
    perf::measure_result(CostClass::SqliteIndex, "sqlite.declaration_count", || {
        let connection = open_readonly(index_path)?;
        let count = connection.query_row("SELECT COUNT(*) FROM declarations", [], |row| row.get::<_, i64>(0))?;
        Ok(count as usize)
    })
}

#[allow(dead_code)]
fn load_declaration(connection: &Connection, handle: &DeclarationHandle) -> Result<HydratedDeclaration> {
    let row = connection
        .query_row(
            "
            SELECT
              d.declaration_id,
              d.origin,
              d.module,
              d.qualified_name,
              d.display_name,
              d.kind,
              d.visibility,
              d.modifiers_json,
              d.source_span_json,
              d.statement_text,
              d.status_flags_json,
              f.feature_version,
              f.fingerprints_json,
              f.role_features_json,
              f.binder_count,
              f.low_signal_markers_json
            FROM declarations d
            JOIN declaration_features f ON f.declaration_handle = d.handle
            WHERE d.handle = ?1
            ",
            params![handle.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, i64>(14)?,
                    row.get::<_, String>(15)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| Error::Index {
            message: "declaration handle was not found in index".to_owned(),
        })?;

    Ok(HydratedDeclaration {
        handle: handle.clone(),
        declaration_id: row.0,
        origin: row.1,
        module: row.2,
        qualified_name: row.3,
        display_name: row.4,
        kind: row.5,
        visibility: row.6,
        modifiers: serde_json::from_str(&row.7)?,
        source_span: match row.8 {
            Some(json) => Some(serde_json::from_str(&json)?),
            None => None,
        },
        statement_text: row.9,
        status_flags: serde_json::from_str(&row.10)?,
        feature_version: row.11,
        fingerprints: serde_json::from_str(&row.12)?,
        role_features: serde_json::from_str(&row.13)?,
        binder_count: row.14 as u64,
        low_signal_markers: serde_json::from_str(&row.15)?,
    })
}

fn handle_for(declaration_id: &str) -> String {
    format!("decl-{}", hex_digest(declaration_id.as_bytes()))
}

fn safe_label(label: &str) -> String {
    let safe = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if safe.is_empty() || safe == "." || safe == ".." {
        "index".to_owned()
    } else {
        safe
    }
}

fn open_readonly(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn optional_text(path: PathBuf) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_to_string(path)?.trim().to_owned()))
}

fn file_digest(path: PathBuf) -> Result<Option<FileDigest>> {
    Ok(optional_hash(path.clone())?.map(|digest| FileDigest {
        path: path.display().to_string(),
        digest,
    }))
}

fn file_digest_relative(root: &Path, path: PathBuf) -> Result<Option<FileDigest>> {
    Ok(optional_hash(path.clone())?.map(|digest| FileDigest {
        path: relative_path(root, &path),
        digest,
    }))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned()
}

fn optional_hash(path: PathBuf) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(hex_digest(&read(path)?)))
}

fn worker_source_digest() -> Result<Option<String>> {
    let worker_root = repo_root().join("lean");
    if !worker_root.exists() {
        return Ok(None);
    }
    let mut files = WalkDir::new(&worker_root)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("lean"))
        .collect::<Vec<_>>();
    files.sort();
    let mut hasher = Sha256::new();
    for path in files {
        hasher.update(
            path.strip_prefix(&worker_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .as_bytes(),
        );
        hasher.update([0]);
        hasher.update(read(path)?);
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    Ok(Some(digest.iter().map(|byte| format!("{byte:02x}")).collect()))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under repo/crates/<component>")
        .to_path_buf()
}

fn record_worker_events(reporter: &mut Reporter, events: &[WorkerEvent]) {
    for event in events {
        reporter.event(
            format!("worker.{}", event.phase),
            event.current,
            event.total,
            event.message.clone(),
        );
    }
}

fn diagnostics_to_strings(diagnostics: Vec<WorkerDiagnostic>) -> Vec<String> {
    diagnostics
        .into_iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect()
}

fn create_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|source| Error::Io {
        message: "could not create directory",
        path: path.to_path_buf(),
        source,
    })
}

fn remove_file(path: &Path) -> Result<()> {
    std::fs::remove_file(path).map_err(|source| Error::Io {
        message: "could not remove file",
        path: path.to_path_buf(),
        source,
    })
}

fn replace_file(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to).map_err(|source| Error::Io {
        message: "could not replace file",
        path: to.to_path_buf(),
        source,
    })
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(|source| Error::Io {
        message: "could not write file",
        path: path.to_path_buf(),
        source,
    })
}

#[allow(dead_code)]
fn expand_home(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if text == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
fn probe_cache_key(pair: &ProbePair) -> String {
    hex_digest(
        serde_json::to_string(pair)
            .expect("probe pair serialization cannot fail")
            .as_bytes(),
    )
}

fn probe_cache_payload(result: &ProbeResult) -> String {
    serde_json::to_string(result).expect("probe result serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{
        CacheStatus, FingerprintKind, IndexBuildKind, IndexBuildRequest, IndexProvenanceKind, IndexReference,
        IndexStore, SemanticFeatureQuery, SemanticFingerprintFeature, SemanticRoleFeature, index_cache_key, safe_label,
        sqlite_cache_is_current,
    };
    use lean_dup_diagnostics::progress::Reporter;
    use lean_dup_project::workspace::{ResolvedWorkspace, WorkspaceRequest, resolve};
    use lean_dup_worker::{ProbePair, ProbeResult, WorkerClient, WorkerVersion};

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .to_path_buf()
    }

    use std::path::PathBuf;

    #[test]
    fn label_sanitization_keeps_cache_paths_under_label_directory() {
        assert_eq!(safe_label("mathlib"), "mathlib");
        assert_eq!(safe_label("../bad label"), ".._bad_label");
    }

    #[test]
    fn build_reuse_resolve_query_and_hydrate_fixture_index() {
        let cache = TempDir::new().unwrap();
        let workspace = resolve(
            WorkspaceRequest {
                requested_root: repo_root().join("tests/fixtures/external"),
                module_root: Some("External".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let store = IndexStore::new(cache.path().to_path_buf());
        let worker = WorkerClient::new();

        let request = super::IndexBuildRequest {
            workspace,
            execution_root: None,
            label: "fixture".to_owned(),
            module_root: "External".to_owned(),
            origin: "external:fixture".to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans: false,
            force: false,
            kind: super::IndexBuildKind::External,
        };

        let first = store
            .build_or_reuse(request.clone(), &worker, &mut Reporter::new(false, false))
            .unwrap();
        assert_eq!(first.cache_status, CacheStatus::Miss);
        assert_eq!(first.path.file_name().unwrap(), "index.sqlite");
        assert!(first.path.exists());
        assert!(!first.index_dir.join("declarations.jsonl.gz").exists());
        assert!(!first.index_dir.join("buckets.sqlite").exists());
        assert!(cache.path().join("indexes/fixture/latest.json").exists());

        let second = store
            .build_or_reuse(request, &worker, &mut Reporter::new(false, false))
            .unwrap();
        assert_eq!(second.cache_status, CacheStatus::Hit);
        assert_eq!(first.path, second.path);

        let by_label = store.resolve(IndexReference::Label("fixture".to_owned())).unwrap();
        let by_path = store.resolve(IndexReference::Path(first.index_dir.clone())).unwrap();

        let handles = by_label
            .matching_handles(SemanticFeatureQuery {
                fingerprints: vec![SemanticFingerprintFeature {
                    kind: FingerprintKind::Statement,
                    key: "missing-key".to_owned(),
                }],
                role_features: vec![],
            })
            .unwrap();
        assert!(handles.is_empty());

        let all = by_path
            .matching_handles(SemanticFeatureQuery {
                fingerprints: vec![],
                role_features: vec![SemanticRoleFeature {
                    role: "const".to_owned(),
                    key: "Prop".to_owned(),
                }],
            })
            .unwrap();
        let sample = all.into_iter().take(2).collect::<Vec<_>>();
        let hydrated = by_path.hydrate(&sample).unwrap();
        assert_eq!(hydrated.len(), sample.len());
        assert!(hydrated.iter().all(|row| row.origin == "external:fixture"));
        let facts = by_path.facts().unwrap();
        assert_eq!(facts.provenance.kind, IndexProvenanceKind::SourceBacked);
        assert_eq!(
            facts.provenance.source_root,
            Some(facts.provenance.execution_root.clone().unwrap())
        );
        assert_eq!(facts.provenance.module_root, "External");
        assert!(facts.provenance.worker_version.is_some());

        let pair = ProbePair {
            pair_id: "p1".to_owned(),
            left_declaration_id: "left".to_owned(),
            right_declaration_id: "right".to_owned(),
        };
        let result = ProbeResult {
            pair_id: "p1".to_owned(),
            left_declaration_id: "left".to_owned(),
            right_declaration_id: "right".to_owned(),
            status: "ok".to_owned(),
            same_statement: true,
            same_up_to_safe_reordering: false,
            connective_equivalent: false,
            specializes_left_to_right: false,
            specializes_right_to_left: false,
            mutual_implication_shape: false,
            same_reducible_definition: false,
            message: None,
        };
        by_path
            .cache_probe_results(&[super::ProbeCacheEntry {
                cache_key: super::probe_cache_key(&pair),
                pair: pair.clone(),
                result: result.clone(),
            }])
            .unwrap();
        assert_eq!(
            by_path.cached_probe_result(&super::probe_cache_key(&pair)).unwrap(),
            Some(result)
        );
    }

    #[test]
    fn resolve_rejects_old_latest_pointer_shape() {
        let cache = TempDir::new().unwrap();
        fs::create_dir_all(cache.path().join("indexes/fixture")).unwrap();
        fs::write(
            cache.path().join("indexes/fixture/latest.json"),
            r#"{"metadata":"/tmp/old.metadata.json"}"#,
        )
        .unwrap();

        let store = IndexStore::new(cache.path().to_path_buf());
        assert!(store.resolve(IndexReference::Label("fixture".to_owned())).is_err());
    }

    #[test]
    fn cache_key_changes_with_semantic_versions_module_roots_and_source_hashes() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("lakefile.toml"),
            r#"
[[lean_lib]]
name = "A"
[[lean_lib]]
name = "B"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("lean-toolchain"), "leanprover/lean4:v4.30.0-rc2\n").unwrap();
        fs::write(temp.path().join("A.lean"), "#check Nat\n").unwrap();
        fs::write(temp.path().join("B.lean"), "#check Bool\n").unwrap();

        let workspace_a = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: Some("A".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let request_a = request_for(workspace_a, "A");
        let version = fake_version("features.v1");
        let first = serde_json::to_string(&index_cache_key(&request_a, &version).unwrap()).unwrap();

        let changed_version =
            serde_json::to_string(&index_cache_key(&request_a, &fake_version("features.v2")).unwrap()).unwrap();
        assert_ne!(first, changed_version);

        let workspace_b = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: Some("B".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let changed_root =
            serde_json::to_string(&index_cache_key(&request_for(workspace_b, "B"), &version).unwrap()).unwrap();
        assert_ne!(first, changed_root);

        fs::write(temp.path().join("A.lean"), "#check String\n").unwrap();
        let workspace_a = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: Some("A".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let changed_source =
            serde_json::to_string(&index_cache_key(&request_for(workspace_a, "A"), &version).unwrap()).unwrap();
        assert_ne!(first, changed_source);
    }

    #[test]
    fn cache_key_ignores_unrelated_files_and_tracks_lake_inputs() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("lakefile.toml"), "[[lean_lib]]\nname = \"A\"\n").unwrap();
        fs::write(temp.path().join("lean-toolchain"), "leanprover/lean4:v4.30.0-rc2\n").unwrap();
        fs::write(temp.path().join("lake-manifest.json"), "{\"packages\":[]}\n").unwrap();
        fs::write(temp.path().join("A.lean"), "#check Nat\n").unwrap();

        let workspace = resolve(
            WorkspaceRequest {
                requested_root: temp.path().to_path_buf(),
                module_root: Some("A".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let version = fake_version("features.v1");
        let first =
            serde_json::to_string(&index_cache_key(&request_for(workspace.clone(), "A"), &version).unwrap()).unwrap();

        fs::write(temp.path().join("README.md"), "not part of the Lean cache key\n").unwrap();
        let after_unrelated =
            serde_json::to_string(&index_cache_key(&request_for(workspace.clone(), "A"), &version).unwrap()).unwrap();
        assert_eq!(first, after_unrelated);

        fs::write(temp.path().join("lean-toolchain"), "leanprover/lean4:v4.31.0\n").unwrap();
        let changed_toolchain =
            serde_json::to_string(&index_cache_key(&request_for(workspace.clone(), "A"), &version).unwrap()).unwrap();
        assert_ne!(first, changed_toolchain);

        fs::write(temp.path().join("lean-toolchain"), "leanprover/lean4:v4.30.0-rc2\n").unwrap();
        fs::write(temp.path().join("lake-manifest.json"), "{\"packages\":[\"mathlib\"]}\n").unwrap();
        let changed_manifest =
            serde_json::to_string(&index_cache_key(&request_for(workspace.clone(), "A"), &version).unwrap()).unwrap();
        assert_ne!(first, changed_manifest);

        fs::write(temp.path().join("lake-manifest.json"), "{\"packages\":[]}\n").unwrap();
        fs::write(
            temp.path().join("lakefile.toml"),
            "[[lean_lib]]\nname = \"A\"\nmoreLinkArgs = []\n",
        )
        .unwrap();
        let changed_lakefile =
            serde_json::to_string(&index_cache_key(&request_for(workspace, "A"), &version).unwrap()).unwrap();
        assert_ne!(first, changed_lakefile);
    }

    #[test]
    fn project_mathlib_cache_key_ignores_absolute_project_paths() {
        let left = TempDir::new().unwrap();
        let right = TempDir::new().unwrap();
        let left_mathlib = left.path().join(".lake/packages/mathlib");
        let right_mathlib = right.path().join(".lake/packages/mathlib");
        for root in [left.path(), right.path()] {
            fs::write(root.join("lakefile.toml"), "[[lean_lib]]\nname = \"Project\"\n").unwrap();
            fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.30.0-rc2\n").unwrap();
            fs::write(root.join("Project.lean"), "#check Nat\n").unwrap();
            let mathlib = root.join(".lake/packages/mathlib");
            fs::create_dir_all(mathlib.join("Mathlib")).unwrap();
            fs::write(mathlib.join("lakefile.toml"), "[[lean_lib]]\nname = \"Mathlib\"\n").unwrap();
            fs::write(mathlib.join("Mathlib.lean"), "#check Nat\n").unwrap();
        }

        let version = fake_version("features.v1");
        let left_workspace = resolve(
            WorkspaceRequest {
                requested_root: left_mathlib,
                module_root: Some("Mathlib".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let right_workspace = resolve(
            WorkspaceRequest {
                requested_root: right_mathlib,
                module_root: Some("Mathlib".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let left_key = serde_json::to_string(
            &index_cache_key(
                &project_mathlib_request(left_workspace, left.path().to_path_buf()),
                &version,
            )
            .unwrap(),
        )
        .unwrap();
        let right_key = serde_json::to_string(
            &index_cache_key(
                &project_mathlib_request(right_workspace, right.path().to_path_buf()),
                &version,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(left_key, right_key);

        fs::write(
            right.path().join(".lake/packages/mathlib/Mathlib.lean"),
            "#check Bool\n",
        )
        .unwrap();
        let right_workspace = resolve(
            WorkspaceRequest {
                requested_root: right.path().join(".lake/packages/mathlib"),
                module_root: Some("Mathlib".to_owned()),
            },
            &mut Reporter::new(false, false),
        )
        .unwrap();
        let changed_key = serde_json::to_string(
            &index_cache_key(
                &project_mathlib_request(right_workspace, right.path().to_path_buf()),
                &version,
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(left_key, changed_key);
    }

    #[test]
    fn sqlite_cache_validation_rejects_schema_mismatch() {
        let temp = TempDir::new().unwrap();
        let index_path = temp.path().join("index.sqlite");
        let connection = rusqlite::Connection::open(&index_path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO metadata VALUES ('schema_version', 'old');
                INSERT INTO metadata VALUES ('cache_key', 'same');
                "#,
            )
            .unwrap();
        drop(connection);

        assert!(!sqlite_cache_is_current(&index_path, "same").unwrap());
    }

    fn request_for(workspace: ResolvedWorkspace, module_root: &str) -> IndexBuildRequest {
        IndexBuildRequest {
            workspace,
            execution_root: None,
            label: "fixture".to_owned(),
            module_root: module_root.to_owned(),
            origin: "external:fixture".to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans: false,
            force: false,
            kind: IndexBuildKind::External,
        }
    }

    fn project_mathlib_request(workspace: ResolvedWorkspace, execution_root: PathBuf) -> IndexBuildRequest {
        IndexBuildRequest {
            workspace,
            execution_root: Some(execution_root),
            label: "mathlib".to_owned(),
            module_root: "Mathlib".to_owned(),
            origin: "mathlib".to_owned(),
            include_private: true,
            include_generated: false,
            require_oleans: true,
            force: false,
            kind: IndexBuildKind::ProjectMathlib,
        }
    }

    fn fake_version(features_version: &str) -> WorkerVersion {
        WorkerVersion {
            protocol_version: "lean-dup.worker.v1".to_owned(),
            worker_version: "worker.v1".to_owned(),
            lean_version: Some("lean".to_owned()),
            extract_version: "extract.v1".to_owned(),
            features_version: features_version.to_owned(),
            probe_version: "probe.v1".to_owned(),
            supported_commands: vec![],
            supported_capabilities: vec![],
        }
    }
}
