# D86 — The numeric primitive core over `core:float`

*Status: **proposed** `2026-09-04` · design note.*

*Companion documents: [D74](d74-eigentt-to-lean-externalization.md) §4.8 (floats are translated;
the relation is not), [D30](d30-eigon-to-lean-faithful-translation.md) (what the mirror emits),
and `docs/guides/esl/09-institutions.md` §9.11.2 (why a comorphism is trusted rather than
checked).*

---

## 1. The gap this closes

D74 §4.8 made a measured quantity expressible: `LitFloat` translates to Lean's `OfScientific`
spine and `core:float` to `Float`, exactly and verifiably. A claim can now say `Measured(0.1)`.

It cannot say `0.0 ≤ x` and mean Lean's `≤`. That relation is
`@LE.le.{0} Float instLEFloat …`, and nothing maps a chain relation onto a Lean typeclass
operator at a chosen instance — D30 mirrors classes as `structure`s and stops there. A chain
axiom `stats:le : core:float -> core:float -> Prop` externalizes to a `Const` under D74 §3.3's
mangling, which is a name the export must declare and which is *not* Lean's `≤`. A proof about it
inherits none of Lean's order lemmas, so the exercise buys nothing.

Every measurement claim in the WRN chain is of this shape — `p < 0.05`, `IC50 ≤ threshold`,
effect size above zero — so this is the boundary between the machinery and the application.

## 2. Why it cannot be closed by declaration alone

Saying "`stats:le` **means** `@LE.le.{0} Float instLEFloat`" is unfalsifiable from inside either
system. It is §9.11.2's **obligation 2** — satisfaction-preservation by the comorphism — which
that section already records as *"Non-executable, because the two inhabitants live in different
type theories and the comorphism maps the proposition rather than the proof term"*, and names as
where the risk actually sits: *"the practical danger in admitting a Lean proof is not that Lean's
kernel accepts falsehoods; it is that the translated `P'` fails to denote the `P` you meant."*

Each such correspondence enters the trusted computing base, which §9.11.2 defines as the kernel's
checker, each hosted external checker, **each formal comorphism**, and the constant specification.

So the goal is not to eliminate the assertion. It is to make the asserted set **small, fixed, and
argued once**, with everything else *derived on the chain* and *generated into the mirror* — so
the two agree by construction rather than by a table someone maintains. That is D74 §3.3's
discipline (one mangling function called by both sides) applied one level up.

## 3. The core: three primitives, and a refinement that stays optional

### 3.1 `core:float` ↔ `Float`

Done (D74 §4.8). IEEE 754/854 binary64. Lean's `Float` is a structure over a `UInt64` bit pattern
with a `binary64.Valid` proof, and its arithmetic reduces in the kernel.

### 3.2 One ordering relation: `≤`

`<`, `>` and `≥` all derive. `<` specifically as `x ≤ y ∧ ¬(x == y)` — correct at signed zero,
where `≤` and `==` both hold and `<` must come out false.

### 3.3 One equality: IEEE `==`, **not** Lean's `Eq`

This is the trap, and getting it wrong is silent. Measured against the pinned toolchain:

```
0.0 == -0.0                → true    -- IEEE equality (BEq / instBEqFloat)
0.0.toBits == -0.0.toBits  → false   -- bit equality, which is what `Eq` compares
```

Lean's propositional `Eq` on `Float` is **structural** — same bits — so it distinguishes `0.0`
from `-0.0`. No measurement claim means that. A chain equality that translated to `Eq` would
quietly be making claims about signed zero, which is D74 §5's failure mode arriving through the
back door.

### 3.4 NaN is admitted, with its defined behaviour — decided `2026-09-05`

```
NaN == NaN  → false
NaN ≤ NaN   → false
1.0 ≤ 1.0   → true
```

Both relations are non-reflexive on NaN, so the order is **partial**: `¬(x ≤ y)` stops implying
`y < x`.

