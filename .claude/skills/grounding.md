---
name: grounding
description: The research + knowledge-base method for the reasoning protocol's Anchor phase — before reasoning in new territory, find what the kernel already knows (D43 text/vector retrieval), fill the gap with external research, and map the results back in as retrievable anchors (reference/CiTO citations) and aligned standard vocabulary (OBO via the obograph importer, schema.org per D57). The kernel is the knowledge base; research feeds it. TRIGGER when entering unfamiliar territory, when a claim needs prior-knowledge support, when deciding which external ontology/vocabulary to adopt, or whenever `reasoning` reaches its Anchor step. Builds on the `eigenius` skill (mechanics) and feeds the `reasoning` skill (method).
---

# Grounding (research, knowledge base, vocabulary)

Anchor reasoning in what is already known before reasoning anew. **The knowledge
base is the kernel** — existing resources, witnesses, conclusions, and imported
vocabulary are all retrievable. The loop is closed: external research doesn't just
inform the current task, it is **mapped back into the kernel** so the next task
retrieves it instead of re-researching it.

This is the engine behind the `reasoning` skill's **Anchor** phase: don't leap into
unfamiliar ground — run this loop first.

## Prerequisites

- Stack up; `eigenius` skill for mechanics (retrieval is EigenQL — query via MCP
  `eigenius_query` or `eigenius … query`).
- External web research is the **`deep-research`** skill (fan-out + adversarial
  verification). Use it for step 2.

## The loop

