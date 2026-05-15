import Other

namespace Tiny

def clone_two (n : Nat) : Nat := n + 1

theorem related_left (p q : Prop) : p ∧ q → p := by
  intro h
  exact h.left

theorem related_right (p q : Prop) : p ∧ q → q := by
  intro h
  exact h.right

theorem impossible_tiny (n : Nat) : n ≠ n → False := by
  intro h
  exact h rfl

end Tiny
