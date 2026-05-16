import Lean

/-!
`LeanDup.Canonical` owns Lean-side semantic fingerprint decisions.

Callers may compare emitted keys for equality under `version`. They must not
depend on expression traversal order, binder representation, universe encoding,
connective-normalization rules, or the private key format.
-/
namespace LeanDup.Canonical

open Lean
open Lean.Meta

/-- Semantic algorithm marker for canonical declaration fingerprints. -/
def version : String := "canonical.expr.v1"

/--
Opaque semantic keys for one declaration statement.

The fields are candidate-generation keys, not proof certificates. Equality of
keys is meaningful only under the matching `version`.
-/
structure Fingerprints where
  statement : String
  safeBinderPermutation : String
  connectiveShape : String
  conclusionShape : String
  binderCount : Nat

private def hashMod : Nat := 18446744073709551557

private def hashSeed : Nat := 1469598103934665603

private def stableHash (text : String) : String :=
  toString <|
    text.foldl
      (fun acc char => (acc * 131 + char.toNat + 17) % hashMod)
      hashSeed

private def fingerprintKey (kind body : String) : String :=
  s!"{version}:{kind}:{stableHash body}"

private structure LevelContext where
  params : Std.HashMap Name Nat

private abbrev FVarOrdinals := Std.HashMap FVarId Nat

private structure SerializerContext where
  levels : LevelContext
  fvars : FVarOrdinals

private inductive ExprMode where
  | exact
  | connective
  deriving BEq

private structure Binder where
  index : Nat
  fvar : Expr
  type : Expr
  binderInfo : BinderInfo
  deps : Array Nat

private def levelContext (params : List Name) : LevelContext := Id.run do
  let mut map : Std.HashMap Name Nat := {}
  let mut index := 0
  for param in params do
    map := map.insert param index
    index := index + 1
  pure { params := map }

private partial def levelKey (ctx : LevelContext) : Level → String
  | .zero => "0"
  | .succ level => s!"s({levelKey ctx level})"
  | .max left right => s!"max({levelKey ctx left},{levelKey ctx right})"
  | .imax left right => s!"imax({levelKey ctx left},{levelKey ctx right})"
  | .param name =>
      match ctx.params.get? name with
      | some index => s!"p{index}"
      | none => s!"p:{name}"
  | .mvar mvarId => s!"m:{mvarId.name}"

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

private def fvarKey (ctx : SerializerContext) (fvarId : FVarId) : String :=
  match ctx.fvars.get? fvarId with
  | some index => s!"v{index}"
  | none => s!"free:{fvarId.name}"

private partial def exprKey (ctx : SerializerContext) (mode : ExprMode) : Expr → String
  | .bvar index => s!"b{index}"
  | .fvar fvarId => fvarKey ctx fvarId
  | .mvar mvarId => s!"mvar:{mvarId.name}"
  | .sort level => s!"(sort {levelKey ctx.levels (Level.normalize level)})"
  | .const name levels =>
      let levelKeys := levels.map fun level => levelKey ctx.levels (Level.normalize level)
      s!"(const {name}[{String.intercalate "," levelKeys}])"
  | expr@(.app ..) =>
      let (head, args) := appFnArgs expr
      if mode == .connective then
        match head, args.toList with
        | .const ``And _, [left, right] =>
            s!"(And {sortParts #[exprKey ctx mode left, exprKey ctx mode right]})"
        | .const ``Or _, [left, right] =>
            s!"(Or {sortParts #[exprKey ctx mode left, exprKey ctx mode right]})"
        | .const ``Iff _, [left, right] =>
            s!"(Iff {sortParts #[exprKey ctx mode left, exprKey ctx mode right]})"
        | .const ``Eq _, [type, left, right] =>
            s!"(Eq {exprKey ctx mode type} {sortParts #[exprKey ctx mode left, exprKey ctx mode right]})"
        | _, _ => appKey ctx mode head args
      else
        appKey ctx mode head args
  | .lam _ domain body binderInfo =>
      s!"(lam {binderInfoKey binderInfo} {exprKey ctx mode domain} {exprKey ctx mode body})"
  | .forallE _ domain body binderInfo =>
      s!"(forall {binderInfoKey binderInfo} {exprKey ctx mode domain} {exprKey ctx mode body})"
  | .letE _ type value body _ =>
      s!"(let {exprKey ctx mode type} {exprKey ctx mode value} {exprKey ctx mode body})"
  | .lit (.natVal value) => s!"(nat {value})"
  | .lit (.strVal value) => s!"(str {value.length}:{value})"
  | .mdata _ body => exprKey ctx mode body
  | .proj typeName index body => s!"(proj {typeName}.{index} {exprKey ctx mode body})"
