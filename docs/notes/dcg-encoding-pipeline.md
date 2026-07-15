# The encoding pipeline: prose → typed propositions

What the `kernel/src/dcg` module actually does, stage by stage, and the literature each stage draws on.

**The thesis.** A sentence is *encoded* when the kernel has type-checked its assembled meaning. Nothing
in this pipeline is trusted to decide that: an importer, an LLM, a heuristic — all of them only ever
*propose*. The kernel's type-checker is the sole oracle, and an unparseable sentence is a first-class
outcome (`Gap`), never a silent drop.

---

## 0. The shape of the thing

```
document
   │
   ├─ Stage A  PREPROCESS ─── abbreviation extraction → grounding → OOV grounding
   │      │                   ──→ a doc-scoped lexicon LAYER
   │      │                   (abbrev.rs, glossary.rs, augment.rs)
   │      │
   ├─           SEGMENT ────── document → sentence units + non-prose flags
   │                          (segment.rs)
   │
   ├─ Stage B  PARSE ───────── per sentence, over base + doc layer
   │      │                   (parse/, rules/, chart/, lexicon.rs)
   │      │
   │      ├── tokenize ──────── sentence → word tokens                             (segment.rs)
   │      ├── seed ──────────── lexicon lookup → lemmas → entries → chart leaves   (parse/seed.rs)
   │      │      └── rank ───── lexicon precedence, sense frequency, LLM rerank    (sense_ranker.rs)
   │      ├── compose ───────── CKY over the categorial rules                      (chart/, rules/)
   │      └── gate ──────────── the KERNEL type-checks the assembled sem           (parse/felicity.rs)
   │
   └─ Stage C  RESOLVE ─────── bind referent holes against the discourse
                              (parse/resolve.rs)
                                        │
                                        ▼
                         SentenceOutcome ∈ { Encoded | Ambiguous | Open | Gap }
```

The orchestration is `pipeline.rs` (`InProcessPipeline::encode_with_layer`). Every LLM-backed step sits
behind a trait (`AbbreviationProposer`, `CategoryProposer`, `SenseRanker`, `Proposer`), with a
deterministic mock in CI and a live Anthropic impl under `--features use-llm`. The trait *is* the seam
between "the pipeline" and "how its untrusted steps run".

---

## 1. Building the lexicon (offline, before any document)

Two importers produce the committed lexicon. Both are **deterministic structural transforms, no LLM**.

**WordNet** (`crates/eigenius-wordnet`) — the general English framework. Noun synsets become
`core:Class`es with `@` hypernymy as `core:subclass_of`; `@i` instance synsets become individuals;
verb/adjective synsets become `eigentt:Axiom` predicates; lemmas become `lexicon:LexicalEntry`s. It also
ports WordNet's **Morphy** stemmer (`morphy.rs`), exposed as the `Lemmatizer` the parser's
surface→lemma step runs on.

**UMLS** (`crates/eigenius-umls`) — the biomedical domain. Parses the Rich Release Format and renders a
faithful typed *mirror* plus a *derived* domain lexicon ("mirror-then-derive").

**Every entry passes the felicity gate at import** (`lexicon.rs::gate_entry`): an entry is admitted iff
`⟦cat⟧ ≡ sem_type` and its `sem` actually inhabits `⟦cat⟧`. A malformed entry never reaches the parser.

> `⟦·⟧ : Cat → Type` is the categorial-to-type homomorphism (`category.rs::denote_cat`), a recursor over
> the `lexicon:Cat` inductive: `⟦S⟧ = Prop`, `⟦N(T)⟧ = Set`, `⟦NP(T)⟧ = T`, `⟦A/B⟧ = ⟦A\B⟧ = ⟦B⟧ → ⟦A⟧`.

---

## 2. Stage A — the document glossary and OOV grounding

`abbrev.rs` (extraction), `glossary.rs` (grounding + emission), `augment.rs` (the OOV transducer).

A paper defines its own vocabulary in its first paragraph, and the committed lexicon has never heard of
it. `microsatellite instability (MSI)` must make a bare `MSI` parse three pages later.

**Abbreviation extraction** (`abbrev.rs`) implements the Schwartz–Hearst algorithm — the standard method
for pulling `long form (SHORT)` pairs out of biomedical abstracts, a pure text transform with no
dependency on the lexicon. Each extracted pair is then *grounded* (`glossary.rs`) to a concept
(e.g. `MSI` → `umlscui:C0920269`) and emitted as **one** `lexicon:LexicalEntry`: the abbreviation is an
**alias** of the grounded concept and inherits *that concept's* category. No new individual is minted,
and no grammar rule is added.

