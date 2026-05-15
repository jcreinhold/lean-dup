namespace Tiny

theorem same_left (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

theorem same_right (a b : Prop) : a → b → a := by
  intro ha _hb
  exact ha

def clone_one (n : Nat) : Nat := n + 1

end Tiny
