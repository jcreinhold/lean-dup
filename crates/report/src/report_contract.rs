use std::collections::BTreeMap;

use serde::Serialize;

use lean_dup_index::{ComparisonEvidenceMode, ComparisonProvenanceReport};
use lean_dup_search::{AuditGroup, AuditProbeSummary, AuditQueueSummary, AuditReview};

pub const REPORT_SCHEMA_VERSION: &str = "lean-dup.report.v3";

/// Stable explanations for audit output.
///
/// This module owns how review, probe, and provenance facts become user-facing
/// explanations. Callers do not need to know hidden-count precedence, probe
/// diagnostic fields, or provenance formatting rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditExplanations {
    pub visible_queue: VisibleQueueExplanation,
    pub hidden_groups: HiddenGroupExplanation,
    pub semantic_probes: SemanticProbeExplanation,
    pub comparison_provenance: ComparisonProvenanceExplanation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VisibleQueueExplanation {
    pub visible: usize,
    pub emitted: usize,
    pub limit: usize,
    pub truncated: bool,
    pub total: usize,
    pub summary: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenGroupExplanation {
    pub total: usize,
    pub visibility_or_noise: usize,
    pub generated: usize,
    pub unverified_proof_grade: usize,
    pub unavailable_probe: usize,
    pub other_blockers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticProbeExplanation {
    pub enabled: bool,
    pub summary: String,
    pub planned_pairs: usize,
    pub verified_results: usize,
    pub rejected_results: usize,
    pub unavailable_results: usize,
    pub cached_hits: usize,
    pub worker_pairs: usize,
    pub unavailable_by_reason: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonProvenanceExplanation {
    pub summary: String,
    pub entries: Vec<ComparisonProvenanceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonProvenanceEntry {
    pub label: Option<String>,
    pub origin: String,
    pub evidence_mode: String,
    pub declaration_count: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupExplanation {
    pub visibility: String,
    pub visibility_reason: String,
    pub evidence_mode: String,
    pub evidence_summary: String,
    pub semantic_summary: String,
    pub blocker_summary: String,
    pub replacement_summary: String,
}

pub fn explain_audit(
    _review: &AuditReview,
    _visible_groups: &[AuditGroup],
    queue: &AuditQueueSummary,
    probes: &AuditProbeSummary,
    provenance: &[ComparisonProvenanceReport],
) -> AuditExplanations {
    let hidden_groups = hidden_groups(queue);
    let visible_queue = visible_queue(queue, &hidden_groups);
    AuditExplanations {
        visible_queue,
        hidden_groups,
        semantic_probes: semantic_probes(probes),
        comparison_provenance: comparison_provenance(provenance),
    }
}

pub fn explain_group(group: &AuditGroup) -> GroupExplanation {
    GroupExplanation {
        visibility: if group.visibility.visible { "visible" } else { "hidden" }.to_owned(),
        visibility_reason: group.visibility.reason.clone(),
        evidence_mode: group.evidence_mode.clone(),
        evidence_summary: evidence_summary(&group.evidence_mode),
        semantic_summary: semantic_summary(group),
        blocker_summary: blocker_summary(group),
        replacement_summary: replacement_summary(group),
    }
}

fn visible_queue(queue: &AuditQueueSummary, hidden: &HiddenGroupExplanation) -> VisibleQueueExplanation {
    let visible = queue.visible;
    let emitted = queue.emitted;
    let total = queue.total;
    let truncated = queue.truncated;
    let summary = if truncated {
        format!("{visible}/{total} ranked groups visible; emitted first {emitted}")
    } else {
        format!("{visible}/{total} ranked groups visible")
    };
    let reason = if total == 0 {
        "No candidate groups were ranked after retrieval and review shaping.".to_owned()
    } else if visible > 0 {
        let truncation = if truncated {
            format!(" Only the first {emitted} groups are included in ordinary audit JSON.")
        } else {
            String::new()
        };
        format!(
            "{visible} groups match the active audit visibility options; {} groups are hidden.{truncation}",
            total - visible
        )
    } else if hidden.visibility_or_noise == hidden.total {
        "All ranked groups are hidden by the active audit visibility options.".to_owned()
    } else if hidden.unverified_proof_grade == hidden.total {
        "All ranked groups require proof-grade semantic evidence that was not verified.".to_owned()
    } else if hidden.unavailable_probe == hidden.total {
        "All ranked groups are blocked by unavailable Lean semantic probes.".to_owned()
    } else {
        "No ranked groups pass the active audit visibility options; see hidden group counts for blockers.".to_owned()
    };
    VisibleQueueExplanation {
        visible,
        emitted,
        limit: queue.limit,
        truncated,
        total,
        summary,
        reason,
    }
}

fn hidden_groups(queue: &AuditQueueSummary) -> HiddenGroupExplanation {
    HiddenGroupExplanation {
        total: queue.hidden.total,
        visibility_or_noise: queue.hidden.visibility_or_noise,
        generated: queue.hidden.generated,
        unverified_proof_grade: queue.hidden.unverified_proof_grade,
        unavailable_probe: queue.hidden.unavailable_probe,
        other_blockers: queue.hidden.other_blockers,
    }
}

fn has_blocker(group: &AuditGroup, blocker: &str) -> bool {
    group.blockers.iter().any(|item| item == blocker)
}

fn semantic_probes(probes: &AuditProbeSummary) -> SemanticProbeExplanation {
    let summary = if !probes.enabled {
        "semantic probes disabled".to_owned()
    } else if probes.planned_pairs == 0 {
        "semantic probes enabled; no probe obligations were planned".to_owned()
    } else if probes.verified_results > 0 {
        format!(
            "{} verified, {} rejected, {} unavailable from {} planned semantic probe pairs",
            probes.verified_results, probes.rejected_results, probes.unavailable_results, probes.planned_pairs
        )
    } else if probes.rejected_results > 0 {
        format!(
            "0 verified, {} rejected, {} unavailable from {} planned semantic probe pairs",
            probes.rejected_results, probes.unavailable_results, probes.planned_pairs
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
        rejected_results: probes.rejected_results,
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
            evidence_mode: comparison_mode_label(report.evidence_mode).to_owned(),
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
                entry.origin, entry.evidence_mode, entry.declaration_count
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

fn evidence_summary(mode: &str) -> String {
    match mode {
        "static" => "static indexed evidence; Lean did not verify this group".to_owned(),
        "source-backed-not-importable" => {
            "source-backed index exists, but its declarations are not importable in this audit".to_owned()
        }
        "proof-grade" => "proof-grade source-backed evidence is required for actionable claims".to_owned(),
        _ => "unknown evidence mode".to_owned(),
    }
}

fn semantic_summary(group: &AuditGroup) -> String {
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

fn blocker_summary(group: &AuditGroup) -> String {
    if group.blockers.is_empty() {
        "none".to_owned()
    } else {
        group.blockers.join(", ")
    }
}

fn replacement_summary(group: &AuditGroup) -> String {
    if let Some(hint) = &group.replacement_hint {
        let mut summary = format!(
            "target {}; import={}; callers={}",
            hint.target_decl, hint.import_status, hint.caller_count
        );
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
    use lean_dup_index::{ComparisonEvidenceMode, ComparisonProvenanceReport};
    use lean_dup_search::{
        AuditGroup, AuditHiddenGroupCounts, AuditProbeSummary, AuditQueueSummary, AuditReview, AuditReviewDiagnostics,
        AuditVisibility,
    };

    #[test]
    fn zero_visible_groups_have_concrete_reason() {
        let review = review(vec![group("g1", "proof-grade", false)]);
        let queue = queue(AuditHiddenGroupCounts {
            total: 1,
            unverified_proof_grade: 1,
            ..AuditHiddenGroupCounts::default()
        });
        let explanations = explain_audit(&review, &[], &queue, &AuditProbeSummary::default(), &[]);

        assert_eq!(explanations.visible_queue.visible, 0);
        assert!(explanations.visible_queue.reason.contains("proof-grade"));
        assert_eq!(explanations.hidden_groups.unverified_proof_grade, 1);
    }

    #[test]
    fn hidden_group_counts_are_exclusive_and_deterministic() {
        let review = review(vec![
            group("generated", "proof-grade", false),
            group("unverified", "proof-grade", false),
            group("unavailable", "proof-grade", false),
            group("noise", "static", false),
        ]);
        let queue = queue(AuditHiddenGroupCounts {
            total: 4,
            generated: 1,
            unverified_proof_grade: 1,
            unavailable_probe: 1,
            visibility_or_noise: 1,
            ..AuditHiddenGroupCounts::default()
        });
        let explanations = explain_audit(&review, &[], &queue, &AuditProbeSummary::default(), &[]);

        assert_eq!(explanations.hidden_groups.total, 4);
        assert_eq!(explanations.hidden_groups.generated, 1);
        assert_eq!(explanations.hidden_groups.unverified_proof_grade, 1);
        assert_eq!(explanations.hidden_groups.unavailable_probe, 1);
        assert_eq!(explanations.hidden_groups.visibility_or_noise, 1);
    }

    #[test]
    fn probe_summary_uses_stable_unavailable_reason_labels() {
        let mut diagnostics = AuditProbeSummary {
            enabled: true,
            planned_pairs: 2,
            rejected_results: 1,
            unavailable_results: 2,
            ..AuditProbeSummary::default()
        };
        diagnostics
            .unavailable_by_reason
            .insert("opaque-or-unreducible".to_owned(), 2);

        let explanations = explain_audit(
            &review(Vec::new()),
            &[],
            &queue(AuditHiddenGroupCounts::default()),
            &diagnostics,
            &[],
        );

        assert_eq!(
            explanations
                .semantic_probes
                .unavailable_by_reason
                .get("opaque-or-unreducible"),
            Some(&2)
        );
        assert_eq!(explanations.semantic_probes.rejected_results, 1);
        assert!(explanations.semantic_probes.summary.contains("1 rejected"));
        assert!(explanations.semantic_probes.summary.contains("2 unavailable"));
    }

    #[test]
    fn group_explanations_distinguish_evidence_modes() {
        let static_group = group("static", "static", true);
        let not_importable = group("not-importable", "source-backed-not-importable", true);
        let proof_grade = group("proof", "proof-grade", true);

        assert!(explain_group(&static_group).evidence_summary.contains("static"));
        assert!(
            explain_group(&not_importable)
                .evidence_summary
                .contains("not importable")
        );
        assert!(explain_group(&proof_grade).evidence_summary.contains("proof-grade"));
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
            &queue(AuditHiddenGroupCounts::default()),
            &AuditProbeSummary::default(),
            &[report],
        );

        assert_eq!(explanations.comparison_provenance.entries[0].origin, "mathlib");
        assert!(explanations.comparison_provenance.summary.contains("proof-grade"));
    }

    fn review(groups: Vec<AuditGroup>) -> AuditReview {
        AuditReview {
            diagnostics: AuditReviewDiagnostics {
                candidate_pairs: groups.len(),
                emitted_groups: groups.len(),
                suppressed_groups: 0,
            },
            group_count: groups.len(),
            groups,
            suppressed_count: 0,
        }
    }

    fn queue(hidden: AuditHiddenGroupCounts) -> AuditQueueSummary {
        AuditQueueSummary {
            visible: 0,
            emitted: 0,
            limit: 500,
            truncated: false,
            total: hidden.total,
            hidden,
        }
    }

    fn group(id: &str, evidence_mode: &str, visible: bool) -> AuditGroup {
        AuditGroup {
            family_id: id.to_owned(),
            id: id.to_owned(),
            pair_id: id.to_owned(),
            pair_count: 1,
            pair_ids: vec![id.to_owned()],
            pair_evidence: Vec::new(),
            pair_evidence_truncated: false,
            relation: "exact-statement".to_owned(),
            members: Vec::new(),
            evidence: Vec::new(),
            signals: Vec::new(),
            blockers: Vec::new(),
            confidence: "high".to_owned(),
            review_priority: "high".to_owned(),
            recommended_action: "manual-review".to_owned(),
            target_decl: None,
            target_module: None,
            evidence_mode: evidence_mode.to_owned(),
            probe_summary: None,
            semantic_obligations: Vec::new(),
            local_caller_count: 0,
            replacement_hint: None,
            visibility: AuditVisibility {
                visible,
                reason: if visible { "visible" } else { "hidden" }.to_owned(),
                hidden_reason: None,
            },
        }
    }
}
