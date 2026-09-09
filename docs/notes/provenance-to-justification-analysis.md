# Analysis — how provenance reaches a justification term

*Written `2026-09-06` at `21bfb3f` on `numeric-core-and-verification-judgement`. Read against code
and ontology, not against the design notes; where the two disagree the tree wins and the disagreement
is recorded.*

**The framing this answers assumed a gap that is largely closed.** The brief was that the remaining
work is "connecting provenance and evidence into justification terms", and that a container
abstraction may be needed to replace the deleted `DeclaredResource` / `ObservedResource`. That
container exists — `justification:Claim` — and all three grounds are wired end to end and exercised
by the WRN publication chain. What remains is narrower, and different in kind, than the framing
suggests. §6 lists it.

---

## 1. The vocabulary, as it now stands

Three namespaces, cleanly split by job.

| namespace | holds | file |
|---|---|---|
| `prov:` | Agent, Activity, the four grounding Trace classes, attribution properties | `ontologies/prov/prov.esl`, 369 lines |
| `justification:` | `Certificate`, `Claim`, `Conclusion` | `ontologies/justification/justification.esl`, 343 lines |
| `witness:` | `IsDeclaredAs`, `IsObservedAs`, `IsVerifiedAs` — zero-constructor `Prop` families | declared beside the calculus |

`reflection:` survives as the program-run trace vocabulary (14 structural trace classes —
`FieldTrace`, `MapTrace`, `CaseTrace` and the rest — plus their properties), which is provenance and
correctly not epistemic. **It also still holds `canonical_proposition`**, which is the single most
load-bearing slot in the whole machinery. See §6.4.

---

## 2. How provenance is captured

### 2.1 On entry

A resource entering the chain carries attribution, not a grade:

- `prov:was_attributed_to` — the agent who asserted it (resource-typed, so the agent must resolve).
- `prov:was_generated_by` — the activity that produced it. Uniform across a kernel run and a
  transcribed one, which is what let `ExternalExecutionTrace` be deleted.
- `prov:had_primary_source` — where an observation came from.
- `prov:rationale`, `prov:timestamp`, `prov:observed_at`.

No property anywhere states a grade. That is the design rule the paper's deprecated-patterns list
enforces: *"grades assigned by class membership, by a trace declaring its own grade, or by the
importer that wrote the resource"* are all replaced by computation from stored evidence.

### 2.2 The traces, and the map to grounds

Four classes ground a witness; the map is four match arms in
`kernel/src/layer/witness_admission.rs:235`:

```
prov:DeclarationTrace  → Declared
prov:ObservationTrace  → Observed
prov:VerificationTrace → Verified
prov:ProgramTrace      → None      (deliberately)
```

`ProgramTrace` grounding nothing is the paper's position, not an omission: a computed claim does not
rest on the fact that a computation ran. It rests on the *declaration* that the plan denotes a
function, and on the inputs. The run record is provenance.

**This map lives in Rust and that is intended.** It is the constant specification the paper names as
a TCB element — *"the kernel asserts the remaining two as proof constants under a defined constant
specification"*. Moving it into chain data would put a TCB element somewhere a layer can extend. B5
attempted exactly that and was refuted against the paper; C2a proposed merging the three witness
families and was declined. Both are closed decisions, not open work.

### 2.3 Verified is captured differently, and this is the important asymmetry

`Declared` and `Observed` are **postulated**: the trace exists, so the kernel asserts the witness.
Nothing is checked.

`Verified` is **computed**. A `prov:VerificationTrace` carries `prov:judgement` holding
`holds(logic, t, P)`, and `emit_from_trace` builds the witness key from *the judgement's own type*
rather than from what the target says about itself. The trace also pins
`prov:permitted_axioms`, `prov:checker_identity` and `prov:proof_system`, so the verdict is
recomputable rather than taken on the trace's word.

Two guards keep this from laundering:

- `judgement_proposition_hash` refuses a judgement whose type is a `Certificate(...)` — that would
  say a checker verified the *certificate*, which establishes nothing about the proposition.
