# End-state scenarios, and the integration gaps they expose

*Written `2026-09-06` at `21bfb3f`. Companion to
[`provenance-to-justification-analysis.md`](provenance-to-justification-analysis.md), which describes
what is built. This one asks what the system should be able to **do**, end to end, and derives the
gaps from that.*

**Status `2026-09-06`, revised the same day.** Scenario A is **built and passing**
(`crates/eigenius-lean/tests/notebook_fixture_test.rs::three_grounds_compose_in_one_certificate`),
G6 is **answered**, and fixing the existing Lean test turned up a defect described in §G7. Scenario
B and gaps G1, G2, G5 are untouched. The premise below is what held before that work.

**The premise.** Each ground works and is tested. What had never been built was a single artifact in
which all three compose, and the seam that went untested was the one that matters: **`verified` had
never been an argument to `app` — zero occurrences tree-wide.** `app` is where a ground does work; it
is how a leaf discharges a premise of a declared implication, and it is the shape of every real
conclusion in the WRN chain. `Declared` and `Observed` are composed that way dozens of times.
`Verified` never once.

---

## Scenario A — a proved fact discharges a premise · **BUILT `2026-09-06`**

`crates/eigenius-lean/tests/notebook_fixture_test.rs::three_grounds_compose_in_one_certificate`.
The certificate as built is
`app(app(declared(RULE, H -> OBS -> E), verified(CLAIM, H)), observed(INTAKE, OBS))`, over the Lean
demo's `Healthy(patient_1)`; the numeric-threshold variant the section below describes is Scenario
B's, and waits on G1. Assertions: the layer commits under full validation; `support` returns one
alternative; `leaves_of` returns exactly one IRI for each of `Verified`, `Observed` and `Declared`;
and repointing the `verified` leg at `claim_patient_2_healthy` is **refused**, so the positive case
is not passing on a lookup that admits anything.

**The smallest thing that exercises the seam.** Crate-local, fast, no WRN dependency. Lives in
`crates/eigenius-lean/tests/`, because Verified must come through the Lean institution's AutoOnLoad
check and that is where the institution is registerable.

**The chain, in two layers.** A generated Eigon-JSON layer carrying the Lean proof term and its
payload, and an ESL layer above it carrying the rest:

| resource | ground it supplies |
|---|---|
| a recorded measurement + `prov:ObservationTrace` | `observed(m, Asserts(m))` |
| a `justification:Claim` whose proposition is a numeric relation, proved in Lean → institution emits `prov:VerificationTrace` with `prov:judgement` | `verified(claim, STAT)` |
| a `justification:Claim` carrying `STAT -> DC` + `prov:DeclarationTrace` | `declared(bridge, STAT -> DC)` |

**The certificate.** `app(declared(BRIDGE, STAT -> DC), verified(CLAIM, STAT))`, with the observed
leg discharging whatever premise the plan claim needs.

**What it asserts**, none of which anything currently checks:

1. The layer commits clean — so a `Verified` witness minted by `emit_from_trace` from a
   `prov:judgement` is consumable by an `app` premise.
2. `support` returns leaves of all three `Ground` kinds from one certificate.
3. `leaves_of(term, Ground::Verified)` is non-empty. **It never has been on any real conclusion.**
4. `survives_without(lean_claim_iri)` is `false` — the proved leg is load-bearing.
5. A witness from the trace route and one from the kernel-checked route are interchangeable at an
   `app` premise. They live in different crates today and have never met.

## Scenario B — the grade climbs on the flagship claim · **BUILT `2026-09-06`, in the reduced form below**

**The real one.** Scenario A proves the machinery composes; B proves the design's thesis — *Verified
grows from the bottom, and every step moved out of an asserted function and into a checked term
climbs a rung.*

Today, `wrn:concl_refine_recomputed`
(`experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl:94`) reads:

