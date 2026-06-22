# D61 — Faithful encoding of reasoning: grounding-discovery and a typed decision layer

*Status: **proposed** (June 2026) · design specification. Extends the **D58** objective
ontology with a typed decision layer (§5) and a fifth admissibility gate (§6). Consolidates the
working note `docs/notes/d61-llm-based-encoding-methodology.md` and the deep-research survey
`docs/notes/llm-based-encoding.md` (primary-read, §9). The ontology + gate (§5–6) are **Phase-1
work**, designed here and validated/refined by the D57-redux dogfood (§7); only the executable
back-stop (§7, Phase 2) is deferred.*

*Companion documents: **[D58 objective framing](d58-objective-framing-and-obligation-graphs.md)**
(the obligation graph this extends — `objective:Milestone`, the four gates), [D39 justification
logic](d39-justification-logic.md) + [D49 ChainWitness](d49-chainwitness-machinery.md)
(certificates / witnesses — the answer-as-witness backbone), [D59 EigenQL array patterns &
derived joins](d59-eigenql-array-patterns-and-derived-joins.md) (the gate-query machinery),
[D43 retrieval](d43-text-and-vector-retrieval.md) (discovery), [D32 FormulaTerm](d32-chain-mirrored-mini-tt-inductives.md)
+ [D47 EigenTT fragment](d47-chain-mirrored-eigentt-type-fragment.md) (the content notation),
[D57 schema.org mapping](d57-schema-org-vocabulary-mapping.md) (the dogfood this is harvested
from + validated against), [D30 Eigon→Lean](d30-eigon-to-lean-faithful-translation.md) /
[D28 Lean institution](d28-lean-4-as-institution.md) (the proof-carrying ideal, §8). Operationalized
by the `reasoning` / `grounding` skills.*

---

## 1. Motivation — the faithfulness gap, and the gap in D58

**The faithfulness gap.** Across the LLM-formalization literature (§9) the pattern is *generate
→ check against a formal oracle → refine*, but **the oracle proves structural/logical validity,
never that the formalization captures intent.** *Checker-passing ≠ faithful*: an
autoformalization pipeline whose back-translation LLM-judge rated it ~97 % accurate was ~66 % on
human review (~34.8 % end-to-end); even human-written formal statements carry 16.4 %/38.5 %
semantic errors (§9).

**In Eigenius terms.** The kernel commit gate is **oracle #1**: a `reasoning:ReasoningSentence`
`Holds` iff its certificate type-checks against an admitted witness (D39/D49). That proves the
claim *follows from admitted evidence* — not that it was grounded in the *right discovered fact*.

**D57 is the proof.** Its load-bearing call — #9: schema.org's `domainIncludes` is *advisory*, so
→ `core:recommends`, never the restrictive `core:domain` — was a **discovery failure**. The
conformance fact lived in schema.org's prose spec; it surfaced reactively, by a human `[steer]`,
cited only when asked (`d57-mapping-decisions.md`). Both encodings **type-check** — `core:domain`
is well-formed — so oracle #1 cannot distinguish the faithful one. Only the *discovered fact*
can.

**The concrete gap in D58.** D58 made the obligation graph typed and gave four gates, but two
weaknesses remain that this spec closes:

1. **The falsifier is prose.** `objective:falsifier : core:string` (D58 §5.2) — "what would
   refute this Milestone" — is a free-text note. It is neither runnable nor checkable; it is
   exactly the "type the wrapper, not the content" half-measure.
2. **The descent's outputs are untyped.** The decisions, desirable properties, limitations,
   tensions, and the cut that *generate* the falsifiers (the content of D57's three after-the-fact
   notes, §4) have no typed home — they live as prose, so "is this decision grounded?" is a
   judgement, not a query.

## 2. Thesis and the two oracles

The goal is **not** a new "encoding" discipline. Eigenius already encodes reasoning (the
`reasoning` protocol). The lever is **doing `grounding` better — including the *discovery* of the
material the reasoning rests on** — and **typing the content of that discovery so it is checkable.**

> **Thesis.** *Reasoning is faithfully encoded only when it rests on properly **discovered**
> grounding.* Make discovery a first-class, **gated** phase whose targets and outcomes are
> **typed, runnable content** — so a conclusion cannot commit while its grounding is undiscovered,
> and "is this grounded?" is a query.

