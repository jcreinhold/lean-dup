namespace Other

theorem imported_dup (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

end Other
