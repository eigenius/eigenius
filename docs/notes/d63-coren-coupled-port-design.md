# D63 — Reference-as-map, coupled port where it pays (core-en → Eigenius DCG)

**Status:** design, pre-implementation. A working design note. Reviewable before any code.
Supersedes the reactive, rule-by-rule grind (each fix +0–2 units,
[d62-controlled-language-style-guide.md](d62-controlled-language-style-guide.md) ~line 300) with a
principled use of the OpenCCG `core-en` reference grammar as a *map* of English's combinatorial
syntax. Sibling to the packed-forest blueprint
([d63-packed-forest-parsing-blueprint.md](d63-packed-forest-parsing-blueprint.md)) and the levers plan
([d63-cnl-parse-levers-plan.md](d63-cnl-parse-levers-plan.md)); the parser-substrate and semantic-
universe decisions it builds on are in
[docs/design/d63-dcg-engine-english-grammar.md](../design/d63-dcg-engine-english-grammar.md).

## 1. Thesis

We already have a comprehensive reference grammar of broad English —
`references/openccg/grammars/core-en/` — and we are *reactively re-discovering its rules by hand*, one
construction at a time. That is backwards. The reference is a **map**: it tells us, ahead of time,
which categories exist, how their morphosyntactic features interlock, which multimodal slash modes
gate which combinators, and what the normal-form constraints are. We should read the map to know
*where to build*, instead of surveying the terrain rule by rule.

But the reference gives us only **one half** — the *syntactic* half (categories, features, slash
modes, the combinator set, NF). Its **semantics is HLDS** — hybrid-logic dependency nominals encoded
as `<lf>`/`<satop>`/`<diamond>` trees (e.g. [`v.xsl`](../../references/openccg/grammars/core-en/v.xsl)
lines 298–325, [`np.xsl`](../../references/openccg/grammars/core-en/np.xsl) lines 59–69). That half is
**not reusable and not wanted**. Eigenius's value proposition is the *other* half: a **dependent-typed
denotation** `⟦·⟧ : Cat → EigenTT type` (common-nouns-as-types; selectional restriction is a *type
error*, not a feature clash; a felicity oracle admits/rejects), realized by
[`denote_cat`](../../kernel/src/dcg/category.rs) at `category.rs:35`. No reference grammar has this.
It is irreducibly ours.

So the strategy has two moves:

1. **Reference-as-map.** Port the syntactic half — categories + features + **multimodal slash modes**
   + NF + the combinator set — as a **coupled whole**, reading `core-en` to know the target shape.
2. **Coupled port where it pays.** Hand-build the typed `⟦·⟧` + kernel-checked sem *only* per
   category, where the value-add lives.

**Why "coupled whole or not at all" is load-bearing — the measured lesson.** The combinatory-core
spike ([`apply_core`](../../kernel/src/dcg/parser.rs) at `parser.rs:667`, gated by
[`combinatory_core`](../../kernel/src/dcg/lookup.rs) at `lookup.rs:224`, toggled via
`EIGENIUS_COMBINATORY_CORE` in
[`db_backed_encoding.rs:132`](../../crates/eigenius-wordnet/tests/db_backed_encoding.rs)) implemented
the full crossed/backward/harmonic composition set behind a flag and was measured **INERT**:
identical parse counts core-on vs core-off on **both** the CNL and the original WRN pages
([d62-controlled-language-style-guide.md](d62-controlled-language-style-guide.md) ~line 285). The
reason is precisely the coupling failure: it ported **one layer** (the combinators) onto our
**plain-slash, type-indexed application categories** without the coupled feature-shaped categories +
slash modes + NF the combinators assume. The combinators had nothing to compose. **You leverage the
reference as a coupled whole or not at all.** That is this note's thesis and the shape of every
decision below.

**Licensing (hard constraint).** `core-en` is **LGPL — read & reimplement, do not ship / do not
vendor / do not link**
([docs/design/d63-dcg-engine-english-grammar.md:1575](../design/d63-dcg-engine-english-grammar.md),
also lines 152, 700). Everything here is a **reimplementation** in our own notation and kernel. The
reference is consulted, never incorporated. Baldridge's dissertation
(`references/publications/Baldridge_dissertation.pdf`, verified present) and Eisner's normal-form
paper (`references/publications/Eisner-Efficeint Normal Form Parsing.pdf`) are the *reference* anchors
for the mode lattice and NF.

## 2. What the reference actually is (grounding)

`core-en` is **XSLT-generated**. The relevant sources (verified):

