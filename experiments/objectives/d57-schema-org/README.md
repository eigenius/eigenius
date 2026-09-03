# Objective: D57 schema.org mapping

The first **dogfood** of the `reasoning` + `grounding` skills and the D58
objective-framing protocol: the unit of work (map schema.org into Eigenius —
[D57](../../../docs/design/d57-schema-org-vocabulary-mapping.md)) is itself
expressed in Eigenius as a typed obligation graph and checked for well-posedness
before execution.

- **[D57](../../../docs/design/d57-schema-org-vocabulary-mapping.md)** — the actual
  deliverable + the settled mapping discipline (§3).
- **[D58](../../../docs/design/d58-objective-framing-and-obligation-graphs.md)** —
  the `objective:` framing ontology this chain instantiates.
- **[HARVEST-d58.md](HARVEST-d58.md)** — the D58 ontology requirements this dogfood
  surfaced (8 findings).

## The chain was retired `2026-08-29`

**The dogfood is done; its output is this directory's other two files.** The 7-file
obligation chain, its validation test and its demo runner were removed during the
judgements-and-warrants refactor (P4). What the exercise produced — the
[HARVEST](HARVEST-d58.md)'s 8 findings and the `objective:` ontology they shaped —
is unaffected and stays.

Why it went rather than being repaired: the chain's capstone discharged each
antecedent by **citing a milestone conclusion as `Verified`**, and none of those
conclusions was proved — they rest on `Declared` leaves. P3 stopped minting a
`Verified` witness for anything but a checked proof term, so the synthesis no longer
committed. Repairing it meant recomposing the certificates through to their real
grounds across 8 citation sites, for the one and only `objective:Objective` ever
authored, last extended in June 2026.

The same repair on the WRN chain is worth doing, because that chain is a live
reproduction of a published result. This one had already returned its value.

`convert-properties.esl` stays — it is the D57 mapping-discipline fixture, exercised
by `crates/eigenius-schemaorg/tests/convert_properties_validate.rs`, and has nothing
to do with the obligation graph.
