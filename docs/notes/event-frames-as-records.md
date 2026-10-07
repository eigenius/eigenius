# Addressing #270 with frames as records

*Design note for `docs/notes/`. It works out Cooper's frames-as-records route to eigenius#270 and
sets it against the event-argument design in [`event-semantics.md`](event-semantics.md). Written
`2026-10-07`. It changes no code. The maintainer accepted the premise that the route needs a record
extension former in the kernel.*

The source is Robin Cooper, *From Perception to Communication* (OUP 2023, open access), §2.2 "The
string theory of events" and §5.4 "Frames as records".

## What Cooper proposes

**Frames are records; roles are fields.** "Our leading idea in modelling frames is that they
correspond to records and that the roles (or frame elements in the terminology of FrameNet) are
represented by the record fields. Records are in turn what we use to model situations so frames and
situations in our view turn out to be the same" (§5.4). Both levels are kept: frames are records,
frame types are record types. FrameNet's `Ambient_temperature` becomes

```
AmbTempFrame = [ x : Real, loc : Loc, e : temp(loc, x) ]
```

**Events are strings of sub-events, and strings are records.** §2.2 follows Fernando: an event is a
finite-state string of punctual observations, "similar to the kind of sampling we are familiar with
from audio technology". `play_fetch(a,b,c)` decomposes into `pick_up(a,c)`, `attract_attention(a,b)`,
`throw(a,c)`, `run_after(b,c)`, `pick_up(b,c)`, `return(b,c,a)`. Strings are modelled as records with
labels `t0, t1, …`, so `a₁a₂a₃` is `[t0 = a₁, t1 = a₂, t2 = a₃]`.

**The two sections are one mechanism, and §5.4 shows the seam.** `AmbTempFrame` revises Cooper's 2012
proposal by deleting its time field: "we now want to treat time in terms of strings of events rather
than introducing time-points as such." The temperature-rise type then reads
`ζtemp(e[0]) < ζtemp(e[1])` — indexing positions in an event string. Frames and events are both
records; an event is a record whose labels are string positions.

## What #270 needs

#270 is one defect: a verb-adjunct PP is predicated of the subject.
`λx.λV.λs. And(V(s), prep(s, x))` makes «Project Achilles screened cell lines with a CRISPR library»
say Project Achilles is with the library. The issue lists three candidate attachment points —
the proposition (option A, `prep(V(s), x)`), an event argument (option B), or case-by-case rulings
(option C). The maintainer chose B on `2026-10-06`.

Cooper offers a fourth: the adjunct is a **field**.

## The design

```
Ev        := lexicon:Eventuality                      as in event-semantics.md
Mod       := Set → Set                                was (Ev → Prop) → Ev → Prop
⟦S[dcl]⟧  =  Mod → Prop
⟦v_t⟧     =  λo.λs.λK. ∃r : K([ ev : Ev, c : v_t(o, s, ev) ]). ⊤
⟦in⟧ adj  =  λx.λV.λs.λK. V(s)(λR. K(R ⊕ [ c_in : prep_in(ev, x) ]))
close     =  λV. V(λR. R)
```

**#270's defect is inexpressible.** There is no `prep(s, x)` to write. The subject is a field value,
and the preposition applies to the `ev` field.

**The continuation stays; only its type changes.** A first draft of this note closed the existential
at the VP boundary and claimed the continuation disappears. That is wrong.
`event-semantics.md`'s table evaluates VP-level closure (Diesing/Kratzer via Maienborn) and rejects
it: "object GQs still raise", fitting a chart CCG with in-situ GQs only "partly". «John kissed every
girl» needs `∃e` below the object quantifier, which is why Champollion puts closure inside the verb's
entry — "putting existential closure into the lexical entry of the verb will automatically derive the
fact that all other quantifiers always have to take scope above existential closure".

Closure must therefore sit inside the verb here too, which means the record is closed before any
adjunct is seen, which means the adjunct needs a continuation to reach it. `Mod` is retyped from
`(Ev → Prop) → Ev → Prop` to `Set → Set` and nothing else about the threading changes. The note's
"three Eigenius facts" still apply: the existential is still impredicative rather than a `Σ`, the
modals are still VP-level, and the continuation still transforms rather than conjoins — the identity
transformer is now `λR. R` instead of `λP. P`.

**Fields are named by preposition, not by role** — `c_in`, `c_with`. That is the one argument in
`event-semantics.md` this route escapes. Its §"General role inventories do not hold up" objects to
*general* inventories: Dowty's proto-roles "do not classify arguments exhaustively … or uniquely …
or discretely", CGEL doubts "a small number of general roles … perhaps in the order of a dozen", and
Williams' *sold ⇒ bought*. Preposition-named fields are not an inventory at all.

## What it costs

### Three kernel additions

