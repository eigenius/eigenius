# D90 — The institution result contract

*Written and built `2026-09-13`. Closes eigenius#226.*

**Status: implemented.** "The work" below records what landed and what building it found. §1-§4
describe the gap as it stood before, in the present tense, because that is what they are for;
`kernel/src/institution/result_contract.rs` is what closed it.

## The gap

An institution's declared contract is an **input** class. `institution/marshal.rs` resolves it
and checks arity and per-property shape on the way in. There is no enforced contract on the way
out: `build_verdict_resource` copies every non-protected property the institution returned onto
the chain-committed `Verdict` verbatim.

That value is not inert. The epistemic claim an institution computes — the statistics
institution's proposition, a checker's result — is read by the witness emitter and can end up
discharging a `justification:Grounds` citation. It is the one thing crossing the boundary that
has to be right.

## What already exists, and is not used

Three pieces are in place. The work is enforcement, not vocabulary.

| declared | enforced |
|---|---|
| `QueryClass.result_class` — "the class of result resources this QueryClass produces", a **required** property | never. Read at registration into the registry struct, copied onto emitted resources, shown by `inspect`. No validation rule reads it; dispatch never consults it |
| `ExportFormat.payload_type` / `ImportFormat.payload_type` | the comorphism path's typed boundary, outside this note's scope |
| `eigentt:Judgement` — `holds(logic, term, type)`, checked in CHECK mode: decode both halves, check `type` is a type, check `term` against it | yes, by Rule 21, **where a slot is ranged on it** |

`dispatch.rs:182` carries a comment saying AutoOnLoad query classes "must declare Verdict as
their result_class … surface it here rather than silently mis-dispatching". The code below it
checks only the dispatch role. The check the comment describes does not exist.

**And the declared result class constrains nothing.** `Verdict` declares no `requires` and no
`recommends`. Combined with open-world carrying — any resource may carry any declared property
— "you return a `Verdict`" permits anything to ride along.

## What is actually reachable today

Measured, not argued: `kernel/tests/which_ranges_admit_a_proposition.rs`. A real D85-encoded
term on a declared property, committed and validated:

| declared range | outcome |
|---|---|
| `core:json` | **admitted** |
| `core:resource`, no `class_types` | **admitted** |
| `core:resource` + `class_types` | refused — `ClassTypeMismatch` |
| `core:inductive`, no `class_types` | refused — requires exactly one InductiveType range |
| `core:inductive` + wrong `class_types` | refused twice |
| `core:string` | refused — `TypeMismatch` |

So the `core:inductive` family is closed: a slot that declares what it holds refuses a term that
is not it. The door is open only where a slot declares **no range** — which is what makes the
obligation in step 2 the fix, rather than anything about how the kernel checks terms.

The live institutions do not walk through it. **Narrower than the draft assumed**: a gate verdict
from statistics or Lean carries `institution:diagnostic` and nothing else, and the proposition and
the numerics ride on the DERIVATIONS rather than on the verdict. The one institution that could
walk through it is `capability/external_institution.rs`, which returns whatever the substrate hands
back, unexamined. **The gap is structural, and only the external path exercises it.**

## Inbound is covered, by ordering rather than by contract

`commit/phases.rs` validates in 3.2, runs the retroactive cascade in 3.3, and dispatches
AutoOnLoad in **3.4** — so a subject reaching an institution has already been through the rules.
The Decidable path evaluates its argument expressions through the kernel's own evaluator before
marshalling them.

Both are true and neither is *stated*. Coverage that holds because of phase ordering is the same
defect class as coverage that holds because of how someone declared a slot. The protocol should
say what it requires.

## The decision: the epistemic claim is DECLARED, in one of two checked forms

A verdict's epistemic slot must be declared as one of:

- **a proposition** — `class_types eigentt:Term` with `eigentt:expected_type`. Rule 21's case 1
  checks the value AGAINST that type rather than inferring; `eigentt:proposition` declares
  `expected_type = Sort(0)`, so it is checked to be a well-formed `Prop`.
- **a judgement** — `class_types eigentt:Judgement`. `holds(logic, term, type)`, checked in CHECK
  mode: both halves decoded, the type checked to be a type, the term checked against it.