```
OBS  = core:Asserts(SS)
STAT = stats:lt(stats:spearman_rho(SS), 0.0)
DC   = onco:DependencyCorrelatesWithMutatorLoad("WRN", "MSI")

app(declared(BRIDGE, STAT -> DC), app(declared(CLAIM, OBS -> STAT), observed(SS, OBS)))
```

`CLAIM` — *"this plan applied to this sample set yields this statistic"* — is **Declared**, and it
carries the threshold comparison inside it. The comparison is the part a checker could establish.

**End state:** split it. The value stays Observed, the comparison becomes Verified by a Lean proof,
the domain lift stays Declared, and the conclusion is unchanged but strictly better warranted. The
projection surface then reports a different answer for the same claim, which is the observable
result.

`stats:lt` is already in the externalizer's numeric correspondence table
(`crates/eigenius-lean/src/externalize.rs:920`), so the relation is Lean-expressible today.

### What was built, and what it deliberately leaves out

`proving_one_step_removes_one_declaration_from_what_a_conclusion_rests_on`
(`crates/eigenius-lean/tests/notebook_fixture_test.rs`) is B's **thesis** without B's statistical
decomposition. One ESL source carries a `LEAF` placeholder, substituted twice:

```rust
let asserted = esl.replace("LEAF", "declared(\"urn:eigenius:scenario:b:asserted_healthy\", H)");
let proved   = esl.replace("LEAF", "verified(\"urn:eigenius:demo:lean:claim_patient_1_healthy\", H)");
```

Both commit over the same Lean fixture, under unfiltered validation. What the test asserts:

| | `leaves_of(Declared)` | `leaves_of(Verified)` |
|---|---|---|
| asserted | `{asserted_healthy, eligibility_rule}` | `{}` |
| proved | `{eligibility_rule}` | `{claim_patient_1_healthy}` |

`is_fully_verified` is **false for both**, asserted explicitly so its absence is not read as a
defect: the domain bridge is Declared either way, and a conclusion resting on a declared bridge is
not fully verified however much below it is proved. What moves is the leaf set.

**Left out:** the statistical decomposition above — splitting `CLAIM` so that `stats:lt` becomes the
Verified step. That is what G1 gates, and separately it is what the `Rat` pivot (D86 §4) gates, since
a *procedural* proof about a threshold needs order lemmas `Float` does not have. The reduced form was
built first so the tracing machinery was proved before either of those arguments was spent.

**First procedural target, once the pivot lands: threshold weakening**, `lt(x,T) → T ≤ T' → lt(x,T')`.
One Mathlib order lemma, compositional in the way the chain actually steps (a claim proved at a tight
threshold discharging a premise needing a looser one), and `x` stays opaque throughout — so no dataset
is ever represented as `Rat`. It is impossible over `Float`, which is what makes it a test of the
pivot rather than an assumption of it. Bonferroni needs a union bound over a *set* of claims and
reaches into the multiple-testing institution D52 §11 scopes out; the one-sided/two-sided relation
would prove a property of a refinement v1 does not implement.

---

## The gaps

### G1 — the proposition shape · **DECIDED `2026-09-06`: do it**

**The decision, and its reason.** Build the decomposition — *to verify the end-to-end mechanism*.
Where the boundary between the statistics and Lean institutions should finally sit is a follow-on
task, argued against machinery that runs rather than in the abstract.

That reframes Scenario B. It is not required to be the right final decomposition of statistical
warrant; it is required to be a working instance of the shape, so the boundary question has
something to be asked of. In particular it does **not** oblige a rewrite of the statistics
institution's emission contract, which is the expensive half and the half the boundary question is
actually about.

**The shape, three legs composed by `app` — no new constructor needed:**

| leg | ground | proposition |
|---|---|---|
| the plan yields this number | Declared | `Asserts(SS) -> Eq(rho(SS), v)` |
| that number puts the statistic below the threshold | **Verified** | `Eq(rho(SS), v) -> lt(rho(SS), 0.0)` |
| the sample set was recorded | Observed | `Asserts(SS)` |

