import Lean
import Lean.Server.InfoUtils

open Lean
open Lean.Meta

namespace LeanDup

private def schemaVersion : String := "lean-dup.declarations.v1"

private def dottedName (text : String) : Name :=
  (text.splitOn ".").foldl
    (init := Name.anonymous)
    fun current segment =>
      if segment.isEmpty then current else Name.str current segment

private structure ModuleEntry where
  name : String
  origin : String

private def parseModuleManifest (path : System.FilePath) : IO (Except String (Array ModuleEntry)) := do
  let text ← IO.FS.readFile path
  match Json.parse text with
  | Except.error err => pure <| Except.error s!"could not parse module manifest: {err}"
  | Except.ok (Json.arr rows) =>
      let mut modules := #[]
      for row in rows do
        match row with
        | Json.str moduleName => modules := modules.push { name := moduleName, origin := "workspace" }
        | Json.obj _ =>
            match row.getObjValAs? String "name", row.getObjValAs? String "origin" with
            | Except.ok moduleName, Except.ok origin => modules := modules.push { name := moduleName, origin }
            | _, _ => return Except.error "module manifest objects need string `name` and `origin`"
        | _ => return Except.error "module manifest must be a JSON array of strings or objects"
      pure <| Except.ok modules
  | Except.ok _ => pure <| Except.error "module manifest must be a JSON array"

private def exprFingerprint : Expr → String
  | .bvar idx => s!"b{idx}"
  | .fvar _ => "fvar"
  | .mvar _ => "mvar"
  | .sort _ => "Sort"
  | .const name _ => s!"C:{name}"
  | .app fn arg => s!"(app {exprFingerprint fn} {exprFingerprint arg})"
  | .lam _ domain body _ => s!"(lam {exprFingerprint domain} {exprFingerprint body})"
  | .forallE _ domain body _ => s!"(forall {exprFingerprint domain} {exprFingerprint body})"
  | .letE _ type value body _ =>
      s!"(let {exprFingerprint type} {exprFingerprint value} {exprFingerprint body})"
  | .lit (.natVal value) => s!"N:{value}"
  | .lit (.strVal value) => s!"S:{value}"
  | .mdata _ body => exprFingerprint body
  | .proj typeName idx body => s!"(proj {typeName}.{idx} {exprFingerprint body})"

private partial def collectConstants (expr : Expr) (acc : Std.HashSet Name := {}) : Std.HashSet Name :=
  match expr with
  | .const name _ => acc.insert name
  | .app fn arg => collectConstants arg (collectConstants fn acc)
  | .lam _ domain body _ => collectConstants body (collectConstants domain acc)
  | .forallE _ domain body _ => collectConstants body (collectConstants domain acc)
  | .letE _ type value body _ => collectConstants body (collectConstants value (collectConstants type acc))
  | .mdata _ body => collectConstants body acc
  | .proj typeName _ body => collectConstants body (acc.insert typeName)
  | _ => acc

private partial def containsBVar : Expr → Bool
  | .bvar _ => true
  | .app fn arg => containsBVar fn || containsBVar arg
  | .lam _ domain body _ => containsBVar domain || containsBVar body
  | .forallE _ domain body _ => containsBVar domain || containsBVar body
  | .letE _ type value body _ => containsBVar type || containsBVar value || containsBVar body
  | .mdata _ body => containsBVar body
  | .proj _ _ body => containsBVar body
  | _ => false

private def stripForalls : Expr → Expr
  | .forallE _ _ body _ => stripForalls body
  | expr => expr

private def binderCount : Expr → Nat
  | .forallE _ _ body _ => binderCount body + 1
  | _ => 0

private partial def headConstants (expr : Expr) : Array String :=
  match stripForalls expr with
  | .app fn _ => headConstants fn
  | .const name _ => #[name.toString]
  | .sort _ => #["Sort"]
  | .forallE .. => #["forall"]
  | .lam .. => #["lambda"]
  | .letE .. => #["let"]
  | .proj typeName _ _ => #[typeName.toString]
  | _ => #[]

private partial def appFnArgs (expr : Expr) : Expr × Array Expr :=
  let rec go (current : Expr) (args : Array Expr) :=
    match current with
    | .app fn arg => go fn (args.push arg)
    | other => (other, args.reverse)
  go expr #[]

private def sortParts (parts : Array String) : String :=
  String.intercalate "," (parts.qsort (· < ·)).toList

