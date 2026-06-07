# Release Hardening

This document defines `G8 release_hardening` in [production-readiness.md](production-readiness.md). It records the
release identity, diagnostics, CI, packaging, and install contract for the 0.1.0 symbolic auditor.

## Design Note

CLI diagnostics own the release identity and operator-facing health report. Project resolution owns workspace, Lake,
selected-root, and toolchain facts. Worker owns protocol version, worker version, semantic algorithm versions, supported
commands, and Lean version. Index owns cache and index schema facts. Report owns the public report schema id. CI owns
reproducible checks. Packaging owns install metadata and user-facing instructions.

The smallest public interface is:

- `lean-dup --version`;
- `lean-dup doctor --workspace <workspace> --format json`;
- README install and basic-use commands that do not require private paths;
- CI jobs for Rust, Lean, fixture eval, report contract, and boundary checks;
- redacted release-diagnostic artifacts under `target/release-diagnostics/`.

Build-script details, Git probing, worker subprocess transport, cache layout, index storage mechanics, and absolute
local paths do not leak into release diagnostics. The preserved user-facing capability is a read-only symbolic audit
binary that can identify itself and explain workspace readiness before a long run. The Python-era behavior intentionally
discarded is relying on ad hoc script names, local checkout paths, or unstructured environment notes to identify what
was run.

## Design It Twice

Three release-diagnostic designs were considered:

- Rely on Cargo metadata and `--help`. Rejected because it does not identify report/index/cache schemas or worker
  compatibility.
- Add ad hoc version strings and doctor prints in CLI command code. Rejected because release identity would be spread
  across parsing, rendering, and workspace diagnostics.
- Make CLI own stable release diagnostics gathered from crate-root facts. Chosen because each crate exposes only its
  status facts while CLI/report own operator presentation.

The chosen boundary is deeper: lower crates keep owning their mechanisms, and release users see one small diagnostic
surface.

## Version Output

`lean-dup --version` succeeds without a workspace and prints:

- package version;
- package name;
- Git revision when the build can determine it, otherwise `unknown`;
- build profile;
- report schema version;
- public index schema label;
- cache-key version;
- instruction to use `doctor` for workspace-dependent Lean worker facts.

Representative output:

```text
lean-dup 0.1.0
package: lean-dup-cli
git revision: 647094d17ac9
build profile: debug
report schema: lean-dup.report.v3
index schema: lean-dup.index.v3
cache key: rust-cli-cache.v1
worker: run `lean-dup doctor --workspace <workspace> --format json` for Lean worker facts
```

The Git revision is a build fact. Release behavior must not depend on invoking Git at runtime.

## Doctor Output

`doctor --format json` includes:

- report schema version;
- release identity facts;
- redacted workspace, Lake, lakefile, and cache-root references;
- selected module roots and source count;
- cache lifecycle diagnostics;
- worker protocol, worker version, Lean version, semantic algorithm versions, supported commands, and supported
  capabilities;
- `require_oleans` state and missing `.olean` diagnostics.

Doctor output uses redacted path references such as:

```json
{ "kind": "workspace-root", "fingerprint": "sha256:07049e02f8629df73d07d007" }
```

It must not expose absolute private paths, cache-entry file names, storage vocabulary, worker rows, or subprocess
transport details. The index crate continues to own the internal persisted schema string; release diagnostics use the
storage-neutral label `lean-dup.index.v3`.

## CI Contract

The CI workflow runs on pull requests, including documentation changes. It checks:

- Lean package and fixture builds;
- `cargo fmt`;
- `cargo clippy --workspace --all-targets --locked -- -D warnings`;
- `cargo test --workspace --locked`;
- CLI boundary tests;
- `lean-dup --version`;
- `doctor --format json` on `tests/fixtures/tiny`;
- default and hard-negative fixture evals;
- ordinary audit report-contract JSON checks.

The report-contract CI check verifies the schema id, bounded emitted group count, and absence of ordinary
`review.groups`.

## Packaging And Install

The core package is `lean-dup-cli`, which installs the `lean-dup` binary. From a checkout:

```sh
cargo install --path crates/cli
lean-dup --version
lean-dup doctor --workspace /path/to/lake/workspace --format json
```

The core symbolic release does not package or depend on vector-search runtime crates through the CLI. Optional tools
follow the external `lean-dup-*` extension convention and must be packaged separately.

Prompt 59 validates source installation with:

```sh
cargo install --path crates/cli --root target/install-smoke --debug --locked --force
target/install-smoke/bin/lean-dup --version
```

Crates.io publication is not claimed by this prompt. Publishing the CLI package there requires publishing the internal
`lean-dup-*` crates in dependency order; the source-install path above is the documented 0.1.0 installation route.

## Prompt 59 Evidence

Prompt 59 generated release diagnostics under `target/release-diagnostics/`:

| Artifact | Purpose |
| --- | --- |
| `version.txt` | `lean-dup --version` output |
| `doctor.json` | fixture `doctor --format json` output |
| `doctor-summary.json` | stable summary of release, worker, cache, and schema facts |
| `target/install-smoke/bin/lean-dup --version` | source-install smoke test |

Leak check:

```sh
rg -n '/Users|index\.sqlite|latest\.json|\bpostings\b|worker row|worker_row|FeatureMatch|IndexQuery|proof_obligation|raw_obligation|backend|tokenizer|lancedb|lance|sqlite|cache layout' \
  target/release-diagnostics/*.json target/release-diagnostics/*.txt
```

Expected result: no matches.

## Verification Commands

```sh
cargo fmt --check
cargo test -p lean-dup-cli --test cli
cargo test -p lean-dup-cli --test boundaries
cargo test
cargo clippy --all-targets -- -D warnings
(cd lean && lake build)
```
