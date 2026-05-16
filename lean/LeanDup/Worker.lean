import Lean
import LeanDup.Protocol

/-!
`LeanDup.Worker` owns subprocess framing and command dispatch for the Lean
semantic worker.

Callers may rely on one UTF-8 JSON request on stdin and JSONL protocol
envelopes on stdout. They must not rely on stderr wording, import scheduling,
internal dispatch helpers, or semantic placeholder implementation details.
-/
namespace LeanDup.Worker

open Lean
open LeanDup.Protocol

private def dottedName (text : String) : Name :=
  (text.splitOn ".").foldl
    (init := Name.anonymous)
    fun current segment =>
      if segment.isEmpty then current else Name.str current segment

private def checkJson (name : String) (status : CheckStatus) (message : Option String := none) :
    DoctorCheck :=
  { name := name, status := status, message := message }

private def doctorResult (checks : Array DoctorCheck) : DoctorResult :=
  { ok := checks.all fun check => check.status != .failed
    checks := checks
    worker := currentVersionResult }

private unsafe def checkImports (modules : Array ModuleDescriptor) : IO DoctorCheck := do
  if modules.isEmpty then
    pure <| checkJson "imports" .skipped "no modules requested"
  else
    Lean.enableInitializersExecution
    initSearchPath (← getBuildDir)
    let imports := modules.map fun descriptor => ({ module := dottedName descriptor.module } : Import)
    try
      let _env ← importModules imports Options.empty (loadExts := true)
      pure <|
        checkJson
          "imports"
          .ok
          (some s!"imported {modules.size} module(s)")
    catch error =>
      pure <|
        checkJson
          "imports"
          .failed
          (some s!"could not import requested modules: {error}")

private def completed (request : Request) (rows : Array Envelope) : Array Envelope :=
  rows.push (completeEnvelope request (rows.map (·.kind)))

private unsafe def handleDoctor (request : Request) : IO (Except ProtocolError (Array Envelope × UInt32)) := do
  match request.modules with
  | .error err => pure <| .error err
  | .ok modules =>
      let importCheck ← checkImports modules
      let result :=
        doctorResult #[
          checkJson "schema" .ok s!"accepted {schemaVersion}",
          checkJson "worker" .ok s!"worker {workerVersion}",
          importCheck
        ]
      let row := envelope request .doctorResult result.toJson
      let exitCode : UInt32 := if result.ok then 0 else 1
      pure <| .ok (completed request #[row], exitCode)

private unsafe def dispatch (request : Request) : IO (Except ProtocolError (Array Envelope × UInt32)) := do
  match request.command with
  | .version =>
      let row := envelope request .versionResult currentVersionResult.toJson
      pure <| .ok (completed request #[row], 0)
  | .doctor => handleDoctor request
  | .extract
  | .features
  | .probe =>
      pure <| .ok (completed request #[], 0)

private def writeJsonLine (json : Json) : IO Unit := do
  IO.println json.compress
  (← IO.getStdout).flush

/-- Run one worker request from stdin and return the process exit code. -/
unsafe def run : IO UInt32 := do
  let input ← (← IO.getStdin).readToEnd
  match Json.parse input with
  | .error message =>
      writeJsonLine (malformedJsonError s!"could not parse request JSON: {message}").toJson
      pure 1
  | .ok json =>
      match Request.parse json with
      | .error err =>
          writeJsonLine err.toJson
          pure 1
      | .ok request =>
          match ← dispatch request with
          | .error err =>
              writeJsonLine err.toJson
              pure 1
          | .ok (rows, exitCode) =>
              for row in rows do
                writeJsonLine row.toJson
              pure exitCode

end LeanDup.Worker

unsafe def main (_args : List String) : IO UInt32 :=
  LeanDup.Worker.run
