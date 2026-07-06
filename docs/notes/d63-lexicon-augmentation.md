# D63 — Document lexicon augmentation: closing lexical gaps in the pipeline

**Status:** design. How the `DocumentPipeline` resolves the lexical gaps biomedical text opens — abbreviations
the text defines *and* out-of-vocabulary (OOV) terms it doesn't — into grounded, typed lexical entries,
exposing the augmentation as a first-class, composable value. Concretises the **"RecQ Phenomenon"** — three
sub-problems a rigid ontology stalls on: **lexical voids** (a multi-word expression with no contiguous entry,
`RecQ DNA helicase`), **gene-family metonymy** (`RecQ` standing for its members WRN/BLM/RECQL), and
**productive morphology** (`RECQ-like`) — into a pipeline shape.

## 1. Problem — closed-world lexicon vs open-world text

The committed lexicon (WordNet + UMLS + NCBI) is **closed-world** (fixed entries); biomedical prose is
**open-world**: multi-word expressions with no contiguous entry (`RecQ DNA helicase` ≈ UMLS `C0084304`, but
not as a string), gene *families* used metonymically (`RecQ` for WRN/BLM/RECQL), and productive morphology
(`RECQ-like`). The parser stalls at OOV atoms (`has_token` = false, [lookup.rs:570](../../kernel/src/dcg/lookup.rs)).
Today the pipeline's **Stage A** ([glossary.rs](../../kernel/src/dcg/glossary.rs)) closes only *intra-document*
abbreviations (Schwartz-Hearst + an LLM proposer). `recq` — the last OOV on the WRN page — is the symptom
that Stage A must generalize.

## 2. Reframe — the abbreviation glossary is one case of "surface → grounded typed entry"

Stage A already does something more general than "abbreviations," unnamed: `glossary_resources` takes a
**grounded binding** (a surface + the concept its long form resolves to) and emits a `lexicon:LexicalEntry`
that is an **alias of that concept, carrying the concept's category** — keyed on the concept's ontological
kind (CUI → common noun, gene → proper noun). That emission tail is **grounding-agnostic**; only the *front*
(Schwartz-Hearst) is abbreviation-specific. So Stage A generalizes from **"abbreviation glossary"** to
**"document lexicon augmentation"** — resolve *every* lexical gap through the same tail.

## 3. The unit — `LexicalBinding` wraps a *proposed* `LexicalEntry`

`LexicalBinding` is **not** a rival to `lexicon:LexicalEntry` ([lexicon-ontology.esl:221](../../ontologies/lexicon/lexicon-ontology.esl)):
it **wraps a proposed, un-committed one** and adds the envelope — how the entry was produced and how far to
trust it. Today's `AbbrDef{short, long, context}` is the same idea *without* the entry (the entry is built
later by `glossary_resources`); here the resolver builds the entry up front and **keeps** its provenance
instead of discarding it.

```
LexicalBinding {
    proposed:   Resource,     // a lexicon:LexicalEntry (form/cat/sem/sem_type/grade) — the SAME type seeded + committed
    provenance: Provenance,
}
Provenance {
    surface:     String,           // the form the gap was found under (pre-normalization)
    long_form:   Option<String>,   // the intra-doc definition — Some for abbreviations, None for a bare OOV
    context:     String,           // the source window (grounding retries + audit)
    method:      ResolutionMethod, // DefinitionExtracted | RetrievalGrounded | LlmSynthesized — a TRUST signal
    grounded_to: Option<Iri>,      // the ontology concept the entry aliases, when grounding (§5)
    confidence:  Option<f32>,      // for retrieval / LLM
}
Gap { surface: String, context: String, tried: Vec<ResolutionMethod> }  // a gap NO proposal closed — separate
```

