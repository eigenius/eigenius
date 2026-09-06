# Work stack — unfinished work (top = active)

The single "where are we" pointer. A **LIFO stack** of the active working notes: work the **top** entry;
when its exit-gate is met, **pop** it and the entry below becomes active. When a sub-task splits off from
an entry, **push** its note on top. Keep this file current — it is the map back to the base plan after
any detour.

---

## Stack (top → bottom)

> **ACTIVE: entry 0 (`2026-08-28`).** *Judgements, Warrants, and Logics*
> (`docs/design/judgements-and-warrants.tex`) is the design; **P0 of
> `docs/notes/judgements-warrants-build-plan.md` is the next task** — measurement only, no code.
> The paper supersedes the D83 markdown draft (removed) and two of D82's conclusions: the
> institution criterion and the constructive/classical conjecture. D82 remains the derivation record.
>
> This subsumes what entry 1 below scheduled as D80 (witness and institution machinery) and reaches
> further: the build plan's P3 and P7 cover D80's W-phases, and D77's merge work is downstream of
> both. Do not start D80 as scheduled below.
>
> **Note on a superseded conclusion:** D82 §5b.7 argued the reasoning institution is not an
> institution and should dissolve. The paper rejects the criterion that rested on — institutions and
> proof systems are not exclusive, and the kernel is a *degenerate* institution. P7 relocates
> vocabulary the kernel owns; it does not dissolve anything.

> **ACTIVE: entry 1 (`2026-08-24`, re-scoped `2026-08-25`), pushed on top of P2.** D75 diagnosed the
> two seams as one problem, so the fusion work outranks the individual P2 issues it subsumes.
> **D76 and D78 are complete.** D77 was drafted as the third follow-on and turned out to carry three
> separable projects; it was split on `2026-08-25` into a dependency chain, and the build order is the
> reverse of the drafting order:
>
> 1. **D79 — the representation of inductive types. ✅ COMPLETE `2026-08-26`.** All seven phases
>    landed: P1 seal, P2 declarations (23 properties), P3 `core:mentions` indexer arm, P4 vestigial
>    ctor `@id`, P5 (already done by D76 Phase F — #228 closeable), P6 qualified ctor syntax (#24),
>    P7 chain-declared `core:List` with both kernel special cases deleted. Reseed clean at
>    **9,439,633 / 0 errors**; index growth measured **+128 MB (+4.7%)**; parse gate all-green;
>    demo passes. See D79 §7 for what each phase actually did versus what was planned — three of
>    seven differed.
> 2. **D80 — witness and institution machinery.** Two facts earned under a binding that survive it
>    changing: witness credit (D75 §3.4, standing test at `witness_admission.rs:1184`) and institution
>    verdicts whose bound data was rebound. Both fire on a *linear* commit. W0 (what revocation
>    means) → W1 binding-aware lookup → W2 AutoOnLoad baseline → W3 provenance closure.
> 3. **D77 — merge as a pushout.** #225. Last because it needs recheckers to call, and for witnesses
>    and verdicts "recheck" was unsettled until D80 — both answers turned out to differ from the
>    resource one and from each other. §3.6's rename defect was fixed early (D79 P2 unblocked it),
>    so F1 no longer needs to size it.
>
> **Detour taken `2026-08-26`, on top of the above:** D80's W-phases were not started. Instead the
> stack was analysed first (`docs/notes/d81-epistemic-stack-analysis-plan.md` → **D81**, a
> description of the epistemic machinery as implemented), and the user then reframed the whole area:
> the system was anchored on *resources* but is about *propositions and how they came to be
> warranted*. That produced **D82** (design, no code) with a seven-step sequence S0–S6. **D80 and
> D77 are now downstream of D82's S1** — the witnessed relation in `WitnessKey` is the fix for the
> environment-blindness D80 W1 was going to address, so W1 should not be built before S1 lands.
>
> **Loose ends from D79, none blocking:** the parse baseline records readings 613 / skeletons 170
> and a live run now measures 688 / 180 — both within ceiling, gate green, cause unconfirmed (P7's
> `core:List` decode is the hypothesis). Updating `baseline.json` needs a recorded **replay** draw
> and an explanation, per its own protocol. Two selection decisions are unadjudicated in
> `reading-adjudications.tsv`. #228 and #188 are confirmed addressed and can be closed.
>
> Entry 2 stays the tracker for what fusion does *not* cover.
>
> **Superseded note (entry 2, `2026-08-22`), kept for the reasoning:** The parser-pipeline spine emptied
> on `2026-08-20` when D71 met its gate; entries (2) and (3) below are the pre-D71 spine, assessed on
> `2026-08-19` as largely implemented-or-obsolete, so neither was promoted by position. P2 was picked
> up instead. The two candidates that were live and stay unstarted:
>
> - **D71 residue** — §14's four open questions (source transport, draw commit granularity, pruning
>   policy, the prefix-replay measurement) and §11's human-override loop, which the §9 draws-on-branch
>   decision shrank from a design problem to a measurement.
> - **D61 faithfulness** — the half that D71 §10 reserved the institution shape for, and the only
>   thing in the tree that still earns it.

### 1. [d75-fusing-eigentt-and-the-knowledge-graph.md](../design/d75-fusing-eigentt-and-the-knowledge-graph.md) — **fusing the type theory with the graph**
The diagnosis: two seams, built twice — Seam A (*the layer chain is `Γ_env`*) and Seam B (*a resource
is a record*) — with nine symptoms and ten questions, all answered. Two implementation documents hang
off it.

#### STATUS `2026-08-25`
- **[D78](../design/d78-resources-as-records.md) — Seam B: complete.** All five phases. Closes D75
  §3.7 (the Σ-chain), §3.8 (undeclared properties projectable) and §6.0's three-implementations
  duplication, plus two latent defects: local-name projection collisions, and `classes.first()`
  discarding 73 % of class claims. Two deferrals remain in its §9. Its §3.1 entailment arm is parked
  on D76 Phase D.
