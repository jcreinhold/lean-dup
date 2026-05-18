use lean_dup_search::ReviewProfile;

use crate::report_contract::GroupExplanation;
use crate::reports::{
    AuditReport, CacheCleanupReportDto, DiffReport, DoctorReport, IndexReport, PerfReport, Report, ReviewGroupReport,
    ShowReport,
};

pub fn render_text(report: &Report) -> String {
    match report {
        Report::Doctor(report) => render_doctor(report),
        Report::CacheCleanup(report) => render_cache_cleanup(report),
        Report::Index(report) => render_index("index", report),
        Report::IndexMathlib(report) => render_index("index-mathlib", report),
        Report::Show(report) => render_show(report),
        Report::Diff(report) => render_diff(report),
        Report::Audit(report) => render_audit(report),
        Report::Eval(report) => lean_dup_eval::render_metrics(&report.metrics),
        Report::Perf(report) => render_perf(report),
    }
}

fn render_perf(report: &PerfReport) -> String {
    let mut lines = vec![
        "command: perf".to_owned(),
        format!("status: {}", report.status),
        format!("workload: {}", report.workload),
        format!("cache root: {}", report.cache_root.display()),
        format!("exit code: {}", report.report.exit_code),
        format!("elapsed ms: {}", report.report.elapsed_ms),
        format!("stdout bytes: {}", report.report.stdout_bytes),
        format!("stderr bytes: {}", report.report.stderr_bytes),
    ];
    for (class, elapsed_ms) in &report.report.summary.elapsed_ms_by_class {
        lines.push(format!("cost {:?}: {elapsed_ms}ms", class).to_ascii_lowercase());
    }
    lines.join("\n")
}

