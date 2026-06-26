---
name: bump-toolchain
description: Bump lean-dup's pinned Lean toolchain (and the upstream lean-rs / lean-semantic-search releases it rides on). Use whenever the user wants to move lean-dup to a newer Lean release, adopt a new lean-rs / lean-semantic-search version, update the `lean-toolchain` pin, or extend toolchain support — even if they only say "bump the toolchain" without naming the coupled dependency work.
---

# Bump lean-dup's pinned Lean toolchain

Unlike lean-rs (which maintains an ABI-verified *window* of supported toolchains via `supported.rs`, header digests,
and symbol checks), lean-dup pins **one** toolchain. That pin is not chosen freely: it is dictated by the upstream
`lean-semantic-search` and `lean-rs` release tags lean-dup consumes. The shared `lean-semantic-search` Lean package must
compile under lean-dup's root toolchain, so `lake build LeanDup` only succeeds when lean-dup's pin equals the toolchain
those release tags ship. **Bumping the toolchain therefore almost always means bumping the upstream deps in lockstep** —
treat that as the default, not an afterthought.

There is no header/symbol/ABI verification to run here and no CI version matrix to edit (CI reads the single pin from
`lean/lean-toolchain`). The whole job is: pick the upstream release, move every pin that must agree, rebuild, test, and
record it.

## Before you start: identify the target

You need three coupled facts, and they come *from upstream*, not from a free choice:

1. The target **Lean toolchain** (e.g. `leanprover/lean4:v4.32.0`).
2. The `lean-semantic-search` **release tag** that ships under that toolchain (e.g. `v0.4.0`) — and its crates.io minor
   for the `lean-semantic-search-*` deps.
3. The `lean-rs` **release tag** for `lean-rs-interop-shims` (e.g. `v0.2.3`) — and its crates.io minor for the
   `lean-rs-worker-*` / `lean-rs-interop-shims` deps.

The published `lean-semantic-search` tag pins the toolchain it was built against; that is the toolchain lean-dup must
adopt. If the user only gives you a Lean version, find the matching `lean-semantic-search` / `lean-rs` tags (their repos
are `github.com/jcreinhold/lean-semantic-search` and `github.com/jcreinhold/lean-rs`). If they only give you upstream
tags, read the toolchain from those tags' `lean-toolchain`. Don't proceed until all three line up — a mismatch is the
single most common failure (see *When it fails*).

A pure point-release bump *without* touching upstream is possible only when the shared package still compiles under the
new toolchain (e.g. an rc→rc bump that is header-identical, like the documented v4.31.0-rc1→-rc2 move). When in doubt,
assume the deps move too.

## The ritual

### 1. Install the toolchain

```sh
elan toolchain install leanprover/lean4:vX.Y.Z
```

~500 MB; skip if already installed.

### 2. Move the upstream dependency pins (skip only for a pure point bump)

Two places, and they must match each other:

- **Cargo** — in the root `Cargo.toml` `[workspace.dependencies]`, bump the minors:
  `lean-semantic-search-{contract,retrieval,runtime,store}` and
  `lean-rs-{worker-parent,worker-child,worker-protocol,interop-shims}`.
- **Lake** — in `lean/lakefile.lean`, bump the two git-require tags:
  `«lean-semantic-search» … @ "vA.B.C"` and `«lean_rs_interop_shims» … @ "vD.E.F"`. Update the inline comment that names
  the matching toolchain so the next reader sees why the tag and the pin agree.

The Cargo minor and the Lake tag describe the same upstream release — keep them consistent.

### 3. Update every `lean-toolchain` pin (all six must agree)

CI's comment is the contract: every `lean-toolchain` file in the repo holds the same version. There are six:

- `lean-toolchain` (root)
- `lean/lean-toolchain` (authoritative — CI and the worker `build.rs` read this one)
- `tests/fixtures/tiny/lean-toolchain`
- `tests/fixtures/external/lean-toolchain`
- `tests/fixtures/source-backed/lean-toolchain`
- `tests/fixtures/large-type/lean-toolchain`