**Not "everything becomes a judgement".** That reading would force every institution to be a
LOGIC, and most are not. `holds(logic, t, P)` asserts *a checker for `logic` verified `t` against
`P`*. Statistics does not prove anything — it
computes a number and compares it to a threshold. Writing that as `holds(logic_statistics, …)`
asserts a verification that did not happen, which is the same overclaim the three-grounds change
removed when it deleted `IsDerivedAs`. The paper's line is that the Verified state is provable
while Declared and Observed are postulated; a computing institution belongs on the postulated
side and should not be issuing `holds`.

The two forms are checked equally strictly. They differ in what they ASSERT, and an institution
should assert the weaker one unless it really proved something.

### Why not a witness

The tempting move is to make a `Verdict` witness-shaped so it plugs straight into the
justification framework. Three reasons it does not work, one of them decisive.

1. **It would re-mint a grade the design deleted.** There is no `IsDerivedAs`. A computed result
   gets no witness of its own; it is grounded as `App(Declared(plan), Observed(inputs))`, because
   running a program once does not establish that the program computes a function of its input —
   determinism is a fact about the environment. `trace_category` says it in the source:
   *"A `ProgramTrace` grounds NOTHING."* A witness-shaped verdict would grant standing for having
   executed, which is exactly what the three-grounds change removed.
2. **A witness has no payload.** Proof irrelevance makes any two witnesses of the same
   `(category, iri, P)` definitionally equal. A verdict carries the computed statistic, the
   p-value, the diagnostic, the runtime invocation and timings — the audit record of a run, which
   a witness has nowhere to put.
3. **A witness has no negative form.** `Fails` and `Undecidable` are verdicts the system must
   record, and there is no witness for them.

### Why a judgement confers nothing it should not

**The trace kind decides the grade, not the presence of a judgement.** `trace_category` maps
`DeclarationTrace → Declared`, `ObservationTrace → Observed`, `VerificationTrace → Verified`, and
`ProgramTrace → None`. So a judgement on a `Verdict` is type-checked without granting the run any
standing:

- a statistics run emits a `ProgramTrace`, so no witness — its claim stays grounded as
  `App(Declared(plan), Observed(inputs))`, exactly as today;
- a Lean check emits a `VerificationTrace` carrying `prov:judgement`, which is what admits
  `Verified` — the path that already works this way.

The audit content stays ordinary properties. Only the epistemic claim moves.

## The κ–τ case: a computation, not a logic

The κ–τ framework for risk-sensitive abduction is the first external framework proposed for the
platform. It uses form one.

**Its threshold comparison is not proved, and does not need to be.** The case study assumes the
comparison "becomes decidable by evaluation", which would let the framework supply a proof term
and reach `Verified`. The kernel cannot do that: `stats:lt` / `le` / `gt` / `ge` are AXIOMS,
uninterpreted predicates the kernel does not evaluate. D86 gives the reason — the kernel cannot
evaluate floats, so a numeric claim is matched syntactically rather than decided, and Mathlib
carries practically no `Float` lemmas because it does exact computation over `Rat`.

**Statistics already shows what to do instead.** The statistics institution compares a p-value to
alpha. It does not prove that comparison — it computes it, asserts the resulting proposition,
emits a `ProgramTrace`, and earns no witness. The claim is grounded downstream as
`App(Declared(plan), Observed(inputs))`, and the ontology states the reason: a computed claim rests on the assertion that the plan denotes a function of its input,
which no execution establishes, plus the input.

κ–τ's threshold comparison is that shape exactly. So:

- **Form one**: it emits a proposition with a declared `expected_type`, not a judgement.
- **No `logic_κτ` to mint.** `holds(logic, t, P)` asserts that a checker for `logic` verified `t`,
  and no checker did.
- **Its trace is a `ProgramTrace`**, so it earns no witness, and `Commits(τ, φ)` is grounded as a
  declared scoring plan applied to observed estimates.