```
app( verified(LEMMA, Eq(rho,v) -> lt(rho,0)),
     app( declared(PLAN, Asserts(SS) -> Eq(rho,v)),
          observed(SS, Asserts(SS)) ) )        : Certificate(lt(rho(SS), 0.0))
```

**What stays Declared, and why that is the honest part.** *"This plan, applied to this input,
yields this number"* is not established by any execution — the paper is explicit that a run record
is provenance and not warrant — so it keeps an accountable owner. The decomposition moves the
arithmetic to Verified and leaves the reproducibility claim exactly where it was. That is the
objection to doing this at all, and the answer is the one above: the mechanism has to work before
the boundary is worth arguing.

**Original statement of the gap, retained:**

### G1 (as found) — the proposition shape the chain authors is not the shape Lean can prove

`STAT` is `stats:lt(stats:spearman_rho(SS), 0.0)`. `stats:spearman_rho(SS)` is an **opaque
application over a sample-set IRI** — it has no normal form, so there is nothing for a checker to
evaluate and no Lean proof to write. D86's table maps `stats:lt`, and D74 §4.8 translates `LitFloat`,
so what Lean can prove is `stats:lt(<float literal>, 0.0)`.

The number exists: the statistics institution commits `stats:computed_statistic : core:float`
(`ontologies/statistics/statistics.esl:1127`). **No certificate in the tree references it.**

Closing this is a modelling decision, not plumbing, and it is the decision Scenario B turns on:

- **Option 1 — the claim quantifies over the recorded value.** The `justification:Claim` proposition
  becomes `stats:lt(v, 0.0)` where `v` is the committed literal, and a separate Declared premise ties
  `v` to `spearman_rho(SS)`. Honest — that tie *is* an assertion that the pipeline computed this
  value from this input — and it keeps the proved part small and genuinely checkable.
- **Option 2 — make `spearman_rho` reduce.** Requires the kernel to evaluate the statistic, which it
  does not and should not: the statistics institution *"defines a satisfaction relation but provides
  no proof language."*

Option 1 is the one the paper's grounds taxonomy already describes: the run record is provenance, the
value is Observed, the tie is Declared, the comparison is Verified.

### G2 — D86's numeric core has no consumer

The correspondence table is built and reviewed as TCB (`externalize.rs:883-976`, five relations, two
asserted and three derived). **Nothing in the tree externalizes a `stats:*` claim to Lean.** It is
feature work with no scenario driving it — which is what this note exists to fix, and Scenario B is
its first consumer.

### G3 — the two Verified routes never meet · **partly closed by Scenario A**

**What Scenario A closed.** The *trace* route — a `Verified` witness minted by `emit_from_trace`
from a `prov:VerificationTrace`'s `prov:judgement` — is now consumed at an `app` premise and
projects as a `Ground::Verified` leaf.

