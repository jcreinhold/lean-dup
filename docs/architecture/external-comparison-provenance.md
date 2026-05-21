# External Comparison Provenance

## Design note

This document owns the release-facing meaning of external comparison evidence. `lean-dup-index`
owns how provenance is stored and resolved; `lean-dup-search` owns how the resolved evidence mode
affects ranking and semantic probes; `lean-dup-report` owns the compact projection. The smallest
public interface is `label`, `origin`, `evidence_mode`, `declaration_count`, and `reason`.

Source roots, execution roots, index paths, storage layout, and worker module-descriptor
construction must not leak into audit JSON or text summaries. The preserved user-facing capability
is comparing the audited workspace with a named external index or project-pinned mathlib evidence.
The Python-era behavior intentionally discarded here is treating labels or old path-shaped artifacts
as proof-grade evidence.

## Design it twice

Three designs were considered:

- expose source roots, execution roots, and index paths so operators can debug provenance manually;
- infer proof-grade status from labels such as `mathlib`;
- make the index layer resolve one stable evidence mode and let search/report consume only that
  mode plus a concise reason.

The third design is deeper. It keeps storage and path comparison rules under the index boundary,
prevents string labels from becoming hidden policy, and gives report consumers a small, hard-to-misuse
interface.

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

Audit JSON includes a comparison provenance record per index with `label`, `origin`, `evidence_mode`,
`declaration_count`, and a human-readable reason. Text reports include a compact provenance summary. Ranked groups
carry the same evidence mode. JSON and text do not expose source roots, execution roots, index paths, cache paths, or
worker import descriptors.

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

## Prompt 57 validation

Validation artifacts were generated under `target/cache/provenance-artifacts/` as redacted summaries:

| Workload | Artifact | Evidence mode | Declarations | Probe behavior | Result |
| --- | --- | --- | ---: | --- | --- |
| source-backed fixture, same Lake execution root | `source-backed-summary.json` | `proof-grade` | 1 | 1 planned, 1 verified | visible proof-grade group |
| source-backed fixture, different Lake execution root | `not-importable-summary.json` | `source-backed-not-importable` | 8 | probes disabled for this static-use run | visible groups remain non-proof-grade |
| provenance metadata removed from fixture index | `static-summary.json` | `static` | 8 | probes disabled for this static-use run | static evidence is not upgraded by label |

Leak check:

```sh
rg -n '/Users|index\.sqlite|latest\.json|metadata|postings|SQLite|sqlite|worker row|target/cache' \
  target/cache/provenance-artifacts/*-summary.json target/cache/doctor-production.json
```

returned no matches. The full ordinary audit report still contains local source-reference details for review groups;
the release evidence for this gate uses the redacted provenance summaries above.
