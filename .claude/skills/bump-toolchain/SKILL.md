---
name: bump-toolchain
description: Bump lean-dup's pinned Lean toolchain (and the upstream lean-semantic-search release it rides on, which pulls in the lean-rs line transitively). Use whenever the user wants to move lean-dup to a newer Lean release, adopt a new lean-semantic-search / lean-rs version, update the `lean-toolchain` pin, or extend toolchain support — even if they only say "bump the toolchain" without naming the coupled dependency work.
---

# Bump lean-dup's pinned Lean toolchain

Unlike lean-rs (which maintains an ABI-verified *window* of supported toolchains via `supported.rs`, header digests,
and symbol checks), lean-dup pins **one** toolchain. That pin is not chosen freely: it is dictated by the upstream
`lean-semantic-search` release tag lean-dup consumes. The shared `lean-semantic-search` Lean package must compile under
lean-dup's root toolchain, so `lake build LeanDup` only succeeds when lean-dup's pin equals the toolchain that tag
ships. **Bumping the toolchain therefore almost always means bumping the upstream dep in lockstep** — treat that as
the default, not an afterthought.

Since v0.3.0 the worker is a native Lean 4 executable (`lean-dup-worker`, JSONL over stdin/stdout) and lean-dup has
**no direct `lean-rs-*` dependencies**: the `lean-rs` crates (`lean-rs-worker-protocol`, `lean-rs-abi`,
`lean-toolchain`) come in **transitively** through the `lean-semantic-search-*` crates. Adopting a new
`lean-semantic-search` minor is how lean-dup "adopts a new lean-rs version" — check the upstream release notes for
which lean-rs line the tag advances onto.

There is no header/symbol/ABI verification to run here and no CI version matrix to edit (CI reads the single pin from
`lean/lean-toolchain`). The whole job is: pick the upstream release, move every pin that must agree, rebuild, test, and
record it.

## Before you start: identify the target

You need coupled facts, and they come *from upstream*, not from a free choice:

1. The target **Lean toolchain** (e.g. `leanprover/lean4:v4.33.0-rc2`).
2. The `lean-semantic-search` **release tag** that ships under that toolchain (e.g. `v0.7.0`) — its crates.io minor
   for the `lean-semantic-search-*` deps, and the lean-rs line it carries transitively (stated in its release notes).

The published `lean-semantic-search` tag pins the toolchain it was built against (read
`lean/lean-toolchain` at that tag); that is the toolchain lean-dup must adopt. If the user only gives you a Lean
version, find the matching `lean-semantic-search` tag (repo: `github.com/jcreinhold/lean-semantic-search`). If they
only give you an upstream tag, read the toolchain from that tag. Don't proceed until the two line up — a mismatch is
the single most common failure (see *When it fails*).

A pure point-release bump *without* touching upstream is possible only when the shared package still compiles under
the new toolchain (e.g. an rc→rc bump that is header-identical, like the documented v4.33.0-rc1→-rc2 move, which still
took the `v0.7.0` tag because that tag is what ships the rc2-capable lean-rs line). When in doubt, assume the dep
moves too.

## The ritual

### 1. Install the toolchain

```sh
elan toolchain install leanprover/lean4:vX.Y.Z
```

~500 MB; skip if already installed.

### 2. Move the upstream dependency pins (skip only for a pure point bump)

Two places, and they must match each other:

- **Cargo** — in the root `Cargo.toml` `[workspace.dependencies]`, bump the minors:
  `lean-semantic-search-{contract,retrieval,runtime,store}`. Then refresh the lockfile
  (`cargo update -p lean-semantic-search-contract -p lean-semantic-search-retrieval -p lean-semantic-search-runtime
  -p lean-semantic-search-store`) and confirm the transitive `lean-rs-*` crates moved to the expected line.
- **Lake** — in `lean/lakefile.lean`, bump the single git-require tag:
  `«lean-semantic-search» … @ "vA.B.C"`. Update the inline comment that names the matching toolchain so the next
  reader sees why the tag and the pin agree.

