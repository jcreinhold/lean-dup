# Shared Semantic-Search Adoption

`lean-dup` consumes the standalone `lean-semantic-search` package for neutral semantic feature extraction, stable
feature DTOs, persistent semantic-index storage, and the storage-neutral retrieval seam. This note records the boundary:
what moved to the shared package, what stays local, and why.

The shared package was extracted from `lean-dup` (prompt sequence 02–06), so the migration is an adoption, not a
redesign. The worker transport was a separate effort, since completed: Rust now drives Lean through the
`lean-rs-worker-parent` pool capability rather than a subprocess (see [worker-protocol.md](worker-protocol.md) and
[validation/worker-migration-validation.md](validation/worker-migration-validation.md)). The `lean-dup.worker.v1` schema
and command semantics are unchanged.

## What moved to `lean-semantic-search`

- **Neutral Lean feature extraction.** Canonical fingerprints (`canonical.expr.v3`) and role features
  (`features.roles.v3`) now come from the shared Lean package
  (`LeanSemanticSearch.{Canonical, RoleFeatures, LeanCompat}`). `lean-dup`'s worker builds a package-owned
  `StatementShape` from each declaration via `LeanCompat.statementOfConstant` and calls the shared
  `Canonical.computeFromStatement` / `RoleFeatures.factsFromStatement`. The former local `LeanDup.Canonical` is deleted;
  `LeanDup.Features` and `LeanDup.Probe` are thin wrappers that keep the `lean-dup.worker.v1` row/probe payloads. The
  shared algorithms are byte-identical (same versions, same FNV-1a `stableHash`), so worker wire JSON is unchanged —
  confirmed by the full CLI suite.
- **Neutral feature DTOs.** `lean_semantic_search_contract::{Fingerprints, RoleFeature, DeclarationFeatureRow,
  OpaqueFeatureKey}` replace local copies. `OpaqueFeatureKey` is `#[serde(transparent)]` over `String`, so transport is
  wire-compatible.
- **Persistent semantic-index storage.** `lean_semantic_search_store` owns the opaque-key postings and the feature rows
  that retrieval consumes. `lean-dup` stops building its own `declaration_features`, `fingerprint_postings`, and
  `role_feature_postings` tables and stops deriving `DeclarationHandle`: the store keys postings and feature rows
  directly by the opaque, stable `declaration_id`.
- **Storage-neutral retrieval primitives.** `lean_semantic_search_retrieval::Corpus` (`document_total` / `fanout` /
  `postings` / `declaration_row`) is the candidate-source seam. The shared SQLite `Store` implements it; `lean-dup`'s
  retrieval drives its own scored fanout/top-k over that trait.

## What intentionally stays in `lean-dup`

Display/hydration metadata (the `declarations` table keyed by `declaration_id`), the probe cache, provenance kind and
evidence-mode resolution, the `mathlib` label and other corpus labels, baselines, review groups, replacement hints,
source-impact analysis, reports, vector search, release diagnostics, the cache-root layout, and the cache-key ingredient
computation. The scored retrieval and the ranking/scorer (`retrieval.rs`, `ranking.rs`, `scorer`, `pair_features.rs`,
`semantic_verification.rs`) stay local: they consume richer per-candidate explanation (`KeyContribution { kind, role,
display, key, score }`) and a numeric score than the shared `retrieve_across` exposes, so `lean-dup` ranks over the
`Corpus` trait rather than calling `retrieve_across`.

## Why `lean-dup` keeps ranking (not `retrieve_across`)

The shared `retrieve_across` returns `Candidate { declaration_id, rank, explanations: { family, match_count } }` — it
hides keys and scores by design. `lean-dup`'s ranking needs the matched key, the role display, and a numeric score per
contribution. The `Corpus` trait supplies the primitives to reconstruct all of that: `fanout` gives the
document-frequency counts the scorer already uses for rarity, `postings` gives candidate ids, and `declaration_row`
reconstructs a candidate's fingerprints and role features (with `display`, preserved in the store's `row_json`). So
audit output is preserved exactly while the storage moves to the shared store.

## On-disk split

| Owner | Holds | Keyed by |
| --- | --- | --- |
| `lean-dup` SQLite | `declarations` (display/hydration), `probe_cache`, `metadata` | `declaration_id` |
| shared `Store` | `feature_rows` (anchor reconstruction) + `postings` (opaque keys) | `declaration_id` |

`DeclarationHandle` and the three local posting/feature tables are removed. `lean-dup`'s index schema version is bumped
(`lean-dup.index.sqlite.v2` → `v3`); old caches are rejected and rebuilt, never migrated (established policy). The
shared `Store` independently rejects incompatible corpora via its own `corpus_token` / `STORE_SCHEMA_VERSION` /
`policy_version` check.

## corpus_token

`lean-dup`'s existing `IndexCacheKey` (Lake files, source digests, toolchain, include policy, selected roots,
worker/protocol/extract/features/probe versions, label, kind, roots) is serialized and hashed into a `cache_id` today.
That same hash becomes the opaque `corpus_token` handed to the store via an explicit conversion. Provenance and label
*meaning* stay on the `lean-dup` side (in `metadata` and `external_provenance.rs`), never inside the token.
Lake/source/toolchain ingredients reach the store only as that opaque token — the store never interprets them.

## Schema compatibility

The two version namespaces are independent and both checked for exact equality, mismatch ⇒ rebuild: `lean-dup`'s index
schema (`lean-dup.index.sqlite.v3`) guards the display/probe SQLite, and the shared store's `corpus_token` +
`lean-semantic-search.store.sqlite.v1` + retrieval policy version guard the semantic corpus. Neither side migrates an
incompatible cache.

## No vocabulary bleed

`lean-dup` does not learn proof-agent vocabulary, and the shared store does not learn duplicate-audit vocabulary:
provenance kinds, labels, evidence modes, and the probe cache stay in `lean-dup`. The shared crates carry only opaque
keys, feature rows, and the corpus token.

## Toolchain and dependency alignment

Adopting the shared Lean package via Lake requires a single toolchain across the dependency tree, so `lean-dup`'s Lean
toolchain is bumped to `leanprover/lean4:v4.31.0-rc1` to match the shared package (the package's `LeanCompat` owned-IR
boundary is what makes the shared extraction version-stable across the bump). The Rust path dependencies require one
`libsqlite3-sys` (`links = "sqlite3"`), so both workspaces align on rusqlite 0.40.
