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

**Working hypothesis, adopted `2026-08-22`: arm 1.** Scoped in
[p2-n1-positivity-criterion.md](p2-n1-positivity-criterion.md); the consequences are folded into the
sections below. Everything about sequencing turns on this, so it is settled first and in writing.

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

**Arm 1 — extend the criterion (ADOPTED, provisionally).** Admit higher-order positive occurrences.
No bootstrap edit, no reseed; `lexicon:Cat` stays as declared. N1 §5 finds this may split into two
independently landable steps — widen the criterion and route declarations through the pass, then add
function-typed induction hypotheses — because `derive_minor_type` and `iota_reduce_impl` filter on
the *same* predicate and would skip a higher-order argument identically, making the current
restriction a completeness limit rather than a soundness one. If that agreement is confirmed, #92
step 1 lands before #138 and the two tracks do not fully serialize.

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
| **N1 — positivity criterion + declaration routing** ✱ drafted | §1's fork; the criterion; mutual/nested posture | #92 |
| **N2 — sized types: wire or delete** | whether #139's solver gets constraint emitters or is removed | #66's costing |
| **N3 — universe polymorphism** | `Exp` representation, whether levels reach ESL surface syntax, migration for persisted terms | #188 |

**#66's answer is not a new note** — it lands in D9 or D19, as the issue's acceptance criteria say.
It cannot be written before N2: option 1 (restrict the surface to the sanctioned recursion forms)
pushes authors onto `Match` over a sized scrutinee, and with the solver unwired every size must be
written by hand — `inductive.rs:482` refuses elision outright. N2's answer is what option 1 actually
costs.

## 3. nanoda_lib is the algorithmic reference

`references/nanoda_lib` is a Lean 4 type checker in Rust, vendored and pinned. **Where an item has a
counterpart there, port the algorithm rather than deriving one.** The subtle parts of this package —
`IMax` and the `leq` decision procedure, the positivity walk, the index-aware motive — are exactly
the places where an independently derived version is both slower to write and harder to trust, and
there is a known-good implementation in the same language with its own test corpus.

**The pin moved and every citation in tree is stale.** The submodule is at `6ae1f0c`; `positivity.rs`
still cites `f58f2f6` in three places (`:24`, `:129`, `:584`), and the line numbers in #92 and #188
predate the repin. Verified positions at `6ae1f0c`:

| item | nanoda reference (at `6ae1f0c`) |
|---|---|
| #92 positivity | `inductive.rs:758` `check_positivity1`, `:867` `which_valid_ind_app`, `:1082` `is_rec_argument` (was `:666` / `:775` / `:990`) |
| #138 motive | `inductive.rs:1058` `mk_motive_dep`, `:1073` `mk_motives`, `:1161` `mk_minors1group` |
| #188 levels | `level.rs` entire, 288 lines — `IMax` `:16`, `Param` `:17`, `simplify` `:55`, `subst_levels` `:101`, `leq_core` `:176`, `leq` `:229`; `env.rs:39` `uparams`; `expr.rs:445` `Const(name, levels)` |
| #20 mutual | `st.all_inductives_incl_specialized`, threaded through `inductive.rs:88,181,183` |
| #21 nested | `st.nested_to_unspecialized_ty_nofvars` (`inductive.rs:180`) and the specialize / check / unspecialize pass around it |

**#138 is where the reference settles a live disagreement.** `mk_motive_dep` builds the motive type
as `abstr_pi_telescope(st.local_indices[i], major)` — an `(m+1)`-ary Π for a family with `m` indices
— and `mk_minors1group` folds that same motive over the constructor's applied indices before the
constructor value (`:1173-1178`). Both read one `local_indices`, so the two halves cannot disagree by
construction. EigenTT's `check_infer_inductive_rec` builds a unary motive with
`indices: Vec::new()` while `derive_minor_types` is already on nanoda's convention, so the reference
resolves #138 in favour of the minors and against the motive.

**#92's fork has a reference answer too, and it is arm 1.** nanoda implements classical strict
positivity and admits higher-order positive constructors; `positivity.rs` refuses them only because
Phase 11b's iota reduction cannot build the induction hypothesis. So arm 1 is *follow nanoda the rest
of the way* — the criterion and the eliminator together, since the induction hypothesis for a
reflexive argument is itself a Π (`inductive.rs:1177-1178`). That is a reason to prefer arm 1, not a
substitute for N1 deciding it.

**Where nanoda is not the reference, say so rather than reaching:**

- **#139 / #66 — sized types and termination.** nanoda is a *checker*: all recursion goes through
  recursors, and there is no `letrec`, no sized types, no termination checker to port. The reference
  for the sized half is MiniAgda's `Warshall.hs`, already ported into `kernel/src/nbe/sized.rs`. What
  nanoda does supply for #66 is an existence argument for option 1 — a kernel with no general
  recursion at all is a working point in the design space.
- **#69 — no-confusion.** Generated by Lean's *elaborator* (`Lean.Elab.Inductive`), not its kernel,
  so it is outside nanoda. The reference is the Lean 4 source and
  `references/type_checking_in_lean4`.
- **#194 — the check/infer sweep.** There is no arm-by-arm mapping, because nanoda has no
  bidirectional split of this shape and no applied-inductive node: `And P Q` is an ordinary `App`
  spine whose arguments `infer_app` checks against the Π binder types. The lesson is structural — the
  disagreements this sweep hunts are artefacts of EigenTT fusing type-former and arguments into one
  node — and it is already recorded in `check/mod.rs:3815`.