A separate `Grounding` union is unnecessary: a concept-alias entry just has `sem = concept`, a
synthesized-type entry has the composed `cat`/`sem` — that is already *in the wrapped entry*, exactly as for
any entry. What's worth keeping — *how* it was grounded — is a **provenance / trust field**
(`method` + `grounded_to` + `confidence`), not a shape. And an unresolved gap is not a wrapper around a
missing entry; it is a separate `Gap` (never silently dropped). So the abbreviation
`{proposed: ⟨"MSI" aliases C0920269⟩, provenance:{long_form:"microsatellite instability", method:DefinitionExtracted}}`
and the OOV `{proposed: ⟨"RecQ" aliases C0084304⟩, provenance:{method:RetrievalGrounded, grounded_to:C0084304}}`
are the **same envelope over different entries**.

`LexicalBinding` is therefore the **proposal envelope in propose → gate → commit**: the resolver emits
bindings, the kernel gates each `proposed` entry (it type-checks; the concept exists), passing entries commit
to the doc-layer, the provenance rides along.

**Harvesting protocol.** Because every closed gap is a *proposed `LexicalEntry` + provenance + trust*, running
the pipeline **harvests candidate permanent lexicon additions as a byproduct**. Each `added` binding is a
review-ready proposal — *what* entry, *from what* evidence, *by which* method, *how* confident — that can be
promoted from the transient doc-layer into a committed `lexicon:Lexicon`. Document processing *is* lexicon
growth: `missing_oov` says what the lexicon still lacks; `added` says what it just learned and how much to
trust it. The `method`/`confidence` fields are exactly the review filter (auto-promote `DefinitionExtracted` +
high-confidence `RetrievalGrounded`; queue `LlmSynthesized` for human review).

## 4. Resolution hierarchy — compose > ground > synthesize

