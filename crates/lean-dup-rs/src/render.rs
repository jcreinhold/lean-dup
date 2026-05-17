use std::io::Write;

use crate::cli::{OutputFormat, ReviewProfile};
use crate::commands::{AuditReport, DiffReport, DoctorReport, IndexReport, Outcome, Report, ShowReport};
use crate::error::Result;
use crate::progress::Reporter;
use crate::ranking::{RankedGroup, ReviewAction, ReviewPriority, ReviewRelation};

pub(crate) fn write_outcome<O: Write, E: Write>(outcome: Outcome, stdout: &mut O, stderr: &mut E) -> Result<()> {
    write_report(&outcome.reporter, stderr)?;
    match outcome.output_format {
        OutputFormat::Json => {
            writeln!(stdout, "{}", serde_json::to_string_pretty(&outcome.report)?)?;
        }
        OutputFormat::Text => {
            writeln!(stdout, "{}", render_text(&outcome.report))?;
        }
    }
    Ok(())
}

fn write_report<E: Write>(reporter: &Reporter, stderr: &mut E) -> Result<()> {
    for event in reporter.events() {
        let count = match (event.current, event.total) {
            (Some(current), Some(total)) => format!(" {current}/{total}"),
            (Some(current), None) => format!(" {current}"),
            _ => String::new(),
        };
        writeln!(
            stderr,
            "progress.{phase}{count}: {message} ({elapsed_ms}ms)",
            phase = event.phase,
            message = event.message,
            elapsed_ms = event.elapsed_ms
        )?;
    }
    for timing in reporter.timings() {
        writeln!(
            stderr,
            "profile.{phase}={elapsed_ms}ms",
            phase = timing.phase,
            elapsed_ms = timing.elapsed_ms
        )?;
    }
    Ok(())
}

fn render_text(report: &Report) -> String {
    match report {
        Report::Doctor(report) => render_doctor(report),
        Report::Index(report) => render_index("index", report),
        Report::IndexMathlib(report) => render_index("index-mathlib", report),
        Report::Show(report) => render_show(report),
        Report::Diff(report) => render_diff(report),
        Report::Audit(report) => render_audit(report),
        Report::Eval(report) => crate::eval::table::render_metrics(&report.metrics),
    }
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
        format!("lean: {}", report.lean_version),
    ];
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
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
        format!("include private: {}", report.include_private),
        format!("include imports: {}", report.include_imports),
        format!("compare mathlib: {}", report.compare_mathlib),
        format!("review profile: {}", review_profile_label(report.review_profile)),
        format!("threshold: {}", report.threshold),
        format!("candidates: {}", report.retrieval.candidate_count),
        format!("review groups: {}", report.review.groups.len()),
        format!("visible groups: {}", report.visible_group_count),
        format!(
            "profile counts: mathlib={} internal={} api-design={} noise={}",
            report.profile_counts.mathlib,
            report.profile_counts.internal,
            report.profile_counts.api_design,
            report.profile_counts.noise
        ),
        format!("suppressed groups: {}", report.review.suppressed.len()),
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
            group.id,
            priority_label(group.review_priority),
            action_label(group.recommended_action),
            relation_label(group.relation),
            target
        ));
        if let Some(hint) = &group.replacement_hint {
            lines.push(format!(
                "  hint: import={:?} callers={} target_module={}",
                hint.import_status, hint.caller_count, hint.target_module
            ));
        }
        if !group.blockers.is_empty() {
            lines.push(format!("  blockers: {}", group.blockers.join(", ")));
        }
    }
    lines.join("\n")
}

fn push_group_detail(lines: &mut Vec<String>, group: &RankedGroup) {
    lines.push(format!("group: {}", group.id));
    lines.push(format!("priority: {}", priority_label(group.review_priority)));
    lines.push(format!("action: {}", action_label(group.recommended_action)));
    lines.push(format!("relation: {}", relation_label(group.relation)));
    if let Some(target) = &group.target_decl {
        lines.push(format!("target: {target}"));
    }
    if let Some(module) = &group.target_module {
        lines.push(format!("target module: {module}"));
    }
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
        lines.push(format!("  {}", evidence.summary()));
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
        lines.push(format!("import status: {:?}", hint.import_status).to_ascii_lowercase());
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

fn priority_label(priority: ReviewPriority) -> &'static str {
    match priority {
        ReviewPriority::High => "high",
        ReviewPriority::Medium => "medium",
        ReviewPriority::Low => "low",
        ReviewPriority::Noise => "noise",
    }
}

fn review_profile_label(profile: ReviewProfile) -> &'static str {
    match profile {
        ReviewProfile::Mathlib => "mathlib",
        ReviewProfile::Internal => "internal",
        ReviewProfile::ApiDesign => "api-design",
        ReviewProfile::Noise => "noise",
    }
}

fn action_label(action: ReviewAction) -> &'static str {
    match action {
        ReviewAction::AlreadyInMathlib => "already-in-mathlib",
        ReviewAction::LocalAlias => "local-alias",
        ReviewAction::ReplaceLocalUses => "replace-local-uses",
        ReviewAction::MergeGeneralization => "merge-generalization",
        ReviewAction::SpecializationOf => "specialization-of",
        ReviewAction::ProbableSourceClone => "probable-source-clone",
        ReviewAction::ManualReview => "manual-review",
    }
}

fn relation_label(relation: ReviewRelation) -> &'static str {
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
