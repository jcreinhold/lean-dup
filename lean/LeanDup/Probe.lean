import Lean
import LeanDup.Extract
import LeanSemanticSearch.Canonical
import LeanSemanticSearch.LeanCompat

/-!
`LeanDup.Probe` owns bounded semantic checks between candidate declarations.

Fingerprint comparison reuses the shared `lean-semantic-search` canonicalization
(`canonical.expr.v3`); the probe's own structural-specialization shape and the
reducibility checks stay local.

Callers may rely on protocol-level probe result fields and per-pair status.
They must not depend on traversal order, matching heuristics, reducibility
guards, timeout policy, or the shape of Lean expressions used to answer a
probe.
-/
namespace LeanDup.Probe

open Lean
open Lean.Meta
open LeanSemanticSearch

/-- Semantic algorithm marker for Lean-owned probe rows. -/
def version : String := "probe.semantic.v1"

/-- Probe errors are mapped by the worker into protocol error envelopes. -/
inductive ErrorKind where
  | invalidRequest
  | importFailed
  | internalError
  deriving BEq, Repr

/-- A bounded probe failure with optional machine-readable details. -/
structure Error where
  kind : ErrorKind
  message : String
  details : Option Json := none

private inductive Status where
  | ok
  | unavailable
  | invalidPair
  deriving BEq

namespace Status

private def asString : Status → String
  | .ok => "ok"
  | .unavailable => "unavailable"
  | .invalidPair => "invalid_pair"

end Status

private structure Pair where
  pairId : String
  leftId : String
  rightId : String

private structure Binder where
  index : Nat
  fvar : Expr
  type : Expr
  binderInfo : BinderInfo
  isProp : Bool

private structure StatementShape where
  binders : Array Binder
  conclusion : Expr

private structure Result where
  pair : Pair
  status : Status
  sameStatement : Bool := false
  sameUpToSafeReordering : Bool := false
  connectiveEquivalent : Bool := false
  specializesLeftToRight : Bool := false
  specializesRightToLeft : Bool := false
  mutualImplicationShape : Bool := false
  sameReducibleDefinition : Bool := false
  message : Option String := none

private def invalidRequest (message : String) (details : Option Json := none) : Error :=
  { kind := .invalidRequest, message := message, details := details }

private def fromExtractErrorKind : LeanDup.Extract.ErrorKind → ErrorKind
  | .invalidRequest => .invalidRequest
  | .importFailed => .importFailed
  | .internalError => .internalError

private def fromExtractError (err : LeanDup.Extract.Error) : Error :=
  { kind := fromExtractErrorKind err.kind
    message := err.message
    details := err.details }

private def optionalJsonField (json : Json) (key : String) : Option Json :=
  match json.getObjVal? key with
  | .ok value => some value
  | .error _ => none

private def messageLimit : Nat := 180

private def boundedMessage (message : String) : String :=
  if message.length <= messageLimit then
    message
  else
    String.ofList ((message.toList.take (messageLimit - 3)) ++ "...".toList)

private def parsePair (json : Json) : Except Error Pair := do
  match json with
  | Json.obj _ =>
      let pairId ←
        match json.getObjValAs? String "pair_id" with
        | .ok value => pure value
        | .error _ => throw <| invalidRequest "probe pairs need string `pair_id`"
      let leftId ←
        match json.getObjValAs? String "left_declaration_id" with
        | .ok value => pure value
        | .error _ => throw <| invalidRequest "probe pairs need string `left_declaration_id`"
      let rightId ←
        match json.getObjValAs? String "right_declaration_id" with
        | .ok value => pure value
        | .error _ => throw <| invalidRequest "probe pairs need string `right_declaration_id`"
      pure { pairId, leftId, rightId }
  | _ => throw <| invalidRequest "probe pairs must be JSON objects"

private def parsePairs (payload : Json) : Except Error (Array Pair) := do
  match optionalJsonField payload "pairs" with
  | none => throw <| invalidRequest "`pairs` must be an array"
  | some (Json.arr values) =>
      let mut pairs := #[]
      for value in values do
        pairs := pairs.push (← parsePair value)
      pure pairs
  | some _ => throw <| invalidRequest "`pairs` must be an array"

