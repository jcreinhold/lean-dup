import Lean

/-!
`LeanDup.Frames` owns the one-line JSON frame shapes of the lean-dup worker's
native JSONL transport (see `LeanDup.Server`). They are byte-compatible with
the retired `lean-rs-worker` streaming envelopes so the Rust parent's decoding
is unchanged across the transport swap.
-/
namespace LeanDup.Frames

open Lean

private def jsonString (value : String) : String :=
  (Json.str value).compress

def row (stream payloadJson : String) : String :=
  "{\"stream\":" ++ jsonString stream ++ ",\"payload\":" ++ payloadJson ++ "}"

def diagnostic (code message : String) : String :=
  "{\"diagnostic\":{\"code\":" ++ jsonString code ++ ",\"message\":" ++ jsonString message ++ "}}"

def progress (phase : String) (current : Nat) (total : Option Nat := none) : String :=
  let totalField :=
    match total with
    | none => ""
    | some total => ",\"total\":" ++ toString total
  "{\"progress\":{\"phase\":" ++ jsonString phase ++ ",\"current\":" ++ toString current ++ totalField ++ "}}"

def metadata (metadataJson : String) : String :=
  "{\"metadata\":" ++ metadataJson ++ "}"

def result (resultJson : String) : String :=
  "{\"result\":" ++ resultJson ++ "}"

def successSummary (command : String) (rowCount skipped : Nat) : String :=
  (Json.mkObj
    [ ("command", Json.str command)
    , ("rows", Json.num rowCount)
    , ("skipped", Json.num skipped)
    , ("ok", Json.bool true)
    ]).compress

def failureSummary (command : String) (message : String) : String :=
  (Json.mkObj
    [ ("command", Json.str command)
    , ("rows", Json.num 0)
    , ("ok", Json.bool false)
    , ("message", Json.str message)
    ]).compress

end LeanDup.Frames
