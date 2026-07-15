---
name: release-lean-dup
description: Cut a lean-dup release — run the pre-release gate, bump the workspace version, update the CHANGELOG, and push a signed tag. Use when releasing lean-dup, bumping the workspace version for a release, or cutting a vX.Y.Z tag.
---

# Release lean-dup

This skill is the release checklist plus the cross-file invariants that are easy to get wrong. Reversible prep (steps
1–4) is free; **step 5 (tag push) is irreversible — stop and get explicit human confirmation before running it.**

> **Release mechanism — read this first.** lean-dup **is** a crates.io workspace: as of the `v0.2.2` release,
> `.github/workflows/release.yml` is tag-triggered (`v[0-9]+.[0-9]+.[0-9]+` or `-rc`-style prereleases). Pushing the
> tag runs the full `verify` gate, then a Lean-free `publish` job uploads every crate to crates.io in dependency order
> (idempotent — a crate already at that version on crates.io is skipped, since crates.io versions are immutable) and
> creates a GitHub Release via `softprops/action-gh-release`, whose body is the matching `## [X.Y.Z]` section of
> `CHANGELOG.md`. Releases before `v0.2.2` (`v0.2.0`, `v0.2.1`) were tag-only, with nothing published — the workflow
> didn't exist yet. Do **not** run `cargo publish` by hand; the tag push does it. `workflow_dispatch` with `dry_run:
> true` exercises the gates without publishing or creating a release, useful for testing the workflow itself without
> burning a crates.io version.

## Steps

### 1. Pre-flight gate

```sh
scripts/prerelease.sh            # mirrors ci.yml's check job; --quick skips the slow eval/audit fixtures
```

Stop on any failure. This is the same gate CI runs (fmt, clippy `-D warnings`, the full test suite, `boundaries`, and
the `jq` schema/eval/report-contract assertions); passing locally is the fast feedback loop.

### 2. Version bump (one source of truth, mirrored in two places)

Pick the new `X.Y.Z` (patch unless the change is breaking/feature — it is pre-1.0, so breaking changes bump the minor).
In the root `Cargo.toml`, set **both**:

- `[workspace.package].version = "X.Y.Z"`
- every internal `[workspace.dependencies]` `lean-dup-*` entry's `version = "X.Y.Z"` (all ten versioned path crates —
  `capability-source`, `diagnostics`, `embedding`, `eval`, `index`, `project`, `report`, `search`, `vector-index`,
  `worker` — share the workspace version; `lean-dup-test-support` has no `version` field and is unaffected, since it's
  a path-only dev-dependency stripped from published crates).

A half-updated version is an inconsistency; bump them together.

### 3. CHANGELOG

Move the `## [Unreleased]` entries into a new `## [X.Y.Z] - YYYY-MM-DD` section (compose fresh if empty). The heading
version must match the tag exactly: tag `v0.2.0` → heading `## [0.2.0]`. Leave a fresh empty `## [Unreleased]` at the
top.

### 4. Schema-contract check (lean-dup-specific)

If this release changed any `lean-dup.*.vN` schema (report, worker protocol, index, cache key), confirm the bump landed
in **three** places already, in the commits being released:

- the version constant (e.g. `crates/report/src/report_contract.rs` `REPORT_SCHEMA_VERSION`),
- the matching `jq` assertion in `.github/workflows/ci.yml`, and
- the relevant `docs/architecture/*.md`.

CI asserts the exact strings (`lean-dup.report.v3`, `lean-dup.worker.v1`); a drift here fails the check job. If they are
out of sync, fix before tagging.

### 5. Land the version bump, then tag — irreversible

Default to a PR with the version + CHANGELOG changes, merged after `ci.yml` is green; push directly to `main` only if
the human explicitly says to skip the PR. Either way, before tagging, re-verify on the commit that will be tagged:

- `git rev-parse --abbrev-ref HEAD` is `main` and up to date with `origin/main`.
- the `[workspace.package].version` and every `[workspace.dependencies]` `lean-dup-*` version match the intended
  `X.Y.Z`.
- `CHANGELOG.md` has a `## [X.Y.Z]` heading.

**Confirm with the human, then push the tag** (the irreversible step — this triggers `release.yml`, which publishes to
crates.io and cannot be undone once a crate version is live):

```sh
git tag -s vX.Y.Z -m "lean-dup vX.Y.Z"   # -s signed (preferred), or -a unsigned annotated
git push origin vX.Y.Z
```

Tags containing `-` (e.g. `vX.Y.Z-rc.1`) are conventionally prereleases.

### 6. Post-tag

- Confirm the tag is on the intended commit: `git show vX.Y.Z --stat | head`.
- The tag push triggers `.github/workflows/release.yml` automatically. Watch it: `gh run list --workflow=release.yml
  --limit 1` to get the run ID, then `gh run watch <id> --exit-status`. Confirm the `publish` job succeeded and the
  GitHub Release was created (its body is generated from the `## [X.Y.Z]` CHANGELOG section — reviewing the release
  page is a quick way to catch a malformed CHANGELOG heading after the fact).
- If `publish` fails partway, don't bump to a new version to "fix" it. The `publish` job is idempotent — each crate is
  skipped once its version is already on crates.io — so re-running the same run via `gh run rerun <id> --failed`
  resumes where it left off.
