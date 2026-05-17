import Lean
import LeanDup.Extract
import LeanDup.Features
import LeanDup.Probe
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
open Lean.Meta
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

private def writeJsonLine (json : Json) : IO Unit := do
  IO.println json.compress
  (← IO.getStdout).flush

private def phaseRows (request : Request) (stats : LeanDup.Extract.RunStats) : Array Envelope :=
  #[
    progressEnvelope
      request
      "lean.import"
      (some stats.declarationCount)
      (some stats.declarationCount)
      (some stats.importMs)
      "imported requested modules",
    progressEnvelope
      request
      "lean.semantic"
      (some stats.rowCount)
      (some stats.declarationCount)
      (some stats.semanticMs)
      "computed semantic rows"
  ]

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

private def toExtractModules (modules : Array ModuleDescriptor) : Array LeanDup.Extract.ModuleSpec :=
  modules.map fun descriptor =>
    { module := descriptor.module
      origin := descriptor.origin
      sourceRoot? := descriptor.sourceRoot? }

private def extractErrorCode : LeanDup.Extract.ErrorKind → ErrorCode
  | .invalidRequest => .invalidRequest
  | .importFailed => .importFailed
  | .internalError => .internalError

private def extractError (request : Request) (err : LeanDup.Extract.Error) : ProtocolError :=
  { requestId? := some request.requestId
    command? := some request.command
    code := extractErrorCode err.kind
    fatal := true
    message := err.message
    details := err.details }

private unsafe def handleExtract (request : Request) : IO (Except ProtocolError (Array Envelope × UInt32)) := do
  match request.modules with
  | .error err => pure <| .error err
  | .ok modules =>
      match ← LeanDup.Extract.runProfiled request.payload (toExtractModules modules) with
      | .error err => pure <| .error (extractError request err)
      | .ok output =>
          let rows :=
            phaseRows request output.stats ++
              (output.rows.map fun payload => envelope request .declarationRow payload)
          pure <| .ok (completed request rows, 0)

private def featureErrorCode : LeanDup.Features.ErrorKind → ErrorCode
  | .invalidRequest => .invalidRequest
  | .importFailed => .importFailed
  | .internalError => .internalError

private def featureError (request : Request) (err : LeanDup.Features.Error) : ProtocolError :=
  { requestId? := some request.requestId
    command? := some request.command
    code := featureErrorCode err.kind
    fatal := true
    message := err.message
    details := err.details }

private unsafe def handleFeatures (request : Request) : IO (Except ProtocolError (Array Envelope × UInt32)) := do
  match request.modules with
  | .error err => pure <| .error err
  | .ok modules =>
      match ← LeanDup.Features.runProfiled request.payload (toExtractModules modules) with
      | .error err => pure <| .error (featureError request err)
      | .ok output =>
          let rows :=
            phaseRows request output.stats ++
              (output.rows.map fun payload => envelope request .featureRow payload)
          pure <| .ok (completed request rows, 0)

private def optionalNatField (json : Json) (key : String) (default : Nat) : Except ProtocolError Nat := do
  match json.getObjVal? key with
  | .error _ => pure default
  | .ok Json.null => pure default
  | .ok (Json.num value) =>
      match value.toString.toNat? with
      | some parsed => pure parsed
      | none =>
          throw
            { requestId? := none
              command? := some .index
              code := .invalidRequest
              fatal := true
              message := s!"`{key}` must be a natural number"
              details := none }
  | .ok _ =>
      throw
        { requestId? := none
          command? := some .index
          code := .invalidRequest
          fatal := true
          message := s!"`{key}` must be a natural number"
          details := none }

private def validateShard
    (request : Request)
    (shardIndex shardCount : Nat) : Except ProtocolError Unit := do
  if shardCount == 0 then
    throw
      { requestId? := some request.requestId
        command? := some request.command
        code := .invalidRequest
        fatal := true
        message := "`declaration_shard_count` must be greater than zero"
        details := none }
  if shardIndex >= shardCount then
    throw
      { requestId? := some request.requestId
        command? := some request.command
        code := .invalidRequest
        fatal := true
        message := "`declaration_shard_index` must be less than `declaration_shard_count`"
        details := none }

private def heartbeatLike (message : String) : Bool :=
  message.contains "maximum number of heartbeats" || message.contains "timeout at"

private def errorDetailsForDeclaration
    (declaration : LeanDup.Extract.AcceptedDeclaration)
    (message : String) : Json :=
  Json.mkObj
    [ ("declaration_id", Json.str declaration.declarationId)
    , ("module", Json.str declaration.moduleSpec.module)
    , ("qualified_name", Json.str declaration.declName.toString)
    , ("lean_error", Json.str message)
    ]

