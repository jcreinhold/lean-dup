---
name: release-lean-dup
description: Cut a lean-dup release — run the pre-release gate, bump the workspace version, update the CHANGELOG, and push a signed tag. Use when releasing lean-dup, bumping the workspace version for a release, or cutting a vX.Y.Z tag.
---

# Release lean-dup

This skill is the release checklist plus the cross-file invariants that are easy to get wrong.
Reversible prep (steps 1–4) is free; **step 5 (tag push) is irreversible — stop and get explicit
human confirmation before running it.**

> **Release mechanism — read this first.** Unlike `lean-rs`, lean-dup is an application/CLI, **not a
> crates.io workspace, and there is no `.github/workflows/release.yml` yet.** So today a release is
> the **local gate + version bump + CHANGELOG + signed git tag** — the tag is the release marker;
> nothing is published to a registry. Do **not** run `cargo publish`. If/when artifact distribution
> is wanted (a GitHub Release with the `lean-dup-cli` binary + Lean worker, built on tag push), that
> is a separate follow-up: add a `release.yml` and then extend step 5 to watch it. Until then, keep
> the scope to tagging.

## Steps

### 1. Pre-flight gate

```sh
scripts/prerelease.sh            # mirrors ci.yml's check job; --quick skips the slow eval/audit fixtures
```

Stop on any failure. This is the same gate CI runs (fmt, clippy `-D warnings`, the full test suite,
`boundaries`, and the `jq` schema/eval/report-contract assertions); passing locally is the fast
feedback loop.

### 2. Version bump (one source of truth, mirrored in two places)

Pick the new `X.Y.Z` (patch unless the change is breaking/feature — it is pre-1.0, so breaking
changes bump the minor). In the root `Cargo.toml`, set **both**:

- `[workspace.package].version = "X.Y.Z"`
- every internal `[workspace.dependencies]` `lean-dup-*` entry's `version = "X.Y.Z"` (all nine path
  crates — `diagnostics`, `embedding`, `eval`, `index`, `project`, `report`, `search`,
  `vector-index`, `worker` — share the workspace version).

A half-updated version is an inconsistency; bump them together.

### 3. CHANGELOG

Move the `## [Unreleased]` entries into a new `## [X.Y.Z] - YYYY-MM-DD` section (compose fresh if
empty). The heading version must match the tag exactly: tag `v0.2.0` → heading `## [0.2.0]`. Leave a
fresh empty `## [Unreleased]` at the top.

### 4. Schema-contract check (lean-dup-specific)

If this release changed any `lean-dup.*.vN` schema (report, worker protocol, index, cache key),
confirm the bump landed in **three** places already, in the commits being released:

- the version constant (e.g. `crates/report/src/report_contract.rs` `REPORT_SCHEMA_VERSION`),
- the matching `jq` assertion in `.github/workflows/ci.yml`, and
- the relevant `docs/architecture/*.md`.

CI asserts the exact strings (`lean-dup.report.v3`, `lean-dup.worker.v1`); a drift here fails the
check job. If they are out of sync, fix before tagging.

### 5. PR, merge, then tag — irreversible

Open a PR with the version + CHANGELOG changes; merge after `ci.yml` is green. Before tagging,
re-verify on the merge commit:

- `git rev-parse --abbrev-ref HEAD` is `main` and up to date with `origin/main`.
- the `[workspace.package].version` and every `[workspace.dependencies]` `lean-dup-*` version match
  the intended `X.Y.Z`.
- `CHANGELOG.md` has a `## [X.Y.Z]` heading.

**Confirm with the human, then push the tag** (the irreversible step):

```sh
git tag -s vX.Y.Z -m "lean-dup vX.Y.Z"   # -s signed (preferred), or -a unsigned annotated
git push origin vX.Y.Z
```

Tags containing `-` (e.g. `vX.Y.Z-rc.1`) are conventionally prereleases.

### 6. Post-tag

- Confirm the tag is on the intended commit: `git show vX.Y.Z --stat | head`.
- If a `release.yml` exists later, `gh run watch` it and confirm the GitHub Release/artifacts.
