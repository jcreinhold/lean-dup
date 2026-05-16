import Lake
open Lake DSL

package «lean-dup-worker» where
  version := v!"0.1.0"

lean_lib LeanDup where
  roots := #[`LeanDup]
  globs := #[.andSubmodules `LeanDup]

@[default_target]
lean_exe lean_dup_worker where
  root := `LeanDup.Worker
  supportInterpreter := true
