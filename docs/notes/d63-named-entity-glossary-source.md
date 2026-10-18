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

### 3a. Recognition rule (extraction) — CONFIRMED: deterministic apposition + LLM tail

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

**Decision: (1) deterministic apposition first**, LLM proposer as the tail — mirrors the abbreviation
source's deterministic-first/LLM-tail split, keeps the measurement reproducible. Validation guard:
require the name to **recur** in the document OR the head to be a known common noun, to reject one-off
sentence-initial Title Case.

### 3b. Grounding + typing — retrieve-first, head-typed on miss

Per D43 retrieve-first: try to ground the full name against the lexicon (a curated concept for the
project is unlikely to exist). On a miss, mint a fresh doc-local individual — but type it from the
**head noun** rather than bare `Entity`: "project Achilles" → `is_a <project-concept>` (the concept
"project"/"research project" grounds to), so `sty` is the head's class, not top. The spike used bare
`Entity` (sufficient to close the gap; the transitive-verb subject slot is `Entity`). Head-typing is
the more faithful denotation and sharpens downstream selectional constraints.

### 3c. Shadow, not add — CONFIRMED (per §2a)

The spike **adds** (the named-individual and the compositional `project`(V)+name parse coexist; the
coordinated case parses but the chart still carries the verb ambiguity). §2a's design goal is
**shadow**: the doc glossary ranks first in `scope`, so the name's span should **suppress** the
component "project"(V)+name compositional parse. Shadowing both closes the gap AND shrinks the chart
(fewer readings, less beam pressure — it removes the crowding that caused the gap in the first place),
and it is the memory-safer form. Adopt shadow for the named-entity span.

**Two shadow forms (do not conflate).**

- **(a) Span-protection shadow** — suppresses the compositional *re-bracketing* over the name's span
  (the "project"+name compound split). Implemented via `multiword_protected_splits` (below). Verified:
  P6 5→3, coord+as 15→9 readings; coverage-safe; but it does NOT touch the sentence-initial
  "project"-as-**verb** parse (which combines rightward, never building a constituent over `[i,j]`), so
  single-subject "Project Achilles identified WRN…" stayed at 111.
- **(b) Sense-precedence shadow** (§2a) — the stronger form: the doc-glossary entry ranks first in
  `scope`, so the component tokens' competing senses (the "project" VERB leaf at that position) are
  de-prioritized/dropped at SEED. This is what actually kills the verb-crowding. Evaluate at re-baseline
  (§3d): if (a) + the named individual holds the readings/beam budget on the full page, (b) is optional;
  if verb-crowding still pressures long coordinations, add (b).

**Span-protection hook (implemented).** The mechanism already exists:
[`chart::multiword_protected_splits`](../../kernel/src/dcg/chart/mod.rs) protects a multiword lexeme's
interior split points on the base pass (`prefer_multiword = true`), pruning its compositional
re-bracketings; widen-on-failure passes `prefer_multiword = false`, re-admitting every split — so it is
**coverage-safe** (`grammar-gap 0` preserved). But [`chart::multiword_spans`](../../kernel/src/dcg/chart/mod.rs)
only marks a span protected when its cell carries a `cat_n` (`cat_n_number`) or `cat_group` item — **not
a `cat_np`**. A named-entity alias is `cat_np`, so shadowing it is a one-line extension: also protect a
span whose cell holds a multiword `cat_np`. This shadows every multiword `cat_np` (named individuals),
which is the intended semantics (a named entity shadows its compositional reading); coverage-safety is
free via the existing widen fallback. Guard the change with the differential oracle (both chart drivers
share this single source of truth) and confirm `grammar-gap 0` holds on the full page.

### 3a′. Recognition is lexicon-aware — and "is a common noun" does NOT discriminate

Abbreviation extraction is purely orthographic, so `abbrev` is text-in/pairs-out and the lexicon enters
only at grounding. Apposition is **not**: the head must be a NOUN, and orthography alone conflates
`project DRIVE` (noun+name), `identified WRN` (verb+object), `in DRIVE` (prep+name). So the head test is
part of recognition — an **injected predicate** (`extract_named_entities_with`, unit-testable with a
closure), with the layer-backed predicate in `glossary`.

**Empirical correction (measured on the CNL-v3 page).** "Is a common noun" (has a `cat_n` entry) was the
first head test — but in the served 7.6M-entry lexicon it is TRUE for essentially every surface
("identified", "evaluated", "other", "somatic", "deficient" all return true). Using it, the recognizer
fired 8 candidates — 6 false positives (verb heads "identified WRN"/"evaluated MSI", adjective heads
"other DNA"/"somatic MMR") — while the broken symmetric name filter *dropped* "Project Achilles"
(because "achilles" also has a noun sense). grammar-gap 0 was reached, but partly via WRONG readings
(the coverage-not-correctness trap). Two signals that DO discriminate, verified to yield EXACTLY the two
real names:

- **Recurrence ≥2** — a named entity is referred to repeatedly; a one-off verb+object is not. (On the
  page: "Project Achilles"/"project DRIVE" ×3; "identified WRN" ×1.) This is the primary filter.
