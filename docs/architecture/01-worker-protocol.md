# Lean-Dup Worker Protocol v1

This document specifies the first versioned protocol between the Lean semantic worker and the Rust audit engine. It
refines the boundary in [00-overview.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/00-overview.md) into a
caller-facing contract for prompt 03 and later implementation prompts.

The protocol is a semantic interface, not a storage format. Lean types and Rust domain structs are the model. JSON and
JSONL are subprocess encodings used to move those facts between processes.

## Protocol Comments

Rust callers may rely on:

- the six worker commands: `extract`, `features`, `index`, `probe`, `doctor`, and `version`;
- the eight response kinds: `version_result`, `doctor_result`, `declaration_row`, `feature_row`, `probe_result`,
    `progress`, `complete`, and `error`;
- stable schema-version and compatibility rules;
- opaque declaration ids, semantic keys, and probe pair ids as values to store and compare;
- machine-readable completion, progress, and structured error envelopes.

Rust callers must not rely on:

- Lean `Expr` constructors, binder representation, universe representation, or traversal order;
- `statement_text`, pretty-printed types, source snippets, or display names as semantic inputs;
- transport field names as encodings of Lean syntax;
- worker batching, import scheduling, stderr wording, or subprocess setup details outside the Rust worker runtime;
- index storage layout, row ids, transaction order, insertion phases, or report/ranking policy.

Any protocol comment or generated interface comment must describe what callers can rely on. It must not describe Lean
traversal algorithms, storage layout, temporary migration scaffolding, or the current Python implementation.

## Design Note

This document owns the hidden knowledge for the worker boundary: the worker schema version, public command contract,
response envelope kinds, row and event shapes, cache-key ingredients, compatibility rules, and failure model.

The smallest public interface is:

- one JSON request object per worker subprocess invocation;
- JSONL response envelopes on stdout;
- the six commands and eight response kinds listed above.

These design decisions must not leak upward or sideways:

- Lean expression traversal, canonicalization, generated-declaration detection, and reducibility policy;
- subprocess framing details outside worker runtime code;
- index persistence layout and cache placement;
- retrieval weights, ranking thresholds, replacement-hint policy, and report formatting.

Validated user-facing capabilities preserved by this protocol:

- full local workspace audits;
- local, external, and mathlib comparison through reusable indexes;
- progress and profile reporting that never corrupts JSON output;
- semantic probes for high-value candidate pairs;
- ranked actionable findings, replacement/import hints, `show`, and baseline review workflows.

Python-era behavior intentionally discarded:

- Rust or Python recomputation of Lean semantic facts from pretty-printed statements;
- source parsing as a fallback for semantic facts Lean should own;
- rows shaped around an index implementation;
- JSON/string-first semantics;
- broad materialization or hydration policies embedded in the semantic boundary.

## Design It Twice

**Rejected: JSON/string-first protocol.** Lean would emit names and pretty-printed statements. Rust would recompute
fingerprints, role features, and probe-like checks from those strings. This leaks Lean semantics into the scale layer,
turns display text into a false abstraction, and creates a shallow module: the interface exposes nearly as much semantic
complexity as the implementation.

**Rejected: storage-first protocol.** The worker would emit rows dictated by the persisted index shape. This makes a
storage decision part of the worker interface, so a persistence change becomes a protocol change. It also creates
information leakage between the Lean worker, index builder, retrieval, and reporting layers.

**Chosen: capability-first worker protocol.** Lean imports modules, emits semantic declaration rows and feature rows,
and answers bounded probe requests. Rust stores opaque ids and keys, builds cache keys, persists indexes, retrieves
candidates, ranks them, and renders reports. This design is deeper because the caller-facing interface is small,
semantic facts remain Lean-owned, storage remains Rust-owned, and later prompts can change either implementation without
changing the other side's abstraction.

## Transport Model

Each worker run receives exactly one UTF-8 JSON request object on stdin. The request object has these common fields:

- `schema_version`: required string. For this document, `lean-dup.worker.v1`.
- `request_id`: required nonempty string chosen by Rust for correlation.
- `command`: required string, one of `extract`, `features`, `index`, `probe`, `doctor`, or `version`.
- `capabilities`: optional array of required capability names. The worker must reject a request if it cannot satisfy any
    required capability.
- `extensions`: optional object for optional, non-required v1 data.

The worker writes UTF-8 JSONL response envelopes to stdout. Each line is a complete JSON object with these common
fields:

- `schema_version`: response schema version.
- `request_id`: copied from the request when the request was parsed.
- `command`: command being answered when known.
- `kind`: one of the eight response kinds.
- `payload`: response-kind-specific object.
- `extensions`: optional object for optional, non-required v1 data.

Stdout is machine-only. Normal progress and profile data use `progress` envelopes, not stderr. Stderr is reserved for
panic diagnostics, Lean runtime fallback diagnostics, or errors that happen before a structured `error` can be emitted.

A successful command must emit exactly one `complete` envelope after all result rows or events. A fatal protocol failure
must emit an `error` envelope when possible and exit nonzero. Rust treats a nonzero exit, EOF before `complete`, or
invalid JSONL as worker failure and discards partial command output.

This document does not specify a Lean/Rust FFI ABI.

## Commands

### `version`

`version` reports the worker and schema versions without importing user modules.

Callers may rely on:

- protocol version;
- worker package version;
- Lean version/toolchain string when available;
- semantic algorithm versions for declaration extraction, features, and probes;
- supported command names and supported optional capabilities.

Hidden decisions:

- how the worker obtains version strings;
- how Lean package metadata is compiled into the executable;
- how future optional capabilities are represented internally.

Response:

- one `version_result`;
- one `complete` on success.

### `doctor`

`doctor` validates that the worker can serve the requested schema and, when asked, can import requested modules or check
for compiled artifacts.

Request payload fields:

- `workspace_root`: optional display path supplied by Rust for diagnostics.
- `modules`: optional array of module descriptors with `module` and `origin`.
- `require_oleans`: optional boolean. When true, missing compiled artifacts are fatal for the check.

Callers may rely on:

- schema support status;
- worker executable health;
- importability status for requested modules;
- compiled-artifact availability status when requested;
- structured diagnostics suitable for `lean-dup doctor`.

Hidden decisions:

- search path construction;
- exact artifact paths checked;
- import order and import batching;
- Lean exception formatting.

Response:

- zero or more `progress` events;
- one `doctor_result`;
- one `complete` on success.

### `extract`

`extract` imports requested modules and streams declaration rows. It answers "which declarations exist, where do they
come from, and what stable display/source facts can callers show?"

Request payload fields:

- `workspace_root`: display path for source spans and diagnostics.
- `modules`: nonempty array of module descriptors with `module` and `origin`.
- `include_private`: boolean.
- `include_generated`: boolean.

Callers may rely on:

- one `declaration_row` per declaration accepted by the request filters;
- every emitted declaration id being stable within the command and suitable as the key for later feature/probe requests
    under the same schema and cache context;
- source spans being 1-based when Lean can supply them;
- `statement_text` being human-facing display text only.

Hidden decisions:

- Lean environment traversal;
- declaration filtering mechanics;
- private/generated declaration detection;
- pretty-printer options;
- source range lookup mechanics.

Response:

- zero or more `progress` events;
- zero or more `declaration_row` envelopes;
- one `complete` on success.

### `features`

`features` emits Lean-owned semantic feature rows for selected declarations or for all accepted declarations in the
requested modules. It answers "which opaque semantic keys may Rust index and compare?"

Request payload fields:

- `workspace_root`: display path for diagnostics.
- `modules`: nonempty array of module descriptors with `module` and `origin`.
- `declaration_ids`: optional array of declaration ids previously emitted under the same schema and cache context.
- `include_private`: boolean.
- `include_generated`: boolean.

Callers may rely on:

- feature rows using declaration ids from `extract` when ids are supplied;
- fingerprint and feature-key equality being meaningful under the advertised semantic algorithm versions;
- `binder_count` being a Lean-computed statement metric;
- low-signal markers being Lean-owned semantic hints, not ranking decisions.