| File | Size | Content |
|---|---|---|
| [`cats.xsl`](../../references/openccg/grammars/core-en/cats.xsl) | 30 KB | base category schemas (`n`, `np`, `s`, `pp`, `num`, VP, predicatives) + feature declarations (`cats.xsl:20–31`) |
| [`types.xml`](../../references/openccg/grammars/core-en/types.xml) | 2.8 KB | the **feature hierarchy** (form / person / number lattices; ontological sorts) |
| [`v.xsl`](../../references/openccg/grammars/core-en/v.xsl) | 24 KB | verb families (IV/TV/DTV, S-comp, control, copula, existentials) |
| [`np.xsl`](../../references/openccg/grammars/core-en/np.xsl) | 7 KB | nouns, names, pronouns, quantified/wh NPs, **type-raised** NPs |
| [`det.xsl`](../../references/openccg/grammars/core-en/det.xsl) | 7 KB | determiners, quant-dets, wh-dets, possessives |
| [`pp.xsl`](../../references/openccg/grammars/core-en/pp.xsl) | 9 KB | prepositions (nominal-postmod, predicative, appositive, particle) |
| [`adj.xsl`](../../references/openccg/grammars/core-en/adj.xsl) / [`adv.xsl`](../../references/openccg/grammars/core-en/adv.xsl) | 2.7 / 4.4 KB | adjectives (attrib + predicative), adverbs (initial/forward/backward) |
| [`conj.xsl`](../../references/openccg/grammars/core-en/conj.xsl) | 15 KB | coordination (`X conj X`), subordinators, list completion |
| [`auxv.xsl`](../../references/openccg/grammars/core-en/auxv.xsl) | 5 KB | modals, progressive, negation, do-support |
| [`unary-rules.xsl`](../../references/openccg/grammars/core-en/unary-rules.xsl) | 6.7 KB | type-changing rules (reduced relative, topicalization, bare-NP, cardinal, purpose) |

The **combinator set is inherited** (not in `core-en/`); the canonical rule inventory is
[`mini-english/rules.xml`](../../references/openccg/grammars/mini-english/rules.xml) lines 6–17:
application (fwd/bwd), composition (fwd/bwd × harmonic/crossed), type-raising (fwd/bwd), substitution
(fwd/bwd × harmonic/crossed). The combinators are **universal**; the **slash modes gate which fire**.

**The slash-mode inventory `core-en` uses (verified counts across `*.xsl`):**

| Surface mode | count | Where it appears | Operational reading (observed) |
|---|---|---|---|
| `mode="&lt;"` (`<`) | 43 | verb subject `\np` (`v.xsl:24`), prep object `/np` (`pp.xsl:23,25`), DTV inner (`v.xsl:150`) | directional application on **argument** slots |
| `mode="&gt;"` (`>`) | 24 | verb object `/np` (`v.xsl:35`) | directional application on the forward **object** slot |
| `mode="^"` (`^`) | 20 | adjective `n/n` (`adj.xsl:23`), determiner `np/n` (`det.xsl:20`, the `fslash-n` slash), adverb `vp/vp` (`adv.xsl:41`), aux VP slot (`auxv.xsl:32`), wh `s/(s\np)` (`np.xsl:74`) | the **modifier / composition** modality |
| `mode="*"` (`*`) | 48 | coordination args (`conj.xsl:22,26`), inverted copula subject (`v.xsl:499`), phrasal particle (`v.xsl:125`), comma absorption (`adv.xsl:33`, `conj.xsl:295`), subordinators (`conj.xsl:128`) | **application-only** (never compose) |
| `varmodality="M" ability="active"` | 8 / 8 | type-raised NPs (`np.xsl:35–37,48–54`) | **variable** modality — the raised functor composes in whatever mode the consumed slot supplies |
| `ability="inert"` | 4 | predicate adjective `\np` (`cats.xsl:1018`), to-infinitive subject `\np` (`v.xsl:341`) | block composition on **this specific** slash |

The takeaway for the port: modes are how the reference **lexically restricts** the otherwise-universal
combinators (coordination absorbs by application only; modifiers compose; type-raised functors are
mode-polymorphic). This is exactly the layer the inert spike was missing.

**The feature hierarchy** ([`types.xml`](../../references/openccg/grammars/core-en/types.xml)) is
richer than ours in three concrete ways:
- **`form`** (lines 12–23) folds mood *and* verb-form into one attribute: `dcl`/`fronted` (`dcl-base`),
  `q`/`wh` (`q-base`), `base`, `emb`, `inf`, `adj`, `ng`.
- **number is a lattice** (lines 33–38): `sg` and `mass` under `sg-or-mass`; `pl` and `mass` under
  `pl-or-mass` — `mass` sits under *both*, so bare-mass licensing is a subsumption, not a wildcard.
- **`pers`** (1st/2nd/3rd, lines 26–30) and **`case`** (nom/acc/gen, used throughout `cats.xsl`'s np
  variants, e.g. `np.2.X.nom` `cats.xsl:107`, `np.3.Y.acc` `cats.xsl:163`) are first-class. Eigenius
  has **neither**.

The ontological sorts (`types.xml:40–67`, `entity`/`abstraction`/`situation`/…) are HLDS type
decoration on the LF nominals — **not** reused; our type lattice is the committed ontology
(`is_subclass_of`).

## 3. Where Eigenius stands today (grounding)

**The category algebra** — [`data lexicon:Cat`](../../ontologies/lexicon/lexicon-ontology.esl) at
`lexicon-ontology.esl:132–209`:

