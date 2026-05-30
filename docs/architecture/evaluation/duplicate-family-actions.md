# Duplicate Family Clustering and Actions

This note records the Prompt 74 repair for family-level review and cleanup action semantics.

## Design Note

Pair evidence, family clustering, target selection, action selection, replacement hints, and visibility remain
search-owned knowledge. Eval consumes stable denominators and label traces. Report projects family facts; it does not
recompute visibility, truth, target selection, or action semantics.

The smallest interface exposed upward is a review family: stable family id, representative relation, member summaries,
bounded pair-evidence summaries, target declaration, action, blockers, confidence, and replacement facts. Retrieval
keys, posting shape, scorer weights, source-scan mechanics, worker rows, private paths, and vector facts stay below the
search boundary.

The preserved user-facing capability is bounded ordinary audit output with a forensic `show` path. Ordinary audit now
surfaces the actionable cleanup unit when several pair findings share one coherent target/action. `show` can expand the
selected family evidence without putting unbounded pair arrays into ordinary JSON. The Python-era behavior intentionally
discarded here is asking users to infer duplicate families manually from a flat pair list.

## Design It Twice

Three designs were considered.

1. Keep ordinary reports pair-shaped and rely on users to infer families. This preserves implementation simplicity, but
   it exposes the wrong task. When several aliases point at the same target, the cleanup is one action, not several
   unrelated rows.

2. Cluster every connected component. This is too broad. It can turn diagnostics, hard negatives, or mixed-action
   evidence into a misleading cleanup family merely because pair evidence overlaps.

3. Build action-oriented review families only when one cleanup action can be stated clearly. This is the selected
   design. Search clusters only coherent target/action families and keeps one-pair findings as one-pair families. The
   report crate receives stable family facts and does not learn the clustering algorithm.

## Family Contract

Ordinary audit `visible_groups` are now review families. For a one-pair finding, the family id remains the ranked pair
group id so baseline diffs and existing links stay stable. For a multi-pair family, the id is deterministic and derived
from the action, target, and sorted pair ids.

Saved baselines remain `lean-dup.baseline.v1` and compare pair evidence. That preserves baseline diff stability while
ordinary audit can project coherent visible pair groups as families. A future baseline schema should move to family ids
only if release validation shows pair-level baselines are the wrong review unit.

The ordinary JSON keeps:

- `family_id` and `id`, where `id` is the selected family id;
- `pair_count`, `pair_ids`, and bounded `pair_evidence`;
- `pair_evidence_truncated` when ordinary output omits extra pair summaries;
- the representative relation, action, target, members, evidence, blockers, confidence, and replacement hint.

`show --group <id>` accepts a family id, ranked group id, or pair id. The returned group is the selected family and can
include full pair summaries for that family. Ordinary audit remains bounded by `visible_group_limit`; family summaries
are bounded separately by the pair-summary limit.

## Action Semantics

Prompt 74 only clusters pairs when the action and target give a single coherent cleanup.

- `replace-local-uses`: pairs can form one family when they share the same replacement target.
- `local-alias`: pairs can form one family when they share the same canonical local target.
- `inline-private-helper`: pairs can form one family when they share the same wrapper or target.
- `already-in-mathlib`: pairs can form one family when they share the same mathlib target.
- `merge-generalization`, `specialization-of`, `probable-source-clone`, and `manual-review` stay one-pair families until
  a later prompt defines a safe family action for them.

This avoids connected-component clustering of noisy or mixed-action evidence. It also keeps `local-alias` and
`replace-local-uses` distinguishable even when they occur in the same module or naming pattern.

## Evidence

The focused search tests cover the two important boundary cases:

- two `replace-local-uses` pairs with the same target become one family with two pair ids;
- a `local-alias` pair with the same target remains separate because the action differs;
- ordinary family pair summaries are bounded and marked truncated.

Smoke commands:

```sh
cargo fmt --check
cargo test -p lean-dup-search
cargo test -p lean-dup-report
cargo test -p lean-dup-cli --test cli
cargo test -p lean-dup-cli --test boundaries
cargo test
cargo clippy --all-targets -- -D warnings
(cd lean && lake build)
cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny --no-semantic-probes --format json --low-priority > target/prompt74-tiny-audit.json
env LEAN_DUP_CACHE_DIR=target/prompt74/cache \
  cargo run -p lean-dup-cli -- audit \
  --workspace "$KANPROOFS_WORKSPACE" \
  --module KanProofs \
  --private \
  --format json > target/prompt74/kanproofs-private.json
```

The Tiny smoke artifact contains stable family fields (`family_id`, `pair_count`, `pair_ids`, and
`pair_evidence_truncated`) without raw retrieval rows, worker rows, private absolute paths, or vector facts.

The KanProofs private smoke run emitted `19` visible families from `7,384` review pair groups. Three families contained
two pair findings each; the remaining findings stayed one-pair families. This is the intended conservative behavior:
related alias pairs with one shared target/action are grouped, while mixed-action or unrelated same-module pairs remain
separate. The largest emitted family had `pair_count = 2`, so ordinary output was not pair-summary truncated.

## Red Flag Review

- Shallow module: avoided. Search owns family/action semantics; report receives stable facts.
- Pass-through wrapper: avoided. The repair changes the review unit from pair rows to action families where coherent.
- Temporal decomposition: avoided. The new surface is organized around the review task, not ranking pipeline phases.
- Information leakage: no retrieval keys, scorer weights, worker rows, private paths, source-scan mechanics, or vector
  facts were added to report output.
- Special-general mixture: avoided for now by clustering only target/action families and leaving mixed-action components
  unclustered.
- Conjoined methods: avoided. Visibility, action selection, and pair evidence remain separate facts.
- Hard-to-describe public API: avoided. The report surface is "family, action, target, pair summaries".
- Implementation details contaminating interface comments: avoided. Interface comments describe stable family facts, not
  the grouping algorithm.

## Follow-Up

Prompt 75 should harden source-use and replacement-hint quality for family actions, especially caller impact and
wrapper-only private helper cleanup. Prompt 76 should validate whether family-level review improves KanProofs
actionability without increasing hard-negative leakage.