private unsafe def computeIndexChunk
    (options : LeanDup.Extract.Options)
    (env : Environment)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) :
    IO (Except LeanDup.Extract.Error (Array Json × Array Json)) := do
  let coreContext : Core.Context :=
    { fileName := "<lean-dup-index>"
      fileMap := default
      options := Options.empty }
  try
    let (rows, _, _) ←
      MetaM.toIO
        (do
          let mut declarationRows := #[]
          for declaration in declarations do
            declarationRows := declarationRows.push (← LeanDup.Extract.rowPayloadFromAccepted options declaration)
          let featureRows ← LeanDup.Features.featureRows declarations
          pure (declarationRows, featureRows))
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

private unsafe def emitIndexRange
    (request : Request)
    (options : LeanDup.Extract.Options)
    (env : Environment)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration)
    (start stop : Nat)
    (emitted : IO.Ref (Array ResponseKind)) :
    IO (Except ProtocolError Unit) := do
  let chunk := declarations.extract start stop
  if chunk.isEmpty then
    pure <| .ok ()
  else
    let starting :=
      progressEnvelope
        request
        "lean.index.chunk.start"
        (some start)
        (some declarations.size)
        none
        s!"starting declarations {start}-{stop} of {declarations.size}"
    writeJsonLine starting.toJson
    emitted.modify (·.push ResponseKind.progress)
    let chunkStart ← IO.monoMsNow
    match ← computeIndexChunk options env chunk with
    | .ok (declarationRows, featureRows) =>
        for payload in declarationRows do
          writeJsonLine (envelope request .declarationRow payload).toJson
          emitted.modify (·.push .declarationRow)
        for payload in featureRows do
          writeJsonLine (envelope request .featureRow payload).toJson
          emitted.modify (·.push .featureRow)
        let chunkFinished ← IO.monoMsNow
        let progress :=
          progressEnvelope
            request
            "lean.index.chunk"
            (some stop)
            (some declarations.size)
            (some (chunkFinished - chunkStart))
            s!"indexed declarations {start}-{stop} of {declarations.size}"
        writeJsonLine progress.toJson
        emitted.modify (·.push ResponseKind.progress)
        pure <| .ok ()
    | .error err =>
        if heartbeatLike err.message && chunk.size > 1 then
          let split :=
            progressEnvelope
              request
              "lean.index.chunk.split"
              (some start)
              (some declarations.size)
              none
              s!"splitting heartbeat-limited declarations {start}-{stop}"
          writeJsonLine split.toJson
          emitted.modify (·.push ResponseKind.progress)
          let mid := start + chunk.size / 2
          match ← emitIndexRange request options env declarations start mid emitted with
          | .error err => pure <| .error err
          | .ok _ => emitIndexRange request options env declarations mid stop emitted
        else
          let details :=
            match chunk[0]? with
            | some declaration => errorDetailsForDeclaration declaration err.message
            | none => Json.mkObj [("lean_error", Json.str err.message)]
          pure <|
            .error
              { requestId? := some request.requestId
                command? := some request.command
                code := .internalError
                fatal := true
                message := "mathlib index chunk failed"
                details := some details }

