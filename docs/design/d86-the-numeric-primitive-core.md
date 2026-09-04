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

## 3. The core: three primitives and one refinement

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

### 3.4 Exclude NaN at the chain level, by refinement

```
NaN == NaN  → false
NaN ≤ NaN   → false
1.0 ≤ 1.0   → true
```

Both relations are non-reflexive on NaN, so the order is **partial** and ordinary reasoning
breaks: `¬(x ≤ y)` stops implying `y < x`. Constrain measured quantities to non-NaN on the chain
and the order is total and classical on that subset.

This costs no new machinery. D30 already emits a refinement as a Lean `Subtype`
(`{ x : Float // … }`, visible in the golden `Mirror.lean`), and D74 §4.5 already translates `Sig`
to exactly that.

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
| non-NaN refinement → `Subtype` | **done** (D30 emission + D74 §4.5) |
| declare `≤` and `==` on the chain | one ESL edit |
| D30 emits chain definitions as Lean `def`s | the real work — it emits `structure`s only |

The last row is the one that carries the design property. If D30 generates the Lean side from the
chain declaration, the two agree because one produced the other; if a human writes both, they
agree until someone edits one. It is the same v1.1 gap that holds inductives
(`mirror_gen/mod.rs`: *"`InductiveType` bucket — those land with D30 v1.1"*).

### 5.1 The ontology edit is one reseed, so it carries passengers

Tracked as eigenius#235; the last row of §5 is eigenius#236.

Any change to a bootstrap ontology's *content* moves the manifest hash — `description` values
included; only ESL comments and JSON layout are exempt
(`bootstrap::tests::the_manifest_hashes_content_not_presentation`). Persisted stores then refuse to
resume with `ManifestDrift` and have to be reseeded. Four edits are queued behind that cost and
should land in one pass:

| edit | file | why |
|---|---|---|
| declare `≤` and `==` over `core:float` | `ontologies/…` (§6.1 decides where) | row 3 |
| a permitted-axiom slot on `prov:VerificationTrace` | `ontologies/prov/prov.esl` | the axiom allowlist is the TCB of every Lean verdict and the trace does not record it — two proofs, one leaning on `Classical.choice` and one not, produce byte-identical traces (`institution.rs::do_proof_check`) |
| `witness:IsVerifiedAs` description | `ontologies/core/core-ontology.json` | says *"Nothing in the tree currently emits one — the producer is the Lean institution under eigenius#160."* The Lean institution emits one as of `2026-09-03` |
| `justification:VerifiedPropositionView` | `ontologies/justification/justification.esl` | its description says the witness emitter looks the view up by `source_verified_resource`; the emitter reads the claim's `canonical_proposition` and never touches the view. Nothing reads the class at all — declared, bootstrap-registered, referenced by two `esl::compile` tests, and otherwise dead. **Recommend deleting it** rather than rewording: the comorphism route it served was replaced by D74's forward externalization (D51 §3) |

## 6. Open

1. **Where the primitive correspondence is stated.** A property on the chain axiom, an entry in
   the generator, or a fixed table in D30. Whichever it is, there should be one of it, and it
   should be short enough to read in full.
2. **Whether `==` should be `BEq` or a `Decidable` equality**, and what the chain-side name is.
3. **Whether the non-NaN refinement is mandatory on `core:float`** or opt-in per property. Making
   it mandatory makes every float total-ordered and removes a class of mistake; making it opt-in
   keeps `core:float` faithful to IEEE, which includes NaN.
