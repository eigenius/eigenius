# D87 — The verification judgement

*Status: **proposed** `2026-09-04` · design note. §4 rewritten the same day; the first draft's
proof-as-axiom proposal is withdrawn and the reason is recorded in §4.1. §9's three open questions
are all closed — 1 in §6, 2 and 3 in §9 itself.*

*Replaces the artifact half of eigenius#160. Companions: the paper
[Judgements, Warrants, and Logics](judgements-and-warrants.tex),
[D74](d74-eigentt-to-lean-externalization.md) (what externalization can and cannot express),
[D28](d28-lean-4-as-institution.md) §2.3 (nanoda runs in process), and
`docs/notes/judgements-warrants-build-plan.md` §"Open after P7".*

---

## 1. The defect

eigenius#160 made a checked Lean proof reach the *verified* grade by emitting a `prov:VerificationTrace`.
`witness_index::emit_from_trace` follows the trace's `prov:resource` to the claim, hashes the
claim's `reflection:canonical_proposition`, and admits `IsVerifiedAs`. Nothing re-checks anything;
the kernel takes the trace's word that nanoda ran.

`eigentt:Judgement`'s own description names the failure mode:

> the reification of justification logic's proof-checker operator — LP's positive-introspection
> axiom `t:F -> !t:(t:F)` names the evidence `!t` that `t` proves `F`, which is what a checker
> returns and what this constructor persists. **An algebra carrying application and sum but no `!`
> runs the checker at commit time and discards the result; a judgement keeps it.**

`witness:IsVerifiedAs` has **zero constructors**, so no term inhabits it and `layer_admits_witness`
is the only way one comes into existence. That puts the function in the TCB, and a wrong admission
cannot be caught downstream because an axiom has no proof to re-check. The witness is postulated on
the strength of a note.

## 2. The shape that keeps the result

`eigentt:Judgement` has one constructor:

```text
holds(logic : eigentt:Logic, term : eigentt:Term, type : eigentt:Term)
```

*"A CHECKED triple: a checker for `logic` verified `term` against `type`."* Two `eigentt:Logic`
values are declared, and the second is named for this: `logic_kernel`, and `logic_lean4` — *"Lean 4,
re-checked in process by the `nanoda_lib` kernel reimplementation."*

The route is already implemented for one resource class. `emit_from_reasoning_sentence`
(`witness_index.rs:291`) reads `justification:proof` off a `justification:Conclusion`, decodes the
judgement, **refuses it when the type is a `Certificate`** — a certificate judgement establishes
nothing about the proposition — and keys `Verified` off the proof's own type.

Nothing populates `justification:proof`. Both `Verified` routes exist; the trusted one is populated
and the checked one is empty.

## 3. The constraint on `t`

`holds`'s `term` argument is typed `eigentt:Term` — structural, not a reference. Two consequences:

1. **A Lean proof term is not representable.** D74 externalizes propositions in one direction,
   EigenTT → Lean. There is no inverse, deliberately (#159), and `Lam` is refused outright
   (D74 §4.4: Mini-TT lambdas carry no domain). `fun _ h => h` has no EigenTT form.
2. **A `ConstRef` to the payload resource does not resolve.** `resolve_const_ref`
   (`eigentt_type_mirror.rs:1201`) dispatches on the target's class — `core:Class` → `EigonClass`,
   `eigentt:Axiom` → `EigonAxiom`, `core:InductiveType` → `Const`, five primitives short-circuit —
   and an unresolved `ConstRef` is a `TermMalformed` rejection (`eigentt_value.rs:619`). A
   `lean:LeanProofPayload` **instance** is none of those.

## 4. What `t` is

### 4.1 Not an axiom — withdrawn

The first draft proposed declaring the checked proof as an `eigentt:Axiom` whose `axiom_statement`
is the claim's proposition, with `t = ConstRef(that axiom)`. It resolves, and it needs no change to
the eigentt fragment. Both true, and both beside the point. `eigentt:Axiom` is:

> A named axiom: a closed term whose type the kernel admits **without checking the term itself**.

That is the opposite of what is being recorded. Using it would put "asserted without proof" and
"checked by nanoda" in one class, with four consequences:

| | |
|---|---|
| a reader holding only the term | learns nothing about checked-ness; must find the judgement and trust it |
| an author | can declare an axiom and write a judgement citing it; nothing refuses either |
| `Declared(a)` and `Verified(a)` | can name the **same** resource, leaving the authored justification term as the only discriminator |
| the chain | has no way to tell the two apart |

**The codebase has diagnosed this exact shape twice.** #205: a hand-authored `ProgramTrace` and a
kernel-minted one are the same class, so *"the kernel cannot tell them apart… The distinction has to
be carried by the class"* — resolved by adding `reflection:ExternalExecutionTrace`. #23: deleted
`epistemic_status`, because a resource must not declare its own grade. The withdrawn proposal walked
back into what both eliminated, and its justification — *"needs no change to the eigentt fragment"* —
is CLAUDE.md's named signal for wedging.

### 4.2 The distinction is carried by the term's own form

`t` names something an institution checked, and the class says so. Three forms, in descending
strength:

| form | discriminates? | cost |
|---|---|---|
| a distinct `eigentt:Term` former (`Checked(iri)` beside `EigonAxiom`) | yes, structurally | a fragment change: D74's exhaustive 43-variant match, the D47 codec, conversion |
| a subclass, `eigentt:CheckedProof : eigentt:Axiom` | by `is_a`, but it still *is* an axiom under subsumption, so anything reading the parent re-conflates | resolves through the existing `ConstRef` path |
| same class + a kernel-only property | **no** — #205 records that no *"kernel-only, refused from input"* mechanism exists anywhere in the validator | unenforceable today |

**Recommended: the distinct former.** D74 §5 is explicit that this is where a wrong call proves the
wrong theorem soundly. A conflation carried forever is worse than one former added to a small
fragment, and the fragment is small precisely so that additions are deliberate.

The class settles **who may assert**. It does not make the assertion checkable — that is §5.

### 4.3 What `Checked(a)` does at the kernel's own check, and why input cannot forge one

`eigentt:Judgement`'s contract is that *"a slot ranging over this type is checked in CHECK mode —
decode both fields, check `type` is a type, check `term` against it"*, and
`validation/rules/eigentt_value.rs` does exactly that for **every** judgement, whatever its `logic`.
So `holds(logic_lean4, Checked(a), P)` raises a question §4.2 leaves implicit: does the kernel check
`Checked(a)` against `P` in its own type theory?

**It must not, and it must not succeed vacuously either.** Checking it would mean re-proving `P`
without the export — impossible. Admitting it for any `P` would make `Verified` assertable by
anybody who writes the judgement, which is the laundering the two-layer separation exists to
forbid.

**`Checked` therefore fails the kernel's check, deliberately, and that is the whole enforcement.**
The kernel has no proof of `P` and will not manufacture one, so a *hand-authored* lean4 judgement is
refused at commit.

**An institution-emitted one is never asked.** `structural_validate` runs **before**
`autoonload_dispatch` in `commit::pipeline`, and the followup slice is `[build, persist]` with no
validation at all — *"kernel-emitted content … re-validation is redundant and forces the ontology to
be permissive enough for every shape the kernel emits."* So the judgement the institution mints
after `check_proof` returned `Holds` is not re-checked in a theory that could not check it, while
the one an author writes is.

**This is the mechanism #205 said did not exist.** §4.2's third row records that no *"kernel-only,
refused from input"* mechanism exists anywhere in the validator, and takes that as the reason to
reject the same-class-plus-a-property option. It exists here without being built: the emission path
and the input path already differ in whether they validate, so the property falls out of the
pipeline's shape rather than from a guard added to it. What `Checked` contributes on top is what
§4.2 argued for — the distinction is structural, so `Declared(a)` and `Verified(a)` cannot name the
same thing and a reader holding only the term knows what it is.

**It does not make the judgement true, and §5 is still what does.** A judgement the kernel minted is
a record that nanoda accepted the export. Anyone can re-run that check; §5 is what pins the two
inputs that make the re-run reach the same verdict.

## 5. Re-decidability, not attestation

**nanoda_lib emits no cryptographic receipt, and should not be made to.** Verified `2026-09-04`: no
`sha2`, `blake`, `ed25519` or `hmac` in its `Cargo.toml`; `unique_hasher.rs` is a 64-bit interning
hasher for expression dedup inside the checker (`digest: u64`, `set_digest` asserted one-time), not
a digest. `check_proof` returns `Verdict::Holds | Fails { diagnostic }` — a boolean and a string.

**A self-signed receipt would be circular.** D28 §2.3 makes nanoda an in-process function call, no
IPC. A signature produced by a key the same process holds is not evidence *to* that process: either
the process is trusted or it is not. Such a receipt attests exactly what the `VerificationTrace`
already attests, and adds key management. Receipts earn their keep when the checker ran somewhere
outside the trust boundary and its result is to be admitted **without re-running** — which needs a
key bound externally (TEE attestation, reproducible-build identity, a notary), a different
architecture from this one.

**The property to hold instead is that the verdict is recomputable.** It is a deterministic function
of five inputs:

| input | on the chain? |
|---|---|
| export bytes | **yes** — `LeanProofPayload.payload_bytes` |
| target declaration name | **yes** — `lean:target_name` |
| the claim's proposition | **yes** — `reflection:canonical_proposition` |
| permitted axiom set | **no** — hardcoded `DEFAULT_LEAN_AXIOMS` in `institution.rs` |
| checker identity (the `nanoda_lib` rev) | **no** — pinned in `Cargo.toml` only |

Pin the last two and any party can re-run `check_proof` and obtain the same verdict. That is
stronger than a receipt: a receipt says *"I checked"*; this says *"check it yourself."* It is also
idiomatic — layers are content-addressed and `runtime:library_content_hash` already serves this role
for mirrors.

The two missing rows are exactly what a receipt would have bound. The axiom set is queued as a
`prov:VerificationTrace` slot in [D86](d86-the-numeric-primitive-core.md) §5.1 / #235; the checker
identity is not recorded anywhere and joins it.

**This is the standard posture for proof-carrying data**, and it is what the Harvard collaboration
wants: proofs arriving from outside are re-checked rather than trusted on a receipt, because
checking is cheap relative to proving. Re-running nanoda over a small export is milliseconds.

**It also relieves §4.** With the re-check defined and cheap, the term does not have to
self-certify. The class settles who may assert; the pinned inputs settle whether anyone can verify
the assertion. Both are needed, and neither substitutes for the other.

## 6. The demo's claim is not a claim

The fixture the whole Lean path is demonstrated on is mis-modelled, and it matters here because
§9's first open question was framed around accommodating it.

`urn:eigenius:demo:lean:patient_1` carries `is_a: [demo:lean:Patient]` and a
`reflection:canonical_proposition`, and nothing else. Decoded, that proposition is:

```text
Π (p : Patient). Π (_ : Healthy(p)). Healthy(p)
```

— closed, universally quantified over *all* Patients, and it never mentions `patient_1`. So the
witness the chain admits is

```text
IsVerifiedAs("urn:eigenius:demo:lean:patient_1", ∀p:Patient. Healthy(p) → Healthy(p))
```

in which the IRI contributes nothing to the pairing: any resource IRI would serve equally.
`witness/mod.rs` states the intent — the `(category, iri)` pair *"determines exactly one canonical
proposition per resource"* — and here the resource is a hook the proposition hangs from.

**Two counts, and the second is the substantive one.**

1. A `Patient` instance is an entity, not an assertion. Patients do not carry propositions.
2. The proposition is not about it. This is *not* a validation violation —
   `reflection:canonical_proposition` declares `class_types eigentt:Term` and no domain, so it is
   permitted anywhere, and a claim legitimately may carry a universally quantified proposition. The
   defect is that `patient_1` is not a claim-bearing resource in the first place, and D39 §4.1's
   default (`Asserts(iri)`, which does mention its subject) shows what the pairing is meant to be.

**The shape already exists.** `justification:Conclusion` `requires justification:judgement`,
`recommends justification:proof` — the `holds(logic, t, P)` slot §2 wants — and carries
`justification:subject_iri`, described as *"The principal Resource this conclusion is about.
**Aboutness, not logic**: no judgement carries it."* That is exactly the distinction the fixture
collapses.

**Built `2026-09-05`, and it came out as `justification:Claim` rather than `Conclusion`.** This
section reached for `Conclusion` because `subject_iri` was the only way to say what a
∀-quantified proposition was about. Make the proposition name its subject instead and that need
disappears, while `Conclusion`'s *required* `justification:judgement` — the kernel's own
`holds(kernel, c, Certificate(j, P))` — has nothing honest to hold for a claim whose warrant is a
Lean proof rather than a certificate over chain grounds. `justification:Claim` is the class the
ontology already describes for this: *"a chain-resident resource carrying a proposition … cited as
a ground"*, requiring exactly `reflection:canonical_proposition`. It also moves the eigenius#159
refusal one step earlier — a claim with no proposition now fails VALIDATION, for every claim on
every chain, rather than being refused by an institution when a Lean proof happens to name it.

Three resources, three jobs:

| resource | class | what it is |
|---|---|---|
| `demo:lean:Patient` | `core:Class` | the type |
| `demo:lean:patient_1` | `eigentt:Axiom` | a named individual — an entity, carrying no proposition |
| `demo:lean:claim_patient_1_healthy` | `justification:Claim` | the assertion, carrying `Healthy(patient_1)` |

The individual is an `eigentt:Axiom` and not an instance of `Patient` because only an axiom-shaped
resource can appear as an argument in a term: `resolve_const_ref` yields an applicable head for
that class and not for a bare instance. A resource that is merely `is_a: [Patient]` is something
the term language **cannot mention** — which is why the old fixture had to quantify over all
Patients instead of naming one. That is the mechanical reason behind this section's two counts.

### What building it found

Three things, none of them visible from reading:

1. **The predicate was constant, and that is worse than the quantification.** `Healthy` was
   `def Healthy (_p) : Prop := True`. With it, `Healthy patient_1` and `Healthy patient_2` are
   both definitionally `True`, so `def_eq` accepts either claim against either proof and **no
   arrangement of subjects can make the demo discriminate**. Measured: the first near-miss came
   back `Holds`. `Healthy p` is now `50 ≤ p.restingHr ∧ p.restingHr ≤ 100`, over a `Patient` with
   `Nat` fields — `Nat` because `Float` operations are `@[extern]` and reduce to nothing in the
   kernel, so no proposition about a `Float` field is provable by `decide`.

   The mirror's own comment had asserted the opposite — *"the body is irrelevant to the statement
   being checked"* — which was true of `healthy_refl`, an implication compared syntactically, and
   false the moment the claim became an application.

2. **`check_statement` resolved names against declarations `def_eq` could not reach.** It runs
   under `EnvLimit::ByName(target)`, which cuts the environment off at the target's index, while
   the `NameTable` was built from *every* declaration in the export. So externalization resolved a
   constant `def_eq` then failed to find, and nanoda answers that with a **panic** — caught
   upstream and reported as "the statement check panicked", which says nothing a reader can act
   on. Resolving against the same set turns it into `UnknownConstant`, naming both the chain IRI
   and the Lean name. That is the module's own discipline (*"a constant the export does not
   declare cannot be `def_eq` to anything in it"*, resolved up front for exactly this reason); it
   had not been applied to the environment limit.

3. **A `Fails` verdict refuses the whole commit**, so the near-miss cannot ship inside the demo's
   document — it would take the demo down with it. It is a second one-resource file, and the
   refusal is the demonstration: load the claim that verifies, then load the one that does not.

**This resolves §9's open question 1.** It was framed as *"where does the judgement live when the
claim is not a `justification:Conclusion`"* — presupposing a `Patient` instance is a legitimate
claim-bearer. It is not, so the machinery does not generalise off `Conclusion` to accommodate a
mis-modelled fixture; the fixture emits a `Conclusion`. `emit_from_reasoning_sentence` already reads
`justification:proof` off one, so the checked route works today for a correctly-shaped claim.

**What it qualifies about eigenius#159.** That issue was *"nothing binds a Lean proof to the claim it
is supposed to prove."* The binding is now checked — `def_eq` against the claim's own proposition,
refused when absent. But in the demo what is bound is a resource IRI to a proposition that says
nothing about that resource, and the proof is a tautology (`fun _ h => h`). The demo therefore
verifies that the plumbing runs, not that it discriminates. A fixture whose proposition is *about*
its subject, and a near-miss variant that must fail, are what would show the mechanism working.

**Cost is small.** `cargo run -p eigenius-lean --example gen_verification_demo` generates
`lean-verification-demo.eigon.json`, so the change is in that generator plus the notebook prose in
`notebooks/examples/lean-verification.json`, which describes `patient_1` as "the Eigon claim".

## 7. What changes

| | from | to |
|---|---|---|
| what the institution emits on `Holds` | `prov:VerificationTrace` | the trace **plus** `holds(logic_lean4, Checked(a), P)` |
| the trace's role | the thing `Verified` is read from | provenance: when the check ran, against which payload, under which axiom set, by which checker build |
| how `Verified` is admitted | `emit_from_trace` hashes the claim's proposition | the judgement's own `type` is the proposition — the `emit_from_reasoning_sentence` shape, on the trace |
| `Certificate.verified` | consumes `witness:IsVerifiedAs(iri, P)` | consumes the judgement |
| `witness:IsVerifiedAs` | postulated, zero constructors, in the TCB | still declared, no longer postulated — see below |

The trace does not go away, and the paper's split is what keeps it: the trace is provenance, the
judgement is warrant.

**The judgement rides on the trace**, in a `prov:VerificationTrace` slot of its own, and the three
alternatives are worse. Writing it onto the author's claim as `justification:proof` means an
institution redefining a user's resource. Emitting a fresh `justification:Conclusion` at
`{claim_iri}:verified` means `Verified(iri)` names that emission rather than the claim, against §9.2,
and `finalize_emitted_resource` would stamp it `reflection:InstitutionEmittedDerivation` — *"grounds
nothing"* on the one resource whose purpose is to be a ground, which is the conflict the trace
exemption already had to resolve once. Leaving it off the chain is what §1 is about. On the trace it
needs no new resource, no exemption and no redefinition: `trace_category` already reads
`VerificationTrace` as `Verified`, and `emit_from_trace` reads the judgement's `type` in place of
the target's stored `canonical_proposition`.

**This is what closes `judgements-warrants-build-plan.md` §"Open after P7", and the answer is about
the INDEX, not the types.** That section asks whether the three surviving witness families are
decision procedures over relations the kernel can read at any time; if so, *"the index is a cache
over relations — rebuildable, droppable, and not a soundness boundary."*

`Declared` and `Observed` were already that, verified `2026-09-05`: `layer_admits_witness` consults
no committed witness, only the layer's Trace resources and the propositions they point at, and its
two caches (`has_witness_candidates`, the in-flight fallback) both fail conservatively — a wrong
guess refuses a certificate, never admits one. `Verified` was the exception, because the trace it
read was a note that a check had run. With the judgement on the trace and its inputs pinned, that
route reads a recorded result whose verdict anyone can recompute. All three lookups are decision
procedures; **the index is a cache.**

**The constant specification is a different thing, and it stays in the TCB.** An earlier draft of
this section said the TCB framing "stops being true" without qualifying which part. That
over-generalised, and `judgements-and-warrants.tex` §"Witnesses and the Trusted Computing Base" is
explicit against it: *"The Verified state is provable, whereas Declared and Observed states are
postulated … Postulation is the correct semantic operation for attributions: verification is
impossible because an attribution merely asserts that an agent made a claim or that a physical
recording occurred."* The paper names the TCB as the kernel's checker, each hosted external checker,
each comorphism, **and the constant specification governing attributions**.

Recomputing a *lookup* does not touch that: it recomputes the same trusted assertion. The chain says
an agent declared `P`, and nothing can check whether they did — which is why postulation is correct
rather than a gap. So what this batch moved is exactly one family: `Verified`, which the paper
already classified as *provable*, and which D87 §5 makes re-runnable in practice by pinning the
inputs. `Declared` and `Observed` are in the TCB permanently, by design.

**An earlier draft of this table said `witness:IsVerifiedAs` was *removable*, and that does not
follow.** `Certificate.verified`'s premise is what makes `Certificate(Verified(iri), P)`
inhabitable only where the chain verified `P` about `iri`. Delete the premise and the constructor
is unconditional — `verified(iri, P)` for any `P`, so `Verified` becomes assertable by anyone who
writes a certificate, which is the laundering this whole design exists to forbid. Nothing else can
occupy that position: a premise ranging over `eigentt:Judgement` would be a *data* type, inhabited
by any well-formed value, and the CHECK-mode rule that would catch a bad one runs at validation
rather than inside the kernel's own conversion.

So the three predicates stay declared and the kernel still synthesises their inhabitants. What
changes is what that synthesis IS: a decision procedure over chain relations rather than an
admission decision, so a wrong answer is catchable by recomputation instead of being an axiom with
no proof to re-check. `witness_index.rs`'s header claim — *"this module is inside the TCB … the
witness itself is postulated, and a wrong admission cannot be caught downstream"* — is what stops
being true, for the reason P7 predicted.

**Answered in [D88](d88-four-questions-the-justification-layer-leaves-open.md) §2: they do.** The
question and the evidence are below; D88 derives the answer from how drift fails — with the type,
the constructor's premise names something that must resolve, so a kernel/ontology divergence breaks
the build; keyed on the constructor instead, the kernel silently stops matching and the side
condition never fires.

**The question as it stood.** The argument above rules out
deleting `Certificate.verified`'s premise and leaving the constructor unconditional. It does not
rule out a third option, which was not evaluated: **keep the condition, drop the type** — check
`verified(iri, P)` against the chain by a rule keyed on the *constructor* rather than by filling an
argument. The witness is already a check-time side condition in all but name: it is elided in the
surface (`declared(RULE, RULE_P)`, two arguments for a three-argument constructor), synthesised by
`CheckHooks::synthesize_chain_witness`, never persisted, and carries no information the trace does
not.

The trade-off is where the special case lives. Today it keys on the *type* — the hook fires when
the expected type is a witness-category inductive — so any constructor in any layer can demand a
witness by naming the type, and the kernel needs no knowledge of `justification:Certificate`.
Without the type it would key on the constructor, pulling the reasoning vocabulary into the
checker, which is the direction the layer-ordering argument in those declarations' own descriptions
warns against. That argument is already partly compromised: `witness_index.rs` hard-codes
`justification:Conclusion` as `REASONING_SENTENCE`.

## 8. Cost

- **Ontology**: `prov:VerificationTrace` slots for the permitted axiom set and for the checker
  identity as a *kind + value* pair (§9.3); an anchor from the checked term to its payload.
  Bootstrap-resident, so it rides #235's reseed.
- **Deploy**: pin `deploy/bicep/modules/kernel.bicep` and `docker-compose.yml` by digest rather
  than tag, and pass `EIGENIUS_IMAGE_DIGEST`. Independently worth doing — a tag is mutable, so
  neither deployment is reproducible today.
- **eigentt fragment**: one term former (§4.2), landing in the D47 codec, conversion, and D74's
  exhaustive match — which will refuse it for externalization, since a checked-proof reference has
  no Lean counterpart to translate to.
- **Institution**: `do_proof_check` already holds all three `holds` arguments at the moment it
  discards them.
- **Kernel**: reuse `emit_from_reasoning_sentence`'s decode-and-refuse-a-certificate logic.
- **Fixture**: regenerate the demo through `gen_verification_demo` so the claim is a `Conclusion`
  with `subject_iri` (§6), and add a proposition that is *about* its subject plus a near-miss that
  must fail — without those the demo cannot show the check discriminating.

## 9. Open

1. ~~**Where the judgement lives** when the claim is not a `justification:Conclusion`.~~
   **Resolved in §6**: the claim is a `Conclusion`, and the demo fixture is what is wrong.
2. ~~**Whether `Verified(iri)` names the claim or the checked term.**~~ **Resolved — it names the
   claim**, by §6's own lever. `witness:IsVerifiedAs`'s signature is `core:string -> Prop -> Prop`:
   *"the IRI of the underlying resource and **the proposition it carries**."* A proof term does not
   *carry* a proposition on the chain — it *proves* one; the `Conclusion` carries it. And the
   parallel holds across the algebra: `Declared(plan)` names the plan, which carries the assertion
   that it denotes `f : I -> O`; `Observed(inputs)` names the dataset, which carries the assertion
   that it was recorded. All three leaves name proposition-carrying chain resources, so `Verified`
   does too.

   The checked term is named in a different slot — `holds(logic_lean4, Checked(t), P)`'s second
   argument. That yields three slots for three distinct things, which is the same separation §6
   restored between logic and aboutness:

   | slot | names |
   |---|---|
   | `holds(_, t, _)` | the evidence — what the checker examined |
   | `Verified(iri)` | the claim whose standing that judgement establishes |
   | `justification:subject_iri` | what the claim is *about* |

   **No circularity.** `Certificate(Verified(c), P)` is what a *downstream* claim cites, with `c` as
   its evidence. `c`'s own standing comes from its `justification:proof` judgement, not from a
   certificate over itself. The self-attestation D81 criticised — *"the reasoning institution
   vouching for its own output"* — is a separate defect about which route populates the witness, not
   about what the leaf names.
3. ~~**What "checker identity" is.**~~ **Decided.** The slot records the strongest available
   binding **and says which kind it is**, so a reader can tell an identity that binds the running
   binary from one that only names the source.

   | kind | value | binds |
   |---|---|---|
   | `image_digest` | the registry digest of the kernel image, injected at deploy time | the running binary |
   | `source_pin` | `nanoda_lib` rev + Lean toolchain | the checker's source and the export format |

   Both are compile-time or deploy-time constants. The `source_pin` values exist today: the rev is
   pinned in `crates/eigenius-lean/Cargo.toml` (`1e44c496…`), and `leanprover/lean4:v4.29.1` is
   already baked into a const by `eigenius-lean-runtime/build.rs` so the Dockerfile composer and
   every Rust caller read one version.

   **Start with `source_pin`; `image_digest` is the upgrade and it is small.** It covers what a
   verdict actually varies along today — which checker source was compiled, which Lean produced the
   export format it parses. What it does not cover is that the running binary was built from that
   source: a different compiler version, different feature flags, or a tampered build yields a
   different binary from the same rev.

   ### The digest comes from the deployment, not from the container

   The deployer already knows which digest it deployed. That is where the fact is authoritative, it
   needs no privilege, and it works on every runtime:

   ```bicep
   image: '${acrLoginServer}/eigenius-kernel@${imageDigest}'
   env: [
     { name: 'EIGENIUS_IMAGE_DIGEST', value: imageDigest }
   ]
   ```

   Same shape in `docker-compose.yml`. **This also fixes something independently broken**: both
   deployments pin by *tag* today — `deploy/bicep/modules/kernel.bicep:31` is
   `eigenius-kernel:${imageTag}`, `docker-compose.yml:28` is `eigenius-kernel:local` — and a tag is
   mutable, so neither deployment is reproducible even in principle. Deploying by digest is the
   prerequisite for the identity meaning anything, and is worth doing on its own.

   Absence of the variable is itself informative — "not deployed by digest" — so no fallback logic
   is needed beyond emitting `source_pin`.

   **Asking the container runtime was considered and rejected.** Reading `/etc/hostname` and
   inspecting `/containers/{id}/json` over `/var/run/docker.sock` (directly, or via `bollard`, which
   the workspace already carries behind `runtime-substrate`'s optional `docker-spawner` feature)
   fails on four counts:

   - **It does not work in production.** `deploy/bicep/modules/kernel.bicep:9` is
     `Microsoft.App/containerApps@2024-03-01` — a managed platform with no Docker socket.
   - **It requires mounting the socket**, which is root-equivalent on the host. The substrate holds
     that capability because its job is spawning containers; extending it to the checker for a read
     inverts the trust relationship the identity exists to establish.
   - **`.Image` is the image ID**, the sha256 of the local image config — not the registry
     `RepoDigests`, which is the only value corresponding to what a registry served or signed.
   - **`/etc/hostname` is the container ID only by Docker's default**; compose `hostname:`,
     Kubernetes (pod name) and ACA all override it.

   Since it cannot work on ACA, in the CLI, or in tests, the `source_pin` path is needed regardless,
   and the digest branch would be dead exactly where verdicts are produced for real.

   ### What it is, and is not

   An injected digest is still **self-reported**: the kernel repeats what it was told, so a
   compromised or misconfigured one writes what it likes. It is provenance, not warrant — the same
   circularity §5 identifies in a self-signed receipt — and it becomes load-bearing only when
   something outside the process vouches for the binding (the orchestrator's own record, a signed
   deployment, a TEE attestation). That is the same trust-boundary trigger §5 gives for receipts,
   and it is the point at which `#43`'s reproducible-build machinery becomes the nearest existing
   answer.

   **What keeps this from becoming a permanent stopgap**: the slot is a *kind* plus a value, so a
   stronger identity adds a kind rather than reshaping the schema — no reseed, and no migration of
   traces already committed. A `source_pin` trace stays readable and stays honestly labelled as the
   weaker thing it is.
