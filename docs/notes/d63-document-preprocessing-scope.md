# D63 — LLM-assisted document preprocessing + abbreviation injection (scope)

**Status:** partially built — the three-stage pipeline is realized in-process as the `DocumentPipeline`
trait (§1a). Stage A (document glossary) + Stage B (parse) + the Stage C discourse loop are built and
tested in Rust (`InProcessPipeline`); remaining are the graded-proposition output (reshape Phase C), the
full-lexicon run, the served impl, and the reference-structure family (figures/tables/citations).
Motivated by `d63-cnl-v2-parsing-diagnosis.md`: the #1 CNL-v2 parsing lever (~8 of 19 grammar-gaps) is
**bare domain abbreviations used as argument NPs** (`MSI` as subject/object), which is a *document-local
abbreviation-definition* problem, not grammar.

This note scopes the **abbreviation lever** as the first concrete piece of a broader **document-
preprocessing stage** that builds document-scoped lookup structures (abbreviations, tables, figures,
footnotes, references) feeding parse-time injection and post-parse resolution.

---

## 1. The pipeline shape (three stages)

```
raw document text
     │
     ▼
┌──────────────────────────────────────────────────────────────────────┐
│ STAGE A — PREPROCESSING (LLM-assisted; UNTRUSTED proposer)            │
│   • extract document-scoped lookup structures:                        │
│       abbreviations   MSI → "microsatellite instability" (→ concept)  │
│       tables/figures  "Fig. 1c", "Table 1" → object refs              │
│       footnotes/refs  "[1]", superscripts → reference:Citation        │
│   • (optional) controlled-English rewrite of body sentences           │
│   • segment into body sentences                                       │
│   ⇒ a typed "document context" committed as a per-document layer      │
│     (kernel-GATED — the felicity oracle admits/rejects each binding)  │
└──────────────────────────────────────────────────────────────────────┘
     │  (document context = committed doc-scoped layer on branch `doc:<id>`)
     ▼
┌──────────────────────────────────────────────────────────────────────┐
│ STAGE B — PARSE (per body sentence; TRUSTED kernel)                   │
│   • ParseSentence over base-lexicon + the doc layer (branch `doc:<id>`)│
│   • the injected abbreviation alias entries make bare `MSI` an         │
│     argument NP (mass→generic / individual→ref) → the ~8 gaps parse    │
└──────────────────────────────────────────────────────────────────────┘
     │  (typed trees, some OPEN awaiting resolution)
     ▼
┌──────────────────────────────────────────────────────────────────────┐
│ STAGE C — POST-PARSE                                                  │
│   • anaphora / referent resolution (D64), using the doc context       │
│   • bind figure/table/reference mentions to their objects             │
└──────────────────────────────────────────────────────────────────────┘
```

The discipline is the standard Eigenius stance: **the LLM only proposes** (Stage A extraction is
untrusted); **the kernel is the oracle** — every extracted binding is committed as a resource through
the felicity gate, so a mis-extracted abbreviation fails closed rather than silently corrupting the
parse. This mirrors the existing sense-reranker / anaphora-proposer pattern (`allms`, D64).

### 1a. Realization — the `DocumentPipeline` trait (the anchor)

The three stages are realized in Rust as **one contract**,
[`dcg::pipeline::DocumentPipeline`](../../kernel/src/dcg/pipeline.rs) —
`encode(&self, document: &str) -> DocumentEncoding`:

- **Input** — the raw document text (upstream of tokenization, so Stage-A extraction still sees the
  parenthetical definitions the tokenizer later strips, §3a).
- **Output** — `DocumentEncoding { glossary: Vec<AbbrDef>, sentences: Vec<SentenceEncoding> }`, one
  `SentenceEncoding { text, outcome }` per body sentence. The `outcome` is a **`SentenceOutcome`**:
  `Encoded(Item)` / `Ambiguous(Vec<Item>)` / `Open(OpenParse)` / `Gap` — the classified per-sentence
  result. Fail-closed: an un-encodable sentence is `Open`/`Gap`, never a wrong closed parse.

This trait is the spine for the remaining work, and it separates that work into **two orthogonal axes**
— which is also, exactly, what the two "Phase" numberings in play count (they are *not* the same axis):

