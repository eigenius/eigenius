# Close verb events inside lexical entries

*Design-note draft for `docs/notes/`. It answers eigenius#270 after the maintainer chose option B, event semantics, on 2026-10-06. It proposes changes; it changes no code.*

**Recommendation.** Every verb gets one Davidsonian event argument, placed after its positional arguments. The subject, the object and any governed PP (`respond to`, `arise from`) stay positional in the verb's named relation. Adjunct PPs and adverbs become conjuncts on the event. The event quantifier closes inside the verb's own lexical entry, as Champollion (2015) proposes and as ccg2lambda and lightblue implement.

**Why lexical closure.** Of the solutions surveyed, lexical closure is the only one that keeps the event quantifier lowest using function application alone. A chart CCG without hypothetical reasoning needs exactly that. It puts `∃e` under every generalised quantifier, under `→ False` and inside `Possible`. So «Some cancers do not respond to immune checkpoint blockade.» becomes `some Cancer (λx. ¬∃e. respond_to(icb, x, e))`, with the scope order it has today.

**What is not needed.**

- *Neo-Davidsonian thematic roles.* None of the five questions needs them. Leaving them out avoids D62's role-relation fork, the role-inventory problem, and three atoms per transitive verb instead of one.
- *Dependent event types* (Luo & Soloviev). They need parameterised coercions that `Layer::is_subclass_of` does not have. They also solve a scope problem that lexical closure never creates. One event class is enough.

**Where the event class sits.** The class must sit under `lexicon:Entity`. `urn:schema_org:Action`, the root D62 chose, does not: it is a subclass of a parentless `schema_org:Thing`.

**Copular and adjectival predicates keep no event.** Their adjuncts stay anchored on the subject. That keeps the PP reading of 8 of the 15 current adjunct pins and of the 23 ledger rows whose PP hangs on an adjective.

**No partial rollout.** There is no path on which only adjuncts introduce an event. Every VP-taking closed-class entry shares the VP's type, so the change lands as one slice:

- one branch in `denote_cat`;
- the verb converter;
- the types of 316 of 465 closed-class entries;
- the engine's coordination rules;
- at least 48 of 63 pins;
- 193 of 228 ledger rows.

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

The event slot is typed `lexicon:Entity`, like every stage-1 verb slot (`crates/eigenius-wordnet/src/convert.rs:223–241`). Its sort comes from the binder `∃e:Ev`; the next section explains why the slot is not typed `Ev`.

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

The continuation ranges over `lexicon:Entity` rather than `Ev`. That lets copular predicates anchor their adjuncts on the subject (see "Copular predicates keep their current encoding" below). It is also why the verb's event slot is typed `Entity`.

