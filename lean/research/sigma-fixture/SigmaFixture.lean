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
