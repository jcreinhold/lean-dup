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
uv run lean-dup audit --workspace /Users/jcreinhold/Code/kan --compare-mathlib --progress
uv run lean-dup show --workspace /path/to/lake/workspace --group exact-statement-1
uv run lean-dup show --workspace /Users/jcreinhold/Code/kan --group exact-statement-1
```

By default, `audit` scans the inferred local Lake workspace roots and includes private
theorem-like declarations when Lean exposes them through compiled modules. Use
`--module Root.Module` to restrict the audit to one root module and its descendants.

Useful audit flags:

- `--public-only`: exclude private declarations from the report.
- `--include-imports`: compare workspace declarations with direct imports.
- `--import-root Mathlib.Some.Module`: compare with an additional named module.
- `--compare-mathlib`: compare workspace declarations with the cached mathlib index.
- `--threshold 0.82`: adjust the near-duplicate threshold.
- `--profile`: include extraction and classification timings in the report.
- `--no-replacement-hints`: skip import/replacement hint generation.

Reports are cached under the audited workspace at `.lean-dup/cache/`.
For confirmed external matches, text and JSON reports include read-only replacement hints:
the target declaration, the specific import line, direct-import status, and bounded local
source references to replace or preserve behind a transitional alias.