- **[D76](../design/d76-the-typing-environment.md) — Seam A: Phases A, C, B complete; D, E2, F open.**
  Order is `A ▸ C ▸ B ▸ D ▸ E2 ▸ F`.

  **Phase B is done and its gate is met:** no `Exp` variant carries an `Arc<InductiveDecl>`, and the
  self-reference stub is gone. Five commits (`9d1f33b`, `a7b1a38`, `da0d747`, `2e7afca`, `f007b87`,
  `6dee735`, `406f0b2`). Six defects surfaced along the way, none findable by the compiler — the
  felicity and parse suites found them all. The pattern, worth carrying into D: **inlining hid every
  place a name could not be resolved**, because the answer travelled inside the term, and a missing
  answer degrades to a *neutral* rather than an error.

  **B2 is deferred by design** — turning on the arity check the stub was suppressing is
  verdict-affecting over the whole chain, so it runs the #194/#92 protocol: instrument, count, then
  enforce. Its natural shape is nanoda's: give a type former the Π-telescope type
  `Π(params)(indices). Sort l` and let the ordinary application rule check the arguments, deleting
  `check_inductive_type_args`.

- **Next:** the reseed (running `2026-08-25`, from `406f0b2`) validates four commits of kernel change
  against the real 9.4M-resource chain and is the only place §4.2's `Global`-memo boundedness gets
  measured. Then Phase D — δ in conversion, 54 call sites — which is what D78 §3.1 is waiting on.

### 2. [p2-type-theory-soundness-plan.md](p2-type-theory-soundness-plan.md) — **P2 · type-theory soundness**
Tracker [#215](https://github.com/eigenius/eigenius/issues/215). What EigenTT wrongly admits, and what
it cannot yet express: nine issues in four tracks, plus #213 as a prerequisite.

#### STATUS
Scoping done `2026-08-22`; no code yet. Three issues closed during scoping because they were already
fixed — #191 (in #193), #71 (done but the deliberately-deferred `Exp::Con`), #22 (delivered by D48,
residue = #138 / #69 / #139). Branch `eigentt-improvements` from `6744c9a`.

**The gating decision is #92's fork** (plan §1) and it is not a code question. The positivity pass
already exists (`kernel/src/nbe/positivity.rs`, called from `check/mod.rs:335`); the ESL declaration
path never reaches it. Routing it in unchanged **rejects the bootstrap** — `cat_forall`,
`cat_fin_forall`, `cat_num_forall` (`ontologies/lexicon/lexicon-ontology.esl:271,280,281`) are
higher-order positive, the shape `positivity.rs:507` pins as rejected. **Working hypothesis `2026-08-22`: arm 1**, extend the criterion — scoped in
[p2-n1-positivity-criterion.md](p2-n1-positivity-criterion.md). Kernel-only, no reseed, `lexicon:Cat`
unchanged. N1 §5 splits it in two, **verified `2026-08-22`**: the current restriction is a *completeness* limit,
not a soundness one — `derive_minor_type` and `iota_reduce_impl` filter on the same predicate and skip
a higher-order argument identically (pinned by
`higher_order_positive_arg_is_skipped_by_both_minor_derivation_and_iota`, `nbe/eval/iota.rs`). So
**#92 step 1 — widen the criterion, route ESL declarations through the pass — lands BEFORE #138**;
step 2 (function-typed IHs) after it.

**#213 is no longer this package's prerequisite.** Under arm 1 nothing here edits a bootstrap
ontology except #188, and fixing #213 itself moves all 21 manifest lines at once (the stored value is
`current_manifest()`'s output at seed time), so it owes one reseed to adopt. Fold it into #188's
reseed rather than paying a standalone one.

#### DONE
- **#92 step 1 (`2026-08-22`).** Criterion widened to classical strict positivity; `RecArgShape`
  (`nbe/positivity.rs`) is now the single definition of "recursive occurrence", consumed by
  positivity, `recursor::derive_minor_type` and `eval::iota_reduce_impl`;
  `InductiveDecl::is_direct_recursive_ref` removed. **Rule 23** (`validation/rules/positivity.rs`)
  routes `core:InductiveType` declarations through `check_positivity` at commit — the edge that was
  missing, since `check_positivity` only ever ran on the TERM form and ESL `data` never produces
  one. Measured before enforcing: 42 declarations on the bootstrap, 42 admitted, 0 decode failures,
  exactly the three predicted higher-order ctors. Revert-checked: without the rule,
  `Validator::validate()` returns `[]` for `(Bad -> boolean) -> Bad`, reproducing #92's symptom.
  Manifest unmoved, no reseed.

- **#64 (`2026-08-22`).** Case-branch binder renamed `__case_arg` → `CB#{level}`, using the
  checker's existing `#` discipline (`TC#`/`G#`) rather than the issue's `__case_arg_{level}`, which
  is still a legal ESL identifier and therefore still forgeable. Two findings recorded on the tests:
  the collision the issue describes is **not reachable today** (readback is normalizing and mints
  only `Gen(j, name)` → `"{name}{j}"`, so no free source name reaches the spliced motive), and the
  arm had no test coverage at all. Closed.

- **#194 (`2026-08-22`).** The sweep found **six** `(Exp::X(..), Val::Sort(_))` arms, not the two the
  issue named — `SizeSort` was a third instance, with code, comment and inference giving three
  different answers. Five are **deleted**, not tightened: each was `check_type(ctx, exp)`, which is
  `check_infer`'s arm minus the universe comparison, so they now fall through to
  `check_by_inference` and check/infer agree by construction. `SizedPi` stays permissive
  deliberately — no `check_infer` rule exists, and the probe logged it unconditionally with **zero**
  workspace hits, so there is no evidence to pick its sort from; reasoning recorded on the arm.
  Measured 12 probe hits, all `inferred <= expected`. Closed.

- **#138 (`2026-08-22`).** Motive is index-aware: `Π (i₁:I₁) … (i_m:I_m). D(params)(i₁…i_m) → Sort`,
  result at the major's own indices. nanoda settled the direction (`mk_motive_dep` /
  `mk_minors1group` both read one `local_indices`), so the motive moved to meet the minors.
  **Watch the name hygiene**: building the motive type as an `Exp` put the burden on names, and the
  first version let an index binder capture a same-named parameter — domain `D(idx, idx)` instead of
  `D(param, idx)`, returning `Ok` because a constant motive checks against either. Fixed by reading
  parameters back from values and naming index binders `IDX#{k}`. Closed; **#69 unblocked**.

