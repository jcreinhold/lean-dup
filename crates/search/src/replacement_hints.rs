use serde::Serialize;

use lean_dup_worker::SourceSpan;

use crate::ranking::{
    ConfidenceTier, RankedGroup, RankedReview, ReviewAction, ReviewEvidenceMode, ReviewFilter, ReviewMember,
};
use crate::scorer;
use crate::source_refs::{ImportStatus, SourceFacts, SourceReference, SourceReferenceStatus};

/// Replacement guidance attached to ranked groups.
///
/// Hints expose target/import/caller impact for safe review. This module owns
/// hint eligibility and transitional-alias notes; it does not decide semantic
/// equivalence, run probes, scan source, or render text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplacementHint {
    pub target_decl: String,
    pub target_module: String,
    pub import_status: ImportStatus,
    pub caller_impact: CallerImpact,
    pub caller_count: usize,
    pub displayed_callers: Vec<SourceReference>,
    pub callers_truncated: bool,
    pub notes: Vec<String>,
    pub blockers: Vec<String>,
}

/// Stable caller-impact category for replacement guidance.
///
/// This describes what search can safely say from bounded source-use facts.
/// It is not proof of semantic replaceability, and it does not expose token
/// scanning, parser heuristics, or local filesystem layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallerImpact {
    NoCallers,
    WrapperOnly,
    BoundedCallers,
    TruncatedCallers,
    UnknownCallers,
    MissingSource,
}

/// Profile defaults for replacement hint display and safety notes.
#[derive(Debug, Clone, Copy)]
pub struct ReplacementHintProfile {
    pub max_displayed_callers: usize,
    pub transitional_alias_callers: usize,
}

impl Default for ReplacementHintProfile {
    fn default() -> Self {
        let thresholds = scorer::thresholds();
        Self {
            max_displayed_callers: thresholds.max_displayed_callers,
            transitional_alias_callers: thresholds.transitional_alias_callers,
        }
    }
}

/// Attach replacement hints to eligible ranked groups.
pub fn attach_replacement_hints(
    mut review: RankedReview,
    facts: &SourceFacts,
    profile: ReplacementHintProfile,
) -> RankedReview {
    for group in &mut review.groups {
        if private_helper_wrapper_cleanup(group, facts) {
            group.recommended_action = ReviewAction::InlinePrivateHelper;
        }
        group.replacement_hint = hint_for_group(group, facts, profile);
    }
    review
}

/// Return workspace declarations that need caller scans for visible hints.
///
/// This keeps source-reference collection aligned with the replacement-hint
/// boundary: audit does not need to know hint eligibility rules or caller
/// display policy.
pub fn reference_declarations_for_hints(review: &RankedReview, filter: ReviewFilter) -> Vec<String> {
    let mut declarations = review
        .visible_groups(filter)
        .into_iter()
        .filter(|group| eligible(group) && group.target_decl.is_some() && group.target_module.is_some())
        .flat_map(|group| group.members.iter())
        .filter(|member| member.origin == "workspace")
        .map(|member| member.declaration_id.clone())
        .collect::<Vec<_>>();
    declarations.sort();
    declarations.dedup();
    declarations
}

