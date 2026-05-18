namespace Tiny

theorem same_left (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

end Tiny
