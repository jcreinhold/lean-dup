# Lean 4.36 migration audit

Recorded 2026-11-17 against clean source revision `93148aa1577a35542a7227290da9e924a706bc07` in an isolated
worktree. The [official Lean 4.36.0 release](https://github.com/leanprover/lean4/releases/tag/v4.36.0) is available at
`611c117bfaf7b7ae1cfcf246ce6fba31e8f509e9`. This audit does not claim that lean-dup is qualified on it.

## Dependency and pin matrix

| Input | Audited value | Migration requirement |
| --- | --- | --- |
| lean-dup package | `0.4.0` | Decide its release version after qualification. |
| Cargo semantic requirements | `lean-semantic-search-{contract,retrieval,runtime,store}`: `0.7` | Select the qualified semantic release and refresh its relevant lock graph. |
| Cargo semantic lock entries | Capability, contract, retrieval, runtime and store `0.7.7`, with registry checksums | Keep a coherent family and verify the selected release's packaged source provenance. |
| Transitive SDK lock entries | `lean-rs-abi`, `lean-rs-worker-protocol`, `lean-toolchain` `0.9.4` | Adopt through the semantic release; there is no direct `lean-rs-*` dependency or Lean FFI in lean-dup. |
| Development Lake require | `lean/lakefile.lean`: semantic Git tag `v0.7.7`, subdirectory `lean` | Adopt the same upstream release as Cargo; verify that tag's Lean package compiles on 4.36. |
| Packaged Lake require | `crates/capability-source/lean/lakefile.lean`: identical tag and source | Update and mirror together with the development source. |
| Rust floor | `1.94` | Change only if required by the selected upstream release. |

The canonical checkout's ignored `lean/lake-manifest.json` resolves the Git require to
`55832816a7b0857ca3d3aa7fe6c3e595f31682a4`, with `inputRev: v0.7.7`, `subDir: lean`, and manifest version
`1.3.0`. No `lake-manifest.json` is tracked here: a clean Git worktree/archive has none. Regenerate the isolated
development manifest after selecting the new tag; do not copy the canonical `.lake` or generated manifest into it.

At that tag, the runtime source records revision `f504ad7616de785fe5cbf6f9d41684f9bd552e23` and digest
`ff539a6d213411332bf3231bae4985567c6c04813b61b4a3ff9bf67630ee35dc`. These are declared source-provenance
values inspected in Git, not a fresh registry-package or native-worker attestation.

All six tracked pins currently equal `leanprover/lean4:v4.35.0`:

- `lean-toolchain`
- `lean/lean-toolchain`
- `tests/fixtures/tiny/lean-toolchain`
- `tests/fixtures/external/lean-toolchain`
- `tests/fixtures/source-backed/lean-toolchain`
- `tests/fixtures/large-type/lean-toolchain`

The `PINNED_TOOLCHAIN` constant, the literal fallback in `ToolchainId::pinned()` and that module's pin-sensitive tests
also name 4.35. Move them in the same change. Update the current requirements in `AGENTS.md`, `README.md`, and
`docs/getting-started.md`, then record the adopted semantic and transitive SDK releases in the Unreleased changelog.
Historical test literals and dated measurements are not all active pins.

## Upstream handoff and worker construction

The semantic 4.36 lane starts from `1ac78f72f2647d74b722eeada53adc8bc5df44a7` (source version `0.7.8`, with
SDK `0.9.10` requirements). The SDK 4.36 lane starts from `a24394e6b5c2330ccf3d81a67cea0bc94b103abc`
(`0.9.10`). Their final revisions, versions and release tag are pending. Do not substitute either working directory
for a qualified release without recording the explicit local override and actual source revision. A possible SDK
`0.10.0` is a pending API/ABI release decision, not the current migration pin.

The SDK's subsequent probe-fix commit `ada669039335eb3616413c07c4adc32ffbcb41ff` still leaves its supported
window at 4.35: on Apple Silicon macOS the 4.36 library lacks required exports `lean_string_data` and
`lean_string_push` (86/88 resolve). The native lean-dup worker does not call those raw SDK symbols, but this prevents
treating the shared SDK/host line as qualified or replacing its ABI with an unchecked fallback.

The semantic preparation branch reached `333c2198f30560c3f3ce662c04c1fbda3325795b`; packaged source revision is
`31cc612937767191748b395ad447d35897244a29`, digest
`ee01ce0d968b9ec34726cae9dfd135c4b1378af31e5a20c41a14ce84ae7b1176`. Its one-file API/JSON/generated-signature
probe passed, while full package tests, Rust qualification, and release/tag selection remain pending. Its Rust version
still reads `0.7.8`, with development Lake metadata `0.7.7`; neither number has been selected as the new release.

There are two distinct native-worker build paths, and both need qualification:

| Path | Source and dependency selection | Required evidence |
| --- | --- | --- |
| Development, `install-worker --source-dir .` | Builds the checkout's `lean/` project with the target sysroot's Lake and the development Git require | Library, executable, isolated manifest, four fixtures and JSONL smoke. |
| Packaged, `install-worker` without `--source-dir` | Copies the vendored LeanDup source and materializes semantic source from the Rust runtime crate; rewrites its Lake require and manifest to that path | Fresh isolated install from packaged source, no sibling checkout or network dependency during materialization, pending-sidecar and version smoke behavior. |

`lean-semantic-search-runtime` generates a toolchain pin for the requested worker during source materialization; its
development pin is not shipped as runtime authority. Consequently, changing lean-dup's development Git require alone
does not update the source used by a registry-installed parent's packaged-worker path. The Cargo runtime payload,
development tag and vendored LeanDup files must all describe the qualified source. The ten vendored LeanDup source
files, including the lakefile, were byte-identical to the development files in this audit.

The native worker imports `LeanSemanticSearch.Canonical`, `.RoleFeatures`, and `.LeanCompat`. It serves JSONL rather
than loading semantic capability exports through the SDK worker pool. The semantic runtime still brings SDK metadata
and protocol crates into the Rust graph; a Rust build that discovers an unsupported 4.36 header can fail its transitive
`lean-toolchain` build script even though the parent never links Lean. The host's raw ABI/export qualification belongs
upstream and does not replace lean-dup's native-worker tests.

Preserve the independent contract versions: semantic protocol `lean-dup.worker.v1`, Rust subprocess substrate `2`,
and report schema `lean-dup.report.v3`. A Lean toolchain bump does not itself justify changing them. The
[4.36 release notes](https://lean-lang.org/doc/reference/latest/releases/v4.36.0/) identify duplicate-key rejection in
`Lean.Json.parse`; `LeanDup.Server.serveLine` calls that parser. Qualify duplicate-key input as the existing
`request.malformed` diagnostic/failed metadata path and verify that the next valid JSONL line still works. Do not add a
parser fallback to restore the old last-key-wins behavior. No `withPtrEq`, Verso docstring parser extension or deprecated
`leanArts` facet usage was found in this repository's Lean source scan.

## Independent prerelease corrections

The migration audit found three gaps in `scripts/prerelease.sh`:

- The script requested `lean_dup_worker`, the package name, instead of the declared executable `lean-dup-worker`.
- Its fixture prebuild omitted `large-type`, which CI and the toolchain-bump workflow include.
- `run_gate` calls `build_lake` inside an `if`, disabling Bash's implicit `errexit` within that function. A failed
  library or earlier fixture build could therefore be hidden by a later successful fixture. Each build now explicitly
  returns its failure.

The isolated change corrects these and updates stale explanatory comments. Its portability comment now accurately says
that `scripts/portability-smoke.sh` builds the parent CLI from an exported tree; that check does not provision a
packaged native worker. No gate was removed or made optional.

## Validation matrix

| Check | Result for this audit | Remaining qualification |
| --- | --- | --- |
| Git state, six pins, default constant, lock families and archive contents | Static checks passed; canonical checkout preserved | Repeat on the integrated release. |
| Vendored/development source equality | All ten files matched | Resync after every Lean or lakefile change and run `vendored_lean_source_matches_dev_project`. |
| `cargo metadata --format-version 1 --no-deps --frozen` | Passed, thirteen workspace members | Workspace metadata only; full dependency resolution and compilation remain pending. |
| `cargo fmt --all -- --check` | Passed | Repeat after source integration. |
| Markdown lint and `git diff --check` | Passed for the audit changes | Keep the audit synchronized with the adopted source pair. |
| Prerelease shell syntax and stubbed build plan | Passed: the declared executable and all four CI fixtures are requested; injected library/fixture failures return status 23 and stop immediately. The original script misses the plan and hides both failures. Zero real Lake invocations. | Real 4.36 Lake builds remain pending. |
| LeanDup library, native executable and four fixtures | Deferred | Use the new isolated manifest and matching 4.36 toolchain. |
| Checkout-source and packaged-source installs | Deferred | Use separate fresh `LEAN_DUP_WORKERS_DIR` directories and verify sidecars plus JSONL version smoke. The override is a bare single-toolchain install directory. |
| Strict Clippy, workspace tests, boundary tests | Deferred | Run with the qualified upstream graph and isolated worker directory. |
| Doctor/report/eval and hard-negative gates | Deferred | Preserve `report.v3`, `worker.v1`, complete/incomplete outcomes and existing quality criteria. |
| Release parent link assertion and archive portability smoke | Deferred | Parent must remain free of `libleanshared`; archive smoke does not cover packaged-worker provisioning. |
| Full `scripts/prerelease.sh` | Deferred | Required before tagging; also inspect the distinct release workflow. |

Obtain a coordinated build lane before substantial compilation and use at most two build jobs. No Rust compilation,
Lean build, worker installation, deployment, push, proof-repository change or Mathlib/cache operation was performed by
this audit. The canonical worker install directory was inspected and empty; no installed worker was started or changed.