**What remains.** The *kernel-checked* route (`emit_from_reasoning_sentence`, reading a
`justification:Conclusion`'s `justification:proof`) is still untested against it, and nothing
asserts the two mint interchangeable keys. That is unchanged by Scenario A because
`justification:proof` is populated nowhere in the tree — see the note below on whether that route
should exist at all.

`emit_from_trace` (a `prov:VerificationTrace` carrying `prov:judgement`) and
`emit_from_reasoning_sentence` (a `justification:Conclusion` carrying `justification:proof`) are
sibling paths in one function family, tested in different crates. Nothing asserts they mint
interchangeable keys.

Related: **`justification:proof` is populated nowhere in the tree.** The kernel-checked route is real
machinery — the slot is `class_types eigentt:Judgement`, so the uniform check-mode rule validates it
at commit — with no producer, because the kernel checks a supplied term and cannot search for one,
and no domain proposition has a term an author could supply.

### G4 — no `observed(` in any Rust test · **CLOSED by Scenario A**

`observed` appears only inside WRN ESL chains, exercised incidentally. `certificate_admission.rs` —
820 lines, and its own doc calls it *"the witness machinery end to end"* — builds `declared` only.
That doc comment overclaims and should be corrected either way.

### G5 — the in-process Activity gap

`w3c-prov-mapping.md` §5.2. `RuntimeInvocation` is built only when the substrate returns a partial
record; in-process institutions return `partial_invocation: None`
(`institution/in_process_registry.rs:217`). Statistics and Lean both run in process, so neither
scenario produces a `prov:Activity` for the work that generated its evidence.

It blocks no certificate — the grounds are the plan and the inputs, not the run — but a scenario that
claims to demonstrate provenance end to end has a hole exactly where the runs are.

### G6 — fixture shape · **ANSWERED, and Scenario A is built on it**

**Yes.** `land()` was refactored into a reusable `Harness` (backend, storage, institution runtime,
bootstrap head) with a `commit(parent, resources)` method, so a second layer commits against the
same chain. Scenario A parents on the **`verdict_provenance` Sibling** — where AutoOnLoad puts the
`VerificationTrace` — because it is a child of the user layer and therefore the deepest point of the
chain, even though the `main` ref does not name it.

The original question, kept because the reasoning still applies:

`notebook_fixture_test.rs` loads one generated Eigon-JSON fixture through the D41 commit
orchestrator. Both scenarios need an ESL layer committed **above** that, in the same test, with the
institution registered. Ordinary chain mechanics, but no existing test does it — worth confirming
before writing either scenario rather than discovering it midway.

---

### G7 — a narrowed test hid a malformed resource · **FIXED `2026-09-06`**

`a_certificate_citing_the_verified_claim_type_checks` authored its judgement as
`type_expr(alias … in holds(…))`. `type_expr` yields a **Term**, so `holds` inside one encodes as
`eigentt:Term-App` rather than `eigentt:Judgement` — `justification:judgement` violated its own
`class_types`, and the resource could never have committed.

The test could not see it: it filtered validation to `TermIllTyped | TermMalformed`. What it proved
was real — the certificate type-checks, and the near-miss is refused naming `IsVerifiedAs` — but a
test narrowed to the rules it is about cannot notice that the resource carrying its subject is
malformed.

Fixed both ways: `holds` moved to the top of the slot with its `term` and `type` arguments
separately `type_expr`-wrapped (what the WRN chain authors), and the positive assertion now runs
**unfiltered**. It passes clean with no filter, so the narrowing was buying nothing. The near-miss
half still filters to `TermIllTyped`, correctly — there the filter *is* the assertion.

**The general lesson, worth carrying into Scenario B.** Both halves of this defect are the pattern
that motivated these scenarios: a unit test written for one rule, passing, while the integration it
implies was never exercised. It is also why Scenario A drives the full commit pipeline rather than
`LayerBuilder::build()`.

---

## Ordering, and what blocks

**B4's reseed blocks running any of this against a persisted store.** The bootstrap manifest has
moved four times on this branch (#235's descriptions, B2's merge, B1's implicit binders, B3's
`core:iri`); every persisted store is unresumable until it runs.

Then:

1. ~~**G6**~~ — done. The two-layer shape works.
2. ~~**Scenario A**~~ — done. Closes G3 and G4 and exercises the seam.
3. **G1** — decide the proposition shape. This is the one that needs a person, not a patch, and
   Scenario B is blocked on it.
4. ~~**Scenario B**~~ — built `2026-09-06` in the reduced form; closes G2. The full form, with
   `stats:lt` as the Verified step, still waits on G1 and on D86 §4's `Rat` pivot.
5. **G5** — independent of both; needed before any claim that provenance export is complete.
6. ~~**B6**~~ — done `2026-09-06`. See §B6 below for what it cost and the one edge it drops.

Scenario A was deliberately first and deliberately small: it settled whether the machinery composes
before anyone spent the modelling argument G1 requires. It did, and it found G7 on the way.

---

## B6 — `core:mentions` reads the declaration · **DONE `2026-09-06`**

`s.starts_with("urn:")` is gone from `kernel/src/layer/term_mentions.rs`. Two dispatches replace it,
and **both** are required — the note's mechanism was only half of it.

**Property dispatch** — for a value resource's own slots, resolve the property and read its
`core:data_type`. This is what reaches a `ConstRef` target, which is not an application.

**Spine dispatch** — at an `App` spine whose head is `CtorApp(I, c)`, resolve `I`, find `c`, and
recover its explicit argument types in order: `core:arg_types` for a positional constructor,
`core:ctor_type`'s Pi chain for a telescope one, skipping binders named in `core:implicit_args`.
This is what reaches the **grounding leaves**, and the note's mechanism did not: a
`Certificate.declared(iri, P)` encodes as an `App` spine and the encoding erases which argument
each position fills. Omitting it drops every premise citation from the index, which is the edge set
P6 enforces well-foundedness over. The first attempt did exactly that and
`a_grounding_leafs_string_iri_becomes_a_mentions_triple` caught it.

**It needed a bootstrap edit the note did not anticipate.** `ConstRef.iri` and `CtorApp.decl_iri`
were declared `core:string`; both name a declaration and always did. B3 declared three *other*
leaves IRI-valued and did not reach these, which went unnoticed precisely because the heuristic
recovered them without the declaration. Retyped to `core:iri`; `CtorApp.ctor_name` stays
`core:string`, since constructors have no chain-resolvable identity (D79 §2.2.1). The `core` layer
moved, the manifest pin is updated with a changelog entry, and **the reseed is owed** — B4.

### The edge B6 drops, deliberately and not silently

**A `spec_poly` instance that is an IRI stops being a mention.**

```
spec_poly(core:string, fun (x : core:string) => …, "urn:eigenius:demo:screen:EIG_0291", …)
```

`x`'s declared type is the bound variable `T`. No declaration records that *this instantiation*
is a reference, so the strict rule reads it as data. Two sites in the tree do this —
`crates/eigenius-statistics/tests/fixtures/d39_composition.esl:193` and
`demo/prose-to-formulas-v2/inference.esl:124` — both the old `spec_str` shape, `T := core:string`.
The WRN chain is unaffected: its instances are `"WRN"`, never IRI-shaped.

**P6 is unaffected** — the instance is not a premise citation, so cycle detection does not see it
either way. What is lost is the index answering *"which claims mention EIG_0291"* for these,
and the instance genuinely does reference a chain resource.

**It is the `program:value` problem in a harder form.** There, one property serves two roles and
could be split. Here the type is *universally quantified*: `spec_poly` cannot declare
`x : core:iri` without giving up the polymorphism that C1's analysis says is the reason `spec_str`
was merged into it. So the reference-ness lives in the instantiation, where no declaration can
reach it.

Three options, none free, and none taken here:

1. **Accept it.** The index answers over declared references only, and an instance is not one.
2. **Re-add a heuristic for this position alone** — `spec_poly`'s `x` when `T` resolves to
   `core:string`. Narrow, but it is a hardcode of one constructor in a reader that otherwise reads
   declarations.
3. **Give the instance its own slot.** A `spec_iri` variant, or an argument declared `core:iri`
   alongside the polymorphic one. Vocabulary growth for one shape.

### Settled while doing it: the other two `urn:` heuristics are different questions

- `kernel/src/nbe/eval/marshal.rs:35` — `resource_value_to_val` takes a bare `Value` with **no
  property context**. Its own comment already names the principled fix and defers it to "when the
  type checker has full property-type awareness during evaluation". Same diagnosis, and it needs
  property context threaded through marshalling.
- `kernel/src/program/expr.rs:903` — `program:value` is declared `core:resource` with
  `domain: [program:Let, program:Literal]`: one property doing two jobs, so reading the declaration
  cannot disambiguate. And unlike the index, `parse_literal` decides `Exp::Var` against
  `Exp::LitString`, so a wrong guess changes what a program **means**. That is an ontology-shape
  problem — split the property — not a reading problem.
