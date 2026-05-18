use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::index::{IndexProvenanceKind, OpenedIndex, OpenedIndexFacts};
use lean_dup_project::ResolvedWorkspace;
use lean_dup_worker::ModuleDescriptor;

/// Evidence mode available for one comparison origin in the current audit.
///
/// The mode describes caller-visible proof status. It does not expose how
/// provenance was stored, which source-root checks were used, or how worker
/// module descriptors are constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComparisonEvidenceMode {
    Static,
    SourceBackedNotImportable,
    ProofGrade,
}

/// JSON-safe provenance facts for audit/profile output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonProvenanceReport {
    pub label: Option<String>,
    pub origin: String,
    pub evidence_mode: ComparisonEvidenceMode,
    pub declaration_count: usize,
    pub index_path: PathBuf,
    pub source_root: Option<PathBuf>,
    pub execution_root: Option<PathBuf>,
    pub execution_policy: String,
    pub reason: String,
}

/// Current-audit policy for comparison evidence and semantic probes.
///
/// Ranking and semantic verification ask this policy about origins. They do
/// not inspect labels, SQLite metadata, source-root layouts, or worker import
/// rules.
#[derive(Debug, Clone, Default)]
pub struct ComparisonEvidencePolicy {
    origins: BTreeMap<String, OriginPolicy>,
}

#[derive(Debug, Clone)]
struct OriginPolicy {
    mode: ComparisonEvidenceMode,
    source_root: Option<PathBuf>,
}

impl ComparisonEvidencePolicy {
    pub fn for_origin(origin: impl Into<String>, mode: ComparisonEvidenceMode) -> Self {
        let mut policy = Self::default();
        policy.origins.insert(
            origin.into(),
            OriginPolicy {
                mode,
                source_root: None,
            },
        );
        policy
    }

    pub fn evidence_mode(&self, origin: &str) -> ComparisonEvidenceMode {
        self.origins
            .get(origin)
            .map(|policy| policy.mode)
            .unwrap_or(ComparisonEvidenceMode::Static)
    }

    pub fn requires_semantic_evidence(&self, origin: &str) -> bool {
        self.evidence_mode(origin) == ComparisonEvidenceMode::ProofGrade
    }

    pub fn probe_module(&self, origin: &str, module: &str) -> Option<ModuleDescriptor> {
        let policy = self.origins.get(origin)?;
        (policy.mode == ComparisonEvidenceMode::ProofGrade).then(|| ModuleDescriptor {
            module: module.to_owned(),
            origin: origin.to_owned(),
            source_root: policy.source_root.clone(),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ComparisonProvenance {
    pub reports: Vec<ComparisonProvenanceReport>,
    #[serde(skip)]
    pub policy: ComparisonEvidencePolicy,
}

/// Resolve comparison-index provenance for the current audit workspace.
pub fn resolve(indexes: &[OpenedIndex], audit_workspace: &ResolvedWorkspace) -> crate::Result<ComparisonProvenance> {
    let mut policy = ComparisonEvidencePolicy::default();
    let mut reports = Vec::new();
    for index in indexes {
        let facts = index.facts()?;
        let resolved = resolve_facts(facts, audit_workspace);
        policy.origins.insert(
            resolved.origin.clone(),
            OriginPolicy {
                mode: resolved.evidence_mode,
                source_root: resolved.source_root.clone(),
            },
        );
        reports.push(resolved);
    }
    Ok(ComparisonProvenance { reports, policy })
}

fn resolve_facts(facts: OpenedIndexFacts, audit_workspace: &ResolvedWorkspace) -> ComparisonProvenanceReport {
    let source_root = facts.provenance.source_root.clone();
    let execution_root = facts.provenance.execution_root.clone();
    let execution_policy = facts.provenance.execution_policy.clone();
    let (evidence_mode, reason) = match facts.provenance.kind {
        IndexProvenanceKind::Static => (
            ComparisonEvidenceMode::Static,
            "index has no source provenance; using static evidence".to_owned(),
        ),
        IndexProvenanceKind::SourceBacked => {
            if execution_root
                .as_ref()
                .is_some_and(|root| same_path(root, &audit_workspace.root))
            {
                (
                    ComparisonEvidenceMode::ProofGrade,
                    "source-backed index is importable from this audit Lake environment".to_owned(),
                )
            } else {
                (
                    ComparisonEvidenceMode::SourceBackedNotImportable,
                    "source-backed index was built in a different Lake environment; using static evidence".to_owned(),
                )
            }
        }
    };
    ComparisonProvenanceReport {
        label: facts.label,
        origin: facts.origin,
        evidence_mode,
        declaration_count: facts.declaration_count,
        index_path: facts.path,
        source_root,
        execution_root,
        execution_policy,
        reason,
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    canonical(left) == canonical(right)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{ComparisonEvidenceMode, resolve_facts};
    use crate::index::{IndexProvenance, IndexProvenanceKind, OpenedIndexFacts};
    use lean_dup_project::ResolvedWorkspace;

    #[test]
    fn missing_provenance_defaults_to_static_policy() {
        let workspace = workspace(TempDir::new().unwrap().path().to_path_buf());
        let facts = facts(
            IndexProvenance::static_index("External"),
            workspace.root.join("index.sqlite"),
        );

        let report = resolve_facts(facts, &workspace);

        assert_eq!(report.evidence_mode, ComparisonEvidenceMode::Static);
    }

    #[test]
    fn same_execution_root_is_probe_importable() {
        let root = TempDir::new().unwrap();
        let workspace = workspace(root.path().to_path_buf());
        let facts = facts(
            source_backed(root.path().to_path_buf()),
            root.path().join("index.sqlite"),
        );

        let report = resolve_facts(facts, &workspace);

        assert_eq!(report.evidence_mode, ComparisonEvidenceMode::ProofGrade);
    }

    #[test]
    fn different_execution_root_is_not_importable() {
        let audit = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let workspace = workspace(audit.path().to_path_buf());
        let facts = facts(
            source_backed(external.path().to_path_buf()),
            external.path().join("index.sqlite"),
        );

        let report = resolve_facts(facts, &workspace);

        assert_eq!(report.evidence_mode, ComparisonEvidenceMode::SourceBackedNotImportable);
    }

    fn workspace(root: std::path::PathBuf) -> ResolvedWorkspace {
        ResolvedWorkspace {
            requested_root: root.clone(),
            root: root.clone(),
            lakefile: root.join("lakefile.toml"),
            module_roots: vec!["Tiny".to_owned()],
            selected_roots: vec!["Tiny".to_owned()],
            source_files: Vec::new(),
        }
    }

    fn facts(provenance: IndexProvenance, path: std::path::PathBuf) -> OpenedIndexFacts {
        OpenedIndexFacts {
            origin: "external:fixture".to_owned(),
            label: Some("fixture".to_owned()),
            declaration_count: 1,
            path,
            provenance,
        }
    }

    fn source_backed(root: std::path::PathBuf) -> IndexProvenance {
        IndexProvenance {
            version: "lean-dup.index.provenance.v1".to_owned(),
            kind: IndexProvenanceKind::SourceBacked,
            source_root: Some(root.clone()),
            execution_root: Some(root),
            execution_policy: "indexed-workspace-lake-environment".to_owned(),
            module_root: "External".to_owned(),
            protocol_version: Some("protocol".to_owned()),
            worker_version: Some("worker".to_owned()),
            extract_version: Some("extract".to_owned()),
            features_version: Some("features".to_owned()),
            probe_version: Some("probe".to_owned()),
        }
    }
}
