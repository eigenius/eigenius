# Addressing #270 with predicate modifiers, no events

*Design note for `docs/notes/`. It works out Luo & Shi's event-free treatment of adverbial
modification as an answer to eigenius#270, and sets it against the event-argument design decided on
`2026-10-06` ([`event-semantics.md`](event-semantics.md)) and the frames-as-records route
([`event-frames-as-records.md`](event-frames-as-records.md)). Written `2026-10-07`. It changes no
code.*

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
events)**." Commutativity of modifiers follows from `&`. Both are machine-checked — "implemented in
the Coq proof development system … including the above inference relationship (23) as a theorem".

**The modifier never touches the subject.** `advₙ₊₁(BUTTER(x), y)` applies the modifier to
`BUTTER(x)` — the verb with the subject already absorbed — and then to the object.

## Why this answers #270

#270 is that `λx.λV.λs. And(V(s), prep(s, x))` predicates the PP of the subject. The three readings
compared:

| | |
|---|---|
| today | `And(screen(cl,pa), prep_with(pa, lib))` — *Project Achilles* is with the library |
| events (decided) | `∃e:Ev. And(screen(cl,pa,e), prep_with(e, lib))` |
| predicate modifier | `And(screen(cl,pa), prep_with(lib, screen(pa), cl))` |

**The ontology already says this is the intent.** `ontologies/ontology/ontology.esl:66–67` documents
the preposition axioms as "`prep_*(s, y)`: **the predication s** stands in the prepositional relation
to y". The declared type is `lexicon:Entity -> lexicon:Entity -> Prop` and the encoding passes the
subject. #270 is the gap between that comment and both the type and the term.

## The proposed encoding

One axiom type changes and one entry changes.

```
                  was:  axiom ontology:prep_in : lexicon:Entity -> lexicon:Entity -> Prop
                  now:  axiom ontology:prep_in : lexicon:Entity
                                               -> (lexicon:Entity -> Prop)
                                               -> lexicon:Entity -> Prop

⟦in⟧ VP adjunct   was:  λx.λV.λs. And(V(s), prep_in(s, x))
                  now:  λx.λV.λs. And(V(s), prep_in(x, V, s))
```

`V : lexicon:Entity -> Prop` is the VP with its object absorbed, awaiting the subject — the type it
already has. `prep_in(x, V)` is Luo & Shi's `ADV`, instantiated at the PP's object; applying it to
`s` says the property `V`, modified by *in x*, holds of `s`.

**`⟦S⟧` stays `Prop`.** The adjunct consumes an `Entity -> Prop` and returns an `Entity -> Prop`, so
the VP type is unchanged, and so is every entry that consumes a VP or a clause.

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

## The cases

### 1. «Project Achilles screened cell lines with a CRISPR library.»

`And(screen(cl, pa), prep_with(lib, screen(pa), cl))`. The library modifies the screening. #270's
defect is inexpressible: `prep_with`'s first argument is the PP's object and its second is a
predicate, so there is no slot a subject could occupy.

### 2. «Some cancers do not respond to immune checkpoint blockade.»

`some Cancer (λx. respond_to(icb, x) → False)`.

**This is today's pinned reading, unchanged** (`expected-readings.tsv`, the «respond» row).
`event-semantics.md` works the same sentence and has to derive `some Cancer (λx. ¬∃e:Ev.
respond_to(icb,x,e))` through lexical closure and a continuation. Here there is no event quantifier,
so there is nothing to scope.

### 3. Quantified PP objects — «in every model»

`∀m. model(m) → And(V(s), prep_in(m, V, s))`. The quantifier wraps the adjunct's output from
outside, which is how the GQ entries already work. `event-semantics.md`'s open question 6(ii)
required the quantifier to scope over the event quantifier; with no event quantifier the requirement
is vacuous.

### 4. Coordination — «promoted apoptosis and cell cycle arrest in MSI models»

`And(And(promote(apo,d), prep_in(msi, promote(apo), d)), And(promote(cca,d), prep_in(msi,
promote(cca), d)))`. Coordination is `And` at `Prop`, which the object language has. No per-event
distribution question arises, and no type-level join is needed — the gap
`event-frames-as-records.md` found in the record route.

### 5. Maienborn frames — unchanged

A PP that restricts the claim's domain is still a different phenomenon from one that modifies the
predication. `frame_in : Prop → Entity → Prop` and the drop test of open question 8(i) carry over
untouched. This route changes participant/circumstance PPs only, exactly as the other two do.

### 6. Purpose «for» — U1

«Scientists can exploit synthetic lethality for cancer therapeutics» gives
`Possible(And(exploit(sl,s), prep_for(ct, exploit(sl), s)))`. The purpose attaches to the exploiting,
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
free-adjunct competitor becomes `And(respond(x), prep_to(y, respond, x))`, which is well-formed
rather than contradictory — the same change `event-semantics.md` records for slice 2's
re-adjudication, reached without events.

## Cost

| | |
|---|---|
| kernel | none |
| ontology | the preposition axioms' type — 8+ axioms in `ontology.esl`. Bootstrap edit, so it rides a reseed |
| grammar | the VP-adjunct entry; `is_vp_adjunct_prep`'s category is unchanged |
| verbaliser | `adjunct_of` |
| closed-class entries | **none** — `⟦S⟧` stays `Prop` |
| event class | none |
| pins | the 8 carrying an adjunct conjunct, per #270's own list for option A |
| ledger | their rows |

Against `event-semantics.md`'s costed path: one `denote_cat` branch, the verb converter, **316 of
465 closed-class entry types**, the coordination rules, 51 of 62 pins, 213 of 228 ledger rows.

## How this relates to #270's option A

#270 lists option A as "attach the adjunct to the predication: `prep(V(s), x)`, with the preposition
relations taking a proposition (`Prop → Entity → Prop`)", and recommends it. The maintainer chose B.

This route is option A's family with a **predicate** type rather than a proposition type:
`prep(x, V, s)` instead of `prep(V(s), x)`. The difference is that A's modifier receives a closed
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

- **Non-intersective modifiers.** `ADV = (e → t) → (e → t)` admits them, which is its usual
  advantage over Davidson. Whether any WRN-page adjunct is non-intersective has not been checked, and
  if one is, the `And(V(s), …)` form in the entry is wrong for it.
- **Adverbs.** `event-semantics.md` §"Adverbs" is not worked here.
- **§3 of the paper.** Event talk, perception words and nominalisation — the paper's argument that
  the *other* benefits of events are obtainable otherwise — were not read. They do not bear on #270
  but bear on whether events are wanted elsewhere.
- **The arity of `prep_*` in the chain.** Changing a shipped axiom's type is a versioned-ADT change;
  the migration for existing encoded terms has not been sized.
- **Whether `V` is the right predicate to modify.** The paper modifies `BUTTER(x)` — subject
  absorbed, object outstanding. Eigenius's `V : Entity -> Prop` is object-absorbed, subject
  outstanding. The two coincide for a transitive verb with both arguments, but the correspondence was
  derived here, not taken from the paper.

## Sources

- Luo, Z. & Y. Shi. 2026. "Variable polyadicity without events: a type-theoretic analysis of event
  semantics." *MSCS* 36, e11. doi:10.1017/S0960129526100504. §§1–2.3.
- `docs/notes/event-semantics.md` — the decided event-argument design.
- `docs/notes/event-frames-as-records.md` — the frames-as-records route.
- eigenius#270 — the defect and its three options.
- `ontologies/ontology/ontology.esl:60–80` — the preposition axioms and their documented intent.
