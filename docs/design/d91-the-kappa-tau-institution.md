# D91 — The κ–τ institution

**Status: implemented through step 4.** Written and built `2026-09-13`. Companion to
[D90](d90-the-institution-result-contract.md), which fixes the contract an institution's output
must satisfy; this note designs the first institution built against it.

**Sources.** The pilot specification is `references/publications/pilot_spec.pdf` (R. Pareschi,
STAKE Lab, University of Molise, v0.1). The logic is `references/publications/main.tex`
(arXiv:2608.08192). The replication whose conclusions the pilot re-adjudicates is arXiv:2608.04457
§6, whose chain is `experiments/publications/wrn-helicase/chain`.

---

## 1. What the pilot asks for

Re-adjudicate the WRN replication's derived conclusions under rival-sensitive commitment, and
answer one question: **which conclusions would have been held in suspended derivation under a
non-degenerate margin, and at what margin does each commit.**

The commitment condition, Definition "Rival-sensitive commitment" in the paper:

> `Commit_S(φ)` iff `sc_S(φ) ≥ τ`, and for every **active rival** `ψ` —
> one with `sc_S(ψ) ≥ ε` and `κ*(t_φ, t_ψ) < 0` —
> `sc_S(φ) − sc_S(ψ) ≥ δ(τ, −κ*(t_φ, t_ψ))`.

Three steps: reconstruct rival sets per conclusion, implement the condition as a commit-time
trigger emitting the standard typed Verdict, then sweep δ over a grid and record where each
conclusion first suspends. One sanity check governs the whole build: **δ ≡ 0 must reproduce the
published result**, every conclusion committed.

Division of labour in the spec: Eigenius supplies institution API guidance, chain read access and
review; STAKE supplies the institution implementation and the analysis.

## 2. What the scoring semantics actually requires

Small enough to state completely, which is why the institution is a good capstone.

| quantity | definition |
|---|---|
| `sc_S(h)` | `w(h)` for an atomic hypothesis |
| `sc_S(o)` | `1` for an accepted observation |
| `sc_S(φ ∨ ψ)` / `sc_S(φ ∧ ψ)` / `sc_S(¬φ)` | `max` / `min` / `1 − sc_S(φ)` |
| `κ*(t₁, t₂)` | mean of `κ(hᵢ, hⱼ)` over `At(t₁) × At(t₂)`; `0` when `t₁ ≡⊗ t₂` |
| `sc_S(t₁ ⊗ t₂)` | `[max(sc t₁, sc t₂) + λ·κ*(t₁,t₂)·sc(t₁)·sc(t₂)]` clamped to `[0,1]` |
| `δ` | `(0,1] × (0,1] → [0,1)`, non-decreasing in each argument, `δ(τ,x) → 0` as `x → 0` |

Every one of these is float arithmetic over declared and observed inputs. None of it is a proof.

**The provenance boundary is the paper's own** (§Neurosymbolic Realization): the epistemic side
`⟨H, w, κ, κ_o⟩` is open to computational estimation, the normative side `⟨τ, ε, δ⟩` is never
learned. The pilot's κ estimation follows the same split — embedding-based where semantic,
declared where operational.

**A name collision to avoid.** The paper calls `Pχ` and `C_τχ` *judgments*, and says they are not
score-bearing. They are threshold operators over a scored content formula. They are not
`eigentt:Judgement`, which is `holds(logic, term, type)` and asserts that a checker ran. The two
words coincide and the concepts do not.

## 3. What exists today, measured against the tree

| what | where | state |
|---|---|---|
| projection algebra | `kernel/src/justification/mod.rs` | `support`, `leaves_of`, `survives_without`, `cited_iris`, `is_fully_verified` — shipped |
| institution dispatch | `kernel/src/institution/dispatch.rs` | AutoOnLoad fires per resource class on commit; returns gate Verdict plus derivations |
| output shape | `kernel/src/institution/runtime.rs` | `QueryOutcome { output, derivations, partial_invocation }` |
| worked template | `crates/eigenius-statistics` | in-process Rust institution, 4 QueryClasses, gate Verdict plus per-effect derivations |
| WRN conclusions | `experiments/publications/wrn-helicase/chain` | **33** `justification:Conclusion`, 56 `justification:Declaration`, 21 `stats:StatisticalAnalysisPlan` |
| axiom precedent | `ontologies/statistics/statistics.esl` | `stats:lt : core:float -> core:float -> Prop`, `stats:mean_diff_of : core:string -> core:float` |
| witness signature precedent | `ontologies/core/core-ontology.json` | `witness:IsDeclaredAs : core:iri -> Prop -> Prop` |

