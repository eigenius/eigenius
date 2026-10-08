# Addressing #270 with predicate modifiers, no events

*Design note for `docs/notes/`. It works out Luo & Shi's event-free treatment of adverbial
modification as an answer to eigenius#270, and sets it against the event-argument design decided on
`2026-10-06` ([`event-semantics.md`](event-semantics.md)) and the frames-as-records route
([`event-frames-as-records.md`](event-frames-as-records.md)). Written `2026-10-07`. It changes no
code.*

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
| predicate modifier | `And(screen(cl,pa), vprep_with(lib, screen(pa), cl))` |

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

      added:  axiom ontology:vprep_in : lexicon:Entity
                                     -> (lexicon:Entity -> Prop)
                                     -> lexicon:Entity -> Prop
                                        — modifies a predication; VP adjunct

⟦in⟧ VP adjunct   was:  λx.λV.λs. And(V(s), prep_in(s, x))
                  now:  λx.λV.λs. And(V(s), vprep_in(x, V, s))
⟦in⟧ noun-internal      unchanged
```

`V : lexicon:Entity -> Prop` is the VP with its object absorbed, awaiting the subject — the type it
already has. `vprep_in(x, V)` is Luo & Shi's `ADV`, instantiated at the PP's object; applying it to
`s` says the property `V`, modified by *in x*, holds of `s`.

This also states a distinction the ontology comment was reaching for and could not make with one
relation. `ontology.esl:66–67` documents the axioms as "`prep_*(s, y)`: **the predication s** stands
in the prepositional relation to y" — true of the VP-adjunct role and false of the noun-internal one,
which relates two entities. The two families separate what the one name conflated.

**`⟦S⟧` stays `Prop`.** The adjunct consumes an `Entity -> Prop` and returns an `Entity -> Prop`, so
the VP type is unchanged, and so is every entry that consumes a VP or a clause.

**Five axioms, not eight, and none retyped.** Only the prepositions used as VP adjuncts need a
`vprep_*`: the four attested in the pins — `vprep_in`, `vprep_from`, `vprep_with`, `vprep_to` — plus
`vprep_for` for «for cancer therapeutics», which `event-semantics.md`'s open question 5 re-pins to the
exploiting and which today's encoding pins to the scientists. `event-semantics.md`'s decision on framing
prepositions (open question 8 ii) names «in» and «with» as the two the page frames, and CGEL's other
domain-adjunct realisations as a planned augmentation; the same staging applies here — one
`vprep_*` axiom per preposition as it is attested or planned.

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

`And(screen(cl, pa), vprep_with(lib, screen(pa), cl))`. The library modifies the screening. #270's
defect is inexpressible: `vprep_with`'s first argument is the PP's object and its second is a
predicate, so there is no slot a subject could occupy.

### 2. «Some cancers do not respond to immune checkpoint blockade.»

`some Cancer (λx. respond_to(icb, x) → False)`.

**This is today's pinned reading, unchanged** (`expected-readings.tsv`, the «respond» row).
`event-semantics.md` works the same sentence and has to derive `some Cancer (λx. ¬∃e:Ev.
respond_to(icb,x,e))` through lexical closure and a continuation. Here there is no event quantifier,
so there is nothing to scope.

### 3. Quantified PP objects — «in every model»

`∀m. model(m) → And(V(s), vprep_in(m, V, s))`. The quantifier wraps the adjunct's output from
outside, which is how the GQ entries already work. `event-semantics.md`'s open question 6(ii)
required the quantifier to scope over the event quantifier; with no event quantifier the requirement
is vacuous.

### 4. Coordination — «promoted apoptosis and cell cycle arrest in MSI models»

`And(And(promote(apo,d), vprep_in(msi, promote(apo), d)), And(promote(cca,d), vprep_in(msi,
promote(cca), d)))`. Coordination is `And` at `Prop`, which the object language has. No per-event
distribution question arises, and no type-level join is needed — the gap
`event-frames-as-records.md` found in the record route.

### 5. Maienborn frames — unchanged

A PP that restricts the claim's domain is still a different phenomenon from one that modifies the
predication. `frame_in : Prop → Entity → Prop` and the drop test of open question 8(i) carry over
untouched. This route changes participant/circumstance PPs only, exactly as the other two do.

### 6. Purpose «for» — U1

«Scientists can exploit synthetic lethality for cancer therapeutics» gives
`Possible(And(exploit(sl,s), vprep_for(ct, exploit(sl), s)))`. The purpose attaches to the exploiting,
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
free-adjunct competitor becomes `And(respond(x), vprep_to(y, respond, x))`, which is well-formed
rather than contradictory — the same change `event-semantics.md` records for slice 2's
re-adjudication, reached without events.

## Cost

| | |
|---|---|
| kernel | none |
| ontology | **5 axioms added**, 0 changed: `vprep_in`, `vprep_from`, `vprep_with`, `vprep_to` for the four attested as adjuncts, plus `vprep_for` for U1's planned re-pin (open question 5). In `ontology.esl`. Bootstrap edit, so it rides a reseed. **The shape is already shipped**: `ontology:has_proportion : lexicon:Entity -> (lexicon:Entity -> Prop) -> units:Quantity(u"1") -> Prop` is the same higher-order form, and `median_over` takes a function argument, so such axioms already round-trip the D47 codec and validate |
| migration of encoded terms | **none** — `prep_*` keeps its type, so every committed term that mentions one stays well-typed |
| grammar | the VP-adjunct entry; `is_vp_adjunct_prep`'s category is unchanged |
| verbaliser | `adjunct_of` |
| closed-class entries | **none** — `⟦S⟧` stays `Prop` |
| event class | none |
| pins | **14**, the VP-adjunct `prep_*` occurrences. #270 lists 8 for option A; the pin count is 14 because `prep_from` (4) and a `prep_to` are adjuncts too |
| ledger | the rows for those 14 |
| noun-internal uses | **54 untouched** |

Against `event-semantics.md`'s costed path: one `denote_cat` branch, the verb converter, **316 of
465 closed-class entry types**, the coordination rules, 51 of 62 pins, 213 of 228 ledger rows.

The additive shape removes a risk the first draft of this note carried. Retyping `prep_*` in place
would have been a versioned-ADT change needing a migration for every already-encoded term that
mentions a preposition; adding `vprep_*` needs none, because nothing committed changes meaning. That
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
- **Adverbs.** `event-semantics.md` §"Adverbs" is not worked here.
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
- **Naming.** `vprep_*` is a placeholder. The two families need names that say which relates entities
  and which modifies a predication; this note does not settle them.
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
