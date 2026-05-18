namespace External

theorem same_as_tiny (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

end External
