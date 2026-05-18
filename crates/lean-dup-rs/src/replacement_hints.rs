use serde::Serialize;

use crate::ranking::{ConfidenceTier, RankedReview, ReviewAction};
use crate::source_refs::{ImportStatus, SourceFacts, SourceReference};

const DEFAULT_MAX_DISPLAYED_CALLERS: usize = 12;
const DEFAULT_TRANSITIONAL_ALIAS_CALLERS: usize = 8;

/// Replacement guidance attached to ranked groups.
///
/// Hints expose target/import/caller impact for safe review. This module owns
/// hint eligibility and transitional-alias notes; it does not decide semantic
/// equivalence, run probes, scan source, or render text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ReplacementHint {
    pub(crate) target_decl: String,
    pub(crate) target_module: String,
    pub(crate) import_status: ImportStatus,
    pub(crate) caller_count: usize,
    pub(crate) displayed_callers: Vec<SourceReference>,
    pub(crate) notes: Vec<String>,
    pub(crate) blockers: Vec<String>,
}

/// Profile defaults for replacement hint display and safety notes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplacementHintProfile {
    pub(crate) max_displayed_callers: usize,
    pub(crate) transitional_alias_callers: usize,
}

impl Default for ReplacementHintProfile {
    fn default() -> Self {
        Self {
            max_displayed_callers: DEFAULT_MAX_DISPLAYED_CALLERS,
            transitional_alias_callers: DEFAULT_TRANSITIONAL_ALIAS_CALLERS,
        }
    }
}

/// Attach replacement hints to eligible ranked groups.
pub(crate) fn attach_replacement_hints(
    mut review: RankedReview,
    facts: &SourceFacts,
    profile: ReplacementHintProfile,
) -> RankedReview {
    for group in &mut review.groups {
        group.replacement_hint = hint_for_group(group, facts, profile);
    }
    review
}

fn hint_for_group(
    group: &crate::ranking::RankedGroup,
    facts: &SourceFacts,
    profile: ReplacementHintProfile,
) -> Option<ReplacementHint> {
    if !eligible(group) {
        return None;
    }
    let target_decl = group.target_decl.clone()?;
    let target_module = group.target_module.clone()?;
    let local_members = group
        .members
        .iter()
        .filter(|member| member.origin == "workspace")
        .collect::<Vec<_>>();
    if local_members.is_empty() {
        return None;
    }

    let import_status = aggregate_import_status(
        local_members
            .iter()
            .map(|member| facts.import_status_for(&member.declaration_id, &target_module)),
    );
    let mut callers = local_members
        .iter()
        .filter_map(|member| facts.declaration(&member.declaration_id))
        .flat_map(|fact| fact.references.iter().cloned())
        .collect::<Vec<_>>();
    callers.sort();
    callers.dedup();

    let mut notes = Vec::new();
    let mut blockers = Vec::new();
    if import_status == ImportStatus::Missing {
        blockers.push("missing-import".to_owned());
        notes.push(format!("add `import {target_module}` before replacing local uses"));
    }
    if callers.len() >= profile.transitional_alias_callers {
        notes.push("many local callers; keep a transitional alias during cleanup".to_owned());
    }
    if group.recommended_action == ReviewAction::LocalAlias {
        notes.push("alias-first cleanup is safer than deleting the local declaration".to_owned());
    }

    Some(ReplacementHint {
        target_decl,
        target_module,
        import_status,
        caller_count: callers.len(),
        displayed_callers: callers.into_iter().take(profile.max_displayed_callers).collect(),
        notes,
        blockers,
    })
}

fn eligible(group: &crate::ranking::RankedGroup) -> bool {
    if matches!(group.confidence, ConfidenceTier::Low | ConfidenceTier::Noise) {
        return false;
    }
    if group.blockers.iter().any(|blocker| {
        matches!(
            blocker.as_str(),
            "generated-declaration" | "broad-head-only" | "weak-feature-overlap"
        )
    }) {
        return false;
    }
    matches!(
        group.recommended_action,
        ReviewAction::AlreadyInMathlib | ReviewAction::ReplaceLocalUses | ReviewAction::LocalAlias
    )
}

