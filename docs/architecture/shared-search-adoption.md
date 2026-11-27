# Shared Semantic-Search Adoption

`lean-dup` consumes the standalone `lean-semantic-search` package for neutral semantic feature extraction, stable
feature DTOs, and the storage-neutral retrieval seam. This note records the boundary: what moved to the shared package,
what stays local, and why. The semantic-index storage also moved there for a time, as `lean-semantic-search-store`, and
has since moved back (see [Semantic corpus storage](#semantic-corpus-storage)).

The shared package was extracted from `lean-dup` (prompt sequence 02–06), so the migration is an adoption, not a
redesign. The worker transport was a separate effort, since completed: Rust now drives Lean through the
`lean-dup-worker` executable over JSONL (see [worker-protocol.md](worker-protocol.md)). The `lean-dup.worker.v1` schema
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
- **Storage-neutral retrieval primitives.** `lean_semantic_search_retrieval::Corpus` (`document_total` / `fanout` /
  `postings` / `declaration_row`) is the candidate-source seam. The SQLite `Store` in `lean-dup-index` implements it;
  `lean-dup`'s retrieval drives its own scored fanout/top-k over that trait.

## What intentionally stays in `lean-dup`

Display/hydration metadata (the `declarations` table keyed by `declaration_id`), the probe cache, provenance kind and
evidence-mode resolution, the `mathlib` label and other corpus labels, baselines, review groups, replacement hints,
source-impact analysis, reports, vector search, release diagnostics, the cache-root layout, the cache-key ingredient
computation, and the semantic corpus store. The scored retrieval and the ranking/scorer (`retrieval.rs`, `ranking.rs`,
`scorer`, `pair_features.rs`, `semantic_verification.rs`) stay local: they consume richer per-candidate explanation
(`KeyContribution { kind, role, display, key, score }`) and a numeric score than the shared `retrieve_across` exposes,
so `lean-dup` ranks over the `Corpus` trait rather than calling `retrieve_across`.

## Semantic corpus storage

The corpus store holds the opaque-key postings and the feature rows that retrieval reads. It began as the shared
`lean-semantic-search-store` crate. `lean-dup` was that crate's only user, so its 0.8.0 code now lives in
`lean-dup-index` as the private `store` module, and `lean-dup` no longer depends on the crate. The file format did not
change, so existing caches stay valid. The store keys postings and feature rows directly by the opaque, stable
`declaration_id`; it replaced the local `declaration_features`, `fingerprint_postings`, and `role_feature_postings`
tables.

## Why `lean-dup` keeps ranking (not `retrieve_across`)

The shared `retrieve_across` returns `Candidate { declaration_id, rank, explanations: { family, match_count } }` — it
hides keys and scores by design. `lean-dup`'s ranking needs the matched key, the role display, and a numeric score per
contribution. The `Corpus` trait supplies the primitives to reconstruct all of that: `fanout` gives the
document-frequency counts the scorer already uses for rarity, `postings` gives candidate ids, and `declaration_row`
reconstructs a candidate's fingerprints and role features (with `display`, preserved in the store's `row_json`). So
ranking over the corpus store gives the same audit output as the old local tables did.

## On-disk split

Each cache entry directory holds two SQLite files, both written by `lean-dup-index`:

| File | Holds | Keyed by |
| --- | --- | --- |
| `index.sqlite` | `declarations` (display/hydration), `probe_cache`, `metadata` | `declaration_id` |
| `corpus.sqlite` (`store` module) | `feature_rows` (anchor reconstruction), `postings` (opaque keys), `metadata` | `declaration_id` |

`DeclarationHandle` and the three local posting/feature tables are removed. `lean-dup`'s index schema version is bumped
(`lean-dup.index.sqlite.v2` → `v3`); old caches are rejected and rebuilt, never migrated (established policy).
`Store::open` refuses a corpus whose store schema version differs. A corpus also records the corpus token and the
retrieval policy version, but nothing compares them.

## corpus_token

`lean-dup`'s existing `IndexCacheKey` (Lake files, source digests, toolchain, include policy, selected roots,
worker/protocol/extract/features/probe versions, label, kind, roots) is serialized and hashed into a `cache_id` today.
That same hash becomes the opaque `corpus_token` handed to the store via an explicit conversion. The `cache_id` also
names the cache entry's directory, so a corpus always sits beside the index built from the same key. Provenance and
label *meaning* stay on the `lean-dup` side (in `metadata` and `external_provenance.rs`), never inside the token.
Lake/source/toolchain ingredients reach the store only as that opaque token — the store never interprets them.