| Oracle | Question | Mechanism | Status |
|---|---|---|---|
| **#1 structural** | does the claim follow from admitted evidence? | the kernel commit gate (D39/D49) | exists |
| **#2 grounding** | is every load-bearing claim grounded in *discovered* fact? | the typed decision layer (§5) + the **Discovered** gate (§6) + the back-stop check (§7) | this doc |

Oracle #2 is **purely additive** — it never weakens or routes around the kernel gate.

## 3. The descent — how a task generates typed discovery targets

Competency questions are not an input; they are the **fixpoint of a descent** that turns a vague
goal into the typed nodes of §5. From the goal:

1. **Name the task-type & its decision structure** — what decisions does any instance force? →
   `objective:DecisionPoint`s (§5). *Retrieve-first* (D43): reuse a recorded structure if one
   exists. The step most easily skipped by diving into implementation.
2. **Ground bilaterally** — source and target, *enough to instantiate* those decisions; spec, not
   just data. → `objective:Axiom`s (cited; D58).
3. **Elicit desirable properties** → `objective:DesirableProperty`s → candidate competency
   questions.
4. **Stress for limitations & tensions** → `objective:Limitation` / `objective:Tension`; the
   tensions carry the **sharpest** competency questions, located where faithfulness is at risk
   (D57 #9 is a move-4 tension).
5. **Surface incompleteness** → `objective:CutItem`s ("nothing silently dropped" becomes a query).
6. **Recurse** — grounding deepens understanding; re-enter (D58 §4.1, loop budget).

This is the operational content of D58 §0 frame⇄ground co-recursion. Each move emits typed nodes
(§5); the **Discovered** gate (§6) blocks a milestone while any target is open.

## 4. Worked example, stated — D57 #9 (the running example through §5–6)

D57 decision #9, today a prose `reflection:rationale`:

> "schema.org `domainIncludes` is advisory … so the faithful target is `core:recommends`, not
> `core:domain` — mapping to `domain` would invent an enforcement schema.org never asserts."

Read off the descent, this is: a **Tension** (preserve open-world ⟂ gain type-safety by
enforcing) → a **DecisionPoint** (map `domainIncludes` to `recommends` vs `domain`) → resolved by
a **DesirableProperty** (`PreservesOpenWorld`) → anchored by a cited **Axiom** (the spec's
advisory-stance conformance fact) → made checkable by a **CompetencyQuestion** (a runnable query:
*no property carries `core:domain`*). §5 types each; §5.6 encodes the whole thing in ESL.

## 5. The typed decision layer (the design — extends `ontologies/objective/`)

### 5.1 Backbone: reuse the reasoning stack (no parallel epistemics)

Lai et al.'s *queries-as-types, answers-as-proof-carrying-witnesses* (§9) is already Eigenius's
model: a `Prop` (encoded `eigentt:TypeExpr`) is a type; a `reasoning:ReasoningSentence` certificate
is a proof-carrying witness (D39). So, exactly as D58:

- a **discovery target** carries its question as `objective:proposition` (a `Prop`/`TypeExpr`);
- a **grounded answer** is a **witness** (`IsObservedAs`/`IsDeclaredAs`/`IsDerivedAs`/`IsVerifiedAs`),
  named by `objective:grounded_by`; an **open** target has none.

The decision layer is a structural planning layer over that stack — never a parallel epistemics
(the D58 discipline).

And the **records-first** reading is even closer: Cooper's TTR (Type Theory with Records; D62 §3)
is a mature semantics built natively on record types, and an Eigenius `Class` *is* a record type
— `requires`/`recommends` are its fields, a `Resource` is a record = a witness, a proposition is
a type true iff witnessed. So the §5 nodes below are **record types** and their content is
**record-typed** (the Class-as-record-signature insight), not flat symbols.

### 5.2 The content-typing principle (the load-bearing requirement)

**Type the content, not the wrapper.** A node's *content* — the mapping rule, the property, the
limitation — is a **typed `Prop`** (in `objective:proposition` / `objective:option_claim`), and a
falsifier is a **runnable EigenQL query** (in `objective:query`), **never** a prose string.

The *deeper* form, per the records-first reading (§5.1): a node's content is itself a **record
type** — a decomposed predicate like `OutputValidates` unfolds into a record with fields
{output, generator, input, condition}, i.e. a *frame* in TTR's sense (Cooper, "frames as
records"), whose witness is the evidence. Building the content vocabulary as such composable
record types — rather than flat per-task `Prop` symbols — is a flagged refinement of this section.

The content notation is the existing one — task-specific `Prop` families declared per objective,
exactly as D57 declares `obj:GeneratorConforms`:

```esl
data obj:MapsConstruct      : core:resource -> core:resource -> Prop { }  // (source, target) constructs
data obj:PreservesOpenWorld : core:string   -> Prop { }                   // a named property of the mapping
data obj:TypeSafetyGain     : core:string   -> Prop { }
```

Connectives beyond `Prop`-application + the arrow `->` (conjunction, negation, quantification over
constructs) are expressed in the D32 FormulaTerm / D47 EigenTT fragment; **where a content
fragment is not yet expressible, extending that notation is part of the harvest** (§7) — surfaced
by the dogfood, not speculated. The *operational* negation needed here ("`¬uses(core:domain)`") is
carried by the falsifier **query** (a `count = 0` check, §5.6), so the running example needs no new
connective.

### 5.3 New classes

Proposed `objective:` classes (ESL, in the style of `objective:Milestone`). Designed here;
validated/refined by the dogfood (§7).

```esl
class objective:CompetencyQuestion {
    description = "A discovery target and RUNNABLE falsifier — the typed form of objective:falsifier (D58). A question the grounding must answer before a dependent Milestone may conclude: the question as a Prop (objective:proposition), a runnable EigenQL falsifier (objective:query) with its expected result (objective:expected_answer), and what it probes (objective:probes). GROUNDED when objective:grounded_by names the witness/verdict that answers it; OPEN otherwise — an open CQ blocks its Milestone via the Discovered gate (§6).";
    requires   objective:proposition, objective:query, objective:expected_answer;
    recommends objective:probes, objective:grounded_by, objective:status;
}

class objective:DecisionPoint {
    description = "A decision any instance of this task-type must make (descent move 1). The decision as a Prop (objective:proposition), a TYPED option space (objective:options → objective:Option), the selected option (objective:selected), and the warrant (objective:warrant → the DesirableProperty / CompetencyQuestion / Axiom that justifies it). Recording DecisionPoints makes the task-type's decision structure retrievable & reusable (move-1 retrieve-first).";
    requires   objective:proposition, objective:options, objective:selected;
    recommends objective:warrant, objective:status;
}
class objective:Option {
    description = "A typed alternative inside a DecisionPoint's option space. Its claim is a Prop (objective:option_claim) — e.g. MapsConstruct(domainIncludes, core:domain) — which is what lets a DesirableProperty be checked against it.";
    requires   objective:option_claim;
    recommends core:description, objective:warrant;
}

class objective:Hypothesis {
    description = "A formulated conjecture (descent): a Prop (objective:proposition) asserted as a Declared hypothesis, with the path to a DECISIVE OUTCOME — objective:resolved_by names the ReasoningSentence/verdict that discharges it (Derived/Verified) or refutes it (a recorded finding); objective:outcome records held|refuted. Unlike a Milestone (a goal to derive), a Hypothesis MAY turn out false — refutation is a valid outcome. Until resolved it is never stated as fact (the reasoning grade-climb, typed).";
    requires   objective:proposition;
    recommends objective:resolved_by, objective:outcome, objective:status;
}

class objective:DesirableProperty {
    description = "A property a good instance should have (descent move 3). Content is a Prop (objective:proposition) — e.g. PreservesOpenWorld(mapping) — that becomes a CompetencyQuestion and/or a DecisionPoint warrant. Carries a priority.";
    requires   objective:proposition;
    recommends objective:priority, objective:checked_by, objective:status;
}
class objective:Limitation {
    description = "A property the target CANNOT satisfy, with reason (descent move 4). Content is a Prop stating the inexpressibility (objective:proposition) + a reason + the consequence (objective:consequence → a CutItem or a demoted DesirableProperty). Limitations locate where faithfulness is at risk.";
    requires   objective:proposition, objective:reason;
    recommends objective:consequence, objective:status;
}
class objective:Tension {
    description = "Two DesirableProperties that cannot be jointly maximized (descent move 4). objective:between → the two; objective:resolved_by → the DecisionPoint that trades them off. The sharpest CompetencyQuestions sit on a Tension's resolution (D57 #9).";
    requires   objective:between;
    recommends objective:resolved_by, objective:status;
}

class objective:CutItem {
    description = "An item the artifact does not cover, recorded with reason + disposition (descent move 5). Makes 'nothing silently dropped' a query. objective:disposition ∈ {residual_recorded, excluded, deferred}.";
    requires   objective:proposition, objective:reason, objective:disposition;
}
class objective:Disposition {
    description = "How a CutItem is handled — a small resource enum (allows_only-constrained).";
}
resource objective:disp_residual_recorded : objective:Disposition { core:description = "Recorded as inert residual/provenance (e.g. D57 Tier-3)."; }
resource objective:disp_excluded          : objective:Disposition { core:description = "Out of scope by an explicit layer/scope rule (e.g. pending/attic)."; }
resource objective:disp_deferred          : objective:Disposition { core:description = "Deferred to a future pass."; }
```

### 5.4 New properties

```esl
// CompetencyQuestion — the typed, runnable falsifier
property objective:query : core:string {
    description = "A runnable EigenQL falsifier for a CompetencyQuestion. Its EXPECTED result is objective:expected_answer; the actual result, run against the objective's branch, grounds the CQ (objective:grounded_by) or leaves it open (Discovered gate, §6). This is what types D58's prose objective:falsifier.";
    domain objective:CompetencyQuestion;
}
property objective:expected_answer : core:string {
    description = "The result objective:query must return for the CQ to be grounded (e.g. the empty set, or a specific count).";
    domain objective:CompetencyQuestion;
}
property objective:probes : core:resource {
    description = "The DecisionPoint / DesirableProperty / Limitation this CQ checks.";
    class_types objective:DecisionPoint, objective:DesirableProperty, objective:Limitation;
    domain objective:CompetencyQuestion;
}
property objective:grounded_by : core:string {
    description = "IRI of the witness / verdict / ReasoningSentence that answers this CQ (filled when the falsifier returns its expected_answer). Empty ⇒ the CQ is OPEN (Discovered gate).";
    domain objective:CompetencyQuestion;
}
property objective:discovery_target : core:resource_array {
    description = "The CompetencyQuestions a Milestone must have GROUNDED before it may conclude. The Discovered gate (§6) joins on this. The typed successor to D58's prose falsifier on a Milestone.";
    class_types objective:CompetencyQuestion;
    domain objective:Milestone;
}

// DecisionPoint / Option
property objective:options : core:resource_array { description = "The option space."; class_types objective:Option; domain objective:DecisionPoint; }
property objective:selected : core:resource { description = "The chosen option."; class_types objective:Option; domain objective:DecisionPoint; }
property objective:option_claim : core:resource { description = "The option's claim, as a Prop."; class_types eigentt:TypeExpr; domain objective:Option; }
property objective:warrant : core:resource_array {
    description = "Why the selection holds: the DesirableProperties / CompetencyQuestions / Axioms that justify it.";
    class_types objective:DesirableProperty, objective:CompetencyQuestion, objective:Axiom;
}

// Hypothesis
property objective:resolved_by : core:string { description = "IRI of the ReasoningSentence/verdict that resolved the hypothesis (held or refuted)."; domain objective:Hypothesis; }
property objective:outcome : core:string { description = "held | refuted | open — the decisive outcome (refuted is a recorded finding, not a failure)."; domain objective:Hypothesis; }

// DesirableProperty / Limitation / Tension / CutItem
property objective:priority : core:integer { description = "Relative importance of a DesirableProperty."; domain objective:DesirableProperty; }
property objective:checked_by : core:resource { description = "The CompetencyQuestion that checks this property."; class_types objective:CompetencyQuestion; domain objective:DesirableProperty; }
property objective:reason : core:string { description = "Why a Limitation holds / why a CutItem is cut."; }
property objective:consequence : core:resource { description = "What a Limitation forces."; class_types objective:CutItem, objective:DesirableProperty; domain objective:Limitation; }
property objective:between : core:resource_array { description = "The two DesirableProperties in tension."; class_types objective:DesirableProperty; domain objective:Tension; }
property objective:disposition : core:resource { description = "How a CutItem is handled."; class_types objective:Disposition; allows_only objective:disp_residual_recorded, objective:disp_excluded, objective:disp_deferred; domain objective:CutItem; }
// objective:resolved_by is reused on Tension → the resolving DecisionPoint (string IRI).
```

### 5.5 `objective:proposition` carries `TypeExpr` already

No change to D58's `objective:proposition` (`core:resource`, `class_types eigentt:TypeExpr`) is
needed — it already holds an encoded `Prop`. The new nodes reuse it (and `option_claim` mirrors
it for Options), which is precisely why the content is *typed* rather than prose.

### 5.6 The running example, encoded

D57 #9, end-to-end in the typed layer (namespace `obj = urn:eigenius:obj:d57`; the `Prop` families
from §5.2; `lit:cite_datamodel_conformance` is the D57 conformance Axiom/Citation):