**Housekeeping, in the first commit that touches either file:** repoint `positivity.rs`'s three
`f58f2f6` citations and their line numbers at `6ae1f0c`, and correct the ranges quoted in #92 and
#188. Citations that drift silently are worse than none, because they read as verified.

## 4. Steps and exit gates

| # | item | design input | exit gate | manifest moves? |
|---|---|---|---|---|
| 1 | #213 | none | a comment-only edit and a JSON reformat both leave the manifest unchanged; `bootstrap_manifest_pinned.rs` and its doc comment updated to the new behaviour | no |
| 2 | #64 | none | no literal `"__case_arg"` in `check/mod.rs`; suite unchanged; a test binding that identifier still checks | no |
| 3 | #194 | none | every `check` arm compared against `check_infer` for the same `Exp` ctor, each permissive one tightened or justified in a comment; the four `Val::Sort(_)` arms at `check/mod.rs:659-673` resolved | **maybe** — see §5 |
| 4 | N1, N2, N3 | — | notes merged | no |
| 5 | #138 | none | a well-typed `InductiveRec` over an indexed family exists as a test — none does today | no |
| 6 | #92 | N1 | ESL `data` declarations reach `check_positivity`; a negative-occurrence declaration is rejected through `Validator::validate()`, which is the path #92's probe took; bootstrap still loads with its three higher-order constructors unchanged | no, under arm 1 |
| 7 | #69 | none | discrimination (`zero ≠ succ n`) and injectivity clauses generated per inductive and usable from a dependent match | no |
| 8 | #139 | N2 | either constraint emitters call `solve` from the check path, or the unreachable half is deleted and the comparison pair stands alone as the sized-types surface | no |
| 9 | #66 | N2 | decision recorded in D9/D19; if gating, ESL rejects bare `Drec` outside the sanctioned forms with tests; if documenting, an `#[ignore]`d divergence test plus a user-facing note | no |
| 10 | #188 | N3 | level algebra, `leq`, `uparams`, codec round-trip, persisted terms still decode | **yes** |

## 5. Reseed accounting

**#213 first, and the reason is this package specifically.** `current_manifest`
(`kernel/src/bootstrap/mod.rs:652`) hashes each bootstrap ontology's raw source bytes, so
reindenting or rewrapping a comment forces a full reseed — the pinned-manifest test's own message
puts the follow-through at reseed, re-point snapshot pins, re-record LLM draws that miss, repin
`EXPECTED`. Step 6 arm 2 iterates on `lexicon-ontology.esl`, which is the file in this package least
likely to be right in one pass.

Items that move the manifest: **#188** (chain-format change by construction) — and #92 only under
arm 2, which §1 has provisionally set aside, so under the working hypothesis #92 is kernel-only and
owes no reseed. **#194 is the uncertain one** — it reads as kernel-only, but #136 is the precedent
where a one-line arm change forced an ontology edit and a reseed, because `spec_poly` depended on
the leniency being removed.

**Batch the bootstrap edits or pay per item — decide before step 3, not after.** #196 is the
cautionary case in this repo: it wrote a careful one-reseed batching plan on an integration branch,
the branch was never used, and the batch paid two reseeds. The plan is worth less than the habit of
checking it. Concretely: if #194 turns out to need an ontology edit, hold it until #92's arm is
known, so the two share one reseed.

## 6. Measurement protocol — #194 and #92

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

## 7. What can run in parallel

- **The three notes** (step 4) are independent of each other and of steps 1–3. Writing all three
  before any code is the cheapest ordering, because two of them gate later steps.
- **Steps 1, 2, 3** need no design input and can proceed while the notes are being written.
- **Track B (5, 7) and Track C (8, 9)** are independent of Track A *unless* §1 goes arm 1, which
  makes #138 a prerequisite of #92.
- **#188 is last regardless.** It replaces the representation #194 audits — `Sort(m) <: Sort(n)` on
  integers becomes level `leq`, `Sort(n) : Sort(n+1)` becomes `Sort(l) : Sort(Succ(l))` — so
  auditing after it means redoing the audit against `IMax`.

## 8. Not in this package

- **#20 (mutual) and #21 (nested)** — dormant, no consumer, no trigger. They enter only as an input
  to N1.
- **Factivity of `JustifiedBy`** — #173 was closed into #159 as a design question. It is about what a
  Verified witness entitles you to assert, not about what the type theory admits.
- **#206** — restricting `ProgramTrace` to kernel-minted. A witness-provenance question; #205 added
  the class, #206 is the enforcement, and it is gated on `RunProgram` and `FIBER … INTO` minting
  traces at all.
- **`Exp::Con` in the D47 codec** — the one form #71 left unencoded, with a stated trigger. Add when
  a consumer needs it.

## 9. Package exit gate

- Every `check` arm either agrees with `check_infer` or carries a comment saying why not.
- An ESL `data` declaration with a negative occurrence is rejected through `Validator::validate()`.
- A well-typed `InductiveRec` over an indexed family exists as a test.
- `sized.rs` has no unreachable public surface — either wired or deleted.
- The bare-`Drec` decision is recorded in D9 or D19.
- `cargo test --workspace`, `cargo fmt --all -- --check`, clippy `-D warnings` clean; the WRN demo
  and both parse baselines green on whatever snapshot the package's last reseed produced.
