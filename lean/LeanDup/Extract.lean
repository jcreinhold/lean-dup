import Lean
import Lean.Server.InfoUtils

/-!
`LeanDup.Extract` owns declaration extraction facts from the Lean environment.

Callers may rely on declaration/display/source facts emitted through the worker
protocol. They must not depend on environment traversal order, temporary
Python-era manifest behavior, source parsing fallback policy, or Rust cache
layout.
-/
namespace LeanDup.Extract

open Lean
open Lean.Meta

/-- Semantic algorithm marker for declaration extraction rows. -/
def version : String := "extract.declarations.v2"

/-- One requested Lean module and the origin label Rust wants attached to its rows. -/
structure ModuleSpec where
  module : String
  origin : String
  sourceRoot? : Option String := none
  deriving Repr

/-- Extract errors are mapped by the worker into protocol error envelopes. -/
inductive ErrorKind where
  | invalidRequest
  | importFailed
  | internalError
  deriving BEq, Repr

/-- A bounded extraction failure with optional machine-readable details. -/
structure Error where
  kind : ErrorKind
  message : String
  details : Option Json := none

/--
Options that affect which declaration facts are emitted.

`statement_text` remains display-only; callers must not treat it as a semantic
feature source.
-/
structure Options where
  workspaceRoot? : Option String
  includePrivate : Bool
  includeGenerated : Bool
  /-- Per-declaration elaboration heartbeat budget (the value Lean prints in a
      timeout message). `0` disables the limit. -/
  maxHeartbeats : Nat
  deriving Repr

/-- Extraction context shared by worker commands that reuse one imported environment. -/
structure Context where
  modules : Array ModuleSpec
  options : Options

/-- Coarse worker cost facts for one semantic command. -/
structure RunStats where
  importMs : Nat
  semanticMs : Nat
  declarationCount : Nat
  rowCount : Nat
  /-- Declarations skipped because their elaboration exceeded the heartbeat
      budget. Non-fatal: the command still completes with the remaining rows. -/
  skippedCount : Nat := 0

/-- Semantic rows plus coarse cost facts. -/
structure RunOutput where
  rows : Array Json
  stats : RunStats

/--
One declaration accepted by extraction filters.

This value is for other Lean worker modules that need the same declaration
universe as `extract`; it is not a protocol row and is never emitted directly.
-/
structure AcceptedDeclaration where
  moduleSpec : ModuleSpec
  declName : Name
  constInfo : ConstantInfo
  generated : Bool
  range? : Option DeclarationRanges

private def invalidRequest (message : String) (details : Option Json := none) : Error :=
  { kind := .invalidRequest, message := message, details := details }

private def optionalJsonField (json : Json) (key : String) : Option Json :=
  match json.getObjVal? key with
  | .ok value => some value
  | .error _ => none

private def stringArrayJson (values : Array String) : Json :=
  Json.arr (values.map Json.str)

private def moduleArrayJson (modules : Array ModuleSpec) : Json :=
  Json.arr (modules.map fun moduleSpec => Json.str moduleSpec.module)

private def parseBoolField (json : Json) (key : String) (default : Bool) :
    Except Error Bool := do
  match optionalJsonField json key with
  | none => pure default
  | some (Json.bool value) => pure value
  | some _ => throw <| invalidRequest s!"`{key}` must be a boolean"

private def parseWorkspaceRoot (json : Json) : Except Error (Option String) := do
  match optionalJsonField json "workspace_root" with
  | none | some Json.null => pure none
  | some (Json.str value) =>
      if value.isEmpty then pure none else pure (some value)
  | some _ => throw <| invalidRequest "`workspace_root` must be a string or null"

private def parseNatField (json : Json) (key : String) (default : Nat) :
    Except Error Nat := do
  match optionalJsonField json key with
  | none | some Json.null => pure default
  | some (Json.num value) =>
      match value.toString.toNat? with
      | some parsed => pure parsed
      | none => throw <| invalidRequest s!"`{key}` must be a natural number"
  | some _ => throw <| invalidRequest s!"`{key}` must be a natural number"

/--
Parse extraction options shared by declaration, feature, and index commands.

The options describe caller-visible filtering policy only; import scheduling
and chunking remain owned by the worker implementation.
-/
def parseOptions (payload : Json) : Except Error Options := do
  let workspaceRoot? ← parseWorkspaceRoot payload
  let includePrivate ← parseBoolField payload "include_private" false
  let includeGenerated ← parseBoolField payload "include_generated" false
  let maxHeartbeats ← parseNatField payload "max_heartbeats" 200000
  pure { workspaceRoot?, includePrivate, includeGenerated, maxHeartbeats }

private def dottedName (text : String) : Name :=
  (text.splitOn ".").foldl
    (init := Name.anonymous)
    fun current segment =>
      if segment.isEmpty then current else Name.str current segment

