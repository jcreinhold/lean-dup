use std::io::Write;

use crate::cli::OutputFormat;
use crate::commands::{AuditReport, DoctorReport, IndexReport, Outcome, Report, SkeletonReport};
use crate::error::Result;
use crate::progress::Reporter;
use crate::ranking::{ReviewAction, ReviewFilter, ReviewPriority, ReviewRelation};

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
        Report::Show(report) | Report::Diff(report) => render_skeleton(report),
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

fn render_skeleton(report: &SkeletonReport) -> String {
    let mut lines = vec![
        format!("status: {}", report.status),
        format!("requested workspace: {}", report.requested_workspace.display()),
        format!("resolved Lake root: {}", report.lake_root.display()),
        format!("selected roots: {}", report.selected_roots.join(", ")),
        format!("source files: {}", report.source_count),
        format!("cache root: {}", report.cache_root.display()),
        format!("cache fingerprint: {}", report.cache_fingerprint),
    ];
    if let Some(label) = &report.label {
        lines.push(format!("label: {label}"));
    }
    if let Some(group) = &report.group {
        lines.push(format!("group: {group}"));
    }
    if let Some(baseline) = &report.baseline {
        lines.push(format!("baseline: {}", baseline.display()));
    }
    if report.force {
        lines.push("force: true".to_owned());
    }
    lines.push(format!("message: {}", report.message));
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
        format!("threshold: {}", report.threshold),
        format!("candidates: {}", report.retrieval.candidate_count),
        format!("review groups: {}", report.review.groups.len()),
        format!("visible groups: {}", report.visible_group_count),
        format!("suppressed groups: {}", report.review.suppressed.len()),
        format!("message: {}", report.message),
    ];
    let filter = ReviewFilter {
        include_generated: report.include_generated,
        show_noise: report.show_noise,
        min_priority: report.min_priority,
    };
    for group in report.review.visible_groups(filter).into_iter().take(20) {
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

fn priority_label(priority: ReviewPriority) -> &'static str {
    match priority {
        ReviewPriority::High => "high",
        ReviewPriority::Medium => "medium",
        ReviewPriority::Low => "low",
        ReviewPriority::Noise => "noise",
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