private def parseMaxPairs (payload : Json) : Except Error (Option Nat) := do
  match optionalJsonField payload "max_pairs" with
  | none | some Json.null => pure none
  | some value =>
      match value.getNat? with
      | .ok maxPairs =>
          if maxPairs == 0 then
            throw <| invalidRequest "`max_pairs` must be positive"
          else
            pure (some maxPairs)
      | .error _ => throw <| invalidRequest "`max_pairs` must be a positive integer"

private def parseRequestPairs (payload : Json) : Except Error (Array Pair) := do
  let pairs ← parsePairs payload
  match ← parseMaxPairs payload with
  | some maxPairs =>
      if pairs.size > maxPairs then
        throw <|
          invalidRequest
            "`pairs` exceeds `max_pairs`"
            (some <|
              Json.mkObj
                [ ("pair_count", Json.num pairs.size)
                , ("max_pairs", Json.num maxPairs)
                ])
      pure pairs
  | none => pure pairs

private def declarationMap
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) :
    Std.HashMap String LeanDup.Extract.AcceptedDeclaration := Id.run do
  let mut map : Std.HashMap String LeanDup.Extract.AcceptedDeclaration := {}
  for declaration in declarations do
    map := map.insert declaration.declarationId declaration
  pure map

private def theoremLike : ConstantInfo → Bool
  | .thmInfo _ => true
  | .axiomInfo _ => true
  | _ => false

private def definitionLike : ConstantInfo → Bool
  | .defnInfo _ => true
  | .opaqueInfo _ => true
  | _ => false

private def reducibleValue? : ConstantInfo → Option Expr
  | .defnInfo info => some info.value
  | _ => none

private def binderInfoKey : BinderInfo → String
  | .default => "explicit"
  | .implicit => "implicit"
  | .strictImplicit => "strictImplicit"
  | .instImplicit => "instImplicit"

private def appFnArgs (expr : Expr) : Expr × Array Expr :=
  let rec go (current : Expr) (args : Array Expr) :=
    match current with
    | .app fn arg => go fn (args.push arg)
    | other => (other, args.reverse)
  go expr #[]

private def sortParts (parts : Array String) : String :=
  String.intercalate "," (parts.qsort (· < ·)).toList

private abbrev FVarOrdinals := Std.HashMap FVarId Nat

private partial def levelKey : Level → String
  | .zero => "0"
  | .succ level => s!"s({levelKey level})"
  | .max left right => s!"max({levelKey left},{levelKey right})"
  | .imax left right => s!"imax({levelKey left},{levelKey right})"
  | .param name => s!"p:{name}"
  | .mvar mvarId => s!"m:{mvarId.name}"

private inductive ExprMode where
  | exact
  | connective
  deriving BEq

private partial def exprKey (fvars : FVarOrdinals) (mode : ExprMode) : Expr → String
  | .bvar index => s!"b{index}"
  | .fvar fvarId =>
      match fvars.get? fvarId with
      | some index => s!"v{index}"
      | none => s!"free:{fvarId.name}"
  | .mvar mvarId => s!"mvar:{mvarId.name}"
  | .sort level => s!"(sort {levelKey (Level.normalize level)})"
  | .const name levels =>
      let levelKeys := levels.map fun level => levelKey (Level.normalize level)
      s!"(const {name}[{String.intercalate "," levelKeys}])"
  | expr@(.app ..) =>
      let (head, args) := appFnArgs expr
      if mode == .connective then
        match head, args.toList with
        | .const ``And _, [left, right] =>
            s!"(And {sortParts #[exprKey fvars mode left, exprKey fvars mode right]})"
        | .const ``Or _, [left, right] =>
            s!"(Or {sortParts #[exprKey fvars mode left, exprKey fvars mode right]})"
        | .const ``Iff _, [left, right] =>
            s!"(Iff {sortParts #[exprKey fvars mode left, exprKey fvars mode right]})"
        | .const ``Eq _, [type, left, right] =>
            s!"(Eq {exprKey fvars mode type} {sortParts #[exprKey fvars mode left, exprKey fvars mode right]})"
        | _, _ => appKey head args
      else
        appKey head args
  | .lam _ domain body binderInfo =>
      s!"(lam {binderInfoKey binderInfo} {exprKey fvars mode domain} {exprKey fvars mode body})"
  | .forallE _ domain body binderInfo =>
      s!"(forall {binderInfoKey binderInfo} {exprKey fvars mode domain} {exprKey fvars mode body})"
  | .letE _ type value body _ =>
      s!"(let {exprKey fvars mode type} {exprKey fvars mode value} {exprKey fvars mode body})"
  | .lit (.natVal value) => s!"(nat {value})"
  | .lit (.strVal value) => s!"(str {value.length}:{value})"
  | .mdata _ body => exprKey fvars mode body
  | .proj typeName index body => s!"(proj {typeName}.{index} {exprKey fvars mode body})"
