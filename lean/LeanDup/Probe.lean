/-!
`LeanDup.Probe` owns bounded semantic checks between candidate declarations.

Callers may rely on protocol-level probe result fields once implemented. They
must not depend on proof-search strategy, reducibility internals, timeout
policy, or the shape of Lean expressions used to answer a probe.
-/
namespace LeanDup.Probe

/-- Semantic algorithm marker for the placeholder probe implementation. -/
def version : String := "probe.placeholder.v0"

end LeanDup.Probe