```
cat_s  : Mood -> Fin -> Cat                    -- S[mood,fin]
cat_n  : Set -> Num -> Cat                     -- N[num](T)      (CN-as-type)
cat_np : Set -> Num -> Cat                     -- NP[num](T)
fwd : Cat -> Cat -> Cat                        -- A/B   ** PLAIN SLASH, NO MODE **   (line 143)
bwd : Cat -> Cat -> Cat                        -- A\B   ** PLAIN SLASH, NO MODE **   (line 146)
cat_forall : Num -> (Set -> Cat) -> Cat        -- dependent determiner over noun type
cat_fin_forall : (Fin -> Cat) -> Cat           -- feature-polymorphic functor
cat_num_forall : (Num -> Cat) -> Cat
cat_group : Set -> Conn -> Num -> Cat          -- coordinated NP group (List C)
cat_q  : Set -> Cat                            -- wh-question (⟦·⟧ = T → Prop)
cat_kind : Cat                                 -- bare-plural kind subject
cat_cp : Cat                                   -- embedded complement clause
cat_pp_than : Cat                              -- comparative than-phrase
cat_pp : Cat                                   -- noun-postmodifying PP
```

with features `Mood{dcl,q,imp}`, `Num{sg,pl,num_any,mass}`, `Fin{fin,bse,inf,ger,pss,pass,adj,fin_any}`,
`Conn{conn_and,conn_or,conn_but_not}` (`lexicon-ontology.esl:76–126`).

**The two decisive facts about our slashes:** `fwd`/`bwd` are **plain** (no mode argument), and
`Num`/`Fin`/`case`/`pers` are *flat with a wildcard* rather than a subsumption lattice. Both are the
coupling gaps the inert spike ran into.

**The kernel machinery that must become mode-aware:**
- [`denote_cat`](../../kernel/src/dcg/category.rs) `category.rs:35` — the `fwd`/`bwd` arm matches
  `[a, b]` (`category.rs:72`); it must accept `[mode, a, b]` and *erase* the mode (like `Fin`/`Num`).
- [`unify_cat`](../../kernel/src/dcg/category.rs)/`unify_into` `category.rs:172,177` — the functor
  arm (`category.rs:214–221`) does covariant-result/contravariant-arg structural subsumption; it must
  match the new arity and (optionally) unify the mode.
- [`combinable`](../../kernel/src/dcg/parser.rs) `parser.rs:280` (the **sem-blind decision**),
  [`build`](../../kernel/src/dcg/parser.rs) `parser.rs:446`, [`build_refine`] `parser.rs:546`,
  [`apply`] `parser.rs:212`, [`apply_core`] `parser.rs:667` — the combinators; the ENF guard is at
  `parser.rs:314–320,344`.
- [`type_raise`](../../kernel/src/dcg/category.rs) `category.rs:889` — builds `S/(S\NP_X)` with plain
  slashes; must build **variable-mode** slashes.
- The `Combinator` provenance enum + Eisner-NF (`parser.rs:36–65`).

**The hand-edited surfaces** (what the reactive grind actually costs):
- [`closed-class.esl`](../../ontologies/lexicon/closed-class.esl) — 1499 lines, **110 `fwd`/`bwd`
  uses** (the dense manual surface; determiners, auxiliaries, prepositions, relativizers, negation).
- Open-class = **~6 templates** in [`FrameKind`](../../crates/eigenius-wordnet/src/convert.rs)
  `convert.rs:145–200` (`Intransitive`/`Transitive`/`Ditransitive`/`Clausal` + the noun/adjective
  emitters) that regenerate the entire imported lexicon (325k WordNet + 7.6M UMLS). Every verb slot is
  `cat_np(Entity, num_any)` (`convert.rs:187–188`) — application categories, plain slashes,
  index-independent. **Reseed/re-import is NOT a blocker** (pre-production posture; user confirmed).

## 4. Decision — the target category algebra

**D-1. Add a `Mode` inductive and thread it onto `fwd`/`bwd`.** This is the single structural change
that makes the combinator core non-inert. Modes are *syntactic routing only* — **erased by `⟦·⟧`**,
exactly like `Fin`/`Num` (`lexicon-ontology.esl:52–58`).

```
// New: the multimodal slash modality (Baldridge). Erased by ⟦·⟧; gates which
// combinators may fire on a slash. A reimplementation of core-en's <,>,^,* modes.
data lexicon:Mode {
    mode_app,    // application only          — core-en `*` (coordination, particles, punctuation)
    mode_harm,   // application + harmonic composition (order-preserving) — core-en `^`, `<`, `>`
    mode_cross,  // application + crossed composition (permuting)         — pied-piping, heavy shift
    mode_perm,   // application + all composition (the permissive top)
    mode_var,    // variable modality — core-en `varmodality="M"`: unifies to the consumed slot's mode
}

// Slashes gain a leading Mode. ⟦fwd(m,A,B)⟧ = ⟦bwd(m,A,B)⟧ = ⟦B⟧ → ⟦A⟧ (m erased).
fwd : lexicon:Mode -> lexicon:Cat -> lexicon:Cat -> lexicon:Cat
bwd : lexicon:Mode -> lexicon:Cat -> lexicon:Cat -> lexicon:Cat
```

