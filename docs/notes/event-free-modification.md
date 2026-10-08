# Addressing #270 with predicate modifiers, no events

*Design note for `docs/notes/`. It works out Luo & Shi's event-free treatment of adverbial
modification as an answer to eigenius#270, and sets it against the event-argument design decided on
`2026-10-06` ([`event-semantics.md`](event-semantics.md)) and the frames-as-records route
([`event-frames-as-records.md`](event-frames-as-records.md)). Written `2026-10-07`.*

> ## Adopted `2026-10-07`
>
> **The maintainer adopts this design — option D, predicate modifiers, no events — in place of the
> event-argument design chosen on 2026-10-06.** Two further decisions were taken with it:
>
> - **Scope: both preposition families.** The measured-value family (`prep_*_value`, `prep_*_offset`,
>   `every_period`) carries the same defect, which its own comment in `ontology.esl` records — "a VP
>   adjunct and a noun modifier share the relation". **22 axioms are added and none changed.** See
>   §"The measured-value family has the same defect".
>
> **The counting rule is per entry, not per pin** (corrected while implementing, `2026-10-07`). An
> `adv_X` is needed wherever a VP-adjunct *entry* exists — `lexicon:prep_X_sem` — because every one
> of them predicates the PP of the subject. Counting the pins instead gave 5 and 8; counting the
> entries gives **13 and 9**. The eight entity-object prepositions the pins do not attest
> (`on between within without among beyond after before`) each have a VP-adjunct entry and so each
> carried the defect; `prep_with_value` likewise. The seven prepositions with only a noun-modifier
> entry (`lexicon:nmod_X_sem`: about, against, at, by, into, of, upon) need none, and neither does
> `prep_of_value`, whose three uses are noun-internal.
> - **Naming: `adv_*`.** It names Luo & Shi's `ADV` type, and one prefix covers PP adjuncts and
>   manner adverbs, which §"Adverbs" shows are the same type. `prep_*` then means exactly "a relation
>   between two entities".
>
> Open questions 1, 2, 3 and 9 of `event-semantics.md` lapse with the event argument. Whether the
> eventivity feature is still wanted for Katz's stative adverb gap is not decided here; nothing in
> this design needs it.

**Recommended `2026-10-07`** as the answer to #270, in place of the event-argument design decided on
2026-10-06. `event-semantics.md` carries a banner pointing here.

Source: Zhaohui Luo & Yunbao Shi, "Variable polyadicity without events: a type-theoretic analysis of
event semantics", *Mathematical Structures in Computer Science* 36 (2026), e11,
doi:10.1017/S0960129526100504. Read §§1–2.3 of 23 pages.

`event-semantics.md` already cites this paper, once, for a remark about where the event quantifier is
introduced. Its proposal was not examined.

## What Luo & Shi propose

Davidson introduced events to solve **variable polyadicity**: an action verb takes a different number
of arguments depending on how many modifiers it carries, and a fixed-arity predicate cannot express
that. Luo & Shi solve it with dependent typing instead.

Adverbials keep the classical Montague type. Verbs become Π-types over the natural numbers:

```
ADV           = (e → t) → (e → t)
butter        : Πn:N. TV-ADV(n)
TV-ADV(0)     = e → e → t
TV-ADV(n+1)   = ADV → TV-ADV(n)

butter(0)                 = BUTTER
butter(n+1, advₙ₊₁, x, y) = butter(n, advₙ, x, y) & advₙ₊₁(BUTTER(x), y)
```

«John buttered the toast with the knife in the kitchen» is `butter(2, with_knife, in_kitchen, j,
toast)`, which **by definition** equals

```
BUTTER(j,toast) & with_knife(BUTTER(j), toast) & in_kitchen(BUTTER(j), toast)
```

They call the two the **VP-form** and the **conjunctive form**, and state the second is "equal to"
the first, not merely entailed by it.

**The entailments are derived.** `TV(n+1, advₙ₊₁, x, y) ⊃ TV(n, advₙ, x, y)` follows from the
defining equation, because the `n+1` case contains the `n` case as a conjunct. The paper is explicit:
"just like event semantics, we have obtained the expected inference relationships between such
sentences concerning their adverbial modifiers **without resorting to meaning postulates (or
events)**." Commutativity of modifiers follows from `&`, **in the paper's flat form** — see
§"Stacked adjuncts nest, and do not commute" for what a categorial grammar can and cannot reproduce
of that. Both are machine-checked — "implemented in the Coq proof development system … including the
above inference relationship (23) as a theorem".

