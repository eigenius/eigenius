# D63 — The DCG engine: a categorial grammar of English over EigenTT

*Status: design + active implementation. Built: the engine core, the full WordNet content lexicon, the
feature set on `Cat`, the determiner/quantifier + coordination + plural-group function words, the full
question set (subject-wh, polar, object-wh extraction via forward composition + Eisner), the copula +
predicative/attributive adjectives + (instance) predicate nominals, and forward composition `B¹` (Slices
1–5 + §8.4; **Slice 3 complete** — copula, predicative + attributive adjectives, instance + kind predicate
nominals; **negation** — Slice 6-neg). Remaining: type-raising `T` + the rest of the combinator set +
relatives, auxiliaries, the tail (Slice 6), and the operational scale-up onto full WordNet (Slice 7). The
§8 slice plan carries the authoritative per-slice status.*

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

## 3. Current state — the `kernel::dcg` engine

Inventory of [`kernel/src/dcg/`](../../kernel/src/dcg/) and the importer. This is a snapshot as of
**Slices 1–5b + the §8.4 phases**; the **§8 slice plan carries the authoritative per-slice status** (so
this section doesn't drift again).

| Module | Provides | Status |
|---|---|---|
| `category.rs` | `denote_cat` (incl. `cat_forall`/`cat_group`/`cat_q`), `cat_subsumes`/`unify_cat`, `type_eq`, generalized coordination + `distribute`/`reciprocate`, `common_super` | **built** |
| `parser.rs` | `Item`; `apply` — fwd/bwd application + dependent `cat_forall` application + distributive group rules; `cky_parse` | **built** (no general T/B composition) |
| `lexicon.rs` | `resolve_sem(_value)`, `gate_entry`, `entry_to_item` | **built** |
| `lemmatizer.rs` | `Lemmatizer` trait, `Identity` (Morphy in `eigenius-wordnet`) | **built** (seam) |
| `lookup.rs` | `LexicalIndex`, `tokenize`, `parse(&str) → forest` (multi-span seeding + CKY + coordination/group/reciprocal rules + the `cat_s`/`cat_q` felicity filter) | **built** |
| `eigenius-wordnet` | the WordNet importer + `MorphyLemmatizer` | **built** |

**What the content lexicon already contains** (full WordNet 3.0 import): 74,385 noun classes, 7,730
proper-noun individuals, 33,006 verb/adjective axioms, 204,088 lexical entries — kernel-validated.

**Done since the original stub** (Slices 1–5b + §8.4):
- **Features on `Cat`** — `Mood` (`dcl`/`q`), `Num` (`sg`/`pl`/`*_any`) with **agreement by feature-meet**,
  `Fin`, `Conn`; the `cat_forall` determiner binder, `cat_group`, `cat_q`. (Case remains absent.)
- **MTT quantifier semantics + the function-word determiner layer** — ∀/∃/¬ over the noun-type
  (`every`/`each`/`all`/`a`/`some`/`no`, subject + object), committed + bootstrapped (`closed-class.esl`);
  the **N-vs-NP gap** is closed (a determiner lifts a `Set`-noun to fill a verb's `NP` slot).
- **Coordination** — generalized conjunction (`S`/`VP`/`TV`, `and`/`or`) + NP-coordination **groups**
  (distributive, basic collective, reciprocal), with the left-branching normal form (§8.4).
- **FraCaS** monotonicity + conjunction-elimination witnesses (§8.4 Phase 5).
- **Subject wh-questions** (`what`/`which`) — the answer-property `cat_q(T)` (§8.5 Slice 5b).

**Still missing (the remaining slices):**
- **Type-raising (T)** + the rest of the combinator set (generalized `B^n`, crossing `B×`, backward
  composition), with the Eisner clauses governing them — for aux-less extraction / relativization and
  scope (Slice 6). **Forward composition (`B¹`)** + its Eisner normal form **is in** (Slice 5c);
  determiners use *lexical* type-raising (`cat_forall`), not the general `T` rule.
- **Type-raising `T`** (+ the rest of the combinator set / Eisner completion) and **relative clauses**, plus
  **auxiliaries** (progressive/perfect/passive/modals) and the **tail** (case, pronouns) — Slice 6 (its
  **negation** sub-slice, **6-neg, is in**; Slice 3 complete).
- **Operational scale-up** of the engine onto the full 204k-entry WordNet layer (Slice 7).

**The honest current bound:** the engine parses declaratives built from the committed
determiners/quantifiers, `S`/`VP`/`TV` coordination, NP-coordination groups
(distributive/collective/reciprocal), and the full **wh + polar question** set (subject-wh, polar
yes/no, object-wh extraction) — over the demo lexicon and, by construction (the grammar is
vocabulary-agnostic), the WordNet content layer. Kernel-confirmed milestones:
*"every gene affects a cell line"* → `∀g:Gene. ∃c:CellLine. affects(g,c) : Prop`; *"HeLa and BRCA1 form a
complex"* → `forms_complex([hela, brca1]) : Prop`; *"which cell line affects HeLa"* → `λx:CellLine.
affects(hela, x) : CellLine → Prop`; *"does HeLa affect BRCA1"* → `affect(brca1, hela) : Prop` (mood `q`);
*"what does HeLa affect"* → `λx:Entity. affect(x, hela) : Entity → Prop`; *"HeLa is primary"* →
`is_primary(hela) : Prop` (copula); *"a primary cell line affects HeLa"* → `∃z:(Σx:CellLine. is_primary(x)).
affects(Fst z, hela) : Prop` (attributive Σ-refinement); *"HeLa is a cell line"* → `is_a(hela, CellLine) :
Prop` (instance predicate nominal); *"genes are cell lines"* → `subclass_of(Gene, CellLine) : Prop`
(kind-subject predicate nominal); *"HeLa does not affect BRCA1"* → `affect(brca1, hela) → logic:False :
Prop` (negation). **Not yet:** type-raising/relatives + auxiliaries + the tail (Slice 6), and parsing at
full-WordNet scale (Slice 7).

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
data lexicon:Mood { dcl, q, imp }
data lexicon:Num  { sg, pl, num_any }
data lexicon:Fin  { fin, bse, inf, ger, pss, fin_any }   // finite / bare / to-inf / -ing / passive
data lexicon:Cat : Type 1 {
    cat_s  : Mood -> Fin -> Cat ;   // mood semantic, fin erased
    cat_n  : Set -> Num -> Cat ;    // common noun of type T (T denotation-erased; carried — see §8.2)
    cat_np : Set -> Num -> Cat ;    // type semantic, num erased (+ Case when pronouns land — deferred)
    fwd : Cat -> Cat -> Cat ; bwd : Cat -> Cat -> Cat
}
```
**`cat_n` carries the noun type `T`** (`⟦cat_n(T,_)⟧ = Set` — `T` is denotation-erased but present).
*An interim draft made it `cat_n(Num)` on a "dead weight" argument; that was wrong — the polymorphic
determiner's category variable unifies with the noun's `T` via this index, and `denote_cat` binds it
as a `Π`. See **§8.2** for the full resolution (implemented: schema + `denote_cat` + importer +
re-import).* `cat_np` likewise carries its type — a name is type-specific, and slot-filling subsumes
on it.

Erasure is then trivial: `denote_cat(cat_s(m,_)) = denote_mood(m)`, `cat_n(_) ↦ Set`,
`cat_np(T,_) ↦ T`. The felicity invariant `⟦cat⟧ ≡ sem_type` is unaffected (erased features never reach
`⟦·⟧`); `cat_subsumes` gains **feature-meet**, plus the existing `is_subclass_of` on `cat_np`'s type `T`.

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
is fully underspecified; nouns are `cat_n(num_any)`, their type carried by the `sem`). The
morphological stage instantiates: Morphy reads "dogs" → (lemma "dog", `Num::Pl`); lookup **meets** the
base category `N[Any]` (sem = `Dog`) with the token's feature → `N[Pl]`. Keeps WordNet unbloated (R4).
**Implementation consequence (deferred to Slice 2):** the `Lemmatizer` seam + `LexicalIndex` must
return **(lemma, features)**, not bare lemma strings — Morphy knows which detachment rule fired, so it
*has* the feature; the current `Vec<String>` API discards it. This is **not needed for Slice 1**: the
feature *mechanism* (inductive + erasure + meet) lands first with features inert (everything `Any`);
the morphological *instantiation* that makes agreement actually bite arrives in Slice 2 with the
determiners that exercise it. (Touches `dcg::lemmatizer`, `dcg::lookup`, `MorphyLemmatizer`.)

*Question denotation: deferred to Slice 5.* The *eventual* `⟦S[q]⟧` is `Entity→Prop` (or a set of
Props); until Slice 5, `S[q]` is a **syntactic tag only** (denotation trapped/`unimplemented!`), used so
auxiliary inversion parses without polluting declaratives. Consistent with §5.3's deferral.

*`denote_cat` location: engine-side Rust* until the inventory locks (post-Slice-3) — promoting to an
in-kernel recursor while the lattice is still churning would force a kernel rebuild per added feature
(D62 §8.6 already defers the in-kernel recursor).

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
3. **Behavioral (FraCaS)**: **346** inference problems
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
  reach verb argument slots — *general WordNet sentences become real.* **Done** (§8.2). The
  **symmetric closed-class determiner buildout** (`every/all/each/a/some/no`, subject+object) that
  populates a committed lexicon over this machinery is detailed in **§8.3**.
- **Slice 3 — copula + predication + attributive adjectives** (D63 **§8.8**). **3a copula + predicative
  adjective ✅** ("HeLa is primary"); **3b attributive adjectives ✅** ("a primary cell line") — engine-level
  Σ-refinement with `Fst`-projection (no kernel change; adjectives gained the `adj` category); **3c predicate
  nominals ✅** — instance ("HeLa is a cell line" → `ontology:is_a`; predicative `a` + copula) *and* kind
  ("Genes are cell lines" → `ontology:subclass_of`; bare-plural `cat_kind` subject + kind copula `are`),
  opaque, grounded downstream by `ChainWitness`. **Slice 3 complete.** ("the"/definiteness stays deferred —
  §8.4.2.)
- **Slice 4 — coordination + plurals** (needs composition). Detailed design in **§8.4**: connectives
  via parser-level generalized conjunction; NP coordination as `List`-groups (distributive /
  reciprocal / basic collective); deep plural semantics deferred.
- **Slice 5 — wh-questions. ✅ Done** (D63 **§8.5**): **5b subject-wh** (gap adjacent → application-only),
  **5a polar** (aux `do`-support + base-form verbs + finiteness root gate, `denote_mood(q)=Prop`), and
  **5c object/embedded wh** (forward composition **B** + object-wh wh-words + **Eisner normal form**). A
  wh-question denotes its **answer-property** `⟦Q(T)⟧ = T→Prop` via a type-carrying `cat_q(T)` category.
  **Type-raising T was deferred to Slice 6** (its use is aux-less extraction / relativization).
- **Slice 6 — negation, auxiliaries, relatives, the tail** (D63 **§8.9**; a cluster, decomposed).
  **6-neg ✅** (verbal + copular negation, `¬P := P → logic:False`). Deferred: **6-T + 6-rel**
  (type-raising `T` + the combinator/Eisner completion + relative clauses, reusing 3b refinement — the
  substantial unit); **6-aux** (progressive/perfect need importer verb-form morphology; passive = voice
  alternation; modals = modal operators); **6-tail** (case, pronouns gated on anaphora resolution).
- **Slice 7 — full-WordNet operationalization (scale-up).** *Orthogonal to the grammar slices* —
  gated only on Slice 2 + the closed-class track (both done), **not** on Slice 5. The 204k-entry WordNet
  content layer (§4a) is already imported and kernel-validated; this slice turns it from a *generatable
  artifact* into a *standing, parseable layer at scale* and fixes what only breaks at volume
  (sense-ambiguity forest blow-up, `LexicalIndex`/parse performance, the D62 §8.7 import residuals). Method: a
  **staged ramp (1% → 10% → all)** via `wordnet-import --limit`. Detailed design in **§8.7**.

### 8.2 Slice 2 — determiner typing (resolved) and the DCG extension

The friction is Montague's single-sorted `e` vs MTT's many-sorted domain (common nouns as base
types). Resolved by an MTT-semantics expert consult: **Option 2 — type/category polymorphism**, which
keeps the per-item felicity discipline *and* exploits coercive subtyping.

**Determiner typing (settled).**
- A determiner is **polymorphic**, predicate argument at the **CN type `A`** (not `Entity`):
  `⟦every⟧ : ΠA:Set. (A→Prop)→Prop = λA:Set. λV:A→Prop. ∀x:A. V(x)` (and `a`/`some` via the
  impredicative `∃`).
- The **category mirrors the `Π`** with a category variable `T`: `∀T. (S/(S\NP_T))/N_T`, so
  `⟦cat⟧ = ΠT:Set. (T→Prop)→Prop` — the felicity invariant `⟦cat⟧ ≡ sem_type` holds **in isolation**.
- **Composition by coercion:** `every`+`gene` (`N_Gene`, sem `Gene:Set`) instantiates `T := Gene` →
  `S/(S\NP_Gene)`. The generic verb's VP is `Entity→Prop`; since `Gene ≤ Entity`, **contravariance**
  lifts `(Entity→Prop) ≤ (Gene→Prop)` — the verb is coerced to fill the determiner's `Gene`-slot; the
  bound `x` is *not* coerced inside the determiner.
- **No CN universe / no bounded `Σ`:** keep `⟦N⟧ = Set`; entity-hood is enforced at the application
  site (`V(x)` fails to compose if the noun isn't a declared `Entity`-subclass). A `CN` universe
  (Tarski-style or a typeclass) is an optional refinement to forbid quantifying over absurd types —
  not needed for parsing.
- **`Prop`-valued, impredicative `∃` is sound** and doesn't change the `ΠA:Set` typing; it forfeits
  only donkey anaphora / discourse binding (no witness from `Prop`) and constructive Σ-modification
  (adjectives via `∧`, not `Σ`) — both accepted (§5.3).

**Correcting §5.1 — `cat_n` carries the type after all.** The polymorphic category's variable `T`
unifies with the noun's type via `cat_n`'s index, so `cat_n` must carry the (denotation-erased) type:
`cat_n(T, Num)`. The earlier "dead weight" call was wrong — the determiner case is exactly where the
index is load-bearing; the original §5.1 sketch had it. The Slice-1 `cat_n(Num)` is reverted in the
build below.

**How a polymorphic category is stored — `cat_forall` (decided).** The variable `T` cannot live as a
*free* `Exp::Var` in the stored category: `lexicon:cat` is `class_types eigentt:TypeExpr`, so the
commit-time felicity check (validation Rule 21) `check_infer`s the value, and an unbound variable is
rejected. The category must be **closed**. We add a category-level binder constructor to `lexicon:Cat`:
`cat_forall : (Set -> lexicon:Cat) -> lexicon:Cat` — the **dependent forward slash over a common-noun
type**. A determiner is `cat_forall(λT:Set. R[T])` where `R[T] = S/(S\NP_T)` is the category *after*
consuming the noun; `⟦cat_forall(λT. R)⟧ = ΠT:Set. ⟦R⟧`. The HOAS body keeps the stored value a
closed, kernel-checked `lexicon:Cat`; the reflexive constructor is **strictly positive, hence sound**.
A probe confirmed the existing kernel already declares + check-mode-checks this constructor and its
lambda payload (no kernel changes), and that the kernel does **not** yet enforce positivity in
general — filed as **eigenius#92** (close that to make the soundness an enforced invariant, not a
happy accident). The free-variable form *does* still appear — as the **transient parse-time
instantiation** inside `apply` (peel the binder → bind `T` → `subst_cat`), never on the chain.

**Implementation — a slice of dependent categorial grammar (Luo Ch 7.3; §2).** The cat engine extends:
1. ✅ **`cat_n` carries the type** (`cat_n(T, Num)`) — reverted Slice-1 `cat_n(Num)` (schema +
   `denote_cat` + importer + re-import).
2. ✅ **Category type-variables + first-order unification** — `unify_cat`/`subst_cat`: a schematic
   `Exp::Var` in a type-index binds to the noun's concrete type at composition and is substituted
   through the result (`apply`); `cat_subsumes` is now `unify_cat(..).is_some()`.
3. ✅ **`cat_forall` denotes `Π`; dependent application** — `denote_cat(cat_forall(λT. R)) = ΠT:Set. ⟦R⟧`
   (matching the polymorphic sem, so the gate admits the determiner in isolation); `apply` instantiates
   `cat_forall` against a noun (`T :=` the noun's type). The closed-binder storage decision above.
4. ✅ **Contravariant structural subsumption** for `fwd`/`bwd` — `unify_into` recurses into functors
   with function variance (covariant result, contravariant argument), so `S\NP_Entity` fills
   `S\NP_Gene` when `Gene ≤ Entity`. Verified end-to-end via `apply`: the determiner-result
   `S/(S\NP_Gene)` composes with a general VP `S\NP_Entity` to an `S` whose sem reduces to
   `∀x:Gene. q(x) : Prop` — the milestone now *produced by the combinators*, not hand-built.
5. ✅ **Determiner + common-noun entries** + the milestone (subject *and* object quantification).
   - **λ-sem on the chain.** A Curry `Lam` is unsynthesizable, so committing a function word's
     λ-semantics needed a **bidirectional annotation** — added `Exp::Ann(e, T)` through the kernel
     (term / eval-erase / `check_infer` mode-switch / D47 codec / positivity & guardedness traversals)
     plus the ESL surface `(e : T)`, so `check_infer(Ann(λ…, T))` succeeds. The λ-sem lives in a
     `lexicon:SemTerm` term-holder (`lexicon:term : eigentt:TypeExpr`), referenced by `sem` — `sem`
     stays uniformly a reference (the 200k WordNet entries reference classes/axioms/instances; a
     function word references a term), and `resolve_sem` dispatches on the referent.
   - **ESL lexer fix (foundational).** `(e : T)` collided with the qualified-name separator `ns:name`:
     a term ending in a bare identifier (`… -> C : T`) was mis-read because `parse_qualified_name`
     greedily consumed `C : …`. Fixed by lexing a **qualified name atomically** (`QualName(ns,name)`,
     tight `:`), freeing the standalone `Colon` to mean *only* the binder/annotation colon. The value
     and expression parsers now accept `QualName`; binder names stay `Ident`-only (correctly rejecting
     `ns:x`). Whole workspace + clippy green.
   - **Object quantification.** The type-raised object determiner `a : cat_forall(λT. (S\NP_E)\((S\NP_E)/NP_T))`
     with the impredicative existential `λT.λTV.λsubj. ∃x:T. TV(x,subj)`; the verb fills the object
     slot by contravariant functor subsumption (item 4).
   - **Milestones (string → bridge → CKY → kernel-checked `Prop`):**
     `every cell line [is] primary` → `∀c:CellLine. is_primary(c)`;
     `every gene affects HeLa` → `∀g:Gene. affects(HeLa, g)`;
     `every gene affects a cell line` → `∀g:Gene. ∃c:CellLine. affects(c, g)`.

   - **Slice-2 tail (done).** *Number agreement:* `cat_forall` carries the determiner's expected
     `Num`; `apply` checks it; `LexicalIndex` refines a noun's `num_any` to the surface number
     (morphology). So `every gene affects HeLa` parses but `every genes …` does not (sg ⊓ pl fails).
     *FraCaS monotonicity runner:* parses premise/hypothesis to Props via the bridge and has the
     kernel check the entailment *witness* (`witness : ⟦premise⟧ → ⟦hypothesis⟧`). `every` is
     downward-monotone in its restrictor (`Gene ≤ Entity`): `every entity affects HeLa` ⊨ `every gene
     affects HeLa` is kernel-verified; the invalid converse is rejected. (The kernel is a *checker* —
     the monotonicity witness is constructed; generic FraCaS entailment = proof search, out of scope.)

   **Slice 2 is complete.** The engine takes a raw English sentence with subject *and* object
   quantifiers + number agreement and produces a kernel-verified dependent proposition.

*Already landed (Slice-2 infrastructure + de-risk).* (a) the check-mode felicity gate (#91-B — admits
lambda sems against their `Pi`); (b) the parser's **NbE-reduce before the final check**
(`lookup.rs::reduced_prop` — the composed determiner term β-reduces to a lambda-free normal form the
kernel can type); (c) **parenthesized (grouped) types** in the ESL `type_expr` parser, so higher-order
types like `(A→Prop)→Prop` are writable; and (d) a kernel-level **validation of the determiner
semantics** (`kernel/tests/lexicon_validates.rs`): the polymorphic sem inhabits `ΠA:Set.(A→Prop)→Prop`
(gates in isolation), and `det(Gene)(q)` NbE-reduces to `∀x:Gene. q(x) : Prop` (the `Gene ⊑ Entity`
coercion firing under `∀`). So Option 2's typing is **confirmed end-to-end at the kernel level**. What
remains is the dependent-category machinery (1–4 above) — to *produce* these terms via parsing — plus
the entries (5).

### 8.3 Determiner buildout — the symmetric closed-class set

Slice 2 proved the determiner *mechanism* (universal subject, existential object) with two hand-authored
test fixtures. This buildout populates the full closed-class set as **committed chain data** in a
dedicated layer (`ontologies/lexicon/closed-class.esl`), not test fixtures. It is pure population on the
existing `cat_forall` machinery plus a few logical primitives — low risk.

**Phase 0 — logical primitives (prerequisite). ✅ Done.** `ontologies/logic/logic.esl` declares
`logic:False` (`⊥`); negation is the idiom `P → logic:False`. (`And`/`Or` arrive with the connectives,
§8.4 Phase 3 — YAGNI; `∃!` only if *the* is taken up.) **`logic` and the `lexicon` schema are now in the
production bootstrap chain** (`…→ reference → logic → lexicon`); `bootstrap()` resolves `logic:False`,
`lexicon:Cat`, `lexicon:LexicalEntry`, `lexicon:SemTerm`.

**The object-determiner subject-type decision (resolved — Option A′).** The object determiner's sem/category
mention `E`, the subject type, and `cat_forall` binds only the noun type `T`. A *universal supertype of
every class* does **not** work — functor-argument contravariance at the verb step requires `E` to *equal*
the verb's subject type, not be a supertype of it. The fix is a **single designated entity top**, with all
verb subjects typed at it and the determiners' `E` pointing at it; specific noun types reach argument slots
by subsumption (the verb fills the object slot, the VP fills the subject slot). This is what both reference
systems do: **lightblue** has one built-in `Entity` and quantifies over it (`English/Lexicon.hs`: a CN is
`Entity→Type`, `"a"` is `Σ Entity …`); **Luo's MTT** uses coercive subtyping with a maximal type for
cross-sortal breadth. The top is **grounded in WordNet's `entity.n.01`** (offset `00001740`, the root of
the noun lattice — no hypernym, everything a hyponym), which the WordNet importer *already* uses as the
generic verb-argument type (`convert.rs::ENTITY_ROOT = "wn:n00001740"`). The demo's `lexicon:Entity` was
its stand-in. **Decision (ii):** promote a **schema-level `lexicon:Entity`** as the canonical entity top
(in the bootstrapped lexicon schema, so the determiner layer stays self-contained — no WordNet-import
dependency); the WordNet importer roots `wn:n00001740` at it and types verb **subjects** at it (exact match
with `E`, per the contravariance constraint); the determiners' `E = lexicon:Entity`. The noun/object side
stays Luo CN-as-types (the determiner is polymorphic in `T:Set`, full sortal precision); only the verb
**subject** position uses the lightblue-style single top. The deferred Option B (a second category
type-variable for genuine subject polymorphism) is the fallback if verbs ever need subject sorts that
*don't* share a top. With this resolved, the closed-class determiner layer becomes domain-independent and
is committed (`ontologies/lexicon/closed-class.esl`).

**Phase 1 — quantifier cores + position templates (factor, don't repeat).** Define the cores once and
derive every entry from them, so the ~12 sems are provably uniform rather than 12 ad-hoc lambdas:
- cores `q_forall = λA.λV. ∀x:A. V(x)`, `q_exists = λA.λV. ∃x:A. V(x)`, `q_no = λA.λV. ∀x:A. ¬V(x)`;
- **subject** determiner sem *is* the core (category `cat_forall(num, λT. S/(S\NP_T))`);
- **object** determiner sem is the template `obj(Q) = λT. λTV. λsubj. Q[T](λx. TV(x, subj))` (category
  `cat_forall(num, λT. (S\NP_E)\((S\NP_E)/NP_T))`).

**Phase 2 — the entries (committed layer).** Cross-product of {core × number × subject/object}:

| Determiner | Core | Number | subj | obj |
|---|---|---|---|---|
| every / each | `∀` | sg | ✓ | ✓ |
| all | `∀` | pl | ✓ | ✓ |
| a | `∃` | sg | ✓ | ✓ |
| some | `∃` | sg/pl | ✓ | ✓ |
| no | `¬∃` (= `∀¬`) | sg/pl | ✓ | ✓ |

`every ≈ each ≈ all` are truth-conditionally equal here (distributivity/collectivity distinctions
deferred to §8.4 plurals). **`no`** needs Phase-0 `¬`/`False`.

**Deferred (decision):** ***the* / definiteness.** It is a uniqueness *presupposition* (`∃!` + projection),
a distinct subtopic; `∃` would be semantically wrong ("paper-over"), so *the* waits for a presupposition
treatment rather than an approximation.

### 8.4 Coordination & plurals (Slice 4)

Two genuinely different mechanisms hide under "connectives." The trust boundary throughout: the
combinators build the term, **the felicity filter kernel-checks the result** — the same boundary as
`apply`. So all of this is *parser-level trusted machinery*, and the produced proposition is always
gated. (Decision: parser-level is **non-blocking** — the result is fully checked regardless; only the
coordinator itself is engine machinery, like the application combinators, not introspectable chain data.
The lone future tension is committing full *derivation trees* as chain objects, which would want the
coordinator typed-on-chain; the path there is to **reflect `denote_cat` (`⟦·⟧`) into the kernel** as a
real function — sizable, and *not foreclosed* by choosing parser-level now.)

**Why not `denote_cat`.** Coordination `X and X → X` is polymorphic over `X : Cat` (the category
itself). It **cannot** route through `denote_cat`: `⟦·⟧` is a Rust meta-recursor, not a kernel function,
so `ΠX:Cat. ⟦X⟧→⟦X⟧→⟦X⟧` is not a single kernel type. Hence coordination is a *parser rule* + a Rust
combinator, never a stored category.

**Phase 3 — `S`/`VP`/`TV` coordination (generalized conjunction). ✅ Done.** `logic:And`/`logic:Or`
landed as `Prop`-inductives in `ontologies/logic/logic.esl` (bootstrapped). `and`/`or` are **parser-level
reserved words** (not lexical entries — `cat_conj` can't denote); the CKY gains a coordination rule
(`lookup.rs::parse`): for a coordinator at position `c`, conjuncts `[i..c-1]` and `[c+1..j]` that are the
**same category** (`cats_coordinate`: mutually unifying + Prop-ending) combine into that category with the
**generalized-conjunction sem** (`coordinate_sem`/`generalized_coord` in `category.rs`): pointwise-lift the
connective by recursion on `⟦X⟧`'s arrow structure (η-expand, `op(a,b)` at the `Prop`) — `S`: `And(P,Q)`;
`VP`: `λx. And(P x, Q x)`; `TV`: `λo.λs. And(P o s, Q o s)`. **Verified** (`closed_class_determiners.rs`):
`HeLa affects BRCA1 and BRCA1 affects HeLa` (`S`), `HeLa affects BRCA1 and affects HeLa` (`VP`), and the
`or` variant all parse → kernel-checked `Prop`. (`not` for `VP`/`S` is a follow-on, same machinery.)

**Phase 6 — NP coordination as `List`-groups (plurals-lite). ✅ Done (distributive + collective +
reciprocal).** A coordinated NP is a **group = `List C`** (members coerced to a common supertype `C`),
built with the kernel's existing `List` + Phase-0 `∧`. Model the group as a member-retaining list from the
start (*not* the members-discarding "type-raise + generalized conjunction", which forecloses the readings
below); the three readings are then *operations over the group*, each producing a kernel-checked `Prop`:
- **distributive ✅** ("HeLa and BRCA1 affect HeLa") — map a one-place predicate over the members and
  ⊕-fold (∧ for `and`, ∨ for `or`) → `affects(HeLa, HeLa) ∧ affects(HeLa, BRCA1)`. *Implemented*: a new
  `cat_group : Set → Conn → Num → Cat` constructor (`⟦cat_group(C, _, _)⟧ = List C`, the kernel
  `list_decl()`), carrying a `lexicon:Conn` feature (`conn_and`/`conn_or`) — the connective must travel with
  the phrase because distribution is *deferred* to the verb. `coordinate_np` builds the group (common
  supertype `C` via the subclass lattice — `CellLine ⊔ Gene = Entity` — with the right conjunct required to
  be a plain NP, keeping n-ary groups left-branching; mixed `and`/`or` is rejected). Two combination rules
  in `apply`: a **distributive subject** (`cat_group(C,_,_)` meeting a VP `S\NP_{C'}`, `C ≤ C'`) and a
  **distributive object** (a TV `(S\NP)/NP` seeking a group object → a VP `λs. V(m₀,s) ⊕ V(m₁,s) ⊕ …`).
  Members are statically known (a literal coordination), so the `Map`/`Reduce(⊕)` is computed at parse
  time, yielding the bare connective chain (no `List`/`Reduce` residue, no `logic:True` unit) — faithful to
  the result shape `pred(m₀) ⊕ pred(m₁) ⊕ …`. Covered by `distributive_np_coordination_parses`,
  `disjunctive_np_coordination_distributes_with_or`, `distributive_object_coordination_parses` (+ `_with_or`),
  and `nary_distributive_group_is_left_branching_single_parse`.
- **reciprocal ✅** ("HeLa and BRCA1 affects each other") — the 2-place verb conjoined over ordered
  distinct member pairs → `affects(brca1, hela) ∧ affects(hela, brca1)`. *Implemented* (`reciprocate`):
  "each other" is a reserved reciprocal anaphor (a parser-level token pair, like `and`/`or` — not a
  lexical entry), and the reciprocal CKY rule keys on the trailing "each other", relating the verb over
  every ordered distinct pair of the subject group's members (`⋀_{i≠j} V(mⱼ, mᵢ)`, object-first). Members
  are statically known, so the pairs are enumerated at parse time (no list/quantifier residue). Reciprocity
  is conjunctive by nature → `and`-groups only, ≥2 members (`reciprocal_rejects_an_or_group`). For a pair
  it is exactly the two-conjunct conjunction; *n* members give *n·(n−1)* ordered pairs
  (`reciprocal_three_members_has_six_ordered_pairs`). A compositional operator over the group, never a
  surface-string rewrite. Covered by `reciprocal_np_coordination_parses` (+ the n-ary and or-rejection
  cases).
- **basic collective ✅** ("HeLa and BRCA1 form a complex") — type the collective verb **over the group**:
  `forms_complex : List Entity → Prop`, applied to `[hela, brca1]`. No mereological sum entity is invented;
  the `List C` *is* the argument. *Implemented*: the collective verb's category is `S\Group(Entity)` —
  `bwd(cat_s, cat_group(Entity, conn_and, _))`, whose `⟦·⟧ = List Entity → Prop` (the `cat_group`
  denotation, finally exercised on the real path) matches the axiom's `sem_type`. `unify_into` gained a
  `cat_group` arm so the group fills the slot under ordinary backward application (no new combination rule);
  the `conn_and` slot restricts it to `and`-groups (`collective_rejects_an_or_group`). This required making
  the kernel's canonical built-in `core:List` referenceable from ESL type expressions: the
  `eigentt:TypeExpr` decoder (`resolve_const_ref`) now short-circuits the `core:List` IRI to the built-in
  `list_decl()` — exactly as it already does the primitive datatypes — so a `core:List(Entity) → Prop`
  axiom commits and gates. Covered by `collective_np_coordination_parses` / `collective_rejects_an_or_group`.

**Deferred to Phase 7 (deep plural semantics), with reasons:** distributive/collective **ambiguity
resolution** (which reading an ambiguous verb takes); **cumulative quantification**; **higher-arity
reciprocity** scope variants (strong/weak/intermediate for groups > 2); true **mereological sums** as
first-class entities (only if some construction genuinely cannot be a `List`); **`N`-coordination as a
union type** (`RNA ⊔ DNA` — we have subtyping + the `is_a`-meet, not arbitrary unions).

**Phase 4 — spurious-ambiguity control. ✅ Done (right-sized after grounding).** *Measured first:* across
the determiner + single-coordination sentences the forest is **exactly one** parse per reading; the only
spurious ambiguity is **n-ary coordination associativity** — `A and B and C` yields two
logically-equivalent parses (`And(And(A,B),C)` vs `And(A,And(B,C))`). **Classic Eisner normal form
(composition / type-raising) does not apply yet** — this grammar is application + *lexical* type-raising
(the determiners' `cat_forall`) + coordination, with **no composition rule**, so the derivational
explosion Eisner targets doesn't arise. The fix is the matching-sized one: a **left-branching coordination
normal form** — the CKY coordination rule (`lookup.rs`) forbids a coordination whose **right** conjunct is
itself a coordination (detected via `is_coordination`: the sem, λ-peeled, is `logic:And`/`logic:Or`-headed
— those connectives arise only from coordination here). So `A and B and C` parses *only* as
`(A and B) and C` (`nary_coordination_has_a_single_left_branching_parse`). **The Eisner machinery returns
as a hard dependency the moment a composition rule or a general type-raising rule lands** (e.g. Phase 6 NP
type-raising, if taken that route) — `references/publications/Eisner-Efficient Normal Form Parsing.pdf`.

**Phase 5 — eval (extend the FraCaS runner). ✅ Determiner monotonicity + conjunction elimination.**
The runner (`treetest_entails`: parse premise + hypothesis → `Prop`, kernel-check the supplied entailment
witness) covers all three new determiner profiles in their **restrictor**, each with a valid case AND the
rejected converse (`lexicon_validates.rs`): `every` ↓ (`every entity affects HeLa ⊨ every gene …`),
`some` ↑ (`some gene … ⊨ some entity …`, witness = the impredicative-∃ lift `λe.λC.λk. e C (λg. k g)`),
`no` ↓ (`no entity … ⊨ no gene …`, same instantiation witness as `every`, since `no = ∀¬`). The
**conjunction inference** `P ∧ Q ⊨ P` is now also verified: `logic:And` is declared with `P, Q` as
**parameters** (sort-typed at `Prop`, Lean's `And (a b : Prop) : Prop`), so first-projection is the
ordinary parametric recursor — witness `λm. match m { conj p q => p }`, checked against `And(P,Q) → P`
with the always-admissible `Prop`-valued (subsingleton) motive `λ_. P`. This required giving ESL `data`
**parameters** the same sort-kind support indices already had (`IndexKind::Sort` for `DataParam.kind`,
so `data And (P : Prop, …)` parses and lowers); the constructor still leads with the parameter binders
(`forall (P, Q) => …`), the kernel convention `peel_ctor_telescope` strips. (Earlier this was deferred
because `And` was declared with `P, Q` as *indices*, whose first-projection needs the harder
index-abstracting recursor — the parameter declaration eliminates that.) Scope/body monotonicity (the
object existential's ↑) and running the *actual* `fracas.xml` corpus (needs a far wider lexicon) remain
follow-ons.

**Scope (decision):** **surface scope only.** The GQ approach yields the surface reading; inverse scope
("a cell line that every gene affects") needs QR — deferred.

#### 8.4.1 Reference utilization

| Resource | Used for | Status |
|---|---|---|
| CGEL — Huddleston & Pullum (`references/publications/`) | determiner classes/distinctions; coordination + reciprocal facts | read-only (copyrighted) |
| OpenCCG (`references/openccg/`, mini-english) | determiner category schemes + the `X conj X → X` rule | LGPL — read & reimplement |
| `lightblue` (`references/lightblue/`) | DTS GQ/determiner sems; confirms the `Ann` annotation node | BSD-3 ✓ |
| FraCaS  | the eval battery — GQ monotonicity (§1) + conjunction | eval-only |
| Eisner, *Normal-Form Parsing* (`references/publications/`) | spurious-ambiguity control (Phase 4) | reference |
| WordNet (`references/WordNet-3.0/`) | the open-class nouns/verbs determiners compose with | shippable (imported) |
| Partee & Rooth 1983, *Generalized conjunction and type ambiguity* | generalized-conjunction theory (Phase 3) | **citation to verify before load-bearing** |

#### 8.4.2 Decisions log

1. ***the* / definiteness** — **deferred** (uniqueness presupposition; `∃` approximation rejected as paper-over).
2. **Category polymorphism for coordination** — **parser-level** generalized conjunction (non-blocking;
   `⟦·⟧` is not a kernel function; reflecting it into the kernel is the open path if chain-typed
   coordination is ever needed).
3. **NP coordination** — **`List`-group model** from the start; distributive + reciprocal + basic
   collective reachable (Phase 6) without mereology. "bind each other" is a reciprocal (pairwise `∧`),
   not a true collective.
4. **Scope** — **surface only**; QR deferred.

### 8.5 Slice 5 — wh-questions

A question denotes its **answer-property**: `⟦Q(T)⟧ = T → Prop` — the predicate an answer must satisfy.
The queried type `T` is **carried in the category** (`cat_q : Set → Cat`, `⟦cat_q(T)⟧ = T → Prop`), the
CN-as-types treatment that lets a restrictor narrow the answer ("which **gene**" → `Gene → Prop`) exactly
as determiners carry `T`. (Polar yes/no questions are a *distinct* shape — `cat_s(q, _)`, `⟦·⟧ = Prop`,
the queried proposition; see 5a.) "wh-questions" decomposes by difficulty, and the decomposition is
**forward-compatible** — 5b's pieces are reused unchanged by 5c, not torn up:

**5b — subject wh-questions. ✅ Done.** The gap is the subject, *adjacent* to the VP, so composition is
**plain application — no extraction, no new combinators**:
- `what : cat_q(Entity)/(S\NP_Entity)` — takes the VP, yields the Entity-ranged answer-property.
- `which : cat_forall(λT. cat_q(T)/(S\NP_T))` — consumes a common-noun restrictor (binding `T`), then the
  VP, yielding the `T`-ranged answer-property; reuses the determiner `cat_forall` machinery + the
  contravariant functor subsumption (§8.2 item 4), so an `Entity`-typed verb answers a `which gene` query.
- Both sems are **η-expanded** (`λV. λx. V(x)` / `λA. λV. λx. V(x)`): the answer-property is a *lambda*,
  so the felicity check (now **check-mode** against `⟦cat⟧`, not `check_infer` — a lambda can't be
  synthesized) pushes the queried type `T` into the binder, and the body uses the **covariant**
  application coercion (`T ≤ Entity`) the kernel supports — sidestepping contravariant function subtyping,
  which the kernel does not do. The lookup felicity filter accepts a full-span `cat_q(T)` (answer-property)
  alongside `cat_s` (declarative `Prop`). Covered by `subject_wh_what_parses_to_an_entity_answer_property`,
  `subject_wh_which_narrows_the_answer_type_to_the_noun`, `subject_wh_which_requires_a_noun_restrictor`.

**5a + 5c — the auxiliary-inversion family (scoped; grouped).** Polar ("does HeLa affect BRCA1?") and
object/embedded wh ("what does HeLa affect?") share an **auxiliary + base-form verbs + finiteness
checking**, so they land together. The shared infrastructure:
- **Auxiliary entries** `does`/`do`/`did` (present/past `do`-support; the copula `is`/`are` is Slice 3,
  perfect `have` later). The aux flips `mood → q` and selects a **base** complement.
- **Base-form verbs** (`Fin = bse`): the aux's VP complement is `S[bse]\NP`, so the `Fin`-meet blocks
  `*does HeLa affects` (agreement bites). The WordNet import already keys on base lemmas; the demo gains a
  `bse` verb beside the existing `fin` one.
- **`denote_mood(q) = Prop`** (flip the fail-closed stub): a polar question denotes the *same `Prop` as the
  declarative*, `mood`-tagged `q` (asked, not asserted) — the felicity filter already admits `cat_s`.

**5a — polar questions. ✅ Done. Application-only (no combinators), like 5b.** The aux carries it:
`does/do/did : (S[q,fin]/(S[dcl,bse]\NP)) / NP` — takes the subject `NP`, then the base VP, yields `S[q]`;
sem `λsubj. λV. V(subj)` → the queried proposition `affect(brca1, hela) : Prop`. *Implemented*:
`denote_mood(q) = Prop`; the `do`-support auxiliaries + a base-form (`Fin=bse`) verb in the demo; a
**finiteness root gate** (`lookup::is_finite_clause`) so a bare base clause `S[_,bse]` is not a standalone
sentence. Covered by `polar_question_parses_to_a_queried_prop`, `bare_base_clause_is_not_a_finite_root`
(rejects `*HeLa affect BRCA1`), `auxiliary_requires_a_base_form_complement` (the `Fin`-meet rejects
`*does HeLa affects BRCA1`).

**5c — object/embedded wh. ✅ Done. Forward composition B (only) + Eisner.** The derivation: `does HeLa`
(`S[q]/(S[bse]\NP)`, aux applied to subject) **forward-composes** with the gapped TV `affect`
(`(S[bse]\NP)/NP`) → `S[q]/NP` (sem `λobj. affect(obj, hela)`); then `what : cat_q(Entity)/(S[q]/NP)`
applies → `λx. affect(x, hela) : Entity → Prop`. (`which gene` is the wh-determiner
`(cat_q(T)/(S[q]/NP)) / N_T`, reusing `cat_forall`.) So 5c needs **forward composition B** + the object-wh
wh-words + the **Eisner normal form** (the spurious-ambiguity control Phase 4 deferred "until a
composition rule lands" — this is that moment).

**Decision — type-raising (T) is deferred to Slice 6.** Object-wh *questions* need **B only**: the aux
absorbs the subject, so no subject type-raising is required. T's genuine use is **aux-less extraction**
(relativization — "the gene **that** HeLa affects"), which is Slice 6. Deferring T also keeps the
spurious-ambiguity surface small now: with B-only, existing declaratives admit **no new B-derivation**
(`(S\NP)/NP ∘ NP` doesn't compose — `NP` is atomic), so Eisner's burden — and the regression risk to the
current single-parse tests — is minimal. T (and the Eisner extension covering it) lands with the relatives
in Slice 6.

**Eisner normal form — the mechanism, and why adding `B` globally is safe.** Composition makes the *same
meaning* derivable many ways (`X/Y ∘ Y/Z` then apply `Z`, vs. apply `Z` then `X/Y` — both yield `X` with
identical sem); a naive parser with `B` returns the whole equivalence class. Eisner 1996
(`references/publications/`) admits exactly one derivation per class via a constraint on **the primary
functor's provenance**: *the output of forward composition (`>B`) may not be the primary (left) input of a
subsequent `>` or `>B`* (symmetric for `<B` on the right). The decisive property for us: this **kills
spurious composition but licenses extraction**, distinguished by whether a `>B` output is consumed as a
**functor** (blocked) or an **argument** (allowed):
- *"does HeLa affect BRCA1"* — the composition path builds `S[q]/NP` (a `>B` output) then uses it as the
  **functor** applying to `BRCA1` → **blocked**; the application derivation survives → one parse.
- *"what does HeLa affect"* — the same `S[q]/NP` (`>B` output) is the **argument** of
  `what : cat_q(Entity)/(S[q]/NP)` → **allowed**; extraction goes through.

The only consumer of a `>B` output *as an argument* in this grammar is the wh-word — so wh-extraction is
exactly the case ENF licenses, which is what makes adding `B` globally safe (the regression gate below is
the witness).

**Implementation (as built).**
- **Provenance on `Item`** — a `Combinator` tag (`ForwardApp`/`BackwardApp`/`ForwardComp`/`Other`) set by
  *every* producer (lexicon seeds + coordination/group/distributive rules → `Other`; fwd/`cat_forall`
  application → `ForwardApp`; bwd application → `BackwardApp`; the new fwd-composition → `ForwardComp`).
  `Item::new` is the leaf constructor (`Other`). Only the forward variants are exercised; backward /
  type-raise variants arrive with Slice 6 (added then — *minor deviation from the original plan*: I did
  **not** pre-declare unused variants, to keep `clippy -D warnings` clean; a one-line enum addition at
  Slice 6, not a refactor).
- **The ENF gate lives in the shared [`apply`](../../kernel/src/dcg/parser.rs)** — the single combination
  point both `cky_parse` and the lookup CKY loop call. Before `>` / `>B`, it rejects when the **left**
  operand's provenance is `ForwardComp`.
- **`apply` stays `Option<Item>`** — *deviation from the "likely `Vec`" plan, justified by the audit:*
  per adjacent pair the rules are **mutually exclusive** (`>` needs `right = B`, `>B` needs `right = B/C`;
  `cat_forall`/distributive are gated by distinct left/right ctors), so first-match drops no reading. The
  spurious ambiguity is across *different `k`-splits*, where ENF prunes it — not within one pair. Keeping
  `Option` avoided churning both call sites for no correctness gain.
- **No `Cat` / semantics changes** — ENF is a structural short-circuit; the category inductive and NbE
  `sem` evaluation are untouched. The composition sem is `λz. left(right(z))` (a fixed binder name is
  safe — the kernel is NbE: environment-based eval + fresh readback is capture-avoiding, and composed sems
  are closed; matches the `distribute_object` precedent).
- **Scope: forward `B¹` only.** Generalized (`B^n`), crossing (`B×`), and backward composition — with
  their Eisner clauses — arrive alongside type-raising in Slice 6; not added speculatively.
- *Tests:* `object_wh_what_extracts_via_composition`, `object_wh_which_narrows_to_the_noun`, and
  `eisner_keeps_polar_single_despite_available_composition` (the regression witness — `does HeLa affect
  BRCA1` stays a single parse with B globally available); the full prior suite still single-parse.

**Forward-compatibility / no-undo (same discipline as 5b→5c):** the object-wh entry is *added* beside the
subject-wh one (CCG uses distinct categories — `cat_q(T)/(S[q]/NP)` vs `cat_q(T)/(S\NP_T)`); the aux and B
are *additive* `apply`/lookup branches; Eisner (landing with B) keeps 5b's application-only parses single
and is *extended*, not rewritten, when T arrives in Slice 6.

**Done-when:** "does HeLa affect BRCA1" → `affect(brca1, hela) : Prop`, `mood = q`; "what does HeLa affect"
→ `λx:Entity. affect(x, hela) : Entity → Prop`; "which gene does HeLa affect" → `Gene → Prop`; the
`Fin`-meet rejects `*does HeLa affects …`; and every existing declarative/coordination/5b test still has
**exactly one** parse (the Eisner-normal-form regression gate).

### 8.7 Slice 7 — full-WordNet operationalization (scale-up)

**The vocabulary is already imported; this slice is operational, not lexical.** `eigenius-wordnet`
(`wordnet-import`) already renders the full WordNet 3.0 content layer — 204,088 `lexicon:LexicalEntry`
resources (74,385 noun classes, 7,730 proper-noun individuals, 33,006 verb/adjective axioms), each
felicity-gated, `sem_type = ⟦cat⟧` by construction (§4a). The grammar is vocabulary-agnostic: a WordNet
entry flows through the same `cat_n`/`cat_np`/axiom typing the demo lexicon uses (verbs at
`lexicon:Entity`, nouns rooted at `entity.n.01`, `num_any` refined by Morphy). So nothing in the *grammar*
gates running over all of WordNet — the slices validate against the small demo only for **fast, exact**
unit tests (asserting `affects(hela, brca1)` needs a controlled vocabulary). This slice turns the 204k
entries from a *generatable artifact* into a *standing, parseable layer*, and surfaces + fixes what only
breaks at volume. **Ordering:** orthogonal to the grammar slices — gated on Slice 2 + the closed-class
track (both done), so it can proceed now and in parallel; richer grammar (Slices 3/5/6) only *widens what
the scaled lexicon can parse*, it is not a prerequisite.

**Why staged, and what each stage is *for*.** The risks are non-linear in corpus size, so we ramp and
**measure at each stage before advancing** (fail-closed: a stage with unresolved regressions does not
ramp). The lever is the existing `wordnet-import --limit N` — cap the per-POS seed to the first *N*
synsets, **closed under hypernymy** (so the `subclass_of` lattice stays rooted and self-consistent at any
size) — growing to `--all`. Stage percentages are of the ~115k-synset / 204k-entry whole.

- **Stage A — ~1% (`--limit` ≈ 1k synsets). Wiring + correctness.** Stand up the end-to-end path:
  import → commit as a standing layer → `LexicalIndex::build` → parse a battery → kernel-gate. At this
  size the forest and timings are hand-inspectable. *Targets:* index-build correctness; MWE multi-span
  seeding on real multiword forms; and the **D62 §8.7 import residuals on real data** — multi-class NP typing
  (kernel #91), instance-NP-vs-class emission, predicate (troponymy) subsumption. Done when a curated
  ~50-sentence battery (declaratives over real WordNet nouns/verbs + the closed-class function words +
  coordination) parses to kernel-checked `Prop`s.
- **Stage B — ~10% (`--limit` ≈ 12k synsets). Ambiguity.** This is where **sense-ambiguity forest
  blow-up** first bites: a content word carries many synsets → many indexed entries → a combinatorial
  parse forest. *Measure* the forest-size distribution over the battery (this is the load-bearing
  measurement of the slice — recorded, not guessed), then **decide the policy**: keep returning the whole
  forest (the §6 forest-returns / encoding-institution-selects boundary) but **cap/rank** it — candidate ranking
  by WordNet sense frequency (the `data.<pos>` sense order is already frequency-sorted) — vs. a hard
  beam. Surface this as an explicit decision with the measurement behind it; do not silently truncate
  (log what was dropped).
- **Stage C — 100% (`--all`, 204k entries). Scale.** Performance of the one-time `LexicalIndex::build`
  (memory + time over 204k entries) and per-sentence `parse`. The CKY `n³` is moot at sentence length
  (n ≈ 10–30); the real cost is the **per-cell item count** (ambiguity × the Stage-B policy) and the
  kernel felicity check run per full-span candidate. Harden (index data structures, the Stage-B
  cap) until the battery parses within a recorded budget.

**Done-when (the slice's checks).**
1. The full WordNet layer commits, is `Validator`-clean, and every entry felicity-gates at scale
   (`--validate` already asserts this on emit; re-confirm as a standing layer).
2. The representative sentence battery parses to kernel-checked `Prop`s over the full layer.
3. **Witnessed baselines recorded** (Derived, not asserted): index-build time + memory, per-sentence
   parse-time distribution, parse-forest-size distribution — the numbers that justify the Stage-B policy.
4. A documented **sense-ambiguity policy** (rank/cap with the dropped-tail logged, or explicit
   return-all), and the **D62 §8.7 residuals** (#91 multi-class NP, instance-vs-class, troponymy subsumption)
   either fixed or recorded as scoped findings.

**Out of scope (stays elsewhere):** improving *grammatical* coverage (Slices 3/5/6); sense
*disambiguation* as inference (choosing the right synset for a token in context — a downstream
encoding-institution / LLM-proposer concern, not the engine's; the engine returns the gated forest).

### 8.8 Slice 3 — copula, predication, predicate nominals

Three sub-parts of differing depth; implementation surfaced that two need machinery beyond the lexicon.

**3a — copula + predicative adjective. ✅ Done.** "HeLa is primary" → `is_primary(hela) : Prop`. The
copula `is`/`are : (S[dcl,fin]\NP)/(S[dcl,bse]\NP)` (sem `λP. P`) supplies finiteness to a **base** (`bse`)
adjective predicate. Decisions, as built:
- **Strict copula** (over the loose `(S[fin]\NP)/(S[fin]\NP)` identity): the `bse` complement makes the
  copula **required** — a bare `*HeLa primary` is not a finite root (the §8.5 finiteness gate) — and the
  `Fin`-meet **blocks the verbal over-generation** `*HeLa is affects HeLa` (a finite verbal VP can't fill
  the `bse` slot). The loose copula accepts any finite VP and was rejected for exactly that over-generation.
- **Adjectives typed at the `Entity` top** (`is_primary : Entity → Prop`), matching the WordNet importer
  and §8.3 decision (ii); specific subjects reach it by coercive subtyping. The demo's old
  `CellLine → Prop` was a demo artifact.
- **Importer change:** `push_adj` now emits `bse` adjectives (so all WordNet adjective predication requires
  the copula — correct English). Tests: `copula_with_predicative_adjective_parses`,
  `bare_adjective_needs_the_copula`, `copula_rejects_a_verbal_complement`,
  `every_cell_line_is_primary_parses_from_entries_to_a_checked_prop`.

**3b — attributive adjectives ("a primary cell line"). ✅ Done — engine-level, no kernel change.** The N/N
modifier is a **Σ-refinement** of the noun, realized at the engine (the Lean-style "coercion in the
elaborator, not the trusted kernel" — `nanoda_lib` confirms Lean's kernel has *no* coercive subtyping):
- **Adjectives get a distinct category** — a new `Fin` value `adj`, so a predicative adjective is
  `S[dcl,adj]\NP` (≈ CCG's `S[adj]`), distinct from base/finite verbs. *This reworks 3a* (the demo adjective,
  the WordNet importer's `push_adj`, and the copula's complement slot all move `bse → adj`), and **fixes a
  latent 3a over-generation**: `*does HeLa primary` is now correctly rejected (do-support selects base
  *verbs*, not adjectives). Needed here so the attributive rule recognizes adjectives and doesn't fire on
  intransitive verbs.
- **Attributive rule** (`apply`): `[adj S[adj]\NP] + [noun cat_n(C)] → cat_n(Σx:C. adj(x))`, built over the
  **concrete** `C` at parse time — so `adj(x)` type-checks at `x:C` directly (sidestepping the
  bounded-quantification gap entirely; no abstract `C`). Reuses the *same* adjective predicate as the
  predicative entry (3a).
- **Determiner-over-refined-noun** (`apply`): when `cat_forall` consumes a refined noun (a `Σ` index), it
  binds `T := C` (the **component** type) for the category — so the GQ composes with `Entity`-typed verbs
  normally — and **Fst-projects** the witness in the sem: `λV. det(Σ)(λz. V(Fst z))`. By Σ/Π currying this
  yields the correct restrictor for **both** quantifiers automatically — `∀z:Σ.V(Fst z) = ∀x:C. adj(x) →
  V(x)` and `∃z:Σ.V(Fst z) = ∃x:C. adj(x) ∧ V(x)` — with no determiner-awareness. The engine inserts the
  `Fst`, so the final `Prop` (`∃z:Σx:CellLine.is_primary(x). affects(Fst z, hela)`) type-checks with the
  **identity** coercion we already have (`Fst z : CellLine ≤ Entity`) — **no Σ-first-projection coercion in
  the kernel**.

So the two "kernel gaps" (Σ coercion, bounded quantification) are both **avoided** by doing the projection
at the engine and building the Σ over concrete nouns — `nanoda_lib`/Lean's elaborator-coercion precedent
made the call. (`lightblue`'s DTS underspecification-`@` model was the heavier alternative, not adopted.)
Tests: `attributive_adjective_existential_parses`, `attributive_adjective_universal_parses`,
`do_support_rejects_an_adjective`. Scope: **subject** position; object-position attributive is a follow-on
(as with distributive). A *reusable refined type* ("the type of primary cell lines") is the one thing not
gained — recoverable later via kernel Σ-coercion if first-class refinement types are ever needed.

**3c — predicate nominals. ✅ Done — instance ("HeLa is a cell line") *and* kind ("Genes are cell lines").**
Membership is a *judgment*, not a `Prop`; the faithful encoding uses **our ontology's own relations**,
subject-dispatched: instance subject → `is_a(hela, CellLine)`, kind subject → `subclass_of(Gene, CellLine)`
(the same relations the WordNet import produces — *not* a parallel `Id`-existential, which was rejected as
minting parallel vocabulary). **Decision: these are *opaque* predicates** (`is_a : Entity → Set → Prop`,
`subclass_of : Set → Set → Prop`), not kernel-decidable. *As built:* a new **`ontology` layer** (between
the lexicon schema and closed-class) declares both axioms; a **predicative `a`** entry (distinct from the
existential `a`) consumes the noun (binding `T`) and yields the **adjectival** predicate `λs. is_a(s, T)`,
which the existing copula (3a) lifts — so `is_a` reuses the copula with no new combinator. **Subject
dispatch is by type-checking**, not engine logic: an instance subject (`hela : CellLine ≤ Entity`) makes
`is_a(hela, CellLine)` felicitous. **Kind subjects** ("Genes are cell lines" → `subclass_of(Gene,
CellLine)`) are also in, via a small **categorial kind-track**: a new `cat_kind` category (⟦·⟧ = `Set` — a
type-valued NP, since a kind denotes its *type*, not an individual); a **bare-plural → kind-subject shift**
(`cat_n(C, pl) → cat_kind`, sem the class `C`); and a **kind copula** `are : cat_forall(λT. S[dcl,fin]\Kind)`
with sem `λT. λk:Set. subclass_of(k, T)` (it `cat_forall`-consumes the predicate noun, then the kind
subject applies). This is the generic/kind reading — distinct from "every gene is a cell line"
(`∀g:Gene. is_a(g, CellLine)`, the universal-over-instances reading, which uses the determiner + the
instance `is_a` predicate). Genericity in full (generics ≠ simple universals) stays out of scope. Tests:
`predicate_nominal_parses_to_is_a`, `kind_subject_predicate_nominal_is_subclass_of`. Rationale (**felicity ≠ truth**): the kernel gate
checks well-formedness; whether the membership *holds against the chain* is a separate **grounding**
judgment. A decidable predicate would eagerly reduce the claim from the lattice, so it could never be
carried as a **hypothesis** or conditional antecedent; opaque keeps the proposition's structure and is
consistent with how verb predications already work (`affects(…)` is opaque too). The lattice check still
exists — as a grounding step, not in the `Prop`'s meaning.

*Fit with the justification machinery (D39/D49).* This is the **same pattern** the Reasoning institution
already uses, which is why opaque is right. `JustifiedBy : JustificationTerm → Prop → Type` takes any
`Prop`, so a predicate-nominal `is_a(hela, CellLine)` rides a `ReasoningSentence` and picks up the four
epistemic grades unchanged. Its **grounding is a `ChainWitness`** — `IsObservedAs(hela, is_a(hela,
CellLine))`, admitted because `hela`'s class-membership is in the chain (D39: `ChainWitness` "projects the
reflection ontology's existing **class-membership** facts"). And `ChainWitness` predicates are themselves
opaque/kernel-internal — a **decidable** `is_a` would have *nothing to justify* and couldn't be a
hypothesis, so it would be *incompatible* with justification logic. **Coherence requirement for the build:**
the predicate-nominal `Prop` must be the *same* canonical membership proposition that `ChainWitness`'s
class-membership projection witnesses (read D49), not a parallel `is_a` axiom — else the witness's `P` and
the engine's `P` wouldn't compose.

### 8.9 Slice 6 — negation, auxiliaries, relatives, the tail

A *cluster* of components with different difficulty and dependencies, deliberately decomposed (not one
build).

**6-neg — negation. ✅ Done.** "HeLa does not affect BRCA1" → `affect(brca1, hela) → logic:False : Prop`;
"HeLa is not primary" → `is_primary(hela) → logic:False`. `¬P := P → logic:False` (reuses `logic:False`).
*As built:* **declarative do-support** `do/does/did : (S[dcl,fin]\NP)/(S[dcl,bse]\NP)` (sem `λP.P` — the
non-inverted counterpart of the 5a question aux); and **`not`** as a predicate-modifier `λP. λs. ¬P(s)`, in
two entries — over `bse` verbal VPs and over `adj` adjectival predicates (no feature-polymorphism, so two
forms). Self-contained, high-value, no new combinators. Tests: `verbal_negation_parses`,
`copular_negation_parses`. (Declarative `does` also licenses the emphatic "HeLa does affect BRCA1" —
grammatical, synonymous with the plain declarative; not spurious.)

**6-T + 6-rel — type-raising + relative clauses. Deferred (the substantial unit).** `T` exists *to serve*
relativization (aux-less object extraction — "the gene **that HeLa affects**"); it lands *with* relatives,
not alone. Needs: the general `T` rule + the rest of the combinator set (generalized `Bⁿ`, crossing `B×`,
backward composition) + the **Eisner extension** covering them (5c shipped only forward-`B¹`). Relativizers
(`that`/`which`/`who`) are noun-modifiers `(N\N)/(S\NP)` (subject relative, application-only) /
`(N\N)/(S/NP)` (object relative, needs `T`+`B`); both **restrict a noun like an attributive adjective, so
they reuse the 3b Σ-refinement + `Fst` machinery** — 3b already built the hard semantics; 6-rel adds the
relativizer + extraction.

**6-aux — auxiliaries beyond do-support. Deferred; splits by dependency.**
- *Progressive / perfect* ("is affecting", "has affected") — the auxes are trivial (finiteness-lifting over
  `ger`/`pp` forms, aspect/tense erased), but need the verb's **`ger`/`pp` morphological forms** in the
  lexicon — the importer emits base/finite only, so this is gated on **importer verb-form morphology**
  (a Morphy-generation follow-on); demo-only it's a few hand-added forms.
- *Passive* ("BRCA1 is affected by HeLa") — a **voice alternation** (subject/object swap + the `by`-phrase),
  not just an aux; its own construction.
- *Modals* ("can/must affect") — need **modal operators** (`Possible`/`Necessary` in the logic layer);
  erasing modality loses meaning. Its own foundation.

**6-tail — case + the long tail. Deferred (demand-driven).** Pronoun case (he/him), complementizers,
comparatives, … — each its own construction. The big one, **pronouns**, is only useful with **anaphora
resolution** (resolve to a chain resource by IRI, §5.3) — a real feature, not a lexical entry. Not a
discrete deliverable; items land as the target corpus demands them.

## 9. References

Local + license-cleared (the Path B shelf):

| Reference | Role | Status |
|---|---|---|
| WordNet 3.0 (`references/WordNet-3.0/`) | content lexicon + Morphy | OSI ✓ (shippable) |
| OpenCCG `core-en` (`references/openccg/`) | categories / features / slash-modes | **LGPL — read & reimplement, do not ship** |
| `lightblue` (`references/lightblue/`) | DTS semantics (determiner-as-Σ) + chart | BSD-3 ✓ |
| FraCaS  | behavioral entailment battery | eval-only — reference |
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