Close a gap by the cheapest, most faithful means first (this ordering *is* the "reuse existing vocab, don't
mint" discipline):

1. **Compositional** — decompose the OOV span into known atoms + one unknown atom; resolve the **atom** and
   let the **grammar** compose the phrase (`compound_kind`, [parser.rs:630](../../kernel/src/dcg/parser.rs)).
   For `RecQ DNA helicase` the one unknown atom is `recq`; the form-index **grounds it** to `C0084304`
   (§6a), after which the noun-noun compound rule (`compound_kind`) composes the phrase — and the same
   grounding unblocks `RECQ-like` (feeds [d63-compound-morphology.md](d63-compound-morphology.md) §3b `-like`)
   and "the four other RecQ helicases." (Because C0084304's *atoms are multiword* — `recq helicase(s)`,
   `recq family of dna helicases` — the whole phrase can also ground directly, §6a; the two converge, §5.)
   **Resolve the unknown atom by grounding it, let the grammar compose the rest.**
2. **Ground to an existing concept** — retrieval matches the surface to an ontology entity → an **alias entry**
   whose `sem` is that concept (`method: RetrievalGrounded`, `grounded_to` set); today's mechanism, extended
   from abbreviations to OOV.
3. **Synthesize a type** — no entity, not decomposable → an LLM produces a provisional Σ-type from a
   *retrieved definition*, **kernel-gated** (`method: LlmSynthesized`, low trust until reviewed).

## 5. The concept is the target; composition is the means

Composition is how we **parse** a lexical void; the **encoding** must still land on the **ontology concept**
(`RecQ DNA helicase` → `C0084304`), or we've built a bespoke Σ-type disconnected from the knowledge graph —
which defeats the point of a taxonomy. So there are **two grounding moments**:

1. **Atom, pre-parse** — `recq → C0084304` (form-text-index over the seeded atoms, §6a), so the parse
   *composes* the phrase.
2. **Phrase, on the result** — recognize the composed NP as `C0084304` and encode it *as that concept*.

**Convergence invariant:** the **atomic-alias reading** (`RecQ DNA helicase → C0084304` directly) and the
**compositional reading** (`Σ(h:helicase). BelongsToFamily(h, RecQ)`) must land on the **same concept**. That
is a definitional equivalence `C0084304 ≡ RecQ-family helicase` — the same "many derivation paths, one
canonical encoding" invariant as the denominal `⟦X-E⟧ = ⟦E link X⟧`
([d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md)). UMLS doesn't *give* the definition
(atomic labels + relations), so **the pipeline establishes it by grounding the composed phrase, then caches
it** as augmentation (§6). The Σ-type is the **justification**; the concept is the **identity**; the encoding
uses the concept.

## 6. The pipeline as a lexicon-augmentation transducer

`DocumentEncoding` already exposes `glossary: Vec<AbbrDef>` ([pipeline.rs:52](../../kernel/src/dcg/pipeline.rs))
and `encode_with_layer` already returns the materialized doc-layer ([pipeline.rs:96](../../kernel/src/dcg/pipeline.rs)).
Generalize both so augmentation is **both an input and an output**:

```
encode(document, opts: &AugmentOptions, seed: Option<&LexiconAugmentation>) -> DocumentEncoding

DocumentEncoding { augmentation: LexiconAugmentation, sentences: … }   // `glossary` generalized
LexiconAugmentation {
    added:       Vec<LexicalBinding>,  // each wraps a PROPOSED lexicon:LexicalEntry + provenance (the harvest)
    missing_oov: Vec<Gap>,             // detected, unresolved → the fail-closed findings
}
AugmentOptions = DocumentOnly | LexiconBacked(LexiconProfile) | LlmBacked   // + combinations
```

- **`AugmentOptions`** = which sources generate entries: **DocumentOnly** (Schwartz-Hearst, deterministic),
  **LexiconBacked** scoped by a `LexiconProfile` (`resolve_lexicon_profile`,
  [lookup.rs:324](../../kernel/src/dcg/lookup.rs), resolves a profile to its ordered scope), **LlmBacked**
  (retrieval + synthesis).
- **`seed`** = the "initial augmentation made available" — and it *is* an input doc-layer chained on (aligning
  with `encode_with_layer`); the output `added` *is* the new layer's entries. So
  **`(document, seed, opts) → (encoding, augmentation')`**, and **`augmentation'.added` feeds the next
  document's `seed`** — corpus-glossary bootstrapping and the feedback cache, through the trait. The
  augmentation is the pipeline's memo of "surface → grounding" learned so far.

**Cache lifecycle** (§5 made concrete):
- *First encounter* — `recq` OOV (exact-index miss) → the form-`TextIndex` token-matches it to the seeded
  atoms `recq helicase(s)` → `C0084304` (§6a) → an alias binding `recq → C0084304` is added; the grammar then
  composes `RecQ DNA helicase`, and the composed span is recognized as `C0084304` too. Encoding = `C0084304`.
- *Next encounter (seeded)* — `recq` (and the MWE) seed directly as `C0084304`; fast path. The token retrieval
  was scaffolding to *reach* the concept the first time.

## 6a. The retrieval index — text-first

`LexiconBacked` grounds an OOV surface against the committed lexicon by **BM25 text retrieval**
(`core:TextIndex`), not embeddings. `form`/`description` are short strings, so a tokenized inverted index is
cheaper (no embedder, no eager-embedding sweep at UMLS scale) and more precise on symbols/terms than a vector
index; **vector is deferred** to a demonstrated paraphrase need text can't meet.

- **Primary — `core:TextIndex` over `lexicon:form`.** A concept's surface identity is its **atoms**, each
  already a `lexicon:form` entry (UMLS emits one per synonym). A token/BM25 match lands `recq` on the atom
  `RecQ Helicase(s)` → its concept — the exact `ValueIndex` misses this (`recq` ≠ `recq helicases`), which is
  *why* `recq` is OOV. **No schema change** — `form` is already there. This is the index that actually closes
  surface OOV.
- **Secondary — `core:TextIndex` over the concept's *full* `core:description`** (kept **on the concept**,
  normalized; `core:description_text_index`, homed in the core ontology, IMPLEMENTED `2026-07-05`). Adds recall only for a query term
  appearing in a *definition* but in *no atom*. The verb/adjective **converter fix** landed with it —
  `eigentt:Axiom` now recommends `core:description`, the ESL `axiom … desc: "…"` clause carries it, and
  `push_verb`/`push_adj` emit the synset gloss — so all POS carry a description (nouns/instances already did).