```esl
// move 4 — the tension that makes #9 the sharpest question
resource obj:dp_open_world  : objective:DesirableProperty { objective:proposition = type_expr( obj:PreservesOpenWorld("schema_org") ); objective:priority = 1; objective:checked_by = obj:cq_no_domain; }
resource obj:dp_type_safety : objective:DesirableProperty { objective:proposition = type_expr( obj:TypeSafetyGain("schema_org") );     objective:priority = 2; }
resource obj:tn_ow_vs_enforce : objective:Tension {
    objective:between     = [obj:dp_open_world, obj:dp_type_safety];
    objective:resolved_by = "urn:eigenius:obj:d57:dec_domain_mapping";
}

// move 1 — the decision, with a TYPED option space
resource obj:opt_domain     : objective:Option { objective:option_claim = type_expr( obj:MapsConstruct(schema_org:domainIncludes, core:domain) );     core:description = "domainIncludes → restrictive core:domain"; }
resource obj:opt_recommends : objective:Option { objective:option_claim = type_expr( obj:MapsConstruct(schema_org:domainIncludes, core:recommends) ); core:description = "domainIncludes → advisory core:recommends"; }
resource obj:dec_domain_mapping : objective:DecisionPoint {
    objective:proposition = type_expr( obj:MapsConstruct(schema_org:domainIncludes, core:recommends) );
    objective:options     = [obj:opt_domain, obj:opt_recommends];
    objective:selected    = obj:opt_recommends;
    objective:warrant     = [obj:dp_open_world, obj:cq_no_domain, lit:cite_datamodel_conformance];
}

// move 3/2 — the CQ: the TYPED, RUNNABLE falsifier (the crux). Empty result ⇒ grounded.
resource obj:cq_no_domain : objective:CompetencyQuestion {
    objective:proposition     = type_expr( obj:PreservesOpenWorld("schema_org") );
    objective:query           = "USING \"urn:eigenius:core:Property\" MATCH ?p { \"urn:eigenius:core:domain\": ?d } RETURN [] { p: ?p } TOP 1";
    objective:expected_answer = "∅  (no urn:schema_org: property carries core:domain)";
    objective:probes          = obj:dec_domain_mapping;
    objective:grounded_by     = "urn:eigenius:obj:d57:verdict_no_domain";  // filled once the query returns ∅
}

// the mapping Milestone may not conclude until cq_no_domain is grounded
resource obj:m_domain_mapped : objective:Milestone {
    objective:proposition      = type_expr( obj:MapsConstruct(schema_org:domainIncludes, core:recommends) );
    objective:acceptance_grade = epistemic:verified;
    objective:witness_kind     = objective:wk_query;
    objective:falsifier        = "a urn:schema_org: property carries core:domain";   // human gloss
    objective:discovery_target = [obj:cq_no_domain];                                  // the TYPED falsifier
}
```