**1. A record extension former.** `Exp::Record(Vec<(Iri, Patt, Exp)>)` and
`Construct(Iri, Vec<(Iri, Box<Exp>)>)` both take *literal* field lists. No term takes a record and
returns it with a field added, and there is no row polymorphism. Lexical semantics is object-level
EigenTT terms on the chain (`lexicon:SemTerm`, `lexicon:term = type_expr(…)`) composed by
application, so `⟦in⟧` above is not writable today. The addition would be
`Extend(Box<Exp>, Vec<(Iri, Patt, Exp)>)` evaluating through `Exp::record`, inheriting its
canonical-order invariant and its `DependencyCycle` and `DuplicateField` rejections — plus its codec
encoding, a conversion arm, and ESL surface.

**2. Width subtyping on `Record`.** Modifier-dropping — «screened with a library» ⊢ «screened» — is
the property Davidson gets from conjunction elimination and `event-semantics.md` gets from the event
conjunct. Here it has to come from `R ⊕ [c_in] <: R`, and **`subtype_of` has no such rule**. Its
record-bearing arms are `Refine(R,S) <: EigonClass(C)` when `C ∈ S`, and `Refine <: Refine` by
constraint-set inclusion with `conjunction_entails`; the carriers then go to `subtype_of_inner`,
which compares them without a width rule. The width subtyping that does exist is at the
class-constraint level in `program/ground.rs` — `entails`, pinned by
`more_fields_entails_fewer_without_any_subclass_declaration` — and it compares *classes* by their
required property sets, not record types by their fields.

Routing the entailment through that existing machinery instead means declaring a frame **class** per
adjunct combination, which is combinatorial in the number of adjuncts on a verb. That is why the
second addition is listed as a cost rather than avoided.

**3. `Extend` must open the record's telescope, and that makes a binder name load-bearing.** The
added field's type mentions the event: `R ⊕ [ c_in : prep_in(ev, x) ]`. `ev` is a *binder* of `R`'s
telescope, not a projection — `PropAccess(Box<Exp>, Iri)` projects a field from a record *value*, and
`R` here is a type. So `Extend(R, fields)` must check `fields` in a context extended by `R`'s
telescope, and the adjunct entry — written once, applied to every verb — can only name that binder if
every event frame binds it identically. The convention "every event frame's eventuality field binds
`ev`" then carries semantic weight. `Exp::record`'s `DuplicateBinder` check already treats binder
names as significant, so there is precedent, but this goes further: it makes a *particular* name part
of the interface between the lexicon and the type language.

### The event argument does not go away

A record's identity is its field values. The constraint field `c : v_t(o, s, ev)` is `Prop`-typed,
and D46 §§4–5 makes `Prop` proof-irrelevant **by design** — `event-semantics.md` line 213 states it,
noting that the kernel applies irrelevance in `def_eq_at_type` and not in `eq_nf`, and that "either
way" two proofs of one `hug(a,b) : Prop` "would be identified by the design". So a constraint field
carries no identity, and without `ev : Ev` two screenings of the same cell lines by the same lab
would be one record.

Hence the `ev : Ev` field in the design above. **Cooper's route contains the Davidsonian event
argument; it does not replace it.** What changes is how adjuncts attach to it — a field rather than a
conjunct — and that the continuation disappears.

This also bounds how much of Cooper transfers. His events are *witnesses* of ptypes: "hug(a,b) can be
considered to be an event type" (Cooper 2023, p. 15). That is the part proof irrelevance rules out,
and it is what `event-semantics.md` already recorded. §5.4's records survive; §2's ontology does not.

### Frames are not unified

`event-semantics.md` uses "frame" for Maienborn's frame adverbial and CGEL's domain adjunct: a PP
saying where the *claim* holds, encoded `frame_in : Prop → Entity → Prop`. Cooper's frames are
FrameNet frames as record types. The words coincide; the phenomena do not.

Making «in MSI models» a field of the event record asserts that the event has an `in` participant,
which is #270's defect one level up. And the note decided copular predicates carry no event, so for
«WRN is essential in MSI models» there is no record to extend. `frame_in` stays, and the drop test —
"would the paper assert the clause without the PP?" — still adjudicates. This route changes the
encoding of participant/circumstance PPs only.

## Side by side, by mechanism

| | `event-semantics.md` (decided) | frames as records |
|---|---|---|
| verb type | `Entity → Entity → Mod → Prop` | `Entity → Entity → Set` |
| adjunct | conjunct on the event, via `K` | field, via `⊕` |
| continuation | `Mod := (Ev → Prop) → Ev → Prop` | `Mod := Set → Set` — retyped, not removed |
| `∃e` closes | inside the verb entry | inside the verb entry (VP boundary fails on object GQs) |
| modifier-dropping | `∧`-elimination | `Record` width subtyping (**absent**) |
| event argument | yes | yes, as the `ev` field |
| roles needed | no | no (fields named by preposition) |
| Maienborn frames | `frame_in : Prop → Entity → Prop` | unchanged — same mechanism still needed |
| coordination | `And` at `Prop` | no type-level join; closes per conjunct |
| kernel change | none | `Extend` former (telescope-opening) + `Record` width subtyping |
| grammar/lexicon | 1 `denote_cat` branch, verb converter, 316/465 entries, coordination, 51/62 pins, 213/228 ledger rows | the same surface, different terms |

