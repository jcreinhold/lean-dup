import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

-- Shared neutral feature extraction (canonical fingerprints, role features) lives
-- in the lean-semantic-search package. lean-dup imports it rather than keeping a
-- second copy.
require «lean-semantic-search» from ".." / ".." / "lean-semantic-search" / "lean"

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