Rationale for the 5-point set: it is the Baldridge lattice `{· , ⋄ , × , ⋆}` plus a variable mode for
type-raising. It is a **superset** of the four surface modes `core-en` actually uses (`<`,`>`,`^`,`*`
map into `mode_harm`/`mode_app`; see the open decision in §8 about the exact `<`/`>` split), so it can
express everything the reference does and leaves room for `mode_cross` where our typed rules need
permuting composition (pied-piping) without opening it globally.

**D-2. Enrich the feature hierarchy toward `types.xml`, but keep Mood separate.** Two additions and
one refinement:
- **Add `case`** `Case{nom, acc, gen, case_any}`. Subject slots take `nom`, object slots `acc`,
  possessives `gen` — mirroring `cats.xsl`'s `np.nom`/`np.acc`/`np.gen`. This removes spurious
  subject↔object readings *structurally* (a fronted object can't re-fill a subject slot).
- **Add `pers`** `Pers{p1, p2, p3, pers_any}` for agreement (copula, do-support).
- **Refine `Num` into a lattice** matching `types.xml:33–38` (`mass ≤ sg-or-mass` and
  `mass ≤ pl-or-mass`), so bare-mass licensing is subsumption, not the current flat `mass`-meets-
  nothing rule (`lexicon-ontology.esl:89–91`). This is the principled form of the D62 "5 curated
  mass abbreviations" hack.

**Keep Mood a separate feature** (`lexicon-ontology.esl:73–80`) rather than folding it into a `form`
attribute as `core-en` does. Our split is *better-motivated than the reference*: Mood is the **only**
feature that alters `⟦·⟧` (`denote_cat` at `category.rs:42` → `denote_mood` at `category.rs:120`),
whereas `core-en` conflates it with the syntactic verb-form under one `form` feature. This is the one
place we deliberately deviate from the map — surfaced here per the plan-deviation discipline.

**Denotation invariance (the safety property).** `⟦·⟧` must be unchanged by D-1/D-2: the new `mode`,
`case`, `pers` positions are all erased, so `denote_cat` gains arms that discard them and every
existing `⟦cat⟧` normal form is byte-identical. This is the Phase-0 differential-oracle obligation
(§7).

## 5. Mapping table — core-en construct → Eigenius typed category

Legend: **Syntax** = does the category/mode/feature shape port mechanically from the map?
**⟦·⟧/sem** = is the typed denotation *have* (implemented), *hand* (must be hand-built — the value-add
half), or *importer* (emitted by the WordNet/UMLS templates).

| core-en construct | core-en category (dir + mode) | Eigenius typed category | Syntax | ⟦·⟧ / sem |
|---|---|---|---|---|
| Noun (`np.xsl:128`) | `n[X]` | `cat_n(C, num)` | mechanical | *have* — `⟦·⟧=Set`, EigonClass |
| Name (`np.xsl:165`) | `np[3rd]` | `cat_np(C, num)` | mechanical | *have* — ResourceRef |
| Determiner (`det.xsl:30`) | `np[X]/^n[X]` | `cat_forall(num, λT. fwd(m_harm, S/(S\NP_T)))` | mechanical (add `^`) | *hand* — `λN.λV.∃/∀` (have) |
| Quant-det TR (`det.xsl:149`) | via `qnp` `s/(s\np)`, varmodality M | `cat_forall` + `type_raise` (var mode) | mechanical | *have* |
| Intransitive V (`v.xsl:21`) | `s[dcl]\<np[nom]` | `bwd(m_harm, cat_s(dcl,fin), cat_np(E,n))` | mechanical | *importer* — `E→Prop` |
| Transitive V (`v.xsl:30`) | `(s\<np)/>np[acc]` | `fwd(m_harm, bwd(...), cat_np(E,acc))` | mechanical | *importer* — `E→E→Prop` |
| Ditransitive V (`v.xsl:146`) | `((s\np)/np)/<np` | nested `fwd`/`bwd` | mechanical | *importer* |
| Clausal V (`v.xsl:241`) | `(s\np)/<s[emb]` | `fwd(m_app, bwd(...), cat_cp)` | mechanical | *hand* — report axiom (have) |
| Subject control (`v.xsl:262`) | `(s\np)/>(s[inf]\np)` | `fwd(bwd(...), bwd(cat_s(dcl,inf), NP))` | mechanical | *hand* — **new** (control sem) |
| Copula predicative (`v.xsl:484`) | `(s[dcl]\<np)/>pred.adj` | copula entry | mechanical | *hand* — `be`/Prop (closed-class) |
| Attributive adj (`adj.xsl:20`) | `n[X]/^n[X]` | `bwd(S[adj]\NP)` + **refine rule** → `cat_n(Σx:C. adj(x))` | mode ports; **shape differs** (we use predicative + Σ-refine) | *have* — Σ-refine (`build_refine` `parser.rs:556`) |
| Predicative adj (`adj.xsl:67`) | `s[adj]\<np`, `ability=inert` | `bwd(m_app, cat_s(dcl,adj), NP)` | mechanical (inert→`m_app`) | *have* |
| Adverb forward (`adv.xsl:38`) | `(s\np)/^(s\np)` | `fwd(m_harm, vp, vp)` (`adverb_modifier_cats` `category.rs:369`) | mechanical | *have* — identity/HasProp |
| Adverb initial (`adv.xsl:20`) | `s[fronted]/^s[dcl]` | `sentence_modifier_cats` S/S (`category.rs:411`) | mechanical | *have* |
| Aux / Modal (`auxv.xsl:27`) | `(s[dcl]\<np)/^(s[base]\np)` | aux entry (closed-class) | mechanical | *hand* — Body |
| Do-support inverted (`auxv.xsl:143`) | `s[q]/^s[base]` | do-support entry | mechanical | *have* (Slice 5c) |
| Prep n-postmod (`pp.xsl:20`) | `(n\<n)/<np[acc]` | `cat_pp / cat_np(E)` + post-nominal refine → `Σx:C. pp(x)` | mode ports; **shape differs** (distinct `cat_pp`) | *have* — `RefineKind::PpMod` (`parser.rs:606`) |
| Prep predicative (`pp.xsl:40`) | `(s[adj]\np)/np` | not distinct yet | port | *hand* — new |
| Particle (`pp.xsl:103`) | `prt` atom, `mode=*` | **no `prt` atom yet** | needs atom + `m_app` | *NoSem* |
| Coordination NP (`conj.xsl:30`) | `np_conj\*np/*np` via `bt` | `cat_group` + `coordinate_np` (`category.rs:601`) | **different scheme** (typed List, not conj-cat) | *have* — List C + distribute |
| Coordination S (`conj.xsl:95`) | `(s\*s)/*s` | `coordinate_sem` S conj S (`category.rs:506`) | mode ports (`*`→`m_app`) | *have* — pointwise op |
| Subordinator (`conj.xsl:124`) | `(s/*s)/*s` (comma) | deferred (factive signature) | partial | *hand* — deferred |
| Wh-NP subj (`np.xsl:71`) | `s[wh]/^(s\<np)` | `cat_q(T)` + rules | partial | *have* (Slice 5) |
| Reduced relative (`unary-rules.xsl:23`) | `n\*n ← s[dcl]/np` (unary `rrel`) | none | needs unary type-change | *hand* — new |
| Bare NP (`unary-rules.xsl:81`) | `np ← n[pl-or-mass]` (unary `bnp`) | `kind_subject` (`category.rs:862`) / bare-plural shift | present (unary) | *have* |
| Cardinal prenominal (`unary-rules.xsl:121`) | `n/^n ← num` (unary `card`) | number path (D52) | partial | *hand* |
| Topicalization (`unary-rules.xsl:42`) | `s[fronted]/(s/np)` (unary `tpc`) | none | needs unary | *hand* — new |
| Restrictive relative | (via composition + `rrel`) | `relativize` `category.rs:934` → `cat_n(Σx:C. body(x))` | gap-threading ports to T+B; **Σ-refine ours** | *have* (typed half) |

**Reading the table.** The **Syntax** column is overwhelmingly "mechanical" — that is the reference-as-
map payoff: the categories, directions, and modes transfer almost verbatim. The **⟦·⟧/sem** column is
overwhelmingly *hand* or *have* — that is the irreducible half. Three rows carry a structural nuance:
attributive adjectives, noun-postmodifying PPs, and restrictive relatives all use a **different shape**
in Eigenius (a **Σ-refined common noun**, CN-as-types) than `core-en`'s `n/n` / `n\n` modifier
categories, because our noun *is* a type. The reference maps *where the modifier attaches*; the
**typed refinement is ours** and does not come from the map. Coordination is a second nuance: it is
special-cased in **both** grammars (a `X conj X` schema in `core-en`, a typed `cat_group`/List in
ours) — so composition does **not** supersede it (see §6).

## 6. Kernel changes — making the engine mode-aware

The center of gravity is **spurious-ambiguity control**: composition (`B`) and type-raising (`T`)
make the *same meaning* derivable many ways. `core-en` controls this with **two coupled mechanisms** —
lexicalized **slash modes** (which slashes may compose at all) and a global **normal form** (which of
the composable derivations survives). The inert spike had the combinators but neither control, so it
either produced nothing new (shapes didn't line up) or would have flooded the chart. The port lands
both.

**6.1 `denote_cat` — erase the mode.** The `fwd`/`bwd` arm at `category.rs:72` changes from
`("fwd"|"bwd", [a, b])` to `[_mode, a, b]`, discarding `_mode` (parallel to the `_num`/`_fin` erasure
already at `category.rs:43,79,99`). `⟦·⟧` is unchanged. Add `Case`/`Pers` erasure to the `cat_np`/
`cat_n`/`cat_s` arms the same way.

**6.2 `unify_cat` — mode + feature unification.** The functor arm (`category.rs:214–221`) already does
covariant-result / contravariant-arg structural subsumption. It gains a **mode unification** step,
built exactly like the existing feature unifier [`unify_feat`](../../kernel/src/dcg/category.rs)
`category.rs:322` (variable binds to concrete; `mode_var` is the binding case, occurs-consistent; two
concretes fall back to a lattice meet). `case`/`pers` unify with the same `unify_feat` machinery
(`case_any`/`pers_any` are the `*_any` tops). **This is why D-1/D-2 are cheap**: the binding-aware
unifier already exists and is tested (`category.rs:1120`); modes and case/pers are new *instances* of
the same pattern, not new machinery.

**6.3 `combinable` — the mode-gated composition decision.** The sem-blind decision
(`parser.rs:280`) already gates forward composition on ENF provenance (`parser.rs:344`,
`!left_is_fwd_comp`). It gains a **mode gate**: forward composition (`parser.rs:343–358`) fires only
if the primary functor's slash mode `∈ {mode_harm, mode_perm, mode_var-bound-to-either}`. Application
(`parser.rs:319–342`) fires for **any** mode (all modes ≥ `mode_app`). Concretely:
- `mode_app` (coordination, particles, punctuation) → application only. Coordination and comma
  absorption stop being *able* to compose — the spurious `(S\S) ∘ (S\S)` chains a naive `<B` would
  spawn are excluded **structurally**, before ENF ever runs.
- `mode_harm` (verb args, modifiers, aux) → application + harmonic `B`. This is what licenses the
  object-wh forward composition already in place (`does HeLa` ∘ `affect` → `S[q]/NP`,
  `d63-dcg-engine-english-grammar.md:783`).
- `mode_var` on a type-raised functor → binds to the consumed slot's mode, so a raised NP composes
  exactly where the verb's slash permits and nowhere else.

**6.4 `apply_core` — fold the spike into the mode-gated core (retire the flag).** `apply_core`
(`parser.rs:667`) currently emits crossed/backward composition **unconditionally** (gated only by
provenance) and is off by default because it is inert *and* unsafe without modes. Post-port it merges
into `combinable`/`build`: `>Bx`/`<Bx` fire only when the primary slash mode permits crossing
(`mode_cross`/`mode_perm`), `<B` only for `mode_harm`+. The `combinatory_core` flag and its router
special-case (`lookup.rs:1017,1086,1910`) are **removed** — this is the blueprint's deferred item 3g.1
([d63-packed-forest-parsing-blueprint.md](d63-packed-forest-parsing-blueprint.md) "Deferred"), now
unblocked because modes make composition safe to leave on.

**6.5 `type_raise` — variable-mode slashes.** `type_raise` (`category.rs:889`) builds
`fwd(S, bwd(S, NP))` with plain slashes; it builds `fwd(mode_var, S, bwd(mode_var, S, NP))` so the
raised functor is mode-polymorphic (the reference's `varmodality="M"`, `np.xsl:35`). ENF still tags
the output `TypeRaised` so it may only *compose*, never forward-*apply* (`parser.rs:318`), preserving
single-parse declaratives.

**6.6 Eisner-NF — refined, not replaced.** With modes carrying most of the lexical restriction, the
residual job of ENF shrinks to: *within the set of composable (mode-permitting) slashes, admit one
derivation per equivalence class.* The existing provenance guard (a `>B` output may not be the primary
functor of a subsequent `>`/`>B`, `parser.rs:314–320`) stays and generalizes to the crossed/backward
variants (`CrossedComp`/`BackwardComp` already in the enum, `parser.rs:44–48`). The division of labor
— **modes = which slashes compose (lexical); ENF = which composition survives (derivational)** — is
the design's answer to spurious ambiguity and is the thing to get right (open decision §8).

**6.7 Packing interaction (must stay sound).** The packed-forest signature is `(cat_shape, ENF-prov)`
([d63-packed-forest-parsing-blueprint.md](d63-packed-forest-parsing-blueprint.md) §4). The mode
argument is a **syntactic** part of the category, so it *is* part of `cat_shape` — packing stays sound
because modes, like everything in the category, are consulted only by the sem-blind
`combinable`/`unify_cat`, never by a sem. `cat_has_selectional_slot` (`category.rs:283`, the Option-A
index-independence guard) is unaffected: it inspects `cat_np`/`cat_n` *type indices*, which modes
don't touch. **Confirm** at implementation that the mode never leaks into the felicity gate (it must
not — `⟦·⟧` erases it).

## 7. Port sequencing — mechanical first, typed-sem where it pays

Each phase is validated by the **differential-oracle + felicity-gate** pattern already in
[`closed_class_determiners.rs`](../../kernel/tests/closed_class_determiners.rs) (108 tests): a
**baseline construction parses exactly once** and each new construction **parses and type-checks to
`Prop`** (e.g. `closed_class_determiners.rs:316` "baseline copular adjective parses once";
`:327` "baseline transitive clause parses once"; `:478–487` pied-piping parses *and* `check_infer`
type-checks). The oracle is `parse` (the forest) and `parse_open` ((closed, open)) forest-equality,
reused exactly as the packed-vs-unpacked oracle
(`packed_forest_equals_unpacked_on_core_grammar`) does.

**Phase 0 — the mode/feature carrier (purely mechanical, behaviour-preserving).** Add `Mode`/`Case`/
`Pers`; thread `Mode` onto `fwd`/`bwd`; `denote_cat` erases; `unify_cat` treats every mode
permissively (no gating yet). Reseed the importer emitting `mode_harm` verb slots (still permissive).
**Oracle: the full forest is byte-identical to pre-mode** on the battery + first-7 CNL sems (the
`⟦·⟧`-invariance property of §4). This phase touches everything and must change *nothing* observable —
it is the coupling scaffold.

**Phase 1 — turn on mode gating (mechanical, spurious-ambiguity drop).** `combinable`/`apply_core`
enforce the mode gate (§6.3–6.4); retire the `combinatory_core` flag. **Oracle: battery 107 green,
declaratives still single-parse, and the spurious composition chains a naive core would add are
absent** (the win the inert spike could never show, now visible because the coupling is complete).

**Phase 2 — feature enrichment (mechanical, reseed).** Emit `nom` subjects / `acc` objects and the
`Num` lattice from `FrameKind::cat` (`convert.rs:183`). Reseed (325k + 7.6M; ~12 min per bootstrap-
drift, per the reseed memory). **Oracle: subject↔object spurious readings drop; bare-mass licensing
now subsumption-driven** (retires the 5-abbreviation hack).

**Phase 3 — retire the bespoke gap-threading in favor of generic T+B (mechanical *decision*, kept
*sem*).** The token-keyed special CKY rules in [`lookup.rs`](../../kernel/src/dcg/lookup.rs) that
thread relative-clause gaps by hand (`relativize` call sites `lookup.rs:2068`, appositive `:2108`)
move to **generic type-raise + mode-gated composition** producing the object-gap `S/NP`; the typed
**build** side (the Σ-refine) is unchanged. This is exactly the existing `combinable`(decision) /
`build`(sem) split (`parser.rs:280`/`446`) applied to relatives. **Oracle: the restrictive/appositive
relative forests are identical** to the hand-threaded ones.

**Phase 4 — new coverage the map gives cheaply (syntax mechanical, `⟦·⟧` hand-built).** Each is a
category port + one hand-built sem: **subject/object control** (`v.xsl:262`), **reduced relatives**
(`unary-rules.xsl:23`), **topicalization** (`unary-rules.xsl:42`), **particles/phrasal verbs**
(`pp.xsl:103` — needs the `prt` atom), **predicative PPs** (`pp.xsl:40`). Prioritize by corpus
frequency (the WRN/CNL residuals). **Oracle: each new construction parses + type-checks; the battery
does not regress.**

**Grading discipline.** A ported category is *Declared* when authored, *Derived* once it clears the
oracle + felicity gate on witnessed sentences, *Verified* only under human/battery sign-off — never
asserted parsed without the forest as witness.

## 8. Keep vs. remove — the reference-as-map boundary is decision-side, not build-side

The sharp finding of this analysis: **composition + modes supersede the syntactic *decision*
(gap-threading, mode-gating, agreement) but never the typed *build* (the sem).** So the retirement
list is about *control flow*, and the keep list is about *typed semantics*.

**Retire / fold in:**
- The **`combinatory_core` flag + `apply_core` as a separate path** (`parser.rs:667`,
  `lookup.rs:224,394,1017,1086,1910`) → folded into mode-gated `combinable`/`build` (§6.4).
- The **bespoke relative-clause gap-threading control flow** in `lookup.rs` (the manual
  `type_raise`→body assembly) → generic T+B (§7 Phase 3). *Keep* the `relativize` **Σ-refine build**
  (`category.rs:934`) as the `build`-side recipe; it is CN-as-types and irreducible.
- The **appositive special rule** `relativize_appos` (`category.rs:1022`) → in `core-en` the
  appositive is a **lexical** relativizer category (`RelPro-Appos`, an `s\s` `Trib`, `pp.xsl:50`
  appositive family), not a rule. Move the *decision* to a lexical entry + generic application;
  **keep** the conjoining sem (`λP. And(P(r), body(r))`) as that entry's `⟦·⟧`.

**Keep (irreducibly typed / special in both grammars):**
- **`cat_group` + `coordinate_np`/`coordinate_sem`/`distribute`/`distribute_object`/`reciprocate`**
  (`category.rs:506,601,732,784,816`). Coordination is special-cased in **`core-en` too** (the
  `X conj X` schema with `np_conj`/`s_conj` intermediate categories, `conj.xsl:30–103`) — it does
  **not** reduce to the combinator core in either grammar. Ours is the *better* typed treatment
  (`List C`, typed distribution). These also **read the sem** (`group_members`, `category.rs:565`),
  which is Harper's packing pitfall — they are the sem-reading carve-out `apply_group`
  (`parser.rs:621`) by construction and cannot become sem-blind combinators. **Keep as-is.**
- **`pied_pipe`** (`category.rs:976`). Genuinely ternary, forms no pile; the blueprint already
  decided against a combinator for it (`d63-packed-forest-parsing-blueprint.md` "DEVIATION"). `core-en`
  does pied-piping via crossed composition (`mode_cross`), so a *future* option is to route it through
  the crossed combinator — but the **typed Σ-restrictor is ours** regardless. **Keep; revisit under
  `mode_cross`.**
- **The Σ-refine builds** (`build_refine`, `parser.rs:546`: attributive, N-N compound, PP-mod,
  restrictive-relative). CN-as-types. **Keep.**

## 9. Risks & open decisions

**The three biggest open decisions** (the ones needing separate deliberation before code):

1. **The mode regime and the modes-vs-ENF balance (the spurious-ambiguity center of gravity).** How
   many modes, and how much restriction lives in *modes* (lexical) vs *ENF* (derivational)? The full
   Baldridge 4-point lattice + `mode_var` (D-1) is expressive but heavier; a coarse `{app, harm, var}`
   is simpler but pushes more onto ENF. **Unverified from the files present:** the exact correspondence
   of `core-en`'s `<`/`>`/`^`/`*` to the abstract lattice `·`/`⋄`/`×`/`⋆` — the modality *declaration*
   and the Java rule-firing (`m' ≤ m`) are **not** in `core-en/` (only the surface distribution in §2
   is verified). This must be confirmed against `references/publications/Baldridge_dissertation.pdf`
   before fixing the lattice. Compounding it: at 7.6M-lexicon scale the dominant residual is
   **structural PP-attachment ambiguity**, which the packed forest does *not* collapse
   (`d63-packed-forest-parsing-blueprint.md` §10c, "packing gives ~5% there") — modes must not make
   this worse.

2. **The WordNet/UMLS importer coupling — the default mode + feature richness a generic verb slot
   emits, gated by a full reseed.** `FrameKind::cat` (`convert.rs:183`) emits every slot as
   `cat_np(Entity, num_any)` with plain slashes. The port must choose: which **mode** does a generic
   verb argument slot get? Too permissive (`mode_perm`) over 7.6M entries risks composition explosion;
   too restrictive (`mode_app`) blocks extraction/relativization on imported verbs. And whether to
   emit `nom`/`acc`/`pers` (D-2) at all is a schema decision paid by a ~12-min reseed. This is the
   single largest coupling; getting the importer's emitted mode wrong is a 7.6M-row mistake.

3. **The keep/remove boundary — the finding that composition supersedes only the *decision*, not the
   *build*, and that coordination is special in both grammars.** The reactive framing assumed
   `relativize`/`relativize_appos`/`coordinate_*`/`reciprocate` would be "superseded by composition."
   The grounded analysis (§8) says otherwise: the typed sem builders are irreducible and coordination
   never reduces to the combinator core. The decision to ratify is that the **reference-as-map boundary
   is decision-side** — we port the map's *combinatorial control* and keep our *typed build*
   throughout.

**Further risks (lower-stakes, noted not to drop them):**
- **HLDS-style underspecification — decline for meaning, accept only for scope.** `core-en`'s
  semantics is HLDS; we keep our eager typed λ-terms and do **not** import hybrid-logic LFs. The one
  place underspecification is already ours and compatible is *scope/referent holes* (`hole_specs`,
  `lookup.rs`; Dörre-style packed underspecification is already the packed-forest premise,
  `d63-packed-forest-parsing-blueprint.md` §3/§7). No new HLDS machinery is warranted.
- **Selectional typing coupling.** Modes gate combinability *syntactically*; the felicity gate gates
  *semantically* (a type error). They must stay orthogonal: modes erased by `⟦·⟧`, `cat_shape` carries
  the mode, felicity never sees it (§6.7). Guard-test this.
- **The Mood-vs-`form` deviation (§4 D-2).** We deliberately keep Mood separate from Fin where
  `core-en` conflates. This is defensible (Mood alone alters `⟦·⟧`) but means the map's `form=fronted`
  / `form=wh` values must be re-split across our Mood+Fin — a translation the port must specify per
  construction, not assume.

## 10. References

Local + license-cleared:

| Reference | Role | Status |
|---|---|---|
| OpenCCG `core-en` (`references/openccg/grammars/core-en/`) | categories / features / slash-modes / unary rules — **the map** | **LGPL — read & reimplement, do not ship** |
| OpenCCG `mini-english/rules.xml` | the inherited combinator inventory | LGPL — read & reimplement |
| Baldridge dissertation (`references/publications/Baldridge_dissertation.pdf`) | multimodal CCG (the mode lattice) | reference (verify §9-1 against it) |
| Eisner, *Normal-Form Parsing* (`references/publications/`) | spurious-ambiguity control | reference |
| [d63-dcg-engine-english-grammar.md](../design/d63-dcg-engine-english-grammar.md) | parser substrate, `⟦cat_s⟧=Prop`, ENF-as-built, licensing | internal (design) |
| [d63-packed-forest-parsing-blueprint.md](d63-packed-forest-parsing-blueprint.md) | packing signature, Option A, PP-attachment finding, 3g.1 | internal (design) |
| [d62-controlled-language-style-guide.md](d62-controlled-language-style-guide.md) | the **inert combinatory-core** measurement | internal (finding) |

*(Grounding hygiene TODO: add the Baldridge and Eisner entries to
[docs/references/eigenius_related_work.bib](../references/eigenius_related_work.bib) as verified
anchors.)*
