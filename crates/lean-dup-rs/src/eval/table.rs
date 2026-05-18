use crate::eval::scoring::{CountMetric, EvaluationMetrics};

pub(crate) fn render_metrics(metrics: &EvaluationMetrics) -> String {
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

fn recall_cell(metrics: &EvaluationMetrics, k: usize) -> String {
    metrics
        .recall
        .iter()
        .find(|recall| recall.k == k)
        .map(|recall| format!("{}/{}", recall.found, recall.total))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn count_cell(metric: &CountMetric) -> String {
    format!("{}/{}", metric.found, metric.total)
}
