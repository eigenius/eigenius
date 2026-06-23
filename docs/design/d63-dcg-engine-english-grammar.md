# D63 — The DCG engine: a categorial grammar of English over EigenTT

*Status: design + partial implementation (the engine core and the content lexicon are built; the
function-word track and the combinator/feature extensions are the work this document scopes).*

*Relation to D62/D61.* D62 (*encoding engine: prose → trees*) is the **encoding architecture** — the
LLM proposer, the faithfulness boundary (D61), and the encoding institution that commits trees as
chain resources. **D63 is the deterministic generation engine that architecture consumes**: a
**dependent categorial grammar** (DCG) that maps English prose to type-checked EigenTT trees, with
the kernel as the felicity oracle and **no LLM in the loop**. In D62's terms it is the *trusted*
generation path (§6's generation/verification split); the LLM path is the untrusted augmentation
(D62 §8.7.8) and the faithfulness check is D61. This document lifts the still-accurate pieces of D62
(§3 formal spine, §8.6 realized engine, §8.7 lexicon import, §8.8.1 lookup bridge, §4 resources) into
one targeted, end-to-end spec for *building the grammar*.

---

## 1. Goal & scope

**Goal.** A deterministic, kernel-gated pipeline `String → Vec<EigenTT tree>`: English prose → the
forest of type-checked propositions, built by categorial composition over a typed lexicon.

**In scope:** the categorial type system (`lexicon:Cat` + the `⟦·⟧` homomorphism), the parser
(combinators + chart + the felicity oracle), the lexicon (the WordNet *content* layer + the
hand-authored *function-word* track), and the validation harness (felicity gate + the FraCaS
behavioral battery).

**Out of scope** (lives in D62/D61): the LLM proposer; the faithfulness oracle; the encoding
institution's commit/select machinery (FIBER-INTO + AutoOnLoad + the selector — D62 §8.8.2–8.8.5).
The engine returns a *forest*; selecting and committing one parse is the institution's job (§6 here
is just the seam).

**Posture.** This is grammar/type-theory engineering — multi-session, no time pressure. The
load-bearing design is the *formalism* (features, combinators, the dependent-type semantics of
function words), not entry typing; getting that shape right is the priority (per the project posture).

## 2. Formal foundation

The engine composes prose into a typed substrate by a principled, checkable derivation — not an
opaque extraction. The full treatment is D62 §3 (grounded against the MTT free appendices, now local
at `references/publications/TT Appendices/`); the essentials:

