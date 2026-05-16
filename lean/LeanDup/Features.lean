/-!
`LeanDup.Features` owns role-aware statement features computed by Lean.

Callers may store and compare future feature keys as opaque semantic facts. They
must not reconstruct features from pretty text, source snippets, declaration
names, Rust ranking policy, or index storage rows.
-/
namespace LeanDup.Features

/-- Semantic algorithm marker for the placeholder feature implementation. -/
def version : String := "features.placeholder.v0"

end LeanDup.Features
