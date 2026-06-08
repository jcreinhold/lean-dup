import Lean
import LeanDup.Extract
import LeanSemanticSearch.Canonical
import LeanSemanticSearch.RoleFeatures
import LeanSemanticSearch.LeanCompat

/-!
`LeanDup.Features` owns worker feature rows computed by Lean.

The neutral semantic computation — canonical fingerprints and role features — is
shared: it comes from the `lean-semantic-search` package (`canonical.expr.v3`,
`features.roles.v3`). This module owns only the `lean-dup.worker.v1` row payload
and the subprocess command integration; it does not reimplement canonicalization.

Callers may store and compare feature keys as opaque semantic facts. They must
not reconstruct features from pretty text, source snippets, declaration names,
Rust ranking policy, or index storage rows.
-/
namespace LeanDup.Features

open Lean
open Lean.Meta
open LeanSemanticSearch

/-- Semantic algorithm marker for Lean-owned feature rows. Matches the shared
    package's `features.roles.v3`; carried explicitly because it is part of the
    `lean-dup.worker.v1` wire contract. -/
def version : String := "features.roles.v3"

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

/-- Compute the shared neutral semantic facts for one declaration: translate its
    signature into the package-owned `StatementShape`, then derive canonical
    fingerprints and role features. Identical keys to the former local
    implementation — same `canonical.expr.v3` / `features.roles.v3` algorithms,
    now sourced from `lean-semantic-search`. -/
private def semanticFacts
    (declaration : LeanDup.Extract.AcceptedDeclaration) :
    MetaM (Canonical.Fingerprints × Array RoleFeatures.RoleFeature × Array String) := do
  let statement ← LeanCompat.statementOfConstant declaration.constInfo
  let fingerprints := Canonical.computeFromStatement statement
  let (roleFeatures, markers) := RoleFeatures.factsFromStatement statement
  pure (fingerprints, roleFeatures, markers)

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

/--
Encode one accepted declaration's semantic features as a protocol payload.

The opaque-key encoders (`Fingerprints.toJson`, `RoleFeatures.featuresJson`,
`RoleFeatures.markersJson`) are shared and byte-identical to the former local
ones; this row keeps the `lean-dup.worker.v1` field set (no `source` field).

Rust may store and compare the returned opaque keys but must not infer Lean
expression structure from them.
-/
def rowPayload
    (declaration : LeanDup.Extract.AcceptedDeclaration)
    (fingerprints : Canonical.Fingerprints)
    (roleFeatures : Array RoleFeatures.RoleFeature)
    (markers : Array String) : Json :=
  Json.mkObj
    [ ("declaration_id", Json.str declaration.declarationId)
    , ("feature_version", Json.str version)
    , ("fingerprints", fingerprints.toJson)
    , ("role_features", RoleFeatures.featuresJson roleFeatures)
    , ("binder_count", Json.num fingerprints.binderCount)
    , ("low_signal_markers", RoleFeatures.markersJson markers)
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
    let (fingerprints, roleFeatures, markers) ← semanticFacts declaration
    rows := rows.push (rowPayload declaration fingerprints roleFeatures markers)
  pure rows

/--
Import requested modules once and emit feature-row payloads for declarations
accepted by the request filters.

The emitted fingerprints and role features are Lean-owned opaque keys. Rust may
compare and weight them, but it must not reconstruct them from display or source
facts.
-/
unsafe def runProfiled (payload : Json) (modules : Array LeanDup.Extract.ModuleSpec)
    (initializeSearchPath : Bool := true) :
    IO (Except Error LeanDup.Extract.RunOutput) := do
  match parseDeclarationIds payload with
  | Except.error err => pure <| Except.error err
  | Except.ok ids? =>
      let result ←
        LeanDup.Extract.withAcceptedDeclarationsProfiled payload modules
          (fun _options declarations => do
            match selectDeclarations ids? declarations with
            | Except.error err => pure <| Except.error err
            | Except.ok selected => do
                let rows ← featureRows selected
                pure <| Except.ok rows)
          (initializeSearchPath := initializeSearchPath)
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
