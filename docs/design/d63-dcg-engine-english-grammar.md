# D63 — The DCG engine: a categorial grammar of English over EigenTT

*Status: design + active implementation. Built: the engine core, the full WordNet content lexicon, the
feature set on `Cat`, the determiner/quantifier + coordination + plural-group function words, the full
question set (subject-wh, polar, object-wh extraction via forward composition + Eisner), the copula +
predicative/attributive adjectives + (instance) predicate nominals, and forward composition `B¹` (Slices
1–5 + §8.4; **Slice 3 complete** — copula, predicative + attributive adjectives, instance + kind predicate
nominals; **negation** — Slice 6-neg; **forward bounded type-raising `T` + restrictive relatives** —
Slice 6-T/6-rel; **the auxiliary system** — Slice 6-aux, complete: progressive/perfect, passive
(short + agentive), and modals via opaque `Prop→Prop` operators). Remaining: the tail (Slice 6-tail —
case, pronouns/anaphora) and the operational scale-up onto full WordNet (Slice 7). The §8 slice plan
carries the authoritative per-slice status.*

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
- **Forward bounded type-raising `T`** (+ its Eisner `TypeRaised`-can't-apply clause) and **restrictive
  relative clauses** **are in** (Slice 6-T/6-rel, §8.9): `T` raises `NP_X → S/(S\NP_X)`, object relatives
  build the body via `T` + the existing forward-`B¹`, and an engine relativizer Σ-refines the noun (reusing
  3b). **Forward composition (`B¹`)** + its Eisner normal form is in (Slice 5c); determiners use *lexical*
  type-raising (`cat_forall`).
- **The rest of the combinator set** (generalized `B^n`, crossing `B×`, backward composition) is **not**
  built — restrictive relatives do not need it (no dead combinators); it returns only when a construction
  demands it. Plus **auxiliaries** (progressive/perfect/passive/modals) and the **tail** (case, pronouns) —
  Slice 6 (its **6-neg** and **6-T/6-rel** sub-slices **are in**; Slice 3 complete).
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
determiners that exercise it. (Touches `dcg::lemmatizer`, `dcg::lookup`, `MorphyLemmatizer`.) *Determiner–
noun* agreement landed in Slice 2; *subject–verb* agreement (the verb side of this deferral) is designed in
**§8.10 (Slice 6-agr)**.

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
never a core engine dependency. (Anaphora resolution is specified in [D64](d64-llm-anaphora-resolution.md):
an LLM resolver behind the felicity oracle — the dispatched institution made concrete.)

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
  **6-neg ✅** (verbal + copular negation, `¬P := P → logic:False`). **6-T + 6-rel ✅** (§8.9):
  forward **bounded-`T`** (target `S`) + the existing forward-`B¹` + an engine relativizer reusing 3b
  refinement — *not* the full combinator set (no `Bⁿ`/`B×`/backward needed for restrictive relatives).
  **6-aux** partial: its **importer verb-form morphology keystone ✅** (the importer generates + emits the
  full `bse`/`fin`-3sg/`ger`/`pss` paradigm per verb — `pss` grounded against `verb.exc`, felicity-clean at
  corpus scale; `bse`≠`fin` makes do-support/questions/negation/modals fire on imported verbs) and
  **progressive + perfect ✅** (finiteness-lifting `be`-over-`ger` / `have`-over-`pss` auxes, sem `λP.P`,
  reusing `copula_sem`), **short/agentless passive ✅** (`be` over the unsaturated `pss` TV, ∃-closing the
  agent), **agentive long passive ✅** ("…by HeLa" via a `pass` voice feature + `by` agent-marker), and
  **modals ✅** (opaque `logic:Possible`/`Necessary : Prop→Prop`; `can`/`must` etc. do-support-shaped
  auxes). **6-aux is complete.** **6-agr ✅** — **subject-verb agreement (§8.10):** the finite verb's
  `num_any` subject slot is replaced by real `sg`/`pl` agreement (sg `affects` / pl-finite `affect`), with
  determiner/proper-noun/group/aux number propagation — closing the verb side of the §5.1 number deferral.
  Agreement through an *object* determiner is now enforced too, via **feature variables**
  (`cat_fin_forall`/`cat_num_forall` + `unify_feat`, OpenCCG/Carpenter precedent) — closing the former
  limitation and removing a ~3× WordNet forest inflation (§8.10). **6-cl ✅** — **clausal complements (§8.11):** clause-taking
  verbs ("X shows that Y") via a `cat_cp` embedded-clause category + a `that` complementizer + an opaque
  `Prop→Entity→Prop` report axiom (intensional — the complement is not asserted); importer frame-26. **6-cmp** —
  **comparatives ✅ (§8.12); superlative deferred (gated on "the").** Degree semantics reusing D52 —
  degrees are `core:float`, comparison is the opaque `stats:gt`, a gradable adjective is a measure `deg_A :
  Entity→float`; "X is larger than Y" → `gt(deg_large(x), deg_large(y))`, "X is large" →
  `gt(deg_large(x), std_large)` (unified positive). The comparison-morphology generator (grounded suppletive
  table + periphrastic) is built; the importer flags gradability by the WordNet pertainym pointer
  (descriptive ⇒ gradable measure + comparative; relational ⇒ Boolean) — corpus felicity-clean.
  **6-mod** — **nominal modification (§8.13): ✅ done.** The highest-leverage gap for real scientific prose
  (most of the WRN litmus). Four opaque, institution-mapped rules reusing 3b's Σ-refine: named-entity
  compound `[NP] [N] → Σx:C. compound(x, m)`; N-N kind compound `[N] [N] → Σx:C. compound_kind(x, M)`;
  prepositions as `(S\NP)\(S\NP)` VP-adjuncts (`λx.λV.λs. And(V(s), prep_*(s, x))`) AND as `cat_pp` post-
  nominal noun-modifiers (`[N] [PP] → Σx:C. prep_*(x, y)`) — both entries per preposition, so PP-attachment
  ambiguity is carried in the forest. Left-branching NF collapses 3+-noun compound chains to one bracketing. Deferred: **6-tail** (case + pronouns — case is the syntactic half, pronouns gated on
  **anaphora resolution**, designed in [D64](d64-llm-anaphora-resolution.md): an LLM resolver behind the
  felicity oracle, pronouns → committed-resource IRIs).
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

**Status (in progress).**
- **Done-when #1 — ✅ met at full scale (Derived).** `--all` imports **114,038 synsets → 325,259
  `LexicalEntry` entries** (74,385 noun classes, 7,730 instances, 15,128 verb axioms, 18,156 adj axioms;
  57,349 ger/pss participle forms), and **all 325,259 felicity-gate clean** (0 structural errors, 0
  rejects). Witnessed baseline: validate pass ≈ 2:20 wall, ≈ 9.4 GB RSS (release). Stage A (`--limit 1000`)
  → 3,129 synsets → 14,288 entries, also 100% clean, ≈ 5 s / 0.6 GB.
- **Instance-vs-class residual — ✅ FIXED (a real Stage-C scale finding, fail-closed).** The first `--all`
  run surfaced **301 structural errors** invisible at Stage A: WordNet has **instance-of-instance `@i`
  chains** (Paternoster `@i` Lord's_Prayer `@i` prayer — 56 cases) and **class-`@`-instance** edges
  (British_West_Indies `@` West_Indies `@i` archipelago — 14 cases). An individual cannot be typed by — nor
  a class be a subclass of — another individual. Fix (`convert.rs::nearest_classes`): every `@i`/`@` target
  is resolved to the nearest **class** (climb instance parents), so an instance's type and a class's
  `subclass_of` always reference a `core:Class`; the intermediate instances collapse into co-referential
  individuals of that class. Witness: `instance_of_instance_chain_resolves_to_nearest_class` +
  `instance_synset_is_an_individual_not_a_class`; confirmed by the 301→0 drop on `--all --validate`.
- **Done-when #2 — ✅ met (Derived).** The **standing layer + `LexicalIndex::build` + battery** is wired
  (`crates/eigenius-wordnet/tests/wordnet_scale.rs`): `select_synsets` → `render_document` → compile over
  the bootstrap head → `LayerBuilder::build` → hold the `Arc<Layer>` → `LexicalIndex::build` → `parse` with
  WordNet's Morphy → kernel-gate. The Stage-A battery (5 declaratives over **real** WordNet nouns/verbs +
  the committed determiners, vocabulary guaranteed by seeding) all yield ≥1 felicitous `Prop`
  (`stage_a_battery_parses_to_props_over_real_wordnet`, always-on).
- **How the layer "stands."** *In-process* (the harness): build once, hold the `Arc<Layer>`, index + parse.
  *Persisted:* **the platform's generic layer-load path, not WordNet-specific** — `eigenius serve --db <path>`
  then `eigenius --endpoint <addr> load wn.esl` commits the import as an ordinary layer onto the server's
  RocksDB and advances the branch (a verified round-trip showed ≈1.5 ms layer reload). The importer's job is
  only to **emit the lexicon** (`--out wn.esl` + `--validate`); committing/loading/persisting it is the same
  machinery as any layer. *(An earlier `wordnet-import --commit/--from` reimplemented this in the importer —
  removed as redundant with `serve`+`load`.)* Note the layer reloads fast but the **`LexicalIndex` is an
  in-memory derived structure, NOT persisted** — rebuilt per load (~14 s at 170k entries); caching it is a
  separate concern from layer persistence. The selection logic lives in `eigenius_wordnet::import`
  (`SeedSpec` + `select_synsets`), shared by the bin and the harness.
- **Done-when #3 — ✅ baselines recorded (Derived).** Build cost scales with corpus size; **forest size is
  driven by per-word polysemy, not corpus size**. Measured (release):

  | scale | entries | layer build | index build | forest / 4–5-word sentence |
  |---|---|---|---|---|
  | Stage A seeded (all senses present) | 1,310 | 0.11 s | 0.08 s | **56 – 630** (genuine) |
  | Stage B `--limit 12k` (partial senses) | 169,997 | 13.1 s | 14.1 s | 12 – 42 |
  | Stage C `--all` (validate pass) | 325,259 | — | — | (= Stage A per-word, all senses) |

  Per-sentence parse is single-digit ms (≤ ~0.4 s with first-call warmup). The Stage-B `--limit` forest is
  *smaller* only because the cap omits some senses; the **seeded numbers are the honest full-WordNet
  ambiguity**. *(Correction: the first measurement read 168 – 1,890; investigation traced ~3× of that to
  the object-determiner feature-laundering bug — now fixed via feature variables, §8.10. The deduped figures
  above are the genuine per-word polysemy.)* Even deduped, **`props == forest` — the felicity gate prunes
  nothing; the forest is pure sense-polysemy** — so a Stage-B ranking/cap is *required*, not optional, for
  the engine to be usable on real vocabulary. The genuine forest is a **sense product**: `no cat eats a
  fish` = 56 = 7 (`cat`: feline / lion / Caterpillar-tractor / cat-o'-nine-tails / khat / spiteful-woman /
  "guy") × 4 (`eat`: corrode / consume / ingest / worry) × 2 (`fish`: animal / flesh) — all type-correct
  because every noun roots at `Entity` and every transitive verb is `Entity → Entity → Prop` (§8.7.4), so
  felicity can't distinguish "ingest(fish)" from "corrode(fish-flesh)". The in-kernel lever to prune these
  felicitous-but-implausible combinations is **selectional restrictions** (more-specific verb argument
  types), tracked as **issue #93** — complementary to the rank (which already surfaces the all-sense-1
  reading first) and to downstream WSD (§6).