**The framework's two internal queries are already answered.** *Which estimates does this
conclusion rest on* is `leaves_of` and `cited_iris`. *Does it survive removing one* is
`survives_without`. Both take a justification term, so both work exactly as long as the institution
emits a composite term rather than one opaque leaf. That is the requirement the pilot spec already
accepted, and it is the reason it matters.

## 4. What does not exist, and what it costs

**There are no chain-resident plausibility scores.** The spec takes `w` from them. A search of
`ontologies/` and `experiments/` for a property named for a score, confidence, plausibility,
weight or strength returns nothing. Nothing on the chain grades a conclusion numerically; what the
chain holds is the justification term, the witnesses, and pinned statistics with their p-values.
So `w` has to be supplied, and §6 decides how.

**The reasoning institution is gone.** The pilot cites Eigenius §4 — "justification logic is an
attached institution" — which was true when the spec was written and is not true now. P7 retired
`reasoning:qc_validate_justification` and its siblings; the algebra moved into the kernel and a
bootstrap test asserts those IRIs no longer resolve. κ–τ attaches to the chain directly, as
statistics does. Nothing in the pilot depends on the retired surface, so this costs the pilot
nothing but a corrected sentence.

**33, not 52.** The spec's 52 is the paper's conclusion count including the narrative layer. The
recomputable chain carries 33. The δ ≡ 0 reproduction gate is therefore 33/33, and the pilot
should say which population it is re-adjudicating before the margin sweep, not after.

**No rivalry vocabulary.** Rival, active, margin, interaction and activation have no declarations
anywhere. §5 mints them.

**λ has no stated provenance.** The paper fixes `λ ∈ (0,1]` as the interaction-scaling parameter
in the synthesis operator, and its provenance table lists `w, κ, κ_o` as estimable and `τ, ε, δ`
as governed. λ appears in neither row. It scales how much interaction moves a score, so treating
it as governed is the conservative reading, and §5 declares it on the policy. This is a question
for the framework's author, not a decision we should make silently.

## 5. The vocabulary

Namespace `urn:eigenius:kappatau`, prefix `kt`. New ontology `ontologies/kappatau/kappatau.esl`,
outside the bootstrap chain, so it costs no reseed.

**The governed parameters, declared and attributed.**

```
class kt:CommitmentPolicy {
    requires kt:threshold, kt:activation,
             kt:margin_form, kt:margin_coefficient, prov:was_attributed_to;
}
property kt:threshold        : core:float    // τ, in [0,1]
property kt:activation       : core:float    // ε, in [0,1] and below τ
property kt:margin_form      : core:resource // δ: zero | by_rivalry | by_stakes_and_rivalry
property kt:margin_coefficient : core:float  // its scale factor c, in [0,1]
```

`prov:was_attributed_to` is required, not recommended. A governance parameter with no owner is the
thing the framework exists to prevent, and `justification:Declaration` already sets the precedent
of requiring attribution rather than recommending it.

`kt:margin_form` names one of three declared families and `kt:margin_coefficient` scales it,
rather than a term. §10 says why: the program AST has no arithmetic, so a chain-resident δ lambda
is not expressible. `zero` is the degenerate policy and the reproduction gate, named rather than
spelled as a zero coefficient so a policy announces that it is first-past-τ.

**The epistemic inputs, each its own resource with its own trace.**

```
class kt:Hypothesis          { requires eigentt:proposition, prov:was_attributed_to; }
class kt:PlausibilityEstimate{ requires kt:estimate_of, kt:weight, kt:estimation_protocol; }
class kt:InteractionEstimate { requires kt:between, kt:kappa, kt:estimation_protocol; }
```

