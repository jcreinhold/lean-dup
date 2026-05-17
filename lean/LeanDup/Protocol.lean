import Lean
import LeanDup.Extract
import LeanDup.Features
import LeanDup.Probe

/-!
`LeanDup.Protocol` owns the versioned worker request/response contract and JSON
encoding.

Callers may rely on schema names, command names, response kinds, payload fields,
and structured error codes. They must not infer Lean expression structure,
storage layout, subprocess framing details, or ranking/report policy from these
types.
-/
namespace LeanDup.Protocol

open Lean

/-- The v1 worker schema accepted and emitted by this package. -/
def schemaVersion : String := "lean-dup.worker.v1"

/-- The package-level worker version reported by the foundation worker. -/
def workerVersion : String := "0.1.0"

/-- Protocol commands exposed by the Lean semantic worker. -/
inductive Command where
  | extract
  | features
  | probe
  | doctor
  | version
  deriving BEq, Repr

namespace Command

/-- The stable protocol spelling for a command. -/
def asString : Command → String
  | .extract => "extract"
  | .features => "features"
  | .probe => "probe"
  | .doctor => "doctor"
  | .version => "version"

/-- Parse a protocol command name. -/
def parse? : String → Option Command
  | "extract" => some .extract
  | "features" => some .features
  | "probe" => some .probe
  | "doctor" => some .doctor
  | "version" => some .version
  | _ => none

end Command

/-- Worker response envelope kinds defined by protocol v1. -/
inductive ResponseKind where
  | versionResult
  | doctorResult
  | declarationRow
  | featureRow
  | probeResult
  | progress
  | complete
  | error
  deriving BEq, Repr

namespace ResponseKind

/-- The stable protocol spelling for a response kind. -/
def asString : ResponseKind → String
  | .versionResult => "version_result"
  | .doctorResult => "doctor_result"
  | .declarationRow => "declaration_row"
  | .featureRow => "feature_row"
  | .probeResult => "probe_result"
  | .progress => "progress"
  | .complete => "complete"
  | .error => "error"

end ResponseKind

/-- Structured worker error codes from the v1 protocol. -/
inductive ErrorCode where
  | malformedJson
  | unsupportedSchema
  | unsupportedCommand
  | invalidRequest
  | importFailed
  | missingOlean
  | probeUnavailable
  | workerPanic
  | internalError
  deriving BEq, Repr

namespace ErrorCode

/-- The stable protocol spelling for an error code. -/
def asString : ErrorCode → String
  | .malformedJson => "malformed_json"
  | .unsupportedSchema => "unsupported_schema"
  | .unsupportedCommand => "unsupported_command"
  | .invalidRequest => "invalid_request"
  | .importFailed => "import_failed"
  | .missingOlean => "missing_olean"
  | .probeUnavailable => "probe_unavailable"
  | .workerPanic => "worker_panic"
  | .internalError => "internal_error"

end ErrorCode

/-- Status values used by `doctor_result.checks`. -/
inductive CheckStatus where
  | ok
  | warning
  | failed
  | skipped
  deriving BEq, Repr

namespace CheckStatus

/-- The stable protocol spelling for a doctor check status. -/
def asString : CheckStatus → String
  | .ok => "ok"
  | .warning => "warning"
  | .failed => "failed"
  | .skipped => "skipped"

end CheckStatus

/-- One module descriptor supplied by a worker request. -/
structure ModuleDescriptor where
  module : String
  origin : String
  deriving Repr

/-- A parsed worker request with command-specific payload left opaque. -/
structure Request where
  requestId : String
  command : Command
  capabilities : Array String
  payload : Json
  extensions : Option Json

/-- A structured protocol error with whatever request context is available. -/
structure ProtocolError where
  requestId? : Option String
  command? : Option Command
  code : ErrorCode
  fatal : Bool
  message : String
  details : Option Json := none

/-- Semantic algorithm versions reported by the worker. -/
structure SemanticVersions where
  extract : String
  features : String
  probe : String
  deriving Repr

