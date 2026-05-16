import Lean

open Lean
open Lean.Meta

namespace LeanDup.SemanticProbe

private def dottedName (text : String) : Name :=
  (text.splitOn ".").foldl
    (init := Name.anonymous)
    fun current segment =>
      if segment.isEmpty then
        current
      else if segment.all Char.isDigit then
        Name.num current segment.toNat!
      else
        Name.str current segment

private structure Pair where
  firstKey : String
  secondKey : String
  first : String
  second : String
  firstKind : String
  secondKind : String
  firstPermutationFingerprint : String
  secondPermutationFingerprint : String
  firstConnectiveFingerprint : String
  secondConnectiveFingerprint : String
  firstConclusionFingerprint : String
  secondConclusionFingerprint : String

private structure ProbeManifest where
  modules : Array String
  pairs : Array Pair

private def jsonString! (json : Json) (key : String) : Except String String :=
  match json.getObjValAs? String key with
  | Except.ok value => Except.ok value
  | Except.error error => Except.error s!"missing string `{key}`: {error}"

private def parsePair (json : Json) : Except String Pair := do
  pure {
    firstKey := ← jsonString! json "first_key"
    secondKey := ← jsonString! json "second_key"
    first := ← jsonString! json "first"
    second := ← jsonString! json "second"
    firstKind := ← jsonString! json "first_kind"
    secondKind := ← jsonString! json "second_kind"
    firstPermutationFingerprint := ← jsonString! json "first_permutation_fingerprint"
    secondPermutationFingerprint := ← jsonString! json "second_permutation_fingerprint"
    firstConnectiveFingerprint := ← jsonString! json "first_connective_fingerprint"
    secondConnectiveFingerprint := ← jsonString! json "second_connective_fingerprint"
    firstConclusionFingerprint := ← jsonString! json "first_conclusion_fingerprint"
    secondConclusionFingerprint := ← jsonString! json "second_conclusion_fingerprint"
  }

private def parseManifest (path : System.FilePath) : IO (Except String ProbeManifest) := do
  let text ← IO.FS.readFile path
  match Json.parse text with
  | Except.error error => pure <| Except.error s!"could not parse probe manifest: {error}"
  | Except.ok json =>
      match json.getObjVal? "modules", json.getObjVal? "pairs" with
      | Except.ok (Json.arr moduleRows), Except.ok (Json.arr pairRows) =>
          let mut modules := #[]
          for row in moduleRows do
            match row with
            | Json.str moduleName => modules := modules.push moduleName
            | _ => return Except.error "probe manifest `modules` must be an array of strings"
          let mut pairs := #[]
          for row in pairRows do
            match parsePair row with
            | Except.ok pair => pairs := pairs.push pair
            | Except.error error => return Except.error error
          pure <| Except.ok { modules, pairs }
      | _, _ => pure <| Except.error "probe manifest needs array fields `modules` and `pairs`"

private def stripForalls : Expr → Expr
  | .forallE _ _ body _ => stripForalls body
  | expr => expr

private def collectForalls (expr : Expr) : Array Expr × Expr :=
  let rec go (current : Expr) (domains : Array Expr) :=
    match current with
    | .forallE _ domain body _ => go body (domains.push domain)
    | other => (domains, other)
  go expr #[]

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

private def eraseOne (needle : String) : List String → Option (List String)
  | [] => none
  | head :: tail =>
      if head == needle then
        some tail
      else
        eraseOne needle tail |>.map (head :: ·)

private def containsMultiset (haystack needles : List String) : Bool :=
  match needles with
  | [] => true
  | needle :: rest =>
      match eraseOne needle haystack with
      | none => false
      | some haystack' => containsMultiset haystack' rest

private def structuralSpecializes (source target : Expr) : Bool :=
  let (sourceDomains, sourceConclusion) := collectForalls source
  let (targetDomains, targetConclusion) := collectForalls target
  connectiveFingerprint sourceConclusion == connectiveFingerprint targetConclusion
    && containsMultiset
      (sourceDomains.map connectiveFingerprint |>.toList)
      (targetDomains.map connectiveFingerprint |>.toList)
    && sourceDomains.size >= targetDomains.size

private def constantValue? : ConstantInfo → Option Expr
  | .defnInfo info => some info.value
  | .opaqueInfo info => some info.value
  | _ => none

private def sameReducibleDef (first second : ConstantInfo) : MetaM Bool := do
  match constantValue? first, constantValue? second with
  | some firstValue, some secondValue =>
      withReducible <| isDefEq (← whnf firstValue) (← whnf secondValue)
  | _, _ => pure false

