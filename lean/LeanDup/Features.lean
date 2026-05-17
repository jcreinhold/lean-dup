import Lean
import LeanDup.Canonical
import LeanDup.Extract

/-!
`LeanDup.Features` owns worker feature rows computed by Lean.

Callers may store and compare feature keys as opaque semantic facts. They must
not reconstruct features from pretty text, source snippets, declaration names,
Rust ranking policy, or index storage rows.
-/
namespace LeanDup.Features

open Lean
open Lean.Meta

/-- Semantic algorithm marker for Lean-owned feature rows. -/
def version : String := "features.roles.v2"

/-- Feature errors are mapped by the worker into protocol error envelopes. -/
inductive ErrorKind where
  | invalidRequest
  | importFailed
  | internalError
  deriving BEq, Repr

/-- A bounded feature failure with optional machine-readable details. -/
structure Error where
  kind : ErrorKind
  message : String
  details : Option Json := none

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

private def stringArrayJson (values : Array String) : Json :=
  Json.arr (values.map Json.str)

private def hashMod : Nat := 18446744073709551557

private def hashSeed : Nat := 1469598103934665603

private def roleKeyVersion : String := "features.role_key.v1"

private def stableHash (text : String) : String :=
  toString <|
    text.foldl
      (fun acc char => (acc * 131 + char.toNat + 17) % hashMod)
      hashSeed

private inductive Role where
  | conclusionConst
  | conclusionHead
  | hypothesisConst
  | hypothesisHead
  | binderDomainHead
  deriving BEq

namespace Role

private def asString : Role → String
  | .conclusionConst => "conclusion_const"
  | .conclusionHead => "conclusion_head"
  | .hypothesisConst => "hypothesis_const"
  | .hypothesisHead => "hypothesis_head"
  | .binderDomainHead => "binder_domain_head"

end Role

private structure RoleFeature where
  role : Role
  name : Name

private def RoleFeature.sortKey (feature : RoleFeature) : String :=
  s!"{feature.role.asString}:{feature.name}"

private def roleKey (feature : RoleFeature) : String :=
  let text := feature.sortKey
  s!"{roleKeyVersion}:{stableHash text}"

private def RoleFeature.toJson (feature : RoleFeature) : Json :=
  Json.mkObj
    [ ("role", Json.str feature.role.asString)
    , ("key", Json.str (roleKey feature))
    , ("display", Json.str feature.name.toString)
    ]

private def containsFeature (features : Array RoleFeature) (feature : RoleFeature) : Bool :=
  features.any fun existing =>
    existing.role == feature.role && existing.name == feature.name

private def pushFeature (features : Array RoleFeature) (feature : RoleFeature) :
    Array RoleFeature :=
  if containsFeature features feature then features else features.push feature

private def sortedFeatures (features : Array RoleFeature) : Array RoleFeature :=
  features.qsort fun left right => left.sortKey < right.sortKey

private def featuresJson (features : Array RoleFeature) : Json :=
  Json.arr (sortedFeatures features |>.map RoleFeature.toJson)

private def broadHeadNames : Std.HashSet String :=
  [ "Eq"
  , "Iff"
  , "Exists"
  , "Nonempty"
  , "False"
  , "True"
  , "Ne"
  , "Not"
  , "And"
  , "Or"
  , "LE.le"
  , "LT.lt"
  , "Membership.mem"
  , "HasSubset.Subset"
  ].foldl (fun set name => set.insert name) {}

private def isBroadHead (name : Name) : Bool :=
  broadHeadNames.contains name.toString

private partial def appHead (expr : Expr) : Expr :=
  match expr with
  | .app fn _ => appHead fn
  | .mdata _ body => appHead body
  | other => other

private def headName? (expr : Expr) : Option Name :=
  match appHead expr with
  | .const name _ => some name
  | _ => none

private def sortedNamesFromSet (names : NameSet) : Array Name :=
  names.toArray.qsort fun left right => left.toString < right.toString

private def addConstants
    (role : Role)
    (expr : Expr)
    (features : Array RoleFeature) : Array RoleFeature := Id.run do
  let mut result := features
  for name in sortedNamesFromSet expr.getUsedConstantsAsSet do
    result := pushFeature result { role := role, name := name }
  pure result

private def addHead
    (role : Role)
    (expr : Expr)
    (features : Array RoleFeature) : Array RoleFeature :=
  match headName? expr with
  | some name => pushFeature features { role := role, name := name }
  | none => features

private def lowSignalMarkers (features : Array RoleFeature) : Array String := Id.run do
  let mut markers := #[]
  for feature in features do
    match feature.role with
    | .conclusionHead | .hypothesisHead | .binderDomainHead =>
        if isBroadHead feature.name then
          let marker := s!"broad_head:{feature.name}"
          if !markers.contains marker then
            markers := markers.push marker
    | .conclusionConst | .hypothesisConst => pure ()
  pure <| markers.qsort (· < ·)

private def markersJson (markers : Array String) : Json :=
  Json.arr (markers.map Json.str)

