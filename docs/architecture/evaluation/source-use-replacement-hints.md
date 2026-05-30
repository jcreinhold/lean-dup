# Source-Use Replacement Hints

Prompt 75 hardens replacement hints beyond unqualified token scanning. The ordinary audit still stays read-only:
replacement hints describe bounded caller impact, importability, and action-specific uncertainty, but they do not prove
that a rewrite is semantically safe and they do not perform source edits.

## Design note

Source-use analysis owns token-aware, bounded source scans, import discovery, path fingerprinting, and caller scan
status. Search owns action-specific hint facts: whether a review family is a public replacement, local alias,
inline-private-helper, same-module cleanup, or source-backed external comparison. Report owns projection only. It
renders stable facts and bounded caller snippets; it does not recompute callers, importability, action, target
selection, or semantic safety.

The smallest public interface is:

- `SourceFacts`: import status, source fingerprints, bounded caller references, and reference-scan status;
- `ReplacementHint`: target declaration/module, import status, caller-impact category, bounded caller count, displayed
  callers, truncation status, notes, and blockers;
- report DTO fields that project those facts with path redaction and bounded source text.

The design keeps scanner heuristics, parser limitations, worker rows, proof obligations, cache layout, retrieval keys,
private paths, and vector facts out of the report surface. The preserved user-facing capability is concise cleanup
guidance for duplicates found by symbolic audit. The discarded Python-era behavior is treating source-shaped text
matches as enough to imply replacement safety.

## Design It Twice

Three designs were considered:

1. Keep token-scanned caller hints and patch individual bad actions. This is shallow: each new bad hint would require
   another renderer or token-scan exception, while users would still see overconfident guidance.
2. Suppress replacement hints for all private or same-module cases. This avoids mistakes by hiding useful information.
   It loses the private-helper workflow that `--private` now makes explicit.
3. Make search own action-specific hint facts with bounded source-use evidence and explicit uncertainty. This is the
   selected design. Source scanning stays private and bounded, search classifies caller impact and importability, and
   report shows concise guidance without claiming proof of rewrite safety.

## Stable hint facts

Caller impact is one of:

- `no-callers`: no local callers were found in the bounded scan;
- `wrapper-only`: the only local caller evidence is inside the public wrapper of a private helper;
- `bounded-callers`: callers were found and displayed up to the ordinary report bound;
- `truncated-callers`: the caller scan hit its configured bound;
- `unknown-callers`: source-use facts were not requested or source files were unavailable;
- `missing-source`: a declaration has no source location.

Importability is one of:

- `direct`: the target module is already available or the replacement is same-module;
- `missing`: an import is needed before replacing local uses;
- `unknown`: importability could not be established from loaded source;
- `source-backed-not-importable`: the evidence came from source-backed external comparison that is not importable in the
  current workspace.

The report also carries `callers_truncated` so tools can distinguish a bounded displayed list from a complete caller
list. Notes give action-specific guidance. Blockers mark missing imports, non-importable external evidence, missing
source, unknown caller impact, and truncated scans.

## Prompt 67 examples

The Prompt 67 KanProofs private audit produced three useful hint shapes:

- an `inline-private-helper` family with one caller inside the public wrapper;
- `local-alias` families with zero local callers;
- a `replace-local-uses` family with five bounded callers.

Those examples remain the smoke workload. The repaired hint contract keeps these actions distinct. In particular,
wrapper-only private helpers do not receive generic "replace uses" deletion guidance, local aliases can report
`no-callers`, and multi-caller replacements carry bounded caller facts instead of pretending the scan is proof of
safety.

## Fixtures and behavior

Focused fixtures cover:

- wrapper-only private helpers, which become `inline-private-helper` with `wrapper-only` impact;
- alias-first findings with `no-callers` guidance and no deletion wording;
- same-module replacements, which report `direct` importability;
- missing imports, which become explicit blockers;
- bounded caller truncation, which reports `truncated-callers` and `callers_truncated`;
- comments and string literals, which are ignored by source-use token scanning;
- non-importable source-backed evidence, which reports `source-backed-not-importable`;
- missing source spans, which report `missing-source`.

No fixture treats token-scanned callers as proof that a rewrite is semantically safe. The scanner is a bounded
observability mechanism; Lean semantic evidence and review action policy remain separate.

## Verification evidence

Checks run for this prompt:

- `cargo test -p lean-dup-search`
- `cargo test -p lean-dup-report`
- `cargo test -p lean-dup-cli --test cli`
- `cargo test -p lean-dup-cli --test boundaries`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `(cd lean && lake build)`

An operator-supplied KanProofs private smoke audit also completed with `status=ok`, `visible_group_count=19`, and
`visible_groups_emitted=19`. Every visible group had a replacement hint. The output included `wrapper-only`,
`no-callers`, `bounded-callers`, and `truncated-callers` impact states, plus `direct` and `missing` import states. The
smoke artifact is under `target/source-use-replacement-hints/` and is intentionally not a checked-in release artifact.

## Red Flag Review

- Shallow module: addressed by putting caller-impact and importability semantics in search-owned hint facts rather than
  renderer conditionals.
- Pass-through wrapper: avoided; report projects DTO fields and does not derive hint semantics.
- Temporal decomposition: avoided; source scanning, search action semantics, and report rendering are split by hidden
  knowledge, not by execution order.
- Information leakage: addressed by excluding scanner internals, raw paths, worker rows, proof obligations, retrieval
  keys, cache layout, and vector facts from reports.
- Special-general mixture: local aliases, inline private helpers, import blockers, and external non-importability are
  separate stable facts, not overloaded notes.
- Conjoined methods: hint creation remains read-only guidance and does not perform rewriting.
- Hard-to-describe public API: caller impact and importability are small closed vocabularies.
- Implementation details contaminating interface comments: public comments describe stable facts and limits, not token
  scanning or parser mechanics.
