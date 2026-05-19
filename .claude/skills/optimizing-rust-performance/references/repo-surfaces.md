# Repo Measurement Surfaces

Before inventing new measurement infrastructure, find what the current repo already provides. The job is to use what
exists, then extend it only where a real gap blocks the work.

## Locating existing benches

Most Rust workspaces put benches under `benches/` directories per crate. Find them with:

```bash
rg --files crates | rg '/benches/'
fd -t f -e rs . crates/*/benches 2>/dev/null
```

Skim each bench file to see what it actually measures: hot inner loops, end-to-end pipeline throughput, allocation
profiles via DHAT, or regression guards for a previously-fixed bug. Common families to expect:

- **Microbenches** at the function or small-module level. Useful for localizing a bottleneck; can mislead if reused as
  the only proof of an end-to-end win.
- **Pipeline / throughput benches** that drive a representative workload through multiple stages. Slower to run, but
  the right tool when the change crosses pass boundaries.
- **Regression benches** that pin specific paths (equality, lookup, env access). Good guards once a fix lands.
- **DHAT or heaptrack benches** for allocation-aware measurement when the change is about memory pressure rather than
  wall time.

## Locating existing profiling support

Some Rust workspaces ship a dedicated profiling crate (a binary or a set of binaries) that orchestrates broader
workloads under a profiler such as `samply`, `perf`, `pprof-rs`, or `dhat`. Find it with:

```bash
rg -l 'criterion_group|with_profiling|ProfilerGuard|dhat' .
fd -t d profiling 2>/dev/null
```

Typical contents:

- a `profile_<workload>` binary per representative scenario (frontend-only, full build, interactive editor, parse-only,
  etc.);
- a baseline collector that captures timing plus allocations under DHAT;
- helper shell scripts for `samply record` / `samply load` / `pprof -http` orchestration.

Outputs usually land under a results directory like `profiling_results/` or `target/perf/`.

## Existing in-source profiling hooks

Some hot paths instrument themselves with a thin observer or guard pattern. Find them with:

```bash
rg -n 'profiling|with_profiling_observer|on_stage_exit|ProfilerGuard|dhat|criterion_group' .
```

Common shapes:

- a `with_profiling_observer(...)` wrapper at a pass boundary;
- per-stage `on_stage_exit(...)` hooks that emit timing events;
- conditional `#[cfg(feature = "dhat-heap")]` blocks that enable allocation tracing under a feature flag.

## What is usually missing or fragmented

In most Rust workspaces, the measurement surface grows organically and ends up uneven:

- Microbenches are stronger than throughput benches.
- A shared profiling crate, when it exists, is stronger than ad-hoc CLI options.
- Some hot areas still rely on comments or one-off benches rather than a shared regression suite.

Implication for a perf task:

- Start with the closest existing bench.
- If the change might affect end-to-end throughput, also run a broader pipeline or profiling workload.
- If a hot path lacks a stable reproducer, add one in the nearest crate bench first; only escalate to the shared
  profiling crate if a new shared workload is genuinely warranted.

## When to add measurement support

Add or expand a bench or profiling surface when:

- the suspected hot path has no stable reproducer;
- the only current bench measures the wrong thing (e.g. setup work instead of steady-state);
- a change touches a pass boundary, cache, or invalidation policy that microbenches cannot cover;
- reviewers would otherwise have no credible way to detect regressions later.