private def shortName : Name → String
  | .anonymous => "_anonymous"
  | .str _ segment => segment
  | .num parent _ => shortName parent

private def displayName (declName : Name) : String :=
  shortName (privateToUserName declName)

private def visibility (declName : Name) : String :=
  if isPrivateName declName then "private" else "public"

private def declarationKind : ConstantInfo → String
  | .thmInfo _ => "theorem"
  | .defnInfo info => if info.hints.isAbbrev then "abbrev" else "def"
  | .opaqueInfo _ => "opaque"
  | .axiomInfo _ => "axiom"
  | .inductInfo _ => "inductive"
  | .ctorInfo _ => "constructor"
  | .recInfo _ => "recursor"
  | .quotInfo _ => "quot"

private def pointJson (line column : Nat) : Json :=
  Json.mkObj [("line", Json.num line), ("column", Json.num column)]

private def moduleFile (workspaceRoot moduleName : String) : String :=
  s!"{workspaceRoot}/{moduleName.replace "." "/"}.lean"

private def sourceSpanJson? (options : Options) (moduleSpec : ModuleSpec)
    (range? : Option DeclarationRanges) : Option Json :=
  let sourceRoot? :=
    match moduleSpec.sourceRoot? with
    | some root => some root
    | none => options.workspaceRoot?
  match sourceRoot?, range? with
  | some workspaceRoot, some ranges =>
      some <|
        Json.mkObj
          [ ("file", Json.str (moduleFile workspaceRoot moduleSpec.module))
          , ("start", pointJson ranges.range.pos.line ranges.range.pos.column)
          , ("end", pointJson ranges.range.endPos.line ranges.range.endPos.column)
          ]
  | _, _ => none

private def generatedShortNames : Array String :=
  #[ "rec"
   , "recOn"
   , "casesOn"
   , "noConfusion"
   , "noConfusionType"
   , "below"
   , "brecOn"
   , "ibelow"
   , "binductionOn"
   , "ctorElim"
   , "elim"
   ]

private def generatedNameShape (declName : Name) : Bool :=
  let text := declName.toString
  let short := shortName declName
  generatedShortNames.contains short ||
    text.contains "._aux_" ||
    text.contains "._unexpand_" ||
    text.contains "._macroRules_" ||
    text.contains ".match_" ||
    text.contains ".proof_" ||
    (short.startsWith "_aux_")

private def isGeneratedDeclaration (declName : Name) : MetaM Bool := do
  let env ← getEnv
  if env.isProjectionFn declName then
    return false
  if isPrivateName declName then
    return false
  let isRecursor ← isRec declName
  let isMatcherDecl ← Lean.Meta.isMatcher declName
  let isMatcherLikeDecl ← Lean.Meta.isMatcherLike declName
  pure <|
    isAuxRecursor env declName ||
      isNoConfusion env declName ||
      isRecursor ||
      isMatcherDecl ||
      isMatcherLikeDecl ||
      declName.isInternal ||
      declName.isInternalDetail ||
      generatedNameShape declName

private def modifiersJson (visibility : String) : Json :=
  let modifiers :=
    if visibility == "private" then #["private"] else #[]
  stringArrayJson modifiers

private def statusFlagsJson (generated : Bool) (sourceSpan? : Option Json) : Json :=
  let flags := #[]
  let flags := if generated then flags.push "generated" else flags
  let flags :=
    match sourceSpan? with
    | some _ => flags
    | none => flags.push "source-range-unavailable"
  stringArrayJson flags

/-- Build the opaque declaration id shared by declaration and feature rows. -/
def declarationId (moduleSpec : ModuleSpec) (declName : Name) : String :=
  s!"{moduleSpec.origin}:{moduleSpec.module}:{declName}"

/-- Return the opaque declaration id for an accepted declaration. -/
def AcceptedDeclaration.declarationId (decl : AcceptedDeclaration) : String :=
  LeanDup.Extract.declarationId decl.moduleSpec decl.declName

private def statementText (kind displayName typeText : String) : String :=
  s!"{kind} {displayName} : {typeText}"

private def definitionBody? : ConstantInfo → Option Expr
  | .defnInfo info => some info.value
  | _ => none

private def maxDefinitionBodyChars : Nat := 4000

private def boundedSemanticText (text : String) : String :=
  let normalized := text
  if normalized.length <= maxDefinitionBodyChars then
    normalized
  else
    (normalized.take maxDefinitionBodyChars).toString ++ " ..."