`kt:Hypothesis` is not a `justification:Declaration`. A Declaration is an assertion by an
accountable agent that a proposition holds; a rival hypothesis is entertained, not asserted.
Reusing Declaration would have the pilot asserting every rival it reconstructs.

Each estimate carries its own trace, and that is what fixes its grade: a
`prov:DeclarationTrace` where the value was expert-elicited or operational, a
`prov:ObservationTrace` where it was read off an embedding. This is the spec's "each estimate
itself committed as a resource with its provenance grade", and it is also what keeps the composite
warrant honest — no application forms over an estimate, so nothing is entailed about the next one.

**The term vocabulary, mirroring the statistics axioms.**

```
axiom kt:score_of : core:string -> core:string -> core:float   // (assessment, hypothesis)
axiom kt:Commits     : core:iri -> Prop -> Prop
axiom kt:SuspendedAt : core:iri -> Prop -> Prop
```

`Commits` takes the **policy IRI**, not a bare τ. The audit trail then runs to an attributed
resource carrying τ, ε, δ and λ together, rather than to a float that says nothing about who chose
it. `core:iri -> Prop -> Prop` is the witness families' signature, so the shape is already carried
by the term language.

## 6. Where `w` comes from

**Declared, per conclusion, attributed.** Not derived from the chain.

The alternative is to compute a weight from what the chain does hold — p-values, warrant grades,
the shape of the justification term. Every version of that invents a scoring rule the framework
does not have, and publishes it as if it were the framework's. A declared weight with an owner is
both honest and exactly what the paper's middle row licenses: `κ` may be "computed, learned, or
expert-calibrated", and `w` is "primarily learned or computed" in deployments that have a neural
front end, which this chain does not.

Consequence for the pilot: eliciting 33 weights is Phase 1 work for the STAKE side, and it is the
one input the chain cannot supply. The spec should be corrected on this point before the phase
plan is agreed.

## 7. The institution

One in-process Rust institution, one AutoOnLoad QueryClass, following the statistics template.

```
class kt:CommitmentAssessment {
    requires kt:subject,               // the justification:Conclusion under adjudication
             kt:policy,                // a kt:CommitmentPolicy
             kt:rival_set,             // the reconstructed rivals
             kt:plausibility_estimates,
             kt:interaction_estimates;
}

resource kt:qc_assess_commitment : institution:QueryClass {
    institution:query_class    = kt:CommitmentAssessment;
    institution:result_class   = institution:Verdict;
    institution:dispatch_role  = [dispatch:auto_on_load];
    institution:query_handler  = kt:proc:assess_commitment;
    institution:institution_ref = kt:kappa_tau_institution;
    institution:permitted_verdicts = [ institution:'Verdict-Holds',
                                       institution:'Verdict-Undecidable' ];
}
```

`institution:Verdict` and not a κ–τ subclass of it, because Rule 25 closes an inductive to
subclassing from a later layer — §10 records that. The gate verdict carries nothing beyond its
constructor, so there is nothing for `institution:result_properties` to declare either.

`permitted_verdicts` is D90 step 5, and κ–τ is the reason it exists. Below threshold means *do not
commit to φ*, never *this chain is invalid*.

**A third outcome the draft did not separate: the institution declining to RUN.** An active rival
nobody weighed makes an assessment unadjudicable. That is not a governance state, and returning
`Undecidable` for it would publish a verdict a reader cannot tell apart from a real `SuspendedAt`.
It returns an error instead, which rejects the commit. §10 records the error variant that took.

**The output is one derivation per assessment**, a `kt:CommitmentDecision` carrying
`eigentt:proposition` plus the audit numbers as ordinary float properties — the same split
statistics uses, where the proposition is `stats:lt(mean_diff_of(s), 0.0)` and the statistic and
p-value ride alongside as plain floats.