A stale fixture pin splits the build and produces confusing `lake`/loader errors, so set them all in one pass. (Confirm
the set with `find . -name lean-toolchain -not -path '*/.lake/*' -not -path '*/target/*'` in case fixtures were added.)

### 4. Update the Rust floor only if upstream raised it

The `lean-rs` worker crates pin a `rust-version`; adopting a new minor can raise lean-dup's floor (it went 1.85 → 1.91
with the 0.2.0 pool crates). If so, bump `rust-version` in `Cargo.toml` `[workspace.package]` and update the matching
prose in `README.md` and `docs/getting-started.md`. If upstream didn't move it, leave it alone.

### 5. Rebuild and test

Mirror CI's pre-build order (see `.github/workflows/ci.yml`): the Rust tests shell out to `lake` and need the `.olean`
artifacts present first.

```sh
# Build the LeanDup shared-facet capability (resolves the new git-require tags)
( cd lean && lake build LeanDup )
# Build each fixture so the Rust tests find its .olean files
for f in tiny external source-backed large-type; do ( cd tests/fixtures/$f && lake build ); done
# Full workspace suite (CLAUDE.md "Test" section)
cargo build -p lean-dup-worker-child --locked
cargo test --workspace --locked
```

Also run the lint gate, since CI treats warnings as errors:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

If the cache-key or schema asserts in the CLI integration suite shift because the worker substrate version moved, that's
expected — refresh goldens only if the change is the intended additive one, never to paper over a real diff.

### 6. Update docs and the CHANGELOG

- Bump the toolchain string in the three prose locations: `CLAUDE.md` ("The pinned Lean toolchain is …"), `README.md`
  ("Requirements"), and `docs/getting-started.md` (the requirements table). Grep for the *old* version to catch any
  other prose mention: `grep -rIn '<old-version>' --exclude-dir=target --exclude-dir=.lake .` (ignore test fixtures'
  source and historical `crates/**` test literals and dated validation docs — those record past runs and are not pins).
- Add a `### Changed` bullet under `## [Unreleased]` in `CHANGELOG.md` naming the new toolchain and the upstream
  release tags adopted, mirroring the 0.2.0 entry's style.

### 7. Commit

Branch first if you're on `main`. Commit message in the repo's style, e.g.
`Bump Lean toolchain to vX.Y.Z and lean-rs/lean-semantic-search deps`. Summarize in the body: the new toolchain, the two
upstream tags, any Rust-floor change, and the test result.

## When it fails

The failure modes here are about *pin alignment*, not ABI drift — don't reach for version-specific shims or allowlists.

| Symptom | Cause | Action |
| --- | --- | --- |
| `lake build LeanDup` fails to compile the shared package | lean-dup's toolchain ≠ the toolchain the `lean-semantic-search` tag was built under | Re-derive the toolchain from the upstream tag's `lean-toolchain`; align step 3's pins to it. Do **not** bump the toolchain ahead of an upstream release that supports it. |
| `libleanshared.so: cannot open shared object file` (loader error at worker startup) | A `lean-toolchain` pin is stale, so the worker-child build baked an rpath to an uninstalled prefix | Make all six pins equal (step 3) and rebuild `lean-dup-worker-child`. |
| Cargo resolves an unexpected upstream version, or `--locked` fails | Cargo minor and Lake git-require tag disagree, or `Cargo.lock` wasn't refreshed | Reconcile the Cargo minor with the Lake tag (step 2); run `cargo update -p <crate> --precise <x.y.z>` then `cargo build --locked`. |
| A Rust test fails only on the new toolchain | Likely an upstream behavior change | Reproduce minimally; if it's an upstream regression, raise it with the lean-rs / lean-semantic-search maintainers rather than pinning around it in lean-dup. |
