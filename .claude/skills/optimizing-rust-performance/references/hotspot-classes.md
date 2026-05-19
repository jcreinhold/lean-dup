# Hotspot Classes

The bottleneck classes that recur in Rust workspaces with compiler-like or interpreter-like cores. Names and paths
will vary; the shapes do not.

## Arena And Phase-Local Allocation

Start here when you see many short-lived vectors, slices, or strings.

Find arenas with:

```bash
rg -n 'arena::|bumpalo|typed_arena|TypedArena|Arena<' .
```

What to look for:

- building temporary `Vec`s only to copy into arena slices
- re-allocating scratch buffers inside loops instead of clearing and reusing
- storing data past a phase boundary when it should die with the arena
- using store/load or owned clones as a borrow workaround instead of fixing lifetime structure

## Normalization, Evaluation, And Read-Back

In codebases with an interpreter or normalizer, these are core hot paths.

Find them with:

```bash
rg -n 'normalize|whnf|nf_at|quote|readback|trampoline' --type rust .
```

Typical bottlenecks:

- repeated closure forcing
- unnecessary quoting or normalization when weak-head work would suffice
- environment growth or copying under binders
- recursive or pointer-heavy traversals in hot steady-state loops

## Traversal Cost

Traversal shows up both directly and as overhead inside other passes.

Find traversals with:

```bash
rg -n 'fold|walk|visit|travers' --type rust .
```

Typical bottlenecks:

- repeated full traversals where a cached or incremental fact would do
- collecting transient child vectors instead of using an explicit stack or iterator
- recomputing structure facts on every pass

## Typechecker, Unifier, Constraint Queue, Metas

This is one of the most important hotspot families in compiler-style codebases.

Find them with:

```bash
rg -n 'unif|constraint|dispatch|meta|infer|synthesis' --type rust .
```

Typical bottlenecks:

- repeated normalization during dispatch
- constraint deduplication or wake-up overhead
- meta-solution lookup churn
- cloning definitions, environments, or metadata at retry boundaries
- persistent structure costs in deep or write-heavy paths

## Registry, Metadata, Side Tables, Cache Lookups

These costs often hide behind "small" operations executed everywhere.

Find them with:

```bash
rg -n 'Registry|registry|metadata|side_table|cache::|Cache<' --type rust .
```

Typical bottlenecks:

- `HashMap<Vec<PathSegment>, ...>` style keys on hot paths
- repeated registry cloning or cache-key cloning across phase boundaries
- converting cheap IDs into expensive path or string keys too early
- cold metadata bloating hot structs instead of living in a side table

## Closure Capture And Environment Representation

Codebases with closures and environments in multiple layers pay for both.

Find them with:

```bash
rg -n 'CapturedEnv|Closure|closure|env|imbl::|Vector|push_back' --type rust .
```

Typical bottlenecks:

- persistent vectors helping lookup but hurting repeated writes
- copying captured environments at instantiation or quoting boundaries
- storing more in every closure than the hot path actually needs

## Pipeline Throughput And Pass Boundaries

Micro wins can lose here.

Find pass boundaries with:

```bash
rg -n 'pipeline|pass|stage|module_cache|compilation_pipeline' --type rust .
```

Typical bottlenecks:

- pass-local clones that scale with module count
- repeated arena setup or source-map allocation
- invalidation that forces broad recomputation
- data converted to a new representation at each pass when a stable handle would do

## Persistent Structures And Immutable Data

Persistent collections are not free. They help when sharing dominates copying, but they hurt when mutation depth
dominates.

Find them with:

```bash
rg -n 'imbl::|Vector<|HashMap<|HashSet<|SmallVec<' --type rust .
```

Questions to ask:

- Is the hot operation mostly reads, appends, random lookups, or full clones?
- Is structural sharing paying for itself?
- Would an arena slice, dense index storage, or reusable `Vec` be cheaper for this phase?