private unsafe def handleIndexStreaming (request : Request) : IO UInt32 := do
  match request.modules with
  | .error err =>
      writeJsonLine err.toJson
      pure 1
  | .ok modules =>
      if modules.isEmpty then
        writeJsonLine
          ({ requestId? := some request.requestId
             command? := some request.command
             code := .invalidRequest
             fatal := true
             message := "`modules` must contain at least one module"
             details := none } : ProtocolError).toJson
        pure 1
      else
        match LeanDup.Extract.parseOptions request.payload with
        | .error err =>
            writeJsonLine (extractError request err).toJson
            pure 1
        | .ok options =>
            match optionalNatField request.payload "declaration_chunk_size" 128 with
            | .error err =>
                writeJsonLine err.toJson
                pure 1
            | .ok requestedChunkSize =>
                match optionalNatField request.payload "declaration_shard_index" 0,
                    optionalNatField request.payload "declaration_shard_count" 1 with
                | .error err, _ =>
                    writeJsonLine err.toJson
                    pure 1
                | _, .error err =>
                    writeJsonLine err.toJson
                    pure 1
                | .ok shardIndex, .ok shardCount =>
                    match validateShard request shardIndex shardCount with
                    | .error err =>
                        writeJsonLine err.toJson
                        pure 1
                    | .ok _ =>
                        let chunkSize := Nat.max 1 requestedChunkSize
                        let emitted ← IO.mkRef #[]
                        let importStarted ← IO.monoMsNow
                        match ← LeanDup.Extract.importRequestedModules (toExtractModules modules) with
                        | .error err =>
                            writeJsonLine (extractError request err).toJson
                            pure 1
                        | .ok env =>
                            let importFinished ← IO.monoMsNow
                            let progress :=
                              progressEnvelope
                                request
                                "lean.import"
                                (some modules.size)
                                (some modules.size)
                                (some (importFinished - importStarted))
                                "imported requested modules"
                            writeJsonLine progress.toJson
                            emitted.modify (·.push ResponseKind.progress)
                            let coreContext : Core.Context :=
                              { fileName := "<lean-dup-index>"
                                fileMap := default
                                options := Options.empty }
                            let collectStarted ← IO.monoMsNow
                            try
                              let (declarations, _, _) ←
                                MetaM.toIO
                                  (LeanDup.Extract.collectAcceptedDeclarations
                                    { modules := toExtractModules modules
                                      options := options })
                                  coreContext
                                  { env := env }
                                  {}
                                  {}
                              let collectFinished ← IO.monoMsNow
                              let progress :=
                                progressEnvelope
                                  request
                                  "lean.index.enumerate"
                                  (some declarations.size)
                                  (some declarations.size)
                                  (some (collectFinished - collectStarted))
                                  s!"enumerated {declarations.size} accepted declarations"
                              writeJsonLine progress.toJson
                              emitted.modify (·.push ResponseKind.progress)
                              let shardStart := declarations.size * shardIndex / shardCount
                              let shardStop := declarations.size * (shardIndex + 1) / shardCount
                              let progress :=
                                progressEnvelope
                                  request
                                  "lean.index.shard"
                                  (some shardStart)
                                  (some declarations.size)
                                  none
                                  s!"worker shard {shardIndex + 1}/{shardCount} owns declarations {shardStart}-{shardStop}"
                              writeJsonLine progress.toJson
                              emitted.modify (·.push ResponseKind.progress)
                              let rec loop (start : Nat) : IO UInt32 := do
                                if start < shardStop then
                                  let stop := Nat.min shardStop (start + chunkSize)
                                  match ← emitIndexRange request options env declarations start stop emitted with
                                  | .error err =>
                                      writeJsonLine err.toJson
                                      pure 1
                                  | .ok _ =>
                                      loop stop
                                else
                                  writeJsonLine (completeEnvelope request (← emitted.get)).toJson
                                  pure 0
                              loop shardStart
                            catch error =>
                              writeJsonLine
                                ({ requestId? := some request.requestId
                                   command? := some request.command
                                   code := .internalError
                                   fatal := true
                                   message := s!"mathlib declaration enumeration failed: {error}"
                                   details := none } : ProtocolError).toJson
                              pure 1

private def probeErrorCode : LeanDup.Probe.ErrorKind → ErrorCode
  | .invalidRequest => .invalidRequest
  | .importFailed => .importFailed
  | .internalError => .internalError

private def probeError (request : Request) (err : LeanDup.Probe.Error) : ProtocolError :=
  { requestId? := some request.requestId
    command? := some request.command
    code := probeErrorCode err.kind
    fatal := true
    message := err.message
    details := err.details }

private unsafe def handleProbe (request : Request) : IO (Except ProtocolError (Array Envelope × UInt32)) := do
  match request.modules with
  | .error err => pure <| .error err
  | .ok modules =>
      match ← LeanDup.Probe.runProfiled request.payload (toExtractModules modules) with
      | .error err => pure <| .error (probeError request err)
      | .ok output =>
          let rows :=
            phaseRows request output.stats ++
              (output.rows.map fun payload => envelope request .probeResult payload)
          pure <| .ok (completed request rows, 0)

private unsafe def dispatch (request : Request) : IO (Except ProtocolError (Array Envelope × UInt32)) := do
  match request.command with
  | .version =>
      let row := envelope request .versionResult currentVersionResult.toJson
      pure <| .ok (completed request #[row], 0)
  | .doctor => handleDoctor request
  | .extract => handleExtract request
  | .features => handleFeatures request
  | .index =>
      pure <|
        .error
          { requestId? := some request.requestId
            command? := some request.command
            code := .invalidRequest
            fatal := true
            message := "`index` is a streaming command"
            details := none }
  | .probe => handleProbe request

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
          if request.command == .index then
            handleIndexStreaming request
          else
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