That is the composite warrant the case study describes, and it matches the framework author's own
reproducibility split: scoring, lifting and the threshold/margin comparison are strictly
reproducible, while the embedding-based estimates are recordings under a declared protocol. An
application forms over the first and none over the second, so nothing is entailed about subsequent
estimates.

**Lean remains the judgement example.** That gives the two forms a clean pairing: Lean proves and
emits a judgement, statistics and κ–τ compute and emit propositions. Both forms still have an
instance, so the split in the section above is not an anticipation of one.

**Both κ–τ outputs take the same warrant.** The framework author's note grades `SuspendedAt` as
Computed; `Commits(τ, φ)` is Computed for the same reason, since one evaluation decides both over
the same inputs. The numeric pivot D86 points at would let a checker DECIDE such a comparison
rather than assert it, which is worth building — nothing here waits on it.

**The contract closes the type gap and not the semantic one.** The framework establishes
`Commits(τ, φ)`, not `φ`. The gap is crossed by a DECLARED bridge `Commits(τ, φ) → φ`, attributed
to an owner, and the justification term exhibits that leaf. Nothing in this contract enforces
that: a slot is checked for holding a well-formed proposition, never for which predicate it names.
The bridge stays a discipline backed by attribution. Say so plainly, so no one later assumes the
contract covers it.

Because the projections that ask *which estimates does this rest on* and *does it survive removing
one* are queries over the justification term, the emitted term must be composite rather than one
opaque leaf — which is the same reason the grounding rule above exists.

### Suspension is a proposition, not an annotation on a refusal

Below threshold, the correct veto semantics is `Undecidable` for `Commits(τ, φ)` — a refusal to
commit, not a claim that the chain is invalid. But suspension is a first-class inferential output
in κ–τ, and its auditability — which hypotheses stayed open, against which rivals, failing which
margins — is half the framework's point. An `Undecidable` verdict records none of that.

**The fix is inside the existing scheme.** A verdict that is not `Holds` carries no proposition,
deliberately: the statistics path attaches the claim only on `Holds`, matching the witness
emitter's structural filter. So the answer is not to hang something off a refusal. It is a
SEPARATE positive claim — `SuspendedAt(τ, φ, rivals, margins)`, a proposition about the governance
state, with nothing about `φ` leaking through it.

That is the two-level shape the tree already uses: the gate `Verdict` attests that the analysis
plan is runnable, while `EmittedDerivation`s carry the per-effect decisions. Suspension rides as
its own derivation with its own `Holds`, while the gate stays `Undecidable` for `Commits(τ, φ)`.
Auditable suspension is then a consequence of the architecture rather than an exception to it,
and `SuspendedAt` is declared in the institution's output vocabulary — which is what a result
contract is for.

## The work

**All six steps landed `2026-09-13`.** `kernel/src/institution/result_contract.rs` runs three
checks at the dispatch boundary, before the dispatch is recorded.

1. **`Verdict` gained a declared shape.** It requires `core:ctor_name` and
   `institution:verdict_subject`, the two halves every committed Verdict already carried, and
   recommends the three audit slots.
2. **A term-bearing output slot is declared in one of the two forms.** A value whose `is_a` names
   a constructor class of `eigentt:Term` or `eigentt:Judgement` may only sit on a property
   declared `class_types [eigentt:Term]` WITH an `eigentt:expected_type`, or `class_types
   [eigentt:Judgement]`. Checked on the gate output AND on every emitted derivation, because a
   derivation is output too and the hole is the same there.
3. **The output contract is closed.** Every property the institution set must be declared by the
   QueryClass's `result_class`, transitively over `subclass_of`, or listed in its
   `institution:result_properties`.
4. **The inbound obligation is stated**, in two places. A test pins that `structural_validate`
   and `retroactive_with_cascade` both precede `autoonload_dispatch` in the phase slice — the
   existing pipeline test asserts phase COUNT, which a reorder would pass. And `QueryClass`'s
   description states both halves of the protocol, so an institution author reads what the kernel
   guarantees inbound and what it checks outbound.
5. **A QueryClass declares which verdict constructors it may return.**
   `institution:permitted_verdicts`, a `core:resource_array` with `allows_only` over the three
   constructor classes D85 materialises. Absent means all three, which is what every QueryClass
   meant before.