- **#69 closed `2026-08-22` as won't implement.** #138 unblocked its eliminator; assessing it then
  showed three unbuilt kernel features behind it and no consumer — `IdJ` carries no motive
  (`check/mod.rs:1105` ignores `_c`; deferred to "Phase 10b" higher-order unification), no
  heterogeneous equality exists for the indexed case its trigger names, and generated lemmas have
  nowhere to live. Also **nanoda does not have `noConfusion`** (grep-verified): it generates
  recursors and `Quot`, both kernel constructions, while `noConfusion` is a frontend definition —
  the issue's pointer at nanoda as "the inductive elaboration this would extend" was wrong.
  **Raise `IdJ`'s motive first if revisited.**

- **#92 step 2 (`2026-08-22`) — CLOSED.** Both halves of the eliminator now build the hypothesis for
  a higher-order positive argument: `Π b₁:B₁ … B_k. motive idx… (arg b₁ … b_k)` in
  `derive_minor_type`, `λ b₁ … b_k. rec … (arg b₁ … b_k)` in `iota_reduce_impl`. Induction through a
  reflexive argument computes, pinned by `iota_recurses_through_a_higher_order_argument`. IH binder
  renamed `__ih_N` → `IH#N` (a ctor argument of that name captured it — third capture defect of the
  session). Track A and Track B are done.