private def rowPayload (options : Options) (moduleSpec : ModuleSpec) (declName : Name)
    (constInfo : ConstantInfo) (generated : Bool) (range? : Option DeclarationRanges)
    (typeText : String) (docString? : Option String) (definitionBodySummary? : Option String) : Json :=
  let kind := declarationKind constInfo
  let vis := visibility declName
  let sourceSpan? := sourceSpanJson? options moduleSpec range?
  Json.mkObj
    [ ("declaration_id", Json.str (declarationId moduleSpec declName))
    , ("origin", Json.str moduleSpec.origin)
    , ("module", Json.str moduleSpec.module)
    , ("qualified_name", Json.str declName.toString)
    , ("display_name", Json.str (displayName declName))
    , ("kind", Json.str kind)
    , ("visibility", Json.str vis)
    , ("modifiers", modifiersJson vis)
    , ("source_span", sourceSpan?.getD Json.null)
    , ("statement_text", Json.str (statementText kind (displayName declName) typeText))
    , ("docstring_text", docString?.map Json.str |>.getD Json.null)
    , ("definition_body_summary", definitionBodySummary?.map Json.str |>.getD Json.null)
    , ("status_flags", statusFlagsJson generated sourceSpan?)
    ]

private def collectModuleDeclarations (context : Context) (moduleSpec : ModuleSpec) :
    MetaM (Array AcceptedDeclaration) := do
  let env ← getEnv
  let moduleName := dottedName moduleSpec.module
  let some moduleIdx := env.header.moduleNames.idxOf? moduleName | return #[]
  let moduleData := env.header.moduleData[moduleIdx]!
  let mut declarations := #[]
  for declName in moduleData.constNames do
    let some constInfo := env.find? declName | continue
    let range? ← findDeclarationRanges? declName
    let generatedByShape ← isGeneratedDeclaration declName
    let generated := generatedByShape || range?.isNone
    if generated && !context.options.includeGenerated then
      continue
    if visibility declName == "private" && !context.options.includePrivate then
      continue
    declarations :=
      declarations.push
        { moduleSpec := moduleSpec
          declName := declName
          constInfo := constInfo
          generated := generated
          range? := range? }
  pure declarations

/--
Collect declarations accepted by extraction filters from the current imported
environment.

The returned order is deterministic for one environment but is not a public
protocol contract.
-/
def collectAcceptedDeclarations (context : Context) : MetaM (Array AcceptedDeclaration) := do
  let mut declarations := #[]
  for moduleSpec in context.modules do
    let moduleDeclarations ← collectModuleDeclarations context moduleSpec
    for declaration in moduleDeclarations do
      declarations := declarations.push declaration
  pure declarations

/-- Elaboration options carrying the request's per-declaration heartbeat budget.
    A budget of `0` disables the limit (Lean convention for `maxHeartbeats`). -/
def elaborationOptions (options : Options) : Lean.Options :=
  Lean.maxHeartbeats.set Lean.Options.empty options.maxHeartbeats

/--
Process each accepted declaration under its own heartbeat budget, skipping (and
counting) any declaration whose elaboration exceeds the budget.

`Core.withCurrHeartbeats` resets the heartbeat baseline per declaration, so each
gets the full budget; a heartbeat timeout in one declaration is caught and that
declaration is omitted rather than failing the whole command. Non-heartbeat
errors still propagate. Returns the successful payloads and the skipped count.
-/
def forEachDeclarationSkippingSlow {α : Type}
    (declarations : Array AcceptedDeclaration)
    (body : AcceptedDeclaration → MetaM α) : MetaM (Array α × Nat) := do
  let mut rows := #[]
  let mut skipped := 0
  for declaration in declarations do
    try
      let row ← Core.withCurrHeartbeats (body declaration)
      rows := rows.push row
    catch ex =>
      if ex.isMaxHeartbeat then
        skipped := skipped + 1
      else
        throw ex
  pure (rows, skipped)

/--
Encode one accepted declaration as the protocol declaration-row payload.

The payload contains display/source facts. Semantic comparison must use the
feature rows emitted by `LeanDup.Features`.
-/
def rowPayloadFromAccepted (options : Options) (decl : AcceptedDeclaration) : MetaM Json := do
  let typeText := (← ppExpr decl.constInfo.type).pretty
  let docString? ← findDocString? (← getEnv) decl.declName (includeBuiltin := false)
  let definitionBodySummary? ←
    match definitionBody? decl.constInfo with
    | some body =>
        let bodyText := (← ppExpr body).pretty
        pure <| some (boundedSemanticText bodyText)
    | none => pure none
  pure <|
    rowPayload
      options
      decl.moduleSpec
      decl.declName
      decl.constInfo
      decl.generated
      decl.range?
      typeText
      (docString?.map boundedSemanticText)
      definitionBodySummary?

private def collectRows (options : Options) (declarations : Array AcceptedDeclaration) :
    MetaM (Array Json × Nat) :=
  forEachDeclarationSkippingSlow declarations (rowPayloadFromAccepted options)