Grounding is keyed on the concept's ontological kind:
- a **class / phenomenon** (a UMLS CUI) → a common noun `cat_n(concept, Num)`. The number feature is
  inherited from the long form's **head noun**: a *mass* head (`instability`) licenses the
  bare-singular-mass subject reading; otherwise `num_any` (a count noun needing a determiner).
- a **named individual** (an HGNC gene symbol like `WRN`) → a proper-noun `cat_np(sty, sg)` alias naming
  the *same* instance.

Residual out-of-vocabulary atoms are handled by the augmentation transducer (`AugmentOptions`):
`DocumentOnly` (deterministic), `LexiconBacked` (grounds against the form text index), `LlmBacked`
(synthesis). **Fail-closed**: a proposal the felicity gate rejects is skipped, and an OOV that *no*
proposal closes is recorded as a `Gap` — never silently dropped.

The result is committed as a **document-scoped lexicon layer chained on the base**, so the document's
vocabulary is visible to the parser without polluting the committed lexicon. (Running the pipeline
*harvests* these as candidate permanent additions — propose → gate → commit.)

---

## 3. Segmentation

`segment.rs`. Deterministic, no LLM.

Splits the document into sentence units and flags non-prose tokens (statistics, figure references) so the
parser skips them. A naive `.`/`!`/`?` split over-segments — measured on the cleaned WRN first page, it
turns 4 paragraphs into 47 units; this segmenter yields ~26, routes stat/figure-ref tokens out, and keeps
gene symbols like `MLH1`/`MSH2` intact.

Tokenization proper (`segment.rs::tokenize`) is the same module, one granularity down — sentence to
word tokens. It strips bracketed asides, normalizes
em-dashes/slashes/brackets to spaces, and **preserves the comma as a standalone token** (the parser keys
list coordination on it).

---

## 4. Stage B — the parser

This is the substance. Four sub-stages: **seed → rank → compose → gate**.

### 4.1 The categorial backbone

The grammar is a **dependent categorial grammar** in the sense of Chatzikyriakidis & Luo — categories are
*type-indexed* (`cat_np(Gene, sg)` carries its class), so `⟦·⟧` is self-contained and selectional
restriction falls out of the type system rather than a feature hack. This is Luo's **coercive subtyping**
move: a specific type reaches a general argument slot by subsumption (Luo 2012).

The category algebra is itself an **inductive type in the kernel** (`lexicon:Cat`), not a string. A
category cannot be malformed — the kernel checks it — and the composition rules pattern-match on it.

