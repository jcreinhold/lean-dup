# Python Deprecation Map

The retired Python implementation, what replaced each piece, and the design mistake it took with it. This document
closes `G9 python_deprecation`. It does not close the remaining quality and release-hardening gates.

For the current Rust/Lean architecture, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Parity evidence

Artifacts:

| Artifact | Result |
| --- | --- |
| `target/eval/prompt27-default.json` | `status = ok`; recall@10 14/14; hard-negative hits 0/4 |
| `target/eval/prompt27-hard-negatives.json` | `status = ok`; hard-negative hits 0/5 |
| `target/eval/prompt27-production-gate.json` | `status = ok`; manual KanProofs/mathlib no longer hits the old 60 s worker timeout |

The production-gate artifact still reports `hard_negative_hits = 3/4` in the manual KanProofs/mathlib child. The
Python switchover is not blocked on this—the Rust gate completes—but `G2 precision_control` remains open.

Stale Python worker tests were not retained. They expected worker stdout to contain only row payloads; the production
worker now streams progress and data events through JSONL. Rust protocol and CLI tests cover the current contract.

## Deletion map

| Removed Python path | Rust/Lean replacement | Mistake eliminated |
| --- | --- | --- |
| `src/lean_dup/cli.py`, `__main__.py`, `__init__.py` | `crates/cli/src/cli.rs`, `commands.rs`, `render.rs` | two production command surfaces |
| `src/lean_dup/workspace.py` | `crates/project/src/workspace.rs`, `mathlib.rs`, `cache.rs` | workspace/package-layout knowledge scattered through audit code |
| `src/lean_dup/extractor.py`, `lean_runtime/Extractor.lean` | `lean/LeanDup/Worker.lean`, Rust `worker` + `index` | source parsing and JSON/string-driven semantic facts in Python |
| `src/lean_dup/features.py`, `matching.py`, `text.py` | Lean worker feature extraction + Rust `retrieval.rs` | Rust/Python recomputing Lean semantic structure from text |
| `src/lean_dup/external_index.py` | `crates/index/src/index.rs`, `external_provenance.rs`, `cache_lifecycle.rs` | cache and SQLite policy leaking into audit workflow |
| `src/lean_dup/candidates.py`, `ranking.py`, `audit.py` | Rust `retrieval.rs`, `ranking.rs`, `semantic_verification.rs`, `commands.rs` | retrieval, ranking, probe, report policy mixed in one path |
| `src/lean_dup/probes.py`, `semantic_probes.py`, `lean_runtime/SemanticProbe.lean` | `lean/LeanDup/Worker.lean`, Rust `semantic_verification.rs`, worker protocol | batch-fatal probe behavior; Python cache semantics as production policy |
| `src/lean_dup/models.py`, `replacement_hints.py` | Rust domain structs, `replacement_hints.rs`, `source_refs.rs`, `report_contract.rs` | Python dataclass shape treated as report contract |
| `tests/test_*.py` | `crates/cli/tests/cli.rs`, Rust unit tests, eval label files, Lean worker tests through the protocol | tests coupled to obsolete modules and progress-free worker stdout |
| `pyproject.toml`, `uv.lock` | Cargo workspace + `lean/` Lake package | Python packaging advertised as the production install path |

Lean fixture projects under `tests/fixtures/` were retained. They are regression inputs for the Rust CLI and eval
suites, not Python implementation code.

## Capability preserved across the switch

`doctor`, `index`, `index-mathlib`, `audit`, `eval`, `show`, `diff`; progress/profile-safe output; Lake workspace
resolution; project-pinned mathlib; SQLite indexes; shared cache; source-backed vs static provenance; bounded
semantic verification; ranked review queues with replacement hints; text and JSON reports with stable explanation
facts.

## What still needs work

The Python switchover closed `G9`. Open items:

- `G2 precision_control`: manual KanProofs/mathlib hard-negative leakage.
- Release-grade CI, packaging, version output, install docs.
- Inspect real-workload findings; decide whether KanProofs/mathlib precision is ranking, labels, or source-backed
  evidence.