- **Commit**: proposition `kt:Commits(policy, φ)`, gate `Holds`.
- **Suspend**: proposition `kt:SuspendedAt(policy, φ)`, gate `Undecidable`, with one
  `kt:RivalMargin` record per active rival carrying its score, the lifted interaction, the margin
  demanded, the separation achieved and whether it cleared — plus `kt:binding_rival`, the one that
  failed by the widest gap and so the one to resolve first.

**Why the rivals and margins are properties, not arguments of the proposition.** The collaborator
proposed `SuspendedAt(τ, φ, rivals, margins)`. Carrying a list inside a Prop-level term requires
list-typed arguments the term language does not have, and it puts the audit record where proof
irrelevance and hashing would have to handle it. The proposition states what is claimed; the
resource records what the run saw. Nothing about the suspension's auditability is lost: the
decision resource is chain-resident, queryable, and carries every rival by IRI. This is the
concrete form of the two-level shape D90 describes, where the gate attests runnability and the
derivations carry the per-item decisions.

## 8. The justification term

The point of the whole exercise, and the reason `survives_without` answers the pilot's question:

```
app( declared(policy, SCORES -> kt:Commits(policy, φ)),
     app( declared(scoring_plan, ESTIMATES -> SCORES),
          observed(estimate₁, …) ) )
```

Composite, three levels deep, with each estimate a separate `observed` or `declared` leaf. The
policy is one declared leaf; the scoring plan is another. Remove one estimate and
`survives_without` answers; ask which estimates the commitment rests on and `leaves_of` answers.

**This is the shape a consumer builds, and the institution does not build it.** What the
institution emits is the PROPOSITION and the audit record: `kt:Commits(policy, φ)` on a
`kt:CommitmentDecision`, with one `kt:RivalMargin` per active rival. Grounding it is a separate
act, the same separation statistics already has — that institution emits a
`StatisticalAnalysisResult` and something else writes the `justification:Conclusion` whose term is
`app(declared(plan), observed(sample_set))`. Writing the κ–τ term takes a declaration that the
scoring plan denotes a function of its input, plus an observation trace on each estimate, and both
are the pilot's to author because both are claims about the pilot's own protocol. Nothing is
missing from the kernel for it: the leaves and the constructors are the ones the WRN chain already
uses.

**The semantic gap stays open, deliberately.** The institution establishes `Commits(policy, φ)`,
not `φ`. Crossing to `φ` requires a declared bridge `Commits(policy, φ) → φ` attributed to an
owner, exhibited as its own leaf in any term that uses it. Nothing in D90's contract enforces the
distinction — a slot is checked for holding a well-formed proposition, never for which predicate it
names. The bridge is discipline backed by attribution, and the pilot should say so rather than
imply the type system prevents the collapse.

## 9. The gate

**δ ≡ 0 must reproduce the published adjudication.** One policy with the zero margin form, one
assessment per conclusion, every gate `Holds`. This is the acceptance test for the institution and
it runs before any margin sweep. A failure here is a defect in the scoring implementation, not a
finding about the chain it ran over.

It runs today over a fixture rather than over the WRN chain, because there are no weights on that
chain to run it on. §10 says what changes when there are.

The margin sweep then instantiates one policy per grid point and re-runs. Each conclusion's
suspension margin is the smallest coefficient at which its gate turns `Undecidable`. Because every
policy is a chain resource and every decision cites the policy it ran under, the sweep's output is
a table the chain itself can be queried for rather than a report generated beside it.

## 10. What landed, and what did not

**Steps 1 through 4 landed `2026-09-13`**, on the same branch as D90.

| step | state |
|---|---|
| D90's six steps | landed — `kernel/src/institution/result_contract.rs` |
| the κ–τ ontology | landed — `ontologies/kappatau/kappatau.esl`, out of chain, no reseed |
| the crate | landed — `crates/eigenius-kappatau`, 15 unit tests and 6 end-to-end |
| the δ ≡ 0 gate | landed **over a fixture**, not over the WRN chain |
| weights and rivals | not started — this is where the STAKE side begins |
| the margin sweep | not started — analysis, not kernel work |

