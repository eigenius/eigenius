# D89 — The justification vocabulary: factoring and names

`2026-09-08`. Branch `numeric-core-and-verification-judgement`.

**Status: decided, unbuilt.** Records what `ontologies/justification/justification.esl` should
declare and what each declaration should be called, and why the present shape differs. The paper is
[`judgements-and-warrants.tex`](judgements-and-warrants.tex); D81 (`4ad5620`) ran the first cleanup
pass against its Appendix A, removing the reasoning institution and crate, `justification:Projection`,
the `Derived` ground and witness category, and the four `*Resource` grade classes. This is the second
pass over what that one left.

## 1. What each declaration does today

Four jobs are mixed across six declarations, and the classes are where they conflate.

| job | carried by | paper asks for it |
|---|---|---|
| term algebra | `justification:Certificate` + 7 constructors | yes — `JustifiedBy(j, P)` |
| storage carrier | `Claim`, `Conclusion` | needs one, names none |
| validation gate | the two `requires` clauses | no |
| query affordance | `subject_iri`, `refutes` | no |

Measured facts behind that table:

- **`reflection:canonical_proposition` has no domain**, deliberately. Its description records that
  the domain "had been DeclaredResource / ObservedResource / DerivedResource, which named three ways
  a resource came to exist and so constrained the warrant slot by a provenance fact." Warrant-bearing
  is therefore already a property, not a class.
- **Three classes require it independently** — `justification:Claim`, `enc:EncodedClaim`, and
  reflection's institution-emitted class — none subclassing another. `Claim` is a peer, not the
  container.
- **No code gates on `justification:Claim`.** Its only kernel-source occurrences are ESL-compile test
  fixtures and bootstrap seeding. `emit_from_trace` reads the target's `canonical_proposition`
  directly.
- **`observed(…)` never cites a `Claim`.** Every occurrence in the tree cites a SampleSet or an
  ingested file. The class's advertised ground-neutrality is unexercised; every instance is on the
  `declared` side.
- **`subject_iri` has no reader** despite its description claiming a first-class EigenQL index.
  **`refutes` has no reader and no writer** outside its own declaration and three docs about it.
- **A reader dispatches on CLASS to find the proposition.** `target_proposition_hash`
  (`kernel/src/layer/witness_admission.rs:471`) tests `is_a` for `justification:Conclusion` to decide
  whether to project `P` out of a judgement instead of reading a slot. A class test standing in for a
  property test is the conflation this ontology removed everywhere else, and it is the concrete form
  of "a consumer must know which container it holds". It is also why `emit_from_trace` and
  `emit_from_reasoning_sentence` are separate paths with nothing asserting their keys agree (G3 in
  `../notes/end-to-end-scenarios-and-integration-gaps.md`).
- **`Conclusion` requires a certificate**, which makes the paper's own *Verified* configuration —
  "the system holds `Judgement(L, t, P)` directly" — unrepresentable on the class that owns
  `justification:proof`.

## 2. The factoring

Warrant-bearing stays the property. Containers are ordinary resources: identity and provenance
attachment come from being a resource, not from joining a class. The kinds share only the property,
and each class requires what its own kind needs.

The *Verified* state needs no class of its own. The paper: "A manually authored claim accompanied by
a checked proof yields a *Verified* warrant alongside *Declared* provenance." The class comes from
the provenance shape; the proof judgement is a warrant fact available on any proposition-bearing
resource.

**`Conclusion` gets no proposition slot.** An earlier draft of this section gave it one so that every
container would bear `P` the same way, which would have meant authoring the value at 41 of the 43
`Conclusion` instances in the tree, each copied out of its own judgement. That is the wrong fix for
the defect. The defect is the **class test**, not the storage: a resource bears `P` explicitly in a
slot, or implicitly in a judgement it carries, and a reader should decide which by looking at the
properties present — never at `is_a`.

So one accessor, dispatching on property, in this order:

1. `justification:proposition` — the explicit slot, which a `Declaration` carries.
2. `grounds_judgement` — type is `Grounds(P)`; unwrap it.
3. `proof_judgement` — type is `P` already; no unwrapping.
4. the D39 §4.1 default `Asserts(iri)`.

Steps 2 and 3 are different unwrappings, which is why merging the two emitters into one lookup is
wrong and why they are separate arms rather than one. `emit_from_reasoning_sentence` becomes a caller
of this accessor rather than a sibling path with its own read, which is what G3 asks for. Zero
authored sites, no second copy of `P` to drift, no equality rule to police one, and the class test is
deleted rather than made redundant.

## 3. Before and after

### Containers