private def uniqueModuleImports (modules : Array ModuleSpec) : Array Import := Id.run do
  let mut seen : Std.HashSet String := {}
  let mut imports := #[]
  for moduleSpec in modules do
    if !seen.contains moduleSpec.module then
      seen := seen.insert moduleSpec.module
      imports := imports.push ({ module := dottedName moduleSpec.module } : Import)
  imports

/--
Import the distinct requested modules and return the resulting environment.

Callers provide module descriptors; this function owns Lean import mechanics.
Subprocess callers use the default search-path initialization. Capability
callers set `initializeSearchPath := false` because the host session has
already installed the audited workspace's import search path.
-/
unsafe def importRequestedModules (modules : Array ModuleSpec)
    (initializeSearchPath : Bool := true) :
    IO (Except Error Environment) := do
  Lean.enableInitializersExecution
  if initializeSearchPath then
    initSearchPath (← getBuildDir)
  let imports := uniqueModuleImports modules
  try
    let env ← importModules imports Options.empty (loadExts := true)
    pure <| .ok env
  catch error =>
    pure <|
      .error
        { kind := .importFailed
          message := s!"could not import requested modules: {error}"
          details := some <| Json.mkObj [("modules", moduleArrayJson modules)] }

/--
Import requested modules and run a Lean action over declarations accepted by the
same filters used by `extract`.

The action receives Lean environment facts, not protocol rows. Callers must emit
their own command-specific payloads and must not treat `statement_text` or source
snippets as semantic input.
-/
unsafe def withAcceptedDeclarationsProfiled {α : Type}
    (payload : Json)
    (modules : Array ModuleSpec)
    (operation : Options → Array AcceptedDeclaration → MetaM α)
    (initializeSearchPath : Bool := true) :
    IO (Except Error (α × RunStats)) := do
  if modules.isEmpty then
    return .error <| invalidRequest "`modules` must contain at least one module"
  match parseOptions payload with
  | .error err => pure <| .error err
  | .ok options =>
      let importStarted ← IO.monoMsNow
      match ← importRequestedModules modules initializeSearchPath with
      | .error err => pure <| .error err
      | .ok env =>
          let importFinished ← IO.monoMsNow
          let context : Context := { modules := modules, options := options }
          let coreContext : Core.Context :=
            { fileName := "<lean-dup-extract>"
              fileMap := default
              options := elaborationOptions options }
          try
            let semanticStarted ← IO.monoMsNow
            let (result, _, _) ←
              MetaM.toIO
                (do
                  let declarations ← collectAcceptedDeclarations context
                  let result ← operation options declarations
                  pure (result, declarations.size))
                coreContext
                { env := env }
                {}
                {}
            let semanticFinished ← IO.monoMsNow
            let stats : RunStats :=
              { importMs := importFinished - importStarted
                semanticMs := semanticFinished - semanticStarted
                declarationCount := result.2
                rowCount := 0 }
            pure <| .ok (result.1, stats)
          catch error =>
            pure <|
              .error
                { kind := .internalError
                  message := s!"declaration processing failed: {error}"
                  details := none }

/--
Import the requested modules once and run a Lean action over declarations
accepted by the same filters used by `extract`.

The action receives Lean environment facts, not protocol rows. Callers must emit
their own command-specific payloads and must not treat `statement_text` or source
snippets as semantic input.
-/
unsafe def withAcceptedDeclarations {α : Type}
    (payload : Json)
    (modules : Array ModuleSpec)
    (operation : Options → Array AcceptedDeclaration → MetaM α)
    (initializeSearchPath : Bool := true) :
    IO (Except Error α) := do
  match ← withAcceptedDeclarationsProfiled payload modules operation
      (initializeSearchPath := initializeSearchPath) with
  | .error err => pure <| .error err
  | .ok (result, _stats) => pure <| .ok result

/--
Import the requested modules once and return declaration-row payloads accepted
by the supplied extraction options.
-/
unsafe def runProfiled (payload : Json) (modules : Array ModuleSpec)
    (initializeSearchPath : Bool := true) :
    IO (Except Error RunOutput) := do
  match ← withAcceptedDeclarationsProfiled
      payload
      modules
      (fun options declarations => collectRows options declarations)
      (initializeSearchPath := initializeSearchPath) with
  | .error err => pure <| .error err
  | .ok ((rows, skipped), stats) =>
      pure <| .ok { rows := rows, stats := { stats with rowCount := rows.size, skippedCount := skipped } }

unsafe def run (payload : Json) (modules : Array ModuleSpec) :
    IO (Except Error (Array Json)) := do
  match ← runProfiled payload modules with
  | .error err => pure <| .error err
  | .ok output => pure <| .ok output.rows

end LeanDup.Extract
