# The W3C PROV-O mapping

*Status: **drafted** `2026-09-04`. Discharges open question 2 of
[`ai-computed-provenance-explainer.md`](ai-computed-provenance-explainer.md) (eigenius#153).*

*Source of truth for the vocabulary: `ontologies/prov/prov.esl`. This document states the
correspondence; that file declares the terms.*

---

## 1. What this is, and what it is not

The explainer commits to *"a downward mapping to ensure records remain consumable by existing PROV
tooling"* and calls it *"inherently lossy"*. This is that mapping, stated as a correspondence
rather than built as an exporter. No serializer exists and none is proposed here — §5 says what an
exporter would additionally need, so that the two questions stay separate.

**Mapping, not adoption** — the schema.org precedent from D57. The names below line up with
PROV-O but the IRIs are `urn:eigenius:prov:*`, because PROV-O is OWL with open-world semantics
while these resources are typed with `requires` enforced at commit. Adopting the W3C IRIs would
mean redeclaring them in this type system anyway, and would assert an equivalence stronger than
§3 and §4 support.

## 2. The rows that map cleanly

| this platform | PROV-O | note |
|---|---|---|
| any committed `core:Resource` | `prov:Entity` | needs no declaration: every resource is one |
| `prov:Agent` | `prov:Agent` | |
| `prov:Person` | `prov:Person` | |
| `prov:Organization` | `prov:Organization` | |
| `prov:Activity` | `prov:Activity` | |
| `prov:was_attributed_to` | `prov:wasAttributedTo` | |
| `prov:was_generated_by` | `prov:wasGeneratedBy` | |
| `prov:was_associated_with` | `prov:wasAssociatedWith` | |
| `prov:used` | `prov:used` | |
| `prov:had_primary_source` | `prov:hadPrimarySource` | |
| `prov:started_at` | `prov:startedAtTime` | |
| `prov:completed_at` | `prov:endedAtTime` | |

`runtime:RuntimeInvocation` is a `prov:Activity` in all but name — it requires `language`,
`script`, `environment`, `inputs`, `output`, `started_at` and `completed_at`, and recommends
`image_digest`. Project it as an Activity, with `runtime:inputs` → `prov:used`, `runtime:output` →
the generated Entity's `prov:wasGeneratedBy`, and the timestamps onto
`startedAtTime` / `endedAtTime`. It is **not declared** a `prov:Activity` on the chain, and §5.2
records why that matters for an exporter.

## 3. The traces: one PROV edge, four distinctions PROV does not draw

This is where the mapping is lossy, and the loss is the whole point of the design.

`prov:DeclarationTrace`, `ObservationTrace`, `ProductionTrace` (with `ProgramTrace` under it) and
`VerificationTrace` all carry `prov:resource`, naming the resource the trace is about. Every one
of them projects to the same PROV shape — an Entity related to an Activity or an Agent — and PROV
has no vocabulary that separates them:

| trace | projects to | what PROV drops |
|---|---|---|
| `DeclarationTrace` | `Entity wasAttributedTo Agent` | that the assertion is *only* an assertion |
| `ObservationTrace` | `Entity wasGeneratedBy Activity` | that the Activity read the world rather than computed |
| `ProductionTrace` / `ProgramTrace` | `Entity wasGeneratedBy Activity` | indistinguishable from the row above |
| `VerificationTrace` | `Entity wasGeneratedBy Activity` | that a proof of the Entity's proposition was **checked** |

A PROV consumer reading the projection sees four provenance records and cannot recover which one
grounds a Verified claim. `prov:proof_system` and `prov:proof_term` survive as literals on the
projected Activity, so the information is not destroyed — but it stops being *structural*, and
nothing in PROV makes a consumer read it.

**`prov:wasDerivedFrom` is deliberately unused.** It covers truth-preserving and guessed
derivation equally, and a producer can assert one without having performed it. Sound as
provenance, unusable as warrant. Nothing in `prov.esl` emits it, and an exporter should emit it
only as a projection of a trace, never as a primary edge.

## 4. The axis PROV has no term for

PROV models identity and lineage. It does not model the *content* of a claim, and it therefore has
no term for the epistemic grade of an entity — which is the distinction the four-category design
exists to carry.

Two points make this more than a missing vocabulary item:

1. **Grade is computed, not stored.** Nothing on the chain carries an epistemic status property
   (eigenius#23 deleted the last one). A grade is derived from a justification term against the
   witnesses a layer admits (`layer_admits_witness`). There is no field for an exporter to read
   and no field for an importer to write.
2. **PROV graphs are entirely producer-writable.** A system can assert any edge it likes. The
   grade here is reached only through a witness the kernel synthesises against a committed trace,
   which is a different kind of claim from anything PROV can express.

So the projection is one-directional by construction. A PROV graph can be produced *from* this
chain; a PROV graph cannot be imported *as* one without every grade collapsing to ungraded.

**Provenance and warrant are orthogonal**, which a PROV reader will not expect. A hand-authored
claim with a checked proof is Verified warrant over Declared provenance. Where a term came from
does not move its grade. The projection preserves the first axis and drops the second.

## 5. What an exporter would additionally need

Out of scope for this document; recorded so the boundary is explicit.

**5.1 A serializer.** There is none. JSON-LD appears in the tree only as an *import* format
(`crates/eigenius-schemaorg`), and there is no Turtle or N-Triples writer.

**5.2 The in-process Activity gap.** `RuntimeInvocation` is built only when the substrate returns
a partial record — `dispatch.partial_invocation.as_ref()?` short-circuits otherwise — and
in-process institutions return `partial_invocation: None`
(`kernel/src/institution/in_process_registry.rs:217`). The reasoning, statistics and Lean
institutions all run in process, so a chain-wide export today would carry Activities for
externally dispatched work and none for the rest. Closing that is a prerequisite for an export
that claims coverage, and is not a prerequisite for this mapping.

**5.3 An IRI policy.** §1 keeps `urn:eigenius:prov:*`. An exporter has to decide whether it
rewrites to `http://www.w3.org/ns/prov#` on the way out — which asserts the equivalence this
document states — or emits both with an alignment vocabulary. That is open question 5's territory
(vendor-namespaced IRIs), not this one's.