fn hint_for_group(
    group: &RankedGroup,
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

    let import_status = if group.evidence_mode == ReviewEvidenceMode::SourceBackedNotImportable {
        ImportStatus::SourceBackedNotImportable
    } else {
        aggregate_import_status(
            local_members
                .iter()
                .map(|member| facts.import_status_for(&member.declaration_id, &target_module)),
        )
    };
    let caller_status = aggregate_reference_status(local_members.iter().map(|member| {
        facts
            .declaration(&member.declaration_id)
            .map(|fact| fact.reference_status)
    }));
    let mut callers = local_members
        .iter()
        .filter_map(|member| facts.declaration(&member.declaration_id))
        .flat_map(|fact| fact.references.iter().cloned())
        .collect::<Vec<_>>();
    callers.sort();
    callers.dedup();

    let mut notes = Vec::new();
    let mut blockers = Vec::new();
    let caller_impact = caller_impact(group, &local_members, facts, caller_status, &callers);
    if import_status == ImportStatus::Missing {
        blockers.push("missing-import".to_owned());
        notes.push(format!("add `import {target_module}` before replacing local uses"));
    }
    if import_status == ImportStatus::SourceBackedNotImportable {
        blockers.push("source-backed-not-importable".to_owned());
        notes.push(
            "source-backed evidence is not importable in this workspace; review before replacing uses".to_owned(),
        );
    }
    match caller_impact {
        CallerImpact::NoCallers => notes.push("no local callers were found in the bounded source scan".to_owned()),
        CallerImpact::WrapperOnly => {
            notes.push("caller impact is wrapper-only; do not replace uses outside the wrapper".to_owned())
        }
        CallerImpact::TruncatedCallers => {
            blockers.push("caller-scan-truncated".to_owned());
            notes.push("caller scan reached its bound; inspect remaining uses before replacing".to_owned());
        }
        CallerImpact::UnknownCallers => {
            blockers.push("caller-impact-unknown".to_owned());
            notes.push("caller impact is unknown because source-use facts are incomplete".to_owned());
        }
        CallerImpact::MissingSource => {
            blockers.push("missing-source".to_owned());
            notes.push("caller impact is unknown because source location is missing".to_owned());
        }
        CallerImpact::BoundedCallers => {}
    }
    if callers.len() >= profile.transitional_alias_callers {
        notes.push("many local callers; keep a transitional alias during cleanup".to_owned());
    }
    if group.recommended_action == ReviewAction::LocalAlias {
        notes.push("alias-first cleanup is safer than deleting the local declaration".to_owned());
    }
    if group.recommended_action == ReviewAction::InlinePrivateHelper {
        notes.push(
            "only the public wrapper calls the private helper; inline the helper body into the wrapper".to_owned(),
        );
    }

    Some(ReplacementHint {
        target_decl,
        target_module,
        import_status,
        caller_impact,
        caller_count: callers.len(),
        displayed_callers: callers.iter().take(profile.max_displayed_callers).cloned().collect(),
        callers_truncated: caller_impact == CallerImpact::TruncatedCallers
            || callers.len() > profile.max_displayed_callers,
        notes,
        blockers,
    })
}

fn eligible(group: &RankedGroup) -> bool {
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
        ReviewAction::AlreadyInMathlib
            | ReviewAction::ReplaceLocalUses
            | ReviewAction::LocalAlias
            | ReviewAction::InlinePrivateHelper
    )
}

fn private_helper_wrapper_cleanup(group: &RankedGroup, facts: &SourceFacts) -> bool {
    if !group.blockers.iter().any(|blocker| blocker == "non-public-declaration") {
        return false;
    }
    if !matches!(
        group.recommended_action,
        ReviewAction::ReplaceLocalUses | ReviewAction::LocalAlias
    ) {
        return false;
    }
    let Some(target_decl) = group.target_decl.as_deref() else {
        return false;
    };
    let Some(target) = group.members.iter().find(|member| {
        member.origin == "workspace" && member.visibility == "public" && member.qualified_name == target_decl
    }) else {
        return false;
    };
    let private_members = group
        .members
        .iter()
        .filter(|member| member.origin == "workspace" && member.visibility != "public")
        .collect::<Vec<_>>();
    let [private] = private_members.as_slice() else {
        return false;
    };
    let Some(private_fact) = facts.declaration(&private.declaration_id) else {
        return false;
    };
    private_fact.reference_status == SourceReferenceStatus::Complete
        && !private_fact.references.is_empty()
        && target.source_span.as_ref().is_some_and(|target_span| {
            private_fact
                .references
                .iter()
                .all(|reference| span_contains(target_span, reference))
        })
}

fn span_contains(span: &SourceSpan, reference: &SourceReference) -> bool {
    reference.file == std::path::Path::new(&span.file)
        && reference.line >= span.start.line as usize
        && reference.line <= span.end.line as usize
}

fn aggregate_import_status(statuses: impl Iterator<Item = ImportStatus>) -> ImportStatus {
    let statuses = statuses.collect::<Vec<_>>();
    if statuses.is_empty() {
        ImportStatus::Unknown
    } else if statuses.contains(&ImportStatus::SourceBackedNotImportable) {
        ImportStatus::SourceBackedNotImportable
    } else if statuses.iter().all(|status| *status == ImportStatus::Direct) {
        ImportStatus::Direct
    } else if statuses.contains(&ImportStatus::Missing) {
        ImportStatus::Missing
    } else {
        ImportStatus::Unknown
    }
}

