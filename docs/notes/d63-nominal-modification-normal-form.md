# D63 — A normal form for nominal modification (the prenominal modifier stack)

**Status:** Design note (grounded, pre-implementation; literature-surveyed + D2 coverage measured
`2026-07-08`, §2a / §4 / §10). **Reframed `2026-07-09` (§4):** the corpus's headline example (`synthetic
lethal`) was a *lexicalization* confound — hyphenating it in the CNL (a style-guide fix, zero code)
collapses S5 ~3×. So the lever order is **lexicalize/hyphenate → sense reranker → this NF on the residual**;
the NF's genuine target is ~3× smaller than the first measurement implied. The Phase-3 structural lever
identified by the `2026-07-08` chart re-measurement
([d63-parsing-scale-and-pruning.md §4b](d63-parsing-scale-and-pruning.md),
[d63-parse-gap-closure.md §6a](d63-parse-gap-closure.md)). Tracked under GH#97.

**One-line problem.** A prenominal modifier stack (`attractive synthetic lethal targets`,
`DNA repair processes`) derives many structurally-distinct trees for one meaning. Sense-ranking does not
reduce this — it is a *bracketing / category-choice* multiplicity, orthogonal to sense polysemy. This note
defines a normal form that collapses the spurious trees to one canonical derivation, and pins down the one
place it must NOT collapse (genuine collocations), which it hands to the lexicon.

---

## 0. The engine this note extends

The normal form below is one rule inside a specific machine: a **dependent categorial grammar (DCG)**
parser ([`kernel/src/dcg/`](../../kernel/src/dcg/), the Chatzikyriakidis–Luo DCGs, `chatzikyriakidis-luo-2020`;
D62 §8.6) that turns prose into **type-checked EigenTT trees**. The organizing principle is
**proposer-behind-oracle**: an untrusted source (the WordNet/UMLS import, or an LLM reranker) only ever
*proposes* lexical entries, senses, and derivations; the kernel type checker is the **felicity oracle** that
admits or rejects. The parser is the trusted half of the prose → typed-trees pipeline; this note tightens
*what it proposes*, never what the oracle accepts.

### 0.1 The type theory — EigenTT

The trees are terms of **EigenTT**, the kernel's dependent type theory
([`kernel/src/nbe/term.rs`](../../kernel/src/nbe/term.rs), ported from a `Core/Abs.hs` reference and extended
with Eigon ground types). The constructs this note uses:

- **Π** (dependent function) and **Σ** (dependent pair) — a refined noun is a `Σx:C. R(x)`: an entity of
  ontology class `C` *paired with* a proof of the restrictor `R`. Modifiers refine a noun by intersecting a
  predicate into that Σ — which is exactly why a modifier *stack* is a *nesting* of Σ's, and why bracketing
  matters.
- **Sort(n)** universes — `Sort(0) = Prop`, `Sort(1) = Set` (D46 §3). A clause denotes a `Prop`; a common
  noun denotes a type in `Set`.
- **Id / Refl / J** (propositional equality), **Data / Case** (sums), **Ann** (the bidirectional-typing mode
  switch, D46) — used elsewhere in the grammar, peripheral here.
- Evaluation is **normalization by evaluation** ([`kernel/src/nbe/`](../../kernel/src/nbe/)): `eval` to a
  `Val`, `readback` to a normal-form `Exp`; **definitional equality** is comparison of normal forms. The
  `felicity_readback` step in the parser is exactly this pass run on each candidate sem.

Grammaticality is not a separate grammar oracle: a full-span `S` parse is **felicitous iff its assembled
sem type-checks to `Prop`** against the ontology layer ([`lookup.rs`](../../kernel/src/dcg/lookup.rs) step 4,
`nbe::check::check`). The type theory *is* the acceptance test.

### 0.2 The lexicon — categories as an inductive type, entries as resources

CCG categories are **not** a hard-coded Rust enum; they are values of an inductive type
**`lexicon:Cat`** (with parameters `lexicon:Mood` / `lexicon:Fin` / `lexicon:Num`) declared in the
`lexicon:` ontology ([`ontologies/lexicon/`](../../ontologies/lexicon/)). `cat_n`, `cat_np`, `cat_s`,
`fwd`, `bwd`, `cat_kind`, `cat_measure`, … are its constructors (resolved via `resolve_inductive`,
[`category.rs`](../../kernel/src/dcg/category.rs)). A **lexical entry** is a committed
`lexicon:LexicalEntry` resource carrying a category and a sem; the lexicon is *data* (the WordNet + UMLS
imports), and the engine consumes it.

Parsing **seeds** the chart from that data ([`lookup.rs`](../../kernel/src/dcg/lookup.rs)): tokenize, then
for every span (bounded by the longest multiword form) reduce the surface to lemmas via the Morphy
lemmatizer and look them up in the [`LexicalIndex`]. **Multiword entries** (`cell line`, `act on`) seed a
multi-token span *alongside* the single-token items for their parts — the MWE-vs-compositional ambiguity is
carried as competing edges, not resolved at seed time. **This multiword path is the mechanism §4 leans on**:
a genuine collocation is a *seeded lexical unit*, not a bracketing the compound rule reconstructs. The
dominant scaling pressure is **WordNet sense polysemy** — many senses per lemma — controlled by the
`sense_rank` frequency prior, the adaptive `sense_cap` (supertagging), the contextual LLM reranker, and
widen-on-failure. That is the *sense* lever; this note is the *structural* lever, orthogonal to it (§6).

### 0.3 The bridge into the ontology

The seam between grammar and knowledge graph is the homomorphism **`⟦·⟧ : Cat → EigenTT type`**
(`denote_cat`, [`category.rs`](../../kernel/src/dcg/category.rs)). Its type parameters **range over ontology
classes**: `Exp::EigonClass(iri)` is a ground type resolved from the layer chain, and subsumption is
`layer.is_subclass_of` — the validator walking the immutable parent-pointer layer chain, with
`lexicon:Entity` at the top of the entity lattice. So `cat_n(C, num)` denotes a common noun over the
ontology class `C`; refining it yields the `Σx:C. R` above; the restrictor axioms `compound_kind`,
`compound`, `kind_of` are **`EigonAxiom` IRIs** (`urn:eigenius:ontology:…`) — opaque typed constants whose
types are registered in the chain's `AxiomEnv`. A parse's felicity is therefore checked *against the
ontology's own type structure*. The engine returns the **whole forest** (no selection, no commit);
selecting one parse and committing it as a `lexicon:Sentence` is the encoding institution's job (the kernel
commit gate) — the parser proposes, the institution disposes.

### 0.4 The packing machinery (already implemented)

The chart is a **packed shared forest** ([`packed.rs`](../../kernel/src/dcg/packed.rs)): a `Forest` of
`PNode`s whose derivations are `Edge`s — `Leaf` / `Combine(left,right)` / `Unary(child,kind)` /
`Binary(left,right,rule)`. Two items **share a node iff they share a signature**
`Sig = (cat_shape, Combinator)`: the category shape with type-indices erased, plus the **Eisner
normal-form** provenance (`Combinator`) that already suppresses spurious composition duplicates. Distinct
content sems under one shared node are enumerated **lazily at extraction** (`kbest`, capped at
`DEFAULT_FOREST_CAP`), each run through the felicity oracle. This is load-bearing for §2's mechanism claim:
because every refined noun shares the shape `cat_n(Σ_)` and the `Combinator::Compound` provenance, the
packing *already* shares the node — so the modifier-stack multiplicity is **distinct sems at extraction**,
and the lever is a **combine-time guard** (refuse to build the non-canonical bracketing), not a change to
the packing key.

**Where this note sits.** Everything above is built and working. This note adds one combine-time
normal-form rule to the *parser* proposal stage (§3), governed by a spurious-vs-genuine criterion that
delegates real collocations to the *lexicon* (§4) — leaving the *type theory*, the *ontology bridge*, and
the *packing machinery* untouched.

## 1. The multiplicity, measured

The chart re-measurement (Derived, `2026-07-08`, snapshot `wordnet-umls-all-2026-07-08`, cap-only,
beam 1024) factors each sentence's reading count into **structural skeletons × sense-product**:

| sentence | closed | classify candidates | structural skeletons | sense × |
|---|---|---|---|---|
| S1 `Synthetic lethality is an interaction between two genetic events` | 240 | 256 | 22 | ~12× |
| S2 `The co-occurrence of these two events leads to cell death` | 150 | 186 | 36 | ~5× |
| S3 `Each event alone does not lead to cell death` | 32 | 32 | 2 | ~16× |
| S4 `Scientists can exploit synthetic lethality for cancer therapeutics` | 8 | 8 | 2 | ~4× |
| S5 `DNA repair processes are attractive synthetic lethal targets` | 48 | 144 | 12 (**3 within one subject-frame**) | ~12× |

The clean case is **S5, within one subject-sense frame: exactly 3 modifier-stack skeletons** (each ×16
WordNet-vs-UMLS senses over the noun slots):

1. **all-adjective** — `Σ:Σ:N. And(And(gt,gt),gt). compound_kind(K,N)` (attractive ∧ synthetic ∧ lethal);
2. **nested-compound** — `Σ:Σ:N. gt(…). compound_kind(K, …compound_kind(K,N))`;
3. **mixed** — one adjective refinement + one `compound_kind` + one adjective-on-compound.

The refined-noun category `cat_n(Σ_, …)` is the dominant saturating mid-chart shape (top in 32 of 173
non-leaf cells). **This is the structural residual the NF targets.**

## 2. Where it comes from in the parser

The refined noun is built in [`parser.rs`](../../kernel/src/dcg/parser.rs) by four rules, one per
`RefineKind` (`parser.rs:264`; assembly `parser.rs:586–643`):

| `RefineKind` | trigger | restrictor added to `Σx:C. _` |
|---|---|---|
| `Attrib` | `S[dcl,adj]\NP` (left) + `cat_n` (right) | `adj(x)` — and if `C` is already `Σx:Base.P`, **conjoins flat**: `Σx:Base. P ∧ adj(x)` (`parser.rs:588–603`) |
| `NamedCompound` | `cat_np` + `cat_n` | `compound(x, np)` |
| `KindCompound` | `cat_n` + `cat_n` | `compound_kind(x, n)` |
| `PpMod` | `cat_n` + PP | `pp(x)` |

Two facts explain the multiplicity, and both point at the fix:

- **The pure-compound bracketing is already normalized.** `KindCompound`/`NamedCompound` are guarded by
  `!is_compound_refined(&right.cat)` (`parser.rs:392`) — the left-branching NF (D63 §8.13): a
  compound-refined noun may not be a compound HEAD, so a 3+-noun compound chain collapses to the single
  left-branching tree. **The pure-adjective stack is also already normalized** — the `Attrib` flat-Σ
  conjunction (`parser.rs:588`) folds `attractive synthetic lethal` into one `Σx:Base. a₁∧a₂∧a₃` rather
  than nesting. So **neither pure adjectives nor pure compounds are the problem.**
- **The adjective↔compound *interleaving* is deliberately left open.** `is_compound_refined`
  (`parser.rs:819`) returns false for an `Attrib`-refined noun, with the explicit comment
  (`parser.rs:818`): *"An attributively-refined noun is NOT compound-refined, so adjective+compound still
  composes."* So `adj(compound(N))`, `compound(adj(N))`, and every interleaving of the two coexist. **That
  interleaving is S5's 3 skeletons.**

**Why packing does not already fix it.** The forest packs by `Sig = (cat_shape, Combinator)`
([`packed.rs:33`](../../kernel/src/dcg/packed.rs)); `cat_shape` erases the Σ-type and all four refine rules
emit `Combinator::Compound`, so every refined noun shares one shape `cat_n(Σ_)` and packs into one node.
But the distinct Σ-**content** sems are enumerated at *extraction* (kbest over the packed node) and each is
felicity-checked — that is the classify-candidate blow-up. **So the lever is a combine-time constraint that
refuses to BUILD the non-canonical bracketings, not a packing-key change** (the node is already shared).
This mirrors the existing left-branching guards, which are combine-time.

## 2a. Prior art — what we inherit, and what is new (literature survey, `2026-07-08`)

A verified survey (references in §10) places each piece of this design against established work.

- **Eisner's combinatory NF (Eisner 1996) does not reach our residual — provably, not by omission.** His two
  constraints collapse a *pure same-category forward chain* (a pure adjective stack `N/N N/N N`) to one
  right-branching derivation, and his Theorem 2 gives one NF tree per *semantic-equivalence class*. But his
  equivalence is **per-λ-recipe-for-all-interpretations** and he explicitly declines to merge "parses that
  happen to have equivalent denotations." Our adj↔compound interleaving is exactly that excluded class: the
  adjective rule builds a conjoined restrictor `And(...)`, the compound rule a nested opaque
  `compound_kind` axiom — **different recipes**, kept distinct by Eisner, that coincide *only because* the
  modifiers are intersective. **So the pure-adjective stack is already Eisner-normal (we get it free,
  §3.2); our new collapse (§3.3) is a strictly stronger, semantics-conditioned normal form beyond Eisner,
  licensed by intersectivity (§5).** This is the sharpest correction the survey makes to the note's framing.
- **The "canonical default + lexicalize the exceptions" strategy is CCGbank's own — including its documented
  failure.** CCGbank (Hockenmaier & Steedman 2007) makes base NPs flat and renders compounds as *strictly
  right-branching* trees, which the authors call "linguistically incorrect" because English N-N compounds
  are **~2/3 left-branching** (Lauer 1995: 66.8% left; Vadas & Curran 2007). Vadas & Curran fixed it by
  gold-annotating the internal brackets of only the non-default cases. **Two takeaways: (i) our
  left-branching compound core is the *empirically correct* default — do not repeat CCGbank's
  right-branching error; (ii) "dominant default + annotate the exceptions" is a validated pattern, and our
  lexicon (§4) plays the role of Vadas & Curran's NML/JJP annotation.** Nuance: the dominant default is
  *per construction class* (left for N-N cores, flat/outer for adjectives), not one global direction.
- **Category choice is an upstream, separate decision — confirms §6.** Broad-coverage CCG fixes a word's
  lexical category with a *supertagger before* the chart parser (Bangalore & Joshi 1999; Clark & Curran
  2004). So the adj-vs-noun choice belongs to the sense/supertag prior (our reranker/cap, Lever A), and the
  NF normalizes bracketing over the surviving categories — do **not** fold category choice into the guard.
- **Lexicalizing genuine collocations is recognized, and coverage is its known risk.** The lexicalized- vs
  institutionalized-MWE split (Sag et al. 2002) and the discovery-vs-identification framing (Constant et al.
  2017) are established; the binding failure mode is **"you cannot identify what is not in the lexicon"** —
  the coverage bound that D2 must confront.
- **Our Σ-refinement is independently the standard MTT account — external validation, confirmed against the
  primary source in-repo.** Luo's "common nouns as types" models `⟦ADJ N⟧ = Σx:N. adj(x)` with coercive
  subtyping `Σx:N.adj(x) ≤ N` — our `Σx:C. R(x)` and `layer.is_subclass_of`, arrived at independently.
  Chatzikyriakidis & Luo's Coq code (in-repo, App. A7 — see §5) states it verbatim: `CN := Set`, adjectival
  refinement as a Coq **record (= Σ-type)**, subtyping as `Coercion Surgeon >-> Human`. **This is not just a
  cite — we have their reference implementation to lift the D1 discriminator from (§5 table).**
- **Using an exact type oracle to prune NP-internal *bracketing* is unattested.** The DTT-semantics
  tradition (Luo; Chatzikyriakidis & Luo; Retoré; Bekki) is uniformly parse-first / type-check-second, and
  types prune *selectional* (predicate–argument) mismatches, never modifier bracketing. So §5's empirical
  adequacy battery — not a borrowed proof — must carry our soundness claim (honest caveat:
  absence-of-evidence over a large literature, not proof of novelty).

## 3. The target normal form

Impose one canonical derivation for a modifier stack over head `N`. §3.1–3.2 are **already Eisner-normal**
(§2a); only §3.3 is new:

1. **Pure compounds** — left-branching (existing, `is_compound_refined` guard; the empirically-correct
   ~2/3-left default, §2a). Keep.
2. **Pure adjectives** — flat conjunction over the base (existing, `Attrib` flat-Σ). Already the Eisner NF
   of a forward chain (§2a) — no new work. Keep.
3. **NEW — adjective/compound interleaving: adjectives attach OUTSIDE the compound.** Canonical form is
   `adj*( compound-core( N ) )`: build the full left-branching compound core first, then apply all
   adjectives as a flat conjunction on top. Forbid `compound_kind` whose head is an `Attrib`-refined noun
   (extend `is_compound_refined` to also return true for `Attrib` refinement *when* the intent is to block
   a compound head over it), and forbid the mixed interleavings. This collapses S5's 3 skeletons to 1.

Mechanism: a combine-time guard, exactly analogous to `parser.rs:392` and the coordination left-branching
NF ([`category.rs:634`](../../kernel/src/dcg/category.rs)). No new `Sig`/`Combinator` field is required for
the node identity (already shared); the guard prunes the recipe before `build` emits the non-canonical
item, so extraction has one sem where it had three.

## 4. The load-bearing decision — spurious vs. genuine, handed to the lexicon

An NF may only collapse trees that are **meaning-identical**. Modifier bracketing is not always spurious:

- `[synthetic lethal] targets` (a domain collocation — *synthetic-lethality* target) is **meaning-distinct**
  from `attractive ∧ synthetic ∧ lethal targets` (three independent properties).
- `[[small cell] lung cancer]` ≠ `small [cell lung cancer]`.

So the NF needs a **criterion for which forks are genuine**, and a home for them. The decision:

> **Genuine collocations are lexical, not emergent. A multiword modifier reading is licensed iff the
> lexicon carries it as a multiword entry; otherwise the NF's canonical (adjective-outside) tree is the
> only derivation.**

This is a **recognized division of labour** (§2a): the lexicalized-vs-institutionalized-MWE split (Sag et
al. 2002) and discovery-vs-identification (Constant et al. 2017). It reuses existing infrastructure —
**WordNet already stores multiword lemmas** (underscore-joined; `convert.rs:50`), and morphy already
resolves multiword collocations (`cell_lines → cell_line`, `act_on`; `morphy.rs:201,253`). So
`synthetic_lethality` (if a WordNet/UMLS lemma) already seeds as a *unit*, and its compound reading is
licensed as a leaf, not reconstructed by the compound rule enumerating brackets. The policy pushes the
spurious/genuine distinction from the parser's combinatorial search (undecidable there) into the lexicon
(decidable: is this surface a lemma?).

**The binding risk is coverage — "you cannot identify what is not in the lexicon" (Constant et al. 2017).**
A genuine collocation absent from the snapshot is invisible to the NF, which will then force the
all-adjective tree on it. And **flat "words-with-spaces" listing is documented to mishandle inflection /
discontinuity / productive families** (Sag et al. 2002): safe for rigid domain terms (`synthetic_lethality`),
risky for productive compounds. So D2 is not just a lookup — it is a coverage policy.

**D2 coverage — measured (Derived, `2026-07-08`, `d2_collocation_coverage` over `wordnet-umls-all-2026-07-08`).**
Coverage is **partial and asymmetric**:

| collocation | in lexicon as a unit? | sense |
|---|---|---|
| `synthetic lethality` (noun) | **yes** | `umls:C4280020` |
| `cell death` | **yes** | `wn:cell_death.n.11486178`, `umls:C0007587` |
| `dna repair` | **yes** | `umls:C0012899` |
| `co-occurrence` | yes (single token) | — |
| **`synthetic lethal` (adjective form)** | **NO** (0 entries) | — |
| `repair process(es)`, `dna repair process(es)` | no | — |
| `cancer therapeutics` | no | — |
| `genetic event(s)` | no (compositional — fine) | — |

**Fail-closed finding.** The **noun** `synthetic lethality` is a unit, but the **adjective form**
`synthetic lethal` — the one in S5 `attractive synthetic lethal targets` — is **absent**. So the NF's
canonical all-adjective tree parses it as `attractive ∧ synthetic ∧ lethal ∧ target` (three independent
properties), which is the **wrong meaning** (the intended reading is a *synthetic-lethality* target), and no
lexical unit licenses the correct compound reading — exactly the "cannot identify what is not in the
lexicon" failure (Constant et al. 2017).

**Resolution — hyphenate in the CNL, NOT inject an alias (corrected `2026-07-09`).** The cleaner fix, since
the CNL is authored input we control, is to **hyphenate the term: `synthetic-lethal`** — a style-guide rule
([d62-controlled-language-style-guide.md](d62-controlled-language-style-guide.md), "Hyphenate lexicalized
compound modifiers"). The D63 hyphen morphology then reads it as **one predicative compound adjective**
(head `lethal`, like `double-stranded`), so the two-adjective masquerade never arises. **Zero parser code,
no lexicon injection.** Measured (Derived, v3 `first-page-cnl-v3.txt`, merged kernel + Rust 1.97): S5 drops
**144→48 classify candidates, 12→4 structural skeletons, 48→24 readings** (§4c of
[d63-parsing-scale-and-pruning.md](d63-parsing-scale-and-pruning.md)); `synthetic-lethal` seeds as one
adjective, no OOV. This is the Vadas–Curran "annotate the exception" move (§2a) done at the CNL surface.

*(My earlier framing — "inject `synthetic lethal` as a multiword alias, non-optional" — was wrong on two
counts: it is not a morphological-derivation gap, and injection is **not** needed for parsing. An alias
`synthetic-lethal → C4280020` is still worthwhile but only for **grounding** the term to the concept — a
separate concern from the structural parse. Where `dna repair` (C0012899) is already a unit, the
left-branching core `[[dna repair] processes]` is naturally preferred, so partial coverage already helps the
compound cases.)*

**Consequence for the NF's priority (measured, §4c).** Hyphenation collapsed S5 ~3×, so **most of S5's
structural blow-up was the un-lexicalized term, not genuine bracketing ambiguity.** The NF's real target is
the **residual 4 skeletons** (the `[[DNA repair] processes]` bracketing + copula predication + the
`attractive`/`synthetic-lethal` gradable pair). At the page level, hyphenation is **ambiguity/cost relief,
not a gap or ENCODED change** (v3 reranked: 62 units → 0/57/5/0, identical unit classification to v2). So the
lever order is: **lexicalize/hyphenate first** (cheapest, done), then the sense reranker, then the NF on the
now-smaller genuine residual.

## 5. Semantic adequacy — what must be proved, not asserted

The refined noun's **type** embeds each modifier (`Σx:C. R`); different bracketings yield different
Σ-nestings, and the kernel felicity check is the exact oracle. The NF's soundness claim, sharpened by the
survey (§2a), is narrower than "intersective adjectives":

- **For *strictly* intersective, context-stable modifiers, `adj(compound(N))` and the interleavings are
  meaning-equivalent** — intersective modification is predicate conjunction (Kennedy 2012), and bracketing-
  invariance follows from `∩` being associative/commutative. **This is a corollary of a recognized property,
  not a named theorem — state it as an inference.** This is the collapse the NF is entitled to make.
- **Relative gradable adjectives are NOT safely intersective.** "tall/big/`attractive`" are covertly
  *subsective*: comparison-class-dependent (Kamp & Partee 1995), so attachment can fix the class
  (`tall jockey` ≠ `tall basketball player`). **The note's own example `attractive synthetic lethal targets`
  contains one** — so `attractive` must be **screened out of the unconditional collapse** (or its comparison
  class pinned to the head noun regardless of bracketing). This is the corpus's real hazard, not abstract.
- **Non-intersective / collocational modifiers are NOT equivalent** — handled by §4 (lexical unit), so the
  NF never sees them as an interleaving to collapse.

**A mechanizable test — and we have the reference implementation in-repo.** Chatzikyriakidis & Luo's Coq
code ships as an appendix of their book, in the repo at
[`references/publications/TT Appendices/…Coq Codes.pdf`](../../references/publications/TT%20Appendices/)
(App. A7). It encodes exactly our setting — `CN := Set`, subtyping by `Coercion Surgeon >-> Human`,
adjectival refinement as Coq **record = Σ-type** — and gives the discriminator as *concrete code*, which
maps onto the restrictor shapes our importers already emit:

| class (C&L App. A7) | their Coq encoding | our restrictor `R` in `Σx:C. R(x)` | collapse? |
|---|---|---|---|
| **intersective** (`Irish`, A7.3) | `Irish : Human → Prop`; `{h:> Human; I: Irish h}` | a plain predicate on `C`, **noun-type-independent** | **yes** |
| **subsective** (`skilful`, A7.3) | `skilful : ∀A:CN, A → Prop`; `skilful Man m` | **CN-polymorphic**, instantiated at the head | no (comparison-class-pin) |
| **gradable** (`tall`, A7.6) | `tall h := ge (height h) (STND HEIGHT TALL)` | `gt(deg_a…(x), std_a…)` — **a degree vs a `std_`/`STND`** | no (screen) |
| **privative** (`fake`, A7.4) | disjoint sum `G := sum G_R G_F` | a sum-membership test, not a Σ-refinement | no |

The theorems in the appendix *prove* the split operative: the intersective subtype inference goes through
(`delegate1 … Qed.`) while the subsective one does not (`skill2 … Abort`). **Two payoffs for D1:** (i) the
intersective/subsective decision is a **typing check in our kernel** (is `R` a plain `C→Prop` or
CN-polymorphic?), not a hand-maintained adjective list; (ii) **the gradable screen is purely syntactic on
the restrictor** — our comparatives already emit C&L's `ge(height)(STND …)` as `gt(deg_a…(x), std_a…)`, so
`attractive`'s restrictor `gt(deg_a00166146(x), std_a00166146)` is detectable by its `std_`/degree-standard
subterm and screened out mechanically.

Because **no prior work prunes NP bracketing with a type oracle (§2a)**, this split must be **witnessed on
the closed-class/compound battery** (no felicitous meaning-distinct parse lost), not argued in prose. The
adequacy battery — not the literature — is the gate on landing the guard.

## 6. Composition with the sense lever

The two levers are orthogonal and multiply (S5: 3 structural × 16 sense = 48):

- **NF (this note)** collapses the structural skeletons at combine time → extraction enumerates the sense
  product **once per canonical tree** instead of once per bracketing.
- **Reranker/cap** (built) collapses the sense product to the contextually-right sense(s).
- **Category-choice is the seam.** A word's adjective-sense routes to `Attrib`, its noun-sense to
  `KindCompound`; the reranker/cap chooses which senses (hence categories) enter the chart, and the NF then
  normalizes the bracketing over whatever categories survived. The NF operates **post-cap**, so it must
  compose with widen-on-failure (a widened cap re-admits categories; the NF re-normalizes). Neither lever
  alone reaches a single ENCODED reading — witnessed in §1.

## 7. Scope boundaries

- **Not** the sense product (reranker/cap) nor the mass-shim over-generation (a separate *sense-side* fix,
  [d63-parse-gap-closure.md §6](d63-parse-gap-closure.md)).
- **Not** verb selectional restrictions (GH#93) — off this corpus's critical path (§4a: the explosion is
  nominal, not verbal).
- **Not** a new grammar construction — every S1–S5 sentence already parses closed (§4b); this is purely
  ambiguity/search control.

## 8. Decision points (to resolve in-session before coding)

- **D1 — the intersectivity criterion, as a typing check on the restrictor. ✅ IMPLEMENTED
  (`2026-07-09`).** `ModifierClass` + `modifier_class(adj_sem) -> ModifierClass` in
  [`kernel/src/dcg/category.rs`](../../kernel/src/dcg/category.rs) — the Chatzikyriakidis & Luo
  discriminator (§5 table) run on our own terms, **not** a hand-maintained adjective list. It strips the
  entity binder and classifies the restrictor by shape: **Gradable** if it mentions the degree machinery
  (`measurements:gt`/`lt`, `deg_*`, `std_*`) — the cheap first cut that screens the corpus hazard
  `attractive`; **Privative** if it eliminates a disjoint sum (`Case`/`Data`); **Subsective** if the head
  class appears in it (`EigonClass` — the CN-polymorphic C&L `skilful`); **Intersective** if it is a clean
  first-order predicate (plain `Entity→Prop`, possibly conjoined); else **Unknown** (fail-safe).
  `is_collapsible()` = *Intersective only* — the NF (D3) may collapse nothing else. 7 unit tests cover each
  class incl. the `And(intersective, gradable)` stack (gradable dominates). Pure/structural, no layer
  lookup, behavior-neutral (not yet wired into the combine path — D3 consumes it).

  **Corpus diagnostic (Derived, `2026-07-09`, `d1_modifier_class_over_corpus` + `LexicalIndex::
  debug_modifier_classes`, over `wordnet-umls-all-2026-07-08`).** Ran the classifier over the v3 corpus's
  *real* adjective entries (every WordNet sense). Verdicts correct: `attractive` → all 3 senses **Gradable**
  (screened ✓); `colorectal`/`endometrial` → **Intersective** (relational, collapse-eligible ✓); `genetic`
  → 3 Intersective (the "of genetics" pertainym senses) + 1 Gradable; `immune`/`specific` similarly mixed.
  **Tally: Gradable 65, Intersective 8** — the importer marks ~89% of adjective *senses* gradable. Three
  consequences: **(i)** D1's intersective-collapse route is **narrow on this corpus** — it fires only where
  the reranker selects a relational sense (`genetic`→relational, `colorectal`); evaluatives (`attractive`,
  `strong`, `novel`, `rare`) have no intersective sense and are always screened, so the residual is
  dominated by (correctly-screened) gradables + compound bracketing, and the sense reranker does more here
  than the NF. **(ii)** Coverage gap: morphology-derived adjectives (`synthetic-lethal`, `double-stranded`,
  `microsatellite-stable`, `hypermutable`) aren't committed `LexicalEntry`s, so the diagnostic (committed
  entries only) doesn't see them — `synthetic-lethal` is Gradable per the v3 parse (`gt(deg,std)` via head
  `lethal`). **(iii)** Observation, separate from D1: `somatic`/`homologous` marked Gradable looks
  semantically wrong (they are relational) — the importer's gradability heuristic (`convert.rs` `push_adj`)
  over-marks; D1 faithfully reports the lexicon, so the fix (if any) is upstream in the importer.
- **D2 — the collocation criterion is a *coverage policy*, not a lookup. MEASURED (§4), and it forces
  action.** Coverage is partial: `synthetic lethality` / `cell death` / `dna repair` are units, but the
  adjective form **`synthetic lethal` is absent** — so the NF alone gives S5 the wrong (all-adjective)
  meaning. **Resolution (corrected `2026-07-09`): hyphenate in the CNL — `synthetic-lethal`** — which the
  hyphen morphology reads as one compound adjective; **zero code, no injection**, measured to collapse S5
  144→48 candidates / 12→4 skeletons (§4). An MWE **alias** `synthetic-lethal → C4280020` is worthwhile only
  for **grounding**, not parsing. Carry the §4 cautions for any collocation that must be *grounded*:
  coverage-bound ("cannot identify what is not in the lexicon"), and flat words-with-spaces listing is safe
  for rigid terms but risky for productive/discontinuous ones (Sag et al. 2002).
- **D3 — mechanism: build-then-subsume. ✅ IMPLEMENTED (`2026-07-09`).** Eisner's exact restricted-grammar
  fallback — refuse to keep a reading semantically equivalent to one already built — chosen over a hard
  combine-time block (risks dropping a genuine reading) or a `Cost` penalty (doesn't kill the enumeration).
  `LexicalIndex::subsume_duplicates` in [`kernel/src/dcg/lookup.rs`](../../kernel/src/dcg/lookup.rs) drops a
  closed reading whose sem **structurally equals** one already kept, wired into both the packed and unpacked
  forest collection before the sort/cap. `reduced_felicitous`/`classify_felicitous` already normalize every
  sem to its NbE normal form, so equal *meaning* is equal *structure* — sound (never drops a distinct
  reading), and cheap (the normal forms are already computed; no fresh NbE pass). Uses full-IRI structural
  `Exp` equality, **not** the lossy `pretty_term` (which shortens an IRI to its local segment and could
  false-merge distinct senses).

  **Measured (Derived, `2026-07-09`, `analyze_chart_cells_first_five` over v3, merged kernel):** it collapses
  large **derivational** duplication on the dense-lexicon units — S1 `closed×240 → ×76` (−68%), S2 `×150 →
  ×69` (−54%), S4 `×8 → ×4`, S5 `×24 → ×20`; S3 unchanged (already distinct). The kernel battery (1605 lib
  + closed-class semantic exact-count tests) stays green — no assertion changed, confirming soundness (the
  removed readings were genuine duplicates). **This is the first structural ambiguity reduction that is not
  lexicalization** — different derivations of the *same* meaning (Eisner's spurious ambiguity), which the
  forest previously kept as separate readings (there was no sem dedup).

  **Full-page authoritative measure (Derived, `2026-07-09`, v3 + D3, `--features use-llm`, merged kernel,
  28.6 min): the first ENCODED units.** 62 units → **ENCODED 2, AMBIG 55, GRAMMAR-GAP 5, MISSING 0** — vs
  v3-without-D3's `ENCODED 0 / AMBIG 57 / GAP 5`. **Two units reach a single clean reading** for the first
  time in the whole arc: *"Germline mutations in the MMR genes MSH2, MSH6, PMS2 or MLH1 cause Lynch
  syndrome."* and *"We hypothesized that MSI and MMR deficiency may create vulnerabilities."* Per-unit AMBIG
  falls sharply under reranker + subsume: **S5 `AMBIG×32 → ×8`**, distribution now min 2 / median 28 / max
  156. Same 5 gaps (#3/#4/#7/#8/#9), 0 missing; 169 handled `felicity_readback` panics. **So `ENCODED > 0`
  (the phase-3 exit-gate) is first achieved by the stack lexicalize (v3) + build-then-subsume (D3) +
  reranker — none of them alone.**

  **NB — this does NOT implement §3.3's adjective-outside-compound collapse, and that rule is a *no-op* on
  this corpus's residual.** Witnessed (`2026-07-09`, the 4 distinct v3 S5 sems): the adjective stack
  `And(gt, gt)` is **identical across all 4** readings; the residual variation is copula predication
  (`kind_of` vs `subclass_of`) × object-compound bracketing + one VP-shaped artifact — none of it the
  adjective/compound interleaving §3.3 targets, and the 4 are definitionally *distinct* (so subsume keeps
  all 4). So S5's residual is **genuine ambiguity** (copula + object-compound), reduced only by the sense
  reranker + object-compound handling, not by an adjective NF. §3.3 stays valid for a corpus that has
  intersective-adjective-over-compound stacks; this one does not.
- **D4 — non-regression. ✅ CONFIRMED for what was built (`2026-07-09`).** The implemented D3
  (build-then-subsume) operates on the *final normalized readings*, not the combine rules — it does **not**
  touch `is_compound_refined`, the `Attrib` flat-Σ (`parser.rs:588`), the Eisner-normal pure-adjective
  chain (§2a), or the pure-compound left-branching NF. The full kernel battery (1605 lib + closed-class
  semantic + integration) is green, so none regressed. *(If §3.3's `is_compound_refined` extension is ever
  built — not needed on this corpus, see D3 — this non-regression check applies to it.)*

## 9. Verification plan

1. **No parse lost** — the closed-class / determiner / compound battery stays green (the §5 adequacy gate).
2. **Structural collapse** — re-run `analyze_chart_cells_first_five` (`EIGENIUS_PARSE_DEBUG=1`): S5's
   within-frame skeletons drop 3→1; the classify-candidate count per sentence falls to ≈ the sense product
   alone (S5: 144 → ~16-scale; S1: 256 → the S1 sense product).
3. **Reading-count drop** — re-measure the full page (reranked, `--features use-llm`): AMBIG×N median falls;
   the target is AMBIG → single ENCODED per unit once the reranker also collapses the sense side.
4. **No new gap** — the 5 first-CNL sentences still parse closed.

## 10. Prior art (verified references)

Anchors used above; each resolves to an ACL-Anthology id, DOI, or arXiv id (verified `2026-07-08`).
Unverified/near-miss sources are deliberately omitted (grounding discipline: never anchor on an
unverified cite).

**Normal-form / spurious ambiguity in categorial grammar.**
- Eisner, J. (1996). Efficient Normal-Form Parsing for CCG. ACL 1996, 79–86. ACL P96-1011;
  DOI 10.3115/981863.981874; arXiv cmp-lg/9605038. *(Theorems 1/2; the per-recipe equivalence boundary.)*
- Hepple, M. & Morrill, G. (1989). Parsing and Derivational Equivalence. EACL 1989. ACL E89-1002.
- Hendriks, H. (1993). Studied Flexibility. PhD, ILLC Amsterdam. ILLC DS-1993-05. *(Lambek-NF lineage.)*

**NP-internal / noun-compound bracketing.**
- Hockenmaier, J. & Steedman, M. (2007). CCGbank. *CL* 33(3):355–396. ACL J07-3004;
  DOI 10.1162/coli.2007.33.3.355. *(Flat NPs; right-branching compound default = "linguistically incorrect".)*
- Lauer, M. (1995). Corpus Statistics Meet the Noun Compound. ACL 1995. arXiv cmp-lg/9504033. *(~2/3 left.)*
- Vadas, D. & Curran, J. R. (2007). Adding Noun Phrase Structure to the Penn Treebank. ACL 2007. ACL P07-1031.
- Vadas, D. & Curran, J. R. (2008). Parsing Noun Phrase Structure with CCG. ACL-08:HLT. ACL P08-1039.
- Honnibal, M., Curran, J. R. & Bos, J. (2010). Rebanking CCGbank. ACL 2010. ACL P10-1022.

**Supertagging / lexical-category choice.**
- Bangalore, S. & Joshi, A. (1999). Supertagging. *CL* 25(2):237–265. ACL J99-2004.
- Clark, S. & Curran, J. R. (2004). The Importance of Supertagging for Wide-Coverage CCG Parsing.
  COLING 2004. ACL C04-1041.

**Multiword expressions in the lexicon.**
- Sag, I. et al. (2002). Multiword Expressions: A Pain in the Neck for NLP. CICLing 2002, LNCS 2276.
  DOI 10.1007/3-540-45715-1_1.
- Constant, M. et al. (2017). Multiword Expression Processing: A Survey. *CL* 43(4):837–892. ACL J17-4005;
  DOI 10.1162/COLI_a_00302. *(Discovery vs identification; the coverage bound.)*

**Dependent-type semantics of modification (our Σ-refinement; the intersective/subsective typing test).**
- Luo, Z. (2012). Common Nouns as Types. LACL 2012, LNCS 7351, 173–185. DOI 10.1007/978-3-642-31262-5_12.
- Luo, Z. (2012). Formal Semantics in MTTs with Coercive Subtyping. *Ling. & Phil.* 35(6):491–513.
  DOI 10.1007/s10988-013-9126-4.
- Chatzikyriakidis, S. & Luo, Z. (2017). Adjectival and Adverbial Modification: The View from MTTs.
  *JoLLI* 26(1):45–88. DOI 10.1007/s10849-017-9246-2. *(Intersective ⇔ noun-type-independent; the D1 test.)*
- Chatzikyriakidis, S. & Luo, Z. (2020). Formal Semantics in Modern Type Theories, App. A7 "Coq Codes".
  ISTE/Wiley. **In-repo primary source:** `references/publications/TT Appendices/…Coq Codes.pdf` — the
  concrete intersective/subsective/gradable/privative encodings (the operational D1 discriminator, §5).

**Adjective semantics (intersective vs subsective; the gradable caveat).**
- Kamp, J. A. W. (1975). Two Theories about Adjectives. In Keenan (ed.). DOI 10.1017/CBO9780511897696.011.
- Kamp, H. & Partee, B. (1995). Prototype Theory and Compositionality. *Cognition* 57(2):129–191.
  DOI 10.1016/0010-0277(94)00659-9; PMID 8556840. *(Relative gradables are covertly subsective.)*
- Morzycki, M. (2016). Modification. CUP. DOI 10.1017/CBO9780511842184.