**The modifier never touches the subject.** `advₙ₊₁(BUTTER(x), y)` applies the modifier to
`BUTTER(x)` — the verb with the subject already absorbed — and then to the object.

## Why this answers #270

#270 is that `λx.λV.λs. And(V(s), prep(s, x))` predicates the PP of the subject. The three readings
compared:

| | |
|---|---|
| today | `And(screen(cl,pa), prep_with(pa, lib))` — *Project Achilles* is with the library |
| events (decided) | `∃e:Ev. And(screen(cl,pa,e), prep_with(e, lib))` |
| predicate modifier | `And(screen(cl,pa), adv_with(lib, screen(pa), cl))` |

**The ontology already says this is the intent.** `ontologies/ontology/ontology.esl:66–67` documents
the preposition axioms as "`prep_*(s, y)`: **the predication s** stands in the prepositional relation
to y". The declared type is `lexicon:Entity -> lexicon:Entity -> Prop` and the encoding passes the
subject. #270 is the gap between that comment and both the type and the term.

## The proposed encoding

**Two preposition families, because `prep_*` already does two jobs.** A first draft of this note
changed `prep_*`'s type in place and claimed "one axiom type changes". That is wrong, and the pins
show why: `ontology:prep_in` serves both a noun-internal modifier and a VP adjunct, and only the
second is broken.

| role | example | pin shape | count | state |
|---|---|---|---|---|
| noun-internal | «deficiencies in homologous recombination» | `prep_in(G#1, kind_of(§))` — a Σ-bound entity | 54 | **correct** |
| VP adjunct | «These mutations occur in nucleotide repeat regions» | `prep_in($demref$1, …)` — the subject | 14 | #270's defect |

Noun-internal uses span eight prepositions — `prep_of` 19, `prep_in` 13, `prep_for` 10, `prep_from` 4,
`prep_with` 3, `prep_between` 3, `prep_to` 1, `prep_on` 1. VP-adjunct uses span four — `prep_in` 6,
`prep_from` 4, `prep_with` 3, `prep_to` 1.

Retyping `prep_*` in place would break all 54 noun-internal uses, which are not broken and have
nothing to do with #270, and would require migrating every already-encoded term that mentions one.
**So the change is additive**: `prep_*` keeps its type and its meaning for noun-internal
modification, and a second family handles VP adjuncts.

```
  unchanged:  axiom ontology:prep_in  : lexicon:Entity -> lexicon:Entity -> Prop
                                        — relates two entities; noun-internal

      added:  axiom ontology:adv_in : lexicon:Entity
                                     -> (lexicon:Entity -> Prop)
                                     -> lexicon:Entity -> Prop
                                        — modifies a predication; VP adjunct

⟦in⟧ VP adjunct   was:  λx.λV.λs. And(V(s), prep_in(s, x))
                  now:  λx.λV.λs. And(V(s), adv_in(x, V, s))
⟦in⟧ noun-internal      unchanged
```

`V : lexicon:Entity -> Prop` is the VP with its object absorbed, awaiting the subject — the type it
already has. `adv_in(x, V)` is Luo & Shi's `ADV`, instantiated at the PP's object; applying it to
`s` says the property `V`, modified by *in x*, holds of `s`.

This also states a distinction the ontology comment was reaching for and could not make with one
relation. `ontology.esl:66–67` documents the axioms as "`prep_*(s, y)`: **the predication s** stands
in the prepositional relation to y" — true of the VP-adjunct role and false of the noun-internal one,
which relates two entities. The two families separate what the one name conflated.

**`⟦S⟧` stays `Prop`.** The adjunct consumes an `Entity -> Prop` and returns an `Entity -> Prop`, so
the VP type is unchanged, and so is every entry that consumes a VP or a clause.

**Thirteen axioms here, and none retyped.** One `adv_X` per preposition that **has a VP-adjunct
entry** — `lexicon:prep_X_sem` in `closed-class.esl` — since each of those entries predicates the PP
of the subject:

```
in  for  with  to  on  from  between  within  without  among  beyond  after  before
```