- `emit_from_reasoning_sentence` applies the same refusal to a `justification:Conclusion`'s
  `justification:proof` slot, and admits no Verified witness for a conclusion with no proof term.
  This is a deliberate tightening of D54 lemma citation: a lemma is citable as `verified` only if it
  was proved, not merely justified.

The Lean institution is the live producer — `crates/eigenius-lean/src/institution.rs:443` emits
`holds(logic_lean4, Checked(payload), P)` beside its verdict.

---

## 3. The term algebra

`justification:Certificate : Prop -> Type 2`, seven constructors. **There is no separate term type**
— B2 collapsed `justification:Term` into the certificate, because the index was determined by the
certificate and nothing read it that could not read the value. An inhabitant of `Certificate(P)` *is*
the justification term.

| constructor | shape |
|---|---|
| `declared` | `(iri : core:iri, P) → IsDeclaredAs(iri, P) → Certificate(P)` |
| `observed` | `(iri : core:iri, P) → IsObservedAs(iri, P) → Certificate(P)` |
| `verified` | `(iri : core:iri, P) → IsVerifiedAs(iri, P) → Certificate(P)` |
| `app` | `Certificate(A → B) → Certificate(A) → Certificate(B)`, `A` and `B` implicit |
| `sum_l` / `sum_r` | both branches justified; the constructor choice records the preferred one |
| `spec_poly` | narrows a universal to an instance; adds no ground |

`app`'s propositions became implicit in B1 (309 call sites lost two arguments each). `spec_poly` stays
fully explicit because `P` applied to a bound variable is outside the first-order unification
fragment — C1 measured the cost of fixing that at 1,943 characters across 7 sites and declined.

The leaf `iri` is `core:iri`, a primitive type added by B3 so that all three declaration forms
(typed telescope, index telescope, positional) inherit the constraint from one place.

---

## 4. Populating the leaves — what each one requires

The worked instance, from `experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl:94`:

```
OBS   = core:Asserts(SS)
STAT  = stats:lt(stats:spearman_rho(SS), 0.0)
DC    = onco:DependencyCorrelatesWithMutatorLoad("WRN", "MSI")

cs    = app(declared(CLAIM, OBS -> STAT), observed(SS, OBS))
        app(declared(BRIDGE, STAT -> DC), cs)
```

Read off that, each leaf needs exactly three things: **an IRI that resolves, a proposition the target
carries, and a trace of the matching kind targeting it through `prov:resource`.**

**Observed.** The proposition is `core:Asserts(SS)` — the D39 §4.1 default that
`target_proposition_hash` falls back to when the target carries no `canonical_proposition`. This is
the paper's *"a recording occurred"*, and it is why an observation cannot launder into a domain
claim: `Asserts(SS)` is all the leaf justifies.

**Declared.** Two distinct roles, both `justification:Claim` resources carrying a
`canonical_proposition`, both under a `DeclarationTrace`:

- the **plan premise** — `CLAIM` carries `Asserts(SS) -> lt(spearman_rho(SS), 0.0)`, bound to this
  hash-pinned sample set rather than universally quantified. The chain's own comment is the reason:
  a universal would assert that *any* recorded set yields a negative rho, which is false.
- the **domain bridge** — `BRIDGE` carries `STAT -> DC`, lifting the statistical claim to the domain
  claim.

Both are the paper's *"every bridging inference constitutes a declared premise attributed to a
specific owner"*, realised as ordinary `App` premises.

**Verified.** Either a `VerificationTrace` carrying `prov:judgement`, or a `justification:Conclusion`
whose `justification:proof` holds a non-certificate judgement (the self-attesting path).

**Institutions do not emit certificates.** The statistics institution emits a
`StatisticalAnalysisResult` carrying the recomputed proposition; the certificate is authored
downstream. That is consistent — the result is a run record, and a run record is not a ground.

---

## 5. The container abstraction already exists

`justification:Claim`, `justification.esl:187`:

> *A chain-resident resource carrying a proposition — a literature rule, a statistical-to-domain
> bridge, a claim that a plan denotes a function of its input, an instrument reading. REQUIRES
> `reflection:canonical_proposition`. Ground-neutral by design — which of Declared or Observed
> applies is which prov trace the resource carries, not which class it is, which is the separation
> the deleted grade classes collapsed.*