```
Ev                 the event class, Ev ⊑ lexicon:Entity (open question 1)
Mod  := (Entity → Prop) → Entity → Prop
⟦S[dcl, f]⟧ := Mod → Prop   for f ≠ adj          was Prop
⟦S[dcl, adj]⟧ := Prop                             unchanged

⟦v_t⟧          = λo.λs.λK. ∃e:Ev. K(λa. v_t(o, s, a))(e)
⟦v_p_to⟧       = λy.λx.λK. ∃e:Ev. K(λa. v_p_to(y, x, a))(e)    governed PP: positional
⟦in⟧ VP adjunct = λx.λV.λs.λK. V(s)(λP. K(λa. And(P(a), prep_in(a, x))))
⟦not⟧          = λV.λs.λK. V(s)(K) → logic:False                the current neg_sem, K passed in
⟦can⟧          = λV.λs.λK. logic:Possible(V(s)(K))
⟦be⟧ + adj     = λP.λs.λK. K(P)(s)                              anchor = subject; no event
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

The narrow scope of `∃e` holds only if no entry puts a quantifier, a negation or a modal inside `K`. The test: every closed-class entry that consumes a VP `V` must do one of two things.

1. Pass `K` through unchanged.
2. Extend it only as `λP. K(λa. And(P(a), φ(a)))`, where `φ` is an atomic relation.

GQs, `→ False` and `Possible` may wrap `V(s)(K)` from outside. Like `reading_ledger_is_consistent`, the test needs no parse. It is the lexicon property Charlow says the narrow scope rests on, and it also guarantees the modifier-drop entailments that Qv→Qv typing alone does not ([Charlow, slide 23](https://schar.github.io/sem2/files/notes/02-19-18.pdf)).

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

**Per-verb event classes are possible but not needed now.** TTR-style classes (`Hugging ⊑ Touching`) could be declared, since a resource may already inhabit several classes (`check/mod.rs:832–846`). They would put WordNet verb hypernymy into the subclass lattice. The Σ-refinement Eigenius already uses for nouns expresses a verb-specific event type when a task needs one: `Σe:Ev. v_t(o, s, e)`, after [Luo 2012, p. 495](https://www.cs.rhul.ac.uk/home/zhaohui/LP13.pdf).

### Where the class sits in the lattice

D62 compared `schema:Action` against `schema:Event`, which covers scheduled happenings, and chose Action. Two facts it did not weigh:

1. **`urn:schema_org:Action` is not an Entity.** It is a subclass of `urn:schema_org:Thing`, which has no parent (`ontologies/schema-org/schema-org.eigon.json`). An Action-typed event therefore cannot fill `prep_in : Entity → Entity → Prop`.
2. **Its definition presumes an agent.** It begins "An action performed by a direct agent". The corpus verbs `arise`, `result` and `occur` are not agentive.

WordNet already has the right node. `event.n.01` (`wn:n00029378`, "something that happens at a given place and time") lies under `entity.n.01`, which the importer roots at `lexicon:Entity`. It is also the ancestor of the corpus's event nouns: `depletion` (n00356199) ⊑ `action` (n00037396) ⊑ `act` (n00030358) ⊑ `event` (n00029378) (`references/WordNet-3.0/dict/data.noun`).

The proposal:

- Bind verb events at `wn:n00029378`.
- Declare `urn:schema_org:Action` equivalent to `wn:n00037396` through `core:EquivalentClasses`. D99 §11 admits equivalence for classes with equal required properties; Action only `recommends` its 12 properties.

D62's root then keeps its advisory roles (agent, object, instrument, location, result, …) and its 14 subclasses inside the lattice. «WRN depletion» and the event of depleting WRN share one class space. The Element shows why disjoint domains fail: with eventualities and physical entities in disjoint domains, simple types cannot type event–object nouns such as *lunch* ([Chatzikyriakidis et al. 2025, p. 36](https://doi.org/10.1017/9781009285322)).

### All verbs, stative ones included, take events

WordNet's lexicographer file 42, `verb.stative`, holds 756 of 13,767 verb synsets. A per-sense stative split would separate senses that the ledger treats as twins:

- «arise» v02624263 and v02625786 are in `verb.stative`;
- «arise» v00339738 is in `verb.change`;
- the maintainer ruled all three near-synonym twins on 2026-09-30.

Under that split the twins would differ in structure, so their skeletons would differ. That contradicts the premise of `kernel/src/dcg/skeleton.rs`: two readings that differ only in which sense fills a slot are the same structure.

## Adjuncts ride the continuation to every event they modify

### PP adjuncts and negation

An intersective adjunct extends `K`, so its condition lands under the verb's `∃e`, whichever operator it attaches above. «X did not respond to ICB in MSI models» yields `¬∃e. And(respond_to(icb, x, e), prep_in(e, m))` whether `in MSI models` attaches below or above `not`. This is Schwarzschild's `not(rain heavily) ≡ (not rain) heavily` ([Schwarzschild 2014, pp. 3–4](http://web.mit.edu/schild/www/papers/public_html/champ.pdf)). The two attachments print one term, so they add no skeleton.

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

Today's pin is `And(And(promote(apo, d), promote(cca, d)), prep_in(d, m))`, with the locative on the depletion. Champollion's generalised conjunction of event quantifiers shares `K` across the conjuncts and gives each its own event ([Champollion 2015, pp. 51–52](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf)). The result is `And(∃e. And(promote(apo, d, e), prep_in(e, m)), ∃e′. And(promote(cca, d, e′), prep_in(e′, m)))`. The locative is printed twice. A PP attached inside the second conjunct of a VP coordination restricts only `e′`, a truth-conditional difference that the forest carries.

Two cases stay uncovered:

- a collective reading, one event with a summed theme ([Champollion 2015, pp. 42–43](https://champollion.com/wp-content/uploads/2018/06/2015-interaction-paper.pdf));
- measure adjuncts over coordinations («within 48 h»), which need a sum event that the 2015 system lacks ([Champollion, Bernard & Bledin 2022, Day 4, slides 6–9](https://lingbuzz.net/lingbuzz/006764/current.pdf)).

A quantified PP object should scope over the event quantifier, as de Groote & Winter's *everyday* does: `∀x. day x → ∃e. kissed e j m ∧ time e x` ([de Groote & Winter 2015, §3](https://members.loria.fr/PdeGroote/papers/lenls14.pdf)). ccg2lambda's VP-modifying preposition template puts it inside instead ([ccg2lambda, l. 837–839](https://github.com/mynlp/ccg2lambda/blob/master/en/semantic_templates_en_event.yaml)).

### Copular predicates keep their current encoding

Maienborn finds that statives and "all copular predicates" fail every event diagnostic. Locatives with them act as "frame adverbials" that restrict the proposition ([Maienborn 2011, pp. 819–822](https://ub01.uni-tuebingen.de/xmlui/bitstream/handle/10900/47121/pdf/Maienborn_2011_Event_semantics.pdf?sequence=1&isAllowed=y)). Katz keeps statives without a Davidsonian argument; "once existential closure has applied … state sentences and event sentences are of the same logical type" ([Katz 2000, §4.1](https://zaspil.leibniz-zas.de/article/download/44/43)).

The design keeps the categorial split this needs. `denote_cat` erases the Fin feature today (`kernel/src/dcg/category.rs:41`). One branch makes `⟦S[dcl,adj]⟧ = Prop` while every other `S[dcl,_]` becomes `Mod → Prop`. The copula then passes the adjective's predicate to `K` with the subject as anchor. As a result:

- the 28 closed-class entries whose categories mention only `S[adj]` keep their types;
- the imported adjectives keep their `gt(deg_a…(x), std_a…)` terms;
- a PP on a copular VP still reads `prep_in(subject, x)`, CGEL's "location of theme" (CGEL, pp. 680–682).

Verbal and copular VPs still share one type and coordinate («is essential and promotes …»).

| Current adjunct pin (15) | Host | Under the proposal |
|---|---|---|
| «…screened cell lines with a CRISPR library.», «…analysed cell lines with an RNA interference library.», «We ascertained MSI status with sequencing.», «These mutations occur in nucleotide repeat regions.», «These findings remained true with PCR-based MSI classifications.», «The MSI relationship compared favourably to other strong biomarkers for vulnerabilities.» (the «to» PP), «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI models.» | verb (7). #270 counts eight because it adds «… dependency in MSI cell lines compared to MSS cell lines», but that PP precedes «compared» and modifies the noun «dependency». | PP moves from the subject to the event; re-adjudicate |
| «WRN was dispensable in models of microsatellite-stable cancers.», «These classifications were highly concordant with …», «The four other RecQ DNA helicases were not preferentially essential in MSI cell lines.», «MSI is most commonly observed in …», «We found that WRN was selectively essential in MSI models.», «PARP-1 inhibitors are successful in cancers with …», «These libraries define genes that were essential for proliferation and survival.» and «We analysed these data sets for genes that are selectively essential in cancer cells with MSI.» (PP on the relative clause's predicate) | copular/adjectival (8) | unchanged |

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
| Kernel | none: impredicative `∃`, class subsumption and `core:EquivalentClasses` exist | 0 |
| `denote_cat` | branch `⟦S[dcl,f]⟧` on `f = adj` | 1 match arm (`kernel/src/dcg/category.rs:41`) |
| Verb entries | `FrameKind::arrow` gains an event slot. The emitted verb sem wraps the axiom in `λ…λK. ∃e:Ev. K(λa. v(…, a))(e)`. D62 §5 calls this "a converter-rule change + reseed". | 8 frame tags in one converter; 13,767 WordNet verb synsets plus the SPECIALIST verbs of D97, re-emitted |
| Closed class | sem types of entries over non-adjectival `S`. Terms change for determiners (pointwise `K`), negation, modals, prepositions, the copula and the complementiser; `do`, passive `be` and `by_agent` change type only | 316 of 465 entries; 137 SemTerms to audit (`ontologies/lexicon/closed-class.esl`) |
| Engine rules | coordination folds pointwise over `K`; relativiser and root close with `λP.P` | 4 files, 7,010 lines, 58 connective sites (`kernel/src/dcg/rules/`) |
| Verbaliser | `adjunct_of` matches the PP to the verb atom by the shared event variable instead of by the subject's printed string; the adjective and copula branches stay | `kernel/src/dcg/verbalize.rs:1497–1569` |
| Pins | each verb adds `ΠG#k:Prop. ΠG#k+1:§.`, one argument and one `→ G#k → G#k`; later binders renumber | ≥48 of 63 pins reprint (every pinned sentence whose ledger rows carry a verb atom); 7 change meaning; the 8 copular or adjectival adjunct pins keep their subject-anchored PP |
| Ledger | rows are keyed on the printed term, so every verb row is re-keyed. The D64 re-pin precedent was "transformed mechanically and checked against the new forest". | 193 of 228 rows re-keyed. 150 carry their verdict mechanically: 145 have no subject-anchored PP, and 5 hang it on an adjective or copula. 43 hang a PP on a verb and need re-adjudication (10 `correct`, 33 `wrong`, 14 of them grounded on subject predication). The 20 verbless rows with a subject-anchored PP on an adjective or copula are unchanged. |
| Canonical hashing | D47 stores binder names as strings, and readback names binders `G#<level>` (`kernel/src/nbe/readback.rs:392`), so α-variants already hash alike. Conjunct order is attachment order, first-attached innermost; the identity closure leaves no `True`. | every `enc:EncodedClaim` with a verb gets a new proposition and witness key; regenerate by drop-and-reseed |
| Term size | positional: +2 binders, +1 argument per verb. Neo-Davidsonian roles would add two role atoms per transitive verb on top; PMB gold has 7,516 role clauses beside 7,545 concept clauses ([van Noord et al. 2018, Table 1](https://aclanthology.org/L18-1267.pdf)) | derived, not measured |

## Recommendation

1. **Arguments: positional.** Adopt Davidsonian positional arguments plus one event argument, typed `lexicon:Entity` and bound by an impredicative `∃e:Ev` inside each verb's lexical entry. Governed PPs stay positional in `v{offset}_p_{prep}`. Do not adopt thematic-role predicates now.
2. **VP type.** Make the VP denotation take a predicate-transforming continuation, `Mod := (Entity → Prop) → Entity → Prop`, closed by the identity at the parse root and at every embedding boundary.
3. **Adjuncts.** Intersective adjuncts extend `Mod` with an opaque `prep_*(a, x)` conjunct on the anchor. Negation, modals and GQs pass `Mod` through. A test enforces this shape.
4. **Event class.** Use one event class, `wn:n00029378` `event.n.01`, with `urn:schema_org:Action` declared equivalent to `wn:n00037396` so D62's root enters the lattice under `lexicon:Entity`.
5. **Copular predicates** keep no event and keep their subject-anchored adjuncts.
6. **Deferred:** dependent event types, per-verb event classes, sum events and Kimian states. Each waits for a task that needs it.

## Implementation path and cost

**Slice 1 (vocabulary, no grammar change).**

- Declare the `core:EquivalentClasses` instance for `urn:schema_org:Action` and `wn:n00037396`, with a SEMAPV justification per D99 §11.
- Fix the event class.
- Witnesses: `is_subclass_of(urn:schema_org:Action, lexicon:Entity)` and `is_subclass_of(wn:n00029378, lexicon:Entity)`.
- Cost: one resource plus a test.

**Slice 2 (the type change, one PR, since no subset type-checks).**

- the `denote_cat` branch;
- the converter's event slot and verb sem;
- the 316 closed-class types and the VP-touching terms;
- the coordination, relativiser and root-closure rules;
- `adjunct_of` keyed on the event variable;
- the Charlow-invariant lexicon test;
- a reseed (`scripts/reseed-lexicon-db.sh`).

The same PR transforms the pins and ledger mechanically:

- at least 41 verb pins without a verbal adjunct;
- 150 ledger rows.

It also brings the 7 verbal-adjunct pins and 43 ledger rows to the maintainer as a re-adjudication batch, because the parse gates fail on stale pins otherwise.

**Slice 3 (persisted claims).** Drop and reseed the encoded artefacts. Every verb-bearing claim gets a new D47 term and witness key.

**Later, each its own decision:**

- thematic roles for knowledge-graph alignment, taking D62's fork (role axioms vs property-as-relation);
- Kimian states, if state anaphora or the stative adverb gap is needed;
- sum events for measure adjuncts;
- the generic reading of present-tense negation.

## Open questions for the maintainer

**1. The event class.** Options:

- (a) bind at `wn:n00029378` and declare `schema_org:Action ≡ wn:n00037396` (recommended);
- (b) mint a `lexicon:Eventuality` above both;
- (c) bind at `urn:schema_org:Action` as D62 named it. This still requires placing Action under `lexicon:Entity`, and it types the events of `arise`, `result` and `occur` as agentive.

`urn:schema_org:Event` is not a candidate: it is "an event happening at a certain time and location, such as a concert, lecture, or festival", its 20 subclasses are of that kind (`Festival`, `SportsEvent`, `ScreeningEvent`), and like Action it sits under the parentless `schema_org:Thing`, not `lexicon:Entity` (D62 §5: "scheduled happenings"). If a task imports schema.org-typed records, `schema_org:Event ⊑ wn:n00029378` is the sound link; #270 needs none.

**2. Copular predicates.** Options:

- (a) keep them eventless with subject-anchored adjuncts (recommended);
- (b) introduce a Kimian state at the copula, which makes eventualities uniform and lets typing enforce the stative adverb gap, at the cost of a state binder in every copular clause;
- (c) read their PPs as proposition-level frame restrictors, #270's option A applied to statives only.

**3. Stative verbs.** Should all verb synsets take events, including the 756 in `verb.stative`, as recommended because of the «arise» twins? Or should the importer start keeping `lex_filenum`, which `Synset` drops today (`crates/eigenius-wordnet/src/wndb.rs:84–126`), and split by class?

**4. Governed prepositions.** Options:

- (a) keep the free-adjunct parse of a preposition that the lexicon places on the verb sense in the forest, and rule it `wrong` on `structure` (recommended; D63 carries attachment ambiguity);
- (b) suppress it by a normal-form rule.

Either way the 14 rows grounded on "predicates … of the subject" need new evidence text.

**5. Purpose *for*.** Rule U1's attachment. Then decide whether purpose *for* stays the one opaque `prep_for` it shares with benefactive and «essential for», or becomes a distinct relation.

**6. Coordination and quantified PP objects.**

- Should the distributive two-event reading of coordinated objects be the only one until sum events exist?
- Should quantified PP objects scope over the event quantifier (de Groote & Winter, Champollion) rather than under it (ccg2lambda)?

**7. Negation in present-tense scientific claims.** «Some cancers do not respond …» comes out as `¬∃e`, meaning no responding event occurs. A generic or dispositional reading, with Gen binding `e` as in Kratzer and Diesing via Maienborn, is not covered by any source read.

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