**Why the gate runs over a fixture.** It cannot run over the WRN chain, for the reason §4 gives:
there are no weights there to run it on. `crates/eigenius-kappatau/tests/fixtures/pilot_shape.esl`
is the pilot's shape at scale one — a chain conclusion, three reconstructed rivals, and three
policies whose margins disagree — and it says in its own header that the numbers are fixture
numbers with a fixture owner. Substituting elicited weights for real conclusions turns it into the
pilot; nothing else about it changes.

### What building it found

**δ cannot be a chain-resident lambda today.** The program AST has `Apply`, `Lambda`, `Var`,
`Literal`, `Case`, `Map`, `Reduce` and `Component`, and no arithmetic at all: every number that
gets multiplied is multiplied inside a `Component` dispatched to a runtime. So `λ τ x. c·τ·x` is
not expressible, and the lambda-valued slot §5 pointed at as precedent, `institution:transformation`,
bottoms out at a `Component` for exactly this reason. δ is a declared FORM plus a coefficient
instead — `zero`, `by_rivalry`, `by_stakes_and_rivalry` — each satisfying the paper's constraints
by construction, evaluated in Rust as the rest of the scoring semantics already is. D86's numeric
primitive core is what would change this. A named form is also more inspectable than a lambda,
which a governed parameter needs: a reviewer reads one family and one number.

**ESL cannot name a materialised constructor class by qualified name.** `institution:Verdict-Holds`
lexes as a subtraction, because `-` is not an identifier character. The quoted-segment form,
`institution:'Verdict-Holds'`, admits exactly the `[A-Za-z0-9_-]` charset and was already there.
Worth knowing before the next ontology needs to name one.

**The subject conclusion is scored like any other explanatory token.** `kt:estimate_of` and
`kt:between` admit a `justification:Conclusion` as well as a `kt:Hypothesis`. The commitment
condition compares the subject's score against each rival's and treats them uniformly; the subject
differs only in already being on the chain with its own justification term. Narrowing those slots
to hypotheses would have forced a second slot for the subject's weight and let the two drift.

**A structural failure is not a suspension, and the error enum had no way to say so.** An active
rival nobody weighed makes an assessment unadjudicable. Returning `Undecidable` for it would
publish a verdict a reader cannot tell apart from a real `SuspendedAt` — which matters more here
than for most institutions, because κ–τ's `Undecidable` IS a meaningful epistemic state rather
than a shrug. So the institution declines to RUN, and the dispatch records that as a validation
error that rejects the commit. `InstitutionError` had four variants and none of them said "this
input is not one I can adjudicate", so `InvalidInput` was added. Rule 1 already requires every
slot on the assessment, so what reaches the handler malformed is a reference resolving to the
wrong thing or a number nobody supplied, not a missing property.

### What a review found after it landed

Five confirmed defects, two of which flipped a verdict. All fixed; recorded because each
is a way to get an institution like this wrong.

**A silently dropped estimate deleted a rivalry and turned a suspension into a
commitment.** An interaction estimate whose pair was not exactly two hypotheses was
skipped, with no error and no record. It fell back to κ = 0, the rival stopped being
active, and the governed policy returned `Holds`. One missing entry in one array produced
exactly the premature convergence the framework exists to prevent. `kt:between` now
declares `min_length 2; max_length 2` so the chain refuses it, and the handler refuses it
too rather than dropping it.

**Conflicting estimates resolved by array position.** Two plausibility estimates for one
hypothesis, or two interaction estimates for one pair, were last-write-wins over a map.
A second weight of 0.10 beside a first of 0.81 dropped a rival below activation and
flipped the verdict. Two independently elicited, independently attributed estimates
disagreeing is exactly what "each estimate is its own resource with its own trace"
produces; picking one by authoring order decides a governance question silently. Refused
now, on the same rule the design already stated for a missing weight.

**Nothing was range-checked, and out-of-range values changed verdicts.** Every numeric
property stated its range in prose and declared none. A policy with ε ≥ τ validated: no
rival is ever active, the decision records NO margins, and that is indistinguishable from
the genuine finding that a conclusion faced no live opposition. τ ≤ 0 validated and made
every conclusion trivially commit-worthy while δ evaluated to 0, byte-identical to a
legitimate first-past-τ policy. Weights above 1 and κ outside [−1,1] validated and were
published. All five now carry `min_value` / `max_value`; ε < τ is a relation between two
slots, so the institution checks it and refuses.

