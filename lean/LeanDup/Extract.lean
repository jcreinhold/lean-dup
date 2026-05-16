/-!
`LeanDup.Extract` owns declaration extraction facts from the Lean environment.

Callers may rely on declaration/display/source facts emitted through the worker
protocol. They must not depend on environment traversal order, temporary
Python-era manifest behavior, source parsing fallback policy, or Rust cache
layout.
-/
namespace LeanDup.Extract

/-- Semantic algorithm marker for the placeholder extraction implementation. -/
def version : String := "extract.placeholder.v0"

end LeanDup.Extract