- **Done-when #4 — ✅ BUILT: rank + cap.** Policy: order the forest by WordNet
  **sense frequency**, return the top-K, and **log the dropped-tail count** (never silently truncate). The
  mechanism keeps the engine sense-agnostic (the §6 forest-returns boundary intact) by carrying a *generic*
  parse-cost, sourced from the lexicon:
  - **Importer** emits a per-entry **`lexicon:sense_rank`** (a 0-based frequency rank): for each lemma, its
    senses in `index.<pos>` order are already frequency-sorted (sense 1 = most frequent), so rank = the
    lemma's sense index. Read `index.<pos>` (today only `data.<pos>` is read) to recover it; closed-class /
    demo entries default to rank 0.
  - **Parser** gains a generic `cost: u32` on `Item` — a leaf's cost is its entry's `sense_rank`; the
    combinators (`apply`, composition, coordination, the 6-mod/3b engine rules) **sum the children's costs**.
    The kernel never learns the cost *means* "sense frequency"; it only sums an abstract weight.
  - **`LexicalIndex::parse`** returns the forest **sorted by ascending cost** (all-sense-1 readings first)
    and **capped to `DEFAULT_FOREST_CAP` = 256**, logging `dropped = forest.len() − K`. Cost 0 throughout
    (closed-class/demo) ⇒ existing single-parse tests are order- and cap-stable (K well above their forest
    sizes). Witnesses (`wordnet_scale`): the Stage-A battery asserts the forest is cost-sorted and
    cap-bounded; the cap log fires on the genuinely-large cases ("a dog sees a bird" 630→256).

  Built across `lexicon-ontology.esl` (the `sense_rank` property), `eigenius-wordnet` (`read_sense_ranks`
  from `index.<pos>` + emit; 111,698 of 325,259 `--all` entries carry a non-zero rank), and `kernel::dcg`
  (`Item.cost` summed by the combinators; `parse` sorts + caps). Other D62 §8.7 residuals (#91 multi-class
  NP — already handled by the check-mode resource rule; troponymy subsumption) re-confirm at battery time.

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

