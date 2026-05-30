# Kind and Low-Signal Policy Repair

This note records the Prompt 73 repair for declaration-kind and low-signal review policy.

## Design Note

Declaration kind classification, low-signal markers, generated-shape detection, candidate-source facts, scorer
calibration, and visibility policy are search-owned knowledge. Eval consumes stable stage denominators and label traces.
Report projects review facts; it does not reinterpret kind, marker, scorer, or candidate-source internals.

The smallest interface exposed upward is a stable set of review facts: declaration kind, declaration visibility, status
flags, leak-safe blocker labels, review priority, confidence, relation, and stage denominators. Raw Lean expressions,
low-level feature keys, scorer weights, retrieval keys, worker rows, private paths, and vector facts stay out of report
and eval artifacts.

The preserved user-facing capability is the conservative default cleanup queue plus composable widening flags:
`--private`, `--low-priority`, and `--diagnostics`. Private helper cleanup remains possible without classifying private
helpers as noise. The Python-era behavior intentionally discarded here is a flat syntactic list where every private,
generated, or low-signal pair is either hidden for the same reason or shown as undifferentiated noise.

## Design It Twice

Three designs were considered.

1. Keep blunt blockers for private, generated, low-signal, and typeclass cases. This preserves precision, but it loses
   useful distinctions: strong private helpers become noise, verified definition duplicates cannot become actionable,
   and low-signal exact/permutation evidence is indistinguishable from broad-head overlap.

2. Let diagnostics mode handle every edge case. This keeps the default queue small, but it makes ordinary users find
   real cleanup work through a debugging mode. That is a shallow UI: the user has to know implementation vocabulary to
   ask for useful findings.

3. Make search own kind-aware policy with focused fixtures and stage denominators. This is the selected design. Search
   keeps marker extraction and classification private, while eval observes stable facts and denominators. Low-signal
   evidence can be non-noise without becoming default cleanup; generated and typeclass noise remain diagnostic.

## Policy

The repaired policy separates scope, priority, and diagnostics.

- `non-public-declaration` remains a blocker fact. It does not demote an otherwise strong pair to noise. Default audit
  hides it through visibility options; `--private` can show it.
- `low-signal-declaration` remains a blocker fact. Strong statement-plus-permutation support can rank as low-priority
  review instead of noise, but low-signal groups are not default visible.
- `generated-declaration` remains diagnostic. Search now recognizes stable Lean-generated declaration names such as
  `recOn` and `casesOn` even when the worker did not attach a generated status flag.
- `typeclass-instance-noise`, `broad-head-only`, unverified non-theorem static evidence, and unverified proof-grade
  static evidence still force diagnostic priority.
- Verified reducible definition evidence can remove `non-theorem-static-only` and produce an actionable exact-statement
  group without exposing definition bodies or worker internals.

## Focused Fixtures

The search unit suite now covers:

- theorem and axiom classification as theorem-like;
- definition, abbrev, and opaque classification as definition-like;
- structure and class classification as data-type declarations;
- constructor, projection, and recursor classification as constructor/projection-like declarations;
- `inst...` declarations as typeclass instance noise;
- low-signal exact statement plus permutation as low-priority, not noise;
- weak low-signal role overlap as diagnostic;
- generated Lean recursor/cases declarations hidden by stable name shape;
- broad predicate-head-only overlap as diagnostic;
- verified reducible definition pairs as actionable;
- unverified definition pairs as diagnostic.

## Evidence

Before artifacts came from `target/scorer-calibration/`. After artifacts are under `target/kind-policy/`.

| Workload | Before | After | Result |
| --- | --- | --- | --- |
| `default` eval | precision `8/8`, visible positives `8/16`, hard negatives `0/3`, visible groups `7/39` | precision `8/8`, visible positives `8/16`, hard negatives `0/3`, visible groups `7/39` | Default precision and hard-negative behavior preserved. |
| `hard-negatives` eval | precision `1/8`, visible positives `1/1`, hard negatives `0/5`, visible groups `7/39` | precision `1/8`, visible positives `1/1`, hard negatives `0/5`, visible groups `7/39` | Zero visible hard-negative leakage preserved. |
| `manual-internal` eval | blocked; resolved positives `1/6`; visible positives `0/6`; visible queue `0/4`; hard negatives `0/3`; visible groups `8/7840` | blocked; resolved positives `1/6`; visible positives `0/6`; visible queue `0/4`; hard negatives `0/3`; visible groups `8/7840` | Prompt 73 does not repair stale labels or replacement-candidate visibility. |
| KanProofs private audit | prior Prompt 67 private audit emitted `5` visible groups; early Prompt 73 draft emitted `75`, including generated recursor/cases groups | final private audit emits `22` visible groups and `0` visible `recOn`/`casesOn` groups | Generated private recursors are removed from private cleanup output. |

The current manual-internal resolved positive `FirstOrder.SetTheory.ZFC.ZFCModel.models_delta0Theory` /
`FirstOrder.SetTheory.ZFC.instZFSetModelsDelta0Theory` is still generated and ranked at rank `4`, but remains hidden by
review policy. That loss is not caused by stale label resolution, declaration kind, generated-name shape, or low-signal
policy. It remains a replacement/actionability policy issue for a later prompt.

## Red Flag Review

- Shallow module: avoided. Search owns kind and low-signal policy; eval/report receive stable facts.
- Pass-through wrapper: avoided. The repair changes search-owned decisions and focused fixtures, not just names.
- Temporal decomposition: avoided. Policy is organized by review facts and user-visible intent, not by processing step.
- Information leakage: no raw expressions, worker rows, retrieval keys, scorer weights, private paths, cache layout, or
  vector facts were added to committed artifacts.
- Special-general mixture: partially present by necessity in generated-name recognition. The names are stable Lean
  generated declaration shapes, and the rule stays private to search policy.
- Conjoined methods: avoided. Scope flags, priority, and diagnostics remain separate mechanisms.
- Hard-to-describe public API: avoided. Publicly visible concepts remain blockers, priority, confidence, and queue
  visibility.
- Implementation details contaminating interface comments: avoided. Interface comments describe review facts, not marker
  extraction or ranking constants.

## Follow-Up

Prompt 74 should decide how replacement-candidate and duplicate-family actions surface, including the current ZFC manual
positive. Prompt 76 should include the private-audit `22` group result when deciding whether the symbolic search repair
sequence is sufficient without vector search.
