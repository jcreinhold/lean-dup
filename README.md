# lean-dup

`lean-dup` is a read-only duplication auditor for Lean 4 Lake workspaces.

It indexes declarations from the elaborated Lean environment, then reports likely duplicate or
subsumed statements. It is intentionally local and deterministic: no network services, no
embeddings, and no proof-term analysis in v1.

## Usage

```sh
uv run lean-dup doctor --workspace /path/to/lake/workspace
uv run lean-dup audit --workspace /path/to/lake/workspace --format text
uv run lean-dup audit --workspace /path/to/lake/workspace --format json
uv run lean-dup show --workspace /path/to/lake/workspace --group exact-statement-1
```

By default, `audit` scans the inferred local Lake workspace roots and includes private
theorem-like declarations when Lean exposes them through compiled modules. Use
`--module Root.Module` to restrict the audit to one root module and its descendants.

Useful audit flags:

- `--public-only`: exclude private declarations from the report.
- `--include-imports`: compare workspace declarations with direct imports.
- `--import-root Mathlib.Some.Module`: compare with an additional named module.
- `--threshold 0.82`: adjust the near-duplicate threshold.
- `--profile`: include extraction and classification timings in the report.

Reports are cached under the audited workspace at `.lean-dup/cache/`.
