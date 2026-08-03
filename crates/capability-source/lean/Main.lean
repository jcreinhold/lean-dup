import Lean
import LeanDup.Server

/-!
`lean-dup-worker` — the native lean-dup semantic worker executable.

The Rust parent (`lean-dup`) spawns this executable under `lake env` in the
audited workspace, one per audit, and speaks JSONL: one request envelope per
stdin line (`{"command": ..., "request": ...}`), framed response lines on
stdout (see `LeanDup.Server`). The process imports the workspace environment at
most once per module signature and holds it for the session; the parent kills
the process to cancel. EOF on stdin exits cleanly.
-/

unsafe def main : IO Unit := do
  Lean.enableInitializersExecution
  let stdin ← IO.getStdin
  while true do
    let line ← stdin.getLine
    if line.isEmpty then
      return
    LeanDup.Server.serveLine line.trim