/-- Version information reported by the `version` command and embedded in doctor output. -/
structure VersionResult where
  protocolVersion : String
  workerVersion : String
  leanVersion : Option String
  semanticVersions : SemanticVersions
  supportedCommands : Array Command
  supportedCapabilities : Array String
  deriving Repr

/-- One structured health check emitted by `doctor`. -/
structure DoctorCheck where
  name : String
  status : CheckStatus
  message : Option String := none
  deriving Repr

/-- The aggregate `doctor` result. -/
structure DoctorResult where
  ok : Bool
  checks : Array DoctorCheck
  worker : VersionResult
  deriving Repr

/-- One worker response envelope before JSON serialization. -/
structure Envelope where
  requestId : String
  command : Command
  kind : ResponseKind
  payload : Json

private def jsonStringOrNull : Option String → Json
  | some value => Json.str value
  | none => Json.null

private def jsonCommandOrNull : Option Command → Json
  | some command => Json.str command.asString
  | none => Json.null

private def stringArrayJson (values : Array String) : Json :=
  Json.arr (values.map Json.str)

private def commandArrayJson (commands : Array Command) : Json :=
  Json.arr (commands.map fun command => Json.str command.asString)

/-- All protocol commands supported by this worker version. -/
def supportedCommands : Array Command :=
  #[.extract, .features, .probe, .doctor, .version]

/-- Optional protocol capabilities supported by this foundation worker. -/
def supportedCapabilities : Array String := #[]

/-- The current semantic version bundle reported by `version`. -/
def currentSemanticVersions : SemanticVersions :=
  { extract := LeanDup.Extract.version
    features := LeanDup.Features.version
    probe := LeanDup.Probe.version }

/-- Current version information for protocol responses. -/
def currentVersionResult : VersionResult :=
  { protocolVersion := schemaVersion
    workerVersion := workerVersion
    leanVersion := some s!"Lean {Lean.versionString}"
    semanticVersions := currentSemanticVersions
    supportedCommands := supportedCommands
    supportedCapabilities := supportedCapabilities }

/-- Encode semantic algorithm versions as protocol JSON. -/
def SemanticVersions.toJson (versions : SemanticVersions) : Json :=
  Json.mkObj
    [ ("extract", Json.str versions.extract)
    , ("features", Json.str versions.features)
    , ("probe", Json.str versions.probe)
    ]

/-- Encode version output as the `version_result` payload. -/
def VersionResult.toJson (result : VersionResult) : Json :=
  Json.mkObj
    [ ("protocol_version", Json.str result.protocolVersion)
    , ("worker_version", Json.str result.workerVersion)
    , ("lean_version", jsonStringOrNull result.leanVersion)
    , ("semantic_versions", result.semanticVersions.toJson)
    , ("supported_commands", commandArrayJson result.supportedCommands)
    , ("supported_capabilities", stringArrayJson result.supportedCapabilities)
    ]

/-- Encode one doctor health check as protocol JSON. -/
def DoctorCheck.toJson (check : DoctorCheck) : Json :=
  let base :=
    [ ("name", Json.str check.name)
    , ("status", Json.str check.status.asString)
    ]
  let fields :=
    match check.message with
    | some message => base ++ [("message", Json.str message)]
    | none => base
  Json.mkObj fields

/-- Encode a doctor result as the `doctor_result` payload. -/
def DoctorResult.toJson (result : DoctorResult) : Json :=
  Json.mkObj
    [ ("ok", Json.bool result.ok)
    , ("checks", Json.arr (result.checks.map DoctorCheck.toJson))
    , ("worker", result.worker.toJson)
    ]

/-- Encode a normal worker envelope as protocol JSON. -/
def Envelope.toJson (envelope : Envelope) : Json :=
  Json.mkObj
    [ ("schema_version", Json.str schemaVersion)
    , ("request_id", Json.str envelope.requestId)
    , ("command", Json.str envelope.command.asString)
    , ("kind", Json.str envelope.kind.asString)
    , ("payload", envelope.payload)
    ]