Four are attested in the pins (`in`, `from`, `with`, `to`) and a fifth, `for`, is planned for
«for cancer therapeutics», which `event-semantics.md`'s open question 5 re-pins to the exploiting. The
other eight are not attested on the gate page but each has an entry today, so each would otherwise
keep the defect — an earlier draft of this note counted pins and said five, which would have fixed
the measured instances and left the grammar broken. The seven prepositions with only a
noun-modifier entry (`lexicon:nmod_X_sem`: about, against, at, by, into, of, upon) need none, and
neither does `prep_per`, which is noun-internal. The measured-value family adds nine more, below.

### The Π over ℕ is not needed here

Luo & Shi's `Πn:N. TV-ADV(n)` gives a verb **one** lexical entry covering every arity. A CCG does not
need that: an adjunct is a category-level modifier `(S\NP)\(S\NP)`, so composing *n* of them builds
their conjunctive form directly. The conjunction that carries the entailment is already in today's
adjunct entry — `And(V(s), …)` — and #270 is only about what the second conjunct is.

This matters for feasibility. The Π route would need a `Nat` and a type-valued function defined by
recursion on it. **Large elimination is available**: the singleton-elim gate in
`nbe/check/inductive.rs` fires only when `decl.sort` is `Sort(0)`, so a `Set`-typed `Nat` is not
gated. But `core:Nat` is **not declared in any bootstrap ontology** — it occurs only in test
fixtures — so the Π route would need one declared. The CCG route needs neither.

### The measured-value family has the same defect

Decided `2026-10-07`: it is in scope, and the same split applies to it.

`ontology.esl` declares a second preposition family over measured values — `prep_*_value`,
`prep_*_offset`, `every_period` — whose comment records the conflation outright: "`prep_X_value(s, u,
q)`: **the predication or entity** s stands in the prepositional relation to the measured value q",
and "a VP adjunct and a noun modifier share the relation, as `in_prep` and `in_nmod` share
`prep_in`". The type says `lexicon:Entity`, so the predication reading is not expressible and the
subject lands in the slot.

**Measured, not inferred.** `kernel/tests/quantity_corpus.rs` parses the corpus over the bootstrap
chain and the content words, with no database. The terms it produces:

```
«The flask was incubated at 37 °C for 1 h.»
  And(And(ΠG#0:Prop. ΠG#1:Entity. incubate(the(Flask), G#1) → G#0 → G#0,
          prep_at_value(the(Flask), ‹K›, 6203/20)),
      prep_for_value(the(Flask), ‹s›, 3600))

«The medium was changed every 3 days.»       every_period(the(Medium), ‹s›, 259200)
«The cells were harvested after 72 h.»        prep_after_value(the(Cell), ‹s›, 259200)
«The RNA was purified 72 h after transduction.»
                                              prep_after_offset(the(Rna), kind_of(Transduction), ‹s›, 259200)