It is authored 85 times in the tree. It does the job the brief asked for and does it the right way
round: the grade classes made the ground a *kind of resource*, so a reading and a rule were different
types of thing; `Claim` makes them the same kind of thing with different origins, and the origin is
the trace.

The `requires` is load-bearing rather than decorative — without it, a resource carrying no
proposition fails as a missing witness at certificate-check time instead of as a missing property at
commit, which is later and further from the cause.

---

## 6. What actually remains

### 6.1 B4 — one reseed, then both baselines

The bootstrap manifest has moved three times on this branch (#235's descriptions, B2's merge, B1's
implicit binders) and B3 is a fourth. **Every persisted store is unresumable until this runs.** Use
the scripted protocols; `measure-parse-rate.sh` builds release, and that is load-bearing — a debug
build overflows the stack in NbE readback and the harness reports it as a grammar gap.

### 6.2 B6 — make `core:mentions` read the declaration

Still open, and confirmed still open: `kernel/src/layer/term_mentions.rs:83` matches
`s.starts_with("urn:")`. B3 declared the leaf `core:iri`; B6 is the consumer half. The mechanism is
already found — each argument of an encoded value is carried under a property named
`<ctor-class>-<arg-name>`, and those are real chain resources carrying `core:data_type`, so the walk
can resolve each definition and treat `core:iri` as a reference. It needs a `&Layer` threaded in
(both callers have one) and it **narrows** the index's mention set, which needs its own tests.

The same heuristic sits at `program/expr.rs:903`; whether that is the same question is B6's to answer.

### 6.3 The in-process Activity gap — the one real provenance hole

`w3c-prov-mapping.md` §5.2. `RuntimeInvocation` is built only when the substrate returns a partial
record, and in-process institutions return `partial_invocation: None`
(`institution/in_process_registry.rs:217`). Statistics and Lean both run in process, so a chain-wide
provenance export today would carry Activities for externally dispatched work and none for the rest.

**This is the item that best matches the brief's framing.** It does not block any certificate — the
grounds are the plan and the inputs, not the run — but it is the gap in *provenance* proper.

### 6.4 Residue, all small, none blocking

| | |
|---|---|
| `reflection:canonical_proposition` | the central slot of the justification machinery, sitting in the namespace that otherwise holds program-run trace plumbing. It did not move when the vocabulary split into `prov:` and `justification:`. 123 authored occurrences |
| `wk::INSTITUTION_EMITTED_DERIVATION` in `is_witness_candidate:201` | attests nothing — `trace_category` gives it no category and `layer_admits_witness` has no arm for it. Its only effect is to set `has_witness_candidates`, which disables the O(1) skip for any layer holding a statistics result. A cost, not a correctness bug |
| `const REASONING_SENTENCE` (`:87`) | now points at `urn:eigenius:justification:Conclusion`. Name only |
| `reflection:InstitutionEmittedDerivation` | still declared, still subclassed by `stats:StatisticalAnalysisResult`. Whether the class still earns its keep once it grounds nothing is a question, not a defect |

### 6.5 `next-steps-after-d88.md` is stale in one place

It marks **B3 as NEXT**; B3 landed in `f629c31`. Anyone picking the note up cold will start on
finished work. B4 and B6 are the live items.

---

## 7. What the coworker was circling

C2 was examined and closed in both halves, and the closure is sound:

- **C2a** (merge the three `witness:Is*As` into one `ChainWitness(category, iri, P)`) — declined. The
  three constants do not disappear, they move from three decl IRIs to three constructors of a new
  inductive plus a value→enum readback, and the independence of the families degrades from a type
  distinction to a value comparison.
- **C2b** (the trace-kind → grade map should live in the ontology) — refuted against the paper, which
  names both proposed shapes in its deprecated-patterns list.

**The abandoned attempts were abandoned for good reasons, and the record says so.** The risk now is
not that C2 is unfinished; it is that the framing survives the decision and someone reopens it.
