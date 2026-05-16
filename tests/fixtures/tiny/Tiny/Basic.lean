namespace Tiny

theorem same_left (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

theorem use_same_left (p q : Prop) : p → q → p := same_left p q

theorem same_right (a b : Prop) : a → b → a := by
  intro ha _hb
  exact ha

theorem reordered_left (p q r : Prop) : p → q → r → r := by
  intro _hp _hq hr
  exact hr

theorem reordered_right (p q r : Prop) : q → p → r → r := by
  intro _hq _hp hr
  exact hr

theorem and_left (p q : Prop) : p ∧ q → q ∧ p := by
  intro h
  exact And.intro h.right h.left

theorem and_right (p q : Prop) : q ∧ p → p ∧ q := by
  intro h
  exact And.intro h.right h.left

theorem dependent_left (α : Type) (x y : α) : x = x := rfl

theorem dependent_right (α : Type) (x y : α) : y = y := rfl

axiom independent_arrow_left (P Q R : Prop) : P → Q → R

axiom independent_arrow_right (P Q R : Prop) : Q → P → R

axiom connective_and_left (P Q : Prop) : P ∧ Q

axiom connective_and_right (P Q : Prop) : Q ∧ P

axiom symmetric_eq_left (α : Type) (x y : α) : x = y

axiom symmetric_eq_right (α : Type) (x y : α) : y = x

axiom nat_domain_key (n : Nat) : n = n

axiom bool_domain_key (b : Bool) : b = b

universe u

axiom universe_structure_left (α : Type u) : α = α

axiom universe_structure_right (α : Type (u + 1)) : α = α

inductive GeneratedProbe where
  | leaf : GeneratedProbe
  | node : GeneratedProbe → GeneratedProbe

private theorem private_dup_left (p q : Prop) : p → q → p := by
  intro hp _hq
  exact hp

private theorem private_dup_right (a b : Prop) : a → b → a := by
  intro ha _hb
  exact ha

def clone_one (n : Nat) : Nat := n + 1

end Tiny