## The two proposals, worked

Both designs are stated over the same four sentences. The first is #270's own example, the next
three are the cases `event-semantics.md` settled as open questions 7, 6(i) and 5.

Shared notation: `Ev := lexicon:Eventuality`, `K` is the continuation, closure is the identity
transformer applied at the root or at an embedding boundary.

### Entries

| | A — event argument (decided) | B — frame as record |
|---|---|---|
| `Mod` | `(Ev → Prop) → Ev → Prop` | `Set → Set` |
| `⟦S[dcl]⟧` | `Mod → Prop` | `Mod → Prop` |
| `⟦v_t⟧` | `λo.λs.λK. ∃e:Ev. K(λa. v_t(o,s,a))(e)` | `λo.λs.λK. ∃r : K([ev:Ev, c:v_t(o,s,ev)]). ⊤` |
| `⟦in⟧` adjunct | `λx.λV.λs.λK. V(s)(λP. K(λa. And(P(a), prep_in(a,x))))` | `λx.λV.λs.λK. V(s)(λR. K(R ⊕ [c_in : prep_in(ev,x)]))` |
| `⟦not⟧` | `λV.λs.λK. V(s)(K) → logic:False` | identical |
| `⟦can⟧` | `λV.λs.λK. logic:Possible(V(s)(K))` | identical |
| `⟦some⟧` subj | `λA.λV.λK. exists_sem(A)(λx. V(x)(K))` | identical |
| closure | `λS. S(λP. P)` | `λS. S(λR. R)` |
| `⟦in⟧` frame | `λx.λS.λK. frame_in(S(K), x)` | identical |

Only three rows differ. Everything that consumes a VP or a clause — negation, modals, quantifiers,
frames — is unchanged, because all of them wrap `V(s)(K)` from outside and never look inside `K`.

### 1. «Project Achilles screened cell lines with a CRISPR library.» — #270's case

| | |
|---|---|
| today (the defect) | `And(screen(cl, pa), prep_with(pa, lib))` — *Project Achilles* is with the library |
| A | `∃e:Ev. And(screen(cl, pa, e), prep_with(e, lib))` |
| B | `∃r : [ev:Ev, c:screen(cl,pa,ev), c_with:prep_with(ev,lib)]. ⊤` |

Both put the library on the screening. Neither can express the defect: in A the preposition's first
argument is the event variable, in B it is the `ev` binder.

### 2. «Some cancers do not respond to immune checkpoint blockade.» — negation under a GQ

| | |
|---|---|
| A | `some Cancer (λx. (∃e:Ev. respond_to(icb,x,e)) → False)` |
| B | `some Cancer (λx. (∃r : [ev:Ev, c:respond_to(icb,x,ev)]. ⊤) → False)` |

Scope is `GQ > ¬ > ∃` in both. Neither design changes `⟦not⟧` or `⟦some⟧`, and the existential is
impredicative in both — `Σ` lands in `Set` at the maximum of its components' levels, so B's
`∃r : R. ⊤` needs the same `lexicon:exists_sem` encoding A's `∃e` does.

### 3. «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.» — coordination

`event-semantics.md` decided coordination is distributive: one event per conjunct, with the shared
continuation putting the adjunct on both.

| | |
|---|---|
| A | `And(∃e₁. And(promote(apo,d,e₁), prep_in(e₁,msi)), ∃e₂. And(promote(cca,d,e₂), prep_in(e₂,msi)))` |
| B | `And(∃r₁ : [ev,c:promote(apo,d,ev),c_in], ⊤, ∃r₂ : [ev,c:promote(cca,d,ev),c_in], ⊤)` |

**This is the one case where B is not a notational variant.** A coordinates two `Prop`s with `And`,
which is what the object language already has. B must either coordinate at `Prop` after closing each
conjunct — in which case the record buys nothing here and the coordination rule differs from the
adjunct rule — or coordinate two record *types*, for which there is no join: `grep` over
`kernel/src/nbe/term.rs` finds no `Join` or `Meet` former. Closing each conjunct separately, as
written above, is the only route B has today, and it means a coordinated VP leaves the record world
and re-enters it per conjunct.

### 4. «Scientists can exploit synthetic lethality for cancer therapeutics.» — modal and purpose

