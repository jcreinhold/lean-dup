import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

-- The development worker uses the approved semantic-search source revision.
-- The packaged installer replaces this Git require with a local path to the
-- Lean payload from the Rust semantic runtime crate.
require «lean-semantic-search» from git
  "https://github.com/jcreinhold/lean-semantic-search.git" @ "c30d8b3d2001fbb7d7e28e9e74217ae711ea4d14" / "lean"

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
