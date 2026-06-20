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

## The chain (load order)

Stacks on `main` (core + reflection + reasoning + reference, all bootstrap-seeded).
Load in order:

| # | File | What it adds |
|---|---|---|
| 1 | [chain/00-objective.esl](chain/00-objective.esl) | the obligation-graph propositions (thesis + milestones as `Prop` decls) + the two anchors (schema.org, Croissant) as `reference:Citation`s |
| 2 | [chain/01-discipline.esl](chain/01-discipline.esl) | milestone **m1** — the mapping discipline, a Declared rule + the `ReasoningSentence` that discharges it (verdict: **Holds**) |
| 3 | `ontologies/objective/objective-ontology.esl` | the D58 `objective:` ontology (Objective / Milestone / Axiom) — a shared layer, not specific to this objective |
| 4 | [chain/02-objective-typed.esl](chain/02-objective-typed.esl) | the **typed obligation graph**: `objective:Objective` + Milestones + Axioms, with acceptance grades (`reflection:epistemic:*`), `depends_on` edges, and `satisfied_by` links |

```bash
EP=http://localhost:50051
H=$(eigenius --endpoint $EP branch list | awk '/^main /{print $2}')
eigenius --endpoint $EP branch create obj-d57 --from "$H"
for f in chain/00-objective.esl chain/01-discipline.esl \
         ../../../ontologies/objective/objective-ontology.esl \
         chain/02-objective-typed.esl; do
  eigenius --endpoint $EP load --branch obj-d57 "$f"
done
```

## Check well-posedness (D58 gates)

Three gates are enforced by the **type system at commit** (a malformed frame won't
load): *Expressible*, *Checkable* (`Milestone` requires grade+witness_kind+falsifier),
and *Anchored-presence* (`Axiom` requires a witness). The two runtime gates are
committed queries — **an empty result means the gate passes**:

```bash
eigenius --endpoint $EP query --branch obj-d57 "$(cat ../well-posed-reachable.eigenql)"  # every node reachable from the thesis
eigenius --endpoint $EP query --branch obj-d57 "$(cat ../well-posed-anchored.eigenql)"   # every axiom's witness resolves
```

Confirm m1 Holds:

```bash
eigenius --endpoint $EP query --branch obj-d57 \
  'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:institution:verdict_subject": ?s, "urn:eigenius:core:ctor_name": ?c } RETURN [] { s: ?s, c: ?c }'
```

## Milestone status

| Milestone | Proposition | Status |
|---|---|---|
| m1 | mapping discipline defined | **satisfied** (`concl_discipline` Holds) |
| m2 | proof-of-shape probe binds to a real file | open |
| m3 | generator emits the mappable vocabulary | open |
| m4 | the cut accounted (mapped vs residual) | open |
| thesis | schema.org is mapped | open (composes m1–m4) |

As each milestone is executed, flip its `objective:status` to `satisfied` and set
`objective:satisfied_by` to the discharging `ReasoningSentence` (as m1 already shows).
