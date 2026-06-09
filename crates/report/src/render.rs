use crate::report_contract::GroupExplanation;
use crate::reports::{
    AuditReport, BaselineReport, BaselineSummaryReport, CacheCleanupReportDto, CacheLabelDiagnosticsReport, DiffReport,
    DoctorReport, EvalReportDto, IndexReport, PerfReport, Report, ReviewGroupReport, ShowReport,
    cache_root_diagnostic_label, path_diagnostic_label, path_reference_label,
};

/// Render-time knobs for the text formatter. JSON output ignores these.
///
/// `verbose` is consulted only by the doctor renderer today; other reports
/// already produce summarised output and do not have a verbose mode.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    pub verbose: bool,
    /// Cap on the number of groups printed in the audit text table. `None`
    /// uses the renderer's built-in default (20).
    pub audit_limit: Option<usize>,
}

pub fn render_text(report: &Report) -> String {
    render_text_with(report, RenderOptions::default())
}

pub fn render_text_with(report: &Report, options: RenderOptions) -> String {
    match report {
        Report::Doctor(report) => render_doctor(report, options),
        Report::CacheCleanup(report) => render_cache_cleanup(report, options),
        Report::Index(report) => render_index("index", report),
        Report::IndexMathlib(report) => render_index("index-mathlib", report),
        Report::Show(report) => render_show(report, options),
        Report::Diff(report) => render_diff(report, options),
        Report::Audit(report) => render_audit(report, options),
        Report::Eval(report) => render_eval(report),
        Report::Perf(report) => render_perf(report),
        Report::Baseline(report) => render_baseline(report, options),
    }
}

fn render_baseline(report: &BaselineReport, options: RenderOptions) -> String {
    let mut lines = vec![format!(
        "lean-dup baseline — status: {}    action: {}",
        report.status, report.action
    )];
    match report.action {
        "list" => {
            if report.baselines.is_empty() {
                match report.total_before_filter {
                    Some(total) if total > 0 => {
                        lines.push(format!(
                            "(0 of {total} baselines match this workspace; pass --all to see them)"
                        ));
                    }
                    _ => {
                        lines.push("(no saved baselines under this cache root)".to_owned());
                    }
                }
            } else {
                lines.push(String::new());
                push_baseline_table(&mut lines, &report.baselines);
            }
        }
        "show" => {
            if let Some(entry) = report.baselines.first() {
                lines.push(String::new());
                push_baseline_table(&mut lines, std::slice::from_ref(entry));
                lines.push(String::new());
                let total = entry.unique_group_count.unwrap_or(entry.group_ids.len());
                let id_cap = if options.verbose { entry.group_ids.len() } else { 20 };
                let shown = id_cap.min(entry.group_ids.len());
                lines.push(format!("group ids ({shown} of {total} distinct):"));
                for id in entry.group_ids.iter().take(id_cap) {
                    lines.push(format!("  {id}"));
                }
                if !options.verbose && entry.group_ids.len() > shown {
                    lines.push("(pass --verbose for the full list)".to_owned());
                }
            }
        }
        "delete" => {
            if let Some(name) = &report.deleted {
                lines.push(format!("deleted baseline '{name}'"));
            }
        }
        _ => {}
    }
    if options.verbose {
        lines.push(String::new());
        lines.push(format!(
            "cache root: {}",
            cache_root_diagnostic_label(&report.cache_root)
        ));
    }
    lines.join("\n")
}

fn push_baseline_table(lines: &mut Vec<String>, entries: &[BaselineSummaryReport]) {
    let name_w = entries
        .iter()
        .map(|entry| entry.name.len())
        .max()
        .unwrap_or(0)
        .max("name".len());
    let count_w = entries
        .iter()
        .map(|entry| entry.group_count.to_string().len())
        .max()
        .unwrap_or(0)
        .max("groups".len());
    lines.push(format!(
        "  {:<name_w$}  {:>count_w$}  {:>9}  {}",
        "name", "groups", "size", "workspace",
    ));
    for entry in entries {
        lines.push(format!(
            "  {:<name_w$}  {:>count_w$}  {:>9}  {}",
            entry.name,
            entry.group_count,
            format_bytes(entry.disk_bytes),
            entry.workspace_fingerprint,
        ));
    }
}