6. **Housekeeping.** The two stale comments at the dispatch site are corrected: the one claiming
   a `result_class` check that did not exist, and the one at the property merge saying the
   institution's output is copied and not type-checked.

### What building it found

**The subclass reading of a closed `result_class` is not available.** Rule 25 closes an inductive:
a class may name one in `subclass_of` only from that inductive's own layer, and
`institution:Verdict` is an inductive. So no later layer can subclass it, and "every institution
declares a `Verdict` subclass" would be refused at commit. `institution:result_properties` on the
QueryClass says the same thing directly, and says it better — two QueryClasses of one institution
legitimately return different property sets, which a class per institution would not express.

**The closed contract costs zero declarations today.** With `Verdict` declaring `ctor_name`,
`verdict_subject` and the three audit slots, every live institution passes on that alone: the
whole workspace suite is green apart from the manifest pin. Statistics returns only
`institution:diagnostic` on its gate verdict — the proposition and the numerics ride on the
derivations, not on the verdict — so it needs no `result_properties` entry at all.

**One layer moved.** `institution` only, so the reseed is the cheapest kind.

### What a review found after it landed

An adversarial review of the first implementation found four soundness defects in the checks
themselves. All four are fixed; they are recorded because each is a way to get this kind of check
wrong, not a one-off slip.

**An exemption list is a hole, and a hand-kept one drifts wider.** The first version carried one
allowlist of "properties the kernel stamps", used at two different positions. Three of its entries
are not stamped at the position where they were exempted — `institution:from_subject` on the gate
output is stamped nowhere and merged verbatim, and `verdict_subject` and `runtime_invocation` on a
derivation are stamped only conditionally, never for an in-process dispatch. All three are declared
`core:resource` with no range, which is the one shape that admits a term. So a term rode out on
`from_subject` with zero violations and zero validation errors: eigenius#226 through a second door,
opened by the fix for the first. Each position now derives its exemption from the code that does
the stamping, so the exemption cannot be wider than what is actually stamped.

**A boundary check has to descend.** The first version looked at top-level properties only, so a
term one level down inside a declared, correctly-ranged wrapper reached the chain: the closed check
saw the outer key and nothing looked at the inner one. That falsified this note's own §Scope claim
that anything the QueryClass does not list is rejected regardless. The walk now recurses, and stops
where it should — once a value IS a term it does not descend into that term's constructor
arguments, which are nested embedded resources that Rule 21 deliberately exempts for the same
reason.

**"Checked" had more forms than the check knew.** Rule 21 checks a term slot four ways; the first
version accepted two and refused the rest, including `eigentt:is_a_type`, which runs a full
`check_type` and is STRICTER than the form that was accepted. That would have refused any
institution emitting an axiom or a definition — `eigentt:axiom_statement`, `core:ctor_type`,
`lexicon:sem_type`. What stays refused is the one form that really does only infer.

**The check the deleted comment described was still missing.** `dispatch.rs` carried a comment
saying AutoOnLoad QueryClasses must declare `Verdict` as their `result_class`; step 6 corrected the
comment and did not add the check. Meanwhile `build_verdict_resource` stamps `is_a: [Verdict]`
whatever was declared, so the closed check ran against a class the committed resource need not be
an instance of, and an institution could widen its own contract by naming a wider one. Now checked.

Two smaller ones: `permitted_verdicts` matched by stripping the suffix off the entry, which also
matched `urn:eigenius:anything-Holds`, and now compares forwards through `ctor_classes::class_iri`;
and `declared_properties` ignored `core:conditional_requires`, so a property a condition makes
required would have been refused as undeclared.

**The fixture did not validate.** The tests built a QueryClass naming an institution and a gated
class that nothing declared, and passed anyway, because dispatch resolves neither. A refusal from a
chain that cannot commit proves nothing about the boundary, so `run_case` now runs the Validator
over its own layer first — which caught this immediately.

### Two things the contract still does not do

