import Lake
open Lake DSL

package lean_dup_worker where
  version := v!"0.1.0"

-- The development worker uses the reviewed 0.7.8 release-candidate source.
-- The packaged installer replaces this Git require with a local path to the
-- Lean payload from the Rust semantic runtime crate.
require «lean-semantic-search» from git
  "https://github.com/jcreinhold/lean-semantic-search.git" @ "f1202a42311fc0926edd617fc35cf5cf1231dc80" / "lean"

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