1. **What the pipeline produces** — the output contract *deepens*; **this note's** Phase 1→2→3. A closed
   sentence is `SentenceOutcome::Encoded(Item)` (a typed tree) today; reshape Phase C makes it a **graded
   proposition** (`epistemic:declared`, a citation warrant climbing the grade — §3c †). The
   document-context family (§2) is the same axis: `glossary` is member 1 of `DocumentEncoding`;
   figures/tables/footnotes/citations join it as further members.
2. **How the pipeline runs** — the *impls multiply* while the contract holds; **`d63-next-steps.md`'s**
   Phase 1→2. The LLM steps sit behind the proposer traits (`AbbreviationProposer`, `Proposer`), so an
   impl is just a choice of proposers + where the doc layer is built:
   - **`InProcessPipeline`** — every stage in Rust, the doc glossary chained on an **in-memory** layer,
     the LLM steps via `--features allms`. Built + tested (`in_process_pipeline_encodes_a_document_end_to_end`).
   - a **persistent** in-process impl (a `with_storage` constructor) for the full-lexicon run — an
     in-memory overlay on the 7.6M snapshot OOMs (§7-2).
   - a **served** impl — the same trait, proposers backed by orchestrator RPCs.

So the served path is **not a rewrite**: it is a second `impl DocumentPipeline`, and the trait is the
seam that guarantees the swap changes nothing the caller reads.

---

## 2. The document context — one typed family, five members

All five members are the same shape: a **document-scoped typed resource** carrying a surface form + a
binding. They live in a per-document layer and are consumed at Stage B (parse) and Stage C (resolve).

| member | surface | binds to | consumed | ontology |
|---|---|---|---|---|
| **Abbreviation** (this note) | `MSI` | an alias `LexicalEntry` of the long-form concept | Stage B (lexeme) + C | new `document:Abbreviation` |
| Figure ref | `Fig. 1c` | a figure object | B/C | new `document:FigureRef` |
| Table ref | `Table 1` | a table object | B/C | new `document:TableRef` |
| Footnote | superscript | a footnote object | C | new `document:Footnote` |
| Reference/citation | `[1]` | a `reference:Reference` (global work) | C | **reuse `reference:Citation`** (in-text use vs global work — already modeled, `ontologies/reference/reference.esl`) |

Designing the abbreviation member first, but with the family in mind, so the extraction component and
the doc-layer commit generalize (Phase 2 adds the others without re-architecting).

### 2a. Abbreviations are lexicon additions → the *document glossary*

The abbreviation member is not a bespoke construct: its output is a `lexicon:LexicalEntry` (§3c), i.e.
a **lexicon addition**. So the right generalization is a **document glossary** — a document-scoped
lexicon layer — populated from several extraction *sources*, all landing in the same doc layer:

1. **Abbreviation definitions** — `Long Form (ABBR)` (Schwartz-Hearst); the Phase-1 focus.
2. **An explicit glossary / definitions section** — if the document supplies one (a "Definitions"
   table, a glossary list, a nomenclature box). Directly a set of `term → definition/binding` entries.
3. **Inline term definitions** — "we call X…", "X, defined here as…", "X refers to…".

This slots into the platform's existing **lexicon hierarchy** (general → domain → document):

```
lexicon:general   WordNet          (the common-noun / lexical core)
lexicon:domain    UMLS, NCBI, …     (injected as sibling importers — see the domain-lexicon track)
lexicon:document  the doc glossary  (this note — most specific, HIGHEST precedence)
```

The document glossary is just the **most-specific, highest-precedence** lexicon layer. It reuses the
same machinery (a `lexicon:Lexicon` layer; `LexicalEntry` resources; the `scope`/`profile` precedence
already on `ParseSentenceRequest`), so "add a document glossary" is not new mechanism — it is a
document-scoped instance of the lexicon-injection track, populated by Stage A instead of a bulk import.