where
  appKey (head : Expr) (args : Array Expr) : String :=
    let parts := args.map (exprKey fvars mode)
    s!"(app {exprKey fvars mode head} [{String.intercalate "," parts.toList}])"

private def collectBinders (fvars : Array Expr) : MetaM (Array Binder) := do
  let mut binders := #[]
  let mut index := 0
  for fvar in fvars do
    let localDecl ← fvar.fvarId!.getDecl
    binders :=
      binders.push
        { index := index
          fvar := fvar
          type := localDecl.type
          binderInfo := localDecl.binderInfo
          isProp := ← Meta.isProp localDecl.type }
    index := index + 1
  pure binders

private def statementShape (constInfo : ConstantInfo) : MetaM StatementShape := do
  forallTelescope constInfo.type fun fvars conclusion => do
    pure { binders := ← collectBinders fvars, conclusion := conclusion }

private def leftOrdinalMap (binders : Array Binder) : FVarOrdinals := Id.run do
  let mut fvars : FVarOrdinals := {}
  for binder in binders do
    fvars := fvars.insert binder.fvar.fvarId! binder.index
  pure fvars

private def rightOrdinalMap
    (rightBinders : Array Binder)
    (mapping : Std.HashMap Nat Nat) : FVarOrdinals := Id.run do
  let mut fvars : FVarOrdinals := {}
  for binder in rightBinders do
    match mapping.get? binder.index with
    | some leftIndex => fvars := fvars.insert binder.fvar.fvarId! leftIndex
    | none => pure ()
  pure fvars

private def sameBinderClass (left right : Binder) : Bool :=
  left.binderInfo == right.binderInfo && left.isProp == right.isProp

private def domainCompatible
    (leftFVars : FVarOrdinals)
    (rightBinders : Array Binder)
    (mapping : Std.HashMap Nat Nat)
    (left right : Binder) : Bool :=
  sameBinderClass left right &&
    exprKey leftFVars .exact left.type ==
      exprKey (rightOrdinalMap rightBinders mapping) .exact right.type

private def chooseLeftBinder
    (left : StatementShape)
    (right : StatementShape)
    (mapping : Std.HashMap Nat Nat)
    (usedLeft : Std.HashSet Nat)
    (rightBinder : Binder) : Option Binder := Id.run do
  let leftFVars := leftOrdinalMap left.binders
  let mut candidates := #[]
  for leftBinder in left.binders do
    if !usedLeft.contains leftBinder.index &&
        domainCompatible leftFVars right.binders mapping leftBinder rightBinder then
      candidates := candidates.push leftBinder
  let sorted :=
    candidates.qsort fun first second =>
      let firstKey := exprKey leftFVars .exact first.type
      let secondKey := exprKey leftFVars .exact second.type
      if firstKey == secondKey then first.index < second.index else firstKey < secondKey
  sorted[0]?

private def matchBinders
    (left right : StatementShape) : Option (Std.HashMap Nat Nat) := Id.run do
  let mut mapping : Std.HashMap Nat Nat := {}
  let mut usedLeft : Std.HashSet Nat := {}
  for rightBinder in right.binders do
    match chooseLeftBinder left right mapping usedLeft rightBinder with
    | some leftBinder =>
        mapping := mapping.insert rightBinder.index leftBinder.index
        usedLeft := usedLeft.insert leftBinder.index
    | none => return none
  pure (some mapping)