| before | after |
|---|---|
| `justification:Claim` — ground-neutral; requires `reflection:canonical_proposition` | split; the neutrality was never exercised |
| …used on the declared side (bridges, literature rules, methodology assertions) | `justification:Declaration` — requires `justification:proposition` **and** `prov:was_attributed_to` |
| …unused on the observed side | unchanged — no class; cite the measurement resource, its own proposition or the `core:Asserts` default |
| `justification:Conclusion` — requires `justification:judgement` | `justification:Conclusion` — requires `grounds_judgement`; bears `P` inside it, read by the §2 accessor |
| `enc:EncodedClaim` — independent; requires `canonical_proposition` + `enc:from_unit` | subclasses `Declaration`, adds `enc:from_unit` |

### Properties

| before | after |
|---|---|
| `reflection:canonical_proposition`, no domain | `justification:proposition`, no domain |
| `justification:judgement` — `holds(kernel, c, Certificate(P))` | `justification:grounds_judgement` — `holds(kernel, c, Grounds(P))` |
| `justification:proof` — domain `Conclusion`; no producer | `justification:proof_judgement` — on any proposition-bearing resource; `logic_kernel` = author-written term, `logic_lean4` = institution-emitted |
| `justification:subject_iri` — no reader | dropped |
| `justification:refutes` — no reader, no writer | dropped |

### The algebra

| before | after |
|---|---|
| `justification:Certificate : Prop -> Type 2` | `justification:Grounds : Prop -> Type 2` |
| `declared`, `observed`, `verified`, `app`, `sum_l`, `sum_r` | unchanged |
| `spec_poly` | `instantiate`, with `implicit(T, P)` |

Rationale for each rename:

- **`Certificate` → `Grounds`.** In the proof-theory literature a certificate is checkable evidence
  that *establishes* the claim. Here `Certificate(P)` is deliberately non-factive, and the
  description spends its opening undoing its own name. `Grounds(P)` reads as what the description
  already says: "The grounds for P, retained whole."
- **`judgement` → `grounds_judgement`, `proof` → `proof_judgement`.** Both slots hold
  `eigentt:Judgement` values, and the D87 hazard was that `holds(kernel, c, Grounds(P))` and
  `holds(L, t, P)` look alike. Giving the more dangerous one the generic name is what made the
  substitution invisible at call sites.
- **`canonical_proposition` → `justification:proposition`.** "Canonical" distinguishes it from no
  other form. The namespace move puts the warrant axis's input beside the classes that require it;
  its carriers are domain resources, not the system reflecting on itself.
- **`Conclusion` keeps its name.** It was chosen by elimination — the description says "Named
  Conclusion because Claim is taken by `enc:EncodedClaim`" — but it names the epistemic role, a
  proposition asserted on the strength of grounds. The freed name `Claim` stays unused; its
  genericness is what produced this note.
- **`spec_poly` → `instantiate`.** Both halves of the old name are wrong. `poly` claims polymorphism, but
  in nine of ten call sites `T` is `core:string` and `x` an IRI literal — instantiating a quantified
  premise over values; only `spec_poly_set_domain.esl` has a type domain, so the name was built on
  the exception. `spec` collides with the vocabulary §4 explains the constructor in: this file uses
  "constant specification" twice, for the postulation of `Declared` and `Observed`, and
  `statistics.esl` uses "analysis spec". `instantiate` is the standard verb for eliminating a
  universal at a term, covers a value or a type domain alike, and makes §4 an identity — the
  constructor is the fused application against the *instantiation axiom*. Written out rather than
  abbreviated to `inst`, which collides with `institution` — an ESL-visible concept appearing in
  this file, in `prov.esl`, and in the paragraph above, and the thing that produces judgements in
  this same area. Length is unremarkable beside `declared` / `observed` / `verified`. With
  `implicit(T, P)`, `spec_poly(core:string, fun x => …, EIG, declared(RULE, RULE_P))` becomes
  `instantiate(EIG, declared(RULE, RULE_P))`.
- **`Declaration`** aligns the resource with its ground (`Declared`), its witness
  (`witness:IsDeclaredAs`) and its trace (`prov:DeclarationTrace`). The collision to accept is
  type-theoretic "declaration", e.g. `prov:checked_declaration`, which namespace qualification
  handles.

### Stale names in code

`emit_from_reasoning_sentence` and 17 occurrences of `sentence` in
`kernel/src/layer/witness_admission.rs` are named for `justification:Sentence` and the `reasoning:`
namespace, neither of which exists. P1.3 renamed `reasoning:JustifiedBy` → `justification:Certificate`
and `Sentence` → `Conclusion`; the code kept the older names.

