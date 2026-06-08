import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

-- Shared neutral feature extraction (canonical fingerprints, role features) lives
-- in the lean-semantic-search package. Cargo builds materialize this dependency
-- through `lean-semantic-search-runtime` in a private generated Lake root. Direct
-- `lake -d lean build` is a developer check; set `LEAN_DUP_SEMANTIC_SEARCH_ROOT`
-- to a materialized semantic-search runtime source root first.
def semanticSearchRoot : System.FilePath :=
  match run_io IO.getEnv "LEAN_DUP_SEMANTIC_SEARCH_ROOT" with
  | some path => path
  | none => ".lake" / "packages" / "lean-semantic-search-runtime"

require lean_semantic_search from semanticSearchRoot

-- Generic Lean/Rust worker-streaming helpers (callback-envelope mechanics) used by
-- the lean-rs-worker-child capability path: LeanDup is built as a shared capability
-- dylib that the worker child loads. This is the only worker build — Rust drives Lean
-- entirely through the lean-rs-worker-parent pool, not a subprocess.
require «lean_rs_interop_shims» from
  ".." / ".." / "lean-rs" / "crates" / "lean-rs" / "shims" / "lean-rs-interop-shims"

@[default_target]
lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]
  defaultFacets := #[LeanLib.sharedFacet]