The Cargo minor and the Lake tag describe the same upstream release — keep them consistent.

### 3. Update every `lean-toolchain` pin (all six must agree)

CI's comment is the contract: every `lean-toolchain` file in the repo holds the same version. There are six:

- `lean-toolchain` (root)
- `lean/lean-toolchain` (authoritative — CI and `install-worker` read this one; it is the toolchain a no-`--toolchain`
  worker build targets)
- `tests/fixtures/tiny/lean-toolchain`
- `tests/fixtures/external/lean-toolchain`
- `tests/fixtures/source-backed/lean-toolchain`
- `tests/fixtures/large-type/lean-toolchain`

A stale fixture pin splits the build and produces confusing `lake` errors, so set them all in one pass. (Confirm the
set with `find . -name lean-toolchain -not -path '*/.lake/*' -not -path '*/target/*'` in case fixtures were added.)

There is also **one Rust constant** that must equal the `lean/lean-toolchain` pin: `PINNED_TOOLCHAIN` in
`crates/worker/src/toolchain.rs` (the default toolchain when no project `lean-toolchain` is found, and the install-dir
label fallback). Update its value, and update the same literal in the `ToolchainId::pinned()` fallback a few lines
below it and in that file's tests. The test `ToolchainId::pinned().elan_label() == PINNED_TOOLCHAIN` keeps them
honest. (lean-dup has no hardcoded `lean.h` digest to bump — `install-worker` hashes the header at build time and
records it in the worker's `worker.json` sidecar.)

### 3a. Resync the vendored capability Lean source

`install-worker` builds the `lean-dup-worker` executable from a **byte-identical vendored copy** of the dev project
that ships inside the published crate: `crates/capability-source/lean/`. The editable source under `lean/` is the
source of truth; mirror it after any Lean change a toolchain bump pulls in (including the lakefile — the vendored
copy carries the same git-require tag). The drift test tracks `lakefile.lean`, `Main.lean`, `LeanDup.lean`, and the
`LeanDup/` tree:

```sh
cp lean/lakefile.lean lean/Main.lean lean/LeanDup.lean crates/capability-source/lean/ \
  && rm -rf crates/capability-source/lean/LeanDup && cp -R lean/LeanDup crates/capability-source/lean/LeanDup
```

The drift test `vendored_lean_source_matches_dev_project` (in `crates/capability-source/src/lib.rs`) fails the build
if the copy diverges, so a forgotten resync is caught by `cargo test`, not at a user's `install-worker`.

### 4. Update the Rust floor only if upstream raised it

The `lean-rs` / `lean-semantic-search` crates pin a `rust-version`; adopting a new minor can raise lean-dup's floor
(check the upstream tag's `Cargo.toml`; e.g. lean-rs sat at 1.91 through the 0.7 line). If it moved, bump
`rust-version` in `Cargo.toml` `[workspace.package]` and update the matching prose in `README.md` and
`docs/getting-started.md`. If upstream didn't move it, leave it alone.

### 5. Rebuild and test

Mirror CI's pre-build order (see `.github/workflows/ci.yml`): the Rust tests shell out to `lake` and need the
`.olean` artifacts present first.

```sh
# Build the LeanDup library (resolves the new git-require tag)
( cd lean && lake build LeanDup )
# Build each fixture so the Rust tests find its .olean files
for f in tiny external source-backed large-type; do ( cd tests/fixtures/$f && lake build ); done
# Provision the native worker for the new pin (lake build of the lean-dup-worker
# executable + JSONL `version` smoke test) into a repo-local dir, mirroring
# ci.yml. The test suite also provisions on first run, but doing it explicitly
# surfaces build errors early.
LEAN_DUP_WORKERS_DIR="$PWD/target/lean-dup-workers" \
  cargo run -p lean-dup --locked -- install-worker --source-dir . --toolchain "$(tr -d '[:space:]' < lean/lean-toolchain)"
LEAN_DUP_WORKERS_DIR="$PWD/target/lean-dup-workers" cargo test --workspace --locked
```

(The full test command set is in AGENTS.md's "Commands" section.)

`install-worker` writes a *pending* `worker.json` sidecar before the smoke test and overwrites it with the outcome —
the smoke run resolves the worker through the parent's runtime path, which refuses a worker with no sidecar, so a
fresh `LEAN_DUP_WORKERS_DIR` must still pass (this was a real bug fixed in v0.3.0; if it regresses, a fresh-dir
provision fails with "no lean-dup worker is installed").

The capability is built by `install-worker` at provision time, not at `cargo build` time, so a plain `cargo build`
(and `cargo install lean-dup`) is Lean-free.

Also run the lint gate, since CI treats warnings as errors:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

If the cache-key or schema asserts in the CLI integration suite shift because the worker substrate version moved,
that's expected — refresh goldens only if the change is the intended additive one, never to paper over a real diff.

### 6. Update docs and the CHANGELOG

- Bump the toolchain string in the three prose locations: `AGENTS.md` ("The pinned Lean toolchain is …"), `README.md`
  ("Requirements"), and `docs/getting-started.md` (the requirements table). Grep for the *old* version to catch any
  other prose mention: `grep -rIn '<old-version>' --exclude-dir=target --exclude-dir=.lake .` (ignore test fixtures'
  source and historical `crates/**` test literals and dated validation docs — those record past runs and are not pins).
- Add a `### Changed` bullet under `## [Unreleased]` in `CHANGELOG.md` naming the new toolchain and the upstream
  release tag adopted (plus the transitive lean-rs line), mirroring the 0.3.0 entry's style.
- If the bump adds or removes any workspace crate, audit `.github/workflows/release.yml` for stale crate names — the
  v0.3.0 tag push failed because it still built/published the deleted `lean-dup-worker-child`. `prerelease.sh`
  mirrors `ci.yml`'s check job only; it does **not** exercise `release.yml`'s verify job.

### 7. Commit

Commit message in the repo's style,
e.g. `Bump Lean toolchain to vX.Y.Z and lean-semantic-search deps to vA.B.C`. Summarize in the body: the new
toolchain, the upstream tag (and the transitive lean-rs line), any Rust-floor change, and the test result.

## When it fails

The failure modes here are about *pin alignment*, not ABI drift — don't reach for version-specific shims or
allowlists.

| Symptom | Cause | Action |
| --- | --- | --- |
| `lake build LeanDup` fails to compile the shared package | lean-dup's toolchain ≠ the toolchain the `lean-semantic-search` tag was built under | Re-derive the toolchain from the upstream tag's `lean-toolchain`; align step 3's pins to it. Do **not** bump the toolchain ahead of an upstream release that supports it. |
| `install-worker` smoke test fails with "no lean-dup worker is installed" on a fresh workers dir | The pending-sidecar write before the smoke test regressed (see step 5) | Restore the pending-sidecar write in `crates/cli/src/install_worker.rs`; the smoke run resolves through the same path the parent uses. |
| Worker spawns but fails under `lake env` in a fixture/audit | A `lean-toolchain` pin is stale, so the executable runs under the wrong toolchain | Make all six pins equal (step 3) and rebuild the fixtures + worker. |
| Cargo resolves an unexpected upstream version, or `--locked` fails | Cargo minor and Lake git-require tag disagree, or `Cargo.lock` wasn't refreshed | Reconcile the Cargo minor with the Lake tag (step 2); run `cargo update -p <crate> --precise <x.y.z>` then `cargo build --locked`. |
| A Rust test fails only on the new toolchain | Likely an upstream behavior change | Reproduce minimally; if it's an upstream regression, raise it with the lean-semantic-search / lean-rs maintainers rather than pinning around it in lean-dup. |