/-- Encode an error envelope, using null context fields when no request parsed. -/
def ProtocolError.toJson (err : ProtocolError) : Json :=
  let payload :=
    Json.mkObj
      ([ ("code", Json.str err.code.asString)
       , ("fatal", Json.bool err.fatal)
       , ("message", Json.str err.message)
       ] ++
       match err.details with
       | some details => [("details", details)]
       | none => [])
  Json.mkObj
    [ ("schema_version", Json.str schemaVersion)
    , ("request_id", jsonStringOrNull err.requestId?)
    , ("command", jsonCommandOrNull err.command?)
    , ("kind", Json.str ResponseKind.error.asString)
    , ("payload", payload)
    ]

/-- Build a normal response envelope for a parsed request. -/
def envelope (request : Request) (kind : ResponseKind) (payload : Json) : Envelope :=
  { requestId := request.requestId
    command := request.command
    kind := kind
    payload := payload }

private def natOrNull : Option Nat → Json
  | some value => Json.num value
  | none => Json.null

/-- Build an opaque progress payload for worker phase attribution. -/
def progressPayload
    (phase : String)
    (current? : Option Nat)
    (total? : Option Nat)
    (elapsedMs? : Option Nat)
    (message : String) : Json :=
  Json.mkObj
    [ ("phase", Json.str phase)
    , ("current", natOrNull current?)
    , ("total", natOrNull total?)
    , ("module", Json.null)
    , ("declaration", Json.null)
    , ("elapsed_ms", natOrNull elapsedMs?)
    , ("message", Json.str message)
    ]

/-- Build a progress envelope without exposing Lean implementation details. -/
def progressEnvelope
    (request : Request)
    (phase : String)
    (current? : Option Nat)
    (total? : Option Nat)
    (elapsedMs? : Option Nat)
    (message : String) : Envelope :=
  envelope request .progress (progressPayload phase current? total? elapsedMs? message)

private def countableKinds : Array ResponseKind :=
  #[ .versionResult
   , .doctorResult
   , .declarationRow
   , .featureRow
   , .probeResult
   , .progress
   , .error
   ]

private def countKind (target : ResponseKind) (kinds : Array ResponseKind) : Nat :=
  kinds.foldl (init := 0) fun count kind =>
    if kind == target then count + 1 else count

/-- Build the `row_counts` object for a `complete` payload. -/
def rowCountsJson (kinds : Array ResponseKind) : Json :=
  let fields :=
    countableKinds.foldl (init := []) fun fields kind =>
      let count := countKind kind kinds
      if count == 0 then fields else (kind.asString, Json.num count) :: fields
  Json.mkObj fields.reverse

/-- Build the terminal `complete` envelope for a successful worker command. -/
def completeEnvelope (request : Request) (emittedKinds : Array ResponseKind) : Envelope :=
  envelope request .complete <|
    Json.mkObj
      [ ("row_counts", rowCountsJson emittedKinds)
      , ("elapsed_ms", Json.null)
      ]

private def optionalJsonField (json : Json) (key : String) : Option Json :=
  match json.getObjVal? key with
  | .ok value => some value
  | .error _ => none

private def payloadJson (json : Json) : Json :=
  match optionalJsonField json "payload" with
  | some payload => payload
  | none => json

private def stringField? (json : Json) (key : String) : Except String String :=
  json.getObjValAs? String key

private def optionalString? (json : Json) (key : String) : Option String :=
  match json.getObjValAs? String key with
  | .ok value => some value
  | .error _ => none

private def invalidRequest
    (requestId? : Option String)
    (command? : Option Command)
    (message : String)
    (details : Option Json := none) : ProtocolError :=
  { requestId? := requestId?
    command? := command?
    code := .invalidRequest
    fatal := true
    message := message
    details := details }

