# Work stack — unfinished work (top = active)

The single "where are we" pointer. A **LIFO stack** of the active working notes: work the **top** entry;
when its exit-gate is met, **pop** it and the entry below becomes active. When a sub-task splits off from
an entry, **push** its note on top. Keep this file current — it is the map back to the base plan after
any detour.

---

## Stack (top → bottom)

> **ACTIVE: entry 1 (`2026-08-22`), pushed onto the empty stack.** The parser-pipeline spine emptied
> on `2026-08-20` when D71 met its gate; entries (2) and (3) below are the pre-D71 spine, assessed on
> `2026-08-19` as largely implemented-or-obsolete, so neither was promoted by position. P2 was picked
> up instead. The two candidates that were live and stay unstarted:
>
> - **D71 residue** — §14's four open questions (source transport, draw commit granularity, pruning
>   policy, the prefix-replay measurement) and §11's human-override loop, which the §9 draws-on-branch
>   decision shrank from a design problem to a measurement.
> - **D61 faithfulness** — the half that D71 §10 reserved the institution shape for, and the only
>   thing in the tree that still earns it.

### 1. [p2-type-theory-soundness-plan.md](p2-type-theory-soundness-plan.md) — **P2 · type-theory soundness**
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
  separates rather than breaks. **One question left open**: Rule 16 is schema-driven, so validating
  `TypeExpr`'s own `type_name` values reads `TypeExpr`'s `ctors`; settle termination before
  committing. **Not part of #188** — separate change, own gate — but it SHOULD ride #188's reseed if ready in time: batching is cheaper, and the validator question surfaces in a 2s bootstrap test, not mid-reseed.
- **`param_kind`'s missing `EigonClass` arm is a live bug**, independent of all the above and of any
  ontology edit: a class-typed inductive parameter is silently typed `Set`, which accepts anything.

#### NEXT
P2's design notes are all written and every code item is closed. Remaining open: **#188** (held, see
N3) and **#213** (folds into #188's reseed, or stands alone if something else forces one). The
package is otherwise done — all independent and all writable before any code: N1 positivity criterion +
declaration routing, N2 sized types wire-or-delete, N3 universe polymorphism. Steps 1–3 (#213, #64,
#194) need no design input and can run alongside.

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

### 2. [d63-parse-gap-closure.md](d63-parse-gap-closure.md) — **Phase 4 of 4: performance**
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

### 3. [d63-next-steps.md](d63-next-steps.md) — the D63 pipeline spine (the base)
Phase 1 is done (reshape, pipeline, grader, ingestion, D47 codec). Of **Phase 2** — "refactor the
LLM parts out into the orchestrator; the served gRPC path" — two of three parts landed via D71, and
not in the shape this note predicts:

- ✔ **Served path.** Built as a service operation (`FormalizeDocument` + task + four surfaces), not
  as the "second `impl DocumentPipeline`" the note anticipates. D71 §1 argues the shape.
- ✔ **Doc-layer home → committed branch.** `with_storage` doc branches.
- ☐ **Proposer impls → orchestrator RPCs.** NOT done. `kernel/src/dcg/resolver_llm.rs` and
  `sense_ranker.rs` still call Anthropic directly from the kernel (`use-llm`), which is why the
  kernel image needs `CARGO_FEATURES=use-llm` and an API key to formalize.

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
