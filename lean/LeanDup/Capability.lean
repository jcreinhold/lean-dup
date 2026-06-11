import Lean
import LeanDup.Extract
import LeanDup.Features
import LeanDup.Probe
import LeanDup.Index
import LeanDup.Protocol
import LeanRsInterop.Worker.Stream

/-!
`LeanDup.Capability` is the `lean-rs-worker` surface for the lean-dup semantic
worker: a shared-library capability that the `lean-rs-worker-child` binary loads,
exposing the worker commands as `@[export]` functions instead of a subprocess
JSONL dispatch loop.

Five commands cross the capability boundary:

* `version` — a request/response export returning the `lean-dup.worker.v1`
  `version_result` payload verbatim.
* `extract`, `features`, `probe` — streaming exports over the unchanged
  `runProfiled` semantics, emitting one row per accepted declaration / pair.
* `index` — a streaming export that imports once and emits both declaration and
  feature rows (see `LeanDup.Index`).

The semantic functions and their row payloads are unchanged and byte-identical
to the former subprocess worker; only the transport differs. Every streaming
export labels rows with a stream name (`"declarations"`, `"features"`,
`"probe"`) and finishes with a terminal `metadata` summary frame. None of these
exports call `initSearchPath`: the host worker session has already installed the
audited workspace's import search path.

`doctor` is intentionally not exported: it was never consumed by the Rust worker
client (the subprocess stream parser rejected `doctor_result`), and the CLI
`doctor` command is built from the `version` command. Adding a capability export
nothing calls would be dead surface.
-/
namespace LeanDup.Capability

open Lean

private def parseModules (json : Json) : Array LeanDup.Extract.ModuleSpec := Id.run do
  let mut out := #[]
  -- The caller hoists the per-request `origin` and `source_root` to the top level
  -- (see `modules_payload`); bare-string module entries inherit them. Per-entry
  -- objects may still override, so legacy object payloads parse unchanged.
  let defaultOrigin := (json.getObjValAs? String "modules_origin").toOption.getD "workspace"
  let defaultSourceRoot? := (json.getObjValAs? String "modules_source_root").toOption
  match json.getObjVal? "modules" with
  | .ok (.arr entries) =>
      for entry in entries do
        match entry with
        | .str moduleName =>
            out := out.push { module := moduleName, origin := defaultOrigin, sourceRoot? := defaultSourceRoot? }
        | _ =>
            match entry.getObjValAs? String "module" with
            | .ok moduleName =>
                let origin := (entry.getObjValAs? String "origin").toOption.getD defaultOrigin
                let sourceRoot? :=
                  match (entry.getObjValAs? String "source_root").toOption with
                  | some root => some root
                  | none => defaultSourceRoot?
                out := out.push { module := moduleName, origin := origin, sourceRoot? := sourceRoot? }
            | .error _ => pure ()
  | _ => pure ()
  return out

private def successSummary (command : String) (rowCount skipped : Nat) : String :=
  (Json.mkObj
    [ ("command", Json.str command)
    , ("rows", Json.num rowCount)
    , ("skipped", Json.num skipped)
    , ("ok", Json.bool true)
    ]).compress

private def failureSummary (command : String) (message : String) : String :=
  (Json.mkObj
    [ ("command", Json.str command)
    , ("rows", Json.num 0)
    , ("ok", Json.bool false)
    , ("message", Json.str message)
    ]).compress

/-- Stream the rows of one `runProfiled`-style command result, framing request,
    import, and internal failures as a terminal diagnostic plus failure summary
    so the Rust pool engine maps them to a fatal worker diagnostic. -/
private def emitProfiledRows
    (handle trampoline : USize)
    (command stream : String)
    (result : Except String (Array Json × Nat)) : IO UInt8 := do
  match result with
  | .error message =>
      LeanRsInterop.Worker.Stream.emitAll handle trampoline
        #[ LeanRsInterop.Worker.Stream.diagnostic s!"{command}.failed" message
         , LeanRsInterop.Worker.Stream.metadata (failureSummary command message)
         ]
  | .ok (rows, skipped) =>
      let mut payloads : Array String := #[]
      for row in rows do
        payloads := payloads.push (LeanRsInterop.Worker.Stream.row stream row.compress)
      payloads := payloads.push (LeanRsInterop.Worker.Stream.metadata (successSummary command rows.size skipped))
      LeanRsInterop.Worker.Stream.emitAll handle trampoline payloads

