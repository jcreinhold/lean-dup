use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::external_provenance::{ComparisonEvidenceMode, ComparisonProvenanceReport};
use crate::ranking::{RankedGroup, RankedReview, ReviewEvidenceMode, ReviewFilter, ReviewPriority};
use crate::semantic_verification::ProbeDiagnostics;

pub(crate) const REPORT_SCHEMA_VERSION: &str = "lean-dup.report.v1";

/// Stable explanations for audit output.
///
/// This module owns how review, probe, and provenance facts become user-facing
/// explanations. Callers do not need to know hidden-count precedence, probe
/// diagnostic fields, or provenance formatting rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AuditExplanations {
    pub(crate) visible_queue: VisibleQueueExplanation,
    pub(crate) hidden_groups: HiddenGroupExplanation,
    pub(crate) semantic_probes: SemanticProbeExplanation,
    pub(crate) comparison_provenance: ComparisonProvenanceExplanation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct VisibleQueueExplanation {
    pub(crate) visible: usize,
    pub(crate) total: usize,
    pub(crate) summary: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct HiddenGroupExplanation {
    pub(crate) total: usize,
    pub(crate) noise_or_profile: usize,
    pub(crate) generated: usize,
    pub(crate) unverified_proof_grade: usize,
    pub(crate) unavailable_probe: usize,
    pub(crate) other_blockers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SemanticProbeExplanation {
    pub(crate) enabled: bool,
    pub(crate) summary: String,
    pub(crate) planned_pairs: usize,
    pub(crate) verified_results: usize,
    pub(crate) unavailable_results: usize,
    pub(crate) cached_hits: usize,
    pub(crate) worker_pairs: usize,
    pub(crate) unavailable_by_reason: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ComparisonProvenanceExplanation {
    pub(crate) summary: String,
    pub(crate) entries: Vec<ComparisonProvenanceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ComparisonProvenanceEntry {
    pub(crate) label: Option<String>,
    pub(crate) origin: String,
    pub(crate) evidence_mode: ComparisonEvidenceMode,
    pub(crate) declaration_count: usize,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GroupExplanation {
    pub(crate) visibility: String,
    pub(crate) visibility_reason: String,
    pub(crate) evidence_mode: ReviewEvidenceMode,
    pub(crate) evidence_summary: String,
    pub(crate) semantic_summary: String,
    pub(crate) blocker_summary: String,
    pub(crate) replacement_summary: String,
}

pub(crate) fn explain_audit(
    review: &RankedReview,
    visible_groups: &[RankedGroup],
    filter: ReviewFilter,
    probes: &ProbeDiagnostics,
    provenance: &[ComparisonProvenanceReport],
) -> AuditExplanations {
    let visible_ids = visible_groups
        .iter()
        .map(|group| group.id.as_str())
        .collect::<BTreeSet<_>>();
    let hidden_groups = hidden_groups(review, &visible_ids, filter);
    let visible_queue = visible_queue(visible_groups.len(), review.groups.len(), &hidden_groups);
    AuditExplanations {
        visible_queue,
        hidden_groups,
        semantic_probes: semantic_probes(probes),
        comparison_provenance: comparison_provenance(provenance),
    }
}

pub(crate) fn explain_group(group: &RankedGroup, filter: ReviewFilter) -> GroupExplanation {
    let (visibility, visibility_reason) = if filter.includes(group) {
        (
            "visible".to_owned(),
            "included by the active review profile and output filters".to_owned(),
        )
    } else {
        (
            "hidden".to_owned(),
            hidden_reason(group, filter)
                .map(hidden_reason_sentence)
                .unwrap_or_else(|| "hidden by the active review filters".to_owned()),
        )
    };

    GroupExplanation {
        visibility,
        visibility_reason,
        evidence_mode: group.evidence_mode,
        evidence_summary: evidence_summary(group.evidence_mode),
        semantic_summary: semantic_summary(group),
        blocker_summary: blocker_summary(group),
        replacement_summary: replacement_summary(group),
    }
}

fn visible_queue(visible: usize, total: usize, hidden: &HiddenGroupExplanation) -> VisibleQueueExplanation {
    let summary = format!("{visible}/{total} ranked groups visible");
    let reason = if total == 0 {
        "No candidate groups were ranked after retrieval and review shaping.".to_owned()
    } else if visible > 0 {
        format!(
            "{visible} groups match the active review profile; {} groups are hidden.",
            total - visible
        )
    } else if hidden.noise_or_profile == hidden.total {
        "All ranked groups are hidden by the active review profile or noise filter.".to_owned()
    } else if hidden.unverified_proof_grade == hidden.total {
        "All ranked groups require proof-grade semantic evidence that was not verified.".to_owned()
    } else if hidden.unavailable_probe == hidden.total {
        "All ranked groups are blocked by unavailable Lean semantic probes.".to_owned()
    } else {
        "No ranked groups pass the active review profile; see hidden group counts for blockers.".to_owned()
    };
    VisibleQueueExplanation {
        visible,
        total,
        summary,
        reason,
    }
}

fn hidden_groups(review: &RankedReview, visible_ids: &BTreeSet<&str>, filter: ReviewFilter) -> HiddenGroupExplanation {
    let mut counts = HiddenGroupExplanation {
        total: 0,
        noise_or_profile: 0,
        generated: 0,
        unverified_proof_grade: 0,
        unavailable_probe: 0,
        other_blockers: 0,
    };
    for group in &review.groups {
        if visible_ids.contains(group.id.as_str()) || filter.includes(group) {
            continue;
        }
        counts.total += 1;
        match hidden_reason(group, filter) {
            Some(HiddenReason::Generated) => counts.generated += 1,
            Some(HiddenReason::UnverifiedProofGrade) => counts.unverified_proof_grade += 1,
            Some(HiddenReason::UnavailableProbe) => counts.unavailable_probe += 1,
            Some(HiddenReason::NoiseOrProfile) => counts.noise_or_profile += 1,
            Some(HiddenReason::OtherBlocker) | None => counts.other_blockers += 1,
        }
    }
    counts
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HiddenReason {
    Generated,
    UnverifiedProofGrade,
    UnavailableProbe,
    NoiseOrProfile,
    OtherBlocker,
}

fn hidden_reason(group: &RankedGroup, filter: ReviewFilter) -> Option<HiddenReason> {
    if filter.includes(group) {
        return None;
    }
    if !filter.include_generated && has_blocker(group, "generated-declaration") {
        return Some(HiddenReason::Generated);
    }
    if has_blocker(group, "unverified-proof-grade-evidence") {
        return Some(HiddenReason::UnverifiedProofGrade);
    }
    if has_blocker(group, "lean-probe-unavailable") {
        return Some(HiddenReason::UnavailableProbe);
    }
    if group.review_priority == ReviewPriority::Noise
        || group.review_priority > filter.min_priority
        || !filter.show_noise
    {
        return Some(HiddenReason::NoiseOrProfile);
    }
    if !group.blockers.is_empty() {
        return Some(HiddenReason::OtherBlocker);
    }
    Some(HiddenReason::OtherBlocker)
}

fn has_blocker(group: &RankedGroup, blocker: &str) -> bool {
    group.blockers.iter().any(|item| item == blocker)
}

fn hidden_reason_sentence(reason: HiddenReason) -> String {
    match reason {
        HiddenReason::Generated => "hidden because generated declarations are excluded".to_owned(),
        HiddenReason::UnverifiedProofGrade => {
            "hidden because proof-grade comparison evidence was required but not verified".to_owned()
        }
        HiddenReason::UnavailableProbe => "hidden because the required Lean semantic probe is unavailable".to_owned(),
        HiddenReason::NoiseOrProfile => "hidden by the active review profile or noise filter".to_owned(),
        HiddenReason::OtherBlocker => "hidden by blockers or output filters".to_owned(),
    }
}

fn semantic_probes(probes: &ProbeDiagnostics) -> SemanticProbeExplanation {
    let summary = if !probes.enabled {
        "semantic probes disabled".to_owned()
    } else if probes.planned_pairs == 0 {
        "semantic probes enabled; no probe obligations were planned".to_owned()
    } else if probes.verified_results > 0 {
        format!(
            "{} verified, {} unavailable from {} planned semantic probe pairs",
            probes.verified_results, probes.unavailable_results, probes.planned_pairs
        )
    } else if probes.unavailable_results > 0 {
        format!(
            "0 verified, {} unavailable from {} planned semantic probe pairs",
            probes.unavailable_results, probes.planned_pairs
        )
    } else {
        format!(
            "0 verified and 0 unavailable from {} planned semantic probe pairs",
            probes.planned_pairs
        )
    };
    SemanticProbeExplanation {
        enabled: probes.enabled,
        summary,
        planned_pairs: probes.planned_pairs,
        verified_results: probes.verified_results,
        unavailable_results: probes.unavailable_results,
        cached_hits: probes.cached_hits,
        worker_pairs: probes.worker_pairs,
        unavailable_by_reason: probes.unavailable_by_reason.clone(),
    }
}

fn comparison_provenance(reports: &[ComparisonProvenanceReport]) -> ComparisonProvenanceExplanation {
    if reports.is_empty() {
        return ComparisonProvenanceExplanation {
            summary: "no comparison indexes".to_owned(),
            entries: Vec::new(),
        };
    }
    let entries = reports
        .iter()
        .map(|report| ComparisonProvenanceEntry {
            label: report.label.clone(),
            origin: report.origin.clone(),
            evidence_mode: report.evidence_mode,
            declaration_count: report.declaration_count,
            reason: report.reason.clone(),
        })
        .collect::<Vec<_>>();
    let summary = entries
        .iter()
        .map(|entry| {
            let label = entry.label.as_deref().unwrap_or("-");
            format!(
                "{label}/{}={} ({} declarations)",
                entry.origin,
                comparison_mode_label(entry.evidence_mode),
                entry.declaration_count
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    ComparisonProvenanceExplanation { summary, entries }
}

fn comparison_mode_label(mode: ComparisonEvidenceMode) -> &'static str {
    match mode {
        ComparisonEvidenceMode::Static => "static",
        ComparisonEvidenceMode::SourceBackedNotImportable => "source-backed-not-importable",
        ComparisonEvidenceMode::ProofGrade => "proof-grade",
    }
}

fn evidence_summary(mode: ReviewEvidenceMode) -> String {
    match mode {
        ReviewEvidenceMode::Static => "static indexed evidence; Lean did not verify this group".to_owned(),
        ReviewEvidenceMode::SourceBackedNotImportable => {
            "source-backed index exists, but its declarations are not importable in this audit".to_owned()
        }
        ReviewEvidenceMode::ProofGrade => {
            "proof-grade source-backed evidence is required for actionable claims".to_owned()
        }
    }
}

fn semantic_summary(group: &RankedGroup) -> String {
    if let Some(summary) = &group.probe_summary {
        return summary.clone();
    }
    if group.signals.iter().any(|signal| signal.starts_with("probe:verified:")) {
        return "Lean verified semantic evidence for this group".to_owned();
    }
    if has_blocker(group, "lean-probe-unavailable") {
        return "Lean semantic probe was unavailable for this group".to_owned();
    }
    if has_blocker(group, "lean-probe-rejected") {
        return "Lean semantic probe rejected the planned obligation".to_owned();
    }
    if has_blocker(group, "unverified-proof-grade-evidence") {
        return "proof-grade evidence is required, but no verified semantic evidence is attached".to_owned();
    }
    "no additional semantic probe evidence is attached".to_owned()
}

fn blocker_summary(group: &RankedGroup) -> String {
    if group.blockers.is_empty() {
        "none".to_owned()
    } else {
        group.blockers.join(", ")
    }
}

fn replacement_summary(group: &RankedGroup) -> String {
    if let Some(hint) = &group.replacement_hint {
        let mut summary = format!(
            "target {}; import={:?}; callers={}",
            hint.target_decl, hint.import_status, hint.caller_count
        )
        .to_ascii_lowercase();
        if !hint.blockers.is_empty() {
            summary.push_str(&format!("; blockers={}", hint.blockers.join(", ")));
        }
        if !hint.notes.is_empty() {
            summary.push_str(&format!("; notes={}", hint.notes.join("; ")));
        }
        summary
    } else {
        format!("manual review; local callers={}", group.local_caller_count)
    }
}

#[cfg(test)]
mod tests {
    use super::{explain_audit, explain_group};
    use crate::external_provenance::{ComparisonEvidenceMode, ComparisonProvenanceReport};
    use crate::ranking::{
        ConfidenceTier, RankedGroup, RankedReview, RankingDiagnostics, ReviewAction, ReviewEvidenceMode, ReviewFilter,
        ReviewPriority, ReviewRelation,
    };
    use crate::semantic_verification::ProbeDiagnostics;

    #[test]
    fn zero_visible_groups_have_concrete_reason() {
        let review = review(vec![group(
            "g1",
            ReviewPriority::Noise,
            vec!["unverified-proof-grade-evidence"],
            ReviewEvidenceMode::ProofGrade,
        )]);
        let explanations = explain_audit(&review, &[], default_filter(), &ProbeDiagnostics::default(), &[]);

        assert_eq!(explanations.visible_queue.visible, 0);
        assert!(explanations.visible_queue.reason.contains("proof-grade"));
        assert_eq!(explanations.hidden_groups.unverified_proof_grade, 1);
    }

    #[test]
    fn hidden_group_counts_are_exclusive_and_deterministic() {
        let review = review(vec![
            group(
                "generated",
                ReviewPriority::Noise,
                vec!["generated-declaration", "unverified-proof-grade-evidence"],
                ReviewEvidenceMode::ProofGrade,
            ),
            group(
                "unverified",
                ReviewPriority::Noise,
                vec!["unverified-proof-grade-evidence"],
                ReviewEvidenceMode::ProofGrade,
            ),
            group(
                "unavailable",
                ReviewPriority::Noise,
                vec!["lean-probe-unavailable"],
                ReviewEvidenceMode::ProofGrade,
            ),
            group("noise", ReviewPriority::Noise, vec![], ReviewEvidenceMode::Static),
        ]);
        let explanations = explain_audit(&review, &[], default_filter(), &ProbeDiagnostics::default(), &[]);

        assert_eq!(explanations.hidden_groups.total, 4);
        assert_eq!(explanations.hidden_groups.generated, 1);
        assert_eq!(explanations.hidden_groups.unverified_proof_grade, 1);
        assert_eq!(explanations.hidden_groups.unavailable_probe, 1);
        assert_eq!(explanations.hidden_groups.noise_or_profile, 1);
    }

    #[test]
    fn probe_summary_uses_stable_unavailable_reason_labels() {
        let mut diagnostics = ProbeDiagnostics {
            enabled: true,
            planned_pairs: 2,
            unavailable_results: 2,
            ..ProbeDiagnostics::default()
        };
        diagnostics
            .unavailable_by_reason
            .insert("opaque-or-unreducible".to_owned(), 2);

        let explanations = explain_audit(&review(Vec::new()), &[], default_filter(), &diagnostics, &[]);

        assert_eq!(
            explanations
                .semantic_probes
                .unavailable_by_reason
                .get("opaque-or-unreducible"),
            Some(&2)
        );
        assert!(explanations.semantic_probes.summary.contains("2 unavailable"));
    }

    #[test]
    fn group_explanations_distinguish_evidence_modes() {
        let static_group = group("static", ReviewPriority::High, vec![], ReviewEvidenceMode::Static);
        let not_importable = group(
            "not-importable",
            ReviewPriority::High,
            vec![],
            ReviewEvidenceMode::SourceBackedNotImportable,
        );
        let proof_grade = group("proof", ReviewPriority::High, vec![], ReviewEvidenceMode::ProofGrade);

        assert!(
            explain_group(&static_group, default_filter())
                .evidence_summary
                .contains("static")
        );
        assert!(
            explain_group(&not_importable, default_filter())
                .evidence_summary
                .contains("not importable")
        );
        assert!(
            explain_group(&proof_grade, default_filter())
                .evidence_summary
                .contains("proof-grade")
        );
    }

    #[test]
    fn comparison_provenance_explanation_omits_paths() {
        let report = ComparisonProvenanceReport {
            label: Some("mathlib".to_owned()),
            origin: "mathlib".to_owned(),
            evidence_mode: ComparisonEvidenceMode::ProofGrade,
            declaration_count: 10,
            index_path: "/tmp/index.sqlite".into(),
            source_root: Some("/tmp/source".into()),
            execution_root: Some("/tmp/project".into()),
            execution_policy: "project".to_owned(),
            reason: "importable".to_owned(),
        };
        let explanations = explain_audit(
            &review(Vec::new()),
            &[],
            default_filter(),
            &ProbeDiagnostics::default(),
            &[report],
        );

        assert_eq!(explanations.comparison_provenance.entries[0].origin, "mathlib");
        assert!(explanations.comparison_provenance.summary.contains("proof-grade"));
    }

    fn default_filter() -> ReviewFilter {
        ReviewFilter {
            include_generated: false,
            show_noise: false,
            min_priority: ReviewPriority::Medium,
        }
    }

    fn review(groups: Vec<RankedGroup>) -> RankedReview {
        RankedReview {
            diagnostics: RankingDiagnostics {
                candidate_pairs: groups.len(),
                emitted_groups: groups.len(),
                suppressed_groups: 0,
            },
            groups,
            suppressed: Vec::new(),
        }
    }

    fn group(
        id: &str,
        priority: ReviewPriority,
        blockers: Vec<&str>,
        evidence_mode: ReviewEvidenceMode,
    ) -> RankedGroup {
        RankedGroup {
            id: id.to_owned(),
            pair_id: id.to_owned(),
            relation: ReviewRelation::ExactStatement,
            members: Vec::new(),
            evidence: Vec::new(),
            signals: Vec::new(),
            blockers: blockers.into_iter().map(str::to_owned).collect(),
            confidence: ConfidenceTier::High,
            review_priority: priority,
            recommended_action: ReviewAction::ManualReview,
            target_decl: None,
            target_module: None,
            evidence_mode,
            probe_summary: None,
            local_caller_count: 0,
            replacement_hint: None,
        }
    }
}