Hidden decisions:

- expression canonicalization;
- binder dependency checks;
- connective normalization;
- conclusion extraction;
- constant/head role extraction;
- low-signal classification rules.

Response:

- zero or more `progress` events;
- zero or more `feature_row` envelopes;
- one `complete` on success.

### `index`

`index` is an internal streaming command for import-once index construction. It answers "stream declaration and feature
rows for this module set without forcing Rust to choose Lean import, chunking, heartbeat recovery, or task scheduling
policy."

Request payload fields:

- `workspace_root`: display path for source spans and diagnostics.
- `modules`: nonempty array of module descriptors with `module`, `origin`, and optional source-root attribution.
- `include_private`: boolean.
- `include_generated`: boolean.
- `declaration_chunk_size`: optional natural number. This is a private worker hint, not CLI policy.
- `declaration_parallelism`: optional natural number. This is derived by Rust from the effective `LEAN_NUM_THREADS`
    setting and remains private to worker runtime code.

Callers may rely on:

- streamed `declaration_row` and `feature_row` envelopes using the same row schemas as `extract` and `features`;
- progress events before import, after import, after declaration enumeration, at chunk start/finish, and during
    heartbeat-driven chunk splitting;
- a final `complete` envelope only after all emitted rows are valid for the request.

Hidden decisions:

- whether chunks execute serially or through Lean tasks;
- how many Lean runtime threads are made available to the worker subprocess;
- declaration chunk boundaries, split policy, task priority, and task completion order;
- SQLite write, cache finalization, and row pairing policy in Rust.

Response:

- zero or more `progress` events;
- zero or more `declaration_row` envelopes;
- zero or more `feature_row` envelopes;
- one `complete` on success.

### `probe`

`probe` runs bounded Lean semantic checks for candidate declaration pairs. It answers "which candidate relations are
confirmed, refuted by this bounded check, or unavailable?"

Request payload fields:

- `workspace_root`: display path for diagnostics.
- `modules`: nonempty array of module descriptors needed to import both sides of each pair.
- `pairs`: array of pair descriptors with `pair_id`, `left_declaration_id`, and `right_declaration_id`.
- `max_pairs`: optional positive integer. Rust owns batching; this field is a defensive limit, not retrieval policy.

Callers may rely on:

- one `probe_result` per accepted pair unless the command fails fatally before pair processing;
- unavailable pair results being nonfatal when imports succeeded;
- boolean probe fields being meaningful only under the advertised probe algorithm version;
- probe results never requiring Rust to inspect Lean syntax.

Hidden decisions:

- theorem-like classification details;
- reducibility guards;
- structural specialization checks;
- defeq tactic or MetaM calls;
- timeout and heartbeat strategy.

Response:

- zero or more `progress` events;
- zero or more `probe_result` envelopes;
- one `complete` on success.

## Response Schemas

### `version_result`

Payload fields:

- `protocol_version`: string.
- `worker_version`: string.
- `lean_version`: string or null.
- `semantic_versions`: object with `extract`, `features`, and `probe` string versions.
- `supported_commands`: array containing `extract`, `features`, `index`, `probe`, `doctor`, and `version`.
- `supported_capabilities`: array of optional capability names.

### `doctor_result`

Payload fields:

- `ok`: boolean.
- `checks`: array of check objects with `name`, `status`, and optional `message`.
- `worker`: object containing version fields also available from `version_result`.

Allowed check statuses are `ok`, `warning`, `failed`, and `skipped`.

### `declaration_row`

Payload fields:

- `declaration_id`: opaque string. Rust may store and compare it; Rust must not parse it.
- `origin`: string chosen by Rust request context, such as workspace, direct import, named import, or external label.
- `module`: Lean module name as a display and grouping fact.
- `qualified_name`: Lean declaration name as a display and lookup fact.
- `display_name`: short human-facing declaration name.
- `kind`: declaration kind, such as theorem, axiom, def, abbrev, opaque, or other supported kinds.
- `visibility`: `public`, `private`, or `unknown`.
- `modifiers`: array of semantic modifier labels.
- `source_span`: object with `file`, `start`, and `end`, or null when no source span is available.
- `statement_text`: pretty-printed statement text for humans only.
- `status_flags`: array of Lean-owned labels, such as generated or source-range-unavailable.