**Derivations get the term check and not the closed check.** `result_class` describes the gate
output, and a derivation carries its own class, so Rule 1, Rule 3 and Rule 21 constrain it at
commit. But nothing says which derivation CLASSES a QueryClass may emit, which means the channel
carrying the epistemic payload — the proposition and the numerics ride on derivations, not on the
verdict — is the one still open-world. That is why closing the gate's contract cost zero
declarations. Worth its own step; it is not this note's.

**The bridge is still discipline.** A slot is checked for holding a well-formed proposition, never
for which predicate it names. Nothing here stops an institution asserting `φ` where it should
assert a proposition about `φ`.

**One of the three checks belongs on the Decidable path as well.** `decide_institution`
calls the same `query` handler during type-check reduction, reads the verdict constructor to
reduce a `NativeDecide`, and DISCARDS everything else. So the closed-property and term-slot
checks have nothing to protect there — no output reaches the chain. The permitted set does: a
QueryClass that declared it returns `Holds` or `Undecidable` and never `Fails` has to be held to
that on both paths, or one declaration would mean two things. `check_permitted_verdict` is split
out for it, with a test and its matched control.

## Open questions

- ~~Is `result_class` a CLOSED contract?~~ **Closed, and carried by
  `institution:result_properties` rather than by a `Verdict` subclass** — see "What building it
  found" above. It cost nothing to close, because no live institution returns a property outside
  what `Verdict` now declares.

- ~~What is `term` for a non-proof institution?~~ **Dissolved.** A non-proof institution should
  not emit `holds` at all, so there is no term to invent.
- **Does a stricter `result_class` close the open-world door?** Checking the verdict against the
  result class constrains what an institution may return. It does not stop a *different* resource
  carrying a term on an unranged slot. Whether to also require a range wherever a slot admits an
  embedded value is a separate question — the 33 resource-typed properties that lack one are the
  program AST slots, and they have no common superclass to name.
- **Manifest and reseed.** Steps 1 and 2 are bootstrap ontology edits and move the manifest.

## What this means for statistics

**Nothing.** It needs no edit, and this is the measured answer rather than the predicted one.

Its gate verdict carries `institution:diagnostic` and nothing else. The proposition and the
numerics ride on the DERIVATIONS — one `StatisticalAnalysisResult` per effect, carrying
`eigentt:proposition`, `computed_statistic` and `computed_p_value`. The draft assumed they sat on
the verdict, and predicted statistics would need a `Verdict` subclass declaring them. It does not:
`Verdict` recommends `diagnostic`, so statistics passes the closed contract on that alone.

Its output shape was already right. `eigentt:proposition` is declared `core:inductive` +
`class_types [eigentt:Term]` + `expected_type = Sort(0)`, which is form one and is checked against
`Prop` in check mode. `computed_statistic` and `computed_p_value` are `core:float` scalars and
carry no term. It emits a `ProgramTrace`, so it earns no witness, and its claim stays grounded as
`App(Declared(plan_yields_effect), Observed(sample_set))`.

**Lean is the same story**, and for the same reason: its verdict carries a ctor and a diagnostic,
and its judgement rides on a `VerificationTrace`.

**The institution the contract actually bites on is the external one.**
`capability/external_institution.rs` returns whatever the substrate hands back, unexamined. That
is the open door, and it is now closed by declaration rather than by trust.

## Scope

**This note is about the institution interface, and nothing else.** An institution declares its
own result properties, so what closes the gap is an obligation on those: step 2. How any other
property in the system is declared does not reach this path, and under a closed result contract
(step 3) anything the QueryClass does not list is rejected regardless.

Two things follow, and they are the only reason anything outside the interface is mentioned here.

**A blanket "every property must declare what it holds" is not available as a cheaper global
alternative.** The properties that declare no range are the program AST's — a body, a function, an
argument can each hold any kind of expression, and the expression classes share no parent to name.
Giving them one is modelling work on how programs are typed, with its own blast radius. So the
obligation goes on institution output, where the declarer is the institution itself.

**`core:json` is not a gap.** A JSON value is a blob; nothing reads one as a term unless a
declaration says to. An institution storing a proposition-shaped blob has stored data, not
smuggled a claim — and step 2 is what stops it calling that blob its epistemic output.