private partial def connectiveFingerprint (expr : Expr) : String :=
  match expr with
  | .forallE _ domain body _ =>
      s!"(forall {connectiveFingerprint domain} {connectiveFingerprint body})"
  | .app .. =>
      let (head, args) := appFnArgs expr
      match head, args.toList with
      | .const ``And _, [left, right] =>
          s!"(And {sortParts #[connectiveFingerprint left, connectiveFingerprint right]})"
      | .const ``Or _, [left, right] =>
          s!"(Or {sortParts #[connectiveFingerprint left, connectiveFingerprint right]})"
      | .const ``Iff _, [left, right] =>
          s!"(Iff {sortParts #[connectiveFingerprint left, connectiveFingerprint right]})"
      | .const ``Eq _, [_type, left, right] =>
          s!"(Eq {sortParts #[connectiveFingerprint left, connectiveFingerprint right]})"
      | _, _ => exprFingerprint expr
  | .mdata _ body => connectiveFingerprint body
  | other => exprFingerprint other

private partial def looseFingerprint : Expr → String
  | .bvar _ => "b"
  | .fvar _ => "fvar"
  | .mvar _ => "mvar"
  | .sort _ => "Sort"
  | .const name _ => s!"C:{name}"
  | .app fn arg => s!"(app {looseFingerprint fn} {looseFingerprint arg})"
  | .lam _ domain body _ => s!"(lam {looseFingerprint domain} {looseFingerprint body})"
  | .forallE _ domain body _ => s!"(forall {looseFingerprint domain} {looseFingerprint body})"
  | .letE _ type value body _ =>
      s!"(let {looseFingerprint type} {looseFingerprint value} {looseFingerprint body})"
  | .lit (.natVal value) => s!"N:{value}"
  | .lit (.strVal value) => s!"S:{value}"
  | .mdata _ body => looseFingerprint body
  | .proj typeName idx body => s!"(proj {typeName}.{idx} {looseFingerprint body})"

private partial def looseConnectiveFingerprint (expr : Expr) : String :=
  match expr with
  | .forallE _ domain body _ =>
      s!"(forall {looseConnectiveFingerprint domain} {looseConnectiveFingerprint body})"
  | .app .. =>
      let (head, args) := appFnArgs expr
      match head, args.toList with
      | .const ``And _, [left, right] =>
          s!"(And {sortParts #[looseConnectiveFingerprint left, looseConnectiveFingerprint right]})"
      | .const ``Or _, [left, right] =>
          s!"(Or {sortParts #[looseConnectiveFingerprint left, looseConnectiveFingerprint right]})"
      | .const ``Iff _, [left, right] =>
          s!"(Iff {sortParts #[looseConnectiveFingerprint left, looseConnectiveFingerprint right]})"
      | .const ``Eq _, [_type, left, right] =>
          s!"(Eq {sortParts #[looseConnectiveFingerprint left, looseConnectiveFingerprint right]})"
      | _, _ => looseFingerprint expr
  | .mdata _ body => looseConnectiveFingerprint body
  | other => looseFingerprint other

private def collectForalls (expr : Expr) : Array Expr × Expr :=
  let rec go (current : Expr) (domains : Array Expr) :=
    match current with
    | .forallE _ domain body _ => go body (domains.push domain)
    | other => (domains, other)
  go expr #[]

private partial def renderPermutationDomains (domains : Array (Expr × Bool)) : String := Id.run do
  let mut parts := #[]
  let mut propSegment := #[]
  for (domain, isPropDomain) in domains do
    if isPropDomain then
      propSegment := propSegment.push (looseConnectiveFingerprint domain)
    else
      if !propSegment.isEmpty then
        parts := parts.push (sortParts propSegment)
        propSegment := #[]
      parts := parts.push (connectiveFingerprint domain)
  if !propSegment.isEmpty then
    parts := parts.push (sortParts propSegment)
  String.intercalate "," parts.toList

private def isSortDomain (type : Expr) : Bool :=
  match type with
  | .sort _ => true
  | _ => false

private def permutationFingerprint (expr : Expr) : MetaM String := do
  forallTelescope expr fun xs body => do
    let mut domains := #[]
    let mut canPermute := true
    for x in xs do
      let localDecl ← getFVarLocalDecl x
      let isPropDomain ← isProp localDecl.type
      if !isPropDomain && !isSortDomain localDecl.type then
        canPermute := false
      domains := domains.push (localDecl.type, isPropDomain)
    if canPermute then
      pure s!"(forall* [{renderPermutationDomains domains}] {looseConnectiveFingerprint body})"
    else
      pure <| exprFingerprint expr

private def pointJson (line column : Nat) : Json :=
  Json.mkObj [("line", Json.num line), ("column", Json.num column)]