private def theoremLike (kind : String) : Bool :=
  kind == "theorem" || kind == "axiom"

private def probePair (pair : Pair) : MetaM Json := do
  let env ← getEnv
  let firstName := dottedName pair.first
  let secondName := dottedName pair.second
  match env.find? firstName, env.find? secondName with
  | some firstInfo, some secondInfo =>
      let sameType ← isDefEq firstInfo.type secondInfo.type
      let sameStatement := theoremLike pair.firstKind && theoremLike pair.secondKind && sameType
      let leftToRight :=
        theoremLike pair.firstKind && theoremLike pair.secondKind
          && structuralSpecializes firstInfo.type secondInfo.type
      let rightToLeft :=
        theoremLike pair.firstKind && theoremLike pair.secondKind
          && structuralSpecializes secondInfo.type firstInfo.type
      let sameReducible ←
        if pair.firstKind == "def" && pair.secondKind == "def"
          || pair.firstKind == "abbrev" && pair.secondKind == "abbrev" then
          sameReducibleDef firstInfo secondInfo
        else
          pure false
      pure <| Json.mkObj
        [ ("first_key", Json.str pair.firstKey)
        , ("second_key", Json.str pair.secondKey)
        , ("first", Json.str pair.first)
        , ("second", Json.str pair.second)
        , ("same_statement", Json.bool sameStatement)
        , ("same_up_to_reordering", Json.bool
            (pair.firstPermutationFingerprint == pair.secondPermutationFingerprint && !sameStatement))
        , ("connective_equivalent", Json.bool
            (pair.firstConnectiveFingerprint == pair.secondConnectiveFingerprint
              && pair.firstPermutationFingerprint != pair.secondPermutationFingerprint))
        , ("specializes", Json.bool (leftToRight || rightToLeft))
        , ("specializes_left_to_right", Json.bool leftToRight)
        , ("specializes_right_to_left", Json.bool rightToLeft)
        , ("mutual_implication_shape", Json.bool
            (pair.firstConclusionFingerprint == pair.secondConclusionFingerprint && (leftToRight || rightToLeft)))
        , ("same_reducible_def", Json.bool sameReducible)
        , ("unavailable", Json.bool false)
        , ("source", Json.str "lean")
        , ("message", Json.null)
        ]
  | _, _ =>
      pure <| Json.mkObj
        [ ("first_key", Json.str pair.firstKey)
        , ("second_key", Json.str pair.secondKey)
        , ("first", Json.str pair.first)
        , ("second", Json.str pair.second)
        , ("same_statement", Json.bool false)
        , ("same_up_to_reordering", Json.bool false)
        , ("connective_equivalent", Json.bool false)
        , ("specializes", Json.bool false)
        , ("specializes_left_to_right", Json.bool false)
        , ("specializes_right_to_left", Json.bool false)
        , ("mutual_implication_shape", Json.bool false)
        , ("same_reducible_def", Json.bool false)
        , ("unavailable", Json.bool true)
        , ("source", Json.str "lean")
        , ("message", Json.str "one or both declarations are not available in the imported environment")
        ]

private def writeRows (env : Environment) (outputPath : String) (pairs : Array Pair) : IO Unit := do
  let coreCtx : Core.Context := {
    fileName := "<lean-dup-semantic-probe>"
    fileMap := default
    options := Options.empty
  }
  IO.FS.withFile outputPath IO.FS.Mode.write fun handle => do
    for pair in pairs do
      let (row, _, _) ←
        MetaM.toIO
          (probePair pair)
          coreCtx
          { env := env }
          {}
          {}
      handle.putStrLn row.compress

unsafe def run (manifestPath outputPath : String) : IO UInt32 := do
  match ← parseManifest manifestPath with
  | Except.error message =>
      IO.eprintln message
      pure 2
  | Except.ok manifest =>
      enableInitializersExecution
      let imports := manifest.modules.map fun moduleName => Import.mk (dottedName moduleName) false true false
      IO.eprintln s!"lean-dup: semantic probe importing {manifest.modules.size} module(s)"
      let env ← importModules imports Options.empty (loadExts := true)
      IO.eprintln s!"lean-dup: semantic probe checking {manifest.pairs.size} pair(s)"
      writeRows env outputPath manifest.pairs
      pure 0

end LeanDup.SemanticProbe

unsafe def main (args : List String) : IO UInt32 := do
  match args with
  | [manifestPath, outputPath] => LeanDup.SemanticProbe.run manifestPath outputPath
  | _ =>
      IO.eprintln "usage: SemanticProbe.lean MANIFEST_JSON OUTPUT_JSONL"
      pure 2
