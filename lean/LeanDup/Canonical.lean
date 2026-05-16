/-!
`LeanDup.Canonical` owns Lean-side semantic fingerprint decisions.

Callers may compare the opaque keys this namespace will eventually produce, but
they must not depend on expression traversal order, binder representation,
pretty-printed statements, or cache/index layout outside Lean.
-/
namespace LeanDup.Canonical

/-- Semantic algorithm marker for the placeholder fingerprint implementation. -/
def version : String := "canonical.placeholder.v0"

end LeanDup.Canonical
