# D62/D63 — implementation plan: closing the encoding-pipeline gaps

*Working plan. Derived from the prototype findings
(`d62-encoding-prototype-findings.md`): on real WRN-paper prose the gating blockers are
**S0 tokenization + closed-class/grammar coverage of ordinary English**, not domain
vocabulary (the three lexica already cover ~all content words) and not disambiguation
(moot until sentences parse). Each phase is independently shippable and **measured against
the cleaned WRN first page** via the prototype ratchet (Phase 0). Spans D63 (grammar +
lexicon) and D62 (driver).*

> **Output contract** — what the pipeline returns (graded propositions + typed obligations) and
> how the wrapping institution is shaped to it: `docs/notes/d62-encoding-output-contract.md`.
> Adverb semantics decision: `docs/notes/d62-adverb-semantics-decision.md`.

## The ratchet — how every phase is measured

**Phase 0 — make the prototype a standing benchmark.** Promote
`crates/eigenius-wordnet/tests/encoding_prototype.rs::prototype_over_wrn_first_page` to the
gap metric: feed the cleaned page, emit the outcome distribution (encoded / ambiguous /
missing-lexeme / grammar-gap) + the OOV categories (function-word / tokenization / domain /
novel). Record the baseline (**today: 0/47 encoded, all missing**). Every slice below states
its target delta on these numbers. *Verify:* the metric prints stably; baseline recorded.

Resources confirmed available: `references/openccg/grammars/core-en` (det/conj/auxv/adv
category modules — the closed-class category source), WordNet+UMLS+NCBI emitted lexica (content
coverage), the Morphy port (inflection). None vendored; `references/` stays gitignored.

---

## Phase 1 — S0 tokenization & segmentation (text-only)

**Goal.** Turn the cleaned page into clean sentence units + clean tokens, so the parser sees
well-formed input. Fixes the over-split (4 paragraphs → 47 units) and the junk tokens.

**Build** (prototype-first in the test's `segment`/tokenize, then promote to a `dcg`
preprocessing module):
- **Tokenizer:** split em-dashes (`—`/`–`), handle parens/slashes (`poly(ADP-ribose)`,
  `and/or`), a hyphenated-compound policy (keep `double-stranded` as a unit but allow
  fallback to parts), and **route non-prose out**: numbers/stats (`10−13`, `0.56`),
  figure-refs (`Fig. 1a`), citation markers → tagged out, not lexemes (the §7.1 routing,
  pulled early for stats/refs).
- **Segmenter:** abbreviation- and decimal-aware sentence splitting (don't break on `Fig.`,
  `0.56`, `et al.`, `e.g.`).

**Verify (ratchet):** the page segments into ~clean sentences (not 47); OOV no longer
contains `10−13 / 1a / n / p / mlh`; tokenization junk → 0.

**Integration (after verification — the deliverable):** S0 lands in the **DCG engine**, not
test-local. A `kernel/src/dcg/segment.rs` module (document → units, with non-prose routing)
plus an enhanced tokenizer (extend/replace `lookup.rs::tokenize`), consumed by the parser /
`ParseSentence` front-end. The prototype then calls the engine's S0 (its own `segment` is
retired). Sequence: prototype the approach → verify on the WRN page → promote to the module
with its own unit tests → the prototype + RPC use it.

**Deps:** none. **Risk:** low. High value (unblocks everything downstream).

---

## Phase 2 — closed-class lexicon + grammar coverage (harvest `core-en`)

**Goal.** Cover the function words + the constructions they license, so real sentences parse
structurally. This is the largest phase; sub-slice it. Harvest categories from `core-en`'s
`det.xsl`/`conj.xsl`/`auxv.xsl` and re-express as `lexicon:LexicalEntry` (our `cat_*`/EigenTT
form) in `ontologies/lexicon/closed-class.esl` + grammar rules in `kernel/src/dcg`.

- **2a — determiners, demonstratives, possessives (lexical; existing machinery).** Add `the`
  (the biggest single gap), `this/that/these/those`, `its/their`. Reuses the determiner
  category we already have for `a/every/no/some`. *Target:* simple NPs with `the` parse.
- **2b — coordination `and`/`or` (grammar; the critical path).** The coordination rule
  already exists in the parser (D63 §8.4: `coord_op` + `coordinate_sem`/`coordinate_np`,
  polymorphic over `Cat`, so *not* lexical entries — the felicity gate can't type a
  category-polymorphic entry). The actual 2b work was making the **missing-lexeme signal**
  honest about it: `has_token` only checked lexical entries, so it reported `and`/`or` as
  OOV and the pipeline would route them to lexical recovery. Fixed via a shared
  `coord_connective()` consulted by both `coord_op` and `has_token`. Contrastive `but` is
  **not** here — see 2d. *Done.*