`source_span.start` and `source_span.end` use 1-based `line` and `column` fields. A caller may show `statement_text`,
but must not hash, parse, normalize, or compare it to derive semantic facts.

### `feature_row`

Payload fields:

- `declaration_id`: opaque declaration id from the same schema and cache context.
- `feature_version`: semantic feature algorithm version.
- `fingerprints`: object with opaque string values for `statement`, `safe_binder_permutation`, `connective_shape`, and
    `conclusion_shape`.
- `role_features`: array of objects with `role`, `key`, and optional `display`. The `key` is opaque; `display` is
    human-facing only.
- `binder_count`: nonnegative integer.
- `low_signal_markers`: array of Lean-owned marker labels.

Rust may index and compare `fingerprints` and `role_features[*].key`. Rust must not reconstruct them from
`statement_text`, declaration names, source snippets, or Lean syntax.

### `probe_result`

Payload fields:

- `pair_id`: opaque string supplied by Rust.
- `left_declaration_id`: opaque declaration id.
- `right_declaration_id`: opaque declaration id.
- `status`: `ok`, `unavailable`, or `invalid_pair`.
- `same_statement`: boolean.
- `same_up_to_safe_reordering`: boolean.
- `connective_equivalent`: boolean.
- `specializes_left_to_right`: boolean.
- `specializes_right_to_left`: boolean.
- `mutual_implication_shape`: boolean.
- `same_reducible_definition`: boolean.
- `message`: optional diagnostic string.

The aggregate "specializes" fact is Rust-side convenience: Rust may compute it as
`specializes_left_to_right || specializes_right_to_left`. Lean reports the directed facts.

### `progress`

Payload fields:

- `phase`: string.
- `current`: nonnegative integer or null.
- `total`: nonnegative integer or null.
- `module`: module label or null.
- `declaration`: declaration display label or null.
- `elapsed_ms`: nonnegative integer or null.
- `message`: short human-readable message.

Progress is an event stream, not a data dependency. Rust may render it or record it for profiles. Rust must not require
specific phase names to interpret semantic results.

### `complete`

Payload fields:

- `row_counts`: object mapping emitted row/event kinds to nonnegative counts.
- `elapsed_ms`: nonnegative integer or null.

`complete` is the commit point for the subprocess response. Rust must discard result rows from a command that exits
before `complete`.

### `error`

Payload fields:

- `code`: one of `malformed_json`, `unsupported_schema`, `unsupported_command`, `invalid_request`, `import_failed`,
    `missing_olean`, `probe_unavailable`, `worker_panic`, or `internal_error`.
- `fatal`: boolean.
- `message`: short human-readable message.
- `details`: optional bounded object or array.

Errors should be aggregated where practical. For example, a module import failure should produce one `import_failed`
error with bounded module diagnostics, not one unstructured traceback per downstream operation.

## Cache Keys

Rust owns cache-key construction and validation. Lean supplies semantic versions and opaque facts; it does not decide
where or how Rust stores indexes.

Local index cache keys must include:

- protocol and response schema version;
- worker version and semantic algorithm versions;
- worker binary or worker source digest;
- Lean version and `lean-toolchain` content;
- Lake configuration and manifest digests;
- requested module list and each module origin;
- command options that affect emitted declarations or features;
- workspace source digests for requested modules;
- Git HEAD and dirty-state facts when available.

External and mathlib index cache keys must include all local ingredients plus:

- index label;
- module root;
- external workspace root;
- source stamps or source digests when source is authoritative;
- compiled-artifact stamps or digests when `require_oleans` is part of the workflow;
- `require_oleans` policy;
- package, manifest, and Git state for the external workspace when available.

Cache keys must not include storage layout names, insertion order, ranking thresholds, report format, or terminal
progress settings unless those settings change emitted semantic rows.

