# Prompt 15 Performance Pass

For the current as-built architecture around these historical measurements, see
[../06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

The new internal performance layer owns workload names, cache-state setup, timing labels, memory snapshots, worker event
attribution, SQLite counters, retrieval counters, and report serialization. Its smallest public interface is the hidden
developer command `lean-dup-rs perf --workload ... --format json --output ...`.

Audit, index, retrieval, ranking, and rendering callers must not learn SQLite table names, JSONL framing policy,
cache-key layout, worker build caching, or Lean expression traversal details. The preserved user-facing capability is
the existing Rust/Lean audit/index path: normal commands and output formats remain unchanged. The intentionally
discarded Python-era behavior is script-shaped, manually interpreted profiling output and project-local one-off cache
conventions.

## Design It Twice

Rejected: ad hoc shell scripts plus manual notes. This is shallow because cache deletion, workload spelling, timing
labels, and skip decisions leak into every command line. It also makes before/after claims hard to reproduce.

Chosen: an internal perf harness plus private instrumentation. Existing commands keep their interfaces, and the harness
runs named workloads with stable cost labels. This is deeper because the caller requests a workload result, not a
sequence of SQLite probes, cache checks, Lake invocations, worker calls, and report parsing steps.

## Checked-In Harness

The hidden command is:

```sh
cargo run -q -p lean-dup-cli -- perf --workload cold-mathlib-index --cache-root target/lean-dup-perf/mathlib-cache --output target/lean-dup-perf/reports/cold-mathlib-index.json
cargo run -q -p lean-dup-cli -- perf --workload warm-mathlib-index --cache-root /Users/jcreinhold/.cache/lean-dup --output target/lean-dup-perf/reports/warm-mathlib-index-home-cache.json
cargo run -q -p lean-dup-cli -- perf --workload kanproofs-targeted-mathlib --cache-root target/lean-dup-perf/kanproofs-targeted-cache --output target/lean-dup-perf/reports/kanproofs-targeted-mathlib.json
cargo run -q -p lean-dup-cli -- perf --workload kanproofs-full-no-mathlib --cache-root target/lean-dup-perf/kanproofs-full-no-mathlib-cache --output target/lean-dup-perf/reports/kanproofs-full-no-mathlib.json
```

The harness defaults to `/Users/jcreinhold/Code/kan-proofs` and `target/lean-dup-perf/cache`. Mathlib workloads now use
the audited project's pinned `.lake/packages/mathlib` by default; `--mathlib-workspace` is only an explicit source-root
override. Normal commands still default to the common cache root `~/.cache/lean-dup` unless `LEAN_DUP_CACHE_DIR` is set.

After the first baseline pass, `lake update` was run in all three relevant Lake workspaces:

```sh
cd /Users/jcreinhold/Code/lean-dup/lean && lake update
cd /Users/jcreinhold/Code/kan-proofs && lake update
cd /Users/jcreinhold/Code/mathlib4 && lake update
```

The lean-dup worker workspace was already up to date. KanProofs updated its mathlib dependency and downloaded the
matching cache. The concrete KanProofs backport modules then built successfully. A full `lake build Mathlib` in
`/Users/jcreinhold/Code/mathlib4` was intentionally stopped at 1254/8435 targets because it was broader than this pass
and not required to validate the lean-dup worker.

## Project-Centered Mathlib Correction

The root-cause follow-up changed the `index-mathlib` boundary from standalone mathlib-workspace indexing to
project-centered dependency indexing. The module now owns Lake package discovery, the pinned mathlib source root, the
worker execution root, source-span attribution, module batching, and SQLite finalization. The public command surface is
still one request: build the mathlib comparison index for this project.

Rejected: running `lean-dup` inside `/Users/jcreinhold/Code/mathlib4` or requiring shell scripts to assemble the right
Lake/cache state. That leaks transport, package-layout, and cache policy to the operator.

Chosen: resolve the local project's `.lake/packages/mathlib`, run the worker through the local project's `lake env`, and
attribute rows to the pinned mathlib source files. This is deeper because audit and index callers do not learn whether
the implementation uses JSONL, module batches, SQLite temp files, or source-root overrides. The Python-era behavior
intentionally discarded here is treating mathlib as a separate global workspace that callers must manually point at for
every project.

## Accepted Optimization

Measured bottleneck: repeated `lake build lean_dup_worker` calls inside one Rust process. This was wasted work, not a
Lean semantic cost. The subprocess transport now serializes worker builds and caches the resolved worker binary path
in-process. An escape hatch, `LEAN_DUP_DISABLE_WORKER_BUILD_CACHE=1`, exists only to reproduce the before number.

| Workload      | Build cache | Wall ms | Worker startup/build ms | Lean import ms | Lean semantic ms | SQLite ms | Retrieval/ranking ms | Report ms | Candidates | Hydrated |
| ------------- | ----------: | ------: | ----------------------: | -------------: | ---------------: | --------: | -------------------: | --------: | ---------: | -------: |
| fixture audit |    disabled |    1584 |                    1534 |            173 |               32 |         5 |                    5 |         8 |        260 |       42 |
| fixture audit |     enabled |     840 |                     791 |            175 |               33 |         5 |                    6 |         9 |        260 |       42 |

Result: 744 ms wall-time reduction on the fixture audit, with the same 260 candidates and 42 hydrated declarations. This
follows the POSD intervention order by removing repeated work and pulling the build-cache policy down into the owning
transport module instead of exposing a build/cache knob to callers.

## Mathlib Throughput Refactor

The next measured bottleneck was the mathlib feature path itself. The first project-centered implementation split
mathlib into small Rust-side module batches; that repeatedly paid Lean import/environment setup and hit heartbeat
recovery around `Mathlib.Algebra.Category.*`. The stopped run reached only about 202/8060 modules after about 22
minutes.

The first import-once implementation fixed repeated imports but still spent almost all sampled CPU in two avoidable
places: repeated `forallTelescope` opening in feature/canonical row construction, and string/Nat-heavy stable hashing
inside the canonical serializer. A sample of the long run showed about 842.7 MB physical footprint, 870.2 MB peak, and
the main thread dominated by `LeanDup.Features.featureRows`, `forallTelescope`, and `LeanDup.Canonical.stableHash`.
That run reached about 79000/312711 declarations after about 19.6 minutes.

Accepted changes:

- Pull telescope ownership into Lean feature extraction: `featureRows` now opens each declaration telescope once and
    passes the opened context to canonical fingerprinting.
- Replace string/Nat hash folding in hot semantic keys with a bounded `UInt64` FNV-style non-cryptographic hash. These
    keys are opaque semantic-index keys, not security boundaries.
- Move parallelism into the Lean worker instead of Rust process sharding. Rust sends one private `index` request; the
    worker imports the project-pinned mathlib environment once, enumerates accepted declarations once, and schedules
    declaration chunks with `IO.asTask`/`IO.waitAny'` inside that shared environment.
- Use `LEAN_NUM_THREADS` as the single operator-facing concurrency knob. Rust clamps it to the current safe maximum of
    2, sends the effective value as private `declaration_parallelism`, and sets the worker subprocess's
    `LEAN_NUM_THREADS` to the same effective value.
- Keep one SQLite writer path in Rust. Parallel tasks stream rows back through one worker stdout stream; SQLite locking
    and row finalization policy do not leak upward.

Rejected after measurement: Rust-side multi-worker sharding. A 180 s exploratory run with two worker subprocesses
processed about 213440/312711 declarations, but it duplicated the imported mathlib environment and made memory pressure
a Rust orchestration policy. It was useful as a probe, not a design to keep.

Current prefix measurements:

| Workload prefix                                  | Concurrency     | Import ms | Enumerate ms | Progress at cutoff     | Cutoff wall |
| ------------------------------------------------ | --------------- | --------: | -----------: | ----------------------: | ----------: |
| import-once before hot-path fixes                | serial          |     22900 |        26400 | 79000/312711            |   ~1175 s   |
| after telescope/hash fixes                       | serial          |     25644 |        29110 | 106368/312711           | 179.697 s   |
| Rust two-worker sharding probe                   | 2 subprocesses  |     21966 |        25366 | ~213440/312711 combined |  ~180 s     |
| single worker, Lean internal tasks               | `LEAN_NUM_THREADS=2` | 18069 |        21647 | 77024/312711            | 74.698 s    |

The single-worker internal-task path is the chosen design even where the short prefix is not always faster than the
two-process probe, because it preserves the deep boundary: one imported environment, one worker protocol request, one
writer, and no caller-visible shard/cache/source-root policy. It also leaves the next measured improvement in the right
place: Lean-owned scheduling can become adaptive by declaration cost without changing Rust or the CLI.

## Realistic Workload Baselines

The elapsed class totals are instrumentation events, not an additive critical path. `worker.subprocess_call` is
subprocess wall time and therefore contains Lean import and Lean semantic work. For transport-backend classification,
the useful raw numbers are subprocess wall, JSON/JSONL encode/parse time, stdin/stdout bytes, and Lean progress events.

| Workload                        | Cache state                                               | Exit | Wall ms |  Peak RSS | Lean import ms | Lean semantic ms | JSON/JSONL ms | Stdout bytes/lines |   SQLite ms | Candidates | Hydrated | Probe batches |
| ------------------------------- | --------------------------------------------------------- | ---: | ------: | --------: | -------------: | ---------------: | ------------: | -----------------: | ----------: | ---------: | -------: | ------------: |
| cold mathlib index              | isolated cold cache                                       |    1 |  285231 | 886947840 |           7477 |           218136 |          5180 | 285520023 / 312611 | not reached |       null |     null |          null |
| warm mathlib index              | common `~/.cache/lean-dup`, stale for current fingerprint |    1 |  329540 | 886898688 |          21700 |           249839 |          5073 | 285520024 / 312611 | not reached |       null |     null |          null |
| KanProofs targeted with mathlib | isolated reuse-or-build                                   |    1 |  371755 | 884572160 |          29093 |           263662 |          5330 | 285608723 / 312653 |           7 |       null |       17 |          null |
| KanProofs full without mathlib  | isolated reuse-or-build                                   |    1 |   71276 |  31424512 |          14546 |             3973 |           105 |     6931085 / 6046 | not reached |       null |     null |          null |

The full KanProofs audit with mathlib was skipped after the targeted mathlib workload proved the same mathlib
comparison-index failure at 371755 ms and the cold mathlib index failed at 285231 ms. Running the larger workload would
spend another multi-minute pass on the same failing prerequisite before reaching audit retrieval, ranking, probes, or
rendering.

After `lake update`, the targeted KanProofs mathlib workload was rerun against the common home cache:

| Workload                                      | Cache root           | Exit | Wall ms |  Peak RSS | Lean import ms | Lean semantic ms | JSON/JSONL ms | Stdout bytes/lines | SQLite ms | Hydrated |
| --------------------------------------------- | -------------------- | ---: | ------: | --------: | -------------: | ---------------: | ------------: | -----------------: | --------: | -------: |
| KanProofs targeted with mathlib after update  | `~/.cache/lean-dup`  |    1 |  395278 | 675807232 |          34213 |           262923 |          5122 | 285608723 / 312653 |         6 |       17 |

This did not hit a reusable current mathlib SQLite index. The home cache contains previous mathlib index directories,
but they were stale for the current workspace fingerprint or semantic version, so the command rebuilt the mathlib
comparison side and failed at the same worker boundary.

## Cost Classification

Worker process startup and build: fixture measurements proved repeated worker builds were a local bottleneck. The
accepted change removed two redundant builds in the fixture audit. In mathlib workloads, worker build is no longer the
bottleneck; `worker.build_process` is under 1 s, while subprocess wall is hundreds of seconds.

Lean import/session/environment setup: mathlib import/setup was 7477 ms in the cold isolated run and 21700 ms in the
stale-home-cache run. Full KanProofs without mathlib spent 14546 ms importing.

Transport/framing JSON/JSONL: JSONL parse was 5143 ms for 285.5 MB and 312608 lines in the corrected cold mathlib run.
This is measurable but not dominant compared with 218136 ms of Lean semantic work.

Lean semantic work: the dominant measured cost in mathlib indexing was semantic feature extraction, 218136 ms cold and
249839 ms in the stale home-cache run.

SQLite/index work: fixture SQLite write/hydrate took 5 ms. Targeted KanProofs wrote and hydrated 17 local declarations
in 7 ms. Mathlib failed before SQLite write, so cache invalidation or SQLite write tuning is not justified by these
numbers.

Retrieval and ranking: fixture retrieval/ranking took 5 to 6 ms. Large workloads failed before candidate retrieval, so
there is no measured basis for retrieval or ranking optimization in this pass.

Report rendering: fixture rendering took 8 to 9 ms. Large workloads failed before report rendering.

## FFI Gate

ffi_spike_recommended = false

Raw support: in the corrected cold mathlib index run, JSON/JSONL encode/parse took 5180 ms, while Lean semantic work
took 218136 ms and Lean import took 7477 ms. The subprocess wall includes Lean work, so it does not by itself justify
FFI. After build caching, worker build/startup is not dominant on completed fixture work either: fixture wall dropped to
840 ms, with two build cache hits and unchanged candidate output.

Prompt 19 remains skipped. The next useful optimization is not a `lean-rs` backend; it is diagnosing why the feature
worker exits nonzero after emitting large mathlib/full-KanProofs extract output, then measuring again once the realistic
workloads complete. The new streaming path already keeps structured diagnostics when a subprocess exits nonzero after
emitting JSONL error envelopes.

## POSD Review

The optimization defines a special case out of existence: callers no longer pay repeated `lake build lean_dup_worker`
calls during a single process. It pulls complexity down into the subprocess transport, which owns worker invocation
policy. It optimizes the abstraction boundary before micro-tuning by keeping JSONL, SQLite, and cache internals private
and exposing only stable cost classes through the hidden harness.

Rejected optimizations: no retrieval cache knobs, SQLite invalidation changes, Rust-side process sharding, or FFI
transport were added. The measured workloads either show tiny retrieval/SQLite costs, or show the bottleneck inside
Lean semantic traversal. Parallelism was added only behind the Lean-owned indexing boundary after measuring the serial
critical path.

## Residual Risk

The report still contains prefix measurements for the new parallel path rather than a completed cold mathlib index.
That is enough to validate the architecture change and live progress, but a full cold/warm pass should be rerun after
the next semantic hot-path improvement. The worker transport now prefers structured worker diagnostics over generic
nonzero-exit errors when a failing subprocess emitted JSONL diagnostics.

## Semantic Probe Recovery Follow-Up

A later full KanProofs audit exposed the next bottleneck after the shared mathlib index completed. The old semantic
probe path sent a broad candidate set to Lean and failed the whole audit when one pair hit the default heartbeat budget
inside `whnf`/`isDefEq`:

```text
worker returned a fatal diagnostic: internal_error fatal: declaration processing failed: (deterministic) timeout at `whnf`, maximum number of heartbeats (200000) has been reached
```

The first no-probe full audit also showed that report construction was still not useful: it spent more than three
minutes in Rust source-reference scanning and ranking after indexes were loaded, with no user-visible progress.

Accepted changes:

- Added a private semantic-verification boundary that owns probe selection, budgets, per-declaration caps, chunking,
  cache keys, worker recovery, and diagnostics.
- Moved expensive probes after cheap ranking and shaped the default mathlib queue before source-reference scanning.
- Made the default mathlib profile hide feature-only subsumption candidates and unprobed non-theorem shape collisions.
- Added hidden tuning flags: `--probe-budget`, `--probe-policy`, and `--probe-chunk-size`.
- Cached probe results with declaration fingerprints and a probe-policy version rather than only the pair id.
- Preserved pair-local Lean failures as `status = "unavailable"` where possible, and used Rust chunk bisection to isolate
  recoverable worker failures.
- Follow-up proof-grade pass: ranking now consumes typed semantic evidence instead of raw worker probe booleans. Static
  fingerprints can plan probe obligations, but default mathlib output requires verified evidence before showing
  actionable findings.

Rejected design: exposing cache, chunk, or Lean reduction controls directly in normal audit workflows. That would make
callers know which failures are heartbeat failures, which candidates are mathlib-source candidates, and which SQLite
cache identity is safe. The chosen design is deeper because audit asks for verified evidence under a review policy; the
semantic-verification module owns the policy mechanics.

Second rejected design: keep the bounded pair verifier as the ranking interface. It recovered from heartbeat failures,
but it still leaked worker result fields into ranking and allowed callers to conflate "same static fingerprint" with
"Lean verified this obligation." The deeper design introduces private proof obligations and typed semantic evidence:
worker rows are cacheable transport facts; ranking sees only verified, rejected, or unavailable review evidence. The
proof-grade requirement is applied to project-pinned `--compare-mathlib`; explicit `--compare-index` inputs retain the
older static-index behavior because the external index source may not be importable for Lean probes.

Measured follow-up workloads:

| Workload | Cache state | Result | Wall observation | Retrieval ms | Raw candidates | Review groups | Visible groups | Planned probes | Cached probes | Worker probes | Unavailable | Verified |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| full KanProofs with mathlib, old broad probes | warm mathlib | failed | n/a | n/a | n/a | n/a | n/a | broad batch | 0 | broad batch | fatal heartbeat | n/a |
| full KanProofs with mathlib, old no-probe path | warm mathlib | manually stopped | >180 s | n/a | n/a | n/a | n/a | 0 | 0 | 0 | n/a | n/a |
| full KanProofs with mathlib, bounded probes after fix | warm mathlib | ok | ~48 s | 22955 | 464707 | 3403 | 0 | 177 | 143 | 34 | 70 | n/a |
| full KanProofs with mathlib, no probes after shaping | warm mathlib | ok | ~15 s | 16647 | 464707 | 3403 | 0 | 0 | 0 | 0 | 0 | n/a |
| targeted `KanProofs.Mathlib4Backports` with mathlib | warm mathlib | ok | ~5 s | 2567 | 1360 | 0 | 0 | 0 | 0 | 0 | 0 | n/a |
| targeted `KanProofs.Mathlib4Backports`, proof-grade pass | warm mathlib | ok | ~12 s | 3501 | 1360 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| full KanProofs with mathlib, proof-grade pass | warm mathlib, probe cache invalidated by semantic key change | ok | ~94 s | 28007 | 464707 | 0 | 0 | 177 | 0 | 177 | 70 | 0 |

The visible result count of zero is intentional for the default mathlib profile. The raw retrieval layer still found
feature overlaps, but the actionable queue rejected them because they were not theorem-level or verified replacement
candidates. A short-lived intermediate run classified unrelated inductive and definition declarations as exact
statement matches from static fingerprints alone; this was rejected as bullshit and fixed by requiring theorem-like
declarations for unprobed statement equivalence. The proof-grade pass tightens this further: mathlib static fingerprints
are planning evidence only, and reducible definitions require verified Lean evidence before they can become visible
review findings.

The full proof-grade pass planned only reducible-definition obligations: 177 planned, 0 verified, 70 unavailable, 61 of
those unavailable because the declaration was not available in the imported probe environment. That is useful negative
evidence: the default KanProofs mathlib queue currently contains no proof-grade duplicates, so the report stays empty
instead of rendering a large weak-candidate queue. The remaining performance cost is mostly retrieval/indexing and the
177 Lean probe pairs; the next optimization target is not broad parallel probing, but avoiding or explaining unavailable
reducible-definition obligations earlier.

POSD intervention mapping:

- Define errors out of existence: one heartbeat-limited pair no longer makes the audit command fail; it becomes an
  unavailable semantic-evidence result or an isolated recoverable diagnostic.
- Pull complexity down: probe budgeting, chunk bisection, cache-key construction, and mathlib module import selection
  are internal to semantic verification.
- Optimize the abstraction boundary before micro-tuning: source-reference scanning now runs over the shaped review
  queue instead of every workspace declaration in the default mathlib workflow.
- Remove work before tuning: the default profile stops building hints, source facts, and JSON-visible findings for
  feature-only mathlib overlaps.
- Hide volatile decisions: ranking consumes `SemanticEvidence`; worker status strings, obligation choice, pair chunking,
  and cache identity no longer leak into review classification.

## Red Flag Review

Shallow module: no remaining red flag. The hidden harness owns workload and metric policy rather than exposing it across
modules.

Pass-through wrapper: no remaining red flag. The harness adds cache setup, metrics collection, memory snapshots, and
report emission.

Temporal decomposition: no remaining red flag. The transport owns build reuse at the worker boundary; callers do not
sequence build/check/invoke manually.

Information leakage: no remaining red flag. Public comments describe caller-facing behavior, not SQLite schema, cache
keys, or Lean traversal.

Special-general mixture: no remaining red flag. `fixture-audit` is a test workload; realistic workload names remain
separate.

Conjoined methods: no remaining red flag. Metric recording is scoped and private; normal audit/index methods keep their
existing responsibilities.

Hard-to-describe public API: no remaining red flag. The only new interface is the hidden
`perf --workload ... --format json --output ...` command.

Implementation details contaminating interface comments: no remaining red flag. Public comments state stable
expectations and hide storage/transport policy.