fn render_doctor(report: &DoctorReport) -> String {
    let mut lines = vec![
        "command: doctor".to_owned(),
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("lakefile: {}", report.lakefile.display()),
        format!("module roots: {}", report.module_roots.join(", ")),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
        format!("cache labels: {}", report.cache.labels.len()),
        format!("cache disk bytes: {}", report.cache.total_disk_bytes),
        format!("lean: {}", report.lean_version),
    ];
    for label in &report.cache.labels {
        lines.push(format!(
            "cache label {}: latest={} entries={} bytes={}",
            label.label,
            label.latest.status,
            label.entries.len(),
            label.disk_bytes
        ));
        for entry in &label.entries {
            lines.push(format!(
                "  {}: status={} active={} expected={} schema={} provenance={} bytes={}",
                entry.index_dir.display(),
                entry.status,
                entry.active_latest,
                entry.expected_current,
                entry.schema_version.as_deref().unwrap_or("missing"),
                entry.provenance_kind,
                entry.disk_bytes
            ));
        }
    }
    if report.require_oleans {
        if report.missing_oleans.is_empty() {
            lines.push("oleans: ok".to_owned());
        } else {
            lines.push(format!(
                "oleans: missing {} ({})",
                report.missing_oleans.len(),
                report
                    .missing_oleans
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    lines.join("\n")
}

fn render_cache_cleanup(report: &CacheCleanupReportDto) -> String {
    let mut lines = vec![
        "command: cache-cleanup".to_owned(),
        format!("status: {}", report.status),
        format!("cache root: {}", report.cache_root.display()),
        format!("executed: {}", report.executed),
        format!("removable entries: {}", report.removable_count),
        format!("protected entries: {}", report.protected_count),
        format!("bytes to remove: {}", report.bytes_to_remove),
        format!("bytes removed: {}", report.bytes_removed),
    ];
    for entry in &report.removed_entries {
        lines.push(format!(
            "  removable {} {} bytes ({})",
            entry.index_dir.display(),
            entry.disk_bytes,
            entry.reason
        ));
    }
    for entry in &report.protected_entries {
        lines.push(format!(
            "  protected {} {} bytes ({})",
            entry.index_dir.display(),
            entry.disk_bytes,
            entry.reason
        ));
    }
    lines.join("\n")
}

fn render_show(report: &ShowReport) -> String {
    let mut lines = vec![
        "command: show".to_owned(),
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
    ];
    lines.push(String::new());
    push_group_detail(&mut lines, &report.group);
    push_group_explanation(&mut lines, &report.explanation);
    lines.join("\n")
}

fn render_diff(report: &DiffReport) -> String {
    let mut lines = vec![
        "command: diff".to_owned(),
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
        format!("baseline: {}", report.diff.baseline),
        format!("baseline path: {}", report.diff.baseline_path.display()),
        format!("appeared: {}", report.diff.appeared.len()),
        format!("disappeared: {}", report.diff.disappeared.len()),
        format!("changed: {}", report.diff.changed.len()),
    ];
    for group in report.diff.appeared.iter().take(20) {
        lines.push(format!("  appeared {}", group.id));
    }
    for group in report.diff.disappeared.iter().take(20) {
        lines.push(format!("  disappeared {}", group.id));
    }
    for change in report.diff.changed.iter().take(20) {
        lines.push(format!("  changed {}", change.id));
    }
    lines.join("\n")
}

fn render_index(command: &str, report: &IndexReport) -> String {
    let mut lines = vec![
        format!("command: {command}"),
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
        format!("label: {}", report.label),
        format!("cache: {:?}", report.cache_status).to_ascii_lowercase(),
        format!("index path: {}", report.index_path.display()),
        format!("index dir: {}", report.index_dir.display()),
        format!("declarations: {}", report.declaration_count),
    ];
    if report.force {
        lines.push("force: true".to_owned());
    }
    if !report.diagnostics.is_empty() {
        lines.push(format!("diagnostics: {}", report.diagnostics.join("; ")));
    }
    lines.join("\n")
}

fn render_audit(report: &AuditReport) -> String {
    let mut lines = vec![
        "command: audit".to_owned(),
        format!("report schema: {}", report.report_schema_version),
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
        format!("include private: {}", report.include_private),
        format!("compare mathlib: {}", report.compare_mathlib),
        format!("review profile: {}", review_profile_label(report.review_profile)),
        format!("candidates: {}", report.retrieval.candidate_count),
        format!(
            "comparison provenance: {}",
            report.explanations.comparison_provenance.summary
        ),
        format!(
            "semantic probes: planned={} cached={} worker={} unavailable={}",
            report.semantic_verification.planned_pairs,
            report.semantic_verification.cached_hits,
            report.semantic_verification.worker_pairs,
            report.semantic_verification.unavailable_results
        ),
        format!("review groups: {}", report.review.groups.len()),
        format!("visible groups: {}", report.visible_group_count),
        format!("visible queue: {}", report.explanations.visible_queue.reason),
        format!(
            "hidden groups: total={} profile/noise={} generated={} unverified-proof-grade={} unavailable-probe={} other={}",
            report.explanations.hidden_groups.total,
            report.explanations.hidden_groups.noise_or_profile,
            report.explanations.hidden_groups.generated,
            report.explanations.hidden_groups.unverified_proof_grade,
            report.explanations.hidden_groups.unavailable_probe,
            report.explanations.hidden_groups.other_blockers
        ),
        format!("probe summary: {}", report.explanations.semantic_probes.summary),
        format!(
            "profile counts: mathlib={} internal={} api-design={} noise={}",
            report.profile_counts.mathlib,
            report.profile_counts.internal,
            report.profile_counts.api_design,
            report.profile_counts.noise
        ),
        format!("suppressed groups: {}", report.review.suppressed_count),
        format!("message: {}", report.message),
    ];
    if let Some(path) = &report.saved_baseline {
        lines.push(format!("saved baseline: {}", path.display()));
    }
    for group in report.visible_groups.iter().take(20) {
        let target = group
            .target_decl
            .as_deref()
            .map(|target| format!(" -> {target}"))
            .unwrap_or_default();
        lines.push(format!(
            "{}: {} {} {}{}",
            group.id, group.review_priority, group.recommended_action, group.relation, target
        ));
        if let Some(hint) = &group.replacement_hint {
            lines.push(format!(
                "  hint: import={} callers={} target_module={}",
                hint.import_status, hint.caller_count, hint.target_module
            ));
        }
        if !group.blockers.is_empty() {
            lines.push(format!("  blockers: {}", group.blockers.join(", ")));
        }
        lines.push(format!("  evidence mode: {}", group.evidence_mode));
    }
    lines.join("\n")
}

fn push_group_detail(lines: &mut Vec<String>, group: &ReviewGroupReport) {
    lines.push(format!("group: {}", group.id));
    lines.push(format!("priority: {}", group.review_priority));
    lines.push(format!("action: {}", group.recommended_action));
    lines.push(format!("relation: {}", group.relation));
    if let Some(target) = &group.target_decl {
        lines.push(format!("target: {target}"));
    }
    if let Some(module) = &group.target_module {
        lines.push(format!("target module: {module}"));
    }
    lines.push(format!("evidence mode: {}", group.evidence_mode));
    lines.push("members:".to_owned());
    for member in &group.members {
        let span = member
            .source_span
            .as_ref()
            .map(|span| format!(" {}:{}", span.file, span.start.line))
            .unwrap_or_default();
        lines.push(format!(
            "  {} {} {}{}",
            member.origin, member.kind, member.qualified_name, span
        ));
    }
    lines.push("evidence:".to_owned());
    for evidence in &group.evidence {
        lines.push(format!("  {}", evidence.summary));
    }
    if !group.signals.is_empty() {
        lines.push(format!("signals: {}", group.signals.join(", ")));
    }
    if !group.blockers.is_empty() {
        lines.push(format!("blockers: {}", group.blockers.join(", ")));
    } else {
        lines.push("blockers: none".to_owned());
    }
    if let Some(summary) = &group.probe_summary {
        lines.push(format!("probe: {summary}"));
    } else {
        lines.push("probe: no additional probe summary".to_owned());
    }
    if let Some(hint) = &group.replacement_hint {
        lines.push(format!("replacement: {}", hint.target_decl));
        lines.push(format!("import status: {}", hint.import_status));
        lines.push(format!("callers: {}", hint.caller_count));
        for caller in &hint.displayed_callers {
            lines.push(format!(
                "  caller {}:{}:{} {}",
                caller.file.display(),
                caller.line,
                caller.column,
                caller.text
            ));
        }
        if !hint.blockers.is_empty() {
            lines.push(format!("replacement blockers: {}", hint.blockers.join(", ")));
        }
        if !hint.notes.is_empty() {
            lines.push(format!("replacement notes: {}", hint.notes.join("; ")));
        }
    } else {
        lines.push("replacement: manual review".to_owned());
        lines.push(format!("callers: {}", group.local_caller_count));
    }
}

fn push_group_explanation(lines: &mut Vec<String>, explanation: &GroupExplanation) {
    lines.push("explanation:".to_owned());
    lines.push(format!("  visibility: {}", explanation.visibility));
    lines.push(format!("  why visible or hidden: {}", explanation.visibility_reason));
    lines.push(format!("  evidence mode: {}", explanation.evidence_mode));
    lines.push(format!("  static/proof evidence: {}", explanation.evidence_summary));
    lines.push(format!("  semantic evidence: {}", explanation.semantic_summary));
    lines.push(format!("  blockers: {}", explanation.blocker_summary));
    lines.push(format!(
        "  replacement/import/callers: {}",
        explanation.replacement_summary
    ));
}

fn review_profile_label(profile: ReviewProfile) -> &'static str {
    match profile {
        ReviewProfile::Mathlib => "mathlib",
        ReviewProfile::Internal => "internal",
        ReviewProfile::ApiDesign => "api-design",
        ReviewProfile::Noise => "noise",
    }
}