fn aggregate_import_status(statuses: impl Iterator<Item = ImportStatus>) -> ImportStatus {
    let statuses = statuses.collect::<Vec<_>>();
    if statuses.is_empty() {
        ImportStatus::Unknown
    } else if statuses.iter().all(|status| *status == ImportStatus::Direct) {
        ImportStatus::Direct
    } else if statuses.contains(&ImportStatus::Missing) {
        ImportStatus::Missing
    } else {
        ImportStatus::Unknown
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{ReplacementHintProfile, attach_replacement_hints};
    use crate::index::{DeclarationHandle, HydratedDeclaration};
    use crate::ranking::{
        ConfidenceTier, RankedGroup, RankedReview, RankingDiagnostics, ReviewAction, ReviewEvidenceMode, ReviewMember,
        ReviewPriority, ReviewRelation,
    };
    use crate::source_refs::{ImportStatus, SourceFactInput, collect_source_facts};
    use crate::worker::{Fingerprints, SourcePoint, SourceSpan};

    #[test]
    fn hints_show_import_and_caller_impact_without_deletion_guidance() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("Tiny.lean");
        std::fs::write(
            &path,
            r#"
namespace Tiny
theorem local : True := by trivial
theorem caller : True := local
end Tiny
"#,
        )
        .unwrap();
        let declaration = declaration("workspace:Tiny:Tiny.local", "Tiny.local", &path);
        let facts = collect_source_facts(SourceFactInput::new(std::slice::from_ref(&declaration)));
        let review = RankedReview {
            groups: vec![RankedGroup {
                id: "review-1".to_owned(),
                pair_id: "p1".to_owned(),
                relation: ReviewRelation::ExactStatement,
                members: vec![ReviewMember {
                    declaration_id: declaration.declaration_id.clone(),
                    origin: "workspace".to_owned(),
                    module: "Tiny".to_owned(),
                    qualified_name: "Tiny.local".to_owned(),
                    display_name: "local".to_owned(),
                    kind: "theorem".to_owned(),
                    visibility: "public".to_owned(),
                    source_span: declaration.source_span.clone(),
                    status_flags: Vec::new(),
                }],
                evidence: Vec::new(),
                signals: vec!["statement-fingerprint".to_owned()],
                blockers: Vec::new(),
                confidence: ConfidenceTier::High,
                review_priority: ReviewPriority::High,
                recommended_action: ReviewAction::LocalAlias,
                target_decl: Some("Mathlib.local".to_owned()),
                target_module: Some("Mathlib".to_owned()),
                evidence_mode: ReviewEvidenceMode::Static,
                probe_summary: None,
                local_caller_count: 1,
                replacement_hint: None,
            }],
            suppressed: Vec::new(),
            diagnostics: RankingDiagnostics::default(),
        };

        let review = attach_replacement_hints(review, &facts, ReplacementHintProfile::default());
        let hint = review.groups[0].replacement_hint.as_ref().unwrap();

        assert_eq!(hint.import_status, ImportStatus::Missing);
        assert_eq!(hint.caller_count, 1);
        assert_eq!(hint.displayed_callers.len(), 1);
        assert!(hint.notes.iter().all(|note| !note.contains("delete")));
    }

    fn declaration(id: &str, name: &str, path: &std::path::Path) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::for_test(id),
            declaration_id: id.to_owned(),
            origin: "workspace".to_owned(),
            module: "Tiny".to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap().to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: Some(SourceSpan {
                file: path.display().to_string(),
                start: SourcePoint { line: 4, column: 1 },
                end: SourcePoint { line: 4, column: 35 },
            }),
            statement_text: "theorem local : True".to_owned(),
            status_flags: Vec::new(),
            feature_version: "features.roles.v1".to_owned(),
            fingerprints: Fingerprints {
                statement: "statement".to_owned(),
                safe_binder_permutation: "permutation".to_owned(),
                connective_shape: "connective".to_owned(),
                conclusion_shape: "conclusion".to_owned(),
            },
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: Vec::new(),
        }
    }
}