## Lean-Computed And Rust-Computed Facts

Lean computes:

- declaration identity and display facts;
- declaration kind, visibility, modifiers, source spans, and generated/private facts where Lean can supply them;
- pretty-printed statement text for display;
- exact statement, safe binder permutation, connective-shape, and conclusion-shape fingerprints;
- role-aware semantic feature keys for constants, heads, binders, and conclusions;
- binder count and low-signal semantic markers;
- bounded probe results, including directed specialization and reducible-definition equality.

Rust computes:

- workspace discovery and module-root inference;
- Lake invocation, worker process lifecycle, and request batching;
- cache keys, cache validation, index labels, and index paths;
- persistence and retrieval data structures;
- weighted retrieval, broad-key suppression, candidate caps, and hydration policy;
- source-reference scans and name-token features used for ranking or display;
- candidate ranking, blockers, priorities, recommended actions, replacement/import hints;
- text, JSON, `show`, profile, and baseline diff reports.

Rust may compare Lean-emitted opaque keys for equality and may store them in indexes. Rust must not inspect Lean
expressions, parse pretty statements, or derive semantic fingerprints from source text.

## Versioning And Compatibility

The v1 schema uses `schema_version = "lean-dup.worker.v1"`. A v1 minor extension may add optional fields only under
`extensions` or in explicitly optional payload fields. A change that adds a required field, removes a field, changes a
field meaning, changes a required command, or changes an existing enum meaning requires a new major schema version.

Generated Rust protocol types must mirror this schema. They must reject:

- unknown top-level envelope fields other than `extensions`;
- unknown required payload fields;
- unknown command names;
- unknown response kinds;
- unknown required capability names.

Generated Rust types may preserve unknown optional `extensions` values without interpreting them. Rust must validate
that the worker supports every command and required capability before using it for an audit or index build.

## Failure Behavior

- **Import failure:** emit fatal `import_failed` when possible and exit nonzero. Rust discards partial rows and does not
    update indexes.
- **Missing compiled artifact:** emit `missing_olean`. `doctor` may report this as a failed or warning check depending
    on `require_oleans`; index-building commands treat it as fatal when compiled artifacts are required.
- **Malformed request JSON:** emit fatal `malformed_json` if enough of the request can be parsed to produce an envelope.
    If not, exit nonzero and let Rust map the failure to worker startup/protocol failure.
- **Unsupported schema or command:** emit fatal `unsupported_schema` or `unsupported_command` before importing modules
    or producing result rows.
- **Worker panic or nonzero exit without `complete`:** Rust reports `worker_panic`, includes bounded stderr, and
    discards partial stdout rows.
- **Probe declaration unavailable:** emit nonfatal `probe_result` with `status = "unavailable"` unless the module import
    failed. Missing declarations inside an imported environment do not invalidate other pair results.
- **Internal worker error after partial output:** emit fatal `internal_error` when possible and exit nonzero. Rust
    discards the command output because `complete` was not reached.

## Red Flag Review

- **Shallow module:** avoided by specifying behavior-rich worker capabilities instead of a pass-through copy of current
    Python rows or future index storage.
- **Pass-through wrapper:** avoided by keeping subprocess setup and framing inside the Rust worker runtime and exposing
    semantic commands to callers.
- **Temporal decomposition:** avoided by organizing commands around capabilities, not around build, insert, query, or
    report phases.
- **Information leakage:** avoided by opaque declaration ids, opaque feature keys, human-only display text, and no
    persistence layout in the protocol.
- **Special-general mixture:** avoided by keeping KanProofs cleanup policy, ranking policy, report policy, and storage
    policy outside the worker schema.
- **Conjoined methods:** avoided because each command has a standalone request/response contract and does not require a
    caller to understand another command's implementation.
- **Hard-to-describe public API:** mitigated by one request shape, one envelope shape, six commands, and eight response
    kinds.
- **Implementation details contaminating interface comments:** avoided by documenting caller guarantees and hidden
    decisions rather than Lean traversal algorithms, storage layout, or migration internals.
