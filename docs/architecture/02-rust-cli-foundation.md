# Rust CLI Foundation

This document records the Rust boundary introduced by prompt 08. It is a foundation for later production work, not the
production audit engine.

## Design Note

This layer owns Lake workspace discovery, module-root and source-file enumeration, Lake command invocation, cache-root
and cache-key policy, CLI rendering, and progress/profile plumbing. Its smallest public interface is the
`lean-dup-rs` binary plus one crate-level `run` entrypoint; all workspace, Lake, cache, progress, and rendering modules
remain internal.

These decisions must not leak upward or sideways:

- how Lakefiles are parsed;
- the conventional nested `lean/` workspace fallback;
- source and manifest hashing details;
- cache-key string format;
- phase names and timing granularity;
- stdout/stderr rendering policy.

The preserved user-facing capability is the command surface: `doctor`, `index`, `index-mathlib`, `audit`, `show`, and
`diff` all exist with typed options and stable progress/profile behavior. The intentionally discarded Python-era
behavior is loosely typed command state, ad hoc stderr writes from internal modules, Rust-side Lean semantic parsing,
source skeleton semantics, and pass-through Rust wrappers over Python implementation details.

## Design It Twice

**Rejected: one large `main.rs`.** This would make the first Rust version easy to write but shallow. Command-line
parsing, Lake discovery, cache-key policy, and rendering would all change together, and future prompts would have to
split apart accidental coupling before implementing worker protocol handling or indexes.

**Rejected: shell out to Python.** This would preserve existing behavior, but only by making Rust a pass-through layer.
It would keep cache and discovery decisions in the old Python modules and postpone the real Rust boundary.

**Chosen: internal decision-hiding modules.** The crate separates command parsing, command orchestration, workspace
discovery, Lake execution, cache construction, typed progress/profile recording, and rendering. This is deeper because
callers see one binary and one run entrypoint, while volatile details stay inside the module that owns them.

## Public Behavior

The foundation CLI is read-only. It discovers workspaces, reports facts, constructs deterministic cache fingerprints,
and returns deterministic skeleton results for commands whose production behavior belongs to later prompts. It does not
persist indexes, rank candidates, parse Lean semantics, or speak the worker protocol.

`doctor` performs real foundation checks: it resolves the Lake workspace, discovers module roots, enumerates source
files, resolves the cache root, computes a cache fingerprint, and runs `lake env lean --version`. `--require-oleans`
checks for compiled artifacts for the selected modules.

Progress and profile output are typed events recorded by internal modules. Rendering owns when and where those events
are printed, so JSON stdout remains machine-clean.

## Red Flag Review

- **Shallow module:** avoided by hiding real workspace, Lake, cache, and rendering decisions behind a narrow CLI.
- **Pass-through wrapper:** avoided because Rust does not delegate to Python for discovery or cache-key construction.
- **Temporal decomposition:** avoided by organizing modules around hidden decisions, not command execution steps.
- **Information leakage:** avoided by keeping Lakefile parsing, cache-key ingredients, and rendering policy private.
- **Special-general mixture:** avoided by keeping production ranking, protocol handling, and KanProofs policy out of
  this foundation.
- **Conjoined methods:** avoided by passing typed workspace and report values between modules.
- **Hard-to-describe public API:** avoided; the public API is "run the CLI".
- **Implementation details contaminating interface comments:** avoided by documenting caller guarantees rather than
  parsing or hashing mechanics.