- **Disambiguation.** BM25 returns *ranked candidates*, often several (`recq` hits C0084304 *and* members
  like C1335609). The resolver **picks** — the top hit when its score margin is clear, else LLM-disambiguated
  against the document context — and the **kernel gates** the chosen alias. A low-margin tie it can't resolve
  fails closed to a `Gap` (§7), never a silent guess.
- **Not on the entry.** A per-entry description was considered and dropped: a *short* gloss (the preferred
  name) is redundant with the forms; the *full* definition belongs on the concept (normalized, not ×N
  synonym-entries). So `LexicalEntry` is unchanged; the two indexes target `lexicon:form` (per-entry, existing)
  and concept `core:description` (normalized).
- **Declaration — a decision, not settled here.** Two options: **(i) static** in the lexicon ontology beside
  `lexicon:form_index` (always-on, simplest — but a BM25 index over the full ~7.6M forms is a real, if
  moderate, build/storage cost paid on every load); **(ii) augmentation-declared**, added as a `core:TextIndex`
  Resource by `LexiconBacked` and **scoped by the `LexiconProfile`** (index only the domain subset, built +
  persisted on first grounding via `store_layer`, reused through the `seed` — the cache lifecycle). (ii) bounds
  cost to when/where grounding is needed; (i) wins if the always-on cost is acceptable. Either way the exact
  `ValueIndex` on `form` stays for parsing / `has_token`, and a `TextIndex` + `ValueIndex` on one property
  coexist (different kinds; one-per-kind multiplicity).

**Consequence for `recq` (verified `2026-07-05`, `db_backed_encoding::probe_recq_atoms_in_snapshot`):**
grounding needs **only the form-text-index — not the HGNC import**. Over the snapshot, the bare token `recq`
is OOV (`has_token=false`, 0 entries), but **every C0084304 atom is already a seeded `lexicon:form` entry** —
`recq helicase`, `recq helicases`, `helicase, recq`, `recq protein`, `recq family of dna helicases` all
resolve to `cat_n(C0084304)`, and `recq helicase-like` to a member `C1335609`. Only the *exact* `ValueIndex`
blocks `recq` (a token *inside* those atoms); a BM25 `TextIndex` over `lexicon:form` closes it. HGNC:1049 adds
the family's *member structure* — a separate enrichment, not the grounding path.

## 7. Disciplines (where this meets the epistemics)

- **LLM proposes, kernel gates.** A grounded entry's `grounded_to` concept must resolve to a *real committed*
  entity; a synthesized entry's `proposed` cat/sem must *type-check* before injection. The resolver only
  proposes — the anaphora / abbreviation proposers
  already follow this ([resolver_llm.rs](../../kernel/src/dcg/resolver_llm.rs), `AbbreviationProposer`). No
  fabricated CUIs (the reference-integrity rule).
- **Fail closed.** An OOV that can't be grounded becomes a `Gap` → a reported **finding** (honest document
  coverage), never a silent drop.
- **Faithfulness over convenience.** Prefer composition / grounding to an existing concept over synthesizing
  a fresh type; the concept is the canonical target, the composition its justification.

## 8. Code touchpoints — reused vs new

**Reuse:** `glossary_resources` (emission + alias model + category inheritance,
[glossary.rs](../../kernel/src/dcg/glossary.rs)); the doc-layer chaining + `encode_with_layer`; `has_token`
(OOV signal, `lookup.rs:570`); the `AbbreviationProposer` / `Proposer` trait pattern; `compound_kind`
(composition, `parser.rs:630`); `resolve_lexicon_profile` (`lookup.rs:324`).

