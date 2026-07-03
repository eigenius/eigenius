# D63 — LLM-assisted document preprocessing + abbreviation injection (scope)

**Status:** scoping (design, not yet built). Motivated by `d63-cnl-v2-parsing-diagnosis.md`: the #1
CNL-v2 parsing lever (~8 of 19 grammar-gaps) is **bare domain abbreviations used as argument NPs**
(`MSI` as subject/object), which is a *document-local abbreviation-definition* problem, not grammar.

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
│   • the injected abbreviation named-individuals make bare `MSI` a      │
│     cat_np argument NP → the ~8 abbreviation gaps parse                │
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

---

## 2. The document context — one typed family, five members

All five members are the same shape: a **document-scoped typed resource** carrying a surface form + a
binding. They live in a per-document layer and are consumed at Stage B (parse) and Stage C (resolve).

| member | surface | binds to | consumed | ontology |
|---|---|---|---|---|
| **Abbreviation** (this note) | `MSI` | a named individual of the long-form concept | Stage B (lexeme) + C | new `document:Abbreviation` |
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
`MSI` resolves to the doc-local named individual and the general/domain `cat_n`/junk senses (the
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
"microsatellite instability" → `umlscui:C0920269`, T049). Two outcomes:
- **Hit** — bind the abbreviation's named individual as an *instance of the found concept class*
  (`<doc:msi> : umlscui:C0920269`). This grounds `MSI` in existing kernel knowledge.
- **Miss** — mint a fresh document-local class for the long form (`<doc:long_form_class>`) and the
  individual under it. Recorded as a Declared binding (no false grounding).

> Note the modeling subtlety (from the diagnosis §4a): UMLS types Microsatellite Instability as a
> *dysfunction class* (`cat_n`), which is why bare `MSI` gaps today. We are **not** reclassifying it
> globally; we inject a **document-local named individual** that refers to it — faithful to how the
> paper *uses* MSI (a referring entity), without touching the UMLS import.

### 3c. Typed model + injection (Stage A commit → Stage B read)
Per abbreviation, the doc layer gets **two resources**, exactly mirroring how the UMLS importer emits a
named individual (`crates/eigenius-umls/src/convert.rs:162–172`):
1. the **named individual** — `resource doc:msi : umlscui:C0920269 { … }` (an instance);
2. a **`lexicon:LexicalEntry`** — `form = "MSI"`, `cat = cat_np(umlscui:C0920269, sg)`,
   `sem = doc:msi`, `sem_type = umlscui:C0920269`, `in_lexicon = doc:<id>`, grade Declared.

The doc layer is **committed on a per-document branch `doc:<id>`** (kernel-gated at commit — 3d). Stage B
then calls `ParseSentence` with `branch = "doc:<id>"` (the RPC already supports this,
`ParseSentenceRequest.branch`/`at_layer`, `proto/eigenius.proto:441`). The parser's `LexicalIndex` is
built over base-lexicon + doc-layer, so `MSI` now seeds a `cat_np` named individual and parses as a
bare argument — no parser/grammar change. This is the "load lexica as chained sub-layers" pattern
(D63/D65), just document-scoped and tiny.

### 3d. The kernel gate (fail-closed)
Committing the doc layer runs the extracted bindings through the felicity gate: each named individual
must type-check as an instance of its concept class, and each lexical entry's `cat`/`sem`/`sem_type`
must be kernel-valid (the same gate every lexeme passes). A mis-extracted abbreviation (e.g. binding to
a non-existent or ill-typed concept) is **rejected at commit**, surfaced as a finding — never silently
used. This is what makes the untrusted LLM extraction safe.

---

## 4. Where the pieces live

| piece | location | notes |
|---|---|---|
| extraction + doc-context build (Stage A) | **new orchestration component** `orchestration/src/components/extract_document_structure.ts` | sibling of `complete_json.ts` / `complete_text.ts`; deterministic Schwartz-Hearst + LLM fallback; emits the doc layer |
| doc-layer commit | existing commit/branch machinery | per-document branch `doc:<id>`; kernel-gated |
| parse-time consumption (Stage B) | **no kernel change** | `ParseSentence(branch="doc:<id>")` reads the injected named individuals |
| typed model — **glossary** | **reuse `lexicon:Lexicon` + `lexicon:LexicalEntry`** (a doc-scoped lexicon layer) + optional provenance slots (`source ∈ {abbrev, glossary, inline}`, `long_form`) | NOT a new `document:Abbreviation` class — a glossary entry *is* a lexicon addition (§2a) |
| typed model — **reference structures** | **new `ontologies/document/…`** (`document:FigureRef`, `TableRef`, `Footnote`) + **reuse `reference:Citation`** | the non-lexicon members (consumed post-parse); small ontology, bootstrap-gated (reseed) |
| post-parse (Stage C) | D64 anaphora + reference binding | consumes the doc context |

---

## 5. Phasing

- **Phase 1 (the #1 lever):** abbreviation extraction (deterministic + LLM) → doc-layer injection →
  parse. Closes the ~8 abbreviation gaps. Deliverables: `document:Abbreviation` class, the extraction
  component, the doc-branch commit, a re-measurement.
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
2. **Doc-layer lifecycle** — a committed per-document branch `doc:<id>` (proposed, idiomatic, kernel-
   gated) vs an inline abbreviation table on `ParseSentenceRequest` (less idiomatic, avoids a commit
   per document). The committed-branch form reuses everything and keeps the fail-closed gate; the inline
   form is lighter for throwaway parses. Recommend committed-branch, with inline as a possible fast path.
3. **Grounding miss policy** — mint a fresh document-local class (proposed) vs defer/flag. Fresh class
   keeps the parse working (Declared), and the missing grounding is itself a recordable finding.
4. **Ontology home** — a new `document:` namespace for the doc-structure family vs folding into an
   existing one; and whether `Abbreviation` should carry the long-form *string* or only the bound
   concept (recommend both: the string is the extraction provenance).
5. **Scope of the LLM rewrite (Phase 3)** — out of scope here; noted so the pipeline shape accommodates
   it (Stage A produces the body-sentence form Stage B consumes).
6. **Shadow vs. add for glossary terms (§2a)** — *add* injects the doc-local `cat_np` entry alongside
   the base senses (simple, closes lever #1, crowding unchanged); *shadow* also suppresses the base
   lexicon's competing senses for the term (also cuts lever #2 crowding), but leans on the layer
   shadowing / `scope`-precedence semantics (`is_shadowed`). Left open: start-with-add-then-measure vs
   commit-to-shadow is a Phase-1 build decision, deferred until the glossary layer exists to measure on.
```
