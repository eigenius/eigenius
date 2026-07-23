# D63 — named-entity glossary source (the fourth extraction source)

Extends `d63-document-preprocessing-scope.md` §2a. The document glossary is populated from several
extraction *sources*, all landing in the same doc-scoped lexicon layer. This note designs a **fourth**
source: **named entities** (proper compounds like "Project Achilles", "project DRIVE") → doc-local
**named individuals**. It is the structural fix for the last WRN-first-page grammar gap.

## 1. Why (the gap this closes)

Unit 4 — "Project Achilles and project DRIVE identified WRN as the top preferential dependency in MSI
cell lines compared to MSS cell lines." — was the last grammar gap on the first page.

It is **not a grammar-coverage gap**. Witnessed (spike `spike_named_entity_closes_unit4`,
`crates/eigenius-wordnet/tests/db_backed_encoding.rs`):

- With a non-verb compound head the grammar derives the whole structure: "Gene Achilles and gene DRIVE
  identified WRN as the top preferential dependency in MSI cell lines compared to MSS cell lines." →
  **12 readings**.
- The gap is caused solely by **"project" being noun+verb**. The verb entries crowd the
  coordinated-subject beam and the gold nominal reading is pruned — predicate-sensitively: the same
  subject gaps under "are essential" (0) but parses under "are dependencies" (28). At 6 tokens ("Project
  Achilles and project DRIVE are essential." → 0) this is not beam length; it is lexical-ambiguity
  crowding.

Registering the two names as `cat_np(Entity, sg)` named individuals (spike, reusing
`abbreviation_resources`) closes it: P6 0→5, coord-subj+`as` 0→15, **full unit 4 0→12**. Honest
grammar-gap → 0 (a real parse, replacing the junk UMLS reification the old baseline masked).

## 2. The source (mechanism — already built)

No new subsystem. A recognized name becomes ONE `lexicon:LexicalEntry`, the proper-noun (individual)
arm of the existing alias machinery:

- Mint a doc-local **named individual** `urn:eigenius:doc:ni_<slug> : <type>` (an instance, not a class).
- `abbreviation_resources` emits its `cat_np(sty, sg)` alias (`sem` = the individual; `sty` = the
  individual's type class). The multiword `form` seeds over its span (the lazy path seeds every span up
  to sentence length) and coordinates via `coordinate_np`.
- Commit into the doc-glossary layer (`with_persistent(backend)` — an in-memory layer chained on the
  7.6M-resource head materialises the parent → OOM).

The only prerequisite bug is fixed (commit `glossary: instance_type_classes accepts String-IRI is_a
targets`): a persisted individual's `is_a` round-trips to a String IRI, which the individual/class fork
must accept or it emits a `cat_n` common-noun alias instead of `cat_np`.

## 3. Design decisions

### 3a. Recognition rule (extraction) — OPEN

The two names share the shape **`<common-noun> <ProperName>`**: a lowercase-able head noun ("project")
apposed to a capitalized/all-caps token ("Achilles", "DRIVE"). Candidate rules:

1. **Deterministic apposition pattern** — a known common-noun head immediately followed by a
   Capitalized or ALL-CAPS token that is NOT itself a sentence start's ordinary word. Highest precision
   for this corpus; needs a guard against sentence-initial Title Case.
2. **Capitalized multiword run** — any maximal run of Capitalized/ALL-CAPS tokens. Higher recall,
   lower precision (fires on ordinary Title Case, headings).
3. **LLM proposer** — like the abbreviation LLM fallback (`AnthropicAbbreviationProposer`): untrusted,
   validated (the name must occur in the text), flows the same ground→emit→gate path. Best recall for
   irregular names; non-deterministic (needs record/replay like the sense ranker).

Recommendation: **(1) deterministic apposition first**, LLM proposer as the tail — mirrors the
abbreviation source's deterministic-first/LLM-tail split, keeps the measurement reproducible.
Validation guard: require the name to **recur** in the document OR the head to be a known common noun,
to reject one-off sentence-initial Title Case.

### 3b. Grounding + typing — retrieve-first, head-typed on miss

Per D43 retrieve-first: try to ground the full name against the lexicon (a curated concept for the
project is unlikely to exist). On a miss, mint a fresh doc-local individual — but type it from the
**head noun** rather than bare `Entity`: "project Achilles" → `is_a <project-concept>` (the concept
"project"/"research project" grounds to), so `sty` is the head's class, not top. The spike used bare
`Entity` (sufficient to close the gap; the transitive-verb subject slot is `Entity`). Head-typing is
the more faithful denotation and sharpens downstream selectional constraints.

### 3c. Shadow, not add — per §2a

The spike **adds** (the named-individual and the compositional `project`(V)+name parse coexist; the
coordinated case parses but the chart still carries the verb ambiguity). §2a's design goal is
**shadow**: the doc glossary ranks first in `scope`, so the name's span should **suppress** the
component "project"(V)+name compositional parse. Shadowing both closes the gap AND shrinks the chart
(fewer readings, less beam pressure — it removes the crowding that caused the gap in the first place),
and it is the memory-safer form. Adopt shadow for the named-entity span.

### 3d. Wiring + re-baseline

- Run the recognizer in Stage A alongside abbreviation extraction; both emit into the one doc-glossary
  layer. The first-page sweep currently applies only OOV grounding (`augment_lexicon_backed`) + no
  named-entity source — add the source to its document stage.
- Re-baseline `experiments/parsing/baseline.json` once the source is live: grammar-gap **0** (honest),
  reranked readings/skeletons re-recorded. The other project-name units ("Project Achilles screened
  cell lines…", "Project DRIVE analysed cell lines…") currently pin **compositional** readings; with
  the named-entity source (esp. shadow) they shift to named-individual readings and must be re-pinned.

## 4. Status

- Mechanism: **built + witnessed** (spike passes; unit 4 → 12).
- Prerequisite kernel fix: **committed**.
- Recognizer (3a), head-typing (3b), shadow (3c), wiring/re-baseline (3d): **to build** — this note is
  the design; 3a and 3c carry the open decisions to confirm before implementing.