fn aggregate_reference_status(statuses: impl Iterator<Item = Option<SourceReferenceStatus>>) -> SourceReferenceStatus {
    let statuses = statuses.collect::<Vec<_>>();
    if statuses.is_empty() || statuses.iter().any(Option::is_none) {
        SourceReferenceStatus::NotRequested
    } else if statuses
        .iter()
        .any(|status| status == &Some(SourceReferenceStatus::MissingSource))
    {
        SourceReferenceStatus::MissingSource
    } else if statuses
        .iter()
        .any(|status| status == &Some(SourceReferenceStatus::SourceUnavailable))
    {
        SourceReferenceStatus::SourceUnavailable
    } else if statuses
        .iter()
        .any(|status| status == &Some(SourceReferenceStatus::Truncated))
    {
        SourceReferenceStatus::Truncated
    } else if statuses
        .iter()
        .any(|status| status == &Some(SourceReferenceStatus::NotRequested))
    {
        SourceReferenceStatus::NotRequested
    } else {
        SourceReferenceStatus::Complete
    }
}

fn caller_impact(
    group: &RankedGroup,
    local_members: &[&ReviewMember],
    facts: &SourceFacts,
    status: SourceReferenceStatus,
    callers: &[SourceReference],
) -> CallerImpact {
    match status {
        SourceReferenceStatus::MissingSource => return CallerImpact::MissingSource,
        SourceReferenceStatus::NotRequested | SourceReferenceStatus::SourceUnavailable => {
            return CallerImpact::UnknownCallers;
        }
        SourceReferenceStatus::Truncated => return CallerImpact::TruncatedCallers,
        SourceReferenceStatus::Complete => {}
    }
    if callers.is_empty() {
        return CallerImpact::NoCallers;
    }
    if group.recommended_action == ReviewAction::InlinePrivateHelper && wrapper_only(group, local_members, facts) {
        CallerImpact::WrapperOnly
    } else {
        CallerImpact::BoundedCallers
    }
}