### 1. Retrieve-first — ask the kernel before the web (D43)
Existing knowledge is found by **retrieval**, not exact-IRI guessing. D43's `~`
operator does text (BM25), vector (embedding), or hybrid (RRF) search over indexed
resources:
```
USING "urn:eigenius:..."          // optional class scope
MATCH ?r { description: ?desc }
WHERE ?desc ~ "<your question in natural language>"
      { via: hybrid, model: "urn:eigenius:embed:<model>:v1", k: 50, limit: 20 }
RETURN [] { r: ?r, desc: ?desc }
TOP 20
```
Use this to find: prior **conclusions/witnesses** (don't re-derive), existing
**domain vocabulary** (don't redefine), already-imported **external terms** (don't
re-import), and relevant **anchors** (don't re-cite). What retrieval surfaces is
reuse; what it misses is the research gap. (Retrievability requires a `text_index`
/ `vector_index` on the property — ESL §4.4a; embeddings need a registered
embedder + `core:vec_model`. Detail: EigenQL guide ch.6, D43.)

### 2. Gap → external research
The terms retrieval *couldn't* answer define the research question. Run
**`deep-research`**; keep only claims that survive adversarial verification. Track,
for each kept claim, its **real** source (resolvable DOI / PMID / URL) — never
fabricate one.

### 3. Map results back into the kernel
Two kinds of result, two homes — both make the finding **retrievable next time**:
- **Claims → anchors.** Each load-bearing external fact becomes a
  `reference:Reference` (the work) + a CiTO-typed `reference:Citation` carrying the
  imported claim as `reflection:canonical_proposition` (+ `DeclarationTrace`). See
  the `reasoning` skill's Anchor template and `chain/02-literature.esl`. These are
  the admissible premises the reasoning builds on.
- **Terms → vocabulary.** Recurring entities/relations that deserve typing become
  domain predicates — or, better, get **aligned to a standard vocabulary** (step 4)
  rather than reinvented.

### 4. Detect & select external vocabularies/ontologies (grounding)
When a domain has a *standard* vocabulary, adopt it instead of bespoke terms — it
grounds your claims in shared, round-trippable IRIs.

**Detect candidates:** OBO Foundry ontologies (GO, ChEBI, MONDO, Uberon, …) for
life-science entities; **schema.org** (`urn:schema_org:`, per **D57**) for generic
descriptive metadata (dataset/file/creator/license/identifier); W3C / domain
standards otherwise.

**Select on:** (a) **coverage** — does it type the task's entities/relations?
(b) **authority & maintenance** — canonical, versioned, maintained; (c)
**license** — importable; (d) **granularity** — import the *minimal relevant
slice*, not all 800 schema.org types / 52k GO classes unless needed; (e)
**overlap** — already imported (step 1)? then *align*, don't duplicate; (f)
**round-trip** — stable IRIs for interchange (record `sameAs` / a prefix rule).

**Import the slice:**
- **OBO** — the obograph importer:
  `cargo run --bin obograph_import -- --input <obo-graph.json> --output <x.eigon.json>`
  then `eigenius … load <x.eigon.json>`. It rewrites `http(s)://` → `urn:` IRIs,
  stamps `core:source_irl` provenance, tags nodes `is_a [..., DeclaredResource]`,
  and relies on the shared `ontologies/obo/obo-meta-ontology.json`.
- **schema.org** — per D57 (**implemented**): own `urn:schema_org:` namespace,
  classes → `core:Class`, properties → `core:Property`, descriptive (`domainIncludes`
  → advisory `recommends`, never the restrictive `core:domain` — schema.org's type
  system is recommendation-based, open-world). The **full vocabulary is generated +
  committed** as a first-class
  ontology at [`ontologies/schema-org/schema-org.eigon.json`](../../ontologies/schema-org/)
  (2114 resources from V30.0) — adopt it by loading/referencing that ontology, not by
  re-deriving. The generator (`crates/eigenius-schemaorg`) regenerates it deterministically.

**Align:** map your ad-hoc task predicates → the standard term IRIs (e.g. a task
`Gene` → the GO/relevant ontology class), so claims reference shared vocabulary.
Imports are **Declared** (adopted on the source's authority), with `core:source_irl`
provenance — never silently re-minted as Eigenius-native.

**Read the spec, not just the data — and cite the decisions it drives, as you make
them.** A vocabulary's machine artifact (JSON-LD, OWL, OBO graph) gives you the
*terms*; its prose specification (data-model / conformance docs) gives you the
*semantics* that govern how to map them — e.g. schema.org's data-model doc states
`domainIncludes`/`rangeIncludes` are **advisory**, which is why they map to advisory
`core:recommends`, not the restrictive `core:domain`. Pull the authoritative docs
*proactively* (`deep-research` / `WebFetch`), and the moment a mapping decision turns
on a documented fact, commit a `reference:Citation` carrying that fact in the same
step — it is a load-bearing anchor. (Anti-pattern, seen in D57: the mapping was built
from the JSON-LD alone and the conformance citation was added only when a human asked.)

### 5. Re-index so it's findable next time
Imported vocabulary + new anchors must be retrievable by the next retrieve-first
pass. D43 consolidation re-extracts text indexes in `LayerBuilder::build` and
rebuilds vector indexes on commit (the sweep triggers reindex). Confirm a `~`
query now surfaces what you just added — that's the loop closing.

### 6. Discover before you conclude — the gated step (D61)
Retrieving and mapping isn't enough: a faithful encoding rests on the *right facts
having been discovered*, and the failure mode is concluding before checking the spec
(D57 #9: `domainIncludes` advisory → `core:recommends`, surfaced only when a human
asked). So make discovery **first-class and gated**. Before a milestone may conclude,
enumerate the load-bearing **discovery targets** it rests on, phrased as **competency
questions** (D61 §3's *descent* — turn the goal into typed, runnable targets:
decisions, desirable properties, the tensions where faithfulness is at risk). Each
unanswered target is a **blocker**: the **Discovered gate** (D61 §6;
[`experiments/objectives/well-posed-discovered.eigenql`](../../experiments/objectives/well-posed-discovered.eigenql),
run alongside the other on-demand gates — empty result = passes) holds a milestone
open while any `objective:CompetencyQuestion` it names via `objective:discovery_target`
is ungrounded. This is `reasoning`'s *fail-closed* moved upstream into grounding.

## Disciplines

1. **Retrieve before you research.** The kernel first; the web only for the gap.
2. **Real sources only.** Anchors cite resolvable DOIs/PMIDs/URLs; never fabricate.
3. **Adopt, don't reinvent.** If a standard vocabulary types the domain, align to
   it; reinventing `urn:eigenius:` equivalents is waste and breaks round-trip.
4. **Minimal slice.** Import the subset the task needs; expand on demand.
5. **Provenance on every import.** `core:source_irl` + `DeclaredResource` grade —
   adopted knowledge is Declared, not asserted as derived.
6. **Close the loop.** What you researched becomes a retrievable anchor/term, so
   the work compounds instead of repeating.
7. **Spec over data; cite proactively.** Adopt a standard from its authoritative
   documentation, not just its machine artifact, and commit the citation for each
   load-bearing decision as you make it — not when prompted.
8. **Discover before you conclude — gate on open targets.** Enumerate the
   load-bearing discovery targets (competency questions) a milestone rests on; an
   open target is a blocker (the Discovered gate, D61 §6), not a thing to conclude
   past.

## Going deeper

- **D43** retrieval — [d43-text-and-vector-retrieval.md](https://github.com/eigenius/eigenius/blob/main/docs/design/d43-text-and-vector-retrieval.md);
  surface in EigenQL guide [ch.6](https://github.com/eigenius/eigenius/blob/main/docs/guides/eigenql/06-text-and-vector-retrieval.md);
  indexes in ESL guide §4.4a; embedder `crates/eigenius-embedder-candle`.
- **obograph importer** — `crates/eigenius-obograph/` (`--bin obograph_import`);
  shared meta-vocab `ontologies/obo/obo-meta-ontology.json`.
- **schema.org mapping** — [D57](https://github.com/eigenius/eigenius/blob/main/docs/design/d57-schema-org-vocabulary-mapping.md) (implemented; the committed ontology is `ontologies/schema-org/`). The first dogfood of this skill + the `reasoning`/objective protocol.
- **Anchors** — `ontologies/reference/reference.esl` (Reference + CiTO Citation);
  worked example `experiments/publications/wrn-helicase/chain/02-literature.esl`.
- **External research** — the `deep-research` skill.
