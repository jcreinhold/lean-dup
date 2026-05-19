# External Comparison Provenance

Every external comparison index has one of three evidence modes. Ranking, semantic verification, and reporting all
consume this mode; nothing else gates how strongly a non-workspace match can be claimed.

For the audit flow that uses this boundary, see
[end-to-end-architecture.md](end-to-end-architecture.md).

## Evidence modes

| Mode | Meaning | What ranking/probing may do |
| --- | --- | --- |
| `proof-grade` | source-backed; built in the same Lake execution root as the current audit, so probes can import both sides | run Lean probes; visible findings may require verified evidence |
| `source-backed-not-importable` | has source provenance but the execution root differs from the current audit | retrieval and static ranking; report the reason; no probe |
| `static` | no source provenance (old cache artifact, or intentionally static external) | static evidence may support suggestions but is reported as static, never as verified |

Missing provenance is treated as `static`, not fatal. Old caches stay readable and a separate migration step is not
required to keep auditing.

## Public surface

- `--compare-mathlib` is always project-centered and source-backed. The index is built from the audited project's
  pinned `.lake/packages/mathlib`, and probes execute from the audited project Lake root. When the cache is current,
  the mode is `proof-grade`.
- `--compare-index <label>` is a named external-index lookup; its mode is decided by the index's provenance, not by
  its label. A static index named `mathlib` cannot silently claim proof-grade.

Audit JSON includes a comparison provenance record per index with `label`, `origin`, `evidence_mode`, `source_root`,
`execution_root`, `execution_policy`, `declaration_count`, and a human-readable reason. Text reports include a compact
provenance summary. Ranked groups carry the same evidence mode.

The JSON shape is diagnostic; the stable JSON contract lives in
[report-contract.md](report-contract.md).

## Why a typed mode, not a label

The tempting design was to drive proof policy from labels and origins: treat `mathlib` as implying importable Lean
declarations. Labels are user workflow names, not provenance. That design leaks string conventions sideways into
ranking and probing, where every caller has to remember which strings imply what.

A private resolver instead maps each opened index to one evidence mode for the current audit. Ranking consumes the
mode and does not inspect SQLite keys, source-root comparison rules, or worker module construction. Semantic
verification builds worker module descriptors only for `proof-grade` origins; source roots and import policy stay
inside the verifier/provenance boundary.