| | |
|---|---|
| A | `Possible(∃e:Ev. And(exploit(sl,s,e), prep_for(e,ct)))` |
| B | `Possible(∃r : [ev:Ev, c:exploit(sl,s,ev), c_for:prep_for(ev,ct)]. ⊤)` |

`⟦can⟧` is VP-level in both and wraps `V(s)(K)` from outside, so the modal is outermost in both and
U1's re-pin is the same under either.

### Where they differ, summarised

| | A | B |
|---|---|---|
| #270 fixed | yes | yes |
| terms carry | an `And` chain, one conjunct per adjunct | one record type, one field per adjunct |
| modifier-dropping | `∧`-elimination, performed by the consumer | `R ⊕ [c] <: R`, structural — **once width subtyping exists** |
| four adjuncts on one verb | a four-way `And` | a six-field record |
| coordination | `And` at `Prop`, unchanged | no type-level join; closes per conjunct (case 3) |
| binder names | not load-bearing | `ev` is part of the lexicon/type-language interface |
| Charlow test | "extend `K` only as `λP. K(λa. And(P(a), φ(a)))`" | restates as "extend `K` only as `λR. K(R ⊕ [c : φ(ev)])`" |
| entries changed vs A | — | three: the verb, the adjunct, the closure |

The Charlow row is the substantive one for soundness. `event-semantics.md` makes narrow `∃e` a
lexicon invariant a test can check without parsing, and B preserves that invariant in the same shape.
Neither design derives the narrow scope; both make it a property of the entries that a test enforces.

## Assessment

**The maintainer rules that record extension and `Record` width subtyping are wanted regardless**
(`2026-10-07`). That removes the kernel additions from this comparison: they are not a price #270
pays. What is left is the difference the two designs make to the semantics, and it is small.

Both designs introduce one event per verb. Both keep governed PPs positional. Both keep
`frame_in : Prop → Entity → Prop` for Maienborn's domain adjuncts. Both close the existential inside
the verb's entry and thread a continuation to reach the event. Both touch the same grammar and
lexicon surface. The record route's event is a field rather than an argument, its adjuncts are field
extensions rather than `And` conjuncts, and its continuation is typed `Set → Set` rather than
`(Ev → Prop) → Ev → Prop`.

What it buys over the decided design, once the kernel features are free:

- modifier-dropping becomes structural — `R ⊕ [c_in] <: R` — rather than an `∧`-elimination the
  consumer performs;
- the semantic terms carry no `And` chain, so a verb with four adjuncts is one record type rather
  than a four-way conjunction;
- event frames become the same artifact FrameNet describes, which §5.4's correspondence makes
  reusable beyond #270.

What it costs, with the kernel additions discounted:

- the `ev` binder name becomes part of the lexicon/type-language interface (addition 3 above);
- coordination is where the two stop being inter-translatable. Worked as case 3 above: A coordinates
  two `Prop`s with the `And` the object language already has, while B has no type-level join —
  no `Join` or `Meet` former exists — so it must close each conjunct and coordinate at `Prop`. A
  coordinated VP therefore leaves the record world and re-enters it per conjunct, and the
  coordination rule stops matching the adjunct rule.

**No recommendation.** The earlier draft recommended against adoption on the strength of the kernel
cost, and that argument is now withdrawn. The remaining difference is a judgement about whether
frames-as-records is the representation wanted for events, which #270 does not settle in either
direction.

Coordination is now worked rather than open, and it is the one asymmetry: B has no route that keeps a
coordinated VP inside the record world. Whether that matters depends on how much weight the record
representation is meant to carry beyond #270 — if event frames are wanted as FrameNet-shaped
artifacts in their own right, a coordination that exits and re-enters the representation is a defect
in the representation; if they are only an encoding of the adjunct attachment, it is a notational
wrinkle.

## A third option exists

`docs/notes/event-free-modification.md` (`2026-10-07`) works out Luo & Shi's event-free treatment:
adverbials as predicate modifiers, `ADV = (Entity → Prop) → (Entity → Prop)`, with the drop
entailments derived from the defining equation rather than from typing. It needs no event class, no
record extension, no width subtyping and no kernel change, and leaves `⟦S⟧` at `Prop` so the 316
closed-class entries do not move. The comparison above is therefore two of three, not two of two.

## Not examined

- Cooper's §2.2 sub-event strings. Both designs treat an event as atomic. Nothing in the WRN corpus
  has been checked for a construction that needs internal event order.
- Whether `Extend` would need row variables to type an adjunct generically, or whether a monomorphic
  `Extend` at each entry suffices.

## Sources

- Cooper, R. 2023. *From Perception to Communication: A Theory of Types for Action and Meaning*.
  Oxford UP (open access). §2.2, §5.4. `references/publications/From Perception to Communication.pdf`
- `docs/notes/event-semantics.md` — the decided design.
- eigenius#270 — the defect, its three options and what rests on it.