## Schema compatibility

Only `index.sqlite` decides whether a cache entry is reused: the entry is current when its index carries schema
`lean-dup.index.sqlite.v3` and the exact cache key. The corpus needs no check of its own, because the cache key also
names its directory. `Store::open` still refuses a corpus whose schema is not `lean-semantic-search.store.sqlite.v1`,
but a refused corpus is an error, not a rebuild. So a change to the corpus layout must also bump `INDEX_SCHEMA_VERSION`.
That changes the cache key, so old entries rebuild instead of failing to open. The store schema string keeps the old
crate's name because existing corpora carry it. Nothing migrates an incompatible cache.

## No vocabulary bleed

`lean-dup` does not learn proof-agent vocabulary, and the shared crates do not learn duplicate-audit vocabulary:
provenance kinds, labels, evidence modes, and the probe cache stay in `lean-dup`. The shared crates carry only opaque
keys and feature rows.

## Toolchain and dependency alignment

Adopting the shared Lean package via Lake requires a single toolchain across the dependency tree, so `lean-dup`'s Lean
toolchain is bumped to `leanprover/lean4:v4.31.0-rc2` to match the shared package (the package's `LeanCompat` owned-IR
boundary is what makes the shared extraction version-stable across the bump). Cargo allows one `libsqlite3-sys`
(`links = "sqlite3"`) in a build, so while the shared store linked SQLite, both workspaces had to agree on one rusqlite
release. No shared crate links SQLite now, so `lean-dup` picks its rusqlite release alone.

## Packaged Lean source boundary

`lean-dup` no longer depends on `../../lean-semantic-search/lean` as the steady-state Lake package path. The worker
build script asks `lean-semantic-search-runtime` to materialize the package-owned `LeanSemanticSearch` source payload
for `lean/lean-toolchain`, then asks `lean-toolchain`'s shared source-package materializer to install a private
generated `LeanDup` Lake root under Cargo's `OUT_DIR` with:

- `require <runtime-owned semantic-search package id> from <materialized runtime source root>`;
- `require «lean_rs_interop_shims» from <materialized lean-rs interop shim source root>`;
- the checked-in `LeanDup.lean` and `LeanDup/` sources copied unchanged by the shared materializer.

This keeps semantic-search source ownership in `lean-semantic-search` while preserving the `lean-dup.worker.v1` command
payloads and duplicate-audit workflow. The checked-in `lean/lakefile.lean` is now a developer-facing Lean check, not the
release build root: direct `lake -d lean build` requires `LEAN_DUP_SEMANTIC_SEARCH_ROOT` to point at a materialized
semantic-search runtime source root. Release and CI builds should use `cargo build -p lean-dup-worker`, which performs
that materialization itself.

The generated root uses the same digest-keyed, locked, provenance-recorded materialization primitive as the packaged
semantic-search runtime and generic interop shims. `lean-dup` still owns the generated Lakefile and manifest text
because only it knows the worker package dependencies and export surface; it no longer owns cache population, source
copying, generated `lean-toolchain` installation, or provenance sidecar mechanics.

The generated capability manifest records the semantic-search dylib through the runtime crate's typed dependency
descriptor and `CargoLeanCapability`'s generic `LeanLibraryDependency` hook, so the dependency is described before the
manifest is written. `lean-dup` does not patch manifest JSON after the build and does not consult a sibling
semantic-search checkout.

The generic `lean_rs_interop_shims` Lake package is likewise materialized through the package-owned
`lean-rs-interop-shims` Rust crate. `lean-dup` no longer resolves the interop shim source from a sibling `lean-rs`
checkout during the worker build.
