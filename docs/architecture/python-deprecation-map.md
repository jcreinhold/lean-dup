# Python Deprecation Map

Prompt 27 retires the Python-era implementation after Rust/Lean parity evidence exists. This document records what was
removed, what replaced it, which capability was preserved, and which design mistake was eliminated.

For the current Rust/Lean-only architecture, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md). This
document closes `G9 python_deprecation`; it does not close the remaining quality and release-hardening gates.

## Design Note

This document owns the hidden knowledge for the switchover: the Python module inventory, the Rust/Lean replacement map,
the parity artifacts, the remaining quality debt, and the rule that Python is evidence rather than architecture.

Its smallest public interface is this deletion map plus the Rust-first README. Later prompts should update production
validation status, but they should not reintroduce Python compatibility shells or treat Python cache layout as a release
contract.

These decisions must not leak upward or sideways:

- Python cache paths, module names, and JSON helper shapes;
- Python worker scripts and stdout assumptions;
- Python scoring thresholds, source-parsing fallbacks, and cache invalidation rules;
- temporary prompt sequencing mechanics.

The preserved user-facing capability is read-only duplicate auditing through the Rust `lean-dup-rs` binary: workspace
indexing, project-pinned mathlib indexing, external comparison, semantic evidence, evaluation gates, report rendering,
`show`, `doctor`, and cache diagnostics.

Python-era behavior intentionally discarded:

- Python as the installed production entry point;
- Python-side semantic policy over Lean statements;
- Python source parsing as a fallback for facts Lean owns;
- Python cache/index layout as a compatibility target;
- Python tests that parse worker stdout as data rows while ignoring progress events.

## Design It Twice

**Rejected: retain a Python compatibility shell.** A wrapper that forwards to the Rust binary would keep an obsolete
entry point alive without hiding useful complexity. It would make docs, packaging, tests, and support burden depend on
two command surfaces while only one implementation is production-grade.

**Chosen: Rust-only production surface with a deletion map.** Python lessons are preserved as Rust fixtures, eval labels,
architecture reports, and hard negatives. The public command surface is the Rust binary. This is deeper because users
learn one tool, while the historical implementation details stay in the release evidence rather than the interface.

## Parity Evidence

Fresh Prompt 27 artifacts:

| Command | Artifact | Result |
| --- | --- | --- |
| `cargo run -p lean-dup-rs -- eval --suite default --format json --output target/eval/prompt27-default.json` | `target/eval/prompt27-default.json` | `status = ok`, recall@10 `14/14`, hard-negative hits `0/4`. |
| `cargo run -p lean-dup-rs -- eval --suite hard-negatives --format json --output target/eval/prompt27-hard-negatives.json` | `target/eval/prompt27-hard-negatives.json` | `status = ok`, hard-negative hits `0/5`. |
| `cargo run -p lean-dup-rs -- eval --suite production-gate --format json --output target/eval/prompt27-production-gate.json` | `target/eval/prompt27-production-gate.json` | `status = ok`; manual KanProofs mathlib no longer fails the old 60 second worker timeout. |

The production-gate artifact still reports a precision problem in the manual KanProofs mathlib child:
`hard_negative_hits = 3/4`. That is not a Python switchover blocker because the Rust gate now completes, but it remains
production quality debt for prompts 29 and 30. Python deletion does not close `G2 precision_control`.

The stale Python worker tests were not retained. They expected worker stdout to contain only row payloads; the production
worker protocol now streams progress events and data events through JSONL. Rust protocol and CLI tests cover the current
contract.

## Deletion Map

