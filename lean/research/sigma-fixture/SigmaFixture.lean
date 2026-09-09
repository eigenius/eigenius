namespace EigeniusFFI

structure eigenius.test.Widget where
  size : Nat
deriving Repr

end EigeniusFFI

-- A chain relation, named as #208's mangling spells it. In ESL these are `axiom`s
-- (`axiom ontology:compound_kind : lexicon:Entity -> Set -> Prop`).
axiom EigeniusFFI.eigenius.test.Big : EigeniusFFI.eigenius.test.Widget → Prop

-- The SUBJECT is a refinement — the shape the DCG builds for a refined noun.
def refined :
    { w : EigeniusFFI.eigenius.test.Widget // EigeniusFFI.eigenius.test.Big w } → PUnit :=
  fun _ => PUnit.unit

-- A projection out of the refinement: `Fst` in EigenTT terms. `Subtype.val` takes the type and
-- the predicate implicitly, so the exported term carries them explicitly — which is what
-- externalization must reconstruct by inference.
def projected :
    { w : EigeniusFFI.eigenius.test.Widget // EigeniusFFI.eigenius.test.Big w } →
      EigeniusFFI.eigenius.test.Widget :=
  fun s => s.val

-- A statement whose TYPE contains the projection: `Big s.val`. In EigenTT that is
-- `Pi(s, Sig(w, Widget, Big w), App(Big, Fst(Var s)))` — the case that needs `Subtype.val`'s
-- implicits reconstructed.
theorem projects_in_the_type :
    ∀ (s : { w : EigeniusFFI.eigenius.test.Widget // EigeniusFFI.eigenius.test.Big w }),
      EigeniusFFI.eigenius.test.Big s.val :=
  fun s => s.property

-- D74 §4.8 — float literals. `0.1` and `-2.5` are `OfScientific` applications over nat literals
-- (the second wrapped in `Neg.neg`), which is what the externalizer has to build rather than emit.
-- A measurement claim is the motivating case: the quantity is the value the computation produced.
axiom EigeniusFFI.eigenius.test.Measured : Float → Prop

theorem measured_refl :
    EigeniusFFI.eigenius.test.Measured 0.1 → EigeniusFFI.eigenius.test.Measured 0.1 :=
  fun h => h

theorem measured_neg_refl :
    EigeniusFFI.eigenius.test.Measured (-2.5) → EigeniusFFI.eigenius.test.Measured (-2.5) :=
  fun h => h

theorem quantifies_over_float :
    ∀ (x : Float), EigeniusFFI.eigenius.test.Measured x → EigeniusFFI.eigenius.test.Measured x :=
  fun _ h => h

-- D86 — the numeric primitive core. These are the SHAPES the externalizer builds for the three
-- chain relations, written out so the round trip has something real to compare against. Before
-- these, `NumericRel`'s correspondence table had no test at all: the two asserted relations
-- (`Le`, `Eq`) and the three derived from them (`Ge`, `Gt`, `Lt`) were reviewed as TCB and never
-- exercised.
--
-- Each is `A -> A` for the same reason `measured_refl` is: the point is the TYPE the externalizer
-- has to reproduce, and an implication from a proposition to itself is provable without deciding
-- anything about `Float`. Deciding these is a different matter — `Float`'s operations are
-- `@[extern]` and do not reduce in the kernel, which is D86 §4's "asserted, not checked".

-- `stats:le(0.1, 0.5)` — `@LE.le.{0} Float instLEFloat`.
theorem le_refl_float :
    ((0.1 : Float) ≤ 0.5) → ((0.1 : Float) ≤ 0.5) :=
  fun h => h

-- `stats:float_ieee_eq(0.1, 0.1)` — `(a == b) = true` over `instBEqFloat`, NOT Lean's `Eq` on
-- `Float`, which is structural and separates `0.0` from `-0.0` (D86 §3.3).
theorem ieee_eq_refl_float :
    (((0.1 : Float) == 0.1) = true) → (((0.1 : Float) == 0.1) = true) :=
  fun h => h

-- `stats:lt(-0.42, 0.0)` — derived as `le(a,b) ∧ ¬eq(a,b)`, the conjunct being what makes `<`
-- come out FALSE at signed zero where `≤` and IEEE `==` both hold (D86 §3.2). `-0.42` is the
-- shape a recomputed Spearman rho takes in the WRN chain.
theorem lt_refl_float :
    ((((-0.42 : Float) ≤ 0.0) ∧ ¬(((-0.42 : Float) == 0.0) = true)) →
     (((-0.42 : Float) ≤ 0.0) ∧ ¬(((-0.42 : Float) == 0.0) = true))) :=
  fun h => h

-- Same derived shape, at the SAME values `le_refl_float` uses, to tell a structural mismatch in
-- the And/Not derivation apart from one in the literals.
theorem lt_simple_float :
    ((((0.1 : Float) ≤ 0.5) ∧ ¬(((0.1 : Float) == 0.5) = true)) →
     (((0.1 : Float) ≤ 0.5) ∧ ¬(((0.1 : Float) == 0.5) = true))) :=
  fun h => h

-- Isolating probes for the literal, not the relation: `le` is one of the two ASSERTED
-- correspondences and carries no And/Not, so a failure here is the float form alone.
theorem le_neg_nonzero_float :
    ((-0.42 : Float) ≤ 0.5) → ((-0.42 : Float) ≤ 0.5) :=
  fun h => h

theorem le_against_zero_float :
    ((0.1 : Float) ≤ 0.0) → ((0.1 : Float) ≤ 0.0) :=
  fun h => h

-- Probe: whole-number floats. `{:e}` renders `1.0` as "1e0" while Lean elaborates the literal
-- `1.0` as mantissa 10 with one fractional digit. If these disagree the zero fix is one instance
-- of a wider mismatch, not the class.
theorem le_whole_float :
    ((1.0 : Float) ≤ 2.0) → ((1.0 : Float) ≤ 2.0) :=
  fun h => h