Contrast with status quo: #9 today is one prose sentence in a `rationale`. Here it is a tension, a
two-option decision whose options are **Props**, a desirable property **discharged by a query that
runs**, and a milestone the **Discovered gate holds open until the query returns ∅**. That is the
difference between *read it and trust* and *run it and know* — and it is what makes the F1
transitive-closure bug (43 ≠ 66) a fail-closed CQ (`obj:cq_enum_closed`, an analogous count check)
rather than a test someone later chose to write.

## 6. The Discovered gate

D58's gate family (Expressible / Checkable / Anchored / Reachable) gains **Discovered**: *a
Milestone may not conclude while a CompetencyQuestion it names is open.* Like Reachable/Anchored
it is an on-demand EigenQL query over the objective's branch (D58 §5.5; D59 features); **empty
result = gate passes**, any row is a milestone blocked on undiscovered grounding.

```eigenql
// Well-posedness gate: DISCOVERED (D61 §6). Mirrors well-posed-anchored.eigenql.
// A CQ is GROUNDED iff its grounded_by IRI resolves to a committed resource.
DEFINE Grounded(?q) FROM
    MATCH "urn:eigenius:objective:CompetencyQuestion"(?q) { "urn:eigenius:objective:grounded_by": ?w },
          ?c {}
    WHERE ?w = ?c
DEFINE OpenCQ(?q) FROM MATCH "urn:eigenius:objective:CompetencyQuestion"(?q) {}, NOT Grounded(?q) {}
DEFINE Undiscovered(?m) FROM
    MATCH "urn:eigenius:objective:Milestone"(?m) { "urn:eigenius:objective:discovery_target": [... ?q ...] },
          OpenCQ(?q) {}
MATCH Undiscovered(?m) {} RETURN [] { "urn:eigenius:objective:undiscovered": ?m }
```