private def roleFacts (constInfo : ConstantInfo) : MetaM (Array RoleFeature × Array String) := do
  forallTelescope constInfo.type fun fvars conclusion => do
    let mut features := #[]
    features := addConstants .conclusionConst conclusion features
    features := addHead .conclusionHead conclusion features
    for fvar in fvars do
      let localDecl ← fvar.fvarId!.getDecl
      if ← Meta.isProp localDecl.type then
        features := addConstants .hypothesisConst localDecl.type features
        features := addHead .hypothesisHead localDecl.type features
      else
        features := addHead .binderDomainHead localDecl.type features
    pure (sortedFeatures features, lowSignalMarkers features)

private def parseDeclarationIds (payload : Json) : Except Error (Option (Array String)) := do
  match optionalJsonField payload "declaration_ids" with
  | none | some Json.null => pure none
  | some (Json.arr values) =>
      let mut ids := #[]
      for value in values do
        match value with
        | Json.str id =>
            if id.isEmpty then
              throw <| invalidRequest "`declaration_ids` must not contain empty strings"
            ids := ids.push id
        | _ =>
            throw <| invalidRequest "`declaration_ids` must contain only strings"
      pure (some ids)
  | some _ => throw <| invalidRequest "`declaration_ids` must be an array"

private def selectDeclarations
    (ids? : Option (Array String))
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) :
    Except Error (Array LeanDup.Extract.AcceptedDeclaration) := do
  match ids? with
  | none => pure declarations
  | some ids =>
      let mut selected := #[]
      let mut missing := #[]
      for id in ids do
        match declarations.find? fun declaration => declaration.declarationId == id with
        | some declaration => selected := selected.push declaration
        | none => missing := missing.push id
      if !missing.isEmpty then
        throw <|
          invalidRequest
            "unknown declaration id requested by `features`"
            (some <| Json.mkObj [("declaration_ids", stringArrayJson missing)])
      pure selected

private def fingerprintsJson (fingerprints : LeanDup.Canonical.Fingerprints) : Json :=
  Json.mkObj
    [ ("statement", Json.str fingerprints.statement)
    , ("safe_binder_permutation", Json.str fingerprints.safeBinderPermutation)
    , ("connective_shape", Json.str fingerprints.connectiveShape)
    , ("conclusion_shape", Json.str fingerprints.conclusionShape)
    ]

/--
Encode one accepted declaration's semantic features as a protocol payload.

Rust may store and compare the returned opaque keys but must not infer Lean
expression structure from them.
-/
def rowPayload
    (declaration : LeanDup.Extract.AcceptedDeclaration)
    (fingerprints : LeanDup.Canonical.Fingerprints)
    (roleFeatures : Array RoleFeature)
    (markers : Array String) : Json :=
  Json.mkObj
    [ ("declaration_id", Json.str declaration.declarationId)
    , ("feature_version", Json.str version)
    , ("fingerprints", fingerprintsJson fingerprints)
    , ("role_features", featuresJson roleFeatures)
    , ("binder_count", Json.num fingerprints.binderCount)
    , ("low_signal_markers", markersJson markers)
    ]

/--
Compute semantic feature rows for accepted declarations in the current Lean
environment.

The caller controls chunking. This function owns the semantic row contents.
-/
def featureRows
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) : MetaM (Array Json) := do
  let mut rows := #[]
  for declaration in declarations do
    let fingerprints ← LeanDup.Canonical.compute declaration.constInfo
    let (roleFeatures, markers) ← roleFacts declaration.constInfo
    rows := rows.push (rowPayload declaration fingerprints roleFeatures markers)
  pure rows

/--
Import requested modules once and emit feature-row payloads for declarations
accepted by the request filters.

The emitted fingerprints and role features are Lean-owned opaque keys. Rust may
compare and weight them, but it must not reconstruct them from display or source
facts.
-/
unsafe def runProfiled (payload : Json) (modules : Array LeanDup.Extract.ModuleSpec) :
    IO (Except Error LeanDup.Extract.RunOutput) := do
  match parseDeclarationIds payload with
  | Except.error err => pure <| Except.error err
  | Except.ok ids? =>
      let result ←
        LeanDup.Extract.withAcceptedDeclarationsProfiled payload modules fun _options declarations => do
          match selectDeclarations ids? declarations with
          | Except.error err => pure <| Except.error err
          | Except.ok selected => do
              let rows ← featureRows selected
              pure <| Except.ok rows
      match result with
      | Except.error err => pure <| Except.error (fromExtractError err)
      | Except.ok (Except.error err, _stats) => pure <| Except.error err
      | Except.ok (Except.ok rows, stats) =>
          pure <| Except.ok { rows := rows, stats := { stats with rowCount := rows.size } }

unsafe def run (payload : Json) (modules : Array LeanDup.Extract.ModuleSpec) :
    IO (Except Error (Array Json)) := do
  match ← runProfiled payload modules with
  | .error err => pure <| .error err
  | .ok output => pure <| .ok output.rows

end LeanDup.Features
