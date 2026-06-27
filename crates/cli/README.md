# lean-dup

A read-only duplication auditor for Lean 4 Lake workspaces. It indexes declarations from the *elaborated* Lean
environment and reports likely duplicate or subsumed statements. The default audit path is local and deterministic: no
network, no embeddings, no proof-term analysis.

## Install

```sh
cargo install lean-dup
```

`cargo install lean-dup` ships the auditor Lean-free. The Lean worker that reads your project's `.olean` files is built
on your machine, once per toolchain you audit:

```sh
# Run inside your Lake project (uses its lean-toolchain), or pass --toolchain.
lean-dup install-worker
```

## Use

```sh
lean-dup doctor                       # check workspace, cache, and worker health
lean-dup audit --workspace . --module MyLib
```

The worker is resolved per audited project from its `lean-toolchain` pin; if one is not installed, `lean-dup` prints the
exact `install-worker` command to run.

See the [repository](https://github.com/jcreinhold/lean-dup) for the full documentation, architecture notes, and the
worker protocol.
