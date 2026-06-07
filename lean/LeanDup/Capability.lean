import Lean
import LeanDup.Extract
import LeanRsInterop.Worker.Stream

/-!
`LeanDup.Capability` is the spike surface for the `lean-rs-worker-*` migration: a
shared-library capability the `lean-rs-worker-child` binary loads, exposing the
semantic worker commands as `@[export]` functions instead of a subprocess JSONL
dispatch loop.

Spike scope: one streaming `extract` export over the unchanged
`LeanDup.Extract.runProfiled`, emitting rows via the generic
`LeanRsInterop.Worker.Stream` helpers. Request parsing here is deliberately
minimal; the full migration reuses `LeanDup.Protocol`.
-/
namespace LeanDup.Capability

open Lean

private def parseModules (json : Json) : Array LeanDup.Extract.ModuleSpec := Id.run do
  let mut out := #[]
  match json.getObjVal? "modules" with
  | .ok (.arr entries) =>
      for entry in entries do
        match entry.getObjValAs? String "module" with
        | .ok moduleName =>
            let origin := (entry.getObjValAs? String "origin").toOption.getD "workspace"
            let sourceRoot? := (entry.getObjValAs? String "source_root").toOption
            out := out.push { module := moduleName, origin := origin, sourceRoot? := sourceRoot? }
        | .error _ => pure ()
  | _ => pure ()
  return out

private def summary (rowCount : Nat) : String :=
  (Json.mkObj [("command", Json.str "extract"), ("rows", Json.num rowCount), ("ok", Json.bool true)]).compress

/-- Spike streaming export: parse a request, import the requested (target-workspace)
modules via `Extract.runProfiled`, and stream declaration rows. The target oleans
are resolved through `initSearchPath` + `LEAN_PATH`, exactly as the subprocess
worker does today; the worker child must therefore be spawned with the audited
workspace's `LEAN_PATH`. -/
@[export lean_dup_capability_extract]
unsafe def extract (requestJson : String) (handle trampoline : USize) : IO UInt8 := do
  match Json.parse requestJson with
  | .error message =>
      LeanRsInterop.Worker.Stream.emitAll handle trampoline
        #[LeanRsInterop.Worker.Stream.diagnostic "request.malformed" message]
  | .ok json =>
      let modules := parseModules json
      match ← LeanDup.Extract.runProfiled json modules with
      | .error err =>
          LeanRsInterop.Worker.Stream.emitAll handle trampoline
            #[LeanRsInterop.Worker.Stream.diagnostic "extract.failed" err.message]
      | .ok output =>
          let mut payloads := #[LeanRsInterop.Worker.Stream.diagnostic "extract.started" "extract started"]
          for row in output.rows do
            payloads := payloads.push (LeanRsInterop.Worker.Stream.row "declarations" row.compress)
          payloads := payloads.push (LeanRsInterop.Worker.Stream.metadata (summary output.rows.size))
          LeanRsInterop.Worker.Stream.emitAll handle trampoline payloads

end LeanDup.Capability