- **Head is not an adjective** (`S[adj]\NP`) — rejects "somatic MMR"/"other DNA" (recurring or not),
  which recurrence alone lets through ("somatic MMR" ×2). The noun/verb homonym "project" stays
  admissible (it is not an adjective); verb-head one-offs are killed by recurrence, not this.

So the head predicate is `is_apposition_head = is_common_noun && !is_adjective`, and admission requires
recurrence. The name-not-a-common-noun filter is **dropped** (unachievable — everything is a noun — and
it wrongly rejects real Title-case names). End-to-end witness: `named_entity_source_closes_unit4_via_overlay`
— the augmentation recognizes exactly the two names, and unit 4 parses as
`And(…ni_project_achilles…, …ni_project_drive…)` (the correct distributed coordination).

### 3d. Emission + wiring — the augment overlay, not a persistent doc layer

Doc-scoped entries reach the parser two ways today: `abbrev`→`glossary` commits `LexicalEntry`
Resources to a **persistent** doc layer; the OOV `augment` overlays `LexicalBinding`s **in-memory**
(`with_document_augmentation`, already chained by the sweep as `build_index_over(&head, Some(&aug))`).
The named-entity source uses the **augment overlay**:

- Emission mints the head-typed individual + the `cat_np` `LexicalEntry` Resource (shared
  `abbreviation_resources` machinery), packaged as a `LexicalBinding { proposed }` +
  `supporting: [individual]`, merged into the `LexiconAugmentation` the sweep already applies.
- Rationale: (1) the OOM the spike hit came from the persistent doc-layer path — the overlay is
  in-memory, no `store_layer`; (2) it unifies named entities with the OOV augmentation (one overlay);
  (3) the production `DocumentPipeline` can still commit the SAME Resources to a persistent doc layer
  (one emission, two commit adapters — do not fork the entry shape).

### 3e. Wiring + re-baseline

- Run the recognizer in Stage A alongside abbreviation extraction; both emit into the one doc-glossary
  layer. The first-page sweep currently applies only OOV grounding (`augment_lexicon_backed`) + no
  named-entity source — add the source to its document stage.
- Re-baseline `experiments/parsing/baseline.json` once the source is live: grammar-gap **0** (honest),
  reranked readings/skeletons re-recorded. The other project-name units ("Project Achilles screened
  cell lines…", "Project DRIVE analysed cell lines…") currently pin **compositional** readings; with
  the named-entity source (esp. shadow) they shift to named-individual readings and must be re-pinned.

## 4. Status

- Mechanism: **built + witnessed** (spike passes; unit 4 → 12).
- Prerequisite kernel fix: **committed** (`instance_type_classes` String-IRI).
- Design decisions: **confirmed** — 3a deterministic apposition + LLM tail; 3c shadow.
- Shadow hook: **located** — extend `multiword_spans` to protect multiword `cat_np` (coverage-safe via
  the existing widen fallback).

### Implementation status

1. **Shadow** — DONE (`4395409`): `multiword_spans` protects multiword `cat_np`; coverage-safe;
   grammar-gap 0. Bites in-chart (P6 5→3, coord+as 15→9).
2. **Recognizer** — DONE (`e3e060b`, redesigned in `d0b65b7`): `extract_named_entities_with`, admitted on
   recurrence ≥2 + head-not-adjective (see 3a′). 6 unit tests.
3. **Emission** — DONE (`d0b65b7`): `glossary::named_entity_augmentation` mints a head-typed doc-local
   individual + `cat_np` alias (`abbreviation_resources`, in-memory resolution) → `LexicalBinding`s.
   Kernel fix `instance_type_classes` String-IRI (`ee02901`).
4. **Wiring** — DONE (`d0b65b7`): the sweep merges the named-entity bindings into the `LexiconAugmentation`
   it already overlays → grammar-gap 0, COVERAGE: PASS. End-to-end witness
   `named_entity_source_closes_unit4_via_overlay`.
5. **Re-baseline** — DONE (`4f04763`): reranked draw, replay 62/0; grammar-gap 0, encoded 10, readings
   931→769 (−17%), skeletons 240→220, expected-hits 18/18→19/19 (unit 4 added, two project-name units
   re-pinned to named-individual readings).
6. **LLM tail** — DEFERRED (optional): a `use-llm` proposer for irregular names the deterministic
   apposition pattern misses (record/replay, like `AnthropicAbbreviationProposer`). The deterministic
   core is validated; the tail is a recall improvement, not a correctness gap.

### Known limits / follow-ups

- **Recall** — v1 requires recurrence ≥2 and a `<head> <Name>` shape; a name mentioned once, or in the
  `<Name> <head>` order ("Lynch syndrome"), is missed. The LLM tail (6) is the intended lift.
- **Head-typing** — the individual is typed at the head noun's `cat_n` concept (`common_noun_concept`),
  else `lexicon:Entity`. Grounding the NAME itself (retrieve-first against the lexicon) is not attempted;
  the individual is always doc-local-minted.