- **N2 written `2026-08-22`** — [p2-n2-sized-types-wire-or-delete.md](p2-n2-sized-types-wire-or-delete.md).
  Recommends **deleting** the ~390-line Warshall solver, keeping the comparison pair. Three findings:
  wiring is not "add a caller" (there is no flexible size in the term language to solve for —
  `FlexId` appears nowhere outside `sized.rs`); **nothing uses sized types at all** (`core:Size` is a
  compiler built-in, declared in no ontology, and no chain term carries a size form); and
  `references/miniagda` **does not exist**, so the port's faithfulness cannot be checked.
  **Correction to the earlier reading of #66**: its option 1 is not expensive because the sized path
  is onerous — *none* of its three sanctioned recursion forms has a chain user, and neither does the
  bare-`letrec` hazard. No recursion of any kind is exercised by any chain in this repo, which makes
  #66 cheap now and expensive later.
  **Corrected `2026-08-22` after review**: sized types exist for **codata productivity** first
  (D19:491 — "required for complete termination story when combining inductive recursion with codata
  corecursion"), not inductive termination. The productivity site — `check/mod.rs`'s
  `Lam`-vs-`SizedPi` arm, which registers `j < upper` in the TSO — is a wired user the first draft
  missed, because it feeds the comparison pair rather than calling it. Recommendation unchanged:
  productivity works from sizes that are WRITTEN; `solve` infers sizes that are unknown, and there
  are none.

- **#139 CLOSED `2026-08-22`** (`e1a404a`). Solver deleted, `sized.rs` 1047 → 410 lines; comparison
  pair and its 32 tests untouched. The deletion exposed two stale docs, both fixed: `sized_rigid.rs`
  described MiniAgda's two constraint systems with a live intra-doc link to the removed one, and the
  user-facing primer §7.6 advertised "the dual-solver pattern" in two places. Both also carried
  dangling `references/miniagda/…` paths — MiniAgda is not vendored at all.
- **#66 CLOSED `2026-08-22` as won't implement.** Option 1 is vacuous: **ESL has no `letrec`** —
  `grep letrec|Drec` over `parser.rs`, `compile.rs`, `ast.rs` returns nothing — and **nothing in
  production constructs a `Decl::Drec`** (the only two sites are in `check/codata.rs`'s test module).
  The divergent program is reachable only from Rust, so there is no surface to gate. The kernel's
  acceptance of a non-terminating `Drec` is a fact about the term language for D9/D19, not an open
  defect. **Successor question, unfiled:** does `Decl::Drec` earn its place at all? Its stated
  justification is recursor derivation, which does not use it — same shape as #139. Needs a look at
  `eval/mod.rs:219` and the codata corecursion tests first.

- **N3 written `2026-08-22`** — [p2-n3-universe-polymorphism.md](p2-n3-universe-polymorphism.md).
  ~~Recommended holding #188~~ — **SUPERSEDED `2026-08-22`: building it**, on the Cooper/TTR reasoning (§5a). §§2-4 and §7 are the design; build log in §8. Its own trigger — "a second level bump is proposed" — is measured and
  has not fired: `Prop` 712 uses, `Set` 230, **`Type 1` exactly 2** (spec_poly's binder and
  `data lexicon:Cat`), **`Type 2` zero** (the one textual hit is a comment describing the ladder).
  The three design questions are settled anyway so they are not re-derived: representation mirrors
  **`lean:LeanLevel`**, which already carries nanoda's five ctors (Zero/Succ/Max/IMax/Param) on the
  chain — a precedent nobody had noticed, and the reason a future Lean externalization becomes a fold
  rather than a special case; **no ESL syntax** initially, level inference via `uparams` instead;
  migration is small (**zero encoded `Sort` terms in any repo chain**) but needs a snapshot check,
  and the decoder should accept the legacy integer. **Watch conditions**, neither in #188: the **TTR work** (Cooper, *From Perception to Communication* —
  already a cited anchor in D18/D61/D62; **TTR uses universe polymorphism**, and stratified predicates
  over record types are exactly what a fixed rung cannot express) is the likeliest trigger; #159/D74
  may also make #188 its prerequisite. Whoever starts the TTR work should read N3 §2 first — the
  level algebra to mirror is already on the chain as `lean:LeanLevel`, and choosing a different one
  there would be expensive to unpick.

- **N4 written `2026-08-23`** — [p2-n4-eigentt-representation-layer.md](p2-n4-eigentt-representation-layer.md),
  *should the EigenTT term representation move to core?* **Yes**, and the bootstrap-cycle objection
  does not hold: the D47 decoder dispatches on hard-coded ctor names, so nothing needs a `TypeExpr`
  value to decode one. Root cause of a recurring symptom — `core` owns the inductive metamodel but
  not the term language, so every type-valued slot there degrades to a string (`result_sort`, fixed
  by #188; `param_kind`, silently types a class-typed parameter `Set`; `type_name`, correct only
  because it has the `EigonClass` arm `param_kind` lacks). `eigentt-type-fragment` bundles two
  strata — the term language, and `Axiom`/`Definition` which consume it — so moving `TypeExpr` down
  separates rather than breaks. **No open questions**: Rule 16's recursion is structural on the VALUE tree, so
  reading `TypeExpr`'s `ctors` while checking a value inside them is a bounded read, not a loop.
  §4a records what the retype closes — **Rule 16 fails open on parameter-typed arguments**,
  returning `Ok` for any `type_name` that is not an IRI, i.e. every parameter-typed ctor arg of
  `logic:And` / `logic:Or` / `core:Option`. Latent (no such values on a chain) but one prose
  encoding away: closed-class gives *"but"* the semantics `logic:And(s₁, s₂)`. **Folded into P2 on this branch** (`2026-08-23`), own gate (§7), rides #188's reseed. **89 hand-authored values** across four JSON ontologies migrate by one-shot script, each guarded by an old-decode/new-decode equivalence check — decompile-then-recompile cannot do it because `eigenius decompile` flattens `data` to `resource` (**#217**, filed, not a prerequisite). The retype cannot be scoped: `core:type_name` is one property, so the Lean mirror's 36 values are in scope and **four** layers move.
- **`param_kind`'s missing `EigonClass` arm is a live bug**, independent of all the above and of any
  ontology edit: a class-typed inductive parameter is silently typed `Set`, which accepts anything.

- **N4 LANDED `2026-08-23`.** `eigentt:Term` moved into `core-ontology.json` (IRI unchanged),
  `SizeSort` ctor added, `param_kind` and `type_name` retyped, 85 values migrated by script with the
  equivalence guard, manifest re-pinned on **five** layers (the four predicted, plus `reasoning`),
  full gate green (185 test binaries, clippy, fmt). **The reseed is the only thing left.**

  N4 estimated **six** code sites; there were **fifteen**, and the nine extra were all silent
  readers — code that read the property as a `Value::String`, got `None`, and carried on. Three
  head-readers had been written by the time the suite was green; they are now one
  `program::ground::arg_type_head`. The Julia mirror generator's two readers had **passing tests
  throughout**, because their fixtures build the property by hand and kept writing strings.

  Four defects the retype surfaced, none of them caused by it:
  - **Every index kind on every chain decoded to `EigonClass(core:Set)`** — `decode_indices` had
    `_ => "urn:eigenius:core:Set"` as its fallback, and that IRI is not a declared resource.
  - **`check_type`'s fallback was `check(a, &Val::sort(1))`** — "is a type" spelled as "inhabits
    `Set`", so `T : Type 1` was not a type. Now `ensure_sort(infer(a))` (nanoda `tc.rs:244`). Same
    defect as the `Level` `Ord` derive: a universe comparison written as a constant.
  - **`check_type`'s `Exp::Inductive` arm never type-checked the telescope** — nanoda gets it free
    because a declaration is one Π-chain `Expr`. Now `check_inductive_decl_telescopes`, and Rule 23
    routes the declaration through `check_type` rather than calling `check_positivity` directly, so
    it is one gate and not a re-listing (renamed `rules/inductive_decl.rs`,
    `InductiveDeclInadmissible`).
  - **nanoda's ctor-argument universe constraint** (`inductive.rs:904`) is ported as well, after
    the #194/#92 probe protocol: logged first, and the whole workspace produced **one** violating
    declaration. `reasoning:JustifiedBy` was at `Type 0` while `spec_poly` binds `T : Type 1`
    (`Sort 2`, so the argument sits at `Sort 3`) — and `spec_poly`'s own comment had predicted the
    fix ("a rule quantifying over `Type 1` domains would need `Type 2`") while the declaration
    contradicted it. One-token edit to `Type 2`; **`reasoning` becomes a fifth moved layer.** Still
    unported: nanoda's parameter-prefix `assert_def_eq` (`:892`), which only bites on the
    independently-authored `core:ctor_type` path.
  - **The three telescope producers had drifted** — the codata-param site used `var_value` where the
    other two used `bare_kind_value`. One `Compiler::lower_kind` now serves all three.

  `core:Set` and `core:Size` were written in ESL fixtures as if they were resources. Neither is
  declared on any chain; `wk::SET_KIND` / `wk::SIZE_KIND` were unreferenced residue and are deleted.

#### FAST-FOLLOW (after the reseed, alongside #217)
Filed `2026-08-23`. All four are residue of this package, and the accounting is worth stating
plainly: **this repo has no third-party code**, so "pre-existing defect" means "written here in an
earlier session". #220 is sharper than that — the `Sort(1)` fallback survived eigenius#188's OWN
`usize`→`Level` migration, two days before it was found.

- **[#217](https://github.com/eigenius/eigenius/issues/217)** — `eigenius decompile` flattens `data`
  to `resource`, so inductives do not round-trip. Wants a document-level ESL round-trip test first:
  the existing suite is entirely term-level, which is why this was invisible.
- **[#218](https://github.com/eigenius/eigenius/issues/218)** — sized types are half-migrated:
  `SizedPi`/`SizeInf`/`SizeSucc` absent from the D47 codec, `core:binder_kind` still a string that
  matches the non-resource `urn:eigenius:core:Size`, `Size` an identifier where `Set`/`Prop`/`Type`
  are keywords. `needs:decision` — N2 §3's "nothing declares a sized type" makes *deleting* the
  half as coherent an ending as closing it, and nobody has called it.
- **[#219](https://github.com/eigenius/eigenius/issues/219)** — two unported declaration-admission
  checks: nanoda's parameter-prefix `assert_def_eq` (`inductive.rs:892`, bites only on the
  independently-authored `core:ctor_type` path) and `no_dupes_all_params` (`tc.rs:167`) — the
  latter introduced by slice 5c, which added `universe` declarations without it. A lint, not a
  soundness hole; the ticket says so rather than overclaiming.
- **[#220](https://github.com/eigenius/eigenius/issues/220)** — hardcoded universe constants
  survived the `usize`→`Level` migration. Carries the full `is_nat(<literal>)` sweep so it is not
  redone: `is_nat(0)` is legitimate throughout, `is_nat(1)` is test assertions plus one redundant
  arm plus **one real defect** — `Exp::Data` is checkable only against `Set`, and unlike the `One`
  arm it has no `check_infer` fallback to rescue it.

#### RESEED — DONE `2026-08-23`
Timings in [reseed-timings-2026-08-23.md](reseed-timings-2026-08-23.md).

| step | result |
|---|---|
| `reseed-lexicon-db.sh --umls-all` | **34 m 40 s**, 9,439,633 resources, 35 loads, 0 errors → `wordnet-umls-2026-08-23` |
| `build-alignment-snapshot.sh` | 40,357 entries redefined from 38,389 merges, 0 errors → `wordnet-umls-aligned-2026-08-23` |
| `measure-parse-rate.sh` (live reranker) | coverage **PASS**, expected-hits **62/62**, invalid-selected **0**, skeletons 170→**168**, readings 613→617 |
| `demo/wrn-helicase/run.sh` | **56 Holds, 0 Fails, 0 errors**, all 6 steps incl. every wrapped-R warrant |

**The WRN demo is where the P2 gate was actually exercised outside the bootstrap.** The lexicon
chains contain zero `data` declarations, so Rule 23 returned at its first line for all 9.4 M
resources. The WRN corpus declares **46** inductives, and they are the shape that matters:
`data onco:TopDifferentialDependency : core:string -> core:string -> Prop` — an INDEX telescope of
`core:string`, which is precisely where `decode_indices`' `_ => "urn:eigenius:core:Set"` fallback
lived. Before the fix those indices decoded to `EigonClass(core:Set)`, a class nothing can inhabit.
All 46 admitted. All end in `Prop`, so the constructor-argument universe constraint takes its
impredicative exemption on every one — exercised as *not firing*, which is correct behaviour.

**Parse gate: one red, accepted as noise (user decision `2026-08-23`).** `reading_correct`
29/40 vs the tracked 30. Traced to two units flipping — one wrong→correct, one correct→wrong — and
**both had byte-identical candidate sets** (8 and 24 candidates, unchanged). The regressed unit
differs in a single WordNet sense of *impairment* (`n00403334` the act → `n14561618` the state);
`cancer`, `exhibit` and `DNA Repair Pathway` are identical. `structure_correct_diagnostic` is 33/40,
exactly the baseline. Two flips in 40 is the ~5 % the baseline records for temperature-0 live draws.
**baseline.json was NOT updated** — it says update deliberately, never to make a red run go green,
and a single live draw cannot distinguish noise from regression either way. Settling it properly
means 2–3 more live draws to establish the band on this snapshot.

#### NEXT
**Fast-follow: #217–#220** (above). Then P2 closes out: every code item is done, N4 has landed, and the reseed above cleared it —
including **#213**, which rode the same reseed as planned.

**Port from `references/nanoda_lib` wherever there is a counterpart** (plan §3) — positivity,
the index-aware motive, the whole level algebra. It also answers #138 outright: nanoda's motive and
minor premises both read one `local_indices`, so they cannot disagree, and the fix is to move the
motive onto `derive_minor_types`' convention. **The pin moved to `6ae1f0c` and every citation is
stale** — `positivity.rs` still cites `f58f2f6` at `:24`, `:129`, `:584`, and the line numbers in #92
and #188 predate the repin.

#### GOTCHAS
- **#213 first, before any bootstrap edit.** Iterating on `lexicon-ontology.esl` while every
  whitespace change costs a reseed is exactly what it removes.
- **Measure before tightening** (#194, #92): instrument to log without rejecting, run the suites,
  count. Precedent 356 / 0 (#137) and 204,703 / all `Sort(1)` (#191); #136 is the case where a
  one-line arm change became a design decision plus a reseed.
- **#196's batching plan was written and then not followed**, and the batch paid two reseeds. If
  #194 turns out to need an ontology edit, hold it until #92's arm is known.

### 3. [d63-parse-gap-closure.md](d63-parse-gap-closure.md) — **Phase 4 of 4: performance**
Four-phase spine (user directive `2026-07-06`, worked in order — stop detouring):
**OOV ✓ → parsing gaps ✓ → ambiguity ✓ → performance (HERE).** Phase 3 closed by selection rather
than by the multiplicity reduction this note planned for it — see STATUS. The performance work
itself is on deck, not in this note.

#### STATUS
Phases 1 and 2 are CLOSED — `grammar-gap 0`, `missing-lexeme 0`. Phase 3 (ambiguity) was met by
**selection**, not by multiplicity reduction: the composed configuration (discourse loop + reading
ranker) reaches encoded 51 / ambiguous 0 / open 11 over the 62 units, against an isolated no-ranker
floor of 2 / 40 / 20. The forest is still ambiguous; the pipeline no longer is.

Live numbers live in `experiments/parsing/baseline.json` (grammar/forest) and
`selection-baseline.json` (ranker/choice) — two files, two gates, both enforced by
`scripts/eval-parse-rate.sh`. This entry does not restate them: a hand-copied table competes with
the gated truth and loses.

#### DONE
- **Phase 1 — OOV: CLOSED.** `missing-lexeme 0`, distinct OOV 0 (Stage-A augmentation grounds the page).
- **Phase 2 — parsing gaps: CLOSED.** `grammar-gap 0` (`20d608e`). History of the 12→…→0 descent and the
  per-gap root causes: **§0 + §3 of [d63-parse-gap-closure.md](d63-parse-gap-closure.md)** — not repeated here.
- **Faithfulness — exclusive-focus `alone` (`22e550a`).** Sentence 3 ("Each event alone does not lead to cell
  death") had **0 universals, a lost negation, and a "Department of Energy" subject**. The reranker was
  *already right* (it ranked `DOE` #19/drop, causative `lead` #0/#1) — the faithful reading existed at **no**
  cap, because post-nominal `alone` had no rule, so widen-on-failure kept lowering the cap until the noun-pile
  was the only complete parse. Fix: `alone` as a bare post-nominal `cat_pp` carrying the opaque
  exclusive-focus operator `ontology:sole` ("this event alone" ≡ "only this event"); reuses the existing
  `RefineKind::PpMod` rule with **zero new parser code**, closed-class ⇒ cap-exempt. Now:
  `∀x:(Σy:event. sole(y)). ¬(x causatively-leads-to cell_death)` — 50/50 readings, **0** noun-pile.

#### The two levers that outlived the gate
`pos_prune` (categorical drop of function-word-as-noun readings, `EIGENIUS_POS_PRUNE`, default-off,
never tested since post-nominal `alone` unblocked it) and the **mass-shim precision fixes** (§6 of
the parse-gap note — strictly-uncountable-head test + acronym↔domain-word collision filter) were
planned as the AMBIG→ENCODED levers. That gate closed by other means. Both still bear on **parse
time**, so they belong to phase 4 / [#97](https://github.com/eigenius/eigenius/issues/97) — see the
performance entry under On deck. Gate either on the deterministic sweep with `GAP` staying 0.

Levers already applied (hyphenation, build-then-subsume D3, sense cap/reranker) and the ones ruled
out for this corpus (NF §3.3 adjective rule): **§6/§6a of the parse-gap note** and
[d63-parsing-scale-and-pruning.md §4c](d63-parsing-scale-and-pruning.md).

#### DO NOT RE-TRY
- **Per-span pooled sense cap — tried, measured, REVERTED (`b91e100`).** Pooling the cap across a span's
  candidate lemmas *does* make the reranker's drop-verdict bite (a rank-dropped sense hiding in a sub-cap lemma
  bucket — `DOE` in the 2-entry `doe` bucket — otherwise slips the per-lemma cap). But it **regressed
  `grammar-gap 0 → 1`** (unit 52, *"The MSI relationship compared favourably…"*) by over-pruning a multi-lemma
  span, and it is **unnecessary now that `alone` exists** (the faithful reading is reachable at the tight cap,
  so widen-on-failure never fires and the junk is never admitted). Isolated by reverting *only* the seeding
  code — now the `dcg/parse/` module (the `dcg-cleanup` refactor split the old `lookup.rs`; the pooled
  sense-cap logic is in `parse/seed.rs`) — against the same store. Do not re-land without repeating that A/B.
- Kept from the same session: the **UMLS grammatical-surface filter** (17 surfaces incl. `does not`/`alone`/
  `lead`) in `crates/eigenius-umls/src/convert.rs` — that one is a keeper.

#### GOTCHAS (both cost real time — read before measuring)
- **Counting.** `summarize()`'s per-unit listing enumerates **only AMBIG units**; grammar-gaps print in a
  different format, so grepping `[AMBIG` **silently misses every gap**. Count from the
  `=== WRN first page over FULL lexicon: … grammar-gap N …===` summary line (or the `[unit N] … TAG` lines).
- **Snapshot drift.** A bootstrap-ontology edit changes its content hash, so older snapshots
  **ManifestDrift** — and the harness **SKIPs fail-closed while reporting `ok`**: every
  `db_backed_encoding` test goes green doing nothing. `kernel/tests/bootstrap_manifest_pinned.rs`
  (`0c33667`) now fails in `cargo test` when the bootstrap moves, so the *edit* is caught at source.
  The harness picks the newest `wordnet-umls-*` store under `../db-snapshot` automatically
  (`a46c107`; same rule as `scripts/measure-parse-rate.sh`) and prints which one it chose — no
  snapshot name is pinned in code any more. Note the rule is newest-by-mtime, so a *raw* reseed
  finishing after an aligned one wins; the printed path is how you notice.

#### Follow-up spun out of the faithfulness work (not started)
**Pre-nominal `only` / `just`.** Same `ontology:sole` operator (already in the ontology), but they attach
**outside the determiner** ("only [this event]") — NP-level focus, a different rule from `alone`'s N-level
refine (an NP-level rule must reach into the generalized quantifier's restrictor). Deliberately deferred rather
than shipping a mis-shaped N-level `only` that would only cover "the only X". Small, self-contained.

### 4. [d63-next-steps.md](d63-next-steps.md) — the D63 pipeline spine (the base)
Phase 1 is done (reshape, pipeline, grader, ingestion, D47 codec). Of **Phase 2** — "refactor the
LLM parts out into the orchestrator; the served gRPC path" — two of three parts landed via D71, and
not in the shape this note predicts:

- ✔ **Served path.** Built as a service operation (`FormalizeDocument` + task + four surfaces), not
  as the "second `impl DocumentPipeline`" the note anticipates. D71 §1 argues the shape.
- ✔ **Doc-layer home → committed branch.** `with_storage` doc branches.
- ☐ **Proposer impls → orchestrator RPCs.** NOT done. `kernel/src/dcg/resolver_llm.rs` and
  `sense_ranker.rs` still call Anthropic directly from the kernel (`use-llm`), which is why the
  kernel image needs `use-llm` and an API key to formalize. That is now the compose default
  (`2026-09-05`) — it was opt-in, which made every parsing run on a fresh doc branch a two-step:
  fail closed, rebuild, rerun.

Also remaining: **grading-phase gaps** (Citation grade-climb; graded-props over the full lexicon).

---

## On deck (pushed onto the stack when its step becomes active)

- **Phase 4 — performance.** The CKY chart explosion:
  [d63-parsing-scale-and-pruning.md](d63-parsing-scale-and-pruning.md) /
  [#97](https://github.com/eigenius/eigenius/issues/97) — adaptive supertagging + intermediate-cell
  felicity pruning. Cheapest first lever is the **mass-shim precision fixes**
  (d63-parse-gap-closure.md §6): spurious `mass` readings inflate BOTH reading count (median
  105/unit, capped at 256) AND parse time (up to 930 s/unit). `pos_prune` is the other untested
  lever (see entry 2).

- **`LayerTopology` resource fetch is uncapped.** `include_resources: true` emits one proto node per
  resource in the layer with no bound, so drilling into a lexicon layer is unbounded. Found during
  the kernel-OOM investigation; did not cause it. Wants a bounded page with an explicit `truncated`
  marker — never a silent cap (D62's rule). Not filed as an issue.

## Parked tracks (real, but off this stack)
Separate threads, not blocking the parse→encode pipeline; pull onto the stack only if picked up:
- **GH#104 — NbE readback panic** (`readback.rs:38`): surface `cell` resolves to UMLS **gene** concepts
  `C1413336`/`C1413337` (TUI **T028**), which are then **applied as functions** → `NotAFunction(ResourceVal(…))`.
  **Pre-existing** (48 panics on the pre-`alone` baseline, 32 on current HEAD — recent work reduced, did not
  cause it) and caught per-candidate, so **no unit is lost** and the sweep still completes. But an ill-formed
  term is reaching readback, so the defect is at the **construction site**, not readback; the `.expect()` is
  also the wrong failure mode. Off the critical path.
- **GH#103 — `CompleteJson` intermittently fails** ("No object generated: could not parse the response",
  patent-analysis notebook). Ruled out: the `main` merge (website-only), reseed/schema explosion (schema is
  class-derived, not chain-derived), `max_tokens` truncation (standalone repro used 304 of 2000 tokens).
  Two real findings: (a) the catch block discards `NoObjectGeneratedError`'s `finishReason`/`usage`/raw `text`,
  making every recurrence undiagnosable; (b) `orchestration/deno.json` pins `ai`/`@ai-sdk/anthropic` to
  **`@latest`** and the Dockerfile never copies `deno.lock` — the container has drifted **two majors**
  (`ai` 6.0.158→7.0.19, `@ai-sdk/anthropic` 3.0.69→4.0.11) and re-resolves on every restart, so local ≠ prod.
- [d61-llm-based-encoding-methodology.md](d61-llm-based-encoding-methodology.md) — grounding-discovery +
  typed decision-making layer (the D61 plan).
- Benchmark pilot (D50/D51) — chem+bio; kernel gaps done, infra gaps remain.
- [d63-passive-voice-handling.md](d63-passive-voice-handling.md) — general passive-voice infrastructure:
  object→subject promotion + agent suppression + `rel(theme, ground)` roles (importer `cat_pss` / a grammar
  passive rule). Serves the denominal phrasal half **and** ordinary passive clauses (`were represented by`,
  `is associated with`, … — in the current grammar-gap list). **Trigger:** closing passive clauses on the
  page, or the denominal phrasal half.
- [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md) — the **spec**: the
  `DenominalElement` table + the `⟦X-E⟧ = ⟦E link X⟧` alignment invariant for the denominal-adjective suffix
  class (`-based`/`-like`/`-mediated`/…). The **compound half is DONE** (compound-morphology §3b, shipped
  `2026-07-05`); the **phrasal** half → d63-passive-voice-handling.md. **Trigger:** after the phrasal half
  lands, to gate the `X-E ≡ E link X` equivalence.
- [d63-lexicon-augmentation.md](d63-lexicon-augmentation.md) — the `DocumentPipeline` generalization for
  **lexical gaps**: `AbbrDef → LexicalBinding{surface, long_form?, grounding}`, the pipeline as a
  lexicon-augmentation transducer (`AugmentOptions`/`LexiconProfile`/seed-in-added-out + the feedback cache),
  two-moment grounding with the concept-convergence invariant (`RecQ DNA helicase → C0084304`). **Trigger:**
  generalizing Stage A / closing `recq` via retrieval-grounding; needs the gene-family source
  ([[gene_family_lexicon_gap]]) + a lexicon/ontology index.

## Completed (record, not work)
- [d71-document-formalization-service.md](../design/d71-document-formalization-service.md) —
  **COMPLETED `2026-08-20`.** Prose formalization is **not** an institution: tested against D14
  §1.2's own four criteria it satisfies one, and every `institution:Institution` in the tree
  *decides* something while five importers doing source→resource-set→Load are none. Built instead as
  a service operation over `DocumentPipeline` emitting a resource-set artifact, run as a D21 task,
  behind four surfaces (CLI / gRPC / MCP / notebook). All seven slices in; verified in the container
  end to end (three sentences ENCODED, «These findings» resolved, 11 draws recorded on the `doc-<id>`
  branch, peak RSS 2.43 GiB) and in the browser for both landing modes. Playwright e2e descoped by
  user decision. Getting there fixed four defects the slices had left: the orchestrator's kernel
  passthrough is a curated list that did not route the two new methods (now pinned by a test); `cli`'s
  `use-llm` did not forward to `eigenius-encoding`; the kernel image had no WordNet dict, so the
  served parser silently used the no-op lemmatizer and every sentence came back `cut_grammar`; and
  `build-alignment-snapshot.sh` copied PROVENANCE verbatim, so an aligned snapshot could not be told
  from a raw one. **Open (§14):** source transport; draw commit granularity; pruning policy; the
  prefix-replay measurement. **§11 human-override loop: separate effort** — the crude form works via
  `--pins` + whole-document re-run. Remaining deferrals: D68 §5/§5a, D67 §8; kind verdicts, the
  selections-edited draw, and the demo re-pins await human sign-off.
- **Kernel OOM during a notebook session** — **FIXED `2026-08-20`** (`e10c9e6`), full account in
  [kernel-oom-notebook-session.md](kernel-oom-notebook-session.md). Retroactive validation's carrier
  scan materialised the whole 7.6M-resource chain per redefined property; it now streams (27.8 GB →
  3.9 GB), and `redefines_ancestor` compares against the shadowed definition in canonical Eigon-CBOR,
  so an identical redeclaration enumerates no dependents. The load that killed the kernel twice takes
  30 ms. Residue: the scan is still O(chain) for a *genuinely changed* definition — measured 3m55s
  for one property, kernel responsive throughout — filed as
  [#117](https://github.com/eigenius/eigenius/issues/117).
- **Reseed OOM** ([reseed-oom-memory-investigation.md](reseed-oom-memory-investigation.md)) —
  **CLOSED `2026-08-20`, not reproducible.** The claim was that it "blocks any fresh full reseed";
  full reseeds have since run to completion routinely (four snapshots dated `2026-08-20` alone),
  starting with `2026-07-10` and most likely fixed by the `--out-dir` chained-load path. The note
  survives as the record of what was measured out. Do not re-open without a fresh reproduction.
- [d69-reading-presentation.md](d69-reading-presentation.md) — **COMPLETED `2026-08-17`.** The ranker's
  question was unanswerable as posed: for «MSI cancer models did not have the exonuclease activity of
  WRN» the prompt carried 120 candidates with 120 distinct sems as only **4 distinct strings**, so the
  reading that landed in the demo artifact was chosen BLIND. Root cause (maintainer): strict
  verbalization is ~a left inverse of parsing, so all readings of one sentence converge by
  construction. Slices 2/5 (`Register::{Surface,Expanded}`, concept legend, injectivity guard, demo v2
  regeneration) done 2026-08-13 — the guard immediately found a second lossy site, a comparative's
  dropped standard, worth 50/1/11 → 51/0/11. Slice 3 done 2026-08-17: selection **21/31 (68%) →
  30/40 (75%)**, structure 23/31 → 33/40, 0 unadjudicated, live run and replay agreeing exactly
  (62/0 ranks, 40/0 selections). So blindness was PART of the earlier error, not all of it — 10
  decisions are still wrong with the pool fully visible. Slice 4 (D69-B two-level presentation)
  implemented, measured and **REJECTED**: 24/40 correct, structure 29/40 — six worse, on the same
  forest and ranks. Flat listing remains default; two-level kept behind `EIGENIUS_SELECT_TWO_LEVEL=1`
  since §5's preferred realisation was a two-CALL ranker and this was the cheap proxy. Logged
  truncation kept ON (D62 no-silent-caps). Detail in §7m.
- [d70-named-entities-syntax-vs-denotation.md](../design/d70-named-entities-syntax-vs-denotation.md) —
  **COMPLETED `2026-08-15`, demo fallout closed `2026-08-17`.** One importer flag (`Concept::symbol`)
  decided BOTH proper-name syntax and entity-vs-kind denotation, so the lexicon offered two of four
  combinations and the corpus kept needing the missing one — a bare-standing KIND-referring NP. Fixed
  at source, two causes: new `lexicon:Num::name` (bare + kind-denoting, no mass claim) granted by
  T047/T191; and T033 Finding removed from `COUNT_VETO_TUIS`, its one motivating collision (`gENE`)
  being handled per-atom by drops.json while the veto blocked head-inheritance for 107591 concepts.
  «MMR deficiency» and «Lynch syndrome» now reach C4522088 / C4552100 instead of Turcot / HNPCC;
  expected-hits 62/62 with an empty miss-set; 0 units discarding their sense ranking; readings 637
  (ceiling 700), skeletons 175. Everything the D69 chase compensated for was a concept that could not
  compose bare. FALLOUT, all closed `2026-08-17`: the bootstrap edit invalidated every snapshot
  (ManifestDrift on the `lexicon` layer), which broke both demos and — silently — the D67 §3.5
  acceptance and the ESL round-trip corpus. v2 re-recorded and verified; v1 RETIRED and deleted
  (sense-erased pins are inventory-dependent; README kept as the record); `acceptance.rs` and
  `esl_round_trip.rs` repointed at v2 (paths AND the `demo:formulas:` → `demo:v2:` namespace);
  `kernel/tests/bootstrap_manifest_pinned.rs` now pins the manifest so the next such edit fails in
  `cargo test` rather than days later by hand. RESIDUE (not blocking, none gated): D2 disease
  kind-vs-entity is unanswerable on this corpus; O4 (a concept's own PT outranking another's CE for
  the same string) unimplemented; abbreviations still reach bare standing via the glossary's `mass`
  inheritance rather than `name`; deep-binder round-trip coverage lost with v1's `rule-general.esl`;
  a lexicon-content change still moves no manifest, so that staleness class has no guard.
- **Phase-2 constructions, Step 5/5b/5c — COMPLETED `2026-07-06`.** RC-6 apposition (`appose_group`, bidirectional concept↔semantic-type felicity),
  comma-list connective inheritance (neutral comma finalized by the trailing `and`/`or`), and the
  **coordination refactor** to core-en's list-with-operator shape (`cat_coord` + `coordinate_prop` +
  `complete_coord`, retiring the eager `coordinate_sem` + the Step-5b n-ary workaround). Together −8
  grammar-gaps (20→12). Kernel lib 1611 + `closed_class` 126 green. Detail in d63-parse-gap-closure.md
  §4 Steps 5/5b/5c.
- [d63-compound-morphology.md](d63-compound-morphology.md) — **COMPLETED `2026-07-05`.** Derived-adjective
  OOV closed (Slices 1–2 + §3b denominal-suffix table + `-like` fix); missing-lexeme 6 → 2 over the
  snapshot. Deferred pieces extracted to the parked tracks above (alignment / passive-voice) and the
  gene-family track ([[gene_family_lexicon_gap]] — `recq`).

## Reference / design notes (consulted, not "work")
Not stack items — background for the above: `d63-{document-preprocessing-scope, kind-predication-reshape,
coren-coupled-port-design, pp-attachment-control-scoping, packed-forest-parsing-blueprint,
cnl-*}`, `d62-*`. Pull in when a step needs them.

---

### Maintenance
- Finishing the top's exit-gate → delete/collapse its entry and promote the next. Note the pop here.
- A new sub-task splitting off the active entry → write its note, push it as the new §1, demote the rest.
- This file is the index; the per-note detail lives in the linked notes, not here.