**That is not a defect to engineer around. It is the standard**
([Goldberg 1991](https://doi.org/10.1145/103162.103163)). NaN's behaviour is *specified*, not
undefined — comparisons are false, and it propagates through arithmetic — and IEEE 754 exists so
that behaviour is principled and portable rather than per-implementation. The statistics tooling
this chain records already behaves that way: R, NumPy and Julia all propagate NaN, so a statistical
computation's output can legitimately be one. `core:float` is IEEE binary64, NaN included.

**An earlier draft of this section excluded NaN by refinement.** That contradicted §3.3 one
paragraph above it. §3.3 rejects Lean's structural `Eq` precisely *because* IEEE semantics are what
a measurement means; excluding NaN declines to defer to IEEE immediately after insisting on it.
Taking the standard seriously in one place and working around it in the next is the inconsistency,
and admitting NaN is the consistent reading.

**What it costs, stated plainly.** A proof about a possibly-NaN quantity has to handle NaN. That is
correct rather than burdensome — the theorem is weaker because the fact is weaker, and a claim
`0.0 ≤ x` is simply **false** for a NaN `x`, which is the right answer: a NaN measurement does not
satisfy a bound.

**§3.2's derivation survives unchanged.** `x < y` as `x ≤ y ∧ ¬(x == y)` gives, for NaN,
`false ∧ ¬false = false` — NaN is less than nothing, which is IEEE's answer.

**Refinement stays available, as an ordinary thing anyone can do.** An author who knows a quantity
is non-NaN can say so per property and get the total order back, and it costs no new machinery: D30
already emits a refinement as a Lean `Subtype` (`{ x : Float // … }`, visible in the golden
`Mirror.lean`) and D74 §4.5 translates `Sig` to exactly that. What changed is that this is a
property author's choice about a specific quantity, not a constraint on the primitive.

### 3.5 Arithmetic is deliberately *out*

`+`, `×`, `-`, `/` are not in the core. D74 §4.2 refuses computation inside a proposition —
`Map`, `Reduce`, `NativeDecide`, `DecEq` are all out — and a proposition **relates quantities the
chain already computed** rather than computing them. `stats:mean_of(X)` stays an opaque chain
term and the claim says `mean_of(X) ≤ 0.05`.

Admitting arithmetic would mean asserting that chain arithmetic and IEEE arithmetic agree — a far
larger obligation, for no gain in what claims can be *stated*.

## 4. What this makes true, and what it does not

**True.** A claim can state a relation between measured quantities, and a Lean proof of that
relation can be checked against it — with the quantities meaning IEEE doubles, rounding included,
which is what the pipeline actually produced.

**Not true.** The correspondence in §3.2 and §3.3 is asserted, not checked. Two primitives is the
smallest set that supports the claim shapes the WRN chain contains, and the argument for each is
the ordinary one: the Lean side is `instLEFloat` / `instBEqFloat`, both defined over the same
binary64 model the chain's `core:float` denotes.

**Deliberately not attempted.** Interval or error-bar semantics. A claim that a measurement lies
within a tolerance is expressible with `≤` alone (`lo ≤ x ∧ x ≤ hi`), and anything richer is a
modelling question about the chain, not about the comorphism.

## 5. What it takes to build

| | |
|---|---|
| `core:float` ↔ `Float` | **done** (D74 §4.8) |
| optional non-NaN refinement → `Subtype` | **done** (D30 emission + D74 §4.5), and optional per §3.4 |
| declare `≤` and `float_ieee_eq` on the chain | **done** `2026-09-05`; `≤` was already there, so one relation. §5.1 |
| the correspondence itself, as §6.1's table | **done** — `NumericRel` in `crates/eigenius-lean/src/externalize.rs`, read at the `App` arm |
| D30 emits chain definitions as Lean `def`s | the real work — it emits `structure`s only. Still open (eigenius#236) |

The last row is the one that carries the design property. If D30 generates the Lean side from the
chain declaration, the two agree because one produced the other; if a human writes both, they
agree until someone edits one. It is the same v1.1 gap that holds inductives
(`mirror_gen/mod.rs`: *"`InductiveType` bucket — those land with D30 v1.1"*).

### 5.1 The ontology edit is one reseed, so it carries passengers

Tracked as eigenius#235; the last row of §5 is eigenius#236.

**Landed `2026-09-05`** on `numeric-core-and-verification-judgement`, and the passenger list grew
from four to seven while the batch was built — each addition free, because the reseed had not run
yet. Two corrections to the table below, both found by measuring:

- **Row 1 is one relation, not two.** `stats:le : core:float -> core:float -> Prop` already existed
  in `ontologies/statistics/statistics.esl` (namespace `urn:eigenius:measurements`), alongside `lt`,
  `gt` and `ge` — §1 above uses it as its own example of a relation that externalizes to a `Const`
  the export does not declare. Only `float_ieee_eq` is new.
- **The four ordering axioms stay axioms**, against §3.2's "`<`, `>` and `≥` all derive". A `def` is
  transparent — decode substitutes the body at the use site — and the DCG recognises
  `measurements:gt` / `lt` **by IRI on the decoded term** (`dcg/category.rs`,
  `dcg/rules/combinators.rs`), which is the form the WordNet importer emits. Deriving them on the
  chain would dissolve the head the parser matches on. The derivation lives in §6.1's table
  instead, where `Ge` is `le(y, x)`, `Gt` is `lt(y, x)` and `Lt` is `le(x, y) ∧ ¬eq(x, y)` — so the
  asserted set stays at two whatever the surface offers, which is what §4 claims for it.

Any change to a bootstrap ontology's *content* moves the manifest hash — `description` values
included; only ESL comments and JSON layout are exempt
(`bootstrap::tests::the_manifest_hashes_content_not_presentation`). Persisted stores then refuse to
resume with `ManifestDrift` and have to be reseeded. Four edits are queued behind that cost and
should land in one pass:

| edit | file | why |
|---|---|---|
| declare `≤` and `==` over `core:float` | `ontologies/…` (§6.1 decides where) | row 3 |
| a permitted-axiom slot **and a checker-identity slot** on `prov:VerificationTrace` ([D87](d87-the-verification-judgement.md) §5) | `ontologies/prov/prov.esl` | the axiom allowlist is the TCB of every Lean verdict and the trace does not record it — two proofs, one leaning on `Classical.choice` and one not, produce byte-identical traces (`institution.rs::do_proof_check`) |
| `witness:IsVerifiedAs` description | `ontologies/core/core-ontology.json` | says *"Nothing in the tree currently emits one — the producer is the Lean institution under eigenius#160."* The Lean institution emits one as of `2026-09-03` |
| `justification:VerifiedPropositionView` | `ontologies/justification/justification.esl` | its description says the witness emitter looks the view up by `source_verified_resource`; the emitter reads the claim's `canonical_proposition` and never touches the view. Nothing reads the class at all — declared, bootstrap-registered, referenced by two `esl::compile` tests, and otherwise dead. **Recommend deleting it** rather than rewording: the comorphism route it served was replaced by D74's forward externalization (D51 §3). **Deleted** |

Three more joined it: `prov:judgement` and `prov:checked_declaration` on `prov:VerificationTrace`
(D87 §5 and §7 — the second found by writing the recomputation test, since `prov:proof_term` names
the whole export and does not say which declaration was checked), the three phantom
`program:Component` declarations from the previous batch, and the `witness:Is*As` descriptions,
whose `IsVerifiedAs` sentence had been false since #160 and whose shared paragraph called the
kernel's synthesis a postulate — which the P7 closeout settled it is not.

## 6. Settled `2026-09-05`

1. **Where the primitive correspondence is stated: a table in the generator**, beside the
   externalizer that consumes it.

   The alternative — a property on the chain axiom — would let whoever authors an axiom write its
   Lean meaning, and `docs/guides/esl/09-institutions.md` §9.11.2 puts **each formal comorphism in
   the TCB**. That makes a TCB entry authorable by committing a resource, which is the
   self-nomination shape eigenius#23 deleted `epistemic_status` for. A table in D30 alone documents
   and enforces nothing. In Rust the correspondence is reviewed like the rest of the TCB (D74, D30,
   `nanoda_lib`) and cannot be extended from the chain. §5's "generate the Lean side from the
   chain" argument does not apply: that guards against two hand-written sides drifting, and this is
   a fixed set of two relations.

2. **`==` is `BEq` / `instBEqFloat`**, and the chain-side proposition is `(x == y) = true`.

   Not a `Decidable` equality: that decides `Eq`, and §3.3's measurement is that `Eq` on `Float` is
   structural — `0.0 == -0.0` is `true` while `0.0.toBits == -0.0.toBits` is `false`. A `Decidable`
   equality therefore delivers exactly the relation §3.3 rejects. **The name must carry the
   IEEE-ness on its face** — `float_ieee_eq` rather than `float_eq` — so it cannot be read as
   propositional equality by someone who has not read §3.3.

3. **NaN is admitted; there is no mandatory refinement.** See §3.4. The question as originally posed
   — mandatory on `core:float` or opt-in per property — assumed the refinement was the expected
   case. It is not: `core:float` is IEEE binary64, and refinement is an ordinary per-property choice
   for an author who knows a quantity is non-NaN.

   Two facts that would have made "mandatory" expensive, recorded for the avoidance of a rerun:
   nothing in the tree checks NaN today (no `is_nan` anywhere in validation or the ontology), so
   mandatory means a new validator rule *plus* a decision about floats already committed; and
   making it mandatory changes what a core primitive means, which is the most load-bearing edit
   available.
