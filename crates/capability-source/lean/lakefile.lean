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
-- in the lean-semantic-search package. The tag's toolchain (v4.33.0-rc1) matches
-- this package's, so Lake compiles it cleanly under the root toolchain.
require «lean-semantic-search» from git
  "https://github.com/jcreinhold/lean-semantic-search.git" @ "v0.4.3" / "lean"

@[default_target]
lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]

-- The native worker executable: Rust spawns this under `lake env` in the
-- audited workspace and speaks JSONL (see `LeanDup.Server`). The retired
-- capability dylib + lean-rs worker pool transport is gone.
lean_exe «lean-dup-worker» where
  root := `Main
  -- `importModules` loads compiled module extensions through the interpreter.
  supportInterpreter := true
