# Evaluation Harness

The evaluation harness makes duplicate detection measurable. It scores observed candidate pairs against gold labels and
reports raw counts for recall, shown-queue precision, hard-negative leakage, candidate volume, runtime, and peak memory
when the platform exposes it.

For the current end-to-end architecture and production-gate status, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md) and
[evaluation/production-gates.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/evaluation/production-gates.md).

## Design Note

This layer owns label normalization, unordered pair identity, cluster-to-pair expansion, recall@k denominators,
shown-queue precision denominators, hard-negative policy, timing names, and memory metric names.

Its smallest public interface is:

- `score_run(labels, observed, k_values) -> EvaluationMetrics`;
- `lean-dup eval --suite <name> --format table|json`.

These decisions must not leak upward or sideways:

- fixture paths and KanProofs paths;
- JSON label file layout;
- retrieval weights, probe policy, and queue thresholds;
- SQLite handles and cache layout;
- table and JSON report formatting.

The preserved user-facing capability is measurable audit quality. Retrieval and later probe/ranking work can now be
checked by recall, shown-queue precision, candidate count, timing, and memory instead of report anecdotes.

Python-era behavior intentionally discarded:

- anecdotal inspection as the only regression signal;
- tuning against positives without hard negatives;
- heuristic scoring policy spread across candidate generation and rendering;
- global broad hydration as the main large-audit safety mechanism.

## Design It Twice

**Rejected: KanProofs-specific scorer.** The scorer would know KanProofs report paths, label names, and retrieval calls.
That design mixes a special corpus with the general metric definitions. It would be shallow because adding a new corpus
would require changing scoring code.

**Chosen: general scorer plus suite definitions.** The scorer consumes normalized labels and observed pairs. Suite code
loads fixture or slow corpus labels, runs indexes and retrieval, records timings, and decides which suites are default
or manual. This design is deeper because the scorer has a small stable interface, corpus paths do not leak into metric
calculation, and future audit outputs can reuse the same metric definitions.

## Public Behavior

`eval --suite default` runs the small fixture suite. It builds the local and external fixture workspaces, reuses
canonical indexes through the normal cache layer, runs retrieval, and prints a compact table by default. The default
suite is a quality gate: all gold positives must appear within recall@10 and no hard negative may enter the shown queue.

`kanproofs-internal` and `kanproofs-mathlib` are explicit slow suites. They use built-in labels from the confirmed
KanProofs reports and require existing compiled artifacts; they are not part of the default test suite.

All percentage-like metrics are reported as raw counts, such as `5/7`, so readers can see the denominator.

## Red Flag Review

- **Shallow module:** avoided by giving scoring one narrow operation with nontrivial label normalization and metric
    policy hidden behind it.
- **Pass-through wrapper:** avoided; the suite runner adds label loading, index orchestration, observation extraction,
    timing, memory sampling, and quality gates.
- **Temporal decomposition:** avoided by splitting modules around hidden knowledge: labels, scoring, suites, table
    rendering, and memory.
- **Information leakage:** avoided because fixture and KanProofs paths live in suite definitions and label files, not
    the scorer.
- **Special-general mixture:** avoided because KanProofs labels are named slow suites; the scorer has no KanProofs
    branches.
- **Conjoined methods:** avoided because scoring accepts a complete observed run and does not share retrieval state.
- **Hard-to-describe public API:** avoided; users run one named suite and get metrics.
- **Implementation details contaminating interface comments:** avoided by documenting caller-visible metric contracts,
    not SQLite layout, Lean traversal, or temporary migration details.