/--
Request/response export reporting `lean-dup.worker.v1` version facts.

Returns the same `version_result` payload object the subprocess worker emitted,
so the Rust client deserializes it unchanged.
-/
@[export lean_dup_capability_version]
def version (_requestJson : String) : IO String :=
  pure LeanDup.Protocol.currentVersionResult.toJson.compress

/--
Streaming export for declaration extraction.

The host worker session owns the target workspace import search path; this
export parses the downstream request and streams the same declaration-row
payloads as the subprocess worker.
-/
@[export lean_dup_capability_extract]
unsafe def extract (requestJson : String) (handle trampoline : USize) : IO UInt8 := do
  match Json.parse requestJson with
  | .error message =>
      LeanRsInterop.Worker.Stream.emitAll handle trampoline
        #[ LeanRsInterop.Worker.Stream.diagnostic "request.malformed" message
         , LeanRsInterop.Worker.Stream.metadata (failureSummary "extract" message)
         ]
  | .ok json =>
      let modules := parseModules json
      let result ← LeanDup.Extract.runProfiled json modules (initializeSearchPath := false)
      emitProfiledRows handle trampoline "extract" "declarations"
        (result.map (fun output => (output.rows, output.stats.skippedCount)) |>.mapError (·.message))

/-- Streaming export for Lean-owned semantic feature rows. -/
@[export lean_dup_capability_features]
unsafe def features (requestJson : String) (handle trampoline : USize) : IO UInt8 := do
  match Json.parse requestJson with
  | .error message =>
      LeanRsInterop.Worker.Stream.emitAll handle trampoline
        #[ LeanRsInterop.Worker.Stream.diagnostic "request.malformed" message
         , LeanRsInterop.Worker.Stream.metadata (failureSummary "features" message)
         ]
  | .ok json =>
      let modules := parseModules json
      let result ← LeanDup.Features.runProfiled json modules (initializeSearchPath := false)
      emitProfiledRows handle trampoline "features" "features"
        (result.map (fun output => (output.rows, output.stats.skippedCount)) |>.mapError (·.message))

/-- Streaming export for bounded semantic pair probes. -/
@[export lean_dup_capability_probe]
unsafe def probe (requestJson : String) (handle trampoline : USize) : IO UInt8 := do
  match Json.parse requestJson with
  | .error message =>
      LeanRsInterop.Worker.Stream.emitAll handle trampoline
        #[ LeanRsInterop.Worker.Stream.diagnostic "request.malformed" message
         , LeanRsInterop.Worker.Stream.metadata (failureSummary "probe" message)
         ]
  | .ok json =>
      let modules := parseModules json
      let result ← LeanDup.Probe.runProfiled json modules (initializeSearchPath := false)
      emitProfiledRows handle trampoline "probe" "probe"
        (result.map (fun output => (output.rows, output.stats.skippedCount)) |>.mapError (·.message))

/--
Streaming export for import-once declaration and feature indexing.

Emits both `"declarations"` and `"features"` rows from a single import, with
bounded parallel chunking and heartbeat-limit splitting (`LeanDup.Index`).
-/
@[export lean_dup_capability_index]
unsafe def index (requestJson : String) (handle trampoline : USize) : IO UInt8 := do
  match Json.parse requestJson with
  | .error message =>
      LeanRsInterop.Worker.Stream.emitAll handle trampoline
        #[ LeanRsInterop.Worker.Stream.diagnostic "request.malformed" message
         , LeanRsInterop.Worker.Stream.metadata (failureSummary "index" message)
         ]
  | .ok json =>
      let modules := parseModules json
      match ← LeanDup.Index.streamIndex handle trampoline json modules with
      | .error message =>
          LeanRsInterop.Worker.Stream.emitAll handle trampoline
            #[ LeanRsInterop.Worker.Stream.diagnostic "index.failed" message
             , LeanRsInterop.Worker.Stream.metadata (failureSummary "index" message)
             ]
      | .ok (status, skipped) =>
          if status == 0 then
            LeanRsInterop.Worker.Stream.emitAll handle trampoline
              #[ LeanRsInterop.Worker.Stream.metadata (successSummary "index" 0 skipped) ]
          else
            pure status

end LeanDup.Capability