- **2c — modals + auxiliaries.** Added `will`/`would`/`should` to the existing modal
  machinery (`can/could/may/might/must`); same base-VP-selecting category, which matches
  core-en's `Modal` family `(S[dcl]\NP)/(S[base]\NP)` exactly. Per `auxv.xsl` each modal
  keeps its own stem-named operator (it never collapses to ◇/□), and per the steer modals
  are carried **opaquely** — `will`/`would`/`should` each get a distinct opaque
  `logic:Will`/`Would`/`Should` axiom (future / conditional / weak-deontic — not ◇/□),
  meaning supplied on the reasoning side by the justification-logic institution. *Done.*
  (Perfect/passive aux already exist; `shall` + epistemic→grade refinement remain follow-ons.)
- **2d — subordinators + sentence connectives.** Expert-revised — see
  [d62-subordinator-design-findings.md](d62-subordinator-design-findings.md). The uniform
  "opaque binary" plan was wrong. **Done:** `if` → native implication `p → q` (not opaque);
  `but` → `logic:And` (verified adequate against every WRN `but` — all "X but (not) Y", plain
  conjunction; contrast rhetorical, carried by explicit negation; contrast-preserving
  `logic:But`/kernel-transparency are the documented upgrade, GH #95). **Gated:**
  `because`/`although`/`while` → a **factive dependent** signature `Π(p q:Prop) → p → q →
  Prop` (presupposition as a felicity/proof-obligation), which needs the **open-parse engine
  extension** (D62 §11.5 item 7), not a fork decision. `however`/`thus` → anaphoric, fold into
  2e/D64.
- **2e — pronouns (`we`, `it`, `they`).** Reference grammar: core-en `ProNP` *is* an NP
  (= a proper name categorially) introducing a **discourse referent**; binding is anaphora
  resolution (D64), not grammar (`np.xsl:156`, `dict.xsl:70-115`). So a referential pronoun's
  sem is a **free `Entity` referent variable** → an **open parse** (D64 hole), *the same
  open-parse capability the factive subordinators need* (D62 §11.5 item 7), differing only in
  hole type/resolver (`Entity`/D64 vs. proof/grounding). `it`/`they`/`its`/`their` therefore
  need that capability; only deictic `we` (and possibly `however`/`thus`) may admit a closed
  first cut. *Target:* the unified open-parse/typed-hole mechanism, then pronoun + factive
  entries on top.

**Verify (ratchet):** missing-lexeme units drop sharply; grammar-gap rises then falls as
each construction lands; first **encoded** units appear. Track per sub-slice.

**Deps:** Phase 1 (clean tokens). **Risk:** 2b (coordination) is the hard, combinatorial
core — sequence it carefully and watch forest size.

---

## Phase 3 — productive `-ly` adverbs: recognize, then transparent / justification-route

**Semantics decided in `docs/notes/d62-adverb-semantics-decision.md`** (read it first). For science,
adverbs are mostly **not load-bearing** → handled **transparently**; the load-bearing minority is
**measurement/quantification-associated** → routed to **justification logic** (not Davidsonian
manner-on-event, which is decoupled and deferred). The fresh-DB run confirmed `-ly` adverbs are OOV
**even over full WordNet** (`has_token("commonly") = false`), so recognition needs derivation.

**Build (two parts):**
1. **Recognize** — a lookup-time **derivational `-ly` rule** in `kernel/src/dcg/lookup.rs` (a sibling
   to Morphy, kept separate from the inflectional port): a `-ly` surface not found is reverse-derived
   to its adjective base (`-ily→-y`, `-ly→-le`, `-ally→-ic/-al`, `truly→true`, `fully→full`, else
   strip); if the adjective exists, emit an **adverb-categorized** item (category from `adv.xsl`:
   manner `(s\np)\(s\np)`, sentential `s/s`/`s\s`, transitional with comma).
2. **Route the sem** — a curated **inert-vs-measurement classification**: inert bulk (`commonly,
   typically, respectively, …`) → **identity** sem (`λV.V`, transparent, recorded cut); measurement
   subset (`selectively, preferentially, significantly, highly, …`) → the **WRN measure+evidence
   pattern** (decision note §4a): a differential/graded **domain predicate** over a contrast + a
   **measurement obligation** (Declared, discharged to Derived/Verified by a
   `stats:StatisticalAnalysisResult`) carried in a **justification-logic certificate** — reusing the
   D64 `ProofObligation` carrier, not an opaque adverb operator. Optionally import WordNet's small
   lexicalized adverb set separately.

**Verify (ratchet):** the `-ly` adverbs leave the OOV list; inert-modified clauses parse to the same
claim as unmodified; a measurement adverb attaches a graded qualifier.

**Deps:** Phase 2 (adjectives + a clause to modify); the justification-logic surface for part 2's
measurement encoding (may be a follow-on slice). **Risk:** low for transparent recognition; medium
for the justification encoding + the inert/measurement boundary. **Not a dep:** event semantics /
`schema:Action` reification — deferred per the decision note (revisit for *verb* reification only).

---

## Phase 4 — content lexica in parse scope (the payoff)

**Goal.** Bring WordNet + UMLS + NCBI into parse scope so domain content words resolve — and
so scope collapses the ambiguity (Finding 1). The lexica are ready; this is plumbing.

**Build / infra:**
- **Value-index-backed form lookup** so a scoped parse over the 7.6M chain does **not**
  full-scan (the same gap that OOM'd the object-value query). Parse already uses the D65 value
  index; ensure scoped lookup + the `lexicon:form` path are value-indexed end-to-end.
- **NCBI `--out-dir` partitioning** (mirror wordnet/umls) so the 165 MB gene list loads
  (currently > the 128 MiB gRPC limit).
- Scope selection threaded through `ParseSentence` (already in the proto) + a `LexiconProfile`
  for the WRN domain (WordNet + UMLS Level-0 + NCBI human).

**Verify (ratchet):** domain terms (`WRN/MSI/helicase`) resolve in scope; content OOV → ~0;
per-sentence forest size **shrinks markedly** under domain scope vs unscoped (measure the
delta — this is the disambiguation lever).

**Deps:** Phases 1–3 (sentences must parse first). **Risk:** medium (index/scope plumbing).

---

## Phase 5 — disambiguation: narrow-then-select (S4)

**Goal.** Reduce the (now scope-narrowed) forest to one reading.

**Build:** S4 as `narrow → select`: domain scope (Phase 4) narrows senses; then
context-consistency against the committed prefix (EigenQL over prior claims) + an LLM judge
among the survivors; emit a `DecisionPoint`. (Per D62 §7.5; the prototype's rank-0 stub is the
fallback.)

**Verify (ratchet):** WRN-page sentences yield a single (or few) felicitous reading;
encoded-unit count rises toward the parseable fraction.

**Deps:** Phase 4. **Risk:** medium.

---

## Phase 6 — the remaining D62 stages + institution

**Goal.** Complete the driver once real prose parses: **D64** reference resolution (open
parses → bound), **S5a** lexical recovery (search/inject for any residual OOV), **S5b**
reformulation (grammar-gap paraphrase + back-translation), **S6** assembly, and the
`FormalizeDocument` institution wrapper (`ontologies/encoding/encoding.esl`, wired into
`BOOTSTRAP_CHAIN`; resolve the env/handler stubs). Faithfulness (S7) stays deferred to D61.

**Verify:** end-to-end — the cleaned WRN page produces committed `EncodedClaim`s with
provenance + grade + the gap stream; the litmus claims (e.g. *"WRN is a synthetic lethal
target"*) encode.

**Deps:** Phases 1–5. **Risk:** D64 + S5b are their own specs (D64 exists; S5b → D66).

---

## Cross-cutting infrastructure (interleave as needed)

- `ParseSentence` carries the **open forest + missed tokens** (D62 §11.5) — needed by Phase 2e
  (open parses) and any orchestration-side recovery.
- **Object-value / property-predicate pushdown** for untyped EigenQL patterns (the OOM item) —
  needed for Phase 4's value-index lookup and for measuring coverage at scale safely.
- Promote the prototype's `segment`/`encode_unit` from the test into a `dcg`/encoding module
  once Phases 1–3 stabilize the shape (the "packaging" we deferred).

## Sequencing summary

```
P0 ratchet ─┬─ P1 S0 tokenization ──┬─ P2 closed-class+grammar ──┬─ P3 -ly adverbs ─┐
            │  (low risk, unblocks)  │   (2b coordination = crit) │                  │
            │                        │                            ▼                  ▼
            └────────────────────────┴──────────────▶ P4 content scope ─▶ P5 disambig ─▶ P6 stages+institution
```

The first two phases (S0 + closed-class/grammar) are where the evidence says the work is;
domain lexica and disambiguation are the payoff that lands once sentences parse. Every phase
ratchets the WRN-page metric, so progress is measured, not asserted.