It lands at `experiments/objectives/well-posed-discovered.eigenql`, run alongside the existing two
on-demand gates. (Like them, it is a query, not a materialized verdict — D58 §5.5.) This turns
"we should have checked the spec" from hindsight into a fail-closed stop: the structural analogue,
moved upstream into grounding, of the `reasoning` skill's *fail closed*.

**Generating the CQs — the practical method.** Authoring good competency questions is the hard
part; the practical generator plays to the LLM's *language* strength, not its (unreliable)
formalization: for a construct, an LLM produces **labelled examples** (premise → hypothesis →
label, *including negatives*) that surface it, and the construct is accepted only if it reproduces
those labels under the kernel. The examples *are* the CQ battery — the LLM generates, the type
system judges. This is the lexicon-bootstrapping validation step (D62 §8.3) generalized to any
construct; it is graded **Derived** (the labels are themselves LLM output → human-sample; prefer
gold where it exists).

## 7. Grading the grounding verdict

A grounding check that passes is **Derived** (a program computed a CQ pass-rate / a back-translation
score) — **never auto-Verified.** The LLM-judge inflates (§9); the strongest *automatic* grade is
Derived. Only a **human spot-check** or a **proof-level correspondence** (§8) elevates toward
Verified. (A CQ grounded purely by an EigenQL query that returns its `expected_answer` — like
`obj:cq_no_domain` — is **Verified-by-check**, since it is a kernel-evaluated query over the real
output, not an LLM judgement; the LLM back-stop, §7-Phase-2, is the Derived case.)