private def spanJson (range : DeclarationRange) : Json :=
  Json.mkObj
    [ ("start", pointJson range.pos.line range.pos.column)
    , ("end", pointJson range.endPos.line range.endPos.column)
    ]

private def jsonLine
    (workspace moduleName origin filePath declKind visibility : String)
    (declName : Name)
    (typeText normalized permutation connective conclusion : String)
    (constants heads : Array String)
    (binders : Nat)
    (range : DeclarationRange) : Json :=
  Json.mkObj
    [ ("schema_version", Json.str schemaVersion)
    , ("workspace", Json.str workspace)
    , ("module", Json.str moduleName)
    , ("origin", Json.str origin)
    , ("file", Json.str filePath)
    , ("name", Json.str declName.toString)
    , ("display_name", Json.str declName.getString!)
    , ("short_name", Json.str declName.getString!)
    , ("kind", Json.str declKind)
    , ("visibility", Json.str visibility)
    , ("modifiers", Json.arr (if visibility == "private" then #[Json.str "private"] else #[]))
    , ("span", spanJson range)
    , ("type_text", Json.str typeText)
    , ("normalized_type", Json.str normalized)
    , ("permutation_normalized_type", Json.str permutation)
    , ("connective_normalized_type", Json.str connective)
    , ("conclusion_normalized_type", Json.str conclusion)
    , ("constants", Json.arr (constants.map Json.str))
    , ("type_heads", Json.arr (heads.map Json.str))
    , ("binder_count", Json.num binders)
    , ("source_fingerprint", Json.null)
    ]

private def declarationKind? : ConstantInfo → Option String
  | .thmInfo _ => some "theorem"
  | .defnInfo info => some <| if info.hints.isAbbrev then "abbrev" else "def"
  | .opaqueInfo _ => some "opaque"
  | .axiomInfo _ => some "axiom"
  | _ => none

private def declarationVisibility (declName : Name) : String :=
  if declName.toString.contains "_private" then "private" else "public"

private def collectModuleRows (workspace : String) (modules : Array ModuleEntry) : MetaM (Array Json) := do
  let env ← getEnv
  let mut rows := #[]
  for moduleEntry in modules do
    let moduleName := dottedName moduleEntry.name
    let some modIdx := env.header.moduleNames.idxOf? moduleName | continue
    let moduleData := env.header.moduleData[modIdx]!
    for h : idx in *...moduleData.constNames.size do
      let declName := moduleData.constNames[idx]
      let constInfo := moduleData.constants[idx]!
      let some declKind := declarationKind? constInfo | continue
      let some rangeInfo ← findDeclarationRanges? declName | continue
      let type := constInfo.type
      let typeText := (← ppExpr type).pretty
      let constants := (collectConstants type).toArray.map (·.toString) |>.qsort (· < ·)
      let heads := headConstants type
      let visibility := declarationVisibility declName
      rows := rows.push <|
        jsonLine
          workspace
          moduleEntry.name
          moduleEntry.origin
          s!"{workspace}/{moduleEntry.name.replace "." "/"}.lean"
          declKind
          visibility
          declName
          typeText
          (exprFingerprint type)
          (← permutationFingerprint type)
          (connectiveFingerprint type)
          (connectiveFingerprint (stripForalls type))
          constants
          heads
          (binderCount type)
          rangeInfo.range
  pure rows

private def writeRows (path : System.FilePath) (rows : Array Json) : IO Unit := do
  let text := String.intercalate "\n" (rows.toList.map Json.compress)
  IO.FS.writeFile path (if text.isEmpty then "" else text ++ "\n")

unsafe def run (workspace manifestPath outputPath : String) : IO UInt32 := do
  match ← parseModuleManifest manifestPath with
  | Except.error message =>
      IO.eprintln message
      pure 2
  | Except.ok modules =>
      enableInitializersExecution
      let imports := modules.map fun moduleEntry => Import.mk (dottedName moduleEntry.name) false true false
      let env ← importModules imports Options.empty (loadExts := true)
      let coreCtx : Core.Context := {
        fileName := "<lean-dup-extractor>"
        fileMap := default
        options := Options.empty
      }
      let (rows, _, _) ← MetaM.toIO (collectModuleRows workspace modules) coreCtx { env := env } {} {}
      writeRows outputPath rows
      pure 0

end LeanDup

unsafe def main (args : List String) : IO UInt32 := do
  match args with
  | [workspace, manifestPath, outputPath] => LeanDup.run workspace manifestPath outputPath
  | _ =>
      IO.eprintln "usage: Extractor.lean WORKSPACE MODULE_MANIFEST_JSON OUTPUT_JSONL"
      pure 2
