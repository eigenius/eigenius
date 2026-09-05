# Build plan — the numeric core and the verification judgement

*Branch: `numeric-core-and-verification-judgement`. Covers eigenius#235, [D86](../design/d86-the-numeric-primitive-core.md),
[D87](../design/d87-the-verification-judgement.md), and §3.5(b) of the kernel-run-records batch.*

*Follows `kernel-run-records-build-plan.md` (merged as #238) and
`judgements-warrants-build-plan.md`, whose P0–P7 all landed.*

---

## 0. Why these travel together

**One reseed.** Every ontology edit below is bootstrap-resident, so each one alone would cost a
reseed — which invalidates every staged snapshot and forces re-deriving the demo artifacts and both
parse baselines. Paid once, the marginal cost of every further edit is zero. That turned out to
matter more than the count suggested: the list grew from five to seven while the batch was built,
and each addition was free because the reseed had not run yet.

**And one argument. These specs were written to close out P7.**
`judgements-warrants-build-plan.md` §"Open after P7" defers one question — whether the witness index
is a soundness boundary or a cache over relations — and says to answer it once, at the end. D87 §7
supplies the missing half: `Verified` was the family where the answer was *no*, and D87 §5 makes it
recomputable. D86 is the same move on the numeric side: shrink what the platform asserts, enlarge
what it can recompute.

So §3.5 is not a follow-on to this batch. It is the batch's conclusion, and the reason the documents
exist.

## 1. The ontology edit — one pass, then one reseed

Seven, in `ontologies/` — the plan opened with five; **3b** arrived while building §3.3 and §3.5,
and **6** came back after §3.5 withdrew what had subsumed it:

| # | edit | source |
|---|---|---|
| 1 | declare `≤` and `float_ieee_eq` over `core:float` | D86 §3.2, §3.3, §6 |
| 2 | a permitted-axiom slot on `prov:VerificationTrace` | D87 §5 |
| 3 | a checker-identity slot on `prov:VerificationTrace`, as **kind + value** | D87 §9.3 |
| 3b | *(added during §3.3)* `prov:judgement` — the checker's result — plus `prov:checked_declaration`, found by §3.5's recomputation test | D87 §5, §7 |
| 4 | delete `justification:VerifiedPropositionView` + `justification:source_verified_resource` | #235, confirmed `2026-09-05` |
| 5 | delete `components:Combine` / `Extract` / `Transform` | kernel-run-records §3.5(b) |
| 6 | `witness:Is*As` descriptions — `IsVerifiedAs`'s is false since #160, and all three call the synthesis a postulate | #235; §3.5 |

**Nothing here is undecided.** D86 §6 settled the three that were open (`2026-09-05`); §3.5(b) was
argued in the previous batch. Edit 4 also needs the two `esl::compile` test references repointed and
the `bootstrap/mod.rs:1375` registration removed.

**Edit 1 is one relation, not two.** `stats:le` already exists
(`ontologies/statistics/statistics.esl`, namespace `urn:eigenius:measurements`), alongside `lt`,
`gt` and `ge` — D86 §1 uses it as its own example of a relation that externalizes to a `Const` the
export does not declare. Only `float_ieee_eq` is new. The four ordering axioms **stay axioms**: a
`def` is transparent, and the DCG recognises `measurements:gt` / `lt` by IRI on the decoded term
(`dcg/category.rs`, `dcg/rules/combinators.rs` — the form the WordNet importer emits), so deriving
them on the chain would dissolve the head the parser matches on. §3.2's derivation moves into §2's
table instead, where it costs the TCB nothing.

**#235's third edit — `witness:IsVerifiedAs`'s description, false since #160 — was dropped as
subsumed by §3.5 step 3 and is BACK IN, because step 3 was withdrawn.** Dropping it was right while
the declaration was slated for deletion; §3.5 establishes that it stays, so the false sentence would
have shipped. Caught before the reseed ran. All three witness descriptions also lose the postulate
framing P7 asked about — the synthesis is a decision procedure over relations, so the index is a
cache and not a soundness boundary.

**Sequencing: land all five, then keep going — the reseed waits for §3.5.** §3.5 step 2 changes the
`Certificate` constructors and cannot join this pass, because the premise replacing `witness:Is*As`
does not exist until §3.3 emits the judgement, so the plan first read as two reseeds. It is one:
nothing between here and §3.5 needs a reseeded store to *develop* against — the unit tests build the
bootstrap in memory, and `gen_verification_demo` (§3.1) writes a JSON file. The reseed is a
verification step, not a prerequisite, so running it once after §3.5's edit costs one pass instead
of two and validates the shape the batch actually ends with.

## 2. D86 — the correspondence lives in Rust

§6.1 decided the primitive correspondence is a **table in the generator**, beside the externalizer
that consumes it, not a property on the chain axiom. The reason is a security one and worth
restating at the implementation site: `docs/guides/esl/09-institutions.md` §9.11.2 puts **each
formal comorphism in the TCB**, so an on-chain property would make a TCB entry authorable by
committing a resource — the self-nomination shape eigenius#23 deleted `epistemic_status` for.

So: a small fixed table mapping the two chain relations to `@LE.le.{0} Float instLEFloat` and
`instBEqFloat`, read by `crates/eigenius-lean/src/externalize.rs` when it meets the corresponding
`Const`. Short enough to read in full, per §6.1.

**`float_ieee_eq`, not `float_eq`.** The name carries the IEEE-ness so it cannot be read as
propositional equality by someone who has not read §3.3 — where `Eq` on `Float` is structural and
separates `0.0` from `-0.0`.

**NaN is admitted** (§3.4, decided `2026-09-05`, [Goldberg 1991](https://doi.org/10.1145/103162.103163)).
No mandatory refinement. Nothing in the tree checks NaN today and nothing should start; a claim
`0.0 ≤ x` is simply **false** for a NaN `x`, which is the right answer.

## 3. D87 — the judgement, and what it needs first

### 3.1 The fixture comes first (D87 §6)

`demo:lean:patient_1` is a `Patient` instance carrying a `canonical_proposition` that never mentions
it, and the proof is a tautology (`fun _ h => h`). So the demo shows the plumbing runs, not that it
discriminates. **Nothing downstream can be tested meaningfully until this is fixed**, which is why
it leads.

Regenerate through `gen_verification_demo` so the claim is a `justification:Conclusion` with
`subject_iri`, carrying a proposition that is *about* its subject — plus a near-miss variant that
must fail.

### 3.2 The term former (D87 §4.2)

A distinct `eigentt:Term` former for a checked proof, so the chain can tell "asserted without proof"
from "checked by nanoda". **Not** `eigentt:Axiom`, which is defined as *"a closed term whose type
the kernel admits without checking the term itself"* — the opposite of what is being recorded, and
the conflation #205 and #23 both eliminated elsewhere.

The largest piece: it lands in the D47 codec, conversion, and D74's exhaustive 43-variant match —
which will **refuse** it for externalization, since a checked-proof reference has no Lean
counterpart to translate to.

### 3.3 The emit (D87 §7)

`do_proof_check` emits `holds(logic_lean4, Checked(t), P)` alongside the trace. It already holds all
three arguments at the moment it currently discards them: the logic is fixed, `P` is the `Exp` it
just compared by `def_eq`, and the payload is resolved.

Then `Certificate.verified` consumes the judgement instead of `witness:IsVerifiedAs`. **Measured
`2026-09-04`: zero certificates cite `Certificate.verified` and zero resources carry
`justification:proof`, so this is greenfield** — no migration, no invalidated leaves.

### 3.4 Deploy by digest (D87 §9.3)

`deploy/bicep/modules/kernel.bicep` and `docker-compose.yml` pin by **tag** today, so neither
deployment is reproducible even in principle. Pin by digest and pass `EIGENIUS_IMAGE_DIGEST`; the
identity slot records `image_digest` when present and falls back to `source_pin`.

Worth doing on its own merits, independent of D87.

### 3.5 The P7 closeout — remove `witness:Is*As`

**This is why D86 and D87 were written.** `judgements-warrants-build-plan.md` §"Open after P7" asks
one question: *"A predicate the kernel inhabits by constant specification, computed from a relation
it can read at any time, is a decision procedure rather than a witness. If all three surviving
families are that, the index is a cache over relations — rebuildable, droppable, and not a soundness
boundary."*

D87 §7 gives the state of the answer: `Declared` and `Observed` are *plausibly* constant
specifications already; **`Verified` is the family where the answer is no today**, and §5 is what
changes it — once the inputs are pinned, *"nanoda accepted this"* becomes recomputable rather than
postulated. So §3.3 does not merely enable the closeout; it is the last input the closeout was
waiting on.

Three steps, in order:

1. ~~**Verify the word "plausibly."**~~ **Answered `2026-09-05`: yes for both, and the kernel
   already does it.** `layer_admits_witness` (`kernel/src/layer/witness_index.rs`) consults no
   committed witness. It is a pure function of the layer's Trace-class resources — reached through
   the triple index on `prov:resource`, or by iterating the layer when it is still in-flight — plus
   the target's `reflection:canonical_proposition`, or D39 §4.1's `Asserts(target_iri)` default when
   it carries none. Nothing about a witness is stored: the `Val::ChainWitness` is synthesised on
   demand at the certificate's type-check site. The module's own header says so — *"a pure
   deterministic function of that Layer's Trace-class resources … nothing here is persisted"* — and
   `lookup_chain_witness`'s first-hit-wins walk is sound for the reason a decision procedure needs,
   Layer immutability making a once-admitted witness stay admitted in every descendant.

   Two caches sit on top and neither is a soundness boundary, because both fail conservatively.
   `LayerHandle::has_witness_candidates` prunes a layer that stamped no candidate; it defaults to
   `true` on deserialization, so an old handle is probed rather than skipped. `any_trace_targeting`
   treats a poisoned `pending` lock as in-flight (`unwrap_or(true)`) and falls back to the full
   scan. Both turn a wrong guess into a refused certificate, never an admitted one.

   So `Declared` and `Observed` are decision procedures over relations, exactly as P7 supposed, and
   the index is a cache for them. `Verified` was the one family where it was not — the
   `VerificationTrace` route admits on the strength of a committed note, because the kernel cannot
   re-run nanoda at lookup time — and §3.3 is what changes that.
2. ~~**Change the three `Certificate` constructors' premises.**~~ ~~3. **Delete
   `witness:Is*As`.**~~ **Both withdrawn `2026-09-05`, and the reason is worth keeping.**

   These came from D87 §7's row calling `witness:IsVerifiedAs` *removable* once
   `Certificate.verified` consumes the judgement. It does not follow. The premise is what makes
   `Certificate(Verified(iri), P)` inhabitable only where the chain verified `P` about `iri`;
   delete it and the constructor is unconditional, so `Verified` becomes assertable by anyone
   writing a certificate — the laundering the two-layer separation exists to forbid. Nothing else
   can occupy the position either: a premise ranging over `eigentt:Judgement` would be a *data*
   type, inhabited by any well-formed value, and the CHECK-mode rule that catches a bad one runs at
   validation rather than inside the kernel's conversion.

   **P7's question was never about the types.** It asks whether the three families are decision
   procedures over relations, and says that if they are, *"the index is a cache over relations —
   rebuildable, droppable, and not a soundness boundary."* That is the conclusion, and it now
   holds. So step 3 became: say so where it was claimed otherwise, and prove the part that was
   newly true.

   - `witness_index.rs`'s header asserted *"this module is inside the TCB … the witness itself is
     postulated, and a wrong admission cannot be caught downstream because an axiom has no proof to
     re-check."* Rewritten: a wrong answer is now catchable by recomputation.
   - **`a_verdict_is_recomputable_from_what_the_trace_pins`** is the gate. It takes the five inputs
     off the committed trace, calls `check_proof` the way a third party would, and asserts the same
     verdict. Writing it found the fifth input: `prov:proof_term` names the export BLOB, which
     holds a whole Lean environment, so bytes plus a proposition does not say what was compared
     against what. **`prov:checked_declaration`** is now required.
   - The diagnostic P7 protects stated one remedy for all three families — commit a matching
     `canonical_proposition` — which is the fix for two and no help for `Verified`, where no
     property an author can write reaches the grade. It is now per-family, and names the trace that
     admits each.

**What must not be swept up**, per P7's own list: `hash_proposition_exp` and
`alpha_canonicalize_proposition_json` (proposition identity, needed by anything comparing
propositions), the α/δ agreement between emit and check sides that
`emit_and_check_sides_agree_on_the_hash` pins, and the diagnostic surface — *"a lookup miss naming
the family, the IRI and the property is the system's most-used error message."*

## 4. Deliberately out

- **eigenius#236** — D30 emitting chain definitions as Lean `def`s. It is the row of D86 §5 that
  carries the design property: generate the Lean side from the chain and the two agree because one
  produced the other, *"if a human writes both, they agree until someone edits one"*.

  §2's Rust table is the second hand-written side, so that warning applies to it. **What makes it
  safe to defer is that the drift cannot be silent.** If the chain relation and the table disagree,
  the externalizer falls through to D74 §3.3's mangling and emits a `Const` the export does not
  declare — and `checker.rs:132` sets `unknown_pp_declar_hard_error: true`, so nanoda refuses,
  while `ExternalizeError::UnknownConstant` names both the IRI and the Lean name it resolved to.
  Agreement-by-construction is stronger than agreement-enforced-by-a-hard-failure, but the weaker
  one does not admit a wrong answer, only a refused one.

  That is the whole of the argument. If a future change could make an unmapped relation externalize
  to something the export *does* declare, this exclusion stops holding.
- **An exporter for the PROV mapping** (`docs/spec/w3c-prov-mapping.md` §5) — needs the in-process
  Activity gap closed first (#145 territory).

## 5. Verification

Per `judgements-warrants-build-plan.md` §"Verification, every phase". Status `2026-09-05`:

| gate | where | state |
|---|---|---|
| `cargo test --workspace`, `fmt`, `clippy -D warnings` | — | green after each of §1–§3.5 |
| a claim about its subject verifies, its near-miss fails | `notebook_fixture_test` | **done**, and the near-miss fails on `def_eq` with both propositions true and both subjects in scope |
| a `Checked(t)` round-trips the D47 codec, and is refused by the externalizer | `eigentt_type_mirror`, `externalize_test` | **done**, plus the arm that matters more — a hand-authored `holds(logic_lean4, …)` is refused at commit |
| a `Holds` emits a judgement whose type is the claim's proposition | `notebook_fixture_test` | **done** |
| the verdict is recomputable from the trace alone | `a_verdict_is_recomputable_from_what_the_trace_pins` | **done**; it found `prov:checked_declaration` |
| the diagnostic still names the family, the IRI and the property | `witness_index` | **done**, and now names the right remedy per family |
| the demo artifacts re-derive | `gen_verification_demo` | **done** — two documents now, the demo and the near-miss |
| the reseed, then both parse baselines | `scripts/reseed-lexicon-db.sh --umls-all` | **done** `2026-09-05` — see below |

**The reseed's provenance has to match on every axis**, `--umls-all` included: the script's default
is the WRN-relevant TUI subset, every tracked snapshot is `--umls-all`, and the mismatch is silent.
Build the image with `CARGO_FEATURES=use-llm` for the same reason the compose default does — a
binary with no live ranker makes every parse a cap-only run, which is a different experiment.

```sh
CARGO_FEATURES=use-llm scripts/reseed-lexicon-db.sh --umls-all
```

### What the baselines then need, and the fork in the middle of it

Two baselines, four gates:

| baseline | gate | |
|---|---|---|
| `experiments/parsing/baseline.json` | `grammar_gap == 0 && missing_lexeme == 0` | **non-negotiable** |
| | `encoded >= 10` | the drift-free floor, not a peak draw |
| `experiments/parsing/selection-baseline.json` | `reading_correct >= expected`, `invalid_selected == 0` | |
| | `reading_unadjudicated == 0` | an unadjudicated decision makes the number a partial count |

Run `scripts/measure-parse-rate.sh` (it autodetects the newest snapshot and builds release —
load-bearing: a debug build overflows the stack in NbE readback and the harness reports it as a
grammar gap indistinguishable from a real one), then `scripts/eval-parse-rate.sh <run.log>
--baseline`.

**The fork is the replay.** The tracked rank draws were recorded against an earlier snapshot, and a
reseed can change the candidate sense lists — new `name` entries, recovered mass entries — so a
replay may MISS. `experiments/parsing/README.md` §3: *"a replay with `misses > 0` is a different
experiment, not a reproduction."* If the draw replays clean, the comparison is drift-free and the
run stands on its own. If it misses, the draw has to be **re-recorded live**, and a new draw
choosing novel readings leaves `reading_unadjudicated > 0` until those ledger rows are adjudicated —
which is judgement work, not a script. Budget for that branch rather than discovering it.

### What the reseed measured — `2026-09-05`

Snapshots: base `wordnet-umls-2026-09-05` (3.65 GiB), aligned
`wordnet-umls-aligned-2026-09-05`, from an image built at `7c24fa1`. Tracked draw
`ranks/2026-08-22-productiontrace.json` + `selections/2026-08-22-productiontrace-live.json`.

**The fork did not arise. `replay: 62 hits, 0 misses`** — the tracked draw reproduces exactly against
the new store, so the reseed did not move the sense space and no re-record was needed.

| | baseline | this run | |
|---|---|---|---|
| grammar-gap / missing-lexeme | 0 / 0 | 0 / 0 | **PASS**, non-negotiable |
| expected-hits | 62/62 | 62/62 | **PASS**, miss-set unchanged |
| reading-correct | 30/40 | 30/41 | **PASS** |
| reading-unadjudicated | 0 | 0 | **PASS** |
| invalid-selected | 0 | 0 | **PASS** |
| total-readings | 674 | 674 | identical |
| total-skeletons | 170 | 171 | +1, ungated, ceiling 250 |

**The live draw was not the measurement, and reading it as one would have produced a false
regression.** A single live run reported `expected-hits 60/62`, losing «Depletion of WRN induced
double-stranded DNA breaks.» and «Synthetic lethality is an interaction between two genetic
events.» Two checks placed it before anything was concluded: `grep -c 'malformed reply'` returned
0, so it was not eigenius#212, and the replay on the identical store and binary held all 62. The
baseline's own protocol note describes this shape — *"a single live run is a DRAW, not a
measurement … four earlier draws that day reported 61 hits twice on DIFFERENT sentences"* — so the
live number is a draw and there is nothing to re-baseline. **Neither baseline file is edited.**

**Nothing in this batch should move either number, and it is worth being exact about why**, since
the parser *does* use two of the relations §1 touches. `measurements:lt` and `gt` are what the
WordNet importer emits for gradable adjectives and what `dcg::category` matches by IRI — and both
are **unchanged**. What §1 adds is `float_ieee_eq`, which nothing in the lexicon path mentions;
what §2 adds is a table read only by the Lean externalizer, which the DCG never invokes. The
remaining edits are the `prov:VerificationTrace` slots, the `Checked` former and two deletions of
declarations with no readers.

The one thing that *does* reach the parser is the reseed itself — a moved manifest means a fresh
store, and a fresh store can change the candidate sense lists, which is the replay fork above. So a
moved number means either that fork or something unintended, and both are worth reading. Do not
update either baseline to make a red run green.

**The P7 closeout's gate changed shape**, because its premise did. It read *"a `Certificate`
type-checks with no `witness:Is*As` in any premise"* — which §3.5 withdrew, since a certificate
that type-checks without one is exactly the unsoundness. The gate is now the recomputation test:
what P7 asked is whether the index is a cache, and a verdict that any party can re-derive from the
chain is what makes it one.

## 6. Order, and what it cost

| | | |
|---|---|---|
| 1 | §1's ontology edits. No reseed here; the single reseed follows §3.5 | **done** |
| 2 | §2 the correspondence table | **done** |
| 3 | §3.2 the term former — "the largest and riskiest piece" | **done**; three exhaustive matches, and §4.3 was the real work |
| 4 | §3.3 the emit, "small once §3.2 exists" | **done**; it needed a fourth `prov` slot |
| 5 | §3.1 the fixture | **done**; it needed a Lean rebuild and turned up three defects |
| 6 | §3.5 the P7 closeout | **done**, with D87 §7's conclusion withdrawn |
| 7 | §3.4 deploy by digest, independent | **done** |
| 8 | the reseed, then §5's baselines | **outstanding** |

**The order changed once, and the reason is worth keeping.** §3.1 was scheduled second, on the
grounds that *"nothing downstream is testable before it."* That was true of the *end-to-end* tests
and false of everything else: §3.2's codec round-trip, its externalizer refusal and — the one that
matters — the refusal of a hand-authored lean4 judgement are all unit-level and need no fixture at
all. Running §3.1 after §3.3 meant the fixture could be tested against the judgement the institution
actually emits, which is what caught the constant predicate: with `Healthy = fun _ => True` the
near-miss verdict came back `Holds`, and a fixture built before the emit existed would have looked
correct.

**Step 1 of §3.5** — verifying "plausibly" for `Declared` and `Observed` — ran early in parallel
with §1 as planned, and was answered by reading `layer_admits_witness` rather than by changing it.