**One stack, three roles.** Carpenter's type-logical semantics says *how words compose* (categorial
slots + Curry–Howard derivation-as-term); MTT-semantics (Chatzikyriakidis & Luo) says *the categories
are dependent types* (common nouns as types, coercive subtyping); TTR (Cooper) says *those types are
records* — exactly Eigenius's `Class`-as-Σ-typed-record. Luo's **dependent categorial grammars**
(Chatz & Luo Ch. 7.3) are the glue. **DTS** (Bekki's Dependent Type Semantics; `lightblue`) is the
*working instance* of this family — a CCG→Σ-type parser with a native type-check — and our closest
prior art (D62 §4.1).

**EigenTT realization (built — D62 §8.6).** The categorial type is a kernel inductive carried as a
`type_expr`:

```
data lexicon:Cat : Type 1 { cat_s ; cat_n ; cat_np(Set) ; fwd(Cat,Cat) ; bwd(Cat,Cat) }
```

with the homomorphism `⟦·⟧ : Cat → EigenTT type` ([`denote_cat`](../../kernel/src/dcg/category.rs)):

| `cat` | `⟦cat⟧` | role |
|---|---|---|
| `cat_s` | `Prop` (`Sort 0`) | a proposition |
| `cat_n` | `Set` (`Sort 1`) | a common noun = a **type** |
| `cat_np(T)` | `T` | a name = an **entity** of type `T` (type-indexed atom) |
| `fwd(A,B)` / `bwd(A,B)` | `⟦B⟧ → ⟦A⟧` | a functor (direction drives the parser, not the type) |

The **four archetypes** map onto kernel constructors: common noun `N` → `EigonClass` (CN-as-type);
name `NP` → `EigonResource`; transitive verb / predicative adjective → `EigonAxiom` (a typed chain
constant). **Felicity** is the kernel's job: an entry is admitted iff `⟦cat⟧ ≡ sem_type` **and** its
`sem` inhabits `⟦cat⟧` ([`gate_entry`](../../kernel/src/dcg/lexicon.rs)). **CN-as-types subsumption**
honors `core:subclass_of` as the `EigonClass` subtype rule (`Layer::is_subclass_of`), so a predicate
typed at a supertype accepts subclass-typed arguments.

## 3. Current state — the `kernel::dcg` stub

Honest inventory of [`kernel/src/dcg/`](../../kernel/src/dcg/) and the importer, *built* vs *stub*:

| Module | Provides | Status |
|---|---|---|
| `category.rs` | `denote_cat`, `cat_subsumes`, `type_eq` | **built** |
| `parser.rs` | `Item`, `apply` (fwd/bwd **application only**), `cky_parse` | **built**, application-only |
| `lexicon.rs` | `resolve_sem`, `gate_entry`, `entry_to_item` | **built** |
| `lemmatizer.rs` | `Lemmatizer` trait, `Identity` | **built** (seam) |
| `lookup.rs` | `LexicalIndex`, `tokenize`, `parse(&str) → forest` (multi-span seeding + chart + Prop filter) | **built** |
| `eigenius-wordnet` | the WordNet importer + `MorphyLemmatizer` | **built** |

**What the content lexicon already contains** (full WordNet 3.0 import): 74,385 noun classes, 7,730
proper-noun individuals, 33,006 verb/adjective axioms, 204,088 lexical entries — kernel-validated.

**What's missing (this document's work):**
- **Features** on `Cat` (`S[dcl]`/`S[q]`, finiteness, agreement, case) — needed to type questions,
  block agreement violations, and thread wh-gaps.
- **The combinator set** — only forward/backward *application* exists; no **type-raising (T)** or
  **composition (B)**, hence no wh-extraction, non-constituent coordination, or scope.
- **MTT quantifier semantics** — no determiners/quantifiers; a bare common noun is a `Set`, not an
  entity, so it cannot fill a verb's `NP` slot (the **N-vs-NP gap**, §4).
- **The function-word track** — the closed class is absent (WordNet is content-word-only).

**The honest current bound:** the engine parses *simple declaratives* whose arguments are names
(WordNet `@i` instances) — e.g. *"HeLa depends on BRCA1"* → `depends_on(brca1, hela) : Prop`, kernel
-confirmed. General sentences with common-noun arguments need the function-word track + the
extensions above.

## 4. The lexicon — two layers