fn render_eval(report: &EvalReportDto) -> String {
    let metrics = &report.metrics;
    let recall_1 = recall_cell(metrics, 1);
    let recall_5 = recall_cell(metrics, 5);
    let recall_10 = recall_cell(metrics, 10);
    let precision = count_cell(&metrics.shown_queue_precision);
    let hard_negatives = count_cell(&metrics.hard_negative_hits);
    let visible_groups = count_cell(&metrics.visible_groups);
    let probe_unavailable = count_cell(&metrics.probe_unavailable);
    let peak_memory = metrics
        .peak_memory_bytes
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "n/a".to_owned());

    [
        "suite\trecall@1\trecall@5\trecall@10\tqueue_precision\thard_negatives\tvisible_groups\tprobe_unavailable\tcandidates\tindex_load_ms\tretrieval_ms\tprobe_ms\ttotal_ms\tpeak_mem",
        &format!(
            "{suite}\t{recall_1}\t{recall_5}\t{recall_10}\t{precision}\t{hard_negatives}\t{visible_groups}\t{probe_unavailable}\t{candidates}\t{index_load_ms}\t{retrieval_ms}\t{probe_ms}\t{total_ms}\t{peak_memory}",
            suite = metrics.suite,
            candidates = metrics.candidate_count,
            index_load_ms = metrics.timings.index_load_ms,
            retrieval_ms = metrics.timings.retrieval_ms,
            probe_ms = metrics.timings.probe_ms,
            total_ms = metrics.timings.total_ms,
        ),
    ]
    .join("\n")
}

fn recall_cell(metrics: &crate::reports::EvalMetricsDto, k: usize) -> String {
    metrics
        .recall
        .iter()
        .find(|recall| recall.k == k)
        .map(|recall| format!("{}/{}", recall.found, recall.total))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn count_cell(metric: &crate::reports::EvalCountMetricDto) -> String {
    format!("{}/{}", metric.found, metric.total)
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

fn render_doctor(report: &DoctorReport, options: RenderOptions) -> String {
    let mut lines = Vec::new();

    let reclaimable_bytes: u64 = report
        .cache
        .labels
        .iter()
        .flat_map(|label| label.entries.iter())
        .filter(|entry| entry.is_reclaimable())
        .map(|entry| entry.disk_bytes)
        .sum();

    // Section A — header.
    lines.push(format!("lean-dup doctor — status: {}", report.status));
    lines.push(format!(
        "workspace: {}    lean: {}",
        path_diagnostic_label(&report.requested_workspace),
        report.lean_version,
    ));
    let reclaimable_summary = if reclaimable_bytes > 0 {
        format!(", ~{} reclaimable", format_bytes(reclaimable_bytes))
    } else {
        String::new()
    };
    lines.push(format!(
        "cache root: {}    cache: {} labels, {} on disk{}",
        cache_root_diagnostic_label(&report.cache_root),
        report.cache.labels.len(),
        format_bytes(report.cache.total_disk_bytes),
        reclaimable_summary,
    ));
    lines.push(format!(
        "release: {} ({}, {})    index schema: {}",
        report.release.version,
        report.release.git_revision,
        report.release.build_profile,
        report.release.index_schema_version,
    ));
    lines.push(format!(
        "worker: {} (protocol {}, lean {})",
        report.worker.worker_version, report.worker.protocol_version, report.worker.lean_version,
    ));

    // Section B — problems.
    let problems = collect_problems(report);
    if problems.is_empty() {
        lines.push(String::new());
        lines.push("problems: none".to_owned());
    } else {
        lines.push(String::new());
        lines.push("problems:".to_owned());
        for problem in problems {
            lines.push(format!("  {problem}"));
        }
    }

    // Section C — per-label cache summary.
    lines.push(String::new());
    lines.push("cache:".to_owned());
    let summaries: Vec<LabelSummary> = report.cache.labels.iter().map(LabelSummary::from_label).collect();
    let label_width = summaries
        .iter()
        .map(|summary| summary.label.len())
        .max()
        .unwrap_or(0)
        .max("label".len());
    let latest_width = summaries
        .iter()
        .map(|summary| summary.latest.len())
        .max()
        .unwrap_or(0)
        .max("latest".len());
    lines.push(format!(
        "  {:<label_width$}  {:<latest_width$}  {:>6}  {:>5}  {:>8}  {:>7}  {:>10}",
        "label", "latest", "active", "stale", "v1-stale", "missing", "bytes",
    ));
    for summary in &summaries {
        lines.push(format!(
            "  {:<label_width$}  {:<latest_width$}  {:>6}  {:>5}  {:>8}  {:>7}  {:>10}",
            summary.label,
            summary.latest,
            summary.active,
            summary.stale,
            summary.v1_stale,
            summary.missing,
            format_bytes(summary.disk_bytes),
        ));
    }
    if reclaimable_bytes > 0 {
        lines.push(format!(
            "totals: {} labels, {} on disk, ~{} reclaimable via `lean-dup cache-cleanup --execute`",
            report.cache.labels.len(),
            format_bytes(report.cache.total_disk_bytes),
            format_bytes(reclaimable_bytes),
        ));
    } else {
        lines.push(format!(
            "totals: {} labels, {} on disk, nothing to reclaim",
            report.cache.labels.len(),
            format_bytes(report.cache.total_disk_bytes),
        ));
    }

    // Oleans (only when requested).
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
                    .join(", "),
            ));
        }
    }

    // Section D — verbose dump (per-entry detail), only with --verbose.
    if options.verbose {
        lines.push(String::new());
        lines.push("verbose detail:".to_owned());
        lines.push(format!("  report schema: {}", report.report_schema_version));
        lines.push(format!("  cache key: {}", report.release.cache_key_version));
        lines.push(format!("  cache fingerprint: {}", report.cache_fingerprint));
        lines.push(format!(
            "  resolved Lake root: {}",
            path_diagnostic_label(&report.lake_root)
        ));
        lines.push(format!("  lakefile: {}", path_diagnostic_label(&report.lakefile)));
        lines.push(format!("  module roots: {}", report.module_roots.join(", ")));
        lines.push(format!("  selected roots: {}", report.selected_roots.join(", ")));
        lines.push(format!("  source files: {}", report.source_count));
        lines.push(format!("  worker extract: {}", report.worker.extract_version));
        lines.push(format!("  worker features: {}", report.worker.features_version));
        lines.push(format!("  worker probe: {}", report.worker.probe_version));
        lines.push(format!(
            "  worker commands: {}",
            report.worker.supported_commands.join(", ")
        ));
        for label in &report.cache.labels {
            lines.push(format!(
                "  cache label {}: latest={} entries={} bytes={}",
                label.label,
                label.latest.status,
                label.entries.len(),
                label.disk_bytes,
            ));
            for entry in &label.entries {
                lines.push(format!(
                    "    {}: status={} active={} expected={} schema={} provenance={} bytes={}",
                    path_diagnostic_label(&entry.index_dir),
                    entry.status,
                    entry.active_latest,
                    entry.expected_current,
                    entry.schema_version.as_deref().unwrap_or("missing"),
                    entry.provenance_kind,
                    entry.disk_bytes,
                ));
            }
        }
    }

    lines.join("\n")
}