**Precedence unlocks a second win (mitigates lever #2, beam-crowding).** Because the doc glossary
ranks *first* in `scope` order, a defined term can **shadow** the base lexicon's competing senses:
`MSI` resolves to the doc-local alias entry and the general/domain `cat_n`/junk senses (the
Microsatellite-Instability dysfunction class *and* the "AML table" collision, diagnosis §4a) can be
de-prioritized or dropped for that document. That directly reduces the sense-crowding that produced the
beam artifacts (diagnosis §3d) — so a document glossary that *pins* the sense of its defined terms
addresses **both** lever #1 (bare abbreviation as argument) **and** part of lever #2 (crowding). This
is a design goal for the injection, not an accident: prefer **shadow** over **add** for glossary terms.

---

## 3. Abbreviation lever — concrete design (Phase 1, the #1)

### 3a. Extraction (Stage A) — deterministic first, LLM for the tail
1. **Deterministic `Long Form (ABBR)` pattern** — the Schwartz-Hearst algorithm (2003) is the standard,
   high-precision extractor for `microsatellite instability (MSI)` / `MSI (microsatellite instability)`.
   Run it *first* (grounding's retrieve/deterministic-first discipline); it covers the common case with
   no LLM.
2. **LLM fallback** — for definitions the pattern misses (abbreviations introduced without a
   parenthetical, or defined across a clause). Untrusted proposer; output is validated in 3d.
   *(User-sanctioned: applying an LLM to the whole document text for extraction is acceptable here.)*

**Critical interaction with `strip_bracketed_asides`** (`lookup.rs:140–148`): the tokenizer drops
`(MSI)` as a gloss. Extraction MUST run on the **raw text, before tokenization** (it is a document-level
preprocessing pass, upstream of the parser) — so the binding is captured even though the parenthetical
is later stripped from the body sentence. No change to `strip_bracketed_asides` is required if
extraction is upstream; the definitional paren can still be dropped from the body sentence once the
binding exists.

### 3b. Grounding the long form (retrieve-first, D43)
For each `ABBR → long form`, resolve the long form to a concept **already in the kernel** before minting
anything (the `grounding` discipline): probe the lexicon/value-index for the long form (e.g.
"microsatellite instability" → `umlscui:C0920269`). Two outcomes:
- **Hit** — the abbreviation is an **alias** of the grounded concept and inherits its category (§3c).
- **Miss** — mint a fresh document-local class for the long form (`doc:class_<abbr> : lexicon:Entity`)
  and alias to it. Recorded as a Declared binding (no false grounding).

> Modeling subtlety, corrected (was: "inject a named individual"). The NP-vs-N fork is **denotational,
> not syntactic** (witnessed, `abbreviation_np_vs_n…` → `abbreviation_emission_keys_on_ontological_kind`):
> in the corpus the *same* abbreviation is both a bare identity ("MSI contributes to cancers") and a
> classifier ("these MSI cell lines"), and both parse regardless of category — the named-entity compound
> rule bridges the second. What matters is the **denotation**. Minting a named individual for every
> abbreviation was a wedge: it reified a *phenomenon* (MSI) as a singleton instance and made "MSI cell
> lines" mean `compound(x, ni_msi)` ("related to the one thing named MSI") instead of the correct
> `compound_kind(x, MSI)` ("microsatellite-unstable"). We do **not** reclassify anything globally; the
> abbreviation carries the grounded concept's *own* category — the alias model the UMLS importer already
> defers bare-argument abbreviations to (`convert.rs:149–157`).

### 3c. Typed model + injection (Stage A commit → Stage B read) — BUILT (alias model)
Per abbreviation the doc layer gets **one resource** — a `lexicon:LexicalEntry` aliasing the grounded
concept, its category keyed on the concept's **ontological kind** (D62 named-individual typing) and, for
common nouns, the long form's **head-noun countability**:

| grounded concept | `cat` | `sem` | bare argument | prenominal modifier |
|---|---|---|---|---|
| **mass phenomenon** (MSI = "microsatellite instability", head `instability` mass) | `cat_n(C, mass)` | the class `C` | ✓ open — deferred generic over the kind † | `compound_kind(x, C)` |
| **count common noun** (CL = "cell line") | `cat_n(C, num_any)` | the class `C` | needs a determiner (no bare) | `compound_kind(x, C)` |
| **named individual** (WRN, an HGNC gene) | `cat_np(sty, sg)` | the SAME instance | ✓ closed entity reference | `compound(x, instance)` |

`sem_type = ⟦cat⟧` (`denote_cat`) by construction, so the felicity gate's `type_eq` holds. Mass vs count
is inherited from the long form's head noun via the general countability lexicon (`form_is_mass`) — the
mechanism the UMLS importer points to, not a per-acronym shim. **No parser/grammar change**: the `mass`
number, the bare-mass shift (`bare_mass_nps`), and `compound_kind` already exist (D62 CNL).

> **† The mass-row `bare argument` reading is under revision.** "✓ open — deferred generic" is the
> *current* built behaviour: a bare mass subject (*MSI contributes to cancers*) comes back OPEN, carrying
> a deferred-quantifier hole. Examining that outcome is what prompted
> [d63-kind-predication-reshape.md](d63-kind-predication-reshape.md): we concluded a generic is a
> *complete* proposition — a **closed** kind-predication `contribute_to(kind_of(MSI), …)`, graded
> `epistemic:declared` — and that the warrant (literature citation / observation / derivation) belongs on
> the **grade**, not a parser hole. The emitter contract in this table (`cat_n(C, mass)`) is **unchanged**;
> only the grammar's handling of that entry *as a subject* changes (open → closed). Until the reshape
> lands, this cell and the `open generic` test witness below describe the real, current behaviour.

**Emission is programmatic, not ESL text.** The load path takes structured resources
(`LoadRequest.resources` = CBOR/Eigon-JSON, `proto/eigenius.proto:252`), so the entry is built **directly
as an in-memory [`Resource`]** by [`dcg::glossary::abbreviation_resources`](../../kernel/src/dcg/glossary.rs):
`Resource::new` + `Resource::set`, the `cat` built as an `Exp` and encoded via `encode_type` (the same D47
encoding ESL emits). Witnessed by `abbreviation_injection_recovers_bare_argument` (bare mass `wsi` OOV →
recovers as the open generic) and `abbreviation_emission_keys_on_ontological_kind` (the three-way table
above) — kernel tests.

The doc layer is **committed on a per-document branch `doc:<id>`** (kernel-gated at commit — 3d, and
required to be persistent per §7-2). Stage B then calls `ParseSentence` with `branch = "doc:<id>"` (the
RPC already supports this, `ParseSentenceRequest.branch`/`at_layer`, `proto/eigenius.proto:441`). The
parser's `LexicalIndex` is built over base-lexicon + doc-layer. This is the "load lexica as chained
sub-layers" pattern (D63/D65), just document-scoped and tiny.

### 3d. The kernel gate (fail-closed)
Committing the doc layer runs the extracted bindings through the felicity gate: each alias entry's
`cat`/`sem`/`sem_type` must be kernel-valid (the same gate every lexeme passes). A mis-extracted
abbreviation (binding to a non-existent or ill-typed concept) is **rejected at commit**, surfaced as a
finding — never silently used. This is what makes the untrusted LLM extraction safe.

---

## 4. Where the pieces live

| piece | location | notes |
|---|---|---|
| extraction + doc-context build (Stage A) | **new orchestration component** `orchestration/src/components/extract_document_structure.ts` | sibling of `complete_json.ts` / `complete_text.ts`; deterministic Schwartz-Hearst + LLM fallback; emits the doc layer |
| doc-layer commit | existing commit/branch machinery | per-document branch `doc:<id>`; kernel-gated |
| parse-time consumption (Stage B) | **no kernel change** | `ParseSentence(branch="doc:<id>")` reads the injected alias entries |
| typed model — **glossary** | **reuse `lexicon:Lexicon` + `lexicon:LexicalEntry`** (a doc-scoped lexicon layer) + optional provenance slots (`source ∈ {abbrev, glossary, inline}`, `long_form`) | NOT a new `document:Abbreviation` class — a glossary entry *is* a lexicon addition (§2a) |
| typed model — **reference structures** | **new `ontologies/document/…`** (`document:FigureRef`, `TableRef`, `Footnote`) + **reuse `reference:Citation`** | the non-lexicon members (consumed post-parse); small ontology, bootstrap-gated (reseed) |
| post-parse (Stage C) | D64 anaphora + reference binding | consumes the doc context |

**In-process vs served (the same trait, two impls — §1a).** This table is the **served** decomposition:
an orchestration component emits a committed doc-branch and `ParseSentence` reads it. The **in-process**
realization, `InProcessPipeline::encode`, collapses Stage-A extraction + the doc layer + Stage-B
consumption into one call over an **in-memory** doc layer — no orchestration component, no branch commit.
Both satisfy `DocumentPipeline`; the served path swaps the proposer impls (deterministic / `allms` → RPC)
and the doc-layer home (in-memory → committed branch), not the contract.

---

## 5. Phasing

These phases are the **output/coverage axis** (§1a, axis 1) — what the pipeline *extracts and produces*,
widening from abbreviations outward. The *run* axis (in-process → served) is tracked in
`d63-next-steps.md`. Both are impls/extensions of the one `DocumentPipeline` contract, not separate
pipelines.

- **Phase 1 (the #1 lever) — BUILT in-process:** abbreviation extraction (deterministic + LLM) →
  doc-glossary injection → parse. Closes the ~8 abbreviation gaps; realized as Stage A of
  `InProcessPipeline` (§1a). The deliverable is the **document glossary** — a *lexicon addition* (§2a),
  **not** a new `document:Abbreviation` class — built directly as resources by `dcg::glossary`; plus the
  re-measurement (full-UMLS: bare `MSI` GAP → CLOSED as `kind_of(C0920269)`).
- **Phase 2:** the rest of the document-context family — figures/tables (`FigureRef`/`TableRef`),
  footnotes, references (`reference:Citation`). Same extraction component, same doc layer.
- **Phase 3:** controlled-English rewrite as a preprocessing sub-step (the CNL-v2 was hand-authored;
  an LLM rewrite step would generate the body-sentence form the parser consumes — a separate large
  effort, and orthogonal to abbreviation injection).

---

## 6. Verification

1. **Phase-1 litmus (Derived):** re-run `scripts/measure-parse-rate.sh` on the **original** page
   (`--page original`, which *contains* the `microsatellite instability (MSI)` definition) with the
   abbreviation doc-layer injected — the ~8 bare-`MSI`-argument gaps must move to parsed (open/closed),
   with no regression on the rest. Also re-run CNL-v2 (which dropped the definition) to confirm the
   pass is a no-op there (nothing to extract) — isolating the fix to real abbreviation definitions.
2. **Fail-closed check:** a planted bad binding (ABBR → ill-typed concept) is rejected at the doc-layer
   commit, surfaced as a finding.
3. **Grounding check:** `MSI` binds to `umlscui:C0920269` (retrieve-first hit), not a fresh class.

---

## 7. Open decisions

1. **Extraction locus** — deterministic-first with LLM fallback (proposed), vs LLM-only. Schwartz-Hearst
   gives high precision for the parenthetical case with zero LLM cost; the LLM earns its keep only on
   the non-parenthetical tail. Recommend deterministic-first.
2. **Doc-layer lifecycle — SETTLED to committed-branch (witnessed constraint).** A committed
   per-document branch `doc:<id>` vs an inline abbreviation table on `ParseSentenceRequest`. The Phase-1
   witness surfaced a hard requirement: an **in-memory doc-layer overlaid on the persistent lexicon
   OOMs** — the lazy value-index doesn't resolve over the mixed in-memory/persistent chain, so
   `LexicalIndex::build` falls back to `scan_eager`'s full-chain `iter_all_resources` over the 7.6M
   snapshot (the "build a layer on the storage it's persisted to" invariant). So the doc-layer **must be
   committed to the served store** (a persistent branch), where its `LexicalEntry` value-index entries
   populate in `store_layer` and the lazy path resolves. Inline-on-the-RPC would need the same
   persistent backing to avoid the eager scan, so it is not a lighter path — committed-branch it is.
   (The lever itself is proven on the in-memory *demo* bootstrap, where the whole chain is small and the
   eager scan is fine: `kernel/tests/closed_class_determiners.rs::abbreviation_injection_recovers_bare_argument`.)
3. **Grounding miss policy** — mint a fresh document-local class (proposed) vs defer/flag. Fresh class
   keeps the parse working (Declared), and the missing grounding is itself a recordable finding.
4. **Ontology home** — a new `document:` namespace for the doc-structure family vs folding into an
   existing one; and whether `Abbreviation` should carry the long-form *string* or only the bound
   concept (recommend both: the string is the extraction provenance).
5. **Scope of the LLM rewrite (Phase 3)** — out of scope here; noted so the pipeline shape accommodates
   it (Stage A produces the body-sentence form Stage B consumes).
6. **Shadow vs. add for glossary terms (§2a)** — *add* injects the doc-local alias entry alongside
   the base senses (simple, closes lever #1, crowding unchanged); *shadow* also suppresses the base
   lexicon's competing senses for the term (also cuts lever #2 crowding), but leans on the layer
   shadowing / `scope`-precedence semantics (`is_shadowed`). Left open: start-with-add-then-measure vs
   commit-to-shadow is a Phase-1 build decision, deferred until the glossary layer exists to measure on.
```
