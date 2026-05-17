namespace External

theorem same_as_tiny (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

theorem external_only_left (p : Prop) : p → p := by
  intro hp
  exact hp

theorem external_only_right (q : Prop) : q → q := by
  intro hq
  exact hq

theorem related_external (p q : Prop) : p ∧ q → p := by
  intro h
  exact h.left

theorem contradiction_external (p _q : Prop) : p → ¬p → False := by
  intro hp hnp
  exact hnp hp

def clone_external (n : Nat) : Nat := n + 1

def ExternalOnlyMarker (n : Nat) : Prop := n = 42

axiom unmatched_external_marker (n : Nat) : ExternalOnlyMarker n

end External
