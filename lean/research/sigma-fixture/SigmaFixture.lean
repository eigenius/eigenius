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
