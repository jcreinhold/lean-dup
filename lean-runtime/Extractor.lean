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

private def parseModuleManifest (path : System.FilePath) : IO (Except String (Array String)) := do
  let text ← IO.FS.readFile path
  match Json.parse text with
  | Except.error err => pure <| Except.error s!"could not parse module manifest: {err}"
  | Except.ok (Json.arr rows) =>
      let mut modules := #[]
      for row in rows do
        match row with
        | Json.str moduleName => modules := modules.push moduleName
        | _ => return Except.error "module manifest must be a JSON array of strings"
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

private def pointJson (line column : Nat) : Json :=
  Json.mkObj [("line", Json.num line), ("column", Json.num column)]

private def spanJson (range : DeclarationRange) : Json :=
  Json.mkObj
    [ ("start", pointJson range.pos.line range.pos.column)
    , ("end", pointJson range.endPos.line range.endPos.column)
    ]

private def jsonLine
    (workspace moduleName filePath declKind : String)
    (declName : Name)
    (typeText normalized conclusion : String)
    (constants heads : Array String)
    (binders : Nat)
    (range : DeclarationRange) : Json :=
  Json.mkObj
    [ ("schema_version", Json.str schemaVersion)
    , ("workspace", Json.str workspace)
    , ("module", Json.str moduleName)
    , ("file", Json.str filePath)
    , ("name", Json.str declName.toString)
    , ("short_name", Json.str declName.getString!)
    , ("kind", Json.str declKind)
    , ("span", spanJson range)
    , ("type_text", Json.str typeText)
    , ("normalized_type", Json.str normalized)
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

private def collectModuleRows (workspace : String) (moduleNames : Array String) : MetaM (Array Json) := do
  let env ← getEnv
  let moduleSet := moduleNames.foldl (init := Std.HashSet.emptyWithCapacity) fun set name => set.insert name
  let mut rows := #[]
  for moduleNameText in moduleNames do
    let moduleName := dottedName moduleNameText
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
      rows := rows.push <|
        jsonLine
          workspace
          moduleNameText
          s!"{workspace}/{moduleNameText.replace "." "/"}.lean"
          declKind
          declName
          typeText
          (exprFingerprint type)
          (exprFingerprint (stripForalls type))
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
      let imports := modules.map fun moduleName => Import.mk (dottedName moduleName) false true false
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