**6-T + 6-rel — type-raising + relative clauses. ✅ Done.** Restrictive relatives ("a cell line
**that affects HeLa**", "a cell line **that HeLa affects**") → a **refined noun**, reusing 3b's Σ-refinement +
the determiner-over-refined-noun `Fst` rule. `T` exists *to serve* relativization (object extraction), so it
lands *with* relatives. *As built:* `type_raise`/`relativize` in [`category.rs`], the `TypeRaised`
provenance + Eisner clause in [`parser.rs`] (forward application rejects a `TypeRaised` left), and the unary
`T` pass + the `that`-keyed relativizer rule in [`lookup.rs`]'s CKY loop. Tests (`closed_class_determiners`):
`subject_relative_clause_parses`, `object_relative_clause_parses`,
`relative_clause_refines_the_noun_to_a_sigma_over_its_base_type`, `type_raising_keeps_plain_declaratives_single`
(the regression gate). Worked design:

*Subject relatives — application only (no `T`).* In "that affects HeLa" the body "affects HeLa" is a VP
`S\NP` (sem `λx. affects(x, hela)`); the gap is the adjacent subject.

*Object relatives — the reason for `T`.* In "that HeLa affects" the body "HeLa affects [gap]" has an object
gap, built as `S/NP` by **type-raising the embedded subject + forward composition**:
- **`T`** (forward, **bounded** to target `S` per §5.2 — a fixed target ⇒ terminating unary closure): raise
  `NP_X → S/(S\NP_X)`, sem `λV. V(x)`.
- `affects = (S\NP_subj)/NP_obj`; then `T(HeLa) ∘ affects` (forward **`B`**, the 5c combinator) →
  `S/NP_obj`, sem `λx. affects(x, hela)`. Exactly the 5c `S/NP` shape — and, as there, it is consumed as an
  **argument** (by the relativizer), so the existing forward-`B` Eisner clause (a `>B` output can't be a
  *functor*) leaves it untouched.

*The relativizer — an engine refine rule (reusing 3b), not a categorial `(N\N)/(S/NP)` entry.* Both bodies
(`S\NP` / `S/NP`) have sem `body : Entity → Prop` (the gap-filler is the λ-argument), so a single rule
covers them: `[noun cat_n(C)] that [body]` → `cat_n(Σx:C. body(x))`, built over the **concrete** `C` (so
`body(x)` type-checks — dodging the abstract-`C` bounded-quantification gap, exactly as 3b's attributive
did). `that` is a reserved relativizer (like `each other`); the refined noun then rides 3b's
determiner-over-refined-noun + `Fst` machinery unchanged. **Why engine, not categorial:** a categorial
`(N\N)/(S/NP)` relativizer would re-hit the abstract-`N` Σ gap that needs kernel bounded quantification —
the same reason 3b's attributive went engine. Consistent; no kernel work.

*Eisner extension for `T`.* A new `TypeRaised` provenance, with one constraint: **forward application
rejects a `TypeRaised` left operand** — a raised functor may only **compose** (`>B`), never apply. This
kills the spurious `T`-duplicate (a plain "HeLa affects BRCA1" must not get a `T(HeLa)`-application
derivation replicating backward application), so existing single-parse declaratives stay single (the
**regression gate**). A `T` output only composes, and only with a verb still seeking its object (`B/C`) —
which arises only in extraction.

*Decisions (resolved):* bounded-`T` target = `S`; relativizer = engine refine rule (reuse 3b); **both**
subject + object relatives (subject is free once the refine rule exists; object is the point of `T`); Eisner
= the `TypeRaised`-can't-apply clause. *Deferred:* non-restrictive (",which…"), reduced relatives ("the gene
affecting HeLa"), pied-piping ("the gene to which…"); `which`/`who` beyond `that` are lexical follow-ons.
*Note — we do NOT need the full combinator set:* only forward `T` (bounded) + the existing forward `B¹`;
generalized `Bⁿ`, crossing `B×`, and backward composition are **not** required by restrictive relatives and
stay unbuilt (no dead combinators).

*Build order (as built):* (1) `T` unary rule + `TypeRaised` provenance + the Eisner clause — the regression
gate held (every existing test single-parse, `type_raising_keeps_plain_declaratives_single`); (2) the
relativizer refine rule with subject-relative (application) bodies; (3) object relatives (`T`+`B` bodies)
fell out once (1)+(2) landed. Witnesses (with the demo lexicon's `cell line`/`affects`/`primary`/HeLa):
"every cell line that affects HeLa is primary" and "every cell line that HeLa affects is primary" → a
refined-noun NP whose verb-saturated sentence is kernel-checked `Prop` (single parse each).

**6-aux — auxiliaries beyond do-support. Splits by dependency; the shared morphology keystone is in.**

*Importer verb-form morphology — the keystone. ✅ Done.* The three aspect/voice auxiliaries all
selected non-finite participles the importer didn't emit (it gave the lemma/finite form only). Generation
is *not* the inverse of Morphy: Morphy lemmatizes (inflected → base) and `verb.exc` doesn't tag which
irregular form is the past participle (`went`/`gone` both reduce to `go`), so the pp can't be recovered
from it. Built as [`eigenius-wordnet::inflect`]: **gerund** (`ger`) is pure orthography (English has no
irregular present participles — silent-`e` drop, `ie→y`, `ee/oe/ye` retention, monosyllabic doubling);
**past participle** (`pss`) is regular `-ed` plus a **grounded ~270-base irregular table**. The table was
sourced from Wikipedia's *List of English irregular verbs* and **witnessed against the in-repo WordNet
`verb.exc`** — every shipped form is an invariant (`cut`/`put`) or an attested inflection of its base;
unattested extractions (kempt, durst, holpen) are dropped fail-closed, and the common `-t`/`-ed` twins
(burnt→burned) are recovered productively from the attested `-t` so no non-word (weared) is ever admitted.
The importer's `push_verb` now emits, per lemma, the **full paradigm** — base (`bse`, the lemma surface),
finite 3sg (`fin`, generated "affects" via `third_singular` — regular `-s`/`-es`/`-ies` + the two
irregular auxiliaries be→is/have→has), gerund (`ger`), past-participle(s) (`pss`) — all pointing at the
*same* predicate axiom (finiteness erased by `⟦·⟧`, so `⟦cat⟧ = sem_type` unchanged → felicitous by
construction). Emitting **`bse` distinct from `fin`** is what makes do-support (polar / object-wh
questions, declarative do-support, verbal negation) and modals fire on **imported** verbs, not just the
hand demo; it also **fixes the former base-as-`fin` mistag** (the lemma was tagged `fin`, so bare "affect"
wrongly parsed as a finite root — now it is `bse`, a non-root). Witnesses: `inflect` unit tests
(`third_singular_present`, the gerund/pp tests) + the `irregular_pp_attested_in_verb_exc` corpus gate;
the verb lexicon imports felicity-clean at scale (e.g. a 2,000-synset sample → 16,250 entries, all four
`bse`/`fin`/`ger`/`pss` forms in balance, **16,250/16,250 admitted by `--validate`**).
*(Also fixed in passing: the `--validate` self-check double-loaded the lexicon schema — now in the
bootstrap chain — re-declaring `Mood:dcl`; it now compiles the import directly over the bootstrap head.)*

With the forms in the lexicon, each auxiliary is unblocked on the morphology side:
- *Progressive / perfect* ("is affecting", "has affected"). **✅ Done.** The auxes are finiteness-lifters
  `(S[dcl,fin]\NP) / (S[dcl,ger|pss]\NP)`, sem `λP.P` (aspect/tense erased), selecting the `ger`/`pss` VPs
  the importer now emits — *as built:* `is_prog`/`are_prog`/`was_prog`/`were_prog` (over `ger`) and
  `has_perf`/`have_perf`/`had_perf` (over `pss`) in [`closed-class.esl`], all reusing `copula_sem`.
  Application-only, no new combinators; the predicate axiom is unchanged. The exact `ger`/`pss` complement
  slots fail closed (`*HeLa is affect BRCA1`, `*HeLa has affecting BRCA1` get no parse); `is`/`are` stay
  unambiguous with the copula (distinct `adj` vs `ger` slots). Tests (`closed_class_determiners`):
  `progressive_auxiliary_parses`, `perfect_auxiliary_parses`, `aspect_auxiliaries_select_the_right_participle`.
- *Passive* — a **voice alternation** (the surface subject is the logical object; the agent is demoted).
  **Short / agentless passive ✅** ("BRCA1 is affected" → `∃a:Entity. affects(brca1, a)`): a single `be`
  entry (`is`/`are`/`was`/`were` in [`closed-class.esl`]) that takes the **unsaturated** transitive
  past-participle TV `(S[dcl,pss]\NP)/NP` and closes the agent with the impredicative ∃
  (`passive_sem = λTV.λp. ∃a. TV(p,a)`). Taking the TV *before* its object slot is filled is what blocks
  over-generation — once "affected BRCA1" saturates to `S[pss]\NP`, the passive `be` no longer matches, so
  `*HeLa is affected BRCA1` has no parse. No engine rule, no new feature. Tests:
  `short_passive_parses_with_existential_agent`, `passive_be_rejects_a_saturated_participle`.
  **Agentive long passive ✅** ("BRCA1 is affected by HeLa" → `affects(brca1, hela)`, the agent supplied):
  a new **voice feature** `pass` + a `by` agent-marker + passive `be` over the `pass` VP — all lexical
  ([`closed-class.esl`]), no engine rule. `by` consumes the agent NP then the unsaturated active TV on its
  left, yielding the patient-VP `S[dcl,pass]\NP` (`by_agent_sem = λagent.λTV.λp. TV(p, agent)`); `be`
  (`is`/`are`/`was`/`were`) lifts it (`λP.P`). The `pass` result is the over-generation guard: an active
  object-saturated participle is `S[pss]\NP` (not `pass`), so it can never feed passive `be`, and
  `*BRCA1 is affected HeLa` still has no parse (`passive_be_rejects_a_saturated_participle`). Test:
  `agentive_long_passive_parses_with_the_by_agent`. (Ditransitive passives — "given to", second-object
  promotion — remain a follow-on.)
- *Modals* ("can/must affect"). **✅ Done.** Resolved the logic-layer decision in favour of **opaque
  operators**: `axiom logic:Possible : Prop → Prop` and `axiom logic:Necessary : Prop → Prop` (◇/□) —
  kernel-uninterpreted (no Kripke/world-indexing), witnessed downstream like `ontology:is_a`, so a modal
  claim can be carried as a hypothesis; **no modal laws** baked in (T/4/5/duality are flavor-dependent
  opt-in axioms), independent intuitionistic primitives, flavor-agnostic (flavor grounding-supplied).
  *As built:* the operators in [`logic.esl`]; modal auxes `can`/`could`/`may`/`might` (→ `Possible`) and
  `must` (→ `Necessary`) in [`closed-class.esl`], a do-support-shaped `(S[dcl,fin]\NP)/(S[dcl,bse]\NP)` aux
  wrapping the proposition (`λP.λs. Possible(P(s))`). "HeLa can affect BRCA1" → `Possible(affects(brca1,
  hela))`. The kernel accepts the `Prop → Prop` axiom + its application (impredicative `Prop`); the
  base-VP slot fails closed (`*HeLa can affects BRCA1`). Tests: `modal_can_wraps_the_proposition_in_possible`,
  `modal_must_wraps_the_proposition_in_necessary`, `modal_selects_a_base_vp`. *Refinements (follow-on):*
  epistemic "may/might" → the Declared **grade** rather than `Possible`; `should`/`will`/`shall`; flavor
  tags harvested from use.

**6-tail — case + the long tail. Deferred (demand-driven).** Pronoun case (he/him), who/whom, possessives,
adverbs, non-restrictive/reduced relatives, … — each its own construction. (Two items have graduated out of
the tail into designed slices: **clausal complements** — "X shows that Y" — **§8.11 (6-cl)**, and
**comparatives/superlatives** — "X is larger than Y" — **§8.12 (6-cmp)**.) The big one, **pronouns**, is only useful with **anaphora
resolution** (resolve to a chain resource by IRI, §5.3) — a real feature, not a lexical entry, **designed
in [D64](d64-llm-anaphora-resolution.md)**: an LLM resolver behind the felicity oracle (a *dispatched
institution*, §5.3), pronouns parse to typed `Exp::Anaphor` holes that resolve to committed-resource IRIs
and re-gate through the kernel (Derived verdict, D61). Case is the cheap syntactic half (a `Case` feature),
folded into the pronoun lexicon. Not a discrete deliverable; items land as the target corpus demands them.

### 8.10 Slice 6-agr — subject-verb agreement

**✅ Done (full scope, one documented limitation).** *As built:* the demo + the importer (`push_verb`)
emit the verb as sg-`fin` (3sg, subject `sg`) + pl-`fin` (lemma surface, subject `pl`) alongside
`bse`/`ger`/`pss` (subject `num_any`); proper nouns / `@i` instances are `sg`; the subject determiners carry
their `Num` into the VP slot (`fin`-tightened to exclude `bse`); the finite auxiliaries are sg/pl
(`is`/`has`/`does`/`was` → `sg`, `are`/`have`/`do`/`were` → `pl`; `did`/`had`/modals/kind-copula stay
`num_any`); and the distributive-subject rule ([`parser.rs`]) checks the group's `Num` against the VP
([`feat_meets`]). Witnesses (`closed_class_determiners`): `subject_verb_agreement_bites`
(`HeLa affects` ✓ / `*HeLa affect`, `*every cell line affect`, `*HeLa and BRCA1 affects` ✗),
`auxiliary_agreement_bites` (`HeLa is/has …` ✓ / `*HeLa are/have …` ✗); corpus import stays felicity-clean
(the 5-form paradigm: bse/sg-fin/pl-fin/ger/pss, all admitted by `--validate`).

**Feature variables (✅ resolved — was a documented limitation).** Subject agreement is now enforced
**through an *object* determiner** too. The bug: `a_obj`/`every_obj`/… fixed the verb's result-clause
finiteness + subject-number as the constant `*_any`, which both *accepts* a non-finite/plural verb and
*launders* its features, so the laundered VP slipped past the subject determiner — admitting `*every cat eat
a fish` **and** inflating the WordNet forest ~3× (Morphy reaches a verb's base/plural forms from the 3sg
surface, and all three then completed identically). The precedent (OpenCCG `tiny.ccg`, Carpenter 1997 —
typed feature structures with unification) is **feature variables**, not `*_any` wildcards. Implemented
(D63 §8.10): two denotation-transparent binders `cat_fin_forall`/`cat_num_forall` (⟦·⟧ erases features, so
they add no Π and the determiner's `sem_type` is unchanged); `unify_feat` (the binding-aware generalization
of `feat_meets`, parallel to `unify_type` for the type index) binds a feature variable from the consumed
verb, and `subst_cat` propagates it; the object determiners are retyped `(S[f]\NP[n]) \ ((S[f]\NP[n])/NP[T])`
so the verb's real finiteness/number flow through to the VP. The parser strips the binders at seed time, so
`f`/`n` are free vars bound call-locally during application. Result: a base verb → `S[bse]` VP → rejected at
the finite root; a plural verb → `NP[pl]` subject → rejected by the singular subject determiner; only the
3sg reading survives. Witnesses: `feature_variable_binds_meets_and_is_occurs_consistent` +
`feature_binder_is_denotation_transparent` (`dcg::category`), `singular_subject_rejects_bare_and_plural_verb`
+ `no_spurious_duplication_from_feature_vars` (`wordnet_scale`). The ~3× forest inflation is gone (e.g. "no
cat eats a fish" 168→56, = its distinct-meaning count).

Closes the verb side of the §5.1 number deferral. *Determiner–noun*
agreement landed in Slice 2 (`cat_forall` carries the determiner's `Num`; `apply` checks it; `LexicalIndex`
refines a noun's `num_any` to the surface number — `every gene` ✓ / `*every genes` ✗). *Subject–verb*
agreement is the gap: the finite verb's subject slot is `cat_np(Entity, num_any)`, so no agreement bites
(`*every gene affect`, `*HeLa and BRCA1 affects` both slip through), and the 6-aux morphology completion
(§8.9), by splitting the lemma into `bse` + 3sg-`fin`, removed the *accidental* plural-finite coverage the
old base-as-`fin` mistag provided. This slice makes agreement actually bite.

*The phenomenon.* English present tense: a **3sg** subject takes verb **+s** ("the gene affect**s**"); a
**non-3sg** (plural; also 1sg/2sg) takes the **base** ("genes affect"). So the finite verb has two
present forms — sg `affects` (subject `sg`) and **pl-finite `affect`** (subject `pl`, `fin`). The pl-finite
surface equals the base, but it is a **distinct entry** from the `bse` form (the do-support / modal
complement): `bse` only fills an auxiliary's base slot, `fin`-pl only heads a clause with a plural subject.
*(Person is number-only for now — entities are 3rd person, so `sg`=3sg→+s, `pl`→base; 1st/2nd person
arrive with pronouns, D64, and fold into agreement then, per §5.1.)*

*The mechanism (reuses `feat_meets`).* Agreement = **the subject's `Num` meets the finite verb's
subject-slot `Num`**. No new feature or kernel change — `Num`/`feat_meets` already exist; this is a
category-shape + lexical-entry change (+ importer emission). Touch points (full scope):
1. **Lexical finite verb → two forms:** sg `affects` (subject slot `sg`) + pl-finite `affect` (subject slot
   `pl`, `fin`). The importer emits both (sg via `third_singular`; pl = the lemma surface, no generation);
   the demo gains the pl-finite entry. (`bse`/`ger`/`pss` unchanged.)
2. **Determiners carry their number into the VP slot:** `every`/`each`/`a`/`some`/`no` → `sg`, `all` → `pl`
   (the VP slot's subject `Num`). And **tighten the determiner VP clause-feature `fin_any → fin`** so the
   `bse` form can't slip in as a spurious second parse of a determiner subject.
3. **Proper nouns / `@i` instances → `sg`** (a KG entity is singular; was `num_any`) — `HeLa affects` ✓ /
   `*HeLa affect` ✗.
4. **Coordinated groups are `pl`** (already) — the distributive / collective / reciprocal VP takes a
   `pl` subject, so `HeLa and BRCA1 affect …` ✓ / `*… affects …` ✗.
5. **Finite auxiliaries get `sg`/`pl` subject slots** (they already exist as pairs): `is`/`has`/`does`/`was`
   → `sg`, `are`/`have`/`do`/`were` → `pl`. **Modals do not inflect** (`can`/`must` — "he can", not
   "he cans"), so they stay number-invariant (`num_any`) — correct, not an omission.

*Number-invariant subjects.* Kind subjects (`cat_kind`, the bare-plural / generic path) carry no `Num` and
read as plural ("Genes are cell lines"); the kind copula `are` is the `pl` form — consistent. Mass /
uncountable subjects are deferred.

*Test churn (honest heads-up — this slice changes which sentences are grammatical).* The current
distributive / disjunctive / reciprocal / n-ary tests pair a **plural** coordinated subject with the **sg**
verb `affects` ("HeLa and BRCA1 **affects** HeLa") — under agreement these become `… **affect** …`. ~6–8
test sentences move to the agreeing form (object-distribution and singular-subject tests are unaffected —
their subject is singular `HeLa`). This is the feature doing its job, not a regression.

*Build order (when taken):* (1) lexical sg/pl finite forms + proper-noun/instance `sg` + the importer
pl-finite emission; (2) determiner VP-`Num` + the `fin_any → fin` tightening; (3) aux `sg`/`pl` subject
slots; (4) group/distributive `pl`. *Verify:* agreement bites (`*HeLa affect`, `*every gene affect`,
`*genes affects`, `*does genes affect`/`*HeLa are affecting`) and the agreeing forms parse single; the
churned coordination tests pass on the corrected sentences; corpus import stays felicity-clean.

*Decisions:* full scope (verbs + determiners + proper nouns + groups + finite auxes); proper nouns `sg`;
modals number-invariant; person + mass nouns deferred (person with pronouns, D64).

### 8.11 Slice 6-cl — clausal complements

**✅ Done.** *As built:* the `cat_cp` constructor + its `denote_cat` arm (⟦·⟧ = Prop); the complementizer
`that_comp` (`cat_cp / cat_s(dcl,fin)`, sem `λp.p`) in [`closed-class.esl`]; a demo report verb (`shows`,
an opaque `Prop → Entity → Prop` axiom, sg/pl finite forms) + the importer un-deferring **frame 26** →
`FrameKind::Clausal` (emitting `(S\NP)/cat_cp` + the report axiom, full agreement paradigm). Witnesses
(`closed_class_determiners`): `clausal_complement_parses_intensionally` ("HeLa shows that BRCA1 affects
HeLa" → `shows(affects(hela, brca1), hela)`, single parse, head = the opaque `shows` so the complement is
**not** asserted), `embedded_cp_is_not_a_clause_root_and_relativizer_still_works` (leading "that …" no
parse; the 6-rel relativizer `that` unaffected); convert test `clausal_verb_emits_report_axiom_and_cp_category`;
corpus import felicity-clean with frame-26 verbs (`--validate`). Graduated the most tractable 6-tail item:
**clause-taking verbs**
("Smith **shows that** BRCA1 affects HeLa", "the assay **demonstrated that** …") — subject + verb + a
*that*-clause complement. High-value for scientific prose, and it un-defers a WordNet verb frame the
importer currently drops. No new subsystem; it's a higher-order verb category over an embedded clause.

*Scope.* **In:** subject + clause-taking verb + `that` + a **finite declarative** complement (WordNet
**frame 26**, "Somebody ----s that CLAUSE"). **Deferred:** interrogative complements ("knows **whether** Y"
— the complement is a question, frames 29/34); **subject** clauses ("That Y is surprising"); **bare**
(that-less) complements ("shows Y affects Z"); the **expletive** "it is known that Y" (needs the
pronoun/expletive `it`, D64) and the passive of report verbs; **control / raising** ("wants **to** affect",
frames 24/25/… — infinitival, a different construction).

*The categorial treatment.* Three pieces:
- **`cat_cp`** — a new `Cat` constructor for the **embedded complement clause**, `⟦cat_cp⟧ = Prop`. It is
  **not a clause root** (the root filter checks `cat_s`), which is what keeps a stray leading "that …" from
  parsing as a standalone sentence.
- **Complementizer `that`** — a closed-class lexical entry `cat_cp / cat_s(dcl, fin)`, sem `λp. p`
  (semantically vacuous — the embedded clause's `Prop`). "that BRCA1 affects HeLa" → `cat_cp`, sem
  `affects(brca1, hela)`.
- **Clause-taking verb** — `(S[dcl,fin]\NP) / cat_cp`, **object-first**, `⟦cat⟧ = Prop → Entity → Prop`; an
  **opaque axiom** `shows : Prop → Entity → Prop` (the agent–proposition report relation). "Smith shows
  that Y" → `shows(Y, smith)`.

*Opacity / intensionality (the load-bearing semantic point).* `shows(Y, x)` does **not** assert `Y` — the
complement sits in a **non-veridical** context (Smith may be wrong; `shows(P, x) ⊬ P`). The opaque-axiom
treatment gives this for free (felicity ≠ truth; the report's truth is a grounding judgment / `ChainWitness`
downstream, like `is_a` and the modal operators). This is *why* the verb is an `axiom`, not a reduction —
collapsing the complement to its truth value would wrongly make every reported claim asserted.

*The `that` overload (relativizer vs. complementizer) — no interference.* `that` already drives the
**relativizer** reserved-word rule (§8.9 6-rel: `[noun] that [gapped body]`). The complementizer is a
distinct **lexical** `cat_cp/S` entry (`[CTV] that [full S]`). They never collide: the relativizer needs a
**noun** on the left and a **gapped** body (`S\NP` / `S/NP`) on the right; the complementizer needs a
**verb** on the left and a **full finite S** on the right. Distinct left and right contexts, and `cat_cp`
not being a root kills the leading-"that" spurious sentence. (Verified by a no-regression check on the
relative-clause tests.)

*Kernel.* The clause-taking axiom takes a **`Prop` argument** — the same capability the modal operators
already exercise (§8.9; impredicative `Prop`); `axiom : Prop → Entity → Prop` to confirm at build (expected
fine). Adds one `denote_cat` arm (`cat_cp → Prop`) and the `cat_cp` ctor to the lexicon schema.

*Importer.* Un-defer **frame 26** in `convert.rs` `classify` → a new `FrameKind::Clausal`, emitting the
verb as the opaque axiom `Prop → Entity → Prop` with cat `(S\NP)/cat_cp`. The full morphology paradigm
(§8.9: `bse`/`fin`-3sg/`ger`/`pss`) applies unchanged (shows / show / showing / shown). The `whether`
frames (29/34) and control/raising frames stay deferred.

*Build order (when taken):* (1) `cat_cp` ctor + `denote_cat` arm + the complementizer `that` entry +
a demo clause-taking verb (axiom + paradigm entries); (2) importer frame-26 → `Clausal` emission;
(3) tests. *Verify:* "HeLa shows that BRCA1 affects HeLa" → `shows(affects(brca1, hela), hela) : Prop`,
single parse; the embedded proposition is **not** independently asserted (the sem is headed by the opaque
`shows`, not by `affects`); a leading "that …" does not parse as a sentence; the 6-rel relativizer tests
still pass (no regression from the `that` overload).

*Decisions:* `cat_cp` (not bare-`S`, which would let a leading "that" form a sentence and overload the
relativizer); `that` **required** (optional that-less complements = a follow-on unary `S → cat_cp` shift);
declarative complements only; the verb is an **opaque** `Prop → Entity → Prop` axiom (intensional). Deferred:
interrogative / subject / expletive / control complements.

### 8.12 Slice 6-cmp — comparatives & superlatives (degree semantics)

**✅ Done (comparative; superlative deferred, gated on "the").** *Built and green:* the `cat_pp_than`
constructor + `denote_cat` arm (⟦·⟧ = Entity); the `than` marker (`cat_pp_than / cat_np(Entity)`, sem
`λy.y`) in [`closed-class.esl`]; a demo gradable adjective `large` (measure `deg_large : Entity → core:float`
+ standard `std_large`) with the **comparative** `larger` (`(S[adj]\NP)/cat_pp_than`, `λy.λx.
measurements:gt(deg_large(x), deg_large(y))`) and the **measure-based positive** (`gt(deg_large(x),
std_large)` — combo 1), both reusing the copula. The **comparison-morphology generator**
([`eigenius-wordnet::inflect`] `comparison`): regular `-er`/`-est` + the grounded suppletive table + the
periphrastic heuristic. The **importer scale-out**: a `wndb` **pertainym flag** (`\` → relational =
non-gradable) + `push_adj`'s **gradable path** — descriptive adjectives emit `deg_A`/`std_A` + SemTerm-based
measure-positive + synthetic comparative entries (via `comparison`); **relational** adjectives stay Boolean
`is_A`. Witnesses: `comparative_compares_degrees`, `positive_gradable_adjective_is_measure_based`,
`comparative_requires_than` (`closed_class_determiners`); `comparison_regular_irregular_and_periphrastic`,
`gradable_adjective_emits_measure_positive_and_comparative`, `relational_adjective_stays_boolean`,
`irregular_comparison_table_is_sorted` (`inflect`/`convert`); adjective corpus import felicity-clean
(`--validate`). *Deferred:* the **superlative** ("the largest", gated on the definite "the" + maximality)
and **periphrastic** comparatives (the `more`/`most` words). The original design follows.

The genuine *degree-semantics* foundation: gradable adjectives and their
comparison — "X is **larger than** Y", "the **largest** gene". High-value for scientific prose ("higher
expression than", "the most dependent cell line"), and it has a real payoff: it **reuses the D52 measurement
machinery** rather than inventing a scale.

*The key grounding — degrees are `core:float`, ordered by the existing opaque relations.* The chain already
ships `stats:lt`/`le`/`gt`/`ge : core:float → core:float → Prop` (opaque orderings, witnessed downstream)
and measurement functionals (`mean_of : core:string → core:float`, …). So **no new `Degree` type**: a degree
is a `core:float`, comparison is the existing opaque `stats:gt`/`lt`. This is the right foundation for a
*scientific* KG, where comparatives are largely over measured quantities — "X has higher expression than Y"
is exactly `gt(expression_of(x), expression_of(y))`.

*Gradable adjective = a measure (recommended over an opaque comparison relation).* Each gradable adjective
`A` gets an opaque **measure** `deg_A : lexicon:Entity → core:float` (the degree-of-`A` function; e.g.
`deg_large`). Then:
- **Comparative** "X is larger than Y" → `stats:gt(deg_large(x), deg_large(y))`.
- **Positive** "X is large" → `stats:gt(deg_large(x), std_large)` for an opaque contextual standard
  `std_large : core:float` — **unifying** the positive with the comparative under one measure (the standard
  degree-semantics analysis: the positive is the measure vs. a threshold). *(For gradable adjectives only;
  relational adjectives keep the Boolean `is_A` — see the resolved gradability + positive decisions below.)*
- **Superlative** "the largest gene" → the unique `g : Gene` maximal in the comparison class:
  `∀g':Gene. g' ≠ g → gt(deg_large(g), deg_large(g'))` — **deferred**, gated on the definite **"the"**
  determiner (§8.3 follow-on) + maximality quantification over the noun's class.
The alternative — an **opaque comparison relation** `more(A, x, y)` with no degrees — was **rejected**: it
can't reach equatives / measure phrases / superlative-maximality, has no transitivity structure, and (the
decisive point) doesn't connect to the measurement layer that scientific comparatives live in.

*The categorial pieces (comparative; the in-scope core).*
- A **`than`-phrase** category (`cat_pp_than`, a complement marking the standard) + the function word
  `than : cat_pp_than / NP`, sem identity (supplies the standard entity `y`).
- The **comparative adjective** `larger : (S[dcl,adj]\NP) / cat_pp_than`, sem
  `λy. λx. stats:gt(deg_large(x), deg_large(y))`. So "larger than Y" → the adjectival predicate
  `λx. gt(deg_large(x), deg_large(y))`, which the **copula `is` lifts** (reusing 3a) → "X is larger than Y"
  → `gt(deg_large(x), deg_large(y))`.

*Comparison morphology (grounded; parallels the verb-form work).* Two strategies. **Synthetic**
`-er`/`-est` ("large→larger→largest"), regular with the same orthographics as the verb inflector
(`e`-final → `+r`/`+st`, consonant-`y → -ier`/`-iest`, monosyllabic-CVC doubling). A validation against a
~200-adjective comparison list confirmed the **rule reproduces ≈96%** of forms by itself; the
**irregular table is small and textbook-stable** — the suppletives `good→better/best`, `bad→worse/worst`,
`little→less/least`, `much`·`many→more/most`, `far→farther·further/…`, `old→older·elder/…`, plus
`shy→shyer/shyest` (keeps the `y`). It is **witnessed against the in-repo WordNet `adj.exc`** (which
attests `better`/`worse`/`shyer` and the orthographic regulars `bigger`/`cuter`/`happier`; the
`more`/`most`/`less`/`least` suppletives are separate headwords, not inflections, so they are the
hand-fixed textbook residual). The witness also **catches source typos** fail-closed (a listed form that is
neither the rule output nor attested — e.g. `clear→clear`, `lonely→lonlier` — is dropped, and the rule's
form is used). **Periphrastic** "more"/"most" for the rest ("more beautiful") — a *fuzzy, low-stakes*
heuristic (monosyllabic / `-y` / `-le` / `-er` / `-ow` → synthetic; else periphrastic; silent final `e`
must **not** be counted as a syllable), low-stakes because the synthetic/periphrastic choice is genuinely
variable in English (`politer` ~ `more polite`) and **fail-closed** (a wrong synthetic guess simply isn't
looked up; `more X` via the grammar still parses). The generator lives in `eigenius-wordnet::inflect`
(alongside the verb forms); the importer emits the synthetic `-er`/`-est` entries and the grammar admits
`more`/`most` + base for the periphrastic case.

*Kernel.* No new capability — `deg_A : Entity → core:float` and the `stats:gt` application are ordinary
opaque axioms over existing types; adds the `cat_pp_than` ctor + a `denote_cat` arm. Reuses the 3a copula
and the existing adjective `adj` category.

*Gradability detection (resolved — WordNet descriptive-vs-relational).* Gradability cannot come from
morphology (the `-er` rule will happily generate "primarier"), so it comes from **WordNet structure**:
**relational** adjectives carry a **pertainym pointer `\`** ("atomic"\→"atom", "presidential"\→"president")
and are **non-gradable**; **descriptive** adjectives (antonym/similar-clustered) lack it and are
**gradable**. So `gradable ⟺ no pertainym`. This covers all ~18k adjectives and is grounded in WordNet's own
classification; a curated comparison list (~200 adjectives, validated to come out gradable) is the
**validation set** + curated override. Requires a small **`wndb` extension** — capture the `\` pertainym
pointer (today `wndb` walks the pointer block but keeps only `@`/`@i`/frames). *Accepted, fail-closed:* a
minority of descriptive adjectives are non-gradable (dead, pregnant, unique, perfect) — flagged gradable →
mild "more pregnant" over-generation, never ill-typed; the curated list can override the worst.

*Positive unification (resolved — combo 1, enabled by the gradability flag).* Because gradability is now
reliably flagged, the **positive unifies with the comparative** for gradable adjectives: the gradable `A`
is the measure `deg_A`, and "X is large" is the derived `gt(deg_large(x), std_large)` (`std_large :
core:float`, an opaque contextual standard). **Non-gradable (relational) adjectives keep the Boolean**
`is_A : Entity → Prop` unchanged. (This refines `push_adj` for gradable adjectives only — the relational
ones, and the existing demo, are untouched.)

*Importer.* For each adjective, read the pertainym flag from `wndb`. **Gradable** → mint `deg_A : Entity →
core:float` + `std_A : core:float`, emit the positive as `gt(deg_A(x), std_A)` and the synthetic
comparative/superlative forms (§ morphology). **Relational** → keep the current Boolean `is_A` (no `deg_A`,
no comparatives). The comparison-form generator lives in `eigenius-wordnet::inflect`.

*Build order (when taken):* (1) `wndb` pertainym capture + a `gradable` flag on the adjective synset,
validated against the comparison list; (2) `cat_pp_than` + `than` + comparative adjective entries +
`deg_A`/`std_A` axioms + the unified positive for gradable adjectives + a demo gradable adjective ("large");
(3) comparison morphology in `inflect` (`-er`/`-est` + the suppletive table + periphrastic policy, already
grounded above) + importer emission; (4) superlative, once the definite "the" lands. *Verify:* "HeLa is
larger than BRCA1" → `gt(deg_large(hela), deg_large(brca1)) : Prop`, single parse; "X is large" →
`gt(deg_large(x), std_large)`; a relational adjective stays Boolean (`is_atomic`, no comparative); the
comparison list all flags gradable.

*Decisions (settled):* degrees = `core:float`, comparison = the opaque `stats:gt`/`lt` (reuse D52, not a new
scale); gradable adjective = a **measure** `deg_A` (not an opaque comparison relation); **gradability =
WordNet pertainym flag** (descriptive ⇒ gradable, relational ⇒ not), validated by the curated list;
**positive unified** (`gt(deg_A, std_A)`) for gradable adjectives, **Boolean `is_A`** for relational; copula
reused for the predicate. *Deferred:* superlative (gated on the definite "the"), equatives ("as large as" →
`ge`), measure phrases ("3cm long"), comparative *clauses* ("larger than Y is"), attributive comparatives
("a larger gene").

### 8.13 Slice 6-mod — nominal modification: compound nouns + PP adjuncts

**✅ Done.** The nominal/adjunct layer — four engine/lexical rules over opaque, institution-mapped
relations, no kernel change (Σ-refine + functor shapes + `logic:And` all pre-exist):

1. **Named-entity compound** — engine rule in [`parser.rs`]: a `cat_np` modifier + head `cat_n(C)` →
   `cat_n(Σx:C. ontology:compound(x, m))`, `m` the modifier entity. "a BRCA1 cell line affects HeLa".
2. **N-N kind compound** — engine rule: `cat_n(M)` modifier + head `cat_n(C)` →
   `cat_n(Σx:C. ontology:compound_kind(x, M))`, where the modifier `M` is the left noun's *kind* (its
   sem, a `Set` — CN-as-types). `compound_kind : Entity → Set → Prop`. "a gene cell line …", "mutator load".
3. **PP VP-adjunct** — closed-class prepositions in [`closed-class.esl`] (`((S[dcl,fin]\NP)\(S[dcl,fin]\NP))
   /NP`, sem `λx.λV.λs. logic:And(V(s), ontology:prep_*(s, x))`): "HeLa affects BRCA1 in HeLa" →
   `And(affects(brca1, hela), prep_in(hela, hela))`. (`in`/`for`/`with`/`on`/`from`.)
4. **PP noun-modifier** — a new `lexicon:cat_pp` category (⟦·⟧ = Entity → Prop) + post-nominal engine rule
   `[cat_n(C)] [cat_pp]` → `cat_n(Σx:C. pp(x))`; prepositions `cat_pp / NP`, sem `λy.λx. ontology:prep_*(x, y)`
   (`of` and also `in`/`for`/`with`/`on`/`from`). "a cell line of BRCA1 …". Because a preposition has BOTH a
   VP-adjunct (3) and a noun-modifier (4) entry, a PP after an object noun **attaches two ways** —
   **PP-attachment ambiguity carried in the forest** ("HeLa affects a cell line in HeLa" → 2 parses).

**Left-branching normal form**: a compound's HEAD (right) may not itself be a compound result
(`is_compound_refined`), so a 3+-noun chain has the single bracketing `[[A B] C]` (no spurious `[A [B C]]`);
an *attributively*-refined head is still allowed (a distinct structure, not spurious). `cat_pp` is kept
distinct from a bare adjective (`S[adj]\NP`) so a post-nominal adjective never spuriously refines, and from
the VP-adjunct preposition so (3) and (4) stay separate parses.

The modifier relations are **opaque** (institution-mapped, like `is_a` / the modals): the grammar
guarantees a felicitous typed tree exposing the parts; the precise relation is grounding-supplied (D62).
**Structural fix landed alongside:** common nouns are now number-**underspecified** (`cat_n(_, num_any)`) in
the lexicon — number is a surface/morphology property the lookup index sets per token (`with_noun_num`) — so
a plural surface reaching a noun via lemmatization no longer keeps a stale `sg`. Witnesses
(`closed_class_determiners`): `compound_noun_refines_the_head`, `n_n_kind_compound_refines_with_compound_kind`,
`pp_adjunct_adds_an_opaque_conjunct`, `pp_noun_modifier_refines_the_head`, `pp_attachment_is_ambiguous`,
`compound_chain_is_left_branching`. *Deferred:* the larger items below (domain-lexicon scale-out, anaphora).
The original design follows.

The **nominal/adjunct layer** — the single highest-leverage gap for real scientific prose. The WRN litmus (`experiments/publications/wrn-helicase/sentence-corpus.md`) shows it
bluntly: **compound nouns** ("WRN depletion", "mutator load", "helicase activity") block ~26 of 31
sentences and **PP adjuncts** ("in MSI", "of WRN dependency", "for the dependence") block ~19 — they are in
nearly every sentence, dwarfing the rest of the tail. The verb-argument spine (Slices 1–6) is necessary but
parses almost none of this corpus without them.

*The unifying idea — opaque modification, two attachment sites, Σ-reuse.* Both constructions are
**modifiers** that contribute an **opaque relation** (the modifier-head relation is notoriously vague —
"WRN depletion" = depletion *of* WRN, "baby oil" = oil *for* babies; the relation is grounding-supplied,
not compositional). So, faithful to the opaque-predicate philosophy (`is_a`, the modal operators), the
relation is a **kernel-uninterpreted axiom**; the grammar delivers a *felicitous typed tree exposing the
parts*, and the **D62 institution maps it onto the domain predicate's argument structure** (e.g. "WRN
depletion causes apoptosis in MSI" → `CausesApoptosis(WRN, MSI)` — "WRN" from the compound head's modifier,
"MSI" from the PP). The grammar's job is **felicity + part-exposure**, not getting the vague relation right.

Two attachment sites, both reusing existing machinery:

**(A) Nominal modification → Σ-refinement (reuses 3b).** A modifier refines the head noun, exactly as the
attributive adjective did ("primary cell line" = `Σx:CellLine. is_primary(x)`):
- **Compound noun** — a modifier noun/NP `M` + head `cat_n(C)` → `cat_n(Σx:C. compound(x, ⟦M⟧))`, an opaque
  `axiom lexicon:compound : Entity → Entity → Prop`. "WRN depletion" = `Σx:Depletion. compound(x, WRN)`.
  An engine rule on adjacent `[N|NP] [N]` (the relativizer/attributive pattern); the refined noun then
  rides the determiner-over-refined-noun `Fst` rule unchanged.
- **PP as post-nominal modifier** — head `cat_n(C)` + PP "of X" → `cat_n(Σx:C. of(x, X))`, a per-preposition
  opaque relation. "biomarker of WRN dependency" = `Σy:Biomarker. of(y, WRN_dependency)`.

**(B) Verbal adjunction → VP post-modifier.** A PP modifies a VP: "affects X **in MSI**" — `in MSI :
(S\NP)\(S\NP)`, sem `λV.λs. logic:And(V(s), in(s, MSI))` (the locative as an opaque conjunct; no event
variables — neo-Davidsonian event semantics is the heavier alternative, deferred). The preposition is the
PP-former + attachment: `in : ((S\NP)\(S\NP)) / NP` (VP-adjunct reading) and `(N\N)/NP`-style (nominal
reading) — **PP attachment is genuinely ambiguous** ("causes apoptosis in MSI": `in MSI` on the VP vs. on
"apoptosis"), so both readings are produced and **carried in the forest** (the institution / felicity
oracle selects — not resolved in the grammar).

*Opaque relations.* A small **closed set of prepositions** (`in`/`of`/`for`/`with`/`on`/`from`/`to`/…) each
an opaque `axiom : Entity → Entity → Prop` in a `prep` (or `lexicon`) layer — kernel-uninterpreted,
witnessed/mapped downstream, like the modal operators. Plus the one `compound` relation. (The `by`-passive
agent (§8.9) is the special, already-built case of a PP.)

*Ambiguity controls.* **Compound bracketing** ("DNA double-strand breaks" = `[DNA [double-strand breaks]]`
vs `[[DNA double-strand] breaks]`) → a **left-branching normal form** (as for coordination, §8.4). The
**MWE-vs-compound** competition ("cell line" the lexicalized term vs. `[cell + line]` productive compound)
is already carried as competing chart edges (§8.4) — the domain lexicon supplies fixed terms; the
productive rule covers the rest. **PP attachment** (N vs VP) is *not* normalized — it is real ambiguity the
forest carries.

*Kernel.* No new capability — the `compound` / preposition relations are opaque axioms over existing types
(as `is_a`/modals); the Σ-refine + `Fst` machinery is 3b's; the VP-adjunct `(S\NP)\(S\NP)` is an existing
functor shape. (`logic:And` for the locative conjunct already exists.)

*Importer.* Productive compounding + PP attachment are **engine rules**, not per-word emission — so the
importer needs no per-noun change. The **prepositions** are closed-class entries (like determiners/auxes);
the `compound`/preposition **relation axioms** go in a small committed layer. A **domain lexicon** (MSI,
WRN, gene names) is the separate prerequisite for the corpus to actually run (D62 §8.7.8).

*Build order (when taken):* (1) the `compound` + preposition opaque-relation axioms + the closed-class
preposition entries; (2) the **compound-noun engine rule** (`[N|NP] [N] → Σ-refined cat_n`, reuse 3b) +
left-branching NF — verify on `[Gene + N]` compounds; (3) **PP-as-N-modifier** (reuse 3b Σ-refine) and
**PP-as-VP-adjunct** (`(S\NP)\(S\NP)`, `And`-conjunct) — verify both attachment readings appear. *Target
witnesses (with a small domain lexicon):* "WRN depletion causes apoptosis in MSI" → a felicitous tree whose
parts ("WRN", "depletion", "apoptosis", "MSI") are exposed; "MSI is a strong biomarker of WRN dependency" →
the `of`-PP refines "biomarker".

*Decisions:* opaque `compound` + per-preposition relations (kernel-uninterpreted, institution-mapped, like
`is_a`); nominal modification reuses 3b's Σ-refine; PP-VP adjunct = opaque `And`-conjunct (no event vars);
compound bracketing left-branching; PP attachment carried (not normalized); the precise relation is
grounding-supplied (D62), the grammar guarantees only felicity + part-exposure. *Deferred:* neo-Davidsonian
event semantics; compound-internal sense disambiguation; measure/degree PPs ("3cm long"), "relative to",
fronted/focus PPs ("Among …, only WRN …"), multi-PP scope ambiguity beyond the carried forest; the
**domain lexicon** (its own track) that makes the corpus runnable.

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