**New:**

| piece | what |
|---|---|
| `LexicalBinding` (generalizes `AbbrDef`) | **wraps a proposed `lexicon:LexicalEntry`** (reuses the committed type) + `Provenance{long_form?, context, method, grounded_to?, confidence?}` |
| `Gap` | an unresolved OOV (`surface, context, tried`) — a finding, separate from a binding |
| `LexiconAugmentation{added: Vec<LexicalBinding>, missing_oov: Vec<Gap>}` | the transducer's exposed state (the harvest); generalizes `DocumentEncoding.glossary` |
| `AugmentOptions` | `DocumentOnly \| LexiconBacked(LexiconProfile) \| LlmBacked` on `encode` |
| OOV pre-pass | tokenize → `has_token` → OOV atoms → resolver |
| **`core:TextIndex` over `lexicon:form`** | BM25/token grounding index — the **primary** surface→concept path (closes `recq`, §6a); declared beside `form_index`; no schema change |
| `core:TextIndex` over concept `core:description` (`core:description_text_index`, homed in core) | **secondary** recall (definition mentions); IMPLEMENTED with the verb/adjective gloss **converter fix** (axiom `desc:` clause → `core:description`) so all POS carry a description; resolver filters hits by `is_a` (drop predicate axioms) |
| `OovResolver` (or a unified `LexicalBinder`) | queries the text index(es) (+ LLM for synthesis); produces bindings, kernel-gated |
| atom-decomposition | split an OOV span → known atoms + one unknown → resolve the atom |
| phrase-grounding | recognize a composed NP as an ontology concept + cache the alias |
| promotion filter | review `added` by `method`/`confidence` → promote into a committed `lexicon:Lexicon` (the harvest lifecycle) |

## 9. Roadmap

- **Phase 1 (deterministic core) — IMPLEMENTED (`2026-07-05`).** [`kernel/src/dcg/augment.rs`](../../kernel/src/dcg/augment.rs):
  `ResolutionMethod`/`Provenance`/`LexicalBinding{proposed, provenance}`/`Gap`/`LexiconAugmentation{added,
  supporting, missing_oov}`/`AugmentOptions`, and `augment_document_only` (the `DocumentOnly` transducer:
  abbreviation defs → grounded bindings with `DefinitionExtracted` provenance, wrapping the emitted
  `lexicon:LexicalEntry`; OOV pre-pass via `has_token` → `Gap`). `DocumentEncoding.glossary → augmentation`
  ([pipeline.rs](../../kernel/src/dcg/pipeline.rs)); `IngestedDocument.glossary → augmentation`
  (eigenius-reasoning). Tests: `document_only_augmentation_harvests_bindings_and_flags_oov` +
  `in_process_pipeline_encodes_a_document_end_to_end` (closed_class_determiners.rs); full suite green.
  **Deferred to Phase 2:** the `encode(opts, seed)` trait-signature change — `opts`/`seed` are only
  functional once `LexiconBacked` exists, so threading them now would be speculative API; `encode(&str)`
  stays (`DocumentOnly` implied) and the `AugmentOptions` enum is defined but not yet routed.