| Removed Python path | Rust/Lean replacement | Validated capability preserved | Design mistake eliminated |
| --- | --- | --- | --- |
| `src/lean_dup/cli.py`, `__main__.py`, `__init__.py` | `crates/lean-dup-rs/src/cli.rs`, `commands.rs`, `render.rs` | `doctor`, `index`, `index-mathlib`, `audit`, `eval`, `show`, `diff`, progress/profile-safe output. | Two production command surfaces and Python entry-point drift. |
| `src/lean_dup/workspace.py` | `crates/lean-dup-rs/src/workspace.rs`, `mathlib.rs`, `cache.rs` | Lake workspace resolution and project-pinned mathlib resolution. | Workspace and package layout knowledge scattered through Python audit code. |
| `src/lean_dup/extractor.py`, `lean_runtime/Extractor.lean` | `lean/LeanDup/Worker.lean`, Rust `worker` and `index` modules | Lean-owned declaration extraction with typed protocol rows and cached indexes. | Source parsing and JSON/string-driven semantic facts in Python. |
| `src/lean_dup/features.py`, `matching.py`, `text.py` | Lean worker feature extraction plus Rust `retrieval.rs` | Canonical fingerprints, role features, low-signal markers, candidate retrieval. | Rust/Python recomputation of Lean semantic structure from text. |
| `src/lean_dup/external_index.py` | `crates/lean-dup-rs/src/index.rs`, `external_provenance.rs`, `cache_lifecycle.rs` | SQLite indexes, shared mathlib cache, source-backed/static provenance, doctor diagnostics. | Cache and SQLite policy leaking into audit workflow. |
| `src/lean_dup/candidates.py`, `ranking.py`, `audit.py` | Rust `retrieval.rs`, `ranking.rs`, `semantic_verification.rs`, `commands.rs` | Candidate generation, ranked review queues, actionability filtering, semantic evidence integration. | Retrieval, ranking, probe, and report policy mixed in one Python path. |
| `src/lean_dup/probes.py`, `semantic_probes.py`, `lean_runtime/SemanticProbe.lean` | `lean/LeanDup/Worker.lean`, Rust `semantic_verification.rs`, worker protocol | Bounded source-backed semantic verification with typed unavailable diagnostics and cache keys. | Batch-fatal probe behavior and Python cache semantics as production policy. |
| `src/lean_dup/models.py`, `replacement_hints.py` | Rust domain structs, `replacement_hints.rs`, `source_refs.rs`, `report_contract.rs` | Replacement hints, source references, report explanations, JSON/text output. | Python dataclass shape treated as report contract. |
| `tests/test_*.py` | `crates/lean-dup-rs/tests/cli.rs`, Rust unit tests, eval label files, Lean worker tests through Rust protocol | Fixture audits, hard negatives, worker protocol behavior, report contract, cache lifecycle. | Tests coupled to obsolete Python modules and progress-free worker stdout. |
| `pyproject.toml`, `uv.lock` | Cargo workspace plus `lean/` Lake package | Local development through `cargo run -p lean-dup-rs` and release-style Rust binary runs. | Python packaging advertised as production install path. |

Lean fixture projects under `tests/fixtures/` were retained. They are not Python implementation code; they are regression
inputs for the Rust CLI and eval suites.

## Remaining Work

Prompt 27 closes the Python implementation switchover, not the full production release.

- `G2 precision_control` remains open because the Prompt 27 production-gate artifact reports manual KanProofs mathlib
  hard-negative leakage.
- Prompt 28 must add release-grade CI, packaging, version output, and install documentation.
- Prompt 29 must inspect real workload findings and decide whether the remaining KanProofs mathlib precision issue is a
  ranking bug, label issue, or source-backed evidence problem.

## Red Flag Review

- **Shallow module:** mitigated. The deletion map records capability replacement and eliminated design mistakes, not just
  a list of deleted files.
- **Pass-through wrapper:** fixed. No Python compatibility wrapper remains.
- **Temporal decomposition:** mitigated. The map is organized by hidden responsibility and capability, not by migration
  chronology.
- **Information leakage:** mitigated. Python cache, SQLite, worker stdout, and source-parsing details are not release
  interfaces.
- **Special-general mixture:** residual risk is documented. KanProofs mathlib hard-negative leakage remains a production
  quality issue for prompts 29 and 30, not a reason to preserve Python code.
- **Conjoined methods:** mitigated. Rust boundaries split worker, index, retrieval, ranking, semantic verification,
  source references, replacement hints, and report contract.
- **Hard-to-describe public API:** mitigated. The public command surface is the Rust `lean-dup-rs` binary.
- **Implementation details contaminating interface comments:** mitigated. This document describes user-facing
  capabilities and evidence, not Python internals callers must learn.
