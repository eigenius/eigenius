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
