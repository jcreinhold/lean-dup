# Repo Patterns

## Preferred verification commands

Prefer `cargo nextest run` when nextest is installed; otherwise `cargo test`.

Common commands:

```bash
cargo nextest run -p <crate>
cargo test -p <crate>            # fallback
make test                        # broader spans if a Makefile target exists
```

## Common test locations

- integration tests: `tests/` or `tests/it/`
- unit tests: inline `mod tests`
- benches: `benches/`

## Common test shapes

- domain math/logic: laws, boundary conditions, negative cases
- registry/storage: roundtrip, ordering, identity, conflict detection
- pipeline passes: preservation and semantic equivalence
- CLI/tooling: visible behavior and persisted state

## Prefer nearby authorities

Before writing tests, inspect:

- nearby docs/spec or architecture notes
- nearby issue or regression context
- existing tests in the same crate
- existing benches before inventing a new perf surface
