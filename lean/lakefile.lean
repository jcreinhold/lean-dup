import Lake
open Lake DSL

package «lean-dup-worker» where
  version := v!"0.1.0"

-- Shared neutral feature extraction (canonical fingerprints, role features) lives
-- in the lean-semantic-search package. lean-dup imports it rather than keeping a
-- second copy; the subprocess worker protocol stays lean-dup's own.
require «lean-semantic-search» from ".." / ".." / "lean-semantic-search" / "lean"

-- Generic Lean/Rust worker-streaming helpers (callback-envelope mechanics) used by
-- the lean-rs-worker-child capability path. Spike: building LeanDup as a shared
-- capability dylib that the worker child loads.
require «lean_rs_interop_shims» from
  ".." / ".." / "lean-rs" / "crates" / "lean-rs" / "shims" / "lean-rs-interop-shims"

lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]
  defaultFacets := #[LeanLib.sharedFacet]

@[default_target]
lean_exe lean_dup_worker where
  root := `LeanDup.Worker
  supportInterpreter := true