Design lineage: **lightblue** (Bekki's Dependent Type Semantics, `references/lightblue`) is the primary
reference implementation — a CCG parser whose semantics are DTS terms and whose type-checker is the
felicity oracle. The combinator set and the chart shape follow Steedman's CCG.

### 4.2 Seed — the lexicon lookup

`parse/seed.rs`. For every token span (bounded by the longest multiword form):

1. **lemmatize** the surface across all four POS (Morphy), plus a domain-plural stem fallback.
2. **look up** each candidate lemma in the `LexicalIndex` (`form → entries`). Two backing modes: a
   **lazy** probe of an active `core:ValueIndex` on `lexicon:form` (the production path — essential at
   WordNet's 325k entries, where an eager scan dominates), and an **eager** full-chain scan as fallback.
3. **scope-filter**: only entries whose `lexicon:in_lexicon` is in the parse scope survive (untagged
   entries — the grammatical closed class — are always available).
4. **cap** the senses (below).
5. **refine morphology**: the surface's number instantiates the entry's underspecified `num_any`, so
   determiner/noun agreement bites at composition (`every gene` ✓ / `every genes` ✗).

A multiword entry (`cell line`) seeds its whole span **alongside** the single-token items for its parts —
the MWE-vs-compositional ambiguity is carried as competing chart edges, not resolved here.

Also seeded: productive morphology with no lexical entry of its own (`-ly` adverbs, denominal and
prefixed adjectives), and the **unary shifts** — bounded type-raising, and the bare-plural/mass **kind
shift**. The latter is Carlson's insight (Carlson 1977) as realized by **Chierchia's ∩**: a bare plural
denotes its *kind*, and as an argument it is that kind realized as an individual, `kind_of(t) : Entity`.
"genes affect HeLa" is a *complete* proposition about the kind, not an open one.

### 4.3 Rank — taming WordNet polysemy

Over the full lexicon, a short sentence yields hundreds to thousands of well-typed parses. The felicity
gate prunes *none* of it (they really are all well-typed), so ranking is load-bearing, not cosmetic.

A parse carries a 2-component additive **cost**, summed over its leaves and sorted lexicographically:

| key | meaning |
|---|---|
| `lexicon_order` (primary) | the leaf's position in the parse scope's ordered lexicon list — soft precedence between lexica (domain before general) |
| `sense_rank` (secondary) | the leaf's WordNet sense-frequency rank (0 = most frequent) |

The kernel never learns what either component *means* — it sums opaque weights, keeping the engine
sense- and lexicon-agnostic.

Three levers bound the search, each **bounded and reversible**:

- **Sense cap** (adaptive supertagging): seed at most *N* senses per lemma, lowest rank first.
- **Contextual sense reranker** (`sense_ranker.rs`): an *untrusted* LLM reorders each content word's
  candidate senses by plausibility *in this sentence*, so the cap keeps contextually-likely senses rather
  than merely globally-frequent ones. This is zero-shot neural contextual supertagging (cf. Clark et al.
  2015 on supertagging for CCG). It reuses the **proposer-behind-oracle** pattern: the ranker only
  reorders the seed beam; the kernel still decides validity.
- **Cell beam**: cap each composed chart cell to the *N* lowest-cost items.

**Widen-on-failure** is what makes all three safe. If a parse yields nothing *and every token is known*
(so it is not an OOV miss but possibly a pruning artifact), the beam is doubled, then the sense cap, and
the parse is retried. A bad rank therefore costs a re-parse, **never a missed parse**. There is also a
static-rank fallback pass, because the untrusted reranker can bury a construction-triggered category
variant that static rank would have kept.

### 4.4 Compose — the chart

`chart/`, `rules/`. Two implementations of one concern, kept deliberately.

**The rules** (`rules/`):
- `combinators.rs` — the categorial rules: forward/backward application, composition (harmonic and
  crossed), the dependent determiner, and the nominal-modification family. Constrained by **Eisner
  normal form** (Eisner 1996): a composition output may not be the primary functor of a subsequent
  application, and a type-raised functor may only *compose*. This is what kills spurious derivational
  ambiguity and keeps declaratives single-parse.
- `constructions.rs` — the construction rules: how coordination, the relatives, the appositives, the
  reciprocal, the distributives, type-raising, the kind shift, and the fronted participial each build
  their result category and semantics. Each is a pure function of the operands' `(cat, sem)`. (The Cat
  *algebra* they call — `⟦·⟧`, unification, subsumption, the feature meet — is `category.rs`, one layer
  below: the algebra is a theory of categories; a construction is a fact about English.)
- `registry.rs` — the token-keyed rules (relatives, coordination, `but not`, the reciprocal, the
  appositives) plus the unary shifts. **One definition of where each rule fires**, consumed by both
  drivers, so they cannot drift apart.

**The sem-blind invariant.** A parse `Item` is split into a `CategoryPayload` (category, provenance,
cost) and a `SemanticPayload` (the assembled term). The combination rules receive **only** the category
payload and therefore *cannot* branch on a meaning. This is not a convention — it is a compile-time
guarantee, and it is exactly the condition that licenses packing. It is the "postcondition vs. carry"
separation of Hopkins & Langmead (2009).

**The packed forest** (`chart/packed.rs`, `chart/forest.rs`) — the production path. A chart cell holds
one node per **signature** rather than a flat list of items, so the sense-product of same-shaped items
collapses to a single node (Billot & Lang 1989; Harper 1994). Combination is decided **once per
node-pair** on representative items — sound precisely because a signature captures everything a decision
can consult. The differing *semantics* are then materialised **lazily**, by cube pruning over the
children's cost-sorted k-best lists (Huang & Chiang 2005), so the forest is built once but only the
low-cost readings are ever assembled.

> **The packing invariant.** `Sig = (category key, ENF provenance, is-coordination-sem)`. The third
> component is there because two token-keyed rules consult a sem in their *decision* — and a completed
> coordination is invisible in the category (`complete_coord` folds it back into its base). Without that
> bit, deciding on a representative would silently drop an edge every *other* item in the node needed.
> The category key (`cat_shape` / `cat_key`) lives beside `node_sig` in `chart/forest.rs` — it is a
> **canonical key, not a display string**; a formatting change to it would silently change which
> derivations the forest keeps.

**The unpacked chart** (`chart/unpacked.rs`) — a flat, beamed, item-level CKY. Retained for three
reasons: it is the **differential oracle** (packed ≡ unpacked is the property that licenses packing at
all, and it is only testable because an independent implementation exists to compare against); it handles
pied-piping (a quaternary rule the packed forest has no edge shape for); and it carries the
combinatory-core spike.

**Coordination** defers its operator: a comma builds a neutral list and the trailing `and`/`or` binds the
whole list, so `A, B, C or D` is all-∨ rather than `(A ∧ B ∧ C) ∨ D`. Conjunction is lifted pointwise
over Prop-ending categories — generalized conjunction, following Partee & Rooth's conjoinable types.

### 4.5 Gate — the kernel as felicity oracle

`parse/felicity.rs`. **This is the only step that decides truth of form.**

A full-span parse is merely a *candidate*: the rules built a term from the categories, but nothing yet
says it is well-typed. The gate evaluates the assembled sem (normalization by evaluation), reads back the
normal form, and `check`s it against `⟦cat⟧`. Reduction first is essential — a composed determiner
sentence is a redex-heavy `App(λ…, …)` tree, and a bare lambda's type cannot be synthesized.

A candidate that fails is simply dropped. **An empty forest is a first-class answer, not an error.**

Two outcomes survive: a **closed** parse (a hole-free `Prop`), and an **`OpenParse`** — felicitous, but
still carrying referent holes.

---

## 5. Stage C — resolving referents

`parse/resolve.rs`. A pronoun or possessor seeds a *hole*: a fresh free variable, named by the span it
was created on (`holes.rs`). The discourse is threaded across sentences, and an **untrusted** `Proposer`
(the LLM resolver under `use-llm`) suggests antecedents from the candidate set — every named entity the
prior sems referenced.

The kernel re-gates every substitution. A proposal that does not type-check is rejected. Same pattern as
the sense ranker: **propose → gate**.

The dynamic-semantics framing (a sentence's meaning is a context update, and an anaphor is bound by an
antecedent in that context) follows Kamp & Partee — and DTS, which is lightblue's own semantics, is where
the Σ-existential treatment of anaphora comes from.

---

## 6. The encoding

`SentenceOutcome` — and note that three of the four are *not* success:

| outcome | meaning |
|---|---|
| `Encoded(Item)` | one closed, resolved proposition — `item.sem()` is the `Prop`. **This is the knowledge.** |
| `Ambiguous(Vec<Item>)` | it parses, but carries unresolved sense/structural ambiguity |
| `Open(OpenParse)` | felicitous, but a referent hole the resolver could not close |
| `Gap` | no parse — an OOV token, or an all-known-tokens *grammar* gap |

The distinction between the last two matters operationally: an OOV `Gap` routes to lexical recovery
(search + inject an entry); an all-known-tokens `Gap` is a **grammar** gap and routes to reformulation.
`Parser::has_token` is the signal that separates them.

The pipeline returns the whole forest and commits nothing. Selecting one parse and committing it as a
`lexicon:Sentence` is the encoding institution's job, not the parser's.

---

## 7. Literature

Cited in the source, load-bearing:

| work | what it gives |
|---|---|
| **Chatzikyriakidis & Luo**, *Formal Semantics in Modern Type Theories* | the dependent categorial grammar; CN-as-types; the modifier classification (intersective / subsective / privative / gradable) |
| **Luo (2012)**, coercive subtyping | how a specific type reaches a general argument slot |
| **Bekki**, Dependent Type Semantics; **lightblue** (`references/lightblue`) | the reference implementation: a CCG parser whose semantics are DTS terms and whose type-checker is the felicity oracle |
| **Steedman**, Combinatory Categorial Grammar | the combinator set and the chart |
| **Eisner (1996)** | the normal form that kills spurious derivational ambiguity |
| **Billot & Lang (1989)**; **Harper (1994)** | the packed shared forest; "Method 3" (deferred/lazy semantics) |
| **Huang & Chiang (2005)** | cube pruning — lazy k-best extraction from the forest |
| **Hopkins & Langmead (2009)** | the postcondition-vs-carry separation ⇒ the sem-blind rule invariant |
| **Carlson (1977)**; **Chierchia's ∩** | bare plurals denote kinds; `kind_of` nominalization |
| **Partee & Rooth**, generalized conjunction / conjoinable types | the pointwise-lifted coordination |
| **Kamp & Partee (1995)** | the dynamic/discourse framing behind hole resolution |
| **Schwartz & Hearst (2003)**, *Pac Symp Biocomput* | abbreviation-definition extraction (Stage A) |
| **Clark et al. (2015)** | supertagging for CCG — the prior the contextual sense reranker generalizes |
| **WordNet** (Miller/Fellbaum); **Morphy** | the general lexicon and its stemmer |
| **UMLS Metathesaurus** | the biomedical lexicon |

**Citation hygiene.** These are the works the source cites; the bibliographic details above are as
recorded in the code and design docs, **not independently verified against DOIs**. Before any of them
becomes load-bearing in a *published* claim, verify it. `docs/design/d63-dcg-engine-english-grammar.md`
already flags Partee & Rooth (1983) as "citation to verify before load-bearing" — that flag stands.

---

## 8. The one property worth remembering

Every untrusted component in this pipeline — the abbreviation extractor, the OOV grounder, the category
proposer, the sense reranker, the anaphora resolver — is a **proposer behind an oracle**. None of them can
admit anything. They reorder, they suggest, they ground; the kernel's type-checker decides. And every
pruning lever is bounded and reversible, so a bad proposal costs a re-parse rather than a lost parse.

That is what makes it safe to put a language model inside a knowledge-encoding pipeline at all.