## 8. Implementation roadmap

**Phase 1 (now) — the typed layer.** Realize §5 in `ontologies/objective/objective-ontology.esl`
and §6 as `well-posed-discovered.eigenql`; **dogfood D57-redux** — re-run the descent (§3) on the
schema.org mapping, **typing each output's content** (D57's three notes are the answer key), and
show #9 and F1 recovered up front as the §5.6 typed nodes + passing CQs. The dogfood is what
*validates and refines* the §5 vocabulary (e.g. reveals a missing property or a content-notation
gap). Plus the skill/doc upgrades: `grounding.md` (discovery as a gated step), `reasoning.md` (the
grounding caveat — a `Holds` ≠ grounded-in-discovered-fact), D58 (add the Discovered gate to §3/§5),
and the §9 anchors into the `.bib` + D18/D30/D39/D50.

**Phase 2 (later) — the executable check** (machinery only; the typed shape is Phase 1):
- a **CQ-runner** — executes each `objective:CompetencyQuestion.query`, compares to
  `expected_answer`, fills `grounded_by` or leaves the CQ open (fail-closed).
- a **back-stop component** `orchestration/src/components/faithfulness_check.ts` (sibling of the
  D8 `complete_json.ts`): back-translate a conclusion, score consistency against its cited ground,
  run **through the kernel** (D56) → a **Derived** `GroundingIsFaithful` verdict or fail closed.
- **multi-candidate scoring** + a **human-review surface** for the residual (a human sign-off
  elevates the grade toward Verified, §7).

## 9. Explore further
- **Lean correspondence** — lift a stable, proof-relevant typed core to Lean (the Lai et al.
  queries-as-types/answers-as-proof-terms ideal at full strength; D28/D30/D40). Research-grade.
- **The prose→trees encoding engine** — schema-constrained extraction (SPIRES-style) extending
  D8 `CompleteJson` with §5 as the contract — **now designed in
  [D62](d62-encoding-engine-prose-to-trees.md)** (the generation front-end this check guards;
  realized as an on-demand *institution*).
- **Encoding/grounding benchmarking** extending D50 (Text2KGBench-style conformance/hallucination
  metrics).

## 10. Prior art (primary-read & verified)