«The mice received a dose of 5 mg/kg.»       ΠG#1:ΣG#1:Dose. prep_of_value(G#1, ‹›, 1/200000). …
```

The first four put **the subject** in slot 1: the medium recurs every 3 days, the cells are after
72 h, the RNA is 72 h after the transduction. The fifth is Σ-bound under `Dose` — noun-internal and
correct, exactly as the entity-object family's 54 are.

| role | count | state |
|---|---|---|
| VP adjunct | 27 | the defect |
| noun-internal (`prep_of_value`) | 3 | **correct** |

Counts from `experiments/parsing/quantities/corpus.tsv`: `prep_for_value` 7, `prep_after_value` 6,
`prep_at_value` 5, `prep_after_offset` 4, `prep_of_value` 3, `every_period` 2, `prep_in_value` 1,
`prep_by_value` 1, `prep_before_offset` 1. The ratio is the inverse of the entity-object family's —
27 adjunct against 3 noun-internal, where that family is 14 against 54 — because these are methods
sentences, where almost every measured PP modifies the procedure.

The measured value goes ahead of the predicate, as the PP's object does:

```
unchanged:  prep_at_value : lexicon:Entity -> forall (u : core:unit) => units:Quantity(u) -> Prop
    added:  adv_at_value  : forall (u : core:unit) => units:Quantity(u)
                         -> (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop
    added:  adv_after_offset : lexicon:Entity -> forall (u : core:unit) => units:Quantity(u)
                            -> (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop
```

**Nine `adv_*` axioms**, by the same per-entry rule: `adv_at_value`, `adv_for_value`,
`adv_in_value`, `adv_with_value`, `adv_after_value`, `adv_by_value`, `adv_after_offset`,
`adv_before_offset`, `adv_every_period` — one for each `lexicon:prep_X_value_sem`.
`prep_with_value` is among them although the corpus does not attest it as an adjunct, because its
entry exists. `prep_of_value` gets no sibling: it has only `nmod_of_value_sem`.

The argument order is `prep_X`'s with the host dropped from the front and `V, s` appended —
`prep_at_value(x, u, q)` → `adv_at_value(u, q, V, s)`, `prep_after_offset(x, y, u, q)` →
`adv_after_offset(y, u, q, V, s)`.

**This is also where the verbaliser's gap is.** `adjunct_of` recovers the adverbial role by testing
whether a `prep_*`'s first argument is the subject of a verb in the same conjunction
(`verbalize.rs:1525`) — reconstructing what the term lost — and returns `None` for every `_value`
relation, so the measured-value family gets no adverbial rendering at all. Under `adv_*` the role is
in the type, so the test goes away and the family is rendered like any other adjunct.

### Stacked adjuncts nest, and do not commute

Found while implementing, `2026-10-07`. The paper applies **every** modifier to `BUTTER(x)`, the bare
verb, so its conjunctive form is flat:

```
   paper:  V(s) & adv₁(V, s) & adv₂(V, s) & … & advₙ(V, s)
```

A categorial adjunct receives the VP it attaches to, never the verb inside it. The first adjunct's
`V` is the bare verb; the second's is the VP the first produced:

```
  VP₁ = λs. And(V(s),   adv_at(x, V, s))      V = the bare verb
  VP₂ = λs. And(VP₁(s), adv_for(y, VP₁, s))    V = the modified predicate
```

No other derivation reaches the flat form: application and composition both nest, and the surface
order fixes which adjunct is outer, so there is nothing to choose. **Three consequences.**

**Modifier drop still holds**, for the outermost modifier — which is the one Luo & Shi's theorem is
about. `VP₁(s)` is a conjunct of `VP₂(s)`, so dropping `for y` leaves exactly the `at x` reading. This
is the property #270 chose the event design over option A for, and it survives.

**Commutativity does not hold.** «at 37 °C for 1 h» and «for 1 h at 37 °C» differ in the nested `V`,
not merely in `And` order. Under the pre-#270 encoding the conjunct *contents* were
order-independent (`And(And(V(s), prep_at(s,x)), prep_for(s,y))`), so this is a change. It is also
arguably the more faithful reading — the 1 h is the duration of the at-37-°C incubating — but it is
not the paper's, and the note said it was.

**The term doubles per stacked adjunct**, because `VP₁` occurs twice in `VP₂` (applied to `s`, and as
the modifier's argument). Measured by parsing with no database:

| sentence | stacked adjuncts | term |
|---|---|---|
| «The flask was incubated at 37 °C for 1 h.» | 2 | 469 chars |
| «The plates were spun at 931g for 2 h at 30 °C.» | 3 | 989 chars |

**The gate page has no stacked adjuncts.** Of its 63 pinned sentences, 49 carry no clause-level
adjunct, 12 carry one, and the single sentence with two — «These classifications were highly
concordant with PCR-based MSI phenotyping **and** with predicted MMR deficiency» — is a
*coordination*, which distributes into two conjuncts, each over the bare predicate. Nothing nests.
The growth is exercised only by the methods-section rows of
`experiments/parsing/quantities/corpus.tsv`, which the gate does not parse. The three-adjunct row
yields two readings that print to the **same** term, so the attachment ambiguity adds no skeleton.

## The cases

### 1. «Project Achilles screened cell lines with a CRISPR library.»

`And(screen(cl, pa), adv_with(lib, screen(pa), cl))`. The library modifies the screening. #270's
defect is inexpressible: `adv_with`'s first argument is the PP's object and its second is a
predicate, so there is no slot a subject could occupy.

### 2. «Some cancers do not respond to immune checkpoint blockade.»

`some Cancer (λx. respond_to(icb, x) → False)`.

**This is today's pinned reading, unchanged** (`expected-readings.tsv`, the «respond» row).
`event-semantics.md` works the same sentence and has to derive `some Cancer (λx. ¬∃e:Ev.
respond_to(icb,x,e))` through lexical closure and a continuation. Here there is no event quantifier,
so there is nothing to scope.

### 3. Quantified PP objects — «in every model»

`∀m. model(m) → And(V(s), adv_in(m, V, s))`. The quantifier wraps the adjunct's output from
outside, which is how the GQ entries already work. `event-semantics.md`'s open question 6(ii)
required the quantifier to scope over the event quantifier; with no event quantifier the requirement
is vacuous.

### 4. Coordination — «promoted apoptosis and cell cycle arrest in MSI models»

`And(And(promote(apo,d), adv_in(msi, promote(apo), d)), And(promote(cca,d), adv_in(msi,
promote(cca), d)))`. Coordination is `And` at `Prop`, which the object language has. No per-event
distribution question arises, and no type-level join is needed — the gap
`event-frames-as-records.md` found in the record route.

### 5. Maienborn frames — unchanged

A PP that restricts the claim's domain is still a different phenomenon from one that modifies the
predication. `frame_in : Prop → Entity → Prop` and the drop test of open question 8(i) carry over
untouched. This route changes participant/circumstance PPs only, exactly as the other two do.

### 6. Purpose «for» — U1

«Scientists can exploit synthetic lethality for cancer therapeutics» gives
`Possible(And(exploit(sl,s), adv_for(ct, exploit(sl), s)))`. The purpose attaches to the exploiting,
which is the re-pin open question 5 calls for. `⟦can⟧` is VP-level and unchanged.

### 7. Copular predicates

`event-semantics.md` decided they carry no event and their PPs are frames (open question 2 → (c)).
Here there is no event anywhere, so the decision reduces to "their PPs are frames", which stands on
its own evidence. Open questions 1, 2, 3 and 9 — the `lexicon:Eventuality` class, its position in the
lattice, stative verbs taking the argument, and the eventivity feature on `S` — **do not arise**.
Whether the eventivity feature is still wanted to state Katz's stative adverb gap is a separate
question this note does not answer.

### 8. Governed PPs

Unchanged. `respond to` stays `respond_to(y, x)` positional, as the 2026-10-05 ruling has it. The
free-adjunct competitor becomes `And(respond(x), adv_to(y, respond, x))`, which is well-formed
rather than contradictory — the same change `event-semantics.md` records for slice 2's
re-adjudication, reached without events.

## Adverbs

The adverb categories **already denote `ADV`**: no category, type or pin changes under this
encoding. `event-semantics.md` §"Adverbs" worked the same ground under events.

**The category is already the right type.** `adverb_modifier_cats` (`kernel/src/dcg/category.rs`)
builds two categories, a forward pre-modifier and a backward VP modifier, both over
`VP = cat_s(dcl, fin) \ cat_np(Entity, num)`. `denote_cat` gives `⟦A\ₘB⟧ = ⟦B⟧→⟦A⟧`, `⟦cat_s(dcl,_)⟧ = Prop`
and `⟦cat_np(T,_)⟧ = T`, so

```
⟦VP⟧      = lexicon:Entity -> Prop
⟦VP \ VP⟧ = (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop
```

which is Luo & Shi's `ADV` exactly. The seeded sem is `λx. x` — `ADV`'s identity at that type. Routing
an adverb to a contentful entry is a change of sem at an **unchanged category and an unchanged
type**. Under events the same category's denotation moves with `⟦S⟧`, which is what makes 316 of 465
closed-class entry types part of that design's cost.

**No pin changes.** The 63 pinned sentences carry 11 adverb occurrences — nine productive `-ly` forms
(`selectively` ×2, `commonly` ×2, `typically`, `simply`, `preferentially`, `highly`, `favourably`) and
two lexicalized discourse adverbs (`also`, `thus`). Every one is erased in its pinned skeleton; no
adverb contributes a conjunct today. So nothing pinned distinguishes the two designs on adverbs, and
nothing pinned changes under this one.

**D62's routing stands**, as it does under events: inert adverbs are the identity, and measurement
adverbs go to justification logic (`d62-adverb-semantics-decision.md` §4). What this encoding adds is
that D62's identity *is already* `ADV`'s identity, so Phase 3's transparent treatment is the `n = 0`
case of the same scheme rather than a placeholder outside it.

**Where a contentful entry would go, by adverb kind.** Of the 11, one is manner on a verb — «compared
favourably», which is also one of the 14 VP-adjunct defects, since its «to» PP is pinned on the
subject. A manner entry and a PP adjunct are then the same shape, differing only in whether the
modifier has an object absorbed:

```
⟦favourably⟧   λV.λs. And(V(s), adv_favourably(V, s))
⟦to x⟧         λx.λV.λs. And(V(s), adv_to(x, V, s))
```

Four are adjective modifiers (`selectively essential` ×2, `highly concordant`, `preferentially
essential`); the forward category binds the clause's `Fin`, so it denotes the same `ADV` whether it
lands on a VP or a predicative adjective, and a contentful entry would have the same shape there.

One is a focus particle — «not simply a result of MMR deficiency», where the skeleton negates the
whole `is_a` and erases `simply`; focus particles are not modifiers of the VP's predicate and this
encoding says nothing about them. Two are the discourse adverbs `also` and `thus`, which attach at
`S/S` and `S\S` (`sentence_modifier_cats`) and stay transparent.

Three are **frequency** adverbs («More commonly», «most commonly observed», «typically arises»),
which quantify over occasions rather than conjoin a condition. `ADV` admits a non-conjunctive entry,
which is the standard advantage of `(e→t)→(e→t)` over a Davidsonian conjunct. Under events they would
need Champollion's outside-`V` quantifier instead, because the event `∃` is already closed inside the
verb's entry (open question 1); `event-semantics.md` names that pattern for `for`-adverbials but does
not work frequency adverbs. **Neither design works them here and no pin depends on it.**

The same holds for the intentional and modal adverbs (`deliberately`) that `event-semantics.md`
§"Adverbs" reports are not event predicates in any source it read — Davidson treats intention as
intensional, de Groote & Winter call modal adverbs "orthogonal to the main tenets of event
semantics", and the ERG gives scopal adverbs a handle argument. `ADV` takes the predicate as an
argument, so such an adverb is writable at the type without a conjunct. None occurs in the corpus.

### A rename reaches every site that matched the old name

The entries' categories and `sem_type`s did not move, and no combinator needed an edit — the parser
composes sem terms without inspecting relation names. That made it look as though no parser change
was involved. It was not so. **Two Rust sites match on the `prep_` prefix in a context that covered
the adjunct role, and both went stale the moment the adjunct moved to `adv_`.** The first commit
shipped without them; they were found by asking the question directly.

**`is_pp_refined` (`constructions.rs`) — a behavioural defect.** It gates `Guard::NotPpRefined`,
which kills classifier capture: a designator sits immediately after the nominal head, so something
postmodifying cannot intervene — «the gene MSH2 in humans», never «*the gene in humans MSH2». Its
`mentions_prep` walks a Σ restrictor for an `ontology:prep_` axiom. A restrictor holding a relative
clause with a VP adjunct — «genes that were essential **for** proliferation», which
`event-semantics-counts.py` names as the Σ-anchored adjunct and which is a ledger row — used to match
and be refused. With `adv_for` it stopped matching. Verified behaviourally, not just on the
predicate: with the `adv_` arm removed, `appose_group` **accepts** the bracketing it exists to
refuse. Both families now match, which is what the guard's surface argument wanted all along.

**`axiom_class` (`chart/attribute.rs`) — a diagnostic.** `compound_shape_label` splits
`Combinator::Compound` by restrictor shape, and an `adv_*` fell through to `"other"`, which the label
drops. It now has its own label, `"adverbial"`, rather than joining `"pp"`: inside a restrictor a VP
adjunct belongs to a relative clause, not to a postmodifier of the head, and lumping the two would
restate the conflation `adv_*` was introduced to end.

**Everything else that names `prep_` is correctly untouched**, and the division is the same one the
two families draw. The test sites all build noun postmodifiers — `prep_in(x, Mmr)` in
`constructions.rs`, `prep_of` in `attribute.rs`, the `prep_to`/`prep_of` restrictor keys in
`combinators.rs`. `category.rs` and `eigenius-wordnet/src/convert.rs` use `lexicon:Prep` **feature
constructors**, a different namespace from the ontology relations: `prep_to` there is a `Cat` feature
value for governed-preposition marking, not a relation a term applies.

The full workspace suite passed over both defects, so nothing covered them. Both now have a
regression test, and the `is_pp_refined` one was checked to fail without the fix.

## Cost

| | |
|---|---|
| kernel | none |
| ontology | **22 axioms added**, 0 changed, in `ontology.esl` — one per VP-adjunct entry. Entity-object (13): `adv_in`, `adv_for`, `adv_with`, `adv_to`, `adv_on`, `adv_from`, `adv_between`, `adv_within`, `adv_without`, `adv_among`, `adv_beyond`, `adv_after`, `adv_before`. Measured-value (9): `adv_at_value`, `adv_for_value`, `adv_in_value`, `adv_with_value`, `adv_after_value`, `adv_by_value`, `adv_after_offset`, `adv_before_offset`, `adv_every_period`. Bootstrap edit, so it rides a reseed. **The shape is already shipped**: `ontology:has_proportion : lexicon:Entity -> (lexicon:Entity -> Prop) -> units:Quantity(u"1") -> Prop` is the same higher-order form, and `median_over` takes a function argument, so such axioms already round-trip the D47 codec and validate |
| migration of encoded terms | **none** — `prep_*` keeps its type, so every committed term that mentions one stays well-typed |
| grammar | the 22 VP-adjunct sem-term bodies in `closed-class.esl`. **No category and no `sem_type` changes**: the entries were already typed `lexicon:Entity -> (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop)`, which is `ADV` with the object — only which relation the body names, and in which argument order. `is_vp_adjunct_prep` is unchanged |
| closed-class entries | **none** — `⟦S⟧` stays `Prop` |
| event class | none |
| pins | **14**, the VP-adjunct `prep_*` occurrences. #270 lists 8 for option A; the pin count is 14 because `prep_from` (4) and a `prep_to` are adjuncts too |
| ledger | the rows for those 14 |
| quantity corpus | **27 rows' relation names** in `experiments/parsing/quantities/corpus.tsv`, plus the `prep_*_value` assertions in `kernel/tests/quantities_in_the_parser.rs`. `kernel/tests/quantity_corpus.rs` parses without a database, so these are checkable before the reseed |
| verbaliser | `adjunct_of`, and its `_value` exclusion goes away — the role is in the type, so the subject test is no longer needed |
| noun-internal uses | **57 untouched** — 54 entity-object plus `prep_of_value`'s 3 |
| parser | **two sites, found `2026-10-07` after the first commit.** No category or combinator *edit* was needed, but two Rust sites match on the `prep_` NAME where the old name meant both roles, so both went stale: `is_pp_refined` (`constructions.rs`), which gates `Guard::NotPpRefined`, and `axiom_class` (`chart/attribute.rs`). See §"A rename reaches every site that matched the old name" |
| term size | **doubles per stacked adjunct** (§"Stacked adjuncts nest, and do not commute"): 469 chars for two, 989 for three. Zero exposure on the gate page, which has no stacked adjuncts; the methods rows of the quantity corpus are the exposure |

Against `event-semantics.md`'s costed path: one `denote_cat` branch, the verb converter, **316 of
465 closed-class entry types**, the coordination rules, 51 of 62 pins, 213 of 228 ledger rows.

The additive shape removes a risk the first draft of this note carried. Retyping `prep_*` in place
would have been a versioned-ADT change needing a migration for every already-encoded term that
mentions a preposition; adding `adv_*` needs none, because nothing committed changes meaning. That
risk was an artifact of overloading one relation for two jobs, not a cost of the design.

## How this relates to #270's option A

#270 lists option A as "attach the adjunct to the predication: `prep(V(s), x)`, with the preposition
relations taking a proposition (`Prop → Entity → Prop`)", and recommends it. The maintainer chose B.

This route is option A's family with a **predicate** type rather than a proposition type:
`vprep(x, V, s)` instead of `prep(V(s), x)`. The difference is that A's modifier receives a closed
proposition while this one receives the predicate and its argument separately, which is what Luo &
Shi's `adv(BUTTER(x), y)` form requires.

What Luo & Shi add to A is the part A was missing. `event-semantics.md` objects that predicate
modifiers do not deliver the drop entailments — the Charlow section notes the test "also guarantees
the modifier-drop entailments that `Qv→Qv` typing alone does not", and `Qv→Qv` is exactly `ADV`. The
objection is right about *typing alone*. Luo & Shi do not rely on typing: the conjunction is in the
defining equation, so `TV(n+1,…) ⊃ TV(n,…)` is a theorem. In the CCG encoding the conjunction is in
the adjunct entry, and the entailment is ∧-elimination — the same mechanism Davidson used, with no
event to quantify over.

## What this does not settle, and what is unverified

- **Non-intersective modifiers — checked `2026-10-07`, none in the corpus.** `ADV = (e → t) → (e → t)`
  admits them, which is its usual advantage over Davidson, but the `And(V(s), …)` form in the entry
  is only right for intersective ones. All twelve `expected-readings.tsv` pins carrying a `prep_*`
  conjunct are intersective: instruments («with a CRISPR library», «with an RNA interference
  library»), locatives («in models of microsatellite-stable cancers», «in nucleotide repeat
  regions»), sources («from Lynch syndrome», «from deficient DNA mismatch repair»). A
  non-intersective adjunct would need an entry without the conjunct, and the type admits it.
- **Adverbs — worked `2026-10-07`**, see §"Adverbs". No pin changes and no category or type changes;
  what stays unworked there is frequency adverbs, which neither design works.
- **§3 of the paper — read `2026-10-07`.** It is weaker than §2 and does not change the
  recommendation. §3.1 replaces event ordering with a **time argument** on every verb
  (`∃t,t'. sing(j,M,t) & salute(j,flag,t') & t < t'`), which for Eigenius is the same per-verb slot
  that events would be. §3.2 handles perception verbs with a local reification `E : t → e`, and is
  explicit that it is local: "introducing the mapping `E` is only for the interpretation of such
  sentences involving perceptual verbs, not in general." §3.3 **leaves nominalisation open** — "it is
  arguable whether event semantics is essential", pointing at Chierchia 1984/1985 and conceding that
  DRT's discourse referents may be "event-like structures", then "we shall not discuss these
  approaches in detail."
  Nominalisation is the one §3 defers and the one the corpus actually uses — «Depletion of WRN»,
  «Somatic MMR inactivation», «The co-occurrence of these two events» — and the corpus already
  handles it as nominals with `prep_of`, with no event and no `E`. Perception verbs and cross-clause
  time ordering do not occur on the gate page.
- **Naming — settled `2026-10-07`: `adv_*`.** It names Luo & Shi's `ADV` type, and one prefix covers
  PP adjuncts and manner adverbs, which §"Adverbs" shows are the same type. `prep_*` keeps its
  meaning: a relation between two entities. `adv_every_period` keeps the recognisable stem of a
  relation that never had the `prep_` prefix.
- **Whether `V` is the right predicate to modify.** The paper modifies `BUTTER(x)` — subject
  absorbed, object outstanding. Eigenius's `V : Entity -> Prop` is object-absorbed, subject
  outstanding. The two coincide for a transitive verb with both arguments, but the correspondence was
  derived here, not taken from the paper.

## Event anaphora, checked `2026-10-07`

Raised as the strongest candidate for needing events beyond adjunct attachment. It does not, as this
corpus exercises it.

- **Anaphora is not resolved at all.** Every demonstrative on the gate page is pinned as a λ-bound
  open parameter — «This success highlights the potential of this approach» is
  `λ($demref$0 : §). λ($demref$1 : §). §(the(ΣG#0:§. prep_of(G#0, $demref$0)).1, $demref$1)`.
  Resolution is a later step, so there is no eventuality to bind to either way.
- **The three demonstratives all have nominal antecedents**: «an impairment … This impairment», «a
  hypermutable state … This state», «are successful … This success». The last is the sharpest: its
  antecedent is a *copular* predication, which `event-semantics.md` decided carries no event, so
  adopting events would not give it a referent.
- **Event-denoting subjects are nominals.** «Depletion of WRN induced double-stranded DNA breaks» is
  `§(kind_of(§), kind_of(ΣG#0:§. prep_of(G#0, kind_of(§))))`. The corpus reifies events as nouns with
  their arguments as PPs, which the nominal machinery already handles.
- **What would need events**, and does not occur on the page: a bare demonstrative over a verbal
  antecedent («WRN was depleted. This caused apoptosis»), temporal predication of an event («It
  occurred within 48 hours»), and event individuation («They analysed the data twice. The second
  analysis…»).
- **Event counting does occur in the methods**, which the gate does not parse: «All experiments were
  performed three times», «immunofluorescence experiments were performed twice» — 17 occurrences of
  «twice» and 10 of «three times» in the full text. Nominalisation cannot encode "performed twice".
  **If methods sections come into scope, this is the construction that reopens the question.**

## Sources

- Luo, Z. & Y. Shi. 2026. "Variable polyadicity without events: a type-theoretic analysis of event
  semantics." *MSCS* 36, e11. doi:10.1017/S0960129526100504. §§1–2.3.
- `docs/notes/event-semantics.md` — the decided event-argument design.
- `docs/notes/event-frames-as-records.md` — the frames-as-records route.
- eigenius#270 — the defect and its three options.
- `ontologies/ontology/ontology.esl:60–80` — the preposition axioms and their documented intent.
