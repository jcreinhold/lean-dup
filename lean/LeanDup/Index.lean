import Lean
import LeanDup.Extract
import LeanDup.Features
import LeanRsInterop.Worker.Stream

/-!
`LeanDup.Index` owns the capability-mode streaming index: it imports the
requested modules once and emits both declaration rows and feature rows from
that single import, with bounded parallel chunking and heartbeat-limit
splitting.

This is the `lean-rs-worker` capability analogue of the former subprocess
`handleIndexStreaming` dispatch in `LeanDup.Worker`. The semantic row payloads
(`Extract.rowPayloadFromAccepted`, `Features.featureRows`) are unchanged and
byte-identical to the subprocess worker; only the transport differs: rows,
progress, and diagnostics are pushed through the
`LeanRsInterop.Worker.Stream` callback trampoline rather than written as JSONL
to stdout.

Rows are labelled with the stream names `"declarations"` and `"features"` so the
Rust pool engine can route each payload to its typed sink. Like the other
capability exports, this never calls `initSearchPath`: the host worker session
has already installed the audited workspace's import search path.
-/
namespace LeanDup.Index

open Lean
open Lean.Meta

/-- Emit one stream frame through the callback trampoline, recording a nonzero
    cancellation status so the scheduler can stop early. -/
private def emitFrame
    (handle trampoline : USize)
    (abort : IO.Ref (Option UInt8))
    (frame : String) : IO Unit := do
  if (← abort.get).isSome then
    return
  let status ← LeanRsInterop.Callback.String.call handle trampoline frame
  if status != 0 then
    abort.set (some status)

/-- The same heartbeat-limit detection the subprocess scheduler used to decide
    whether a failing chunk should be split rather than aborting the index. -/
private def heartbeatLike (message : String) : Bool :=
  message.contains "maximum number of heartbeats" || message.contains "timeout at"

private def optionalNatField (json : Json) (key : String) (default : Nat) : Except String Nat :=
  match json.getObjVal? key with
  | .error _ => .ok default
  | .ok Json.null => .ok default
  | .ok (Json.num value) =>
      match value.toString.toNat? with
      | some parsed => .ok parsed
      | none => .error s!"`{key}` must be a natural number"
  | .ok _ => .error s!"`{key}` must be a natural number"

private structure IndexChunkResult where
  start : Nat
  stop : Nat
  declarationRows : Array Json
  featureRows : Array Json
  skipped : Nat

private abbrev IndexChunkTask :=
  Task (Except IO.Error (Nat × Nat × Nat × Nat × Except LeanDup.Extract.Error IndexChunkResult))

private unsafe def computeIndexChunk
    (options : LeanDup.Extract.Options)
    (env : Environment)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) :
    IO (Except LeanDup.Extract.Error (Array Json × Array Json × Nat)) := do
  let coreContext : Core.Context :=
    { fileName := "<lean-dup-index>"
      fileMap := default
      options := LeanDup.Extract.elaborationOptions options }
  try
    let (rows, _, _) ←
      MetaM.toIO
        (do
          -- Process each declaration's display row and feature row together under
          -- one per-declaration heartbeat budget, so a slow declaration is skipped
          -- from both streams consistently rather than failing the whole chunk.
          let (pairs, skipped) ←
            LeanDup.Extract.forEachDeclarationSkippingSlow declarations
              (fun declaration => do
                let declarationRow ← LeanDup.Extract.rowPayloadFromAccepted options declaration
                let featureRow ← LeanDup.Features.featureRow declaration
                pure (declarationRow, featureRow))
          pure (pairs.map (·.1), pairs.map (·.2), skipped))
        coreContext
        { env := env }
        {}
        {}
    pure <| .ok rows
  catch error =>
    pure <|
      .error
        { kind := .internalError
          message := s!"index chunk processing failed: {error}"
          details := none }

private unsafe def computeIndexRange
    (options : LeanDup.Extract.Options)
    (env : Environment)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration)
    (start stop : Nat) :
    IO (Except LeanDup.Extract.Error IndexChunkResult) := do
  let chunk := declarations.extract start stop
  match ← computeIndexChunk options env chunk with
  | .ok (declarationRows, featureRows, skipped) =>
      pure <| .ok { start, stop, declarationRows, featureRows, skipped }
  | .error err => pure <| .error err

private unsafe def spawnIndexRange
    (handle trampoline : USize)
    (abort : IO.Ref (Option UInt8))
    (options : LeanDup.Extract.Options)
    (env : Environment)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration)
    (serial : Nat)
    (start stop : Nat) : IO IndexChunkTask := do
  emitFrame handle trampoline abort
    (LeanRsInterop.Worker.Stream.progress "lean.index.chunk.start" start (some declarations.size))
  IO.asTask do
    let chunkStarted ← IO.monoMsNow
    let result ← computeIndexRange options env declarations start stop
    let chunkFinished ← IO.monoMsNow
    pure (serial, start, stop, chunkFinished - chunkStarted, result)

private def chunkErrorMessage
    (declarations : Array LeanDup.Extract.AcceptedDeclaration)
    (start : Nat)
    (err : LeanDup.Extract.Error) : String :=
  match declarations[start]? with
  | some declaration => s!"mathlib index chunk failed at {declaration.declName}: {err.message}"
  | none => s!"mathlib index chunk failed: {err.message}"

private unsafe def emitIndexChunkResult
    (handle trampoline : USize)
    (abort : IO.Ref (Option UInt8))
    (declarations : Array LeanDup.Extract.AcceptedDeclaration)
    (result : IndexChunkResult) : IO Unit := do
  for payload in result.declarationRows do
    emitFrame handle trampoline abort (LeanRsInterop.Worker.Stream.row "declarations" payload.compress)
  for payload in result.featureRows do
    emitFrame handle trampoline abort (LeanRsInterop.Worker.Stream.row "features" payload.compress)
  emitFrame handle trampoline abort
    (LeanRsInterop.Worker.Stream.progress "lean.index.chunk" result.stop (some declarations.size))