where
  appKey (ctx : SerializerContext) (mode : ExprMode) (head : Expr) (args : Array Expr) : String :=
    let parts := args.map (exprKey ctx mode)
    s!"(app {exprKey ctx mode head} [{String.intercalate "," parts.toList}])"

private def dependencies (type : Expr) (fvars : Array Expr) : Array Nat := Id.run do
  let used := (collectFVars {} type).fvarSet
  let mut deps := #[]
  let mut index := 0
  for fvar in fvars do
    if used.contains fvar.fvarId! then
      deps := deps.push index
    index := index + 1
  pure deps

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
          deps := dependencies localDecl.type fvars }
    index := index + 1
  pure binders

private def bindFVar (ctx : SerializerContext) (binder : Binder) (ordinal : Nat) :
    SerializerContext :=
  { ctx with fvars := ctx.fvars.insert binder.fvar.fvarId! ordinal }

private def allDepsScheduled (scheduled : Std.HashSet Nat) (deps : Array Nat) : Bool :=
  deps.all fun dep => scheduled.contains dep

private def binderSortKey (ctx : SerializerContext) (binder : Binder) : String :=
  s!"{binderInfoKey binder.binderInfo}:{exprKey ctx .exact binder.type}"

private partial def scheduleBinders
    (baseCtx : SerializerContext)
    (binders : Array Binder) : Array Binder := Id.run do
  let mut result := #[]
  let mut scheduled : Std.HashSet Nat := {}
  let mut ctx := baseCtx
  while result.size < binders.size do
    let mut ready := #[]
    for binder in binders do
      if !scheduled.contains binder.index && allDepsScheduled scheduled binder.deps then
        ready := ready.push binder
    if ready.isEmpty then
      for binder in binders do
        if !scheduled.contains binder.index then
          ready := ready.push binder
    let sortedReady :=
      ready.qsort fun left right =>
        let leftKey := binderSortKey ctx left
        let rightKey := binderSortKey ctx right
        if leftKey == rightKey then left.index < right.index else leftKey < rightKey
    match sortedReady[0]? with
    | some next =>
        let ordinal := result.size
        result := result.push next
        scheduled := scheduled.insert next.index
        ctx := bindFVar ctx next ordinal
    | none =>
        return result
  pure result

private def bindersContext (baseCtx : SerializerContext) (binders : Array Binder) :
    SerializerContext := Id.run do
  let mut ctx := baseCtx
  let mut ordinal := 0
  for binder in binders do
    ctx := bindFVar ctx binder ordinal
    ordinal := ordinal + 1
  pure ctx

private def statementBody
    (baseCtx : SerializerContext)
    (binders : Array Binder)
    (conclusion : Expr)
    (mode : ExprMode) : String := Id.run do
  let mut ctx := baseCtx
  let mut ordinal := 0
  let mut binderKeys := #[]
  for binder in binders do
    let domainKey := exprKey ctx mode binder.type
    binderKeys := binderKeys.push s!"({binderInfoKey binder.binderInfo} {domainKey})"
    ctx := bindFVar ctx binder ordinal
    ordinal := ordinal + 1
  pure s!"(forall [{String.intercalate "," binderKeys.toList}] {exprKey ctx mode conclusion})"

/--
Compute all canonical statement fingerprints for one Lean declaration.

The returned keys are opaque. Callers may store and compare them, but all
statement traversal, binder dependency, universe, and connective policy remains
owned by this module.
-/
def compute (constInfo : ConstantInfo) : MetaM Fingerprints := do
  let baseCtx : SerializerContext :=
    { levels := levelContext constInfo.levelParams
      fvars := {} }
  forallTelescope constInfo.type fun fvars conclusion => do
    let binders ← collectBinders fvars
    let scheduled := scheduleBinders baseCtx binders
    let conclusionCtx := bindersContext baseCtx binders
    let statement := statementBody baseCtx binders conclusion .exact
    let safeBinderPermutation := statementBody baseCtx scheduled conclusion .exact
    let connectiveShape := statementBody baseCtx binders conclusion .connective
    let conclusionShape := exprKey conclusionCtx .connective conclusion
    pure
      { statement := fingerprintKey "statement" statement
        safeBinderPermutation := fingerprintKey "safe_binder_permutation" safeBinderPermutation
        connectiveShape := fingerprintKey "connective_shape" connectiveShape
        conclusionShape := fingerprintKey "conclusion_shape" conclusionShape
        binderCount := binders.size }

end LeanDup.Canonical