## 4. `spec_poly` stays, and the paper gains a section

The paper's appendix targets six justification-term constructors — grounding constructors four to
three, so seven to six. The file declares seven. `spec_poly` is in neither count and predates the
paper (`7bb19f0`, before D81 landed it at `4ad5620`).

It is not an unreconciled leftover. **The paper's algebra is propositional** — its own table places
the design at "J: application, sum; no `!`, no factivity" — and LP has no quantifiers, so
specialisation cannot arise there. The kernel's propositions are dependently typed and quantified, so
the algebra needs an elimination the propositional presentation never needs.

In FOLP (Artemov & Fitting 2020, ch. 10, `references/publications/`) the proof-term operations are
`+`, `·`, `!` and `gen_x` (Def. 10.1–10.2). `gen_x` is *introduction* — (B5),
`t:X A → gen_x(t):X ∀xA`. **There is no elimination operator.** Instantiation goes through
application against a justified logical axiom, worked out at Example 10.7:

```
1. ∀xA → A                                    logical axiom (A1)
2. c:(∀xA → A)                                axiom necessitation (R3)
4. c:{x}(∀xA → A) → (u:{x}∀xA → (c·u):{x}A)   application (B2)
5. u:{x}∀xA → (c·u):{x}A
```

So `spec_poly(T, P, x, j)` is Artemov's `c·j`. Three facts decide whether to reify `c`:

1. **ESL's `axiom` is the wrong device.** `eigentt:Axiom` is "a closed term whose type the kernel
   admits without checking the term itself" — a postulated *proof*, the factive stratum. FOLP's
   `c:A` is a justification constant, the non-factive stratum. No rule connects them, by design.
2. **The instantiation lemma is not a postulate.** `(∀y:T. P y) → P x` is inhabited by
   `fun h => h x`. The honest reification is a committed resource carrying a real proof judgement,
   cited as a `verified` leaf — no fourth leaf kind is needed.
3. **But it regresses.** Using one *polymorphic* instantiation lemma requires specialising it first,
   which is the operation being defined. So each specialisation needs its own monomorphic resource:
   today's 10 call sites become ~20 committed resources, growing with campaign size — the
   per-instance cost the polymorphic bridge exists to avoid. FOLP escapes this by taking the constant
   specification to be **total** ("contains `c:A` for all `c` and `A`"), which a chain cannot have:
   every constant here is a committed resource with an IRI.

**`spec_poly` is the chain-resident stand-in for FOLP's total constant specification over the
instantiation schema.** Keep the constructor, rename it `instantiate`, and add a paper section deriving it
as the fused `c·` form. The paper's count then reads seven with a reason rather than six with an
exception.

An adjacent hole, unverified: `build_axiom_env` admits any `eigentt:Axiom` whose statement
type-checks as a well-formed type, with no restriction to `Prop`. Whether
`axiom c : justification:Certificate(P)` is declarable — a postulated certificate entering the
justification stratum through the proof-stratum door — and what `support` / `leaves_of` do with an
`EigonAxiom` node where a constructor is expected, is untraced. Same family as the
`holds(logic_kernel, some_axiom, P)` route to `Verified`, and both close with the same fix.

## 5. Open

- **The logic/term pairing.** Whether the check-mode rule enforces that `logic_lean4` admits only a
  `Checked` term and `logic_kernel` only a non-`Checked` one. Today the refusal keys on the term
  form alone, so `holds(logic_lean4, <ordinary term>, P)` is accepted while
  `witness_admission.rs:663` and the `witness:IsVerifiedAs` description both state the enforcement
  more broadly than it runs. Once `logic` discriminates the two producers of `proof_judgement`, the
  gap becomes load-bearing.
- **Axiom-headed proof terms.** `Exp::EigonAxiom` resolves against the chain axiom env, so
  `holds(logic_kernel, my:axiom, P)` type-checks and mints `Verified` for a postulate.
- **`Grounds` as a name** is a recommendation, not a settled decision.

## 5a. Can `spec_poly`'s `T` and `P` be implicit? Measured `2026-09-08`

C1 in `../notes/next-steps-after-d88.md` files this as "widen the unification fragment past
first-order patterns", priced at 1,943 characters across 10 call sites and marked *not needed*. A
prototype says the fragment is not the obstacle. Five things are, and four are deferred code paths.

Method: `spec_decl` in `kernel/src/nbe/check/inductive.rs`'s test module — `S : Set -> Set` with
`spec : forall (T : Set, P : T -> Set, x : T) => S(forall (y : T) => P(y)) -> S(P(x))`, the same
shape as `justification:Certificate.spec_poly`, checked with `T` and `P` declared implicit.

