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
def version : String := "extract.declarations.v1"

/-- One requested Lean module and the origin label Rust wants attached to its rows. -/
structure ModuleSpec where
  module : String
  origin : String
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
  deriving Repr

private structure Context where
  modules : Array ModuleSpec
  options : Options

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

private def parseOptions (payload : Json) : Except Error Options := do
  let workspaceRoot? ← parseWorkspaceRoot payload
  let includePrivate ← parseBoolField payload "include_private" false
  let includeGenerated ← parseBoolField payload "include_generated" false
  pure { workspaceRoot?, includePrivate, includeGenerated }

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

private def sourceSpanJson? (options : Options) (moduleName : String)
    (range? : Option DeclarationRanges) : Option Json :=
  match options.workspaceRoot?, range? with
  | some workspaceRoot, some ranges =>
      some <|
        Json.mkObj
          [ ("file", Json.str (moduleFile workspaceRoot moduleName))
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

private def declarationId (moduleSpec : ModuleSpec) (declName : Name) : String :=
  s!"{moduleSpec.origin}:{moduleSpec.module}:{declName}"

private def statementText (kind displayName typeText : String) : String :=
  s!"{kind} {displayName} : {typeText}"

private def rowPayload (context : Context) (moduleSpec : ModuleSpec) (declName : Name)
    (constInfo : ConstantInfo) (generated : Bool) (range? : Option DeclarationRanges)
    (typeText : String) : Json :=
  let kind := declarationKind constInfo
  let vis := visibility declName
  let sourceSpan? := sourceSpanJson? context.options moduleSpec.module range?
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
    , ("status_flags", statusFlagsJson generated sourceSpan?)
    ]

private def collectModuleRows (context : Context) (moduleSpec : ModuleSpec) :
    MetaM (Array Json) := do
  let env ← getEnv
  let moduleName := dottedName moduleSpec.module
  let some moduleIdx := env.header.moduleNames.idxOf? moduleName | return #[]
  let moduleData := env.header.moduleData[moduleIdx]!
  let mut rows := #[]
  for declName in moduleData.constNames do
    let some constInfo := env.find? declName | continue
    let range? ← findDeclarationRanges? declName
    let generatedByShape ← isGeneratedDeclaration declName
    let generated := generatedByShape || range?.isNone
    if generated && !context.options.includeGenerated then
      continue
    if visibility declName == "private" && !context.options.includePrivate then
      continue
    let typeText := (← ppExpr constInfo.type).pretty
    rows := rows.push <| rowPayload context moduleSpec declName constInfo generated range? typeText
  pure rows

private def collectRows (context : Context) : MetaM (Array Json) := do
  let mut rows := #[]
  for moduleSpec in context.modules do
    let moduleRows ← collectModuleRows context moduleSpec
    for row in moduleRows do
      rows := rows.push row
  pure rows

private def uniqueModuleImports (modules : Array ModuleSpec) : Array Import := Id.run do
  let mut seen : Std.HashSet String := {}
  let mut imports := #[]
  for moduleSpec in modules do
    if !seen.contains moduleSpec.module then
      seen := seen.insert moduleSpec.module
      imports := imports.push ({ module := dottedName moduleSpec.module } : Import)
  imports

private unsafe def importRequestedModules (modules : Array ModuleSpec) :
    IO (Except Error Environment) := do
  Lean.enableInitializersExecution
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
Import the requested modules once and return declaration-row payloads accepted
by the supplied extraction options.
-/
unsafe def run (payload : Json) (modules : Array ModuleSpec) :
    IO (Except Error (Array Json)) := do
  if modules.isEmpty then
    return .error <| invalidRequest "`extract` requires at least one module"
  match parseOptions payload with
  | .error err => pure <| .error err
  | .ok options =>
      match ← importRequestedModules modules with
      | .error err => pure <| .error err
      | .ok env =>
          let context : Context := { modules := modules, options := options }
          let coreContext : Core.Context :=
            { fileName := "<lean-dup-extract>"
              fileMap := default
              options := Options.empty }
          try
            let (rows, _, _) ←
              MetaM.toIO
                (collectRows context)
                coreContext
                { env := env }
                {}
                {}
            pure <| .ok rows
          catch error =>
            pure <|
              .error
                { kind := .internalError
                  message := s!"declaration extraction failed: {error}"
                  details := none }

end LeanDup.Extract