private def parseStringArray
    (json : Json)
    (field : String)
    (requestId? : Option String)
    (command? : Option Command) : Except ProtocolError (Array String) := do
  match optionalJsonField json field with
  | none => pure #[]
  | some (Json.arr values) =>
      let mut parsed := #[]
      for value in values do
        match value with
        | Json.str text => parsed := parsed.push text
        | _ => throw <| invalidRequest requestId? command? s!"`{field}` must contain only strings"
      pure parsed
  | some _ => throw <| invalidRequest requestId? command? s!"`{field}` must be an array"

/-- Parse a JSON worker request and validate schema, command, and capabilities. -/
def Request.parse (json : Json) : Except ProtocolError Request := do
  let requestId? := optionalString? json "request_id"
  let schema ←
    match stringField? json "schema_version" with
    | .ok schema => pure schema
    | .error _ => throw <| invalidRequest requestId? none "missing string `schema_version`"
  if schema != schemaVersion then
    throw
      { requestId? := requestId?
        command? := none
        code := .unsupportedSchema
        fatal := true
        message := s!"unsupported schema `{schema}`"
        details := some <| Json.mkObj [("supported_schema", Json.str schemaVersion)] }
  let requestId ←
    match requestId? with
    | some requestId =>
        if requestId.isEmpty then
          throw <| invalidRequest requestId? none "`request_id` must be nonempty"
        else
          pure requestId
    | none => throw <| invalidRequest none none "missing string `request_id`"
  let commandText ←
    match stringField? json "command" with
    | .ok commandText => pure commandText
    | .error _ => throw <| invalidRequest (some requestId) none "missing string `command`"
  let command ←
    match Command.parse? commandText with
    | some command => pure command
    | none =>
        throw
          { requestId? := some requestId
            command? := none
            code := .unsupportedCommand
            fatal := true
            message := s!"unsupported command `{commandText}`"
            details := some <| Json.mkObj [("supported_commands", commandArrayJson supportedCommands)] }
  let capabilities ← parseStringArray json "capabilities" (some requestId) (some command)
  let unsupported := capabilities.filter fun capability =>
    !(supportedCapabilities.contains capability)
  if !unsupported.isEmpty then
    throw <|
      invalidRequest
        (some requestId)
        (some command)
        "request requires unsupported capabilities"
        (some <|
          Json.mkObj
            [ ("unsupported_capabilities", stringArrayJson unsupported)
            , ("supported_capabilities", stringArrayJson supportedCapabilities)
            ])
  pure
    { requestId := requestId
      command := command
      capabilities := capabilities
      payload := payloadJson json
      extensions := optionalJsonField json "extensions" }

private def parseModuleDescriptor
    (json : Json)
    (request : Request) : Except ProtocolError ModuleDescriptor := do
  match json with
  | Json.obj _ =>
      let moduleName ←
        match json.getObjValAs? String "module" with
        | .ok moduleName =>
            if moduleName.isEmpty then
              throw <| invalidRequest (some request.requestId) (some request.command) "module name must be nonempty"
            else
              pure moduleName
        | .error _ =>
            throw <| invalidRequest (some request.requestId) (some request.command) "module descriptors need string `module`"
      let origin ←
        match json.getObjValAs? String "origin" with
        | .ok origin =>
            if origin.isEmpty then pure "workspace" else pure origin
        | .error _ => pure "workspace"
      pure { module := moduleName, origin := origin }
  | _ =>
      throw <| invalidRequest (some request.requestId) (some request.command) "module descriptors must be JSON objects"

/-- Parse optional command payload modules from a request. -/
def Request.modules (request : Request) : Except ProtocolError (Array ModuleDescriptor) := do
  match optionalJsonField request.payload "modules" with
  | none => pure #[]
  | some (Json.arr values) =>
      let mut modules := #[]
      for value in values do
        modules := modules.push (← parseModuleDescriptor value request)
      pure modules
  | some _ =>
      throw <| invalidRequest (some request.requestId) (some request.command) "`modules` must be an array"

/-- Build a malformed JSON error without request context. -/
def malformedJsonError (message : String) : ProtocolError :=
  { requestId? := none
    command? := none
    code := .malformedJson
    fatal := true
    message := message
    details := none }

end LeanDup.Protocol