- **Phase 2 (retrieval, text-first) — resolver IMPLEMENTED + tested in-process (`2026-07-05`).**
  [`augment.rs`](../../kernel/src/dcg/augment.rs): `ground_via_form_index` (BM25 `run_text_search` over the
  active `core:TextIndex` on `lexicon:form` → hit entries → their `lexicon:sem` concepts, **summed per
  concept** = disambiguation → top concept + confidence) and `augment_lexicon_backed` (grounds each `Gap`
  via the form index → `RetrievalGrounded` binding aliasing the concept, reusing `abbreviation_resources`;
  un-grounded gaps stay `Gap` with `tried` recorded — fail-closed). Test:
  `lexicon_backed_augmentation_grounds_oov_via_the_form_text_index` — bare `recq` is OOV under the exact
  `ValueIndex` but grounds to the concept via the form `core:TextIndex` (the RecQ finding, mechanized).
  The whole chain builds on one shared storage, mirroring the production single backend — index discovery
  scans the per-storage triple index, so a form index declared in an ancestor layer is only visible to a
  child built on the same storage (`probe_recq_form_index_active_and_populated` isolates this: 2 active
  indexes, `recq` hits).
  - **(a) production form index — VERIFIED over the reseed (`2026-07-05`).** `lexicon:form_text_index :
    core:TextIndex` over `lexicon:form` (analyzer `en-stem-v1`) in
    [lexicon-ontology.esl](../../ontologies/lexicon/lexicon-ontology.esl), beside `form_index`. A full
    reseed materialized it (the fresh-index reindex path skips existing layers, so a declaration alone
    doesn't backfill — the seed does). Over the `wordnet-umls-2026-07-05` snapshot (~2.4M resources),
    `augment_lexicon_backed("recq …")` grounds bare `recq` → `urn:eigenius:umlscui:C0084304` (RecQ
    Helicases) — the RecQ finding mechanized over the real atoms, no HGNC import
    (`verify_grounding_indexes_over_snapshot`). Confidence is low (~0.10 = C0084304's share of summed BM25
    across every `recq`-bearing form in the full lexicon), but it is the top-summed concept.
  - **(b) pipeline threading — IMPLEMENTED (`2026-07-05`).** `InProcessPipeline::with_augment_options` +
    `encode_with_layer` dispatch (`LexiconBacked` → `augment_lexicon_backed`, else `augment_document_only`);
    `encode(&str)` unchanged (`DocumentOnly` default). [pipeline.rs](../../kernel/src/dcg/pipeline.rs).
  - **(c) secondary concept-`core:description` index — IMPLEMENTED (`2026-07-05`).**
    `core:description_text_index : core:TextIndex` over `core:description`. **Homed in the core
    ontology**, not the lexicon: an index is per-property, so it lives with the property it targets, and
    `core:description` is a universal core property (any resource may carry a description) — the same
    reasoning that puts `form_text_index` with `lexicon:form`. It therefore indexes *every*
    description-bearing resource chain-wide, not just lexicon concepts; that breadth is fine because
    **grounding eligibility is the resolver's job, not the index's** (see below). The converter fix:
    nouns/instances already carried the gloss as `core:description`; verbs/adjectives dropped it because
    they compile from `axiom` one-liners. A verb entry's `lexicon:sem` **is** its axiom, so the axiom is
    the sense's denotation and the gloss's home. Enabled by: `core:description` added to
    `eigentt:Axiom.recommends` ([eigentt-type-fragment.json](../../ontologies/eigentt/eigentt-type-fragment.json));
    an ESL grammar extension `axiom N : <stmt> [desc: "…"] [note: "…"]` (`desc:` → `core:description`) —
    [parser.rs](../../kernel/src/esl/parser.rs) `parse_axiom`, [compile.rs](../../kernel/src/esl/compile.rs)
    `compile_axiom`; `push_verb`/`push_adj` emit `desc: "<gloss>"`
    ([convert.rs](../../crates/eigenius-wordnet/src/convert.rs)). The reseed report confirms all 15578 verb +
    18156 adjective axioms carry the gloss.
  - **Resolver-side description grounding — IMPLEMENTED (`2026-07-05`).**
    [`ground_via_description_index`](../../kernel/src/dcg/augment.rs) is the secondary path in
    `augment_lexicon_backed`: form index first, then the concept-`core:description` index on a miss. A
    description hit **is** the concept (no entry→`sem` hop). Both grounding paths are POS-aware (below);
    the kernel felicity gate is the backstop on the minted alias. Vector deferred.
  - **POS-aware grounding — the (B) step — IMPLEMENTED (`2026-07-05`).** The resolver takes the OOV's
    expected category (`ExpectedCat {Nominal, Verb, Adjective}`) and keeps only concept hits whose kind
    matches it: nominal ⇒ a non-axiom class/instance; verb/adjective ⇒ a predicate `eigentt:Axiom` — on
    **both** the form and description paths (`is_a ∋ eigentt:Axiom` over the triple index). The mint then
    branches on the concept's kind: a class → the nominal `cat_n` alias (`abbreviation_resources`); an
    axiom → [`predicate_alias_resources`](../../kernel/src/dcg/augment.rs), which **clones a committed
    sibling entry's verb/adjective cat** (found via `scan_chain` over `lexicon:sem` = the axiom —
    resource-typed, so triple-indexed even after the persist String-collapse) rather than reconstructing
    the cat, so the converter's category stays the single source of truth.
    - **Where the category comes from: an *untrusted proposer*, not the parse.** The earlier note that it
      would come from "the parse's typed open-holes" was wrong — those holes are anaphora-only
      (`HoleKind::EntityRef`); an OOV yields `SentenceOutcome::Gap` (no parse, no typed hole). Instead a
      [`CategoryProposer`](../../kernel/src/dcg/augment.rs) proposes the POS from the OOV's sentence
      (carried on `Gap.context`), the same "propose → kernel gates" contract as the abbreviation/anaphora
      proposers: `NominalCategoryProposer` (deterministic default = the nominal-only (A) behaviour) and
      `AnthropicCategoryProposer` (`use-llm`). Installed on the pipeline via
      `InProcessPipeline::with_category_proposer`.
    - **Tests.** `…grounds_nominal_oov_to_class_not_axiom` and `…grounds_verb_oov_to_axiom_with_verb_cat`:
      the SAME OOV (`supercoils`) grounds to the class under a nominal proposer and to the verb axiom
      (minting the sibling's verb cat) under a verb proposer — the proposed POS, not ranking, selects the
      concept kind.
    - **Deferred refinements:** verb-vs-adjective disambiguation among axioms (both currently match a
      predicate OOV; sibling-clone gives the concept's own cat regardless); inflection-matching the cloned
      sibling to the OOV's surface (currently the first sibling by sorted IRI). (HGNC gene-group import —
      [[gene_family_lexicon_gap]] — is a separate *enrichment*, not the grounding path.)
- **Phase 3 (synthesis + cache)** — `LlmBacked` `LlmSynthesized` entries (kernel-gated); the promotion filter
  (by `method`/`confidence`) and the seed-in/added-out feedback cache across a corpus.

## 10. Cross-references

- **Metonymy caveat (the "RecQ binds DNA" case).** A Σ-type/π₁ *coercion* (a `FamilyMember(F) = Σ(p:Protein).
  BelongsTo(p,F)` reading coerced by `π₁` to `Protein`) is *not* in our checker today — coercion is
  subclass-lattice inclusion (Luo 2012, [check.rs:670](../../kernel/src/nbe/check.rs)), not Σ-projection —
  **and** it is partly moot: verbs are typed generically at `Entity`, so `bind(kind_of(RecQ_Family),
  kind_of(DNA))` type-checks today with no coercion. π₁ metonymy is future machinery gated on a decision to
  sharpen verb argument typing to specific classes.
- [d63-compound-morphology.md](d63-compound-morphology.md) §3b — `-like` shipped; `recq` deferred here.
- [[gene_family_lexicon_gap]] — the HGNC gene-group import for the family's *member structure*; an enrichment,
  **not** the `recq` grounding path (§6a: the UMLS C0084304 atoms already ground it via the form-text-index).
- [d61-llm-based-encoding-methodology.md](d61-llm-based-encoding-methodology.md) — the grounding-discovery
  discipline the retrieval/LLM resolver instantiates (retrieve from the kernel first, gate, map back).