**The margin clamp recorded a number the policy never named.** `δ` was clamped to [0,1],
so a coefficient of 5 published a demanded margin of 1.0 where the policy asked for 4.0.
That is the Band-Aid shape: a clamp guarding against a bad coefficient instead of a
declared bound eliminating it, and it corrupted the audit record, which is half of what
auditable suspension is for. The coefficient is bounded now and δ lands in the codomain
by construction.

**An unweighed COMPATIBLE alternative refused the whole assessment.** The weight lookup
ran before the interaction test, so an alternative with positive κ — one that can never
impose a margin — made the assessment unadjudicable, with a diagnostic calling it an
"active rival". The fixture itself carries such an alternative, so this was the expected
shape of a rival set, not a corner case. κ is tested first now.

### Two things removed rather than kept

**The general lifting was wrong in both directions while looking general.** `κ*` took
atom slices and returned 0 when they were equal. The paper's structural equivalence is
generated by commutativity and deliberately NOT by associativity, so two orderings of the
same atoms must give 0 and two bracketings must give the average — and neither is a
function of atom sets. The function returned the average for the first and 0 for the
second. It is narrowed to the atomic case, which is all the vocabulary expresses, with
the reason a general one needs the TERM written down.

**λ went with it.** `kt:interaction_scale` was required on every policy and read into the
institution, and its only consumer was the synthesis operator, which nothing calls. A
margin sweep varying λ would have produced identical output at every grid point with no
indication why. The justification ontology already set the precedent by deleting two
properties that had no reader. When a ⊗ constructor arrives, the lifting, the synthesis
operator and λ arrive together, because they all need the same term structure.

That also answers one of §11's open questions before it was asked: the paper states κ is
symmetric, so reading the table symmetrically is the paper's own convention rather than
a choice to put to the author.

### One thing found in the kernel, not here

The emitted decision is committed by the structural-followup pipeline, whose phase slice
is `[build, persist]` with no `structural_validate`. So Rule 21 never runs on an emitted
term at commit. The κ–τ term passes when the validator is run over it by hand, and the
D90 boundary check covers the shape, but the layer-validation pass that would check the
term itself does not execute on that path. Pre-existing and outside this note; worth its
own look.

**Rule 25 shaped D90, not this.** Recorded there: an inductive is closed, so no institution can
declare a `Verdict` subclass, and the closed output contract lives on the QueryClass instead.

## 11. Open questions

- ~~λ's provenance.~~ **Moot for now.** λ scales the synthesis operator, which needs a ⊗ the
  vocabulary does not declare, so no policy carries a λ. The question returns with ⊗.
- **Where semantic κ comes from.** `crates/eigenius-embedder-candle` exists. Whether the pilot
  wires it or supplies declared κ throughout decides whether any estimate carries an
  `ObservationTrace` at all. Declared-throughout is a complete pilot and a simpler one.
- **Whether `sc` should be kernel-evaluable.** The scoring arithmetic is computed in Rust, as
  statistics computes its p-value. D86's numeric pivot would let a checker decide the threshold
  comparison rather than assert it. Nothing in this design waits on that.
- **Rival reconstruction is judgement work.** Three sources are named in the spec; none is
  mechanical. Every rival is therefore an attributed resource, and the pilot's findings are
  conditional on that reconstruction, which the chain will record.
- **Which margin families the pilot wants.** Three are implemented: `zero`, `by_rivalry`
  (δ = c·x) and `by_stakes_and_rivalry` (δ = c·τ·x). The paper constrains δ but names no family, so
  these are a reading of its prose. A fourth is an ontology entry and one match arm; the question
  is which the author would sweep over.
- ~~Whether κ should stay symmetric.~~ **Answered by the paper**, which assumes κ symmetric.
  Reading the table symmetrically is its convention, not a choice to put to the author.