/-- Schedule the accepted declarations into bounded parallel chunks, emitting
    declaration and feature rows as each chunk completes and splitting any
    heartbeat-limited chunk rather than failing the whole index. -/
private unsafe def emitIndexRanges
    (handle trampoline : USize)
    (abort : IO.Ref (Option UInt8))
    (options : LeanDup.Extract.Options)
    (env : Environment)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration)
    (chunkSize parallelism : Nat) : IO (Except String Nat) := do
  let mut pending : List (Nat × Nat) := []
  let mut start := 0
  while start < declarations.size do
    let stop := Nat.min declarations.size (start + chunkSize)
    pending := pending.concat (start, stop)
    start := stop

  let maxActive := Nat.max 1 parallelism
  let mut active : List IndexChunkTask := []
  let mut serial := 0
  let mut failed : Option String := none
  let mut skippedTotal := 0

  while failed.isNone && (← abort.get).isNone && active.length < maxActive && !pending.isEmpty do
    match pending with
    | [] => pure ()
    | (rangeStart, rangeStop) :: rest =>
        pending := rest
        let task ← spawnIndexRange handle trampoline abort options env declarations serial rangeStart rangeStop
        active := active.concat task
        serial := serial + 1
  while failed.isNone && (← abort.get).isNone && !active.isEmpty do
    match active with
    | [] => pure ()
    | task :: rest =>
        let (taskResult, remainingTasks) ← IO.waitAny' (task :: rest)
        active := remainingTasks
        match taskResult with
        | .error ioErr =>
            failed := some s!"mathlib index task failed: {ioErr}"
        | .ok (_serial, _rangeStart, _rangeStop, _elapsedMs, .ok result) =>
            skippedTotal := skippedTotal + result.skipped
            emitIndexChunkResult handle trampoline abort declarations result
        | .ok (_serial, rangeStart, rangeStop, _elapsedMs, .error err) =>
            if heartbeatLike err.message && rangeStop - rangeStart > 1 then
              emitFrame handle trampoline abort
                (LeanRsInterop.Worker.Stream.progress "lean.index.chunk.split" rangeStart (some declarations.size))
              let mid := rangeStart + (rangeStop - rangeStart) / 2
              pending := (rangeStart, mid) :: (mid, rangeStop) :: pending
            else
              failed := some (chunkErrorMessage declarations rangeStart err)
        while failed.isNone && (← abort.get).isNone && active.length < maxActive && !pending.isEmpty do
          match pending with
          | [] => pure ()
          | (rangeStart, rangeStop) :: rest =>
              pending := rest
              let task ← spawnIndexRange handle trampoline abort options env declarations serial rangeStart rangeStop
              active := active.concat task
              serial := serial + 1

  for task in active do
    IO.cancel task
  match failed with
  | some err => pure <| .error err
  | none => pure <| .ok skippedTotal

/--
Import the requested modules once and stream declaration and feature rows from
that single import through the worker callback trampoline.

Returns `.error message` for a request, import, enumeration, or chunk failure
(the caller frames it as a terminal diagnostic), or `.ok status` where `status`
is `0` for a clean run or the nonzero cancellation byte reported by the host
when it asked the stream to stop.
-/
unsafe def streamIndex
    (handle trampoline : USize)
    (json : Json)
    (modules : Array LeanDup.Extract.ModuleSpec) : IO (Except String (UInt8 × Nat)) := do
  if modules.isEmpty then
    return .error "`modules` must contain at least one module"
  match LeanDup.Extract.parseOptions json with
  | .error err => return .error err.message
  | .ok options =>
      match optionalNatField json "declaration_chunk_size" 128 with
      | .error message => return .error message
      | .ok requestedChunkSize =>
          match optionalNatField json "declaration_parallelism" 1 with
          | .error message => return .error message
          | .ok requestedParallelism =>
              let chunkSize := Nat.max 1 requestedChunkSize
              let parallelism := Nat.max 1 requestedParallelism
              let abort ← IO.mkRef (none : Option UInt8)
              match ← LeanDup.Extract.importRequestedModules modules (initializeSearchPath := false) with
              | .error err => return .error err.message
              | .ok env =>
                  emitFrame handle trampoline abort
                    (LeanRsInterop.Worker.Stream.progress "lean.import" modules.size (some modules.size))
                  let coreContext : Core.Context :=
                    { fileName := "<lean-dup-index>"
                      fileMap := default
                      options := LeanDup.Extract.elaborationOptions options }
                  let context : LeanDup.Extract.Context := { modules := modules, options := options }
                  let collected ←
                    try
                      let (declarations, _, _) ←
                        MetaM.toIO
                          (LeanDup.Extract.collectAcceptedDeclarations context)
                          coreContext
                          { env := env }
                          {}
                          {}
                      pure <| Except.ok declarations
                    catch error =>
                      pure <| Except.error s!"mathlib declaration enumeration failed: {error}"
                  match collected with
                  | .error message => return .error message
                  | .ok declarations =>
                      emitFrame handle trampoline abort
                        (LeanRsInterop.Worker.Stream.progress "lean.index.enumerate"
                          declarations.size (some declarations.size))
                      emitFrame handle trampoline abort
                        (LeanRsInterop.Worker.Stream.progress "lean.index.scheduler" 0 (some declarations.size))
                      match ← emitIndexRanges handle trampoline abort options env declarations chunkSize parallelism with
                      | .error message => return .error message
                      | .ok skipped => return .ok ((← abort.get).getD 0, skipped)

end LeanDup.Index
