# External Comparison Provenance

This document closes the production-readiness gate for source-backed versus static external comparison semantics.

For the current end-to-end audit flow that consumes this provenance boundary, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

This boundary owns the hidden knowledge for external comparison provenance: whether an index is source-backed or static,
which source and execution roots produced it, whether the current audit Lake environment can import its declarations,
and which evidence mode reports may claim.

Its smallest public interface is a typed comparison evidence policy plus JSON-safe provenance diagnostics in audit
reports. Callers ask whether an origin is static, source-backed but not importable, or proof-grade; they do not inspect
SQLite metadata, cache paths, worker JSONL, source-root mapping, or Lean import mechanics.

These decisions must not leak upward or sideways:

- SQLite metadata keys and table layout;
- source-root and execution-root comparison rules;
- worker module-descriptor construction;
- Lean probe chunking, heartbeat policy, or JSONL framing;
- cache-key and cache-publication details.

The validated user-facing capability preserved here is read-only duplicate auditing against local indexes, named
external indexes, and project-pinned mathlib. Static indexes still work; source-backed indexes receive proof-grade
semantic evidence only when the current audit worker can import their declarations.

Python-era behavior intentionally discarded: labels such as `mathlib` no longer imply semantic provenance by convention,
and external indexes are not treated as proof-grade because a Python-era workflow or cache path happened to name them
that way.

## Design It Twice

**Rejected: label-driven proof policy.** The tempting design is to keep using labels and origins such as `mathlib` to
decide whether ranking should require Lean semantic evidence. That is shallow: labels are user workflow names, not
source provenance. It also leaks policy sideways into ranking and probing, where every caller must remember which
strings imply importable Lean declarations.

**Chosen: internal provenance resolver.** The index layer persists typed source provenance, and a private resolver maps
opened indexes into a current-audit evidence policy. This is deeper because ranking and semantic verification receive
one question-oriented interface: what evidence mode does this origin have? The resolver hides source roots, execution
roots, importability checks, and static fallback policy.

## Contract

`--compare-mathlib` is always project-centered and source-backed. The mathlib index is built from the audited project's
pinned `.lake/packages/mathlib`, and probes execute from the audited project Lake root. When the cache is current, it is
proof-grade in that audit environment.

`--compare-index <label>` remains a named external-index lookup. Its evidence mode is determined by provenance:

- `proof-grade`: the index is source-backed and was built in the same Lake execution root as the current audit, so Lean
  probes may import both sides.
- `source-backed-not-importable`: the index has source provenance, but its execution root differs from the current
  audit Lake root. Retrieval and static ranking still work, but proof-grade claims require a later source-backed import
  plan.
- `static`: the index has no source provenance, usually because it is an old cache artifact or an intentionally static
  external index. Static evidence can support review suggestions, but it is reported as static and cannot silently look
  like verified Lean evidence.

Missing provenance is defined as static rather than fatal. This keeps old caches readable and avoids a migration step
inside a correctness prompt.

## Ranking And Probing

Ranking consumes a comparison evidence policy, not a `compare_mathlib` boolean. Static origins may use strong indexed
evidence directly. Proof-grade origins require verified semantic evidence before strong static matches become visible
as proof-grade findings. Source-backed-but-not-importable origins fall back to static evidence and report the reason.

Semantic verification uses the same policy to decide which non-workspace origins can be probed. It constructs worker
module descriptors only for proof-grade origins, keeping source roots and import policy inside the verifier/provenance
boundary.

## Report Semantics

Audit JSON includes comparison provenance records with label, origin, evidence mode, source root, execution root,
execution policy, declaration count, and a human-readable reason. Text reports include a compact provenance summary.

Ranked groups include an evidence mode:

- `proof-grade` means a Lean probe verified the evidence or the comparison origin requires and can support proof-grade
  evidence;
- `source-backed-not-importable` means the index has source provenance but cannot be imported in this audit environment;
- `static` means the group rests on indexed/static evidence.

The JSON shape is intentionally diagnostic rather than a final production report schema; prompt 26 owns the stable JSON
contract.

## Red Flag Review

- **Shallow module:** avoided. The resolver hides provenance mechanics and exposes evidence policy, not pass-through
  metadata.
- **Pass-through wrapper:** avoided. The boundary translates persisted provenance into current-audit proof capability.
- **Temporal decomposition:** avoided. The design is organized around provenance and importability decisions, not the
  order in which audit opens indexes, retrieves candidates, probes pairs, and renders reports.
- **Information leakage:** avoided. Ranking and reporting do not inspect SQLite keys, source-root comparison rules, or
  worker module construction.
- **Special-general mixture:** avoided. `--compare-mathlib` is one source-backed case of the general provenance policy,
  not a special string rule inside ranking.
- **Conjoined methods:** avoided. Index metadata, provenance resolution, semantic verification, and ranking communicate
  through typed facts rather than shared phase state.
- **Hard-to-describe public API:** no remaining red flag. The public concept is one evidence mode per comparison
  origin.
- **Implementation details contaminating interface comments:** avoided. Interface comments state what callers may rely
  on, not the metadata storage layout or Lean traversal details.