fn wrapper_only(group: &RankedGroup, local_members: &[&ReviewMember], facts: &SourceFacts) -> bool {
    let Some(target_decl) = group.target_decl.as_deref() else {
        return false;
    };
    let Some(target) = local_members
        .iter()
        .copied()
        .find(|member| member.visibility == "public" && member.qualified_name == target_decl)
    else {
        return false;
    };
    let Some(target_span) = target.source_span.as_ref() else {
        return false;
    };
    let private_facts = local_members
        .iter()
        .copied()
        .filter(|member| member.visibility != "public")
        .filter_map(|member| facts.declaration(&member.declaration_id))
        .collect::<Vec<_>>();
    !private_facts.is_empty()
        && private_facts
            .iter()
            .all(|fact| fact.reference_status == SourceReferenceStatus::Complete)
        && private_facts
            .iter()
            .flat_map(|fact| fact.references.iter())
            .all(|reference| span_contains(target_span, reference))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{CallerImpact, ReplacementHintProfile, attach_replacement_hints};
    use crate::ranking::{
        ConfidenceTier, RankedGroup, RankedReview, RankingDiagnostics, ReviewAction, ReviewEvidenceMode, ReviewMember,
        ReviewPriority, ReviewRelation,
    };
    use crate::source_refs::{ImportStatus, SourceFactInput, SourceReferenceStatus, collect_source_facts};
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::{Fingerprints, SourcePoint, SourceSpan};

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
                semantic_obligations: Vec::new(),
                local_caller_count: 1,
                replacement_hint: None,
            }],
            suppressed: Vec::new(),
            diagnostics: RankingDiagnostics::default(),
        };

        let review = attach_replacement_hints(review, &facts, ReplacementHintProfile::default());
        let hint = review.groups[0].replacement_hint.as_ref().unwrap();

        assert_eq!(hint.import_status, ImportStatus::Missing);
        assert_eq!(hint.caller_impact, CallerImpact::BoundedCallers);
        assert_eq!(hint.caller_count, 1);
        assert_eq!(hint.displayed_callers.len(), 1);
        assert!(hint.notes.iter().all(|note| !note.contains("delete")));
    }

    #[test]
    fn private_helper_used_only_by_public_wrapper_gets_inline_action() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("Tiny.lean");
        std::fs::write(
            &path,
            r#"
namespace Tiny
private theorem helper : True := by trivial
theorem public : True := helper
end Tiny
"#,
        )
        .unwrap();
        let private = declaration_with_span(
            "workspace:Tiny:_private.Tiny.0.Tiny.helper",
            "_private.Tiny.0.Tiny.helper",
            "private",
            &path,
            3,
            3,
        );
        let public = declaration_with_span("workspace:Tiny:Tiny.public", "Tiny.public", "public", &path, 4, 4);
        let facts = collect_source_facts(SourceFactInput::new(&[private.clone(), public.clone()]));
        let review = RankedReview {
            groups: vec![RankedGroup {
                id: "review-1".to_owned(),
                pair_id: "p1".to_owned(),
                relation: ReviewRelation::ExactStatement,
                members: vec![member(&public), member(&private)],
                evidence: Vec::new(),
                signals: vec![
                    "statement-fingerprint".to_owned(),
                    "probe:verified:exact-theorem".to_owned(),
                ],
                blockers: vec!["non-public-declaration".to_owned()],
                confidence: ConfidenceTier::High,
                review_priority: ReviewPriority::High,
                recommended_action: ReviewAction::ReplaceLocalUses,
                target_decl: Some("Tiny.public".to_owned()),
                target_module: Some("Tiny".to_owned()),
                evidence_mode: ReviewEvidenceMode::ProofGrade,
                probe_summary: Some("Lean verified semantic evidence".to_owned()),
                semantic_obligations: Vec::new(),
                local_caller_count: 1,
                replacement_hint: None,
            }],
            suppressed: Vec::new(),
            diagnostics: RankingDiagnostics::default(),
        };

        let review = attach_replacement_hints(review, &facts, ReplacementHintProfile::default());
        let group = &review.groups[0];
        assert_eq!(group.recommended_action, ReviewAction::InlinePrivateHelper);
        let hint = group.replacement_hint.as_ref().unwrap();
        assert_eq!(hint.caller_impact, CallerImpact::WrapperOnly);
        assert_eq!(hint.caller_count, 1);
        assert!(
            hint.notes
                .iter()
                .any(|note| note.contains("inline the helper body into the wrapper"))
        );
    }

    #[test]
    fn local_alias_without_callers_reports_no_caller_impact() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("Tiny.lean");
        std::fs::write(
            &path,
            r#"
namespace Tiny
theorem local : True := by trivial
end Tiny
"#,
        )
        .unwrap();
        let declaration = declaration_with_span("workspace:Tiny:Tiny.local", "Tiny.local", "public", &path, 3, 3);
        let facts = collect_source_facts(SourceFactInput::new(std::slice::from_ref(&declaration)));
        let review = RankedReview {
            groups: vec![group_for(
                vec![member(&declaration)],
                ReviewAction::LocalAlias,
                ReviewEvidenceMode::Static,
                Some("Tiny.local"),
                Some("Tiny"),
                Vec::new(),
            )],
            suppressed: Vec::new(),
            diagnostics: RankingDiagnostics::default(),
        };

        let review = attach_replacement_hints(review, &facts, ReplacementHintProfile::default());
        let hint = review.groups[0].replacement_hint.as_ref().unwrap();
        assert_eq!(hint.caller_impact, CallerImpact::NoCallers);
        assert!(hint.notes.iter().any(|note| note.contains("no local callers")));
        assert!(hint.notes.iter().any(|note| note.contains("alias-first")));
    }

    #[test]
    fn bounded_caller_truncation_is_explicit() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("Tiny.lean");
        std::fs::write(
            &path,
            r#"
namespace Tiny
theorem local : True := by trivial
theorem caller_one : True := local
theorem caller_two : True := local
theorem caller_three : True := local
end Tiny
"#,
        )
        .unwrap();
        let declaration = declaration_with_span("workspace:Tiny:Tiny.local", "Tiny.local", "public", &path, 3, 3);
        let mut input = SourceFactInput::new(std::slice::from_ref(&declaration));
        input.max_references_per_declaration = 2;
        let facts = collect_source_facts(input);
        assert_eq!(
            facts.declaration(&declaration.declaration_id).unwrap().reference_status,
            SourceReferenceStatus::Truncated
        );
        let review = RankedReview {
            groups: vec![group_for(
                vec![member(&declaration)],
                ReviewAction::ReplaceLocalUses,
                ReviewEvidenceMode::Static,
                Some("Tiny.local"),
                Some("Tiny"),
                Vec::new(),
            )],
            suppressed: Vec::new(),
            diagnostics: RankingDiagnostics::default(),
        };

        let review = attach_replacement_hints(
            review,
            &facts,
            ReplacementHintProfile {
                max_displayed_callers: 1,
                transitional_alias_callers: 10,
            },
        );
        let hint = review.groups[0].replacement_hint.as_ref().unwrap();
        assert_eq!(hint.caller_impact, CallerImpact::TruncatedCallers);
        assert!(hint.callers_truncated);
        assert!(hint.blockers.iter().any(|blocker| blocker == "caller-scan-truncated"));
        assert_eq!(hint.displayed_callers.len(), 1);
    }

    #[test]
    fn missing_source_and_source_backed_importability_are_blockers() {
        let missing = declaration_without_span("workspace:Tiny:Tiny.local", "Tiny.local");
        let facts = collect_source_facts(SourceFactInput::new(std::slice::from_ref(&missing)));
        let review = RankedReview {
            groups: vec![group_for(
                vec![member(&missing)],
                ReviewAction::ReplaceLocalUses,
                ReviewEvidenceMode::SourceBackedNotImportable,
                Some("Mathlib.local"),
                Some("Mathlib"),
                Vec::new(),
            )],
            suppressed: Vec::new(),
            diagnostics: RankingDiagnostics::default(),
        };

        let review = attach_replacement_hints(review, &facts, ReplacementHintProfile::default());
        let hint = review.groups[0].replacement_hint.as_ref().unwrap();
        assert_eq!(hint.import_status, ImportStatus::SourceBackedNotImportable);
        assert_eq!(hint.caller_impact, CallerImpact::MissingSource);
        assert!(
            hint.blockers
                .iter()
                .any(|blocker| blocker == "source-backed-not-importable")
        );
        assert!(hint.blockers.iter().any(|blocker| blocker == "missing-source"));
    }

    fn declaration(id: &str, name: &str, path: &std::path::Path) -> HydratedDeclaration {
        declaration_with_span(id, name, "public", path, 4, 4)
    }

    fn declaration_with_span(
        id: &str,
        name: &str,
        visibility: &str,
        path: &std::path::Path,
        start_line: u64,
        end_line: u64,
    ) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id(id),
            declaration_id: id.to_owned(),
            origin: "workspace".to_owned(),
            module: "Tiny".to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap().to_owned(),
            kind: "theorem".to_owned(),
            visibility: visibility.to_owned(),
            modifiers: Vec::new(),
            source_span: Some(SourceSpan {
                file: path.display().to_string(),
                start: SourcePoint {
                    line: start_line,
                    column: 1,
                },
                end: SourcePoint {
                    line: end_line,
                    column: 35,
                },
            }),
            statement_text: "theorem local : True".to_owned(),
            docstring_text: None,
            definition_body_summary: None,
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

    fn declaration_without_span(id: &str, name: &str) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id(id),
            declaration_id: id.to_owned(),
            origin: "workspace".to_owned(),
            module: "Tiny".to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap().to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: "theorem local : True".to_owned(),
            docstring_text: None,
            definition_body_summary: None,
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

    fn member(declaration: &HydratedDeclaration) -> ReviewMember {
        ReviewMember {
            declaration_id: declaration.declaration_id.clone(),
            origin: declaration.origin.clone(),
            module: declaration.module.clone(),
            qualified_name: declaration.qualified_name.clone(),
            display_name: declaration.display_name.clone(),
            kind: declaration.kind.clone(),
            visibility: declaration.visibility.clone(),
            source_span: declaration.source_span.clone(),
            status_flags: Vec::new(),
        }
    }

    fn group_for(
        members: Vec<ReviewMember>,
        action: ReviewAction,
        evidence_mode: ReviewEvidenceMode,
        target_decl: Option<&str>,
        target_module: Option<&str>,
        blockers: Vec<String>,
    ) -> RankedGroup {
        RankedGroup {
            id: "review-1".to_owned(),
            pair_id: "p1".to_owned(),
            relation: ReviewRelation::ExactStatement,
            members,
            evidence: Vec::new(),
            signals: vec!["statement-fingerprint".to_owned()],
            blockers,
            confidence: ConfidenceTier::High,
            review_priority: ReviewPriority::High,
            recommended_action: action,
            target_decl: target_decl.map(str::to_owned),
            target_module: target_module.map(str::to_owned),
            evidence_mode,
            probe_summary: None,
            semantic_obligations: Vec::new(),
            local_caller_count: 0,
            replacement_hint: None,
        }
    }
}