private def conclusionCompatible
    (left right : StatementShape)
    (mapping : Std.HashMap Nat Nat) : Bool :=
  let leftKey := exprKey (leftOrdinalMap left.binders) .connective left.conclusion
  let rightKey := exprKey (rightOrdinalMap right.binders mapping) .connective right.conclusion
  leftKey == rightKey

private def structuralSpecializes (left right : StatementShape) : Bool :=
  match matchBinders left right with
  | some mapping => conclusionCompatible left right mapping
  | none => false

private partial def consumeExpr (expr : Expr) (remaining : Nat) : Option Nat :=
  match remaining with
  | 0 => none
  | remaining + 1 =>
      match expr with
      | .app fn arg =>
          match consumeExpr fn remaining with
          | some remaining => consumeExpr arg remaining
          | none => none
      | .lam _ domain body _ | .forallE _ domain body _ =>
          match consumeExpr domain remaining with
          | some remaining => consumeExpr body remaining
          | none => none
      | .letE _ type value body _ =>
          match consumeExpr type remaining with
          | some remaining =>
              match consumeExpr value remaining with
              | some remaining => consumeExpr body remaining
              | none => none
          | none => none
      | .mdata _ body => consumeExpr body remaining
      | .proj _ _ body => consumeExpr body remaining
      | _ => some remaining

private def reducibleExprLimit : Nat := 220

private def withinReducibleGuard (expr : Expr) : Bool :=
  (consumeExpr expr reducibleExprLimit).isSome

private inductive ReducibleCheck where
  | notApplicable
  | checked (same : Bool)
  | unavailable (message : String)

private def sameReducibleDefinition (left right : ConstantInfo) : MetaM ReducibleCheck := do
  if !(definitionLike left && definitionLike right) then
    return .notApplicable
  match reducibleValue? left, reducibleValue? right with
  | some leftValue, some rightValue =>
      if !(withinReducibleGuard leftValue && withinReducibleGuard rightValue) then
        return .unavailable "definition body exceeds reducible probe guard"
      let sameType ← isDefEq left.type right.type
      let sameValue ← withReducible <| isDefEq leftValue rightValue
      pure <| .checked (sameType && sameValue)
  | _, _ =>
      pure <| .unavailable "definition body is opaque or unavailable to the reducible probe"

private def Result.toJson (result : Result) : Json :=
  Json.mkObj
    [ ("pair_id", Json.str result.pair.pairId)
    , ("left_declaration_id", Json.str result.pair.leftId)
    , ("right_declaration_id", Json.str result.pair.rightId)
    , ("status", Json.str result.status.asString)
    , ("same_statement", Json.bool result.sameStatement)
    , ("same_up_to_safe_reordering", Json.bool result.sameUpToSafeReordering)
    , ("connective_equivalent", Json.bool result.connectiveEquivalent)
    , ("specializes_left_to_right", Json.bool result.specializesLeftToRight)
    , ("specializes_right_to_left", Json.bool result.specializesRightToLeft)
    , ("mutual_implication_shape", Json.bool result.mutualImplicationShape)
    , ("same_reducible_definition", Json.bool result.sameReducibleDefinition)
    , ("message", match result.message with
        | some message => Json.str (boundedMessage message)
        | none => Json.null)
    ]

private def invalidPairResult (pair : Pair) (message : String) : Json :=
  ({ pair := pair
     status := .invalidPair
     message := some message } : Result).toJson

private def unavailableResult (pair : Pair) (message : String) : Json :=
  ({ pair := pair
     status := .unavailable
     message := some message } : Result).toJson

