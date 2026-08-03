import Lean
import LeanDup.Protocol
import LeanDup.Extract
import LeanDup.Features
import LeanDup.Probe
import LeanDup.Index
import LeanDup.Frames

/-!
`LeanDup.Server` is the native transport for the lean-dup semantic worker: a
JSONL server spoken by the `lean-dup-worker` executable (`Main.lean`). It
replaces the retired `lean-rs-worker` capability transport (a dlopen'd dylib
behind an FFI worker pool) with a plain subprocess: the Rust parent spawns the
executable under `lake env` in the audited workspace, writes one request
envelope per line on stdin, and reads framed responses, one JSON object per
line, on stdout.

The command set and row payloads are identical to the retired capability
transport — `version`, `extract`, `features`, `probe`, `index` — only the
framing changes. `extract`/`features`/`probe` stream rows under their stream
names and finish with a terminal `metadata` summary frame; `index` streams
`"declarations"` and `"features"` rows plus progress and finishes with a
terminal `metadata` frame. Malformed requests and command failures are framed
as a `diagnostic` plus a failure `metadata` frame, so one bad command never
desynchronizes the line protocol.

Frame shapes (one JSON object per line):

* `{"stream": <name>, "payload": <row-json>}` — a data row
* `{"progress": {"phase": <name>, "current": <n>, "total": <n?>}}` — progress
* `{"diagnostic": {"code": <code>, "message": <text>}}` — fatal diagnostic
* `{"metadata": <summary-json>}` — terminal per-command summary
* `{"result": <json>}` — request/response reply (`version`)

Request envelope: `{"command": <name>, "request": <payload-json>}` where the
payload has exactly the shape the retired capability exports accepted
(including the hoisted `modules`/`modules_origin`/`modules_source_root`
encoding). Cancellation is owned by the parent: it kills the process. The
import-once session environment (`LeanDup.Extract.sessionEnv`) makes a killed
process cheap to restart — every command after the first in one signature
shares a single import.
-/
namespace LeanDup.Server

open Lean



/-- Emit one framed line on stdout, flushing so the parent observes rows,
    progress, and termination as they happen. -/
def emit (frame : String) : IO Unit := do
  let stdout ← IO.getStdout
  stdout.putStr frame
  stdout.putStr "\n"
  stdout.flush

/-- Parse the `modules` array of one request payload, honoring the hoisted
    `modules_origin`/`modules_source_root` constants the Rust caller sends
    (per-entry objects may still override). -/
def parseModules (json : Json) : Array LeanDup.Extract.ModuleSpec := Id.run do
  let mut out := #[]
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

/-- Stream the rows of one `runProfiled`-style command result, framing request,
    import, and internal failures as a terminal diagnostic plus failure
    summary. -/
unsafe def emitProfiledRows
    (command stream : String)
    (result : Except String (Array Json × Nat)) : IO Unit := do
  match result with
  | .error message =>
      emit (Frames.diagnostic s!"{command}.failed" message)
      emit (Frames.metadata (Frames.failureSummary command message))
  | .ok (rows, skipped) =>
      for row in rows do
        emit (Frames.row stream row.compress)
      emit (Frames.metadata (Frames.successSummary command rows.size skipped))

/-- Run one `version` command: a single `result` frame with the
    `lean-dup.worker.v1` `version_result` payload. -/
def runVersion : IO Unit :=
  emit (Frames.result LeanDup.Protocol.currentVersionResult.toJson.compress)

/-- Run one streaming command (`extract`, `features`, or `probe`) over the
    shared session environment. -/
unsafe def runStreaming (command : String) (json : Json) : IO Unit := do
  let modules := parseModules json
  match command with
  | "extract" =>
      let result ← LeanDup.Extract.runProfiled json modules
      emitProfiledRows command "declarations"
        (result.map (fun output => (output.rows, output.stats.skippedCount)) |>.mapError (·.message))
  | "features" =>
      let result ← LeanDup.Features.runProfiled json modules
      emitProfiledRows command "features"
        (result.map (fun output => (output.rows, output.stats.skippedCount)) |>.mapError (·.message))
  | "probe" =>
      let result ← LeanDup.Probe.runProfiled json modules
      emitProfiledRows command "probe"
        (result.map (fun output => (output.rows, output.stats.skippedCount)) |>.mapError (·.message))
  | other =>
      let message := s!"unknown streaming command `{other}`"
      emit (Frames.diagnostic "request.unknown_command" message)
      emit (Frames.metadata (Frames.failureSummary other message))

/-- Run one `index` command: import once and stream declaration and feature
    rows from that single import through `emit`. -/
unsafe def runIndex (json : Json) : IO Unit := do
  let modules := parseModules json
  match ← LeanDup.Index.streamIndex emit json modules with
  | .error message =>
      emit (Frames.diagnostic "index.failed" message)
      emit (Frames.metadata (Frames.failureSummary "index" message))
  | .ok skipped =>
      emit (Frames.metadata (Frames.successSummary "index" 0 skipped))

/-- Dispatch one decoded request envelope. Never throws: every failure is
    framed so the parent's line protocol stays synchronized. -/
unsafe def dispatch (command : String) (json : Json) : IO Unit := do
  try
    match command with
    | "version" => runVersion
    | "index" => runIndex json
    | other => runStreaming other json
  catch error =>
    let message := s!"{command} command failed: {error}"
    emit (Frames.diagnostic s!"{command}.failed" message)
    emit (Frames.metadata (Frames.failureSummary command message))

/-- Serve one raw request line. -/
unsafe def serveLine (line : String) : IO Unit := do
  match Json.parse line with
  | .error message =>
      emit (Frames.diagnostic "request.malformed" message)
      emit (Frames.metadata (Frames.failureSummary "unknown" message))
  | .ok envelope =>
      match envelope.getObjValAs? String "command" with
      | .error message =>
          emit (Frames.diagnostic "request.malformed" message)
          emit (Frames.metadata (Frames.failureSummary "unknown" message))
      | .ok command =>
          let request := envelope.getObjVal? "request" |>.toOption.getD Json.null
          dispatch command request

end LeanDup.Server
