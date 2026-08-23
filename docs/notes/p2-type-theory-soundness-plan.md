# P2 — Type-theory soundness: work plan

Tracker: [#215](https://github.com/eigenius/eigenius/issues/215). Written `2026-08-22` on
`eigentt-improvements`, from `6744c9a`.

The package is *what EigenTT wrongly admits, and what it cannot yet express*. Every state claim below
was verified by reading the code at `6744c9a`, not taken from the issue text — three issues were
already done at filing-time-plus-fixes and have been closed (#191, #71, #22).

## 0. Membership

| | issue | track |
|---|---|---|
| prerequisite | #213 — manifest hashes raw bytes | before any bootstrap edit |
| A | #194 — check mode more permissive than inference | the gate fails open |
| A | #92 — declarations never reach the positivity pass | the gate fails open |
| B | #138 — recursor motive is not index-aware | indexed families |
| B | #69 — no-confusion generation | indexed families |
| B | #139 — size-constraint solver has no caller | indexed families |
| C | #66 — bare `Decl::Drec` escape hatch | totality |
| C | #64 — literal `"__case_arg"` | hygiene |
| D | #188 — universe polymorphism | the universe ladder |
| dormant | #20 mutual, #21 nested | inputs to §1's decision only |

Closed while scoping: #191 (fixed in #193), #71 (done but `Exp::Con`), #22 (delivered by D48;
residue = #138 / #69 / #139).

## 1. The decision that shapes the package: #92's fork

Everything about sequencing turns on this, so it is settled first and in writing.

**The pass exists.** `kernel/src/nbe/positivity.rs` implements strict positivity following nanoda's
`check_positivity1`, and `check_type` calls it at `check/mod.rs:335` on the `Exp::Inductive` arm.
What does not happen is the ESL declaration path reaching that arm — `Exp::Inductive(` appears
nowhere in `kernel/src/esl/compile.rs`. That is why #92's probe returned 0 errors through
`Validator::validate()`. The work is routing plus a rejection test, not writing a checker.

**Routing it unchanged rejects the bootstrap.** `positivity.rs` deliberately refuses higher-order
positive constructors — `Foo { mk : (Nat → Foo) → Foo }` — because Phase 11b's iota reduction cannot
build the corresponding induction hypothesis; `rejects_higher_order_positive` (`positivity.rs:507`)
pins it. Three constructors of `lexicon:Cat` are that shape, in a bootstrap ontology:

```
ontologies/lexicon/lexicon-ontology.esl:271   cat_forall     : lexicon:Num -> (Set -> lexicon:Cat) -> lexicon:Cat
ontologies/lexicon/lexicon-ontology.esl:280   cat_fin_forall : (lexicon:Fin -> lexicon:Cat) -> lexicon:Cat
ontologies/lexicon/lexicon-ontology.esl:281   cat_num_forall : (lexicon:Num -> lexicon:Cat) -> lexicon:Cat
```

#92 asks for the opposite of what the pass does — *"reflexive/infinitary constructors of the form
`(A -> Self) -> Self` are sound and should be retained"* — and the probe that prompted it was
`cat_forall`'s shape, accepted only because the pass never ran.

**Arm 1 — extend the criterion.** Admit higher-order positive occurrences, which requires the
recursor to build induction hypotheses for them. That is the same machinery gap #138 names for
indices, so this arm makes **#138 a prerequisite of #92**. No bootstrap edit, no reseed.

**Arm 2 — keep the criterion, re-represent categories.** Rewrite the polymorphic-category algebra
(D63 §8.2) without higher-order constructors. Touches the parser's category representation, is a
bootstrap edit, and costs a reseed. Track A and Track B stay independent.

**The note must also answer** whether it designs for #20 (mutual) and #21 (nested). Both change what
the criterion has to be — mutual requires tracking every type in a block simultaneously; nested is
neither positive nor negative and needs Lean's specialize / check / unspecialize before positivity
applies at all. They are dormant, but *designing as if they do not exist* is a choice to make on
purpose rather than by omission.

**Exit gate for the decision:** a design note recording the arm, the criterion in force, and the
answer on #20/#21. No code before it.

## 2. Design notes required

Three, independent of each other and of every code item. All three can be written before any code.

| note | settles | blocks |
|---|---|---|
| **N1 — positivity criterion + declaration routing** | §1's fork; the criterion; mutual/nested posture | #92 |
| **N2 — sized types: wire or delete** | whether #139's solver gets constraint emitters or is removed | #66's costing |
| **N3 — universe polymorphism** | `Exp` representation, whether levels reach ESL surface syntax, migration for persisted terms | #188 |

**#66's answer is not a new note** — it lands in D9 or D19, as the issue's acceptance criteria say.
It cannot be written before N2: option 1 (restrict the surface to the sanctioned recursion forms)
pushes authors onto `Match` over a sized scrutinee, and with the solver unwired every size must be
written by hand — `inductive.rs:482` refuses elision outright. N2's answer is what option 1 actually
costs.

## 3. Steps and exit gates

| # | item | design input | exit gate | manifest moves? |
|---|---|---|---|---|
| 1 | #213 | none | a comment-only edit and a JSON reformat both leave the manifest unchanged; `bootstrap_manifest_pinned.rs` and its doc comment updated to the new behaviour | no |
| 2 | #64 | none | no literal `"__case_arg"` in `check/mod.rs`; suite unchanged; a test binding that identifier still checks | no |
| 3 | #194 | none | every `check` arm compared against `check_infer` for the same `Exp` ctor, each permissive one tightened or justified in a comment; the four `Val::Sort(_)` arms at `check/mod.rs:659-673` resolved | **maybe** — see §4 |
| 4 | N1, N2, N3 | — | notes merged | no |
| 5 | #138 | none | a well-typed `InductiveRec` over an indexed family exists as a test — none does today | no |
| 6 | #92 | N1 | ESL `data` declarations reach `check_positivity`; a negative-occurrence declaration is rejected through `Validator::validate()`, which is the path #92's probe took; bootstrap still loads | **arm 2 only** |
| 7 | #69 | none | discrimination (`zero ≠ succ n`) and injectivity clauses generated per inductive and usable from a dependent match | no |
| 8 | #139 | N2 | either constraint emitters call `solve` from the check path, or the unreachable half is deleted and the comparison pair stands alone as the sized-types surface | no |
| 9 | #66 | N2 | decision recorded in D9/D19; if gating, ESL rejects bare `Drec` outside the sanctioned forms with tests; if documenting, an `#[ignore]`d divergence test plus a user-facing note | no |
| 10 | #188 | N3 | level algebra, `leq`, `uparams`, codec round-trip, persisted terms still decode | **yes** |

## 4. Reseed accounting

**#213 first, and the reason is this package specifically.** `current_manifest`
(`kernel/src/bootstrap/mod.rs:652`) hashes each bootstrap ontology's raw source bytes, so
reindenting or rewrapping a comment forces a full reseed — the pinned-manifest test's own message
puts the follow-through at reseed, re-point snapshot pins, re-record LLM draws that miss, repin
`EXPECTED`. Step 6 arm 2 iterates on `lexicon-ontology.esl`, which is the file in this package least
likely to be right in one pass.

Items that move the manifest: **#92 under arm 2** (lexicon), **#188** (chain-format change by
construction). **#194 is the uncertain one** — it reads as kernel-only, but #136 is the precedent
where a one-line arm change forced an ontology edit and a reseed, because `spec_poly` depended on
the leniency being removed.

**Batch the bootstrap edits or pay per item — decide before step 3, not after.** #196 is the
cautionary case in this repo: it wrote a careful one-reseed batching plan on an integration branch,
the branch was never used, and the batch paid two reseeds. The plan is worth less than the habit of
checking it. Concretely: if #194 turns out to need an ontology edit, hold it until #92's arm is
known, so the two share one reseed.

## 5. Measurement protocol — #194 and #92

Both tighten a gate that runs on the commit path, so both measure before tightening: instrument to
**log without rejecting**, run the suites, count. Precedent:

- #137 — 356 index comparisons, 0 mismatches.
- #191 — 204,703 class-against-universe checks, all at `Sort(1)`.

#92 needs the same plus one more: the bootstrap must be shown to still load, since `lexicon:Cat` is
what is at risk. Wire the pass in log-only mode first and read what it would have rejected.

The standing finding for #194, so the sweep does not re-derive it: the four arms at
`check/mod.rs:659-673` match `Val::Sort(_)` and delegate to `check_type`, and **`check_type(ctx,
exp)` takes no expected type** (`check/mod.rs:258`). The expected universe is discarded before the
delegation, so those arms cannot be doing the comparison — they are `Ok(())` with extra steps.
Against `check_infer` (`:1282-1297`) that gives two live disagreements: `codata { … } : Prop` passes
while inference says `Sort(1)`, and an inductive declared at `Set` passes against `Sort(0)`.

## 6. What can run in parallel

- **The three notes** (step 4) are independent of each other and of steps 1–3. Writing all three
  before any code is the cheapest ordering, because two of them gate later steps.
- **Steps 1, 2, 3** need no design input and can proceed while the notes are being written.
- **Track B (5, 7) and Track C (8, 9)** are independent of Track A *unless* §1 goes arm 1, which
  makes #138 a prerequisite of #92.
- **#188 is last regardless.** It replaces the representation #194 audits — `Sort(m) <: Sort(n)` on
  integers becomes level `leq`, `Sort(n) : Sort(n+1)` becomes `Sort(l) : Sort(Succ(l))` — so
  auditing after it means redoing the audit against `IMax`.

## 7. Not in this package

- **#20 (mutual) and #21 (nested)** — dormant, no consumer, no trigger. They enter only as an input
  to N1.
- **Factivity of `JustifiedBy`** — #173 was closed into #159 as a design question. It is about what a
  Verified witness entitles you to assert, not about what the type theory admits.
- **#206** — restricting `ProgramTrace` to kernel-minted. A witness-provenance question; #205 added
  the class, #206 is the enforcement, and it is gated on `RunProgram` and `FIBER … INTO` minting
  traces at all.
- **`Exp::Con` in the D47 codec** — the one form #71 left unencoded, with a stated trigger. Add when
  a consumer needs it.

## 8. Package exit gate

- Every `check` arm either agrees with `check_infer` or carries a comment saying why not.
- An ESL `data` declaration with a negative occurrence is rejected through `Validator::validate()`.
- A well-typed `InductiveRec` over an indexed family exists as a test.
- `sized.rs` has no unreachable public surface — either wired or deleted.
- The bare-`Drec` decision is recorded in D9 or D19.
- `cargo test --workspace`, `cargo fmt --all -- --check`, clippy `-D warnings` clean; the WRN demo
  and both parse baselines green on whatever snapshot the package's last reseed produced.