Read from primaries during this design (the `grounding` discipline applied to this doc); reading
them **corrected the secondary survey** — corrections noted. Full entries →
`docs/references/eigenius_related_work.bib`.

**Dependent type theory as a KG/ontology substrate** (→ D30/D39, D18)
- Cooper, R. *From Perception to Communication: A Theory of Types for Action and Meaning.* OUP,
  2023 (open access; DOI 10.1093/oso/9780192871312.001.0001). **TTR** — records-first: a record
  type *is* the Class-as-record-signature (`requires`/`recommends` = fields; `Resource` = record =
  witness); types not possible worlds; first-class types + reflection. The closest external
  substrate match (D62 §3). Primary-read.
- Lai, Z.; Ng, A. B.; Wong, L. Z.; See, S.; Lin, S. *Dependently Typed Knowledge Graphs.*
  arXiv:2003.03785 (2020). RDF + SPARQL in CIC/Coq; *"explainability in answers to queries through
  witnesses … compositionality and automation in the construction of witnesses"*; explicitly *"a
  proof of concept."* The precedent for §5.1.
- Barlatier, P.; Dapoigny, R. *A type-theoretical approach for ontologies: the case of roles.*
  Applied Ontology 7(3), 2012 (DOI 10.3233/AO-2012-0113); *Modeling Contexts with Dependent
  Types*, Fundamenta Informaticae, 2010. CIC/CCω + Dependent Record Types. *(Correction: drop the
  secondary "SUMO→GF" detail — not theirs.)*
- Luo, Z. *Common Nouns as Types*, LACL 2012 (DOI 10.1007/978-3-642-31262-5_12); *Formal Semantics
  in Modern Type Theories with Coercive Subtyping*, Linguistics & Philosophy, 2012. Types +
  coercive subtyping for subsumption — the treatment D18 parallels.
- Chatzikyriakidis, S.; Luo, Z. *Formal Semantics in Modern Type Theories.* ISTE/Wiley, 2020
  (DOI 10.1002/9781119489252). The comprehensive MTT-semantics reference: dependent types +
  coercive subtyping, **both model- and proof-theoretic**, Coq-verified NL semantics, dependent
  event types; impredicative `Prop` ≈ Eigenius D46. (Main chapters paywalled; characterized from
  abstract + TOC + free appendices.)

**LLM ontology learning / typed KG construction** (→ D50, D8)
- Mihindukulasooriya, N.; Tiwari, S.; Enguix, C. F.; Lata, K. *Text2KGBench.* arXiv:2308.02357
  (2023). Ontology-conformance + subject/relation/object hallucination metrics.
- Babaei Giglou, H.; D'Souza, J.; Auer, S. *LLMs4OL.* ISWC 2023, arXiv:2307.16648. Term typing /
  taxonomy / non-taxonomic relations; foundational LLMs alone insufficient.
- Caufield, J. H.; et al. *SPIRES: … populating knowledge bases using zero-shot learning.*
  Bioinformatics 40(3), btae104 (2024). LinkML-schema-constrained recursive extraction grounded to
  ontology IDs.

**The faithfulness gap** (→ this doc, D30/D39)
- Gao, G.; et al. *Herald: A Natural Language Annotated Lean 4 Dataset.* arXiv:2410.10878, ICLR
  2025. Back-translation + LLM-judge as the faithfulness check.
- Ospanov, A.; Farnia, F.; Yousefzadeh, R. *miniF2F-Lean Revisited.* NeurIPS 2025, arXiv:2511.03108.
  Herald **~97 % (LLM-judge) → ~66 % (human)**, **~34.8 % end-to-end**. *(Correction: this audit's
  figures, not Herald's own claims.)*
- Chen, G.; et al. *ReForm: Reflective Autoformalization …* ICLR 2026, arXiv:2510.24592. **16.4 % /
  38.5 %** semantic errors in *human-written* miniF2F / ProofNet statements; LLM-judge ceiling
  **85.8 %**. *(Correction: human-statement error rates, not autoformalizer output.)*

## 11. Out of scope
- A production RDF↔CIC toolchain or HoTT-on-KG (research-grade; §9's Lean correspondence is the
  stepping stone).
- Anything that weakens or routes around the kernel commit gate — oracle #2 is additive.
- schema.org mapping rules (D57) and the objective-framing core (D58) — this doc *uses and extends*
  them; the new nodes (§5) and gate (§6) are additive to the D58 ontology.
```