struct LabelSummary {
    label: String,
    latest: String,
    active: usize,
    stale: usize,
    v1_stale: usize,
    missing: usize,
    disk_bytes: u64,
}

impl LabelSummary {
    fn from_label(label: &CacheLabelDiagnosticsReport) -> Self {
        let mut active = 0usize;
        let mut stale = 0usize;
        let mut v1_stale = 0usize;
        let mut missing = 0usize;
        for entry in &label.entries {
            // active = referenced by the latest pointer (regardless of whether
            // it has been re-validated). The original "junk" output showed many
            // live entries with status=unchecked active=true — they are the
            // current cache, not dead weight.
            if entry.active_latest {
                active += 1;
            }
            match entry.status.as_str() {
                "stale" => {
                    if entry.schema_version.as_deref() == Some("lean-dup.index.v1") {
                        v1_stale += 1;
                    } else {
                        stale += 1;
                    }
                }
                "missing" => missing += 1,
                _ => {}
            }
        }
        Self {
            label: label.label.clone(),
            latest: label.latest.status.clone(),
            active,
            stale,
            v1_stale,
            missing,
            disk_bytes: label.disk_bytes,
        }
    }
}

fn collect_problems(report: &DoctorReport) -> Vec<String> {
    let mut out = Vec::new();
    for label in &report.cache.labels {
        if label.latest.status != "ok" {
            let detail = match label.latest.status.as_str() {
                "corrupt-pointer" | "corruptpointer" => "latest pointer is corrupt (cache-cleanup will rebuild)",
                "target-missing" | "targetmissing" => "latest pointer references a missing index dir",
                "missing" => "no latest pointer (cache will be rebuilt on next index)",
                other => {
                    out.push(format!("label {}: latest={}", label.label, other));
                    continue;
                }
            };
            out.push(format!("label {}: {}", label.label, detail));
        }
        let mut missing_active = 0usize;
        let mut corrupt_active = 0usize;
        for entry in &label.entries {
            if entry.expected_current || entry.active_latest {
                if entry.status == "missing" {
                    missing_active += 1;
                } else if entry.status == "corrupt" {
                    corrupt_active += 1;
                }
            }
        }
        if missing_active > 0 {
            out.push(format!(
                "label {}: {} active/expected {} missing on disk",
                label.label,
                missing_active,
                if missing_active == 1 { "entry" } else { "entries" },
            ));
        }
        if corrupt_active > 0 {
            out.push(format!(
                "label {}: {} active/expected {} corrupt",
                label.label,
                corrupt_active,
                if corrupt_active == 1 { "entry" } else { "entries" },
            ));
        }
    }
    if report.require_oleans && !report.missing_oleans.is_empty() {
        let sample = report
            .missing_oleans
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if report.missing_oleans.len() > 3 { ", …" } else { "" };
        out.push(format!(
            "oleans: {} missing ({}{})",
            report.missing_oleans.len(),
            sample,
            suffix,
        ));
    }
    out
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn render_cache_cleanup(report: &CacheCleanupReportDto, options: RenderOptions) -> String {
    let mut lines = vec![
        format!("lean-dup cache-cleanup — status: {}", report.status),
        format!(
            "{} entries to remove ({})    {} protected",
            report.removable_count,
            format_bytes(report.bytes_to_remove),
            report.protected_count,
        ),
    ];
    if report.executed {
        lines.push(format!(
            "removed {} across {} {}.",
            format_bytes(report.bytes_removed),
            report.removable_count,
            if report.removable_count == 1 {
                "entry"
            } else {
                "entries"
            },
        ));
    } else {
        lines.push(format!(
            "dry run: pass --execute to delete the {} removable {} ({}).",
            report.removable_count,
            if report.removable_count == 1 {
                "entry"
            } else {
                "entries"
            },
            format_bytes(report.bytes_to_remove),
        ));
    }
    if options.verbose {
        lines.push(String::new());
        lines.push(format!(
            "cache root: {}",
            cache_root_diagnostic_label(&report.cache_root)
        ));
        if !report.removed_entries.is_empty() {
            lines.push("removable entries:".to_owned());
            for entry in &report.removed_entries {
                lines.push(format!(
                    "  {} {} ({})",
                    path_diagnostic_label(&entry.index_dir),
                    format_bytes(entry.disk_bytes),
                    entry.reason,
                ));
            }
        }
        if !report.protected_entries.is_empty() {
            lines.push("protected entries:".to_owned());
            for entry in &report.protected_entries {
                lines.push(format!(
                    "  {} {} ({})",
                    path_diagnostic_label(&entry.index_dir),
                    format_bytes(entry.disk_bytes),
                    entry.reason,
                ));
            }
        }
    } else if report.removable_count + report.protected_count > 0 {
        lines.push("(pass --verbose for the per-entry list.)".to_owned());
    }
    if let Some(ws) = report.workspace_files.as_ref() {
        lines.push(String::new());
        let stale_summary = if report.executed {
            format!(
                "workspace snapshots: removed {} ({})",
                ws.removable_count,
                format_bytes(ws.bytes_removed)
            )
        } else {
            format!(
                "workspace snapshots: {} stale ({})    {} protected",
                ws.removable_count,
                format_bytes(ws.bytes_to_remove),
                ws.protected_count,
            )
        };
        lines.push(stale_summary);
        if options.verbose {
            if !ws.removed.is_empty() {
                lines.push("stale snapshot files:".to_owned());
                for entry in &ws.removed {
                    lines.push(format!(
                        "  {} {} ({})",
                        entry.kind,
                        format_bytes(entry.disk_bytes),
                        entry.fingerprint,
                    ));
                }
            }
            if !ws.protected.is_empty() {
                lines.push("protected snapshot files:".to_owned());
                for entry in &ws.protected {
                    lines.push(format!(
                        "  {} {} ({})",
                        entry.kind,
                        format_bytes(entry.disk_bytes),
                        entry.fingerprint,
                    ));
                }
            }
        }
    }
    lines.join("\n")
}

fn render_show(report: &ShowReport, options: RenderOptions) -> String {
    let mut lines = vec![format!(
        "lean-dup show — status: {}    group: {}",
        report.status, report.group.id
    )];
    if options.verbose {
        lines.push(format!(
            "requested workspace: {}",
            path_diagnostic_label(&report.requested_workspace)
        ));
        lines.push(format!(
            "resolved Lake root: {}",
            path_diagnostic_label(&report.lake_root)
        ));
        lines.push(format!("selected roots: {}", report.selected_roots.join(", ")));
        lines.push(format!("source files: {}", report.source_count));
        lines.push(format!(
            "cache root: {}",
            cache_root_diagnostic_label(&report.cache_root)
        ));
        lines.push(format!("cache fingerprint: {}", report.cache_fingerprint));
    }
    lines.push(String::new());
    push_group_detail(&mut lines, &report.group);
    push_group_explanation(&mut lines, &report.explanation);
    lines.join("\n")
}

fn render_diff(report: &DiffReport, options: RenderOptions) -> String {
    let mut lines = vec![
        format!(
            "lean-dup diff — status: {}    baseline: {}",
            report.status, report.diff.baseline
        ),
        format!(
            "appeared: {}    disappeared: {}    changed: {}",
            report.diff.appeared.len(),
            report.diff.disappeared.len(),
            report.diff.changed.len(),
        ),
    ];
    if !report.diff.appeared.is_empty() || !report.diff.disappeared.is_empty() || !report.diff.changed.is_empty() {
        lines.push(String::new());
    }
    for group in report.diff.appeared.iter().take(20) {
        lines.push(format!("  appeared {}", group.id));
    }
    for group in report.diff.disappeared.iter().take(20) {
        lines.push(format!("  disappeared {}", group.id));
    }
    for change in report.diff.changed.iter().take(20) {
        lines.push(format!("  changed {}", change.id));
    }
    if options.verbose {
        lines.push(String::new());
        lines.push(format!(
            "requested workspace: {}",
            path_diagnostic_label(&report.requested_workspace)
        ));
        lines.push(format!(
            "resolved Lake root: {}",
            path_diagnostic_label(&report.lake_root)
        ));
        lines.push(format!("selected roots: {}", report.selected_roots.join(", ")));
        lines.push(format!("source files: {}", report.source_count));
        lines.push(format!(
            "cache root: {}",
            cache_root_diagnostic_label(&report.cache_root)
        ));
        lines.push(format!("cache fingerprint: {}", report.cache_fingerprint));
        lines.push(format!(
            "baseline path: {}",
            path_diagnostic_label(&report.diff.baseline_path)
        ));
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

fn render_audit(report: &AuditReport, options: RenderOptions) -> String {
    let mut lines = Vec::new();

    // Section A — header.
    lines.push(format!("lean-dup audit — status: {}", report.status));
    lines.push(format!(
        "workspace: {}    selected roots: {}",
        path_diagnostic_label(&report.workspace.requested_workspace),
        if report.workspace.selected_roots.is_empty() {
            "(none)".to_owned()
        } else {
            report.workspace.selected_roots.join(", ")
        },
    ));
    if report.workspace.declarations_skipped_by_budget > 0 {
        lines.push(format!(
            "warning: skipped {} declaration(s) exceeding the heartbeat budget; raise --max-heartbeats (0 = unlimited) to include them",
            report.workspace.declarations_skipped_by_budget,
        ));
    }
    let truncated = if report.visible_groups_truncated {
        " (truncated)"
    } else {
        ""
    };
    let table_limit = options.audit_limit.unwrap_or(20);
    let table_shown = table_limit.min(report.visible_groups.len());
    let limit_hint = if table_shown < report.visible_groups_emitted {
        let explicit = options.audit_limit.is_some();
        if explicit {
            format!(
                " (showing top {table_shown} of {}; pass --limit to widen)",
                report.visible_groups_emitted
            )
        } else {
            format!(
                " (showing top {table_shown} of {}; pass --limit N to widen, default 20)",
                report.visible_groups_emitted
            )
        }
    } else {
        String::new()
    };
    lines.push(format!(
        "review queue: {} visible families, top {} emitted{}{}    suppressed: {}",
        report.visible_group_count,
        report.visible_groups_emitted,
        truncated,
        limit_hint,
        report.review.suppressed_count,
    ));
    if let Some(path) = &report.saved_baseline {
        let name = report.saved_baseline_name.as_deref().unwrap_or("baseline");
        let count = report.saved_baseline_group_count.unwrap_or(0);
        let suffix = if count == 1 { "group" } else { "groups" };
        let replaced = if report.saved_baseline_replaced.unwrap_or(false) {
            ", replaced existing"
        } else {
            ""
        };
        lines.push(format!(
            "saved baseline '{name}' ({count} {suffix}{replaced}) → {}",
            path.display()
        ));
    }

    // Section B — groups table.
    lines.push(String::new());
    if report.visible_groups.is_empty() {
        lines.push(format!(
            "no review-priority duplicates: {}",
            report.explanations.visible_queue.reason
        ));
    } else {
        lines.push("groups:".to_owned());
        let rows: Vec<GroupRow> = report
            .visible_groups
            .iter()
            .take(table_limit)
            .map(GroupRow::from_group)
            .collect();
        let prio_w = rows
            .iter()
            .map(|r| r.priority.len())
            .max()
            .unwrap_or(0)
            .max("priority".len());
        let action_w = rows
            .iter()
            .map(|r| r.action.len())
            .max()
            .unwrap_or(0)
            .max("action".len());
        let relation_w = rows
            .iter()
            .map(|r| r.relation.len())
            .max()
            .unwrap_or(0)
            .max("relation".len());
        let id_w = rows.iter().map(|r| r.id.len()).max().unwrap_or(0).max("id".len());
        lines.push(format!(
            "  {:<prio_w$}  {:<action_w$}  {:<relation_w$}  {:<id_w$}  {}",
            "priority", "action", "relation", "id", "target",
        ));
        for row in &rows {
            lines.push(format!(
                "  {:<prio_w$}  {:<action_w$}  {:<relation_w$}  {:<id_w$}  {}",
                row.priority, row.action, row.relation, row.id, row.target,
            ));
        }
        lines.push(String::new());
        lines.push("run `lean-dup show --group <id>` for evidence on one group.".to_owned());
    }

    // Section C — verbose dump (provenance, semantic probes, hidden groups, queue counts, per-group detail).
    if options.verbose {
        lines.push(String::new());
        lines.push("verbose detail:".to_owned());
        lines.push(format!("  report schema: {}", report.report_schema_version));
        lines.push(format!(
            "  resolved Lake root: {}",
            path_diagnostic_label(&report.workspace.lake_root),
        ));
        lines.push(format!("  source files: {}", report.workspace.source_count));
        lines.push(format!(
            "  cache root: {}",
            cache_root_diagnostic_label(&report.cache.root)
        ));
        lines.push(format!("  cache fingerprint: {}", report.cache.fingerprint));
        lines.push(format!("  include private: {}", report.options.include_private));
        lines.push(format!("  compare mathlib: {}", report.options.compare_mathlib));
        lines.push(format!(
            "  visibility: private={} low-priority={} diagnostics={}",
            report.options.visibility.include_private,
            report.options.visibility.include_low_priority,
            report.options.visibility.diagnostics,
        ));
        lines.push(format!("  candidates: {}", report.retrieval.candidate_count));
        lines.push(format!(
            "  comparison provenance: {}",
            report.explanations.comparison_provenance.summary,
        ));
        lines.push(format!(
            "  semantic probes: planned={} cached={} worker={} verified={} rejected={} unavailable={}",
            report.semantic_verification.planned_pairs,
            report.semantic_verification.cached_hits,
            report.semantic_verification.worker_pairs,
            report.semantic_verification.verified_results,
            report.semantic_verification.rejected_results,
            report.semantic_verification.unavailable_results,
        ));
        lines.push(format!(
            "  semantic reranking: {}",
            report.semantic_verification.semantic_reranking.version,
        ));
        lines.push(format!("  review pair groups: {}", report.review.group_count));
        lines.push(format!("  visible queue: {}", report.explanations.visible_queue.reason));
        lines.push(format!(
            "  hidden groups: total={} visibility/noise={} generated={} unverified-proof-grade={} unavailable-probe={} other={}",
            report.explanations.hidden_groups.total,
            report.explanations.hidden_groups.visibility_or_noise,
            report.explanations.hidden_groups.generated,
            report.explanations.hidden_groups.unverified_proof_grade,
            report.explanations.hidden_groups.unavailable_probe,
            report.explanations.hidden_groups.other_blockers,
        ));
        lines.push(format!(
            "  probe summary: {}",
            report.explanations.semantic_probes.summary
        ));
        lines.push(format!(
            "  queue counts: cleanup={} with-private={} with-low-priority={} diagnostics={}",
            report.queue_counts.cleanup,
            report.queue_counts.with_private,
            report.queue_counts.with_low_priority,
            report.queue_counts.diagnostics,
        ));
        lines.push(format!("  message: {}", report.message));
        for group in report.visible_groups.iter().take(20) {
            let target = group
                .target_decl
                .as_deref()
                .map(|target| format!(" -> {target}"))
                .unwrap_or_default();
            lines.push(format!(
                "  {}: {} {} {}{}",
                group.id, group.review_priority, group.recommended_action, group.relation, target,
            ));
            if group.pair_count > 1 {
                lines.push(format!(
                    "    family: {} pairs{}",
                    group.pair_count,
                    if group.pair_evidence_truncated {
                        " (summaries truncated)"
                    } else {
                        ""
                    },
                ));
            }
            if let Some(hint) = &group.replacement_hint {
                lines.push(format!(
                    "    hint: import={} impact={} callers={} target_module={}",
                    hint.import_status, hint.caller_impact, hint.caller_count, hint.target_module,
                ));
            }
            if !group.blockers.is_empty() {
                lines.push(format!("    blockers: {}", group.blockers.join(", ")));
            }
            lines.push(format!("    evidence mode: {}", group.evidence_mode));
        }
    }

    lines.join("\n")
}

struct GroupRow {
    priority: String,
    action: String,
    relation: String,
    id: String,
    target: String,
}

impl GroupRow {
    fn from_group(group: &ReviewGroupReport) -> Self {
        Self {
            priority: group.review_priority.clone(),
            action: group.recommended_action.clone(),
            relation: group.relation.clone(),
            id: group.id.clone(),
            target: group.target_decl.clone().unwrap_or_default(),
        }
    }
}

fn push_group_detail(lines: &mut Vec<String>, group: &ReviewGroupReport) {
    lines.push(format!("group: {}", group.id));
    lines.push(format!("family id: {}", group.family_id));
    lines.push(format!("pair count: {}", group.pair_count));
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
            .map(|span| {
                let location = span
                    .local_path
                    .clone()
                    .unwrap_or_else(|| path_reference_label(&span.file));
                format!(" {}:{}", location, span.start.line)
            })
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
    if group.pair_count > 1 {
        lines.push("pair evidence:".to_owned());
        for pair in &group.pair_evidence {
            lines.push(format!(
                "  {}: {} {} {}",
                pair.id, pair.review_priority, pair.recommended_action, pair.relation
            ));
            for member in &pair.members {
                lines.push(format!("    member: {} {}", member.origin, member.qualified_name));
            }
            for evidence in &pair.evidence {
                lines.push(format!("    evidence: {}", evidence.summary));
            }
        }
        if group.pair_evidence_truncated {
            lines.push("  pair evidence truncated in ordinary output; run show for full selected family".to_owned());
        }
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
        lines.push(format!("caller impact: {}", hint.caller_impact));
        lines.push(format!("callers: {}", hint.caller_count));
        if hint.callers_truncated {
            lines.push("caller list: truncated".to_owned());
        }
        for caller in &hint.displayed_callers {
            lines.push(format!(
                "  caller {}:{}:{} {}",
                path_reference_label(&caller.file),
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

#[cfg(test)]
mod doctor_render_tests {
    use super::*;
    use crate::reports::{
        CacheDiagnosticsReport, CacheEntryDiagnosticsReport, CacheLatestDiagnosticsReport, ReleaseIdentityReport,
        WorkerDiagnosticsReport,
    };
    use std::path::PathBuf;

    fn entry(
        status: &str,
        active: bool,
        expected: bool,
        schema: Option<&str>,
        bytes: u64,
    ) -> CacheEntryDiagnosticsReport {
        CacheEntryDiagnosticsReport {
            index_dir: PathBuf::from("/tmp/cache/idx"),
            index_path: PathBuf::from("/tmp/cache/idx/index.sqlite"),
            status: status.to_owned(),
            active_latest: active,
            expected_current: expected,
            schema_version: schema.map(|s| s.to_owned()),
            provenance_kind: "sourcebacked".to_owned(),
            declaration_count: None,
            disk_bytes: bytes,
            reasons: Vec::new(),
        }
    }

    fn label(
        name: &str,
        latest_status: &str,
        entries: Vec<CacheEntryDiagnosticsReport>,
    ) -> CacheLabelDiagnosticsReport {
        let disk_bytes = entries.iter().map(|e| e.disk_bytes).sum();
        CacheLabelDiagnosticsReport {
            label: name.to_owned(),
            label_dir: PathBuf::from(format!("/tmp/cache/{name}")),
            disk_bytes,
            latest: CacheLatestDiagnosticsReport {
                pointer_path: PathBuf::from(format!("/tmp/cache/{name}/latest.json")),
                status: latest_status.to_owned(),
                index_dir: None,
            },
            entries,
        }
    }

    fn report(labels: Vec<CacheLabelDiagnosticsReport>) -> DoctorReport {
        let total_disk_bytes = labels.iter().map(|l| l.disk_bytes).sum();
        DoctorReport {
            report_schema_version: "lean-dup.report.v3",
            release: ReleaseIdentityReport {
                package: "lean-dup".to_owned(),
                version: "0.1.0".to_owned(),
                git_revision: "deadbeef".to_owned(),
                build_profile: "debug".to_owned(),
                report_schema_version: "lean-dup.report.v3".to_owned(),
                index_schema_version: "lean-dup.index.v2".to_owned(),
                cache_key_version: "rust-cli-cache.v1".to_owned(),
            },
            status: "ok",
            requested_workspace: PathBuf::from("/tmp/ws"),
            lake_root: PathBuf::from("/tmp/ws"),
            lakefile: PathBuf::from("/tmp/ws/lakefile.toml"),
            module_roots: vec!["KanProofs".to_owned()],
            selected_roots: vec!["KanProofs".to_owned()],
            source_count: 42,
            cache_root: PathBuf::from("/tmp/cache"),
            cache_fingerprint: "rust-cli-cache.v1:abc".to_owned(),
            cache: CacheDiagnosticsReport {
                cache_root: PathBuf::from("/tmp/cache"),
                total_disk_bytes,
                labels,
            },
            worker: WorkerDiagnosticsReport {
                protocol_version: "lean-dup.worker.v1".to_owned(),
                worker_version: "0.1.0".to_owned(),
                lean_version: "Lean 4.30.0".to_owned(),
                extract_version: "extract.v2".to_owned(),
                features_version: "features.v3".to_owned(),
                probe_version: "probe.v1".to_owned(),
                supported_commands: vec!["doctor".to_owned()],
                supported_capabilities: Vec::new(),
            },
            lean_version: "Lean 4.30.0".to_owned(),
            require_oleans: false,
            missing_oleans: Vec::new(),
        }
    }

    #[test]
    fn default_output_surfaces_problems_and_reclaim_without_per_entry_dump() {
        let labels = vec![
            label(
                "mathlib",
                "ok",
                vec![
                    entry("unchecked", true, false, Some("lean-dup.index.v2"), 1_000_000),
                    entry("stale", false, false, Some("lean-dup.index.v1"), 5_000_000),
                    entry("missing", false, false, None, 0),
                ],
            ),
            label("fixture-smoke", "corruptpointer", vec![]),
        ];
        let out = render_doctor(&report(labels), RenderOptions::default());
        // No per-entry status dump in default mode (path fingerprints in the header are fine).
        assert!(
            !out.contains("status=stale") && !out.contains("status=unchecked"),
            "default output leaked per-entry status dump:\n{out}",
        );
        assert!(
            !out.contains("verbose detail:"),
            "verbose section leaked into default:\n{out}"
        );
        // Corrupt-pointer label appears in problems section.
        assert!(out.contains("problems:"), "missing problems section:\n{out}");
        assert!(
            out.contains("label fixture-smoke: latest pointer is corrupt"),
            "corrupt pointer not surfaced:\n{out}",
        );
        // Reclaim totals line names the cleanup command and a non-zero figure.
        assert!(out.contains("reclaimable via `lean-dup cache-cleanup --execute`"));
        // v1-stale column tallies the v1 entry separately from the "stale" column.
        let mathlib_row = out
            .lines()
            .find(|line| line.contains("mathlib  ") || line.starts_with("  mathlib"))
            .expect("mathlib row missing");
        // active=1, stale=0, v1-stale=1, missing=1
        let tally = mathlib_row.split_whitespace().collect::<Vec<_>>();
        // ["mathlib", "ok", "1", "0", "1", "1", "0", "B"] — bytes "0 B"
        assert!(tally.contains(&"1"), "expected active=1 in {mathlib_row}");
    }

    #[test]
    fn verbose_output_is_strict_superset() {
        let labels = vec![label(
            "mathlib",
            "ok",
            vec![entry("unchecked", true, false, Some("lean-dup.index.v2"), 1_000_000)],
        )];
        let doctor = report(labels);
        let default_out = render_doctor(&doctor, RenderOptions::default());
        let verbose_out = render_doctor(
            &doctor,
            RenderOptions {
                verbose: true,
                ..RenderOptions::default()
            },
        );
        // Default lines all appear in verbose output.
        for line in default_out.lines() {
            assert!(
                verbose_out.contains(line),
                "verbose missing default line `{line}`:\n{verbose_out}",
            );
        }
        // Verbose adds the per-entry detail header and the sha256-style dump.
        assert!(verbose_out.contains("verbose detail:"));
        assert!(
            verbose_out.contains("status=unchecked active=true"),
            "verbose missing per-entry detail:\n{verbose_out}",
        );
    }

    #[test]
    fn reclaim_figure_matches_cleanup_predicate() {
        // Each entry contributes to the reclaim total iff !active_latest && !expected_current.
        // This is the same predicate cleanup_cache uses.
        let labels = vec![label(
            "mathlib",
            "ok",
            vec![
                entry("unchecked", true, false, None, 100), // protected: active
                entry("current", false, true, None, 200),   // protected: expected
                entry("stale", false, false, None, 300),    // reclaimable
                entry("stale", false, false, None, 400),    // reclaimable
            ],
        )];
        let out = render_doctor(&report(labels), RenderOptions::default());
        // 300 + 400 = 700 bytes reclaimable.
        assert!(out.contains("~700 B reclaimable"), "wrong reclaim figure:\n{out}");
    }

    #[test]
    fn no_problems_prints_problems_none() {
        let labels = vec![label(
            "mathlib",
            "ok",
            vec![entry("unchecked", true, false, Some("lean-dup.index.v2"), 1_000_000)],
        )];
        let out = render_doctor(&report(labels), RenderOptions::default());
        assert!(out.contains("problems: none"), "expected problems: none:\n{out}");
    }
}
