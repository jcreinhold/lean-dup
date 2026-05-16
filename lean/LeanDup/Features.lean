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
def version : String := "features.canonical.v1"

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

private def rowPayload
    (declaration : LeanDup.Extract.AcceptedDeclaration)
    (fingerprints : LeanDup.Canonical.Fingerprints) : Json :=
  Json.mkObj
    [ ("declaration_id", Json.str declaration.declarationId)
    , ("feature_version", Json.str version)
    , ("fingerprints", fingerprintsJson fingerprints)
    , ("role_features", Json.arr (#[] : Array Json))
    , ("binder_count", Json.num fingerprints.binderCount)
    , ("low_signal_markers", Json.arr (#[] : Array Json))
    ]

private def featureRows
    (declarations : Array LeanDup.Extract.AcceptedDeclaration) : MetaM (Array Json) := do
  let mut rows := #[]
  for declaration in declarations do
    let fingerprints ← LeanDup.Canonical.compute declaration.constInfo
    rows := rows.push (rowPayload declaration fingerprints)
  pure rows

/--
Import requested modules once and emit feature-row payloads for declarations
accepted by the request filters.

The emitted fingerprints are Lean-owned opaque keys. This command does not emit
role-aware retrieval features yet; prompt 06 owns those.
-/
unsafe def run (payload : Json) (modules : Array LeanDup.Extract.ModuleSpec) :
    IO (Except Error (Array Json)) := do
  match parseDeclarationIds payload with
  | Except.error err => pure <| Except.error err
  | Except.ok ids? =>
      let result ←
        LeanDup.Extract.withAcceptedDeclarations payload modules fun _options declarations => do
          match selectDeclarations ids? declarations with
          | Except.error err => pure <| Except.error err
          | Except.ok selected => do
              let rows ← featureRows selected
              pure <| Except.ok rows
      match result with
      | Except.error err => pure <| Except.error (fromExtractError err)
      | Except.ok (Except.error err) => pure <| Except.error err
      | Except.ok (Except.ok rows) => pure <| Except.ok rows

end LeanDup.Features
