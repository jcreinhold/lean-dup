import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

-- Dependencies resolve from the *published* upstream sources, pinned to release
-- tags, so a clean `lake build LeanDup` works without sibling checkouts or a
-- pre-materialized runtime root. This mirrors the worker `build.rs` path, which
-- materializes these same crates' Lean sources before building the capability.
--
-- Shared neutral feature extraction (canonical fingerprints, role features) lives
-- in the lean-semantic-search package. The tag's toolchain (rc2) matches this
-- package's, so Lake compiles it cleanly under the root toolchain.
require «lean-semantic-search» from git
  "https://github.com/jcreinhold/lean-semantic-search.git" @ "v0.3.1" / "lean"

-- Generic Lean/Rust worker-streaming helpers (callback-envelope mechanics) used by
-- the lean-rs-worker-child capability path: LeanDup is built as a shared capability
-- dylib that the worker child loads. This is the only worker build — Rust drives Lean
-- entirely through the lean-rs-worker-parent pool, not a subprocess.
require «lean_rs_interop_shims» from git
  "https://github.com/jcreinhold/lean-rs.git" @ "v0.2.2" / "crates/lean-rs/shims/lean-rs-interop-shims"

@[default_target]
lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]
  defaultFacets := #[LeanLib.sharedFacet]
