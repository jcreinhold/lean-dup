use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ranking::{RankedReview, ReviewAction, ReviewPriority, ReviewRelation};
use crate::{Error, Result};
use lean_dup_diagnostics::read_to_string;

const BASELINE_SCHEMA_VERSION: &str = "lean-dup.baseline.v1";

/// Baseline snapshot for comparing audit runs across cleanup work.
///
/// A snapshot records stable group identities and evidence summaries. It is not
/// a rendered report, and callers do not need to know where or how it is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    pub schema_version: String,
    pub workspace_fingerprint: String,
    pub groups: Vec<BaselineGroup>,
}

/// One comparable group in a baseline snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineGroup {
    pub id: String,
    pub relation: String,
    pub review_priority: String,
    pub recommended_action: String,
    pub member_ids: Vec<String>,
    pub evidence_summary: Vec<String>,
    pub evidence_digest: String,
}

/// Baseline diff result for workflow output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineDiff {
    pub baseline: String,
    pub baseline_path: PathBuf,
    pub appeared: Vec<BaselineGroup>,
    pub disappeared: Vec<BaselineGroup>,
    pub changed: Vec<BaselineChange>,
}

/// One group whose stable identity survived but evidence changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineChange {
    pub id: String,
    pub before: BaselineGroup,
    pub after: BaselineGroup,
}

/// Build the storage-independent snapshot used by save-baseline and diff.
pub fn snapshot(review: &RankedReview, workspace_fingerprint: String) -> BaselineSnapshot {
    BaselineSnapshot {
        schema_version: BASELINE_SCHEMA_VERSION.to_owned(),
        workspace_fingerprint,
        groups: review.groups.iter().map(BaselineGroup::from_ranked).collect(),
    }
}

/// Save a named baseline under the cache root.
pub fn save(cache_root: &Path, name: &str, snapshot: &BaselineSnapshot) -> Result<PathBuf> {
    let path = baseline_path(cache_root, name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            message: "could not create baseline directory",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let body = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(&path, body).map_err(|source| Error::Io {
        message: "could not write baseline",
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Load a named baseline from the cache root.
pub fn load(cache_root: &Path, name: &str) -> Result<(PathBuf, BaselineSnapshot)> {
    let path = baseline_path(cache_root, name)?;
    let snapshot = serde_json::from_str(&read_to_string(path.clone())?)?;
    Ok((path, snapshot))
}

/// Compare current audit evidence against a named baseline.
pub fn diff(
    name: String,
    baseline_path: PathBuf,
    baseline: BaselineSnapshot,
    current: BaselineSnapshot,
) -> BaselineDiff {
    let before = baseline
        .groups
        .into_iter()
        .map(|group| (group.id.clone(), group))
        .collect::<BTreeMap<_, _>>();
    let after = current
        .groups
        .into_iter()
        .map(|group| (group.id.clone(), group))
        .collect::<BTreeMap<_, _>>();
    let before_ids = before.keys().cloned().collect::<BTreeSet<_>>();
    let after_ids = after.keys().cloned().collect::<BTreeSet<_>>();

    let appeared = after_ids
        .difference(&before_ids)
        .filter_map(|id| after.get(id).cloned())
        .collect();
    let disappeared = before_ids
        .difference(&after_ids)
        .filter_map(|id| before.get(id).cloned())
        .collect();
    let changed = before_ids
        .intersection(&after_ids)
        .filter_map(|id| {
            let before_group = before.get(id)?;
            let after_group = after.get(id)?;
            (before_group.evidence_digest != after_group.evidence_digest).then(|| BaselineChange {
                id: id.clone(),
                before: before_group.clone(),
                after: after_group.clone(),
            })
        })
        .collect();

    BaselineDiff {
        baseline: name,
        baseline_path,
        appeared,
        disappeared,
        changed,
    }
}

impl BaselineGroup {
    fn from_ranked(group: &crate::ranking::RankedGroup) -> Self {
        let mut evidence_summary = Vec::new();
        evidence_summary.push(format!("relation={}", relation_name(group.relation)));
        evidence_summary.push(format!("priority={}", priority_name(group.review_priority)));
        evidence_summary.push(format!("action={}", action_name(group.recommended_action)));
        evidence_summary.extend(group.signals.iter().map(|signal| format!("signal={signal}")));
        evidence_summary.extend(group.blockers.iter().map(|blocker| format!("blocker={blocker}")));
        evidence_summary.extend(group.evidence.iter().map(|evidence| evidence.summary()));
        if let Some(target) = &group.target_decl {
            evidence_summary.push(format!("target={target}"));
        }
        if let Some(hint) = &group.replacement_hint {
            evidence_summary.push(format!("import={:?}", hint.import_status).to_ascii_lowercase());
            evidence_summary.push(format!("callers={}", hint.caller_count));
        }
        evidence_summary.sort();
        evidence_summary.dedup();
        let evidence_digest = digest(&evidence_summary);

        Self {
            id: group.id.clone(),
            relation: relation_name(group.relation).to_owned(),
            review_priority: priority_name(group.review_priority).to_owned(),
            recommended_action: action_name(group.recommended_action).to_owned(),
            member_ids: group
                .members
                .iter()
                .map(|member| member.declaration_id.clone())
                .collect(),
            evidence_summary,
            evidence_digest,
        }
    }
}

fn baseline_path(cache_root: &Path, name: &str) -> Result<PathBuf> {
    let safe = safe_name(name)?;
    Ok(cache_root.join("baselines").join(format!("{safe}.json")))
}

fn safe_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(Error::Search {
            message: format!("invalid baseline name: {name}"),
        });
    }
    Ok(trimmed.to_owned())
}

fn digest(parts: &[String]) -> String {
    let encoded = serde_json::to_vec(parts).expect("string lists serialize");
    let digest = Sha256::digest(&encoded);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn relation_name(relation: ReviewRelation) -> &'static str {
    match relation {
        ReviewRelation::ExactStatement => "exact-statement",
        ReviewRelation::PermutedStatement => "permuted-statement",
        ReviewRelation::ConnectiveEquivalent => "connective-equivalent",
        ReviewRelation::Specialization => "specialization",
        ReviewRelation::SourceClone => "source-clone",
        ReviewRelation::SubsumptionCandidate => "subsumption-candidate",
        ReviewRelation::NearStatement => "near-statement",
    }
}

fn priority_name(priority: ReviewPriority) -> &'static str {
    match priority {
        ReviewPriority::High => "high",
        ReviewPriority::Medium => "medium",
        ReviewPriority::Low => "low",
        ReviewPriority::Noise => "noise",
    }
}

fn action_name(action: ReviewAction) -> &'static str {
    match action {
        ReviewAction::AlreadyInMathlib => "already-in-mathlib",
        ReviewAction::LocalAlias => "local-alias",
        ReviewAction::ReplaceLocalUses => "replace-local-uses",
        ReviewAction::InlinePrivateHelper => "inline-private-helper",
        ReviewAction::MergeGeneralization => "merge-generalization",
        ReviewAction::SpecializationOf => "specialization-of",
        ReviewAction::ProbableSourceClone => "probable-source-clone",
        ReviewAction::ManualReview => "manual-review",
    }
}