**(a) Content layer — WordNet (built; D62 §8.7).** A deterministic, kernel-gated import: noun synset
→ `core:Class` (`@` hypernym → `core:subclass_of`); `@i` instance → `EigonResource` individual; verb
/adjective synset → `eigentt:Axiom` (category from the sentence frames); lemma → `lexicon:LexicalEntry`.
`MorphyLemmatizer` (a faithful Morphy port) is the surface→lemma reference impl. Residuals: multi-class
NP typing (kernel issue #91), instance NP-vs-class emission, predicate (troponymy) subsumption.

**(b) Function-word track — hand-authored (Path B; the new work).** The closed class — determiners,
quantifiers, copula, auxiliaries, negation, coordinators, wh-words, complementizers — is **not** in
WordNet and carries the compositional weight. We author it **Apache-owned**, sourced by the
syntax/semantics split:
- **categories + features + slash-modes** from the CCG tradition — OpenCCG `core-en`
  (`references/openccg/`, LGPL — *read as reference, reimplement; do not ship*), Steedman, Baldridge;
- **semantics** in our dependent-type setting from **DTS** — `lightblue`'s
  `src/Parser/Language/English/Lexicon.hs` (the determiner-as-Σ pattern) + Chatz & Luo + the TT
  appendices;
- **inventory + distinctions** from CGEL (Huddleston & Pullum) — the authoritative *what to capture*.

**The N-vs-NP gap (why this layer is load-bearing).** `⟦cat_n⟧ = Set` (a type); a verb wants
`⟦cat_np(T)⟧ = T` (an entity). A bare common noun is a type, not an entity, so it cannot saturate a
verb — you cannot get an entity from a (possibly empty, possibly many-membered) type for free. The
**determiner injects the binding**: *a dog barks* = `∃x:Dog. barks(x)`, *every dog barks* =
`∀x:Dog. barks(x)`. Names (`@i` instances) escape this because they *are* entities. So the
function-word track is what turns the content lexicon into a grammar of general sentences.

## 5. Slice 0 — the formalism decisions

The function-word categories presuppose machinery the stub lacks. These three decisions gate all
authoring and must be settled (and written up) first.

**5.1 Features on `Cat` — parametrized atoms, erased-by-`⟦·⟧`, lattice-unified (settled).**

*The split (the crux).* **Mood is the only atomic feature that alters `⟦·⟧`.** Agreement
(number/person), case, and finiteness are **syntactic routing only — fully erased** by the
homomorphism: `⟦S[dcl,3sg]⟧ = ⟦S[dcl,pl]⟧ = Prop`. (Finiteness is syntactic *because we don't model
tense*; if tense/aspect is added later, that — not finiteness per se — carries the semantic import.)
**Gaps are not atomic features** — they are the slash structure (`⟦S/NP⟧ = Entity→Prop`), handled by
the combinators (§5.2), not the feature payload.

*Representation: parametrize the atoms, in the kernel inductive.* Each atom carries exactly its
relevant features — no generic `feat` wrapper (not every atom takes every feature), no sibling/external
layer (breaks K1 / complicates the recursor). Concretely this extends the **`lexicon:Cat` kernel
inductive** (the Rust-enum sketch maps to ESL `data` decls):
```
data lexicon:Mood { dcl ; q ; imp }
data lexicon:Num  { sg ; pl ; num_any }
data lexicon:Fin  { fin ; bse ; to ; ng ; pss ; fin_any }
data lexicon:Cat : Type 1 {
    cat_s(Mood, Fin) ;        // mood semantic, fin erased
    cat_n(Set, Num) ;         // type semantic, num erased
    cat_np(Set, Num) ;        // (+ Case when pronouns land — deferred)
    fwd(Cat, Cat) ; bwd(Cat, Cat)
}
```
Erasure is then trivial: `denote_cat(cat_s(m,_)) = denote_mood(m)`, `cat_n(T,_) ↦ Set`,
`cat_np(T,_) ↦ T`. The felicity invariant `⟦cat⟧ ≡ sem_type` is unaffected (erased features never reach
`⟦·⟧`); `cat_subsumes` gains **feature-meet** alongside the existing `is_subclass_of` on the type `T`.

*Value model + unification: a subsumption lattice, `Any` as Top, no logic variables.* Unification is
the **meet `⊓`**: `Sg ⊓ Any = Sg`, `Sg ⊓ Pl = ⊥` (fail); `*_any` is the underspecified top. No
Prolog-style feature variables (R5: simple, deterministic). The one genuinely category-polymorphic case
is **coordination** (`and : (X\X)/X`, Slice 4): handle it as a **coordination rule that matches the two
conjuncts' categories with feature-meet** — *not* plain binary application, but still no logic variables.
For Slices 1–2 (no coordination) the meet lattice alone suffices.

*Inventory: minimal.* Mood {dcl, q, imp}, Fin {fin, bse, to, ng, pss}, Num {sg, pl} (+ `*_any`).
**Defer** Case (until pronouns) and Person (fold into agreement later). Not the CCGbank set
(Penn-tailored, messy) — values only. *(Note: the `cat_np` `Case` slot is deferred from the inductive
too, per K3's cheap re-import — diverges from the sketch, which showed it; reconcile if you'd rather
bake the always-`Any` slot in now.)*

*Import defaults: WordNet → `Any`, Morphy instantiates.* Imported atoms carry `*_any` (the verb "run"
is fully underspecified). The morphological stage instantiates: Morphy reads "dogs" → (lemma "dog",
`Num::Pl`); lookup **meets** the base category `N[Dog, Any]` with the token's features → `N[Dog, Pl]`.
Keeps WordNet unbloated (R4). **Implementation consequence:** the `Lemmatizer` seam + `LexicalIndex`
must return **(lemma, features)**, not bare lemma strings — Morphy knows which detachment rule fired, so
it *has* the feature; the current `Vec<String>` API discards it. This is a Slice-1 change touching
already-built code (`dcg::lemmatizer`, `dcg::lookup`, `MorphyLemmatizer`).

*Question denotation: deferred to Slice 5.* The *eventual* `⟦S[q]⟧` is `Entity→Prop` (or a set of
Props); until Slice 5, `S[q]` is a **syntactic tag only** (denotation trapped/`unimplemented!`), used so
auxiliary inversion parses without polluting declaratives. Consistent with §5.3's deferral.

*`denote_cat` location: engine-side Rust* until the inventory locks (post-Slice-3) — promoting to an
in-kernel recursor while the lattice is still churning would force a kernel rebuild per added feature
(§8.6 already defers the in-kernel recursor).

*Source:* CCGbank feature scheme (Hockenmaier & Steedman — values, not the full set); Steedman
(coordination); the CN-as-types substrate (§2).

**5.2 The parse substrate and combinator set.**

*Substrate — CKY (settled), not Earley/LRE(k).* The chart stays **CKY-style bottom-up** (as the stub
is). The LRE(k) hybrid (McLean & Horspool, `references/publications/FastEarleyParser.pdf`) gets its
speed by precomputing LR(k) item sets **from a grammar's productions** over a finite nonterminal set —
and CCG has neither: it is lexicalized + combinatory (a small schematic combinator set + a huge
lexicon) with an **unbounded** category set (composition/type-raising *generate* categories), so there
is nothing to precompute over short of CFG-approximating the grammar, which discards the very
categorial/dependent-type structure that is the point. LR/Tomita methods also win only on
**low-conflict** grammars, whereas categorial combination is high-ambiguity (the LR advantage
evaporates). CKY is the established CCG substrate (C&C, EasyCCG, depccg), fits our binary/unary
combinators directly, and — at sentence scale (n ≈ 10–30), where the n³ asymptotics are moot — its
transparency beats Earley + SPPF / LR-table machinery for a *verifiable kernel component*. Crucially,
the real bottlenecks are **off the chart** and untouched by this choice: lexical ambiguity → the
felicity gate + sense priors + selection (the supertagging analog); spurious ambiguity → Eisner
normal form. *(Earley would earn its place only under a production-based phrase-structure grammar —
not our path — or for incremental/streaming parsing — not a requirement.)*

*Combinators.* Within CKY, extend `apply` (today application-only) with **type-raising (T)** and
**forward/backward composition (B)** — type-raising via **bounded unary closure** in each cell (a
fixed target set → termination) — plus an **Eisner normal-form** constraint to suppress the spurious
ambiguity T+B introduce. Decide **multimodal slashes** (Baldridge — the modes `core-en` uses, e.g.
`mode="^"/"<"/"*"`) vs. a coarser global regime. *Source:* Steedman; Baldridge
(`references/publications/Baldridge_dissertation.pdf`); Eisner
(`references/publications/Eisner-…Normal Form Parsing.pdf`).

**5.3 The semantic universe and quantifier semantics — `⟦cat_s⟧ = Prop` (settled).**

*Decision.* A sentence denotes a **`Prop`** (`Sort 0`), **not** a proof-relevant `Set`. Determiners
quantify with Σ/Π **over the noun-type**, but the sentence-level existential closes into `Prop` via
the **impredicative ∃**, so the engine's output is always a `Prop`.

*Why — the D46 constraint.* The reasoning layer (goals, objectives, hypotheses, the stored
`lexicon:prop` propositions) is built on [D46](d46-prop-universe-and-proof-irrelevance.md)'s
**proof-irrelevant `Prop`**, and the engine exists to feed it — an encoded statement *becomes* such a
proposition. A proof-*relevant* `Set` meaning (the DTS/lightblue default, where Σ-existentials live in
`Set`) cannot be handed where the reasoning layer expects a `Prop`: a universe *and* a
proof-relevance mismatch at the boundary. So `⟦cat_s⟧` stays `Prop`. (This is what makes Option A —
proof-relevant sentence meanings — untenable for us, despite the DTS lineage.)

*The forms — CN-as-types.* Nouns are types (`EigonClass`), so determiners quantify over the noun-type
directly, with `N ≤ Entity` supplied by our existing `is_subclass_of` coercion:
- `every = λN. λV. Π x:N. V(x)` : `Set → (Entity→Prop) → Prop`
- `a / some = λN. λV. ∃ x:N. V(x)` : `Set → (Entity→Prop) → Prop`, where
  `∃ x:N. P := Π C:Prop. (Π x:N. P → C) → C` is the **impredicative existential** (D46 — Π *into*
  `Prop` is `Prop`).

This is **not** lightblue's `Σ (Σ Entity (N x)) …` Entity-plus-predicate form — that's the
entities+predicates variant; we are CN-as-types, so the noun *is* the domain (simpler, and it reuses
the subsumption we already built). The conflict was always narrow: verb predicates already target
`Prop` (`depends_on : … → Prop`), universals are Π-into-`Prop`, and only the existential needed the
impredicative encoding — so the **Slice-2 milestone stands as written**:
`every gene affects a cell line = Π g:Gene. ∃ c:CellLine. affects(g,c) : Prop`.

*Σ is retained* where it is natural — the noun-records themselves (`EigonClass` *is* a Σ-type in the
kernel, `check.rs`) and intermediate composition — just not as the sentence-level existential.

*No term-notation extension, no prover, NbE for free.* EigenTT already has `Sig` / `Pi` / `Pair` /
`Fst` / `Snd` (`kernel/src/nbe/term.rs`), so the forms are directly expressible. The kernel is an NbE
machine: compose `sem` in the `Val` domain through the chart and `readback` a β-normal `Exp` once for
the gate (no substitution, no capture). Producing and **type-checking** the tree is *decidable* — **no
proof-search engine**. A prover is needed only downstream (entailment for the FraCaS battery; anaphora
resolution if added) and fits as a *dispatched institution* (like the Lean/R/Julia computations),
never a core engine dependency.

*Cross-reference is structural, not Σ-witness-based.* DTS needs proof-relevant Σ because witnesses are
its only handle on entities; our antecedents are **committed resources referenced by IRI** (the chain
— a `lexicon:Sentence` is a resource, D62 §8.8). So linguistic anaphora resolves to a *resource
reference*, which is the payoff that would otherwise motivate proof-relevant meanings.

*Escape hatch (door open, D46 untouched).* If intra-sentential **donkey anaphora** ever needs a
reusable witness, compose *that sentence* in `Set` (genuine Σ) and **truncate to `Prop` at the
sentence boundary** (`‖Σ x:N. P‖ := Π C:Prop. (Σ x:N. P → C) → C : Prop`). Proof-relevance stays
local to encoding one sentence; the reasoning layer only ever sees `Prop`.

*Source:* [D46](d46-prop-universe-and-proof-irrelevance.md); Chatz & Luo + the TT appendices
(CN-as-types, records = Σ); lightblue DTS (the entities+predicates contrast); `kernel/src/nbe/term.rs`
(`Sig`/`Pi`/`Pair`).

## 6. The pipeline & the integration seam

**The pipeline (built — D62 §8.8.1; extends with combinators).**
`tokenize` → lemmatize (`Lemmatizer`/Morphy) → `LexicalIndex` lookup with **multi-span seeding**
(a multiword form seeds a multi-token span alongside its parts — MWE-vs-compositional as competing
chart edges) → CKY composition (`apply`; to be extended with T/B + normal form) → the **felicity
filter** (every full-span `S` whose assembled `sem` the kernel types to `Prop`) → the **forest**.

**The seam to D62.** The engine returns the whole forest as transient terms (no selection, no
commit). The **encoding institution** (D62 §8.8.2–8.8.5) selects one parse, records the alternatives,
and commits it as a `lexicon:Sentence` via a FIBER-INTO query gated by AutoOnLoad. D63 stops at the
forest; D62 owns the commit. Keep this boundary thin.

## 7. Validation ladder

Every entry / construction climbs:

1. **Felicity (type)** — `gate_entry`: `⟦cat⟧ ≡ sem_type` ∧ `sem` inhabits `⟦cat⟧` (built; extend for
   features).
2. **Parse / compose** — the construction parses and the assembled `sem` type-checks to the right
   type (`Prop` for declaratives, the question type for `S[q]`).
3. **Behavioral (FraCaS)** — `references/FraCaS/fracas.xml`: **346** inference problems
   (203 yes / 98 unknown / 33 no / 12 undef). Does the grammar derive the right **entailments**?
   (The quantifier section is the determiner milestone's battery; eval-only — reference, do not ship.)
   This is the D61 faithfulness back-stop applied to the grammar.
4. **Coverage + ambiguity** — parse held-out text; measure coverage and spurious-derivation count
   (the normal-form check).
5. **Regression** — the existing fragment keeps parsing.
6. **Grading** — Declared (authored) → Derived (gate + battery) → Verified (human/proof). Never
   Verified on assertion.

A **FraCaS runner** (the behavioral harness) is a tool to build alongside Slice 2.

## 8. Slice plan

Each slice ships **with** its check. The order is dependency-forced.

- **Slice 0 — formalism design (§5).** Features, combinators+normal-form, MTT quantifier semantics.
  *Gates everything.*
- **Slice 1 — features on `Cat`.** Extend the inductive + `denote_cat` + gate + parser; existing
  tests green, featured entries gate.
- **Slice 2 — determiners + quantifiers (the milestone).** `NP/N` + `∃`/`∀`/`ι` + type-raising for
  scope. **Done when:** *"every gene affects a cell line"* → `∀g:Gene. ∃c:CellLine. affects(g,c) :
  Prop`, gate-checked, and the FraCaS monotonicity subset passes. This is the moment common nouns
  reach verb argument slots — *general WordNet sentences become real.*
- **Slice 3 — copula + predication + attributive adjectives** (`is a gene`, `the primary cell line`).
- **Slice 4 — coordination** (needs composition).
- **Slice 5 — wh-questions** (needs T + composition + `S[q]`).
- **Slice 6 — negation, auxiliaries, relatives, the tail.**

## 9. References

Local + license-cleared (the Path B shelf):

| Reference | Role | Status |
|---|---|---|
| WordNet 3.0 (`references/WordNet-3.0/`) | content lexicon + Morphy | OSI ✓ (shippable) |
| OpenCCG `core-en` (`references/openccg/`) | categories / features / slash-modes | **LGPL — read & reimplement, do not ship** |
| `lightblue` (`references/lightblue/`) | DTS semantics (determiner-as-Σ) + chart | BSD-3 ✓ |
| FraCaS (`references/FraCaS/fracas.xml`) | behavioral entailment battery | eval-only — reference |
| CGEL — Huddleston & Pullum (`references/publications/`) | descriptive inventory / distinctions | reference (copyrighted) |
| Baldridge dissertation (`references/publications/`) | multimodal CCG (slash modes) | reference |
| Eisner, *Normal-Form Parsing* (`references/publications/`) | spurious-ambiguity control | reference |
| McLean & Horspool, *A Faster Earley Parser* (`references/publications/`) | parser-substrate comparison (CKY chosen over Earley/LRE(k), §5.2) | reference |
| TT Appendices (`references/publications/TT Appendices/`) | MTT/TTR semantics (records = Σ) | reference |
| Chatz & Luo 2020; Luo CN-as-types / coercive; Carpenter; Cooper | the formal spine (bib) | verified anchors |

*Not used:* CCGbank corpus (LDC-encumbered — only its category scheme, as facts); depccg/C&C
(license/staleness).

## 10. Explore further / out of scope

- **Out of scope here:** the LLM proposer (D62 §8.7.8), the faithfulness oracle (D61), the encoding
  institution (D62 §8.8.2–8.8.5).
- **Explore:** a Lean/Coq correspondence for the derivations (Chatz & Luo's Coq-verified NL inference,
  D28/D30); wide multimodal coverage; mining the closed class via the D62 proposer harness (LLM
  proposes in our notation → the §7 ladder disposes) as a *scale* accelerator once the formalism is
  fixed.
