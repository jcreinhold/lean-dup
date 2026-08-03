# Worker protocol v1

The Lean worker answers five commands served by the native `lean-dup-worker` executable over line-framed JSONL. This
document is the caller-facing contract: schema versions, command guarantees, what Rust may rely on, and what stays
hidden inside the worker. Rust drives Lean through one warm subprocess per audited workspace — see
[Transport model](#transport-model) below.

For the pipeline that uses this protocol, see [end-to-end-architecture.md](end-to-end-architecture.md). For the layering
rule behind the boundary, see [overview.md](overview.md).

## Commands

| Command | Answers |
| --- | --- |
| `version` | what worker, protocol, and semantic algorithm versions am I? |
| `extract` | which declarations exist, where do they come from, what display/source facts can callers show? |
| `features` | which opaque semantic keys may Rust index and compare? |
| `index` | stream declaration and feature rows for these modules without forcing caller-side chunking policy |
| `probe` | which candidate relations are confirmed, refuted, or unavailable? |

`doctor`-style workspace health is composed Rust-side from `version` plus workspace/cache checks; it is not a worker
command (see [Transport model](#transport-model)).

## What Rust may and may not rely on

**May rely on:** the five command exports; the response kinds (`version_result`, `declaration_row`, `feature_row`,
`probe_result`, `progress`, `complete`, `error`); schema-version and compatibility rules; opaque declaration ids,
semantic keys, and probe pair ids as values to store and compare; machine-readable completion, progress, and structured
error events.

**May not rely on:** Lean `Expr` constructors, binder representation, universe representation, or traversal order;
`statement_text`, pretty-printed types, or display names as semantic inputs; transport field names as encodings of Lean
syntax; worker batching, import scheduling, session/process lifecycle, or frame ordering beyond the terminal-frame
rule; index storage layout, row ids, transaction order, or report/ranking policy.

## Why opaque ids and JSONL

A string-first protocol would have Lean emit pretty-printed statements and let Rust recompute fingerprints and probe
checks from text. That leaks Lean semantics into the scale layer and turns display text into a false abstraction. A
storage-first protocol would shape rows around the persisted index, so a persistence change becomes a protocol change.

The chosen design hands Rust opaque ids and opaque keys it can store and compare but never parse. Lean owns expression
traversal; Rust owns storage and retrieval; either side can be re-implemented without touching the other's abstraction.

## Transport model

Rust drives Lean by spawning the native `lean-dup-worker` executable under `lake env` in the audited workspace — one
warm child per workspace, reused across every command of an audit. Only that executable links `libleanshared`; the
parent stays free of the Lean runtime ABI. The executable is built per toolchain by `lean-dup install-worker` from the
packaged Lean source (`lean-dup-capability-source`) and resolved behind the private engine seam in the
`lean-dup-worker` crate. `lake env` installs the workspace's `.olean` search path; Lean-side, a single-entry session
environment cache makes all commands share one import per module signature.

The wire format is line-framed JSON on stdin/stdout. One request envelope per line:

```json
{"command": "extract", "request": { ... }}
```

where `request` has exactly the payload shape the command schema defines (modules, filters, pairs, chunk sizes). The
commands and their kinds:

| Command | Kind |
| --- | --- |
| `version` | request/response: one terminal `{"result": ...}` frame |
| `extract` | streaming |
| `features` | streaming |
| `probe` | streaming |
| `index` | streaming |

Response frames, one JSON object per line:

* `{"stream": <name>, "payload": <row>}` — a typed data row on a caller-named stream (`declarations`, `features`,
  `probe`); `index` emits on both `declarations` and `features`, routed by stream name;
* `{"progress": {"phase", "current", "total?"}}` — progress events;
* `{"diagnostic": {"code", "message"}}` — structured fatal diagnostics;
* `{"metadata": {"command", "rows", "skipped", "ok", "message?"}}` — the terminal frame of a streaming command
  (`ok: false` maps to a fatal worker diagnostic);
* `{"result": <payload>}` — the terminal frame of `version`.

Common request fields are unchanged from the wire schema: `schema_version` (required, `lean-dup.worker.v1`), `command`,
optional required `capabilities`, and optional `extensions`. There is no `request_id`: one command runs at a time per
child, so correlation is the line order.

Two protocol surfaces are **not** worker commands and are composed Rust-side instead:

- **`doctor`** is not a worker command. The CLI `doctor` subcommand is built from the `version` facts plus Rust-owned
  workspace and cache checks (olean presence, cache-entry status).
- **Worker substrate facts** (transport-framing protocol version and worker runtime version) are Rust-owned constants:
  `2` for the JSONL subprocess transport (`1` was the retired `lean-rs-worker` pool) and the `lean-dup-worker` crate
  version. The index cache key folds them in (see
  [cache-validity-lifecycle.md](cache-validity-lifecycle.md)) so a transport change invalidates stale entries. Pids,
  session keys, and queue counters are deliberately excluded.

A fatal failure surfaces as a mapped `WorkerError`: timeouts and cancellation stay recoverable (probe chunk-splitting
depends on this) and kill the child — the next command respawns it, and the Lean session cache bounds the cost to one
re-import. Child exit and worker-reported errors are fatal and discard partial output. The protocol does not commit
callers to a specific process ABI — only to the request schema, the command set, and the frame kinds above.

## Failure behavior

| Failure | Worker response | Rust behavior |
| --- | --- | --- |
| Import failure | fatal `import_failed`, exit nonzero | discard partial rows; do not update indexes |
| Missing compiled artifact | `missing_olean` | `doctor` reports it; index commands treat it as fatal when oleans are required |
| Malformed request JSON | fatal `malformed_json` if envelope can be produced; else exit nonzero | mapped to worker startup/protocol failure |
| Unsupported schema or command | fatal `unsupported_schema` / `unsupported_command` before importing | abort, surface diagnostic |
| Worker panic or nonzero exit without `complete` | none guaranteed | `worker_panic` with bounded stderr; discard partial stdout |
| Probe declaration unavailable | nonfatal `probe_result` with `status = "unavailable"` | continue with remaining pairs |
| Declaration exceeds the heartbeat budget | nonfatal: omit that declaration's rows, count it in the `complete` `skipped` total | continue; surface the skip count and raise `--max-heartbeats` to include it |
| Internal worker error after partial output | fatal `internal_error` when possible, exit nonzero | discard command output |

### Heartbeat budget and skips

The per-declaration heartbeat budget (`max_heartbeats`, optional; `0` = unlimited) and the strategy for recovering from
a declaration that exceeds it are **Lean-owned** — Rust supplies the budget as a hint and never inspects elaboration
state. The worker resets the budget per declaration, and a declaration whose elaboration exceeds it is skipped (its rows
omitted) rather than failing the whole command. The skip is never silent: the worker counts skips in the `complete`
envelope's `skipped` total, Rust persists that total in the index and surfaces it (`declarations_skipped_by_budget` in
audit JSON, a stderr progress event, and an index diagnostic). Skips are deterministic for a fixed budget, so
`max_heartbeats` participates in the index cache key — a different budget is a different corpus.

## Command reference

Each entry lists request payload fields, what callers may rely on, what the worker hides, and the response shape.

### `version`

Reports worker and schema versions without importing user modules.

- *Callers may rely on:* protocol version; worker package version; Lean version/toolchain string when available;
  semantic algorithm versions for extraction, features, and probes; supported command names and capabilities.
- *Hidden:* how version strings are obtained; how Lean package metadata is compiled into the executable; how future
  capabilities are represented internally.
- *Response:* one `version_result`; one `complete`.

### `doctor`

Validates that the worker can serve the requested schema and, when asked, can import requested modules or check for
compiled artifacts.

Request payload:

- `workspace_root`: optional display path for diagnostics.
- `modules`: optional array of module descriptors with `module` and `origin`.
- `require_oleans`: optional boolean. When true, missing compiled artifacts are fatal for the check.

- *Callers may rely on:* schema support status; worker executable health; importability for requested modules;
  compiled-artifact availability when requested; structured diagnostics suitable for `lean-dup doctor`.
- *Hidden:* search path construction; exact artifact paths checked; import order and batching; Lean exception
  formatting.
- *Response:* zero or more `progress`; one `doctor_result`; one `complete`.

### `extract`

Imports requested modules and streams declaration rows.

Request payload:

- `workspace_root`: display path for source spans and diagnostics.
- `modules`: nonempty array of module descriptors with `module` and `origin`.
- `include_private`, `include_generated`: booleans.
- `max_heartbeats`: optional natural number; the per-declaration Lean elaboration heartbeat budget in the same units the
  toolchain prints (`0` = unlimited). Additive `lean-dup.worker.v1` field; omitted means the worker's default. A
  declaration whose elaboration exceeds the budget is skipped, not fatal (see *Heartbeat budget and skips* below).

- *Callers may rely on:* one `declaration_row` per declaration accepted by the request filters; declaration ids stable
  within the command and usable as keys for later feature/probe requests under the same schema and cache context;
  1-based source spans when Lean can supply them; `statement_text` as human-facing display text only.
- *Hidden:* Lean environment traversal; declaration filtering mechanics; private/generated detection; pretty-printer
  options; source-range lookup.
- *Response:* zero or more `progress`; zero or more `declaration_row`; one `complete`.

### `features`

Emits Lean-owned semantic feature rows for selected declarations or for all accepted declarations in the requested
modules.

Request payload:

- `workspace_root`: display path for diagnostics.
- `modules`: nonempty array of module descriptors.
- `declaration_ids`: optional array of ids previously emitted under the same schema/cache context.
- `include_private`, `include_generated`: booleans.
- `max_heartbeats`: optional natural number; same meaning as for `extract` (`0` = unlimited). Additive
  `lean-dup.worker.v1` field. A declaration whose feature computation exceeds the budget is skipped, not fatal.

- *Callers may rely on:* feature rows using declaration ids from `extract` when supplied; fingerprint and feature-key
  equality being meaningful under the advertised semantic algorithm versions; `binder_count` as a Lean-computed
  statement metric; low-signal markers as Lean-owned hints, not ranking decisions.
- *Hidden:* expression canonicalization; binder dependency checks; connective normalization; conclusion extraction;
  constant/head role extraction; low-signal classification rules.
- *Response:* zero or more `progress`; zero or more `feature_row`; one `complete`.

### `index`

Streaming command for import-once index construction. Avoids forcing Rust to choose import, chunking, heartbeat
recovery, or task scheduling policy.

Request payload:

- `workspace_root`: display path for source spans and diagnostics.
- `modules`: nonempty array of module descriptors with `module`, `origin`, and optional source-root attribution.
- `include_private`, `include_generated`: booleans.
- `declaration_chunk_size`: optional natural number, a private worker hint.
- `declaration_parallelism`: optional natural number, derived by Rust from the effective `LEAN_NUM_THREADS` setting.
- `max_heartbeats`: optional natural number; same meaning as for `extract` (`0` = unlimited). Additive
  `lean-dup.worker.v1` field. Declarations exceeding the budget that cannot be split further are skipped, not fatal.

- *Callers may rely on:* streamed `declaration_row` and `feature_row` envelopes using the same row schemas as `extract`
  and `features`; progress events before import, after import, after declaration enumeration, at chunk start/finish, and
  during heartbeat-driven chunk splitting; a final `complete` only after all emitted rows are valid for the request.
- *Hidden:* whether chunks execute serially or through Lean tasks; how many Lean runtime threads are made available;
  declaration chunk boundaries, split policy, task priority, completion order; SQLite write, cache finalization, and row
  pairing in Rust.
- *Response:* zero or more `progress`; zero or more `declaration_row` and `feature_row`; one `complete`.

### `probe`

Runs bounded Lean semantic checks for candidate declaration pairs.

Request payload:

- `workspace_root`: display path for diagnostics.
- `modules`: nonempty module descriptors needed to import both sides of each pair.
- `include_private`, `include_generated`: booleans. They must match the declaration universe used by the index that
  produced the pair ids.
- `pairs`: array of pair descriptors with `pair_id`, `left_declaration_id`, `right_declaration_id`.
- `max_pairs`: optional positive integer; a defensive limit, not retrieval policy. Rust owns batching.
- `max_heartbeats`: optional natural number; same meaning as for `extract` (`0` = unlimited). Additive
  `lean-dup.worker.v1` field. Probe already recovers from per-pair heartbeat timeouts via chunk-split and `unavailable`.

- *Callers may rely on:* one `probe_result` per accepted pair unless the command fails fatally before pair processing;
  unavailable pair results being nonfatal when imports succeeded; boolean probe fields being meaningful only under the
  advertised probe algorithm version; probe results never requiring Rust to inspect Lean syntax.
- *Hidden:* theorem-like classification; reducibility guards; structural specialization checks; defeq/MetaM calls;
  timeout and heartbeat strategy.
- *Response:* zero or more `progress`; zero or more `probe_result`; one `complete`.

## Response schemas

### `version_result`

- `protocol_version`, `worker_version`: strings.
- `lean_version`: string or null.
- `semantic_versions`: object with `extract`, `features`, `probe` string versions.
- `supported_commands`: array containing the six commands.
- `supported_capabilities`: array of optional capability names.

### `doctor_result`

- `ok`: boolean.
- `checks`: array of `{name, status, message?}` with `status` in `ok | warning | failed | skipped`.
- `worker`: object with version fields from `version_result`.

### `declaration_row`

- `declaration_id`: opaque string; Rust may store and compare but must not parse.
- `origin`: string chosen by Rust request context (workspace, direct import, named import, external label).
- `module`: Lean module name, display/grouping fact.
- `qualified_name`, `display_name`: Lean and short human-facing names.
- `kind`: declaration kind: theorem, axiom, def, abbrev, opaque, or other supported kinds.
- `visibility`: `public | private | unknown`.
- `modifiers`: array of semantic modifier labels.
- `source_span`: `{file, start, end}` with 1-based `line`/`column`, or null.
- `statement_text`: pretty-printed statement for humans only.
- `status_flags`: array of Lean-owned labels such as generated or source-range-unavailable.

A caller may show `statement_text` but must not hash, parse, normalize, or compare it to derive semantic facts.

### `feature_row`

- `declaration_id`: opaque id from the same schema/cache context.
- `feature_version`: semantic feature algorithm version.
- `fingerprints`: object with opaque strings for `statement`, `safe_binder_permutation`, `connective_shape`,
  `conclusion_shape`.
- `role_features`: array of `{role, key, display?}`. `key` is opaque; `display` is human-facing only.
- `binder_count`: nonnegative integer.
- `low_signal_markers`: array of Lean-owned labels.

Rust may index and compare `fingerprints` and `role_features[*].key`. It must not reconstruct them from
`statement_text`, declaration names, source snippets, or Lean syntax.

### `probe_result`

- `pair_id`: opaque string supplied by Rust.
- `left_declaration_id`, `right_declaration_id`: opaque declaration ids.
- `status`: `ok | unavailable | invalid_pair`.
- `same_statement`, `same_up_to_safe_reordering`, `connective_equivalent`: booleans.
- `specializes_left_to_right`, `specializes_right_to_left`, `mutual_implication_shape`: booleans.
- `same_reducible_definition`: boolean.
- `message`: optional diagnostic string.

The aggregate "specializes" fact is Rust-side convenience: `specializes_left_to_right || specializes_right_to_left`.
Lean reports the directed facts.

### `progress`

- `phase`: string.
- `current`, `total`: nonnegative integers or null.
- `module`, `declaration`: labels or null.
- `elapsed_ms`: nonnegative integer or null.
- `message`: short human-readable message.

Progress is an event stream. Rust may render or record it but must not require specific phase names to interpret
semantic results.

### `complete`

- `row_counts`: object mapping emitted row/event kinds to nonnegative counts.
- `skipped`: nonnegative integer; declarations omitted because their elaboration exceeded the heartbeat budget. Additive
  `lean-dup.worker.v1` field; absent or `0` when nothing was skipped.
- `elapsed_ms`: nonnegative integer or null.

`complete` is the commit point. Rust discards result rows from a command that exits before `complete`.

### `error`

- `code`: one of `malformed_json | unsupported_schema | unsupported_command | invalid_request | import_failed |
  missing_olean | probe_unavailable | worker_panic | internal_error`.
- `fatal`: boolean.
- `message`: short human-readable message.
- `details`: optional bounded object or array.

Errors are aggregated where practical: a module import failure produces one `import_failed` with bounded module
diagnostics, not one unstructured traceback per downstream operation.

## Cache-key ingredients

Rust owns cache-key construction. Lean supplies semantic versions and opaque facts; it does not decide where or how Rust
stores indexes.

Local index keys must include: protocol and response schema version; worker version and semantic algorithm versions;
worker binary or source digest; Lean version and `lean-toolchain` content; Lake configuration and manifest digests;
requested module list and each module origin; command options that affect emitted declarations or features; workspace
source digests for requested modules; Git HEAD and dirty-state facts when available.

External and mathlib keys add: index label; module root; external workspace root; source stamps or source digests when
source is authoritative; compiled-artifact stamps or digests when `require_oleans` is part of the workflow;
`require_oleans` policy; package, manifest, and Git state for the external workspace when available.

Cache keys must not include storage layout names, insertion order, ranking thresholds, report format, or terminal
progress settings unless those settings change emitted semantic rows.

## Versioning and compatibility

The v1 schema uses `schema_version = "lean-dup.worker.v1"`. A v1 minor extension may add optional fields only under
`extensions` or in explicitly optional payload fields. Adding a required field, removing a field, changing a field
meaning, changing a required command, or changing an existing enum meaning requires a new major version.

Generated Rust protocol types must mirror this schema and reject: unknown top-level envelope fields other than
`extensions`; unknown required payload fields; unknown command names; unknown response kinds; unknown required
capability names. They may preserve unknown optional `extensions` values without interpreting them. Rust must validate
that the worker supports every command and required capability before using it for an audit or index build.
