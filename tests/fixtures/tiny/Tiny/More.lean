import Other

namespace Tiny

def clone_two (n : Nat) : Nat := n + 1

theorem related_left (p q : Prop) : p ∧ q → p := by
  intro h
  exact h.left

theorem related_right (p q : Prop) : p ∧ q → q := by
  intro h
  exact h.right

end Tiny
