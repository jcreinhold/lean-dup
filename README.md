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

Use `--module Root.Module` to restrict the audit to one root module and its descendants.

Reports are cached under the audited workspace at `.lean-dup/cache/`.