private def probePair
    (declarations : Std.HashMap String LeanDup.Extract.AcceptedDeclaration)
    (pair : Pair) : MetaM Json := do
  if pair.pairId.isEmpty || pair.leftId.isEmpty || pair.rightId.isEmpty then
    return invalidPairResult pair "probe pair ids must be nonempty"
  if pair.leftId == pair.rightId then
    return invalidPairResult pair "probe pair must contain two distinct declarations"
  let some left := declarations.get? pair.leftId
    | return unavailableResult pair "left declaration is not available in the imported environment"
  let some right := declarations.get? pair.rightId
    | return unavailableResult pair "right declaration is not available in the imported environment"

  let theoremPair := theoremLike left.constInfo && theoremLike right.constInfo
  let sameStatement ←
    if theoremPair then
      isDefEq left.constInfo.type right.constInfo.type
    else
      pure false
  let leftFingerprints := Canonical.computeFromStatement (← LeanCompat.statementOfConstant left.constInfo)
  let rightFingerprints := Canonical.computeFromStatement (← LeanCompat.statementOfConstant right.constInfo)
  let sameSafe :=
    theoremPair &&
      leftFingerprints.safeBinderPermutation == rightFingerprints.safeBinderPermutation
  let sameConnective :=
    theoremPair &&
      leftFingerprints.connectiveShape == rightFingerprints.connectiveShape
  let leftShape ← statementShape left.constInfo
  let rightShape ← statementShape right.constInfo
  let leftToRight := theoremPair && structuralSpecializes leftShape rightShape
  let rightToLeft := theoremPair && structuralSpecializes rightShape leftShape
  let reducible ← sameReducibleDefinition left.constInfo right.constInfo
  match reducible with
  | .unavailable message =>
      pure
        ({ pair := pair
           status := .unavailable
           message := some message } : Result).toJson
  | .checked sameDefinition =>
      pure
        ({ pair := pair
           status := .ok
           sameReducibleDefinition := sameDefinition } : Result).toJson
  | .notApplicable =>
      if !theoremPair then
        pure <| unavailableResult pair "probe supports theorem/axiom statements and reducible def/abbrev bodies"
      else
        pure
          ({ pair := pair
             status := .ok
             sameStatement := sameStatement
             sameUpToSafeReordering := sameSafe && !sameStatement
             connectiveEquivalent := sameConnective && !sameSafe && !sameStatement
             specializesLeftToRight := leftToRight
             specializesRightToLeft := rightToLeft
             mutualImplicationShape := leftToRight && rightToLeft } : Result).toJson

private def probeRows
    (pairs : Array Pair)
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) : MetaM (Array Json) := do
  let declarationById := declarationMap declarations
  let mut rows := #[]
  for pair in pairs do
    let row ←
      try
        probePair declarationById pair
      catch error =>
        let message ← error.toMessageData.toString
        pure <|
          unavailableResult
            pair
            s!"probe failed for this pair: {message}"
    rows := rows.push row
  pure rows

/--
Import requested modules once and emit probe-result payloads with worker phase
statistics for candidate declaration pairs.

Each pair is isolated: a pair-local failure becomes `status = "unavailable"`;
only malformed requests or import/environment failures abort the command.
-/
unsafe def runProfiled (payload : Json) (modules : Array LeanDup.Extract.ModuleSpec)
    (initializeSearchPath : Bool := true) :
    IO (Except Error LeanDup.Extract.RunOutput) := do
  match parseRequestPairs payload with
  | Except.error err => pure <| Except.error err
  | Except.ok pairs =>
      let result ←
        LeanDup.Extract.withAcceptedDeclarationsProfiled payload modules
          (fun _options declarations => do
            probeRows pairs declarations)
          (initializeSearchPath := initializeSearchPath)
      match result with
      | Except.error err => pure <| Except.error (fromExtractError err)
      | Except.ok (rows, stats) =>
          pure <| Except.ok { rows := rows, stats := { stats with rowCount := rows.size } }

/--
Import requested modules once and emit probe-result payloads for candidate
declaration pairs.
-/
unsafe def run (payload : Json) (modules : Array LeanDup.Extract.ModuleSpec) :
    IO (Except Error (Array Json)) := do
  match ← runProfiled payload modules with
  | Except.error err => pure <| Except.error err
  | Except.ok output => pure <| Except.ok output.rows

end LeanDup.Probe
