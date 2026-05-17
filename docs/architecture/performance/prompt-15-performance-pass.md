# Prompt 15 Performance Pass

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
cargo run -q -p lean-dup-rs -- perf --workload cold-mathlib-index --cache-root target/lean-dup-perf/mathlib-cache --output target/lean-dup-perf/reports/cold-mathlib-index.json
cargo run -q -p lean-dup-rs -- perf --workload warm-mathlib-index --cache-root /Users/jcreinhold/.cache/lean-dup --output target/lean-dup-perf/reports/warm-mathlib-index-home-cache.json
cargo run -q -p lean-dup-rs -- perf --workload kanproofs-targeted-mathlib --cache-root target/lean-dup-perf/kanproofs-targeted-cache --output target/lean-dup-perf/reports/kanproofs-targeted-mathlib.json
cargo run -q -p lean-dup-rs -- perf --workload kanproofs-full-no-mathlib --cache-root target/lean-dup-perf/kanproofs-full-no-mathlib-cache --output target/lean-dup-perf/reports/kanproofs-full-no-mathlib.json
```

The harness defaults to `/Users/jcreinhold/Code/kan-proofs`, `/Users/jcreinhold/Code/mathlib4`, and
`target/lean-dup-perf/cache`. Normal commands still default to the common cache root `~/.cache/lean-dup` unless
`LEAN_DUP_CACHE_DIR` is set.

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
workloads complete.

## POSD Review

The optimization defines a special case out of existence: callers no longer pay repeated `lake build lean_dup_worker`
calls during a single process. It pulls complexity down into the subprocess transport, which owns worker invocation
policy. It optimizes the abstraction boundary before micro-tuning by keeping JSONL, SQLite, and cache internals private
and exposing only stable cost classes through the hidden harness.

Rejected optimizations: no retrieval cache knobs, SQLite invalidation changes, parallelism, or FFI transport were added.
The measured workloads either show tiny retrieval/SQLite costs or fail before those phases, so those changes would leak
internals or add policy without evidence.

## Residual Risk

The report currently captures top-level stderr and output tails, but a nonzero worker exit still compresses the root
cause to `worker exited with status 1` at the Rust CLI boundary. That is a diagnostics limitation, not an optimization
target. It should be fixed before the next performance pass so failed worker envelopes preserve their structured Lean
error payloads.

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