1. **The expected-type equation is genuinely unsolvable, and stays so.** `?P x` with `x` a concrete
   term is not a pattern: from `HasLowIC50("EIG_0291")` at `x = "EIG_0291"`, `?P` could abstract the
   occurrence or not. `x` therefore stays explicit — which is right, since `x` is what the author
   means to say. Observed as `non-pattern spine (spine entry 0 is not a bound variable:
   LitString(...))`.
2. **`x`'s own type is the unsolved `?T`, so the argument must be INFERRED, not checked.** The first
   run used `Exp::Unit` and failed with `CannotInfer`. Every real call site passes an IRI, and a
   literal infers, so this is satisfied in practice — but it is why `T` is reachable at all.
3. **Evaluation never populates `Neut::Meta`'s spine.** `?P y` evaluates to
   `Neut::App(Meta(id, []), y)`; `Neut::Meta` is constructed with an empty spine at exactly three
   sites, all of them binder setup. So `solve_meta`'s non-empty-spine branch — deferred "until a
   real consumer with non-empty spines arrives" — was unreachable by construction, and the consumer
   could not have arrived.
4. **`unify`'s Pi arm is restricted to anonymous binders**; `forall (y : T) => P(y)` falls through to
   `eq_nf`, which solves nothing.
5. **`zonk_val` has no `Val::Pi` arm and no arm for an applied meta**, so a solved `?P` is never
   substituted and the final index check re-derives `?P x` from an `arg_env` captured before the
   premise solved it.

With spine recognition, λ-abstraction in `solve_meta`, and `zonk` resolving applied metas, `T` and
`P` are both inferred and only `x` and the premise are written. Kernel suite 1843 passing, every
integration suite, `eigenius-statistics` and `eigenius-lean` green, fmt and clippy clean.

Both the constant and the **dependent** case pass. `P := fun y => Id(String, y, y)` makes the
equation `?P G#0 ≟ Id(String, G#0, G#0)`, where the bound variable must be placed twice in the
solution — the shape every real bridge takes, and the one a constant `P` does not exercise.

**The capture question, answered.** Extending the Pi arm does not reopen capture. `solve_meta`'s
comment refuses a scope check because "a `Val` hides variables inside closure environments, so 'does
this mention a variable above level N' is not decidable by a structural walk" — true of a walk over a
`Val`, not of one over a **readback**, which forces every closure and makes the free variables
syntactic. A pattern spine names exactly what the solution may keep, and anything else is refused;
`a_pattern_spine_still_refuses_a_solution_mentioning_a_variable_it_does_not_name` pins it.

**What it does collide with is D49 witness-key byte stability.** An unguarded named-binder arm also
matches an anonymous arrow against a named-but-unused binder, which readback deliberately
distinguishes and witness keys hash, so it would identify two propositions with different keys.
`meta_free_function_types_are_still_compared_by_readback` caught it on the first full run. Gating the
arm to named-on-both-sides preserves the distinction.

**One fragility blocks landing.** The scope check must recognise a generated variable after readback,
and `Neut::Gen(j, name)` reads back as `Var("{name}{j}")` keeping the producer's tag — the tree uses
at least `G#` (readback), `TC#` (`env::gen_val`) and ad-hoc tags in tests. A check keyed on one prefix
silently passes exactly the cases it exists to refuse: with the check keyed on `G#`, the escape test
*succeeded*, solving a meta to a variable out of scope. The prototype over-approximates by parsing
trailing digits off any name, which fails closed but reads a user variable named `foo12` as
generated. A landed version needs canonical naming at readback, or a readback that carries levels
rather than reconstructing them from names.

Prototype lives in the working tree, uncommitted, marked `PROTOTYPE (D89 experiment)`.

## 6. Cost and sequencing

The renaming and the refactoring are one edit: both rewrite `is_a` values and property keys across
the WRN chain, the statistics fixtures and the encoding pipeline, both move the bootstrap manifest,
and both ride one reseed. Doing them separately pays the reseed twice and leaves an intermediate
state whose names describe the old factoring.

The WRN example and the parser encoding pipeline are reshaped to this design, not the reverse.

## 7. Note on the paper's two copies

`docs/design/judgements-and-warrants.tex` (448 lines) and
`../publications/papers/judgements-and-warrants/judgements-and-warrants.tex` (444 lines, `2026-08-28`)
have diverged, with the in-repo copy ahead — the ground-set column in the Computed/Sampled table, the
`sec:inhabitation` cross-reference, and the reworded *Verified*-ceiling paragraphs are in-repo only.
The constructor-count sentence §4 rests on is identical in both.
