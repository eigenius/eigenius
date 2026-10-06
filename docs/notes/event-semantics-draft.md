# Close verb events inside lexical entries

*Design-note draft for `docs/notes/`. It answers eigenius#270 after the maintainer chose option B, event semantics, on 2026-10-06. It proposes changes; it changes no code.*

**Decided so far (the maintainer):**

- **Events** (2026-10-06, #270 option B): verb adjuncts attach to an event, not to the subject.
- **Verbs denote relations with an event slot** (2026-10-06): `screen : Entity → Entity → Ev → Prop`, not event-type families `screen : Entity → Entity → Set` with every `screen(o, s) ⊑ Ev`. The relation gives both views: the type of screenings is definable from it at any time (`Σe:Ev. screen(o, s, e)`), while a family gives no proposition saying of an event obtained elsewhere — a graph resource, a coreferent event in another sentence — that it is a screening of `o` by `s`. Families also leave `∃e : screen(o, s). ⊤` for an unmodified verb. Their kernel cost would be small (see "Kind 1 and kind 2 coercions" below), so the reason is the missing predicate, not the kernel.
- **Frames** (2026-10-06): a PP that says where a claim holds («in MSI models», «with PCR-based MSI classifications») can attach to the whole clause, clause-final (`S\S`) or fronted (`S/S`), as an opaque relation `frame_in : Prop → Entity → Prop` with sem `λx.λS.λK. frame_in(S(K), x)`. This is Maienborn's frame adverbial. The frame passes the continuation into the clause, so adjuncts inside still reach their events, and wraps the closed clause the way a modal does. Grounding maps the frame's object onto the domain predicate's argument. See "Frames: a PP can restrict the whole claim".
- **Copular predicates keep no event, and their PPs are frames** (2026-10-06, open question 2 → (c)). There is no Kimian state at the copula, and no reading anchors an adjunct on the subject.
- **Every verb takes the eventuality argument, stative ones included** (2026-10-06, open question 3). Where a sense is known to be stative, a later step may refine the argument's class; the structure never depends on it.
- **The class is a new `lexicon:Eventuality`** (2026-10-06, open question 1 → (b)), under `lexicon:Entity` and above WordNet's `event.n.01` and `state.n.02`. Declaring `schema_org:Action ≡ action.n.01` is a separate, later step.
- **When the event condition and the frame are both faithful, the ledger rules by what the PP does** (2026-10-06, open question 8 (i)). The frame is best when the PP gives the population, model, method or classification within which the finding holds; the event condition is best when it gives a participant or circumstance of the event, such as an instrument. The test is the entailment that separates the two readings: would the paper assert the clause without the PP? If not, the frame. A per-preposition rule fails on the page: «with» frames in «remained true with PCR-based MSI classifications» and is an instrument in «screened cell lines with a CRISPR library».
- **Framing prepositions** (2026-10-06, open question 8 (ii)): «in» and «with», in both positions, which are the two the page uses as frames. CGEL's other realisations of domain adjuncts are a planned augmentation, not left to attestation (slice 4): the other spatial-location prepositions and the dedicated domain PPs («with respect to», «as regards», «regarding», «from a … point of view», «as far as … is concerned»). The design keeps each addition to one `frame_*` axiom and its entries.
- **Frames inside embedded clauses** (2026-10-06, open question 8 (iii)): main clauses and «that»-complements take the clause-level frames as they are. Relative clauses take a VP-level frame, `λx.λV.λs.λK. frame_in(V(s)(K), x)`, for «in» and «with» on finite VPs. Where a PP could frame more than one clause, the frame restricts the smallest clause whose unrestricted content the paper would not assert, and all of that clause, so a main-clause frame sits above a quantified subject.
- **An eventivity feature on `S` keeps VP adjuncts off copular VPs** (2026-10-06, open question 9). `cat_s(mood, fin, evt)` gains `lexicon:Eventivity` (`eventive | eventless`), erased by ⟦·⟧ like Fin, Num, Prep and Mode. Verbs build eventive VPs and the copula eventless ones; VP adjuncts select eventive VPs; entries that pass a VP through bind the feature, entries that close a clause accept either, and a coordination is eventive only if every conjunct is. The grammar thereby states Katz's stative adverb gap for copular predicates.

**Recommendation.** Every verb gets one Davidsonian event argument, placed after its positional arguments. The subject, the object and any governed PP (`respond to`, `arise from`) stay positional in the verb's named relation. Adjunct PPs and adverbs become conjuncts on the event, and a PP that says where the claim holds can instead frame the whole clause. The event quantifier closes inside the verb's own lexical entry, as Champollion (2015) proposes and as ccg2lambda and lightblue implement.

**Why lexical closure.** Of the solutions surveyed, lexical closure is the only one that keeps the event quantifier lowest using function application alone. A chart CCG without hypothetical reasoning needs exactly that. It puts `∃e` under every generalised quantifier, under `→ False` and inside `Possible`. So «Some cancers do not respond to immune checkpoint blockade.» becomes `some Cancer (λx. ¬∃e. respond_to(icb, x, e))`, with the scope order it has today.

**What is not needed.**

- *Neo-Davidsonian thematic roles.* None of the five questions needs them. Leaving them out avoids D62's role-relation fork, the role-inventory problem, and three atoms per transitive verb instead of one.
- *Dependent event types* (Luo & Soloviev). Their families rely on function-inserting parameterised coercions that `Layer::is_subclass_of` does not have. They also solve a scope problem that lexical closure never creates. One event class is enough, and verbs stay relations (decided above).

**Where the event class sits.** Under `lexicon:Entity`, as a new `lexicon:Eventuality` above WordNet's `event.n.01` and `state.n.02` (decided). `urn:schema_org:Action`, the root D62 chose, is not under `lexicon:Entity`: it is a subclass of a parentless `schema_org:Thing`.

**Copular and adjectival predicates keep no event, and their PPs are frames.** No reading anchors an adjunct on the subject. The 8 copular or adjectival adjunct pins change meaning, and so do the 25 ledger rows whose PP hangs on an adjective or copula.

**No partial rollout.** There is no path on which only adjuncts introduce an event. Every VP-taking closed-class entry shares the VP's type, so the change lands as one slice:

- one branch in `denote_cat`;
- the verb converter;
- the types of 316 of 465 closed-class entries;
- the engine's coordination rules;
- at least 51 of 62 pins;
- 213 of 228 ledger rows.

The kernel does not change.

## Davidson's argument/adjunct line is the one Eigenius already draws

### What Davidson proposed

Davidson's 1967 proposal gives "verbs of action" one hidden place for an event. Subject and object stay positional arguments: `(∃x)(Kicked(Shem, Shaun, x))`. Prepositions become separate predicates of that event, conjoined under the one quantifier: `(∃x)(Flew(I, my spaceship, x) & To(the Morning Star, x))`. Dropping a modifier is then conjunction elimination. Davidson's argument: "we conceal logical structure when we treat prepositions as integral parts of verbs" ([Davidson 1967, pp. 47–48](https://terpconnect.umd.edu/~pietro/fall2020e/LogicalFormOfActionSentences.pdf)).

He kept two classes of adverb outside the scheme:

- attributive *slowly*;
- intentional *deliberately*, because "doing something intentionally is not a manner of doing it" ([Davidson 1967, pp. 38, 50](https://terpconnect.umd.edu/~pietro/fall2020e/LogicalFormOfActionSentences.pdf)).

When Castañeda objected that *to* has a different sense with each verb, Davidson met him "half-way" with a verb-class-specific *to* ([Davidson 1967, p. 54](https://terpconnect.umd.edu/~pietro/fall2020e/LogicalFormOfActionSentences.pdf)).

### The neo-Davidsonian line and Kratzer

The neo-Davidsonian line comes from Castañeda's reply and from Parsons (1990). Parsons is known here only through Maienborn, Kratzer and Williams. It makes the verb a one-place event predicate, and every participant enters through a role relation. As a result "it isn't possible anymore to read off the number of arguments a verb has from the logical representation" ([Maienborn 2011, p. 811](https://ub01.uni-tuebingen.de/xmlui/bitstream/handle/10900/47121/pdf/Maienborn_2011_Event_semantics.pdf?sequence=1&isAllowed=y)). Syntax then has to reject subjectless *kiss Mary*, which the semantics would assign a truth value ([Champollion 2015, p. 35](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)).

Kratzer severs only the external argument, and keeps the object positional for three reasons:

1. Objects select verb senses (*kill a conversation*).
2. Schein's cumulativity argument extends to agents and possessors but not to themes ([Kratzer, ch. 2, pp. 7–9](https://semanticsarchive.net/Archive/GU1NWM4Z/The%20Event%20Argument%20and%20the%20Semantics%20of%20Verbs.%20Chapter%202.pdf)).
3. The Cumulativity Universal "immediately disqualifies the 'theme' or 'object' relation" ([Kratzer, ch. 1, pp. 10–12](https://semanticsarchive.net/Archive/GU1NWM4Z/The%20Event%20Argument%20and%20the%20Semantics%20of%20Verbs.%20Chapter%201.pdf)).

### General role inventories do not hold up

Dowty's proto-roles "do not classify arguments exhaustively … or uniquely … or discretely" ([Dowty 1991, p. 576](https://linguistics.berkeley.edu/~syntax-circle/syntax-group/dowty91.pdf)). CGEL doubts that "a small number of general roles … perhaps in the order of a dozen" exists (CGEL, p. 228). General roles combined with ordinary event identity validate *Mo sold sausage ⇒ Mo bought sausage* ([Williams, §§20.8–20.12](https://linguistics.umd.edu/sites/default/files/2020-10/web-linguistics-williams-20-eventsinsemantics-galley.pdf)).

| Position | Verb meaning | Subject | Object | Adjunct PP | Eigenius type of a transitive verb |
|---|---|---|---|---|---|
| Davidson | `v(x, y, e)` | positional | positional | `P(e, z)` | `Entity → Entity → Ev → Prop` |
| Kratzer | `λy.λe. v(y)(e)` + Voice | Agent/Holder role | positional | `P(e, z)` | `Entity → Ev → Prop` + `Agent, Holder : Ev → Entity → Prop` |
| Parsons | `λe. v(e)` | role | role | role or `P(e, z)` | `Ev → Prop` + one role relation per participant |

### Governed PPs are complements

For governed PPs the sources agree with the 2026-10-05 ruling.

- **CGEL.** A preposition "specified by the verb" (*consist of, depend on*) gives the clearest PP complement. It cannot be swapped without an "unsystematic change in the meaning" (*look at* / *look for*), and with it "it is the verb that plays the major part in assigning the role" (CGEL, pp. 220, 227–228).
- **EAGLES.** It treats such prepositions as case markers whose value "is encoded as an attribute of the predicative head" ([EAGLES §2.9.2](https://www.ilc.cnr.it/EAGLES96/rep2/node13.html)).
- **ERG.** It gives a selected preposition no predicate and folds it into the verb's predicate name (`_argue_v_about` vs `_argue_v_for`). An adjunct preposition is its own predication whose ARG1 is the verb's event ([DELPH-IN PredicateRfc](https://github.com/delph-in/docs/wiki/PredicateRfc); [Copestake et al. 2005, §6](https://www.cl.cam.ac.uk/~aac10/papers/mrs.pdf)).
- **PropBank.** `respond.01` takes "to the bid" as ARG1, "in response to" ([PropBank respond.xml](https://github.com/propbank/propbank-frames/blob/main/frames/respond.xml)).
- **Boxer and ccg2lambda.** Their event templates write governed and adjunct PPs alike as `prep(e, y)` and erase the distinction ([Bos 2008, Fig. 3](https://aclanthology.org/W08-2222.pdf); [ccg2lambda event templates, l. 806–839](https://github.com/mynlp/ccg2lambda/blob/master/en/semantic_templates_en_event.yaml)).

| Representation of *X responds to Y* | Preposition contributes | Status in this proposal |
|---|---|---|
| `respond_to(Y, X, e)` (Davidson/ERG positional) | nothing; part of the relation's identity | **governed PP** |
| `respond(e) ∧ Stim(e, Y)` (neo-Davidsonian) | a verb-chosen role | rejected: needs a role inventory |
| `respond(X, e) ∧ prep_to(e, Y)` (Davidson's `To(x, e)`) | a general preposition relation | **adjunct PP** only; the forest keeps it as the competing parse |

### Decision for Eigenius: Davidsonian positional arguments plus one event argument

The decision follows the ERG precedent of positional arity plus an event argument ([ErgSemantics_Basics](https://github.com/delph-in/docs/wiki/ErgSemantics_Basics)). In WordNet-importer terms:

- `v{offset}_t : Entity → Entity → Entity → Prop` (object, subject, event);
- `v{offset}_i : Entity → Entity → Prop`;
- `v{offset}_p_{prep} : Entity → Entity → Entity → Prop`.

The event slot is typed `lexicon:Entity`, like every stage-1 verb slot (`crates/eigenius-wordnet/src/convert.rs:223–241`). Its sort comes from the binder `∃e:Ev` and from the continuation, which ranges over `Ev` (next section).

No role relation appears in a term, so D62's fork (role axioms vs a property-as-relation kernel feature, `docs/notes/d62-adverb-semantics-decision.md` §5) is not on the path.

**Effect on the 2026-10-05 rulings.** Their outcome survives and their stated ground changes. Under events, the free-adjunct reading of «MSI can arise from Lynch syndrome.» is `Possible(∃e. And(arise(msi, e), prep_from(e, lynch)))`. It predicates «from» of the arising, not of MSI. It stays `wrong` because the pinned governed reading `Possible(∃e. arise_from(lynch, msi, e))` is the one CGEL's criterion selects. Fourteen `wrong` ledger rows give "predicates … of the subject" as their ground, and their evidence text needs rewording.

## The event quantifier closes inside the verb

### Sentence-level closure gets the scope wrong

If one existential closure binds `e` at the top of the clause, every quantifier, negation or modal inside the clause ends up under `∃e`:

- «No boy laughed» would come out as "there is an event that is not a laughing by a boy";
- «John didn't laugh» would come out as "there is an event in which John does not laugh";
- «John kissed every girl» becomes a contradiction once roles are functions.

Empirically, the implicit event quantifier always takes the lowest scope ([Champollion 2015, pp. 36–37](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)). Winter & Zwarts named the question of what rules out the wide reading "the event quantification problem" ([Winter & Zwarts 2011, p. 177](https://www.phil.uu.nl/~yoad/papers/WinterZwartsEventSemantics.pdf)).

| Approach | Where `∃e` closes | Mechanism needed | Fits a chart CCG with in-situ GQs |
|---|---|---|---|
| Sentence-level closure (Parsons/Landman, via Champollion) | top of clause | QR of every GQ above closure | no |
| VP-level closure (Diesing/Kratzer, via Maienborn) | VP boundary | object GQs still raise | partly |
| Krifka 1989 (via Champollion) | top; negation via event fusion | non-classical negation | no: changes `→ False` |
| Winter & Zwarts 2011; de Groote & Winter 2015 (ACG) | sign `EC : vp → s` | hypothetical reasoning over NP slots | no: CCG cannot discharge hypotheses |
| Champollion 2015 | inside the verb's entry | function application only | **yes** |
| Bernard & Champollion 2018/2023 | top, with negative events | continuations, a Neg axiom | heavier; needed only for negative events as perception or anaphora objects |

Notes on the table:

- **Winter & Zwarts.** Their account derives narrow scope by typing alone. It rests on an NP hypothesis discharged after closure ([Winter & Zwarts 2011, pp. 183–185](https://www.phil.uu.nl/~yoad/papers/WinterZwartsEventSemantics.pdf)).
- **Champollion's construction.** He rewrites each verb as an existential quantifier over events, `⟦rain⟧ = λf.∃e[rain(e) ∧ f(e)]`, and closes the sentence with `λe.true`. "Putting existential closure into the lexical entry of the verb will automatically derive the fact that all other quantifiers always have to take scope above existential closure" ([Champollion 2015, pp. 38–40](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)).
- **Implementations.** ccg2lambda adopts this explicitly for its event templates ([Mineshima et al. 2016, §3.1–3.2](https://aclanthology.org/D16-1242.pdf)). lightblue builds it in Dependent Type Semantics: a Σ introduced by the verb, `negOperator = λp.λc. not(p c)`, and a `terminator = λ_.⊤` at the root and at every embedding boundary (`references/lightblue/src/Parser/Language/Japanese/Templates.hs:178–223, 261–263, 296–304`; `references/lightblue/src/Parser/CCG.hs:1305–1328`).
- **Charlow's objection.** Narrowest `∃e` "doesn't follow from anything deep in the setup". An equally typed role head could reverse the scope ([Charlow, slides 24–27](https://schar.github.io/sem2/files/notes/02-19-18.pdf)). In a grammar without QR, that objection becomes a lexicon invariant that a test can check.

### Three Eigenius facts fix the details

**1. The existential must be impredicative, not a Σ.** EigenTT puts a Σ at the maximum of its components' levels (D46 §3.4; `kernel/src/nbe/check/conv.rs:242–252`). So `Σ(e:Ev). φ` lands in `Set` when `Ev : Set`. The determiners need `Prop` bodies (`some : ∀A:Set. (A → Prop) → Prop`). The event existential must therefore use the same impredicative encoding `∀C:Prop. (∀e:Ev. φ(e) → C) → C` that `lexicon:exists_sem` already uses (`ontologies/lexicon/closed-class.esl:31–37`).

**2. The modals are VP-level today, not clause-level.** `poss_sem` is `λP.λs. logic:Possible(P(s))` at type `(Entity → Prop) → Entity → Prop` (`closed-class.esl:655–666`). A GQ subject therefore scopes over `Possible` today. Champollion's modal `λV.λf. ◇V(f)` is the same shape ([Champollion 2015, p. 50](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)), so this scope order carries over unchanged.

**3. The continuation should transform the predicate, not conjoin with it.** Champollion's `f` is closed with `λe.true`, which leaves a `True` conjunct in every clause. ccg2lambda's output carries exactly that artefact ([Martínez-Gómez et al. 2016, §6](https://aclanthology.org/P16-4015.pdf)), and pins would carry it too. Mineshima et al.'s Japanese system instead types sentences as `((Ev⇒Prop)⇒Ev⇒Prop)⇒Prop`. The continuation receives the verb's predicate and returns a predicate, so intensional modifiers can also wrap it ([Mineshima et al. 2016, §3.2](https://aclanthology.org/D16-1242.pdf)). With that shape the closure is the identity transformer and no `True` is left over. That is a derivation from their type, not a claim the paper makes.

### The proposed entries

The continuation ranges over `Ev`, the eventuality class. The copula passes it nothing (see "Copular predicates keep no event; their PPs are frames"), and a `K` applied to the subject would not type-check, since an `Entity` is not an `Ev`. The verb's event slot stays typed `Entity`, like every stage-1 slot, and takes an `Ev` by class subsumption.

```
Ev  := lexicon:Eventuality ⊑ lexicon:Entity        above wn:n00029378 event, wn:n00024720 state
Mod := (Ev → Prop) → Ev → Prop
⟦S[dcl, f]⟧ := Mod → Prop   for f ≠ adj          was Prop
⟦S[dcl, adj]⟧ := Prop                             unchanged

⟦v_t⟧          = λo.λs.λK. ∃e:Ev. K(λa. v_t(o, s, a))(e)
⟦v_p_to⟧       = λy.λx.λK. ∃e:Ev. K(λa. v_p_to(y, x, a))(e)    governed PP: positional
⟦in⟧ VP adjunct = λx.λV.λs.λK. V(s)(λP. K(λa. And(P(a), prep_in(a, x))))
⟦in⟧ frame     = λx.λS.λK. frame_in(S(K), x)                    S\S and S/S; frame_in : Prop → Entity → Prop
⟦in⟧ VP frame  = λx.λV.λs.λK. frame_in(V(s)(K), x)              finite VP; reaches relative clauses
⟦not⟧          = λV.λs.λK. V(s)(K) → logic:False                the current neg_sem, K passed in
⟦can⟧          = λV.λs.λK. logic:Possible(V(s)(K))
⟦be⟧ + adj     = λP.λs.λK. P(s)                                 no event: K has nothing to extend
⟦some⟧ subject = λA.λV.λK. exists_sem(A)(λx. V(x)(K))           quantifier core unchanged
⟦that⟧ comp    = λS. S(λP.P)                                    closure at the embedding
root           = λS. S(λP.P)
```

### Worked example: «Some cancers do not respond to immune checkpoint blockade.»

Today's pinned reading is `ΠG#0:Prop. ΠG#1:§. §(kind_of(§), G#1) → False → G#0 → G#0`, that is `some Cancer (λx. respond_to(icb, x) → False)` (`experiments/parsing/expected-readings.tsv`, the «respond» row). Under the proposal:

1. `not(respond_to(icb)) = λs.λK. (∃e. K(λa. respond_to(icb, s, a))(e)) → False`.
2. Adding the subject gives `λK. ∃x:Cancer. (∃e. K(…)(e)) → False`.
3. Closing with the identity gives `some Cancer (λx. ¬∃e:Ev. respond_to(icb, x, e))`.

The scope is GQ > ¬ > `∃e`. This is Champollion's (29b) pattern, with a quantified subject ([Champollion 2015, p. 47](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)).

«Some cancers can respond …» gives `some Cancer (λx. Possible(∃e. …))`. «MSI can arise from Lynch syndrome.» has a kind subject, not a GQ, and gives `Possible(∃e:Ev. arise_from(lynch, msi, e))`, so the modal is outermost as in today's pin.

In the printed term, each verb adds two binders and one argument: an extra `ΠG#k:Prop. ΠG#k+1:…` and `… → G#k → G#k`.

### Charlow's objection as a test

The narrow scope of `∃e` holds only if no entry puts a quantifier, a negation or a modal inside `K`. The test: every closed-class entry that consumes a VP `V` or a clause `S` must do one of two things.

1. Pass `K` through unchanged.
2. Extend it only as `λP. K(λa. And(P(a), φ(a)))`, where `φ` is an atomic relation.

GQs, `→ False`, `Possible` and the frames may wrap `V(s)(K)` or `S(K)` from outside. Like `reading_ledger_is_consistent`, the test needs no parse. It is the lexicon property Charlow says the narrow scope rests on, and it also guarantees the modifier-drop entailments that Qv→Qv typing alone does not ([Charlow, slide 23](https://schar.github.io/sem2/files/notes/02-19-18.pdf)).

## One event class, because EigenTT subtypes nominally

### What the type-theoretic literature offers

**Luo & Soloviev: dependent event types.** They keep a root `Event` and add families indexed by participants: `EvtA(a)`, `EvtP(p)`, `EvtAP(a,p)`, related by coercions `EvtAP(a,p) ≤ EvtA(a) ≤ Event` under a coherence condition. In MTT, `bark : Πx:Dog. EvtA(x) → Prop` makes the wide-scope event reading ill-typed ([Luo & Soloviev 2017, pp. 218–227](https://www.cs.rhul.ac.uk/~zhaohui/WoLLIC17.pdf)). Three facts limit what the families offer Eigenius:

- They are indexed by participants, never by verb. Verb meaning stays a predicate over events.
- The authors expect some roles to "be formalised by means of logical predicates/relations" ([Luo & Soloviev 2017, p. 227](https://www.cs.rhul.ac.uk/~zhaohui/WoLLIC17.pdf)).
- Luo's later work states that the problem arises only when the event quantifier is introduced outside the verb's denotation ([Luo & Shi 2026, p. 15](https://www.cs.rhul.ac.uk/home/zhaohui/MSCS.pdf)).

**Chatzikyriakidis & Luo** bring events in only for manner adverbs, as `Event` indexed by `Manner` ([Chatzikyriakidis & Luo 2017, PDF pp. 34–35](https://www.cs.rhul.ac.uk/home/zhaohui/JoLLI17.pdf)).

**What a flat `Event` loses.** With `talk : Event → Prop`, *Tables talk* becomes well-typed ([Luo, LACompLing 2018, slide 21](https://staff.math.su.se/rloukanova/LACompLing2018-web/slides-LACompLing2018/ZhaohuiLuo-LACompLing18final.pdf)). Eigenius loses nothing here: its stage-1 verb slots are all `lexicon:Entity` and enforce no selection restriction today.

**Cooper's TTR.** It takes the opposite route. A predicate applied to arguments is a type whose witnesses are events: "hug(a,b) can be considered to be an event type" ([Cooper 2023, p. 15](https://academic.oup.com/book/45788)). Event-type hierarchy comes from field inclusion and from postulates such as `buy(a,b,c) ⊑ sell(c,b,a)` ([Cooper 2023, pp. 260–261](https://academic.oup.com/book/45788)).

**The framing reference.** *Types and the Structure of Meaning* argues one position on events: an intensional type system lets one event carry two non-identical types, such as a selling that is also a buying, so adverbials can be predicate modifiers ([Chatzikyriakidis et al. 2025, pp. 19–21](https://doi.org/10.1017/9781009285322)). It stays neutral between MTT and TTR ([p. 56](https://doi.org/10.1017/9781009285322)), and it does not discuss dependent event types or the event quantification problem.

### Two kernel facts the notes left open

**Prop is proof-irrelevant by design.** D46 makes `Prop` impredicative and proof-irrelevant (D46 §§4–5). The kernel applies irrelevance in `def_eq_at_type`, whose production call sites are the `refl` check. `eq_nf` does not apply it (`kernel/src/nbe/check/conv.rs:42–59, 184–202`). Either way, two hugs of *b* by *a* as proofs of one `hug(a,b) : Prop` would be identified by the design. So TTR's events-as-witnesses is not available in EigenTT, and events must inhabit a class in `Set`. Luo (2012) also requires proof irrelevance, for CN-as-types semantics ([Luo 2012, p. 495](https://www.cs.rhul.ac.uk/home/zhaohui/LP13.pdf)).

**The subclass test has no parameterised coercions.**

- `Layer::is_subclass_of(sub, super)` walks closed `core:subclass_of` edges and D99 §11 class equivalences between IRIs (`kernel/src/layer/mod.rs:893–930`).
- The checker applies it only when both sides are `Val::EigonClass` (`kernel/src/nbe/check/mod.rs:883–889`).
- Inductive families compare parameters and indices invariantly (`conv.rs:459–496`).

So `EvtA(a) ≤ Event` cannot be stated, and dependent event types would be a kernel extension. Lexical closure makes them unnecessary.

### Kind 1 and kind 2 coercions

A family of conversions for every parameter value — `Π(o s : Entity). screen(o, s) → Entity`, or the first projection of `Σe:Entity. screen_rel(o, s, e)` — is an ordinary dependent function and can be stated today. What dependent types cannot add is the subtyping judgement that lets the checker accept a `screen(o, s)` member where an `Entity` is expected. `check_by_inference` (`kernel/src/nbe/check/mod.rs:875`) is the one place that judgement is made; it already accepts a subclass member for its superclass ("the inclusion-coercion fragment of coercive subtyping") and relates sized inductive parameters, and it returns `Result<(), CheckError>`: it accepts or rejects, it never rewrites a term.

- **Kind 1, inclusion coercions for families.** "Every member of `screen(o, s)` is a member of `Ev`", with the identity as the conversion. One more case at that check site, a declaration form and its validation rule; terms, `eq_nf`, hashing, pins and ledger untouched. It enters the trusted kernel and is sound only for families that are subsets of their class. Well below D46's 3–6 weeks.
- **Kind 2, Luo's coercive subtyping with inserted functions** (projections, buy/sell role permutations). The checker would have to elaborate — a change to the interface of the ~8,900-line checking module — or a separate elaboration pass would duplicate its inference; coherence (every path between two types gives one function) must be restricted to be decidable, and stored terms carry the inserted functions. At least D46's scale.

#270 needs neither: one class `Ev` uses the named-class rule the kernel has. Kind 1 is the cheap path to verbs as event-type families, which the relation form keeps available but was not chosen (see "Decided so far").

**Per-verb event classes are possible but not needed now.** TTR-style classes (`Hugging ⊑ Touching`) could be declared, since a resource may already inhabit several classes (`check/mod.rs:832–846`). They would put WordNet verb hypernymy into the subclass lattice. The Σ-refinement Eigenius already uses for nouns expresses a verb-specific event type when a task needs one: `Σe:Ev. v_t(o, s, e)`, after [Luo 2012, p. 495](https://www.cs.rhul.ac.uk/home/zhaohui/LP13.pdf).

### Where the class sits in the lattice

D62 compared `schema:Action` against `schema:Event`, which covers scheduled happenings, and chose Action. Two facts it did not weigh:

1. **`urn:schema_org:Action` is not an Entity.** It is a subclass of `urn:schema_org:Thing`, which has no parent (`ontologies/schema-org/schema-org.eigon.json`). An Action-typed event therefore cannot fill `prep_in : Entity → Entity → Prop`.
2. **Its definition presumes an agent.** It begins "An action performed by a direct agent". The corpus verbs `arise`, `result` and `occur` are not agentive.

WordNet has the two nodes an eventuality class needs, both under `entity.n.01`, which the importer roots at `lexicon:Entity` (`references/WordNet-3.0/dict/data.noun`):

- `event.n.01` (`wn:n00029378`, "something that happens at a given place and time"), under `psychological_feature.n.01`. It is the ancestor of the corpus's event nouns: `depletion` (n00356199) ⊑ `action` (n00037396) ⊑ `act` (n00030358) ⊑ `event` (n00029378).
- `state.n.02` (`wn:n00024720`, "the way something is with respect to its main attributes"), under `attribute.n.02`. It is the «state» in «a hypermutable state».

Since stative verbs take the argument too (next section), `event.n.01` alone would type the eventualities of «require» and «remain» as happenings.

**Decided (2026-10-06):**

- Verb eventualities range over a new class `lexicon:Eventuality ⊑ lexicon:Entity`, declared in the lexicon ontology.
- `wn:n00029378` and `wn:n00024720` become its subclasses. The importer emits the extra parent the way it roots `entity.n.01` at `lexicon:Entity` (`push_noun` in `crates/eigenius-wordnet/src/convert.rs`, which already writes several parents for a class).
- Each verb's eventuality is a member of the shared class, not of a per-verb subclass.

«WRN depletion» and the event of depleting WRN share one class space. Declaring `urn:schema_org:Action` equivalent to `wn:n00037396` through `core:EquivalentClasses` is a separate, later step, taken when role alignment needs D62's root, its advisory roles (agent, object, instrument, location, result, …) and its 14 subclasses inside the lattice. D99 §11 admits the equivalence for classes with equal required properties; Action only `recommends` its 12 properties.

The Element shows why disjoint domains fail: with eventualities and physical entities in disjoint domains, simple types cannot type event–object nouns such as *lunch* ([Chatzikyriakidis et al. 2025, p. 36](https://doi.org/10.1017/9781009285322)).

### All verbs, stative ones included, take events

*Decided (2026-10-06).* WordNet's lexicographer file 42, `verb.stative`, holds 756 of 13,767 verb synsets. A per-sense stative split would separate senses that the ledger treats as twins:

- «arise» v02624263 and v02625786 are in `verb.stative`;
- «arise» v00339738 is in `verb.change`;
- the maintainer ruled all three near-synonym twins on 2026-09-30.

Under that split the twins would differ in structure, so their skeletons would differ. That contradicts the premise of `kernel/src/dcg/skeleton.rs`: two readings that differ only in which sense fills a slot are the same structure.

The file is not a stativity test either: it holds the two «arise» senses, which are changes, while «stay», a state, is in `verb.change`. Where a sense is known to be stative, a later step may bind its argument at `state.n.02` rather than at `Ev`. A wrong label then gives the wrong class, never the wrong structure, so pins and skeletons do not depend on it. A PP that states a stative claim's scope («These mutations occur in nucleotide repeat regions.») gets the frame reading beside the condition on the state.

## Adjuncts ride the continuation to every event they modify

### PP adjuncts and negation

An intersective adjunct extends `K`, so its condition lands under the verb's `∃e`, whichever operator it attaches above. «X did not respond to ICB in MSI models» yields `¬∃e. And(respond_to(icb, x, e), prep_in(e, m))` whether `in MSI models` attaches below or above `not`. This is Schwarzschild's `not(rain heavily) ≡ (not rain) heavily` ([Schwarzschild 2014, pp. 3–4](http://web.mit.edu/schild/www/papers/public_html/champ.pdf)). The two attachments print one term, so they add no skeleton. As a frame, the same PP scopes above the negation instead: `frame_in(¬∃e. respond_to(icb, x, e), m)`, "in MSI models, X did not respond to ICB".

Downward-monotone contexts reverse modifier drop, as they should: «Nobody stabbed Caesar ⊨ Nobody stabbed Caesar with a sword» ([Beaver & Condoravdi 2007, p. 5](https://platform.openjournals.nl/PAC/article/download/22750/24310/56165)).

Champollion's *for*-adverbial, a quantifier over times placed outside `V`, is the pattern for adjuncts that must scope above negation. de Groote & Winter give quantified adverbials such as *everyday* and *in every room* the same shape ([de Groote & Winter 2015, §3](https://members.loria.fr/PdeGroote/papers/lenls14.pdf)).

### Adverbs

D62's routing stands. Inert adverbs are the identity, and measurement adverbs go to justification logic (`docs/notes/d62-adverb-semantics-decision.md` §4). Events give a manner adverb, should one ever be routed, the same intersective shape as a PP. Intentional and modal adverbs (*deliberately*) are not event predicates in any source read:

- Davidson treats intention as intensional ([Davidson 1967, p. 50](https://terpconnect.umd.edu/~pietro/fall2020e/LogicalFormOfActionSentences.pdf));
- de Groote & Winter treat modal adverbs as "orthogonal to the main tenets of event semantics" ([de Groote & Winter 2015, §3](https://members.loria.fr/PdeGroote/papers/lenls14.pdf));
- the ERG gives scopal adverbs a handle argument ([Copestake et al. 2005, §6.1.2](https://www.cl.cam.ac.uk/~aac10/papers/mrs.pdf)).

### Purpose PPs: «Scientists can exploit synthetic lethality for cancer therapeutics.»

The sentence gets two readings that differ in truth conditions:

- **Pinned NP attachment:** `Possible(∃e. exploit(kind_of(Σz:SL. prep_for(z, T)), S, e))`.
- **VP attachment:** `Possible(∃e. And(exploit(SL, S, e), prep_for(e, T)))`. Attaching *for* to the base VP below `can` or to the finite VP above it gives this one term, because `K` passes into `Possible`.

Here `T` is `kind_of(Σy:§. compound_kind(y, §))`, `SL` is synthetic lethality and `S` is scientists.

The Davidson/CGEL objection that purpose is not an extensional participant relation does not bite here. The PP's object is a kind term (Chierchia's ∩), so neither reading entails that a therapeutic exists, and `prep_for` is opaque and institution-mapped (D63 §8.13). Others do the same:

- AMR writes purpose as `:purpose` on the event ([AMR 1.2 guidelines](https://github.com/amrisi/amr-guidelines/blob/master/amr.md));
- PropBank uses ARGM-PRP ([Bonial et al. 2015, §1.4.10](https://github.com/propbank/propbank-documentation/blob/master/annotation-guidelines/Propbank-Annotation-Guidelines.pdf));
- PropBank's own `respond.01` example annotates a following *for*-PP inside ARG1 ([respond.xml](https://github.com/propbank/propbank-frames/blob/main/frames/respond.xml)).

The attachment stays the maintainer's call (U1 in `docs/notes/d69-structure-call-errors.md`).

### Locatives over coordinated events: «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.»

Today's pin is `And(And(promote(apo, d), promote(cca, d)), prep_in(d, m))`, with the locative on the depletion. Champollion's generalised conjunction of event quantifiers shares `K` across the conjuncts and gives each its own event ([Champollion 2015, pp. 51–52](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)). The result is `And(∃e. And(promote(apo, d, e), prep_in(e, m)), ∃e′. And(promote(cca, d, e′), prep_in(e′, m)))`. The locative is printed twice. A PP attached inside the second conjunct of a VP coordination restricts only `e′`, a truth-conditional difference that the forest carries. As a frame, «in MSI models» wraps the coordination once: `frame_in(And(∃e. promote(apo, d, e), ∃e′. promote(cca, d, e′)), m)`. The frame is the best reading by the rule decided for open question 8 (i): the page does not assert the effects outside MSI models.

Two cases stay uncovered:

- a collective reading, one event with a summed theme ([Champollion 2015, pp. 42–43](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf));
- measure adjuncts over coordinations («within 48 h»), which need a sum event that the 2015 system lacks ([Champollion, Bernard & Bledin 2022, Day 4, slides 6–9](https://lingbuzz.net/lingbuzz/006764/current.pdf)).

A quantified PP object should scope over the event quantifier, as de Groote & Winter's *everyday* does: `∀x. day x → ∃e. kissed e j m ∧ time e x` ([de Groote & Winter 2015, §3](https://members.loria.fr/PdeGroote/papers/lenls14.pdf)). ccg2lambda's VP-modifying preposition template puts it inside instead ([ccg2lambda, l. 837–839](https://github.com/mynlp/ccg2lambda/blob/master/en/semantic_templates_en_event.yaml)).

### Frames: a PP can restrict the whole claim

*Decided (2026-10-06).* On the WRN page, most PPs on state predicates give the population, model or method within which a finding holds: «in MSI models», «in cancers with deficiencies in homologous recombination», «with PCR-based MSI classifications». Maienborn reads locatives with statives and copular predicates as "frame adverbials" that restrict the proposition ([Maienborn 2011, pp. 819–822](https://ub01.uni-tuebingen.de/xmlui/bitstream/handle/10900/47121/pdf/Maienborn_2011_Event_semantics.pdf?sequence=1&isAllowed=y)). CGEL calls them domain adjuncts, which "restrict the domain to which the rest of the clause applies" (CGEL, pp. 765–766):

- **Realisation.** Adverbs (*economically*), dedicated PPs (*from a … point of view*, *as far as … is concerned*, *as regards*, *regarding*, *with respect to*), and "adjuncts of spatial location and a narrow range of conditional constructions" that "simultaneously serve to restrict the domain".
- **The test.** Omitting *in this country* from «In this country giving bribes to secure foreign contracts is permitted» "would result in a statement understood to apply universally". This is the drop test of open question 8 (i).
- **Events too.** "The clause usually expresses a state, but occurrences are not excluded" (*From an economic perspective, we acted foolishly*).
- **Position.** Domain adjuncts "prefer front position and also accept end position" (CGEL, p. 580).

Neither encoding available before this decision says what such a PP says:

- Subject anchoring makes WRN "in" MSI models.
- An event condition locates a state that, on her analysis, has no location.
- For «We found that WRN was selectively essential in MSI models.» today's forest puts «in MSI models» on *we* or on *WRN*.

A frame attaches to a whole clause, clause-final (`S\S`) or fronted (`S/S`, «In MSI models, …»):

```
frame_in : Prop → Entity → Prop                 opaque; one per framing preposition
⟦in⟧ frame = λx.λS.λK. frame_in(S(K), x)
```

- **Shape.** A clause denotes `Mod → Prop`, and the frame is a function on it. It passes `K` into the clause, so adjuncts inside still reach their events, and wraps the result the way `Possible` does. It meets the lexicon test above.
- **Scope.** Above the subject quantifier, negation and modals. «The four other RecQ DNA helicases were not preferentially essential in MSI cell lines.» gets `frame_in(¬essential(H), L)`, with `H` the four helicases and `L` the MSI cell lines.
- **No modifier drop.** `frame_in` is kernel-uninterpreted, like the modal operators, so `frame_in(φ, x)` does not entail `φ`: «WRN is essential in MSI models» does not entail «WRN is essential». An event condition licenses the drop; the two readings differ in their entailments.
- **Grounding.** The structured encodings put the frame's object in an argument: the WRN chain has `onco:SelectiveViabilityDependence("WRN", "MSI")` (`experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl:870`), and D63 §8.13 maps «WRN depletion causes apoptosis in MSI» to `CausesApoptosis(WRN, MSI)`. The encoding institution maps `frame_*` onto that argument, as it supplies the `prep_*` relations (D62).
- **Vocabulary.** The `frame_*` axioms sit beside the `prep_*` axioms in `ontologies/ontology/ontology.esl`, a bootstrap ontology. Slice 2 declares `frame_in` and `frame_with` (decided, open question 8 (ii)). Each later framing preposition adds one axiom and its entries; no rule and no existing entry changes, and the ledger rule of open question 8 (i) covers it unchanged.
- **Complements are not frames.** Nothing gives «essential», «dispensable», «successful» or «true» an *in* or *with*. SPECIALIST does govern «essential for» and «concordant with», and D97 slice 3a reads a governed adjective preposition as the adjective's relational argument, which frames leave alone.
- **Embedded clauses** (decided, open question 8 (iii)). A «that»-complement is a full clause (`S[dcl]`), so the clause-level frames attach to it as they are: «We found that WRN was selectively essential in MSI models.» gets `∃e. find(frame_in(essential(WRN), m), we, e)`. A relative clause's body is a VP missing its subject (`S[dcl]\NP`), which a clause-level frame cannot reach, so «in» and «with» also get a VP-level frame on finite VPs, `λx.λV.λs.λK. frame_in(V(s)(K), x)`. «… genes that are selectively essential in cancer cells with MSI.» then gets `ΣG:Gene. frame_in(essential(G), c)`; every other attachment of that PP («genes», «data sets», «analysed», a main-clause frame) misplaces it.
- **The VP-level frame elsewhere.** It also attaches to main-clause VPs. With a name, kind or definite subject it prints the same term as the clause-level frame, so it adds no skeleton; with a quantified subject it adds a frame under the quantifier. No clause-final «in» or «with» PP on the page has a quantified subject. It wraps a copular VP from outside instead of extending `K`, so nothing is lost when the copula ignores `K`.
- **Which clause.** Where a PP could frame more than one clause, the frame restricts the smallest clause whose unrestricted content the paper would not assert, and all of that clause, CGEL's "the domain to which the rest of the clause applies". «WRN was selectively essential» is not asserted unrestricted, so the «found» sentence's frame goes on the complement; a main-clause frame sits above a quantified subject.

On event verbs the forest offers both readings of a clause-final PP. The ledger rules by what the PP does (decided, open question 8 (i)), testing whether the paper would assert the clause without it:

- «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.»: no, the page also says «WRN was dispensable in models of microsatellite-stable cancers.» The frame is best.
- «These findings remained true with PCR-based MSI classifications.»: no. The frame is best.
- «Project Achilles screened cell lines with a CRISPR library.», «We ascertained MSI status with sequencing.» and «These mutations occur in nucleotide repeat regions.»: yes. The event condition is best.

Open question 8 lists what the frame reading still leaves to decide.

### Copular predicates keep no event; their PPs are frames

*Decided (2026-10-06), open question 2 → (c).* Maienborn finds that statives and "all copular predicates" fail every event diagnostic ([Maienborn 2011, pp. 819–822](https://ub01.uni-tuebingen.de/xmlui/bitstream/handle/10900/47121/pdf/Maienborn_2011_Event_semantics.pdf?sequence=1&isAllowed=y)). Katz keeps statives without a Davidsonian argument; "once existential closure has applied … state sentences and event sentences are of the same logical type" ([Katz 2000, §4.1](https://zaspil.leibniz-zas.de/article/download/44/43)).

The design keeps the categorial split this needs. `denote_cat` erases the Fin feature today (`kernel/src/dcg/category.rs:41`). One branch makes `⟦S[dcl,adj]⟧ = Prop` while every other `S[dcl,_]` becomes `Mod → Prop`. The copula applies the adjective's predicate to the subject and passes `K` nothing: `λP.λs.λK. P(s)`. As a result:

- the 28 closed-class entries whose categories mention only `S[adj]` keep their types;
- the imported adjectives keep their `gt(deg_a…(x), std_a…)` terms, and D97's relational readings keep theirs;
- a PP on a copular clause reads as a frame, no longer as `prep_in(subject, x)`, CGEL's "location of theme" (CGEL, pp. 680–682).

Verbal and copular VPs still share one type and coordinate («is essential and promotes …»). A VP adjunct on such a coordination conditions the verbal conjunct's event only.

Because the copula discards `K`, a VP adjunct on a copular VP would drop out of the term: «WRN was dispensable in models of microsatellite-stable cancers.» would get a reading with no «in». The copula's VP is `S[dcl,fin]\NP`, the same category as a finite verb's (`is_copula` in `closed-class.esl`), and each preposition's six VP-adjunct entries accept it. The grammar has to stop deriving that attachment. The VP-level frame is not affected: it wraps the copular VP instead of extending `K`.

*Decided (2026-10-06), open question 9:* an eventivity feature on `S`, `cat_s(mood, fin, evt)` with `evt : lexicon:Eventivity = eventive | eventless`, erased by ⟦·⟧ like the other syntactic features.

- Verbs build `S[dcl,f,eventive]\NP`; the copula builds `S[dcl,fin,eventless]\NP`.
- The VP-adjunct entries select an eventive VP.
- Entries that pass a VP through bind the feature as a variable, as `cat_fin_forall` binds Fin: modals, negation, «do», «to», the adverb modifiers and the VP-level frame. An adjunct above a modal over a copular VP is then refused too.
- Entries that close a clause accept either: the complementiser, the relativiser, the root and the clause-level frames.
- A coordination is eventive only if every conjunct is. «is essential and promotes …» still parses, and a VP adjunct attaches inside the verbal conjunct, which prints the term an attachment to the coordination would. The page has no VP coordination.

This states Katz's stative adverb gap in the grammar for copular predicates: an event modifier selects a VP that has an event. Stative verbs take an eventuality (question 3), so their VPs are eventive and the gap for them («resembled her mother slowly») is not stated. A stativity source could state it later on the argument's class (`state.n.02`), leaving structure alone.

| Current adjunct pin (15) | Host | Under the proposal |
|---|---|---|
| «…screened cell lines with a CRISPR library.», «…analysed cell lines with an RNA interference library.», «We ascertained MSI status with sequencing.», «These mutations occur in nucleotide repeat regions.», «These findings remained true with PCR-based MSI classifications.», «The MSI relationship compared favourably to other strong biomarkers for vulnerabilities.» (the «to» PP), «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.» | verb (7). #270 counts eight because it adds «… dependency in MSI cell lines compared to MSS cell lines», but that PP precedes «compared» and modifies the noun «dependency». | PP moves from the subject to the event, with a frame reading beside it. By the rule of open question 8 (i), the frame is best for «remained true with …» and «… in MSI models», the event condition for the three instruments and «occur in». Re-adjudicate |
| «WRN was dispensable in models of microsatellite-stable cancers.», «These classifications were highly concordant with …», «The four other RecQ DNA helicases were not preferentially essential in MSI cell lines.», «MSI is most commonly observed in …», «We found that WRN was selectively essential in MSI models.», «PARP-1 inhibitors are successful in cancers with …», «These libraries define genes that were essential for proliferation and survival.» and «We analysed these data sets for genes that are selectively essential in cancer cells with MSI.» (PP on the relative clause's predicate) | copular/adjectival (8) | PP becomes a frame, or the adjective's relational argument where SPECIALIST governs the preposition («concordant with», «essential for»). «… genes that are selectively essential in cancer cells with MSI» takes the VP-level frame inside its relative clause (open question 8 (iii)). Re-adjudicate |

## The type change is atomic and its cost is countable

### No partial path introduces events only where adjuncts appear

**Per occurrence.** Introducing an event only on modified verb occurrences would give each verb two logical forms. Each form would need a bridging axiom (`∃e. v(o,s,e) ↔ v(o,s)`) to recover modifier drop. It would also duplicate every VP-taking closed-class entry at both types, since adjuncts attach above negation, modals and coordination.

- None of ERG, Boxer/PMB, ccg2lambda or AMR does this.
- AMR's optional reification is the nearest analogue. It scores two equivalent gold standards 73.9 points apart ([Opitz 2023, §5.2](https://aclanthology.org/2023.findings-eacl.118.pdf)).

**Per sense.** A per-sense stative split fails on the «arise» twins (see "All verbs, stative ones included, take events").

**Per category.** Events on verbs and none on copular predicates is the one split that keeps the skeleton sense-independent, and the proposal adopts it.

`⟦S[dcl,_]⟧` changes in one place for all 316 closed-class entries whose categories mention a non-adjectival `S`. No intermediate state keeps the grammar type-correct.

### Cost by component

| Component | Change | Size (measured 2026-10-06) |
|---|---|---|
| Kernel | none: impredicative `∃` and class subsumption exist | 0 |
| Ontologies | `lexicon:Eventuality ⊑ lexicon:Entity`, and the importer's extra parent for `event.n.01` and `state.n.02`; one opaque `frame_*` axiom per framing preposition in `ontology.esl`. `lexicon-ontology.esl` and `ontology.esl` are bootstrap ontologies and the importer change re-imports WordNet, so the chain reseeds | 1 class, 2 parent edges, 2 frame axioms (`frame_in`, `frame_with`) |
| `denote_cat` | branch `⟦S[dcl,f]⟧` on `f = adj` | 1 match arm (`kernel/src/dcg/category.rs:41`) |
| Verb entries | `FrameKind::arrow` gains an event slot. The emitted verb sem wraps the axiom in `λ…λK. ∃e:Ev. K(λa. v(…, a))(e)`. D62 §5 calls this "a converter-rule change + reseed". | 8 frame tags in one converter; 13,767 WordNet verb synsets plus the SPECIALIST verbs of D97, re-emitted |
| Closed class | sem types of entries over non-adjectival `S`. Terms change for determiners (pointwise `K`), negation, modals, prepositions, the copula (it discards `K`) and the complementiser; `do`, passive `be` and `by_agent` change type only. New: six frame entries, «in» and «with» in `S\S`, in `S/S` and on finite VPs | 316 of 465 entries; 137 SemTerms to audit (`ontologies/lexicon/closed-class.esl`) |
| Engine rules | coordination folds pointwise over `K` and takes the meet of the eventivity feature; relativiser and root close with `λP.P` | 4 files, 7,010 lines, 58 connective sites (`kernel/src/dcg/rules/`) |
| Eventivity feature | `cat_s` gains `lexicon:Eventivity` (open question 9): the copula's VPs are eventless, verbs' eventive, VP adjuncts select eventive, VP-passing entries bind it | 676 `cat_s` occurrences in 344 closed-class entries; 19 importer emission sites; 12 Rust files that build or match `cat_s` |
| Verbaliser | `adjunct_of` matches the PP to the verb atom by the shared event variable instead of by the subject's printed string. Its adjective and copula branches lose their input; a frame rendering ("«in MSI models» says where the claim holds") replaces them | `kernel/src/dcg/verbalize.rs:1497–1569` |
| Forest | every clause-final «in» or «with» PP gets a frame reading beside its other attachments; a PP that ends a «that»-complement and its main clause gets one frame reading per clause; the VP-level frame adds a reading only under a quantified subject. The ranker chooses among more readings | 15 of 62 page sentences end in an «in» or «with» PP; growth derived, not measured |
| Pins | each verb adds `ΠG#k:Prop. ΠG#k+1:§.`, one argument and one `→ G#k → G#k`; later binders renumber | ≥51 of 62 pins reprint: the 48 pinned sentences whose ledger rows carry a verb atom, and 3 verbless copular adjunct pins. The 15 adjunct pins change meaning (7 verbal, 8 copular or adjectival); at least 36 reprint mechanically |
| Ledger | rows are keyed on the printed term, so every verb row is re-keyed. The D64 re-pin precedent was "transformed mechanically and checked against the new forest". | 213 of 228 rows re-keyed. 142 verb rows with no PP on the subject carry their verdict mechanically. 71 hang a PP on the subject and need re-adjudication: 46 on a verb (11 `correct`, 35 `wrong`, 14 of the `wrong` grounded on subject predication) and 25 on an adjective or copula (13 `correct`, 12 `wrong`; 20 of them in verbless rows, 1 inside a relative clause). The 15 verbless rows without such a PP are unchanged. |
| Canonical hashing | D47 stores binder names as strings, and readback names binders `G#<level>` (`kernel/src/nbe/readback.rs:392`), so α-variants already hash alike. Conjunct order is attachment order, first-attached innermost; the identity closure leaves no `True`. | every `enc:EncodedClaim` with a verb gets a new proposition and witness key; regenerate by drop-and-reseed |
| Term size | positional: +2 binders, +1 argument per verb. Neo-Davidsonian roles would add two role atoms per transitive verb on top; PMB gold has 7,516 role clauses beside 7,545 concept clauses ([van Noord et al. 2018, Table 1](https://aclanthology.org/L18-1267.pdf)) | derived, not measured |

## Recommendation

1. **Arguments: positional.** Adopt Davidsonian positional arguments plus one event argument, typed `lexicon:Entity` and bound by an impredicative `∃e:Ev` inside each verb's lexical entry. Governed PPs stay positional in `v{offset}_p_{prep}`. Do not adopt thematic-role predicates now.
2. **VP type.** Make the VP denotation take a predicate-transforming continuation over eventualities, `Mod := (Ev → Prop) → Ev → Prop`, closed by the identity at the parse root and at every embedding boundary.
3. **Adjuncts.** Intersective adjuncts extend `Mod` with an opaque `prep_*(a, x)` conjunct on the event. Negation, modals, GQs and frames pass `Mod` through. A test enforces this shape.
4. **Frames** (decided). A PP that says where a claim holds can attach to the clause as `frame_*(φ, x)`, above the subject quantifier, negation and modals.
5. **Event class** (decided). `lexicon:Eventuality ⊑ lexicon:Entity`, above `wn:n00029378` `event.n.01` and `wn:n00024720` `state.n.02`. Every verb takes the argument, stative ones included.
6. **Copular predicates** (decided) keep no event; their PPs are frames.
7. **Deferred:** dependent event types, per-verb event classes, sum events, Kimian states and `schema_org:Action ≡ wn:n00037396`. Each waits for a task that needs it.

## Implementation path and cost

**Slice 1 (vocabulary, no grammar change).**

- Declare `lexicon:Eventuality ⊑ lexicon:Entity` in the lexicon ontology, and have the WordNet importer give `wn:n00029378` and `wn:n00024720` it as an extra parent.
- Declare `frame_in` and `frame_with` beside the `prep_*` axioms in `ontology.esl`.
- Witnesses: `is_subclass_of(wn:n00029378, lexicon:Eventuality)`, `is_subclass_of(wn:n00024720, lexicon:Eventuality)` and `is_subclass_of(lexicon:Eventuality, lexicon:Entity)`.
- Cost: one class, two parent edges, the frame axioms and a test. Both ontologies are in the bootstrap, so the slice reseeds; landing it with slice 2 shares that reseed.

**Slice 2 (the type change, one PR, since no subset type-checks).**

- the `denote_cat` branch;
- the converter's event slot and verb sem;
- the 316 closed-class types and the VP-touching terms;
- the six frame entries;
- the copula's sem, and the eventivity feature on `cat_s` that keeps VP adjuncts off copular VPs (open question 9);
- the coordination, relativiser and root-closure rules;
- `adjunct_of` keyed on the event variable, and the frame rendering;
- the Charlow-invariant lexicon test;
- a reseed (`scripts/reseed-lexicon-db.sh`).

The same PR transforms the pins and ledger mechanically:

- at least 36 verb pins without an adjunct PP;
- 142 ledger rows.

It also brings the 15 adjunct pins and 71 ledger rows to the maintainer as a re-adjudication batch, because the parse gates fail on stale pins otherwise.

**Slice 3 (persisted claims).** Drop and reseed the encoded artefacts. Every verb-bearing claim gets a new D47 term and witness key.

**Slice 4 (CGEL's domain adjuncts, any time after slice 2).** A planned augmentation, decided with open question 8 (ii):

- the other spatial-location prepositions. «within», «among», «on» and «at» have entries and `prep_*` axioms today; «across» and «under» have argument-marker entries only (D97 decision 3); «throughout» has none;
- the dedicated domain PPs with a fixed form, «with respect to», «as regards» and «regarding», as multiword closed-class forms, as «at least» and «more than» are; «regarding» exists today only as an argument marker;
- «from a … point of view» and «as far as … (is concerned)», which take a phrase inside the frame and need entries of their own.

Each preposition adds one `frame_*` axiom, its entries and a frame reading on every clause-final PP it heads.

**Later, each its own decision:**

- thematic roles for knowledge-graph alignment, taking D62's fork (role axioms vs property-as-relation);
- `schema_org:Action ≡ wn:n00037396`, when role alignment needs D62's root in the lattice;
- a stativity source, to bind known-stative senses at `state.n.02`;
- CGEL's other realisations of domain adjuncts, adverbs («Economically, …») and participials («economically speaking»). A productive -ly adverb seeds identity entries today (D62 Phase 3, `adverb_modifier_cats` in `kernel/src/dcg/category.rs`), so where one attaches, its restriction is lost;
- Kimian states, if state anaphora is needed. The eventivity feature already states the stative adverb gap for copular predicates;
- the stative adverb gap for stative verbs, on the argument's class once a stativity source exists;
- sum events for measure adjuncts;
- the generic reading of present-tense negation.

## Open questions for the maintainer

**1. The event class.** *Decided 2026-10-06: (b).* The options were:

- (a) bind at `wn:n00029378` and declare `schema_org:Action ≡ wn:n00037396`. With stative verbs taking the argument, this types their eventualities as happenings.
- (b) mint a `lexicon:Eventuality` above `wn:n00029378` and `wn:n00024720`. **Chosen.** The Action equivalence becomes a later step.
- (c) bind at `urn:schema_org:Action` as D62 named it. This still requires placing Action under `lexicon:Entity`, and it types the events of `arise`, `result` and `occur` as agentive.

`urn:schema_org:Event` is not a candidate: it is "an event happening at a certain time and location, such as a concert, lecture, or festival", its 20 subclasses are of that kind (`Festival`, `SportsEvent`, `ScreeningEvent`), and like Action it sits under the parentless `schema_org:Thing`, not `lexicon:Entity` (D62 §5: "scheduled happenings"). If a task imports schema.org-typed records, `schema_org:Event ⊑ wn:n00029378` is the sound link; #270 needs none.

**2. Copular predicates.** *Decided 2026-10-06: (c), with the frame reading available on every clause, verbal or copular.* The options were:

- (a) keep them eventless with subject-anchored adjuncts. This makes WRN "in" MSI models.
- (b) introduce a Kimian state at the copula, which makes eventualities uniform and lets typing enforce the stative adverb gap, at the cost of a state binder in every copular clause. A PP would then condition a state that Maienborn says has no location.
- (c) read their PPs as proposition-level frame restrictors. **Chosen.**

**3. Stative verbs.** *Decided 2026-10-06: all verb synsets take the argument,* including the 756 in `verb.stative`, because of the «arise» twins. The importer does not start splitting by `lex_filenum`, which `Synset` drops today (`crates/eigenius-wordnet/src/wndb.rs:84–126`). A stativity source, if one is adopted later, refines the argument's class only.

**4. Governed prepositions.** Options:

- (a) keep the free-adjunct parse of a preposition that the lexicon places on the verb sense in the forest, and rule it `wrong` on `structure` (recommended; D63 carries attachment ambiguity);
- (b) suppress it by a normal-form rule.

Either way the 14 rows grounded on "predicates … of the subject" need new evidence text.

**5. Purpose *for*.** Rule U1's attachment. Then decide whether purpose *for* stays the one opaque `prep_for` it shares with benefactive and «essential for», or becomes a distinct relation.

**6. Coordination and quantified PP objects.**

- Should the distributive two-event reading of coordinated objects be the only one until sum events exist?
- Should quantified PP objects scope over the event quantifier (de Groote & Winter, Champollion) rather than under it (ccg2lambda)?

**7. Negation in present-tense scientific claims.** «Some cancers do not respond …» comes out as `¬∃e`, meaning no responding event occurs. A generic or dispositional reading, with Gen binding `e` as in Kratzer and Diesing via Maienborn, is not covered by any source read.

**8. Frames.** The frame reading leaves three choices:

- (i) Which reading the ledger rules best when the event condition and the frame are both faithful, as in «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.» *Decided 2026-10-06: by what the PP does,* with the drop test (see "Decided so far"). Rejected: a per-preposition rule, which «with» defeats; always the frame, which loses the drop where it holds («Project Achilles screened cell lines»); always the event condition, which removes the domain restriction from verbal clauses.
- (ii) Which prepositions frame. *Decided 2026-10-06: «in» and «with» now, CGEL's list as a planned augmentation (slice 4).* On the page, 15 of 62 sentences end in a PP run containing «in» or «with». By the drop test the frame is best in 8 (7 «in», 1 «with»), the event condition in 5, the governed reading in 1 («concordant with») and the noun-internal reading in 1. No page sentence needs a «for» frame: in «a promising drug target for MSI cancers» the frame and the noun-internal reading both restrict the claim. Rejected: frames for all 14 prepositions that have a finite VP-adjunct entry today, which would add frame readings to about 45 of the 62 sentences (hand count), mostly for «from», «to» and «for».
- (iii) Whether frames attach inside embedded clauses. *Decided 2026-10-06: main clauses, «that»-complements and, through a VP-level frame, relative clauses; the frame restricts the smallest clause whose unrestricted content the paper would not assert, and all of it* (see "Frames"). Rejected: main and complement clauses only, which leaves «… genes that are selectively essential in cancer cells with MSI» without a faithful reading; main clauses only, which also loses the «found» sentence's best reading and needs a marking to keep frames out of complements.

**9. Copular VPs and VP adjuncts.** The copula discards `K`, so a VP adjunct on a copular VP must not be derivable (see "Copular predicates keep no event; their PPs are frames"). The copular VP and the finite verbal VP share the category `S[dcl,fin]\NP`. *Decided 2026-10-06: (a), an eventivity feature on `S`.* The options were:

- (a) a third feature on `cat_s`, eventive or eventless, which VP adjuncts select. **Chosen.** It follows the grammar's separate syntactic features (Fin, Num, Prep, Mode) and keeps the feature meet a lattice.
- (b) a copular Fin value `cop`. Accepting it wherever a finite clause or VP is selected needs a pair `cop ⊑ fin` in the meet, which lets the VP adjuncts' `fin` accept it too; refusing it there needs a further value only verbs meet, and the meet stops being a lattice.
- (c) a filter that refuses a VP adjunct whose condition is missing from the result: a guard, since the categories would still license the attachment.
- (d) a separate type for eventless VPs, which needs (a)'s distinction for `denote_cat` to branch on and then both types on every VP-taking entry.

## Sources

Opened and read, per the research notes. "Via" marks secondary access.

- Davidson, D. 1967. "The logical form of action sentences." In N. Rescher (ed.), *The Logic of Decision and Action*, 81–95. Read in the reprint *The Essential Davidson*, pp. 37–59, which includes "Criticism, Comment, and Defence". https://terpconnect.umd.edu/~pietro/fall2020e/LogicalFormOfActionSentences.pdf
- Kratzer, A. *The Event Argument and the Semantics of Verbs*, ch. 1 (Dec. 2002 draft) and ch. 2 (Aug. 2000 draft). Semantics Archive GU1NWM4Z.
- Kratzer, A. 2000. "Building statives." *BLS 26*. https://semanticsarchive.net/Archive/GI5MmI0M/kratzer.building.statives.pdf
- Kratzer, A. 1996. "Severing the external argument from its verb." Via Stranahan, L. 2012, handout. https://www.lainestranahan.com/wp-content/uploads/2012/11/Kratzer_handout_public_2.pdf
- Maienborn, C. 2011. "Event semantics." In Maienborn, von Heusinger & Portner (eds.), *Semantics* (HSK 33.1), 802–829. de Gruyter. This is also the secondary source for Parsons 1990/2000, Higginbotham, Chierchia and Kratzer 1995.
- Williams, A. "Events in semantics," ch. 20, galley 2020. https://linguistics.umd.edu/sites/default/files/2020-10/web-linguistics-williams-20-eventsinsemantics-galley.pdf
- Dowty, D. 1991. "Thematic proto-roles and argument selection." *Language* 67(3): 547–619. Only pp. 572 and 576 were read.
- Schütze, C. T. & E. Gibson. 1999. "Argumenthood and English prepositional phrase attachment." *Journal of Memory and Language* 40: 409–431.
- EAGLES. 1996. *Linguistic Aspects of Lexical Organization*, §2.9.2. https://www.ilc.cnr.it/EAGLES96/rep2/node13.html
- Huddleston, R. & G. K. Pullum et al. 2002. *The Cambridge Grammar of the English Language* (CGEL). Cambridge UP. Local: `references/publications/the-cambridge-grammar-of-the-english-language.pdf`.
- Casati, R. & A. Varzi. "Events." *SEP*, rev. 12 May 2025. https://plato.stanford.edu/entries/events/
- Champollion, L. 2015. "The interaction of compositional semantics and event semantics." *Linguistics and Philosophy* 38: 31–66. DOI 10.1007/s10988-014-9162-8. This is also the secondary source for Krifka 1989, Landman, Parsons 1995 and Diesing.
- Champollion, L. 2011. "Quantification and negation in event semantics." *Baltic International Yearbook* 6: 1–23.
- Winter, Y. & J. Zwarts. 2011. "Event semantics and Abstract Categorial Grammar." *MOL 12*, LNCS 6878, 174–191.
- de Groote, Ph. & Y. Winter. 2015. "A type-logical account of quantification in event semantics." *New Frontiers in AI*, LNCS 9067, 53–65. DOI 10.1007/978-3-662-48119-6_5. Read as the author preprint.
- Schwarzschild, R. 2014. "Distributivity, negation and quantification in event semantics: Recent work by L. Champollion." Ms.
- Charlow, S. "Scope in event semantics." Semantics II slides, Feb. 19 (2018 per URL).
- Beaver, D. & C. Condoravdi. 2007. "On the logic of verbal modification." *Proc. 16th Amsterdam Colloquium*, 3–9.
- Bernard, T. & L. Champollion. 2018. "Negative events." SALT 28 abstract. Their 2023 *Journal of Semantics* 40(4): 585–620 paper was read only as a model summary of the HTML.
- Champollion, L., T. Bernard & J. Bledin. 2022. ESSLLI course slides, Day 4.
- Mineshima, K., P. Martínez-Gómez, Y. Miyao & D. Bekki. 2015. *EMNLP*, 2055–2061.
- Mineshima, K., R. Tanaka, P. Martínez-Gómez, Y. Miyao & D. Bekki. 2016. *EMNLP*, 2236–2242.
- Martínez-Gómez, P. et al. 2016. "ccg2lambda." *ACL System Demonstrations*, 85–90.
- Martínez-Gómez, P. et al. 2017. *EACL*, 710–720.
- Haruta, I., K. Mineshima & D. Bekki. 2020. *COLING*, 1758–1764.
- ccg2lambda repository, `en/semantic_templates_en_event.yaml` and `en/semantic_templates_en_emnlp2015.yaml`. Fetched 2026-10-06.
- Lewis, M. & M. Steedman. 2013. "Combined distributional and logical semantics." *TACL* 1: 179–192.
- Blom, C., Ph. de Groote, Y. Winter & J. Zwarts. 2012. *Amsterdam Colloquium 2011*, LNCS 7218, 240–250.
- Tomita, A. 2025. ESSLLI slides on CCG + DTS inference.
- lightblue source, local: `references/lightblue/`.
- Luo, Z. & S. Soloviev. 2017. "Dependent Event Types." *WoLLIC 2017*, LNCS 10388, 216–228. DOI 10.1007/978-3-662-55386-2_15.
- Chatzikyriakidis, S. & Z. Luo. 2017. "Adjectival and Adverbial Modification: The View from Modern Type Theories." *JoLLI*. DOI 10.1007/s10849-017-9246-2.
- Chatzikyriakidis, S. & Z. Luo. 2017. "On the Interpretation of Common Nouns: Types Versus Predicates." In *Modern Perspectives in Type-Theoretical Semantics*, Springer.
- Chatzikyriakidis, S. & Z. Luo. 2020. *Formal Semantics in Modern Type Theories*, Appendix 7 only. DOI 10.1002/9781119489252.app7.
- Luo, Z. 2012. "Formal semantics in modern type theories with coercive subtyping." *Linguistics and Philosophy* 35(6): 491–513.
- Asher, N. & Z. Luo. 2012. *Sinn und Bedeutung 17* abstract.
- Luo, Z. 2018. LACompLing slides.
- Luo, Z. 2023. ESSLLI Lecture II slides.
- Luo, Z. & Y. Shi. 2026. "Variable polyadicity without events." *MSCS* 36, e11. doi:10.1017/S0960129526100504.
- Cooper, R. 2023. *From Perception to Communication: A Theory of Types for Action and Meaning*. Oxford UP (open access).
- Chatzikyriakidis, S., R. Cooper, E. Gregoromichelaki & P. R. Sutton. 2025. *Types and the Structure of Meaning: Issues in Compositional and Lexical Semantics*. Cambridge Elements. DOI 10.1017/9781009285322.
- Copestake, A., D. Flickinger, C. Pollard & I. A. Sag. 2005. "Minimal Recursion Semantics: An Introduction." *Research on Language and Computation* 3(2–3): 281–332.
- DELPH-IN ERG Semantic Documentation (ErgSemantics_Basics, _Design, _Essence, PredicateRfc). Accessed 2026-10-06.
- Bos, J. 2008. "Wide-coverage semantic analysis with Boxer." *STEP 2008*, 277–286.
- Abzianidze, L. et al. 2017. "The Parallel Meaning Bank." *EACL* (short), 242–247.
- van Noord, R., L. Abzianidze, H. Haagsma & J. Bos. 2018. "Evaluating scoped meaning representations." *LREC*, 1685–1693.
- AMR 1.2 Guidelines, `amr.md`, amrisi/amr-guidelines.
- Bonial, C. et al. 2015. *English PropBank Annotation Guidelines*.
- PropBank frames, `respond.xml`.
- Katz, G. 2000. "A semantic account of the stative adverb gap." *ZAS Papers in Linguistics* 17: 135–151.
- Cai, S. & K. Knight. 2013. "Smatch." *ACL* (vol. 2), 748–752.
- Opitz, J. 2023. "SMATCH++." *Findings of EACL*, 1595–1607.

Not opened, so no claim above rests on them directly: Parsons 1990/1995; Castañeda 1967, beyond Davidson's replies; Krifka 1989; Landman 2000; Schein 1993; Steedman's monographs.
