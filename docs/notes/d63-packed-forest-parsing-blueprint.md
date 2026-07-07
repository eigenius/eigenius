# D63 — Packed-forest parsing with lazy semantics (blueprint)

**Status:** design, pre-implementation. Reviewable before any code (per the two expert reviews +
the grounding pass). Supersedes the "shape-aware cell beam" heuristic (Fix #2′) with the
principled form it was approximating.

## 1. Problem (witnessed)

Over the full WordNet+UMLS lexicon (~7.6M entries), a single chart cell accumulates tens of
thousands of parse items that **share one category shape** (`cat_shape` with type-indices erased —
e.g. `cat_n(Σ_, sg)`, the whole clause read as one deeply-nested compound noun) and **differ only
in their attached semantics** (which WordNet synset / UMLS CUI fills each type index — a Cartesian
sense-product). Measured: 30,128 same-shape items in one span; 51,060 top-level `cat_n` items in a
full-span cell (`db_backed_encoding` dumps, 2026-06-30). A naive global top-K beam fills with these
near-tied items and **evicts the rarer, correct `cat_s` clause reading** — a combinatorial loss, not
a linguistic one.

Three prior levers were witnessed **insufficient or refuted** for this: the sense cap + global cell
beam (tolerates, doesn't cure), the compound-depth CAP (A/B **refuted**, ~0 benefit — the piles are
shallow), and the LLM reranker (**refuted** — a sole-reading multi-word entry is never cap-truncated).
See [d63-cnl-parse-levers-plan.md](d63-cnl-parse-levers-plan.md). The `pos_prune` extension (Fix #2″,
landed) removes the *stop-word-bridge* subclass only.

## 2. Decision

**Method A — local-ambiguity packing with lazy semantics — not a diversity-preserving beam.**
The 30k same-shape items are one *equivalence class* for all future combination (our `apply`
combines on the **category only**; the sem is never consulted for combinability). So they should be
**packed under one representative**, with the differing sems materialized **lazily** on demand, and
the k-best extracted by **cube pruning** over the packed forest. A diversity beam is a lossy
approximation used only when a fine scorer must additionally rank/prune under budget — layered *on
top of* the packed forest, never as a global top-K.

## 3. Prior art (grounding — all primary, verify-confirmed 2026-06-30)

- **Billot & Lang 1989**, *The Structure of Shared Forests in Ambiguous Parsing*, ACL
  [P89-1018](https://aclanthology.org/P89-1018/): all parses as a shared AND-OR forest in ≤ cubic
  space; packing **signature = (category, span)**, not the semantics; extends to Horn-clause /
  unification grammars ("proof forests") — the thinnest bridge toward our proof-term semantics.
- **Harper 1994**, *Storing Logical Form in a Shared-Packed Forest*, CL 20(4),
  [J94-4006](https://aclanthology.org/J94-4006/): **Method 3** — store a *deferred procedure call*
  at each packed node, build the LF on demand; same node-count as the bare forest (9,658 B vs
  1,477,644 B eager at 132 parses). **This is our design.** Also states **the pitfall** (§6).
- **Dörre 1997**, *Efficient Construction of Underspecified Semantics under Massive Ambiguity*,
  ACL/EACL [cmp-lg/9706028](https://arxiv.org/abs/cmp-lg/9706028): build one *packed* UDRS off the
  forest; an OR-node's semantics is the disjunction of its children under one shared variable.
  ⇒ underspecified holes (scope/referent) are **compatible with packing**.
- **Huang & Chiang 2005**, *Better k-best Parsing*, IWPT [W05-1506](https://aclanthology.org/W05-1506/):
  k-best over a weighted hypergraph (= packed forest); k-best is bounded **per vertex (signature)**,
  not globally — `O(|V|·k)` — which is exactly what stops the pile evicting the real reading.
  Algorithm 3 (LazyKthBest) extracts k-best top-down, materializing only as needed.
- **Chiang 2007**, *Hierarchical Phrase-Based Translation*, CL 33(2),
  [J07-2003](https://aclanthology.org/J07-2003/): cube pruning — merged items retain the multiset of
  antecedent back-pointers; best-first over a sorted cube, prune the rest.
- **Hopkins & Langmead 2009**, *Cube Pruning as Heuristic Search*, EMNLP
  [D09-1007](https://aclanthology.org/D09-1007/): the clean formal seam — **postcondition
  (category) = combinability signature; carry (semantics/score) = cost only**. Maps 1:1 onto our
  felicity-by-category (postcondition) vs. differing sems (carry).
- **Clark & Curran 2007**, *Wide-Coverage Efficient Statistical Parsing with CCG*, CL 33(4)
  ([pdf](https://www.cs.ox.ac.uk/people/stephen.clark/papers/cl07parser.pdf)): the C&C **packed
  chart** — signature = *category + head + unfilled dependencies*; entries in a class are
  interchangeable for all subsequent combination. The CCG-specific precedent for our signature.
- **Oepen & Carroll 2000**, *Ambiguity Packing in Constraint-based Parsing*, NAACL
  [A00-2022](https://aclanthology.org/A00-2022.pdf): subsumption/equivalence packing at scale in HPSG
  (the LKB/PET lineage) — the practical-systems precedent.

*(Grounding hygiene TODO: add these as verified entries to
[docs/references/eigenius_related_work.bib](../references/eigenius_related_work.bib).)*

## 4. The packing signature

**`(cat_shape, ENF-provenance-class)`** — the category with type-indices erased, plus the Eisner
normal-form status that governs future combinability.

- **Why `cat_shape`:** `apply`/`apply_combine` combine on the category only ([parser.rs:190](../../kernel/src/dcg/parser.rs)),
  via `unify_cat`/`is_ctor`; they never branch on a sem's *value*. So two items with the same
  category shape are interchangeable for all future combination.
- **Why `ENF-prov`:** ENF blocks a `>B`/type-raised output from being the primary functor of a later
  application ([parser.rs:264](../../kernel/src/dcg/parser.rs), keyed on `left.prov`) — provenance is
  a *syntactic* feature that affects combinability, so it must stay in the signature. (Analogue of
  C&C's "unfilled dependencies".)
- **Dependent-type nuance (the one subtlety):** a type-index is **"carry" for combination** (never
  consulted by `apply`) but **"postcondition" for the top-span felicity filter** (the kernel
  type-check *can* reject an ill-typed sense combination). This is sound to erase from the *packing*
  signature **because** the felicity check is never an intermediate combination gate (§6) — it runs
  only at extraction, on the materialized sem. Fail-closed rule (Harper/Hopkins): *do not erase any
  feature we cannot show is unconsulted by both `apply` and every intermediate admission step.*
- **Discovered exception — `cat_group` is NOT pack-safe (audit 2026-06-30).** The metamorphic audit
  (§6) found that the coordination/distributive rules read the sem: `distribute`/`distribute_object`
  call `group_members(group_sem)`, which walks the sem as a `cons/nil` list and returns `None` on a
  non-list ([category.rs](../../kernel/src/dcg/category.rs)) — so combinability there is
  *sem-determined*, not category-determined. This is Harper's pitfall instantiated. **Resolution:**
  exclude `cat_group` from packing (its own non-packed / singleton path); costs nothing since
  coordination is orthogonal to the `cat_n` pile. Every *pile-forming* rule (application,
  composition, N-N compound, attributive-Σ, determiner) is pack-safe — verified: they embed
  `left.sem`/`right.sem` as **opaque subterms** only.
- **Erasing the type-index needs a PRECONDITION — the index-independence of functor slots (2026-06-30,
  Option A).** `cat_shape` erases the type index, but `unify_type` does concrete subsumption
  (`type_subsumes` → `is_subclass_of`, [category.rs](../../kernel/src/dcg/category.rs)) when a functor
  argument slot is a **concrete** type. So combinability is index-dependent iff a functor slot is a
  concrete subtype (not `Entity`-top / not a type variable). Checked both lexicons:
  - **Imported corpus (WordNet+UMLS, 7.6M — the pile):** every verb/adj slot is
    `cat_np(lexicon:Entity, …)` ([convert.rs:187](../../crates/eigenius-wordnet/src/convert.rs#L187)) →
    `type_subsumes(Entity, X)` true for the whole noun lattice → **index-independent → `cat_shape`
    packing is sound.**
  - **Demo lexicon (battery):** has real selectional restrictions — `depends_on :
    (S\NP_CellLine)/NP_Gene` ([lexicon.esl:152](../../experiments/lexicon/lexicon.esl#L152)) →
    index-dependent → node-level `cat_shape` packing would be **unsound** there.

  **DECISION — Option A (gated node-level packing).** Node-level packing by `(cat_shape, ENF-prov)`
  collapses combination from O(N×M) per cell to **O(1) per node-pair** (the only thing that scales to
  7.6M). Correctness is preserved regardless: node-level packing merely **defers selectional pruning
  from parse-time `unify_type` to the felicity pop-filter** — the exact cube-pruning premise (pack the
  syntax, defer the fine check). Final k-best is identical **iff felicity ⊇ `unify_type`** (the
  differential oracle proves this). Its residual risk is a selectional lexicon causing the pop queue
  to spin on locally-packed items that fail felicity — bounded by `max_pops`.

  **Hardening — the static grammar-load guard + router.** Enforce the index-independence precondition
  once, at grammar-load (not per parse): scan the active grammar's functor **argument** slots; set
  `has_selectional_restrictions = true` if any is a concrete type other than `Entity` (or a type
  variable). Router at parse start: `if !has_selectional_restrictions && packing_enabled` → the
  **packed** CKY loop; **else** → the current **unpacked** loop (with `pos_prune`) — the default and
  the fallback for selectional lexicons. So packing is *never* applied where its precondition fails
  (fail-closed), and the differential oracle validates equivalence on a non-selectional corpus.
  (Chosen over the shape-aware *beam* (Option C), which bounds a cell's output but still pays the
  Cartesian `combinable` toll in the inner loop — no O(1) node-pair win.)

## 5. Data structures (grounded in today's `Item`/`Cost`/`classify_felicitous`)

```
PackedNode {                       // one per (cat_shape, ENF-prov) per cell
    sig: (CatShape, ProvClass),
    rep_cat: Exp,                  // a representative full category (indices intact) for unify_cat
    items: Vec<PackedItem>,        // cost-sorted; the lazy stream (Harper Method 3)
}
PackedItem { cost: Cost, build: Thunk<Exp> }   // build() materializes the sem Exp on demand

CubeCandidate { cost: Cost, left_idx: usize, right_idx: usize }  // min-heap by cost
```

- Combination builds a **packed** result: `apply` runs once on the two representatives' categories;
  the result's sem stream is the lazy product of the children's streams (not materialized).
- `Cost` is already an additive, monotone key (`saturating_add` of non-negative leaf costs +
  compound penalty) → cube-pruning monotonicity holds with no A* heuristic.
- **Structural guard (make combinability sem-blind) — IMPLEMENTED as decision/build (Stage 2).**
  The literal `apply(&CategoryPayload) -> CategoryPayload` is **not achievable** here: several
  nominal-modification rules (attributive-Σ, PP-mod) build the result *category* out of the modifier's
  *semantics* — in this CN-as-types system the noun's type index embeds the modifier predicate (e.g.
  the attributive rule sets both cat and sem to `Σx:C. adj_predicate(x)`, and the adjective's
  category `S[adj]\NP` does not contain that predicate). So the achievable form is **decision/build**:
  - `combinable(&CategoryPayload, &CategoryPayload) -> Option<SemRecipe>` — the **sem-blind DECISION**
    (handed only `CategoryPayload`s ⇒ compiler-enforced it cannot read a sem); returns a `SemRecipe`
    carrying category-derived data.
  - `build(recipe, &Item, &Item) -> Item` — the ONLY place a child sem is read; for the nominal rules
    it builds the category too (CN-as-types).
  - `apply_group(&Item, &Item)` — the `cat_group` carve-out (§4), sem-reading, tried only after
    `combinable` returns `None` (group categories never match a sem-blind rule, so order is preserved).
  This still delivers the guarantee packing needs — the *combinability decision* is provably sem-blind
  (the packing signature is sound) — which is the real content of Hopkins & Langmead's postcondition/
  carry split under dependent types. (`apply_core`, the flag-off combinatory-core spike, is still
  item-level — a follow-up; not on the default path.)
- **`max_pops` bound (extractor):** the k-best loop stops at `min(k, max_pops)`; if it hits
  `max_pops` before `k` felicitous items (a dense pocket of kernel-rejected pops), it yields the
  partial list and `log()`s the shortfall — never stalls, never silently truncates.

## 6. The felicity oracle is already the pop-filter (no checker changes)

Verified by trace (2026-06-30):

- **Combination does zero type-checking.** The CKY loop ([lookup.rs:1247-1266](../../kernel/src/dcg/lookup.rs))
  calls `apply` only; `apply_combine` is pure categorial unification. **⇒ sems are pure "carry"
  through the whole chart; there is no intermediate semantic admission gate.** (This is the soundness
  lemma for §4's erasure — worth a guard-test.)
- **The felicity check is a pure filter, run only at the full span:**
  `reduced_felicitous(&Item) -> Option<Item>` and `classify_felicitous(&Item, hole_specs) ->
  Option<FelicitousOutcome{Closed|Open}>` ([lookup.rs:1734](../../kernel/src/dcg/lookup.rs),
  [:1758](../../kernel/src/dcg/lookup.rs)). It is **1→Option** (routes Closed vs Open; never
  branches), **cost-invariant** (`cost: it.cost`), and needs only `(cat, sem)`.

⇒ The cube-pruning extractor calls the **existing** `classify_felicitous` as its per-pop filter,
unchanged. Extraction loop: pop lowest-cost `CubeCandidate`; materialize `sem = App(L[i], R[j])`;
run `classify_felicitous`; on `Some`, yield to the k-best buffer; **regardless of pass/fail**, push
neighbors `(i+1,j)` and `(i,j+1)` (dedup via a `visited` set); stop at k or empty.

## 7. Holes / underspecification (`hole_specs`) — unchanged

`hole_specs` is a **global, sentence-level, span-indexed superset** built once from `n`
([lookup.rs:1635](../../kernel/src/dcg/lookup.rs)); hole names are **span-pure**
(`$quant$i_j`, `$anaphor$i_j`), freshened per span at the shift site — never sense-generated.
`classify_felicitous` self-selects each item's holes via `exp_mentions_var`. ⇒ **no per-item
`hole_specs` payload is needed** in the packed stream; freshening composes with lazy materialization
for free (leaf sems carry their holes; lazy `App` propagates them). The 1→N *resolution* branching
(`resolve_open`/`resolve_with`) stays a post-extraction global pass, also cost-invariant (Dörre:
resolve scope *off* the packed structure, after extraction).

## 8. Rollout & verification

- **Incremental, behind a flag**, keeping the current global beam as the byte-deterministic fallback
  (like `with_cell_beam` / `with_pos_prune`).
- **Soundness guard-test (§6 lemma):** a kernel test asserting no intermediate cell admission depends
  on sem content (combination is a function of category+prov alone).
- **Regression gates (unchanged bar):** battery 104 green; the first-7 CNL sems parse identically;
  closed-term determinism preserved.
- **Win metric:** on the pile sentences, the real `cat_s` reading survives extraction (GAP→parse
  where it was *beam-blocked*, not grammar-blocked) and the full-span materialized-item count drops
  from ~30k to O(k). Grammar-blocked sentences (MSI alias, comparatives) are unaffected — separate
  fixes.

## 9. Open questions — status after expert review (2026-06-30)

1. **Dependent-type definitional-equality signature — RESOLVED (operationally).** No cited result
   covers erasing dependent-type indices before a post-hoc check, but the erasure is safe *iff* no
   intermediate combination branches on `sem`. That is now enforced two ways: the §6 metamorphic
   guard-test (executable) and the §5 structural split (`combine_cat` has no sem in scope). Rests on
   the test + structure, fail-closed — not on a citation.
2. **C&C head + unfilled-dependencies analogue — RESOLVED.** C&C needed the lexical head because
   their *statistical model conditioned on head-word dependencies*; our combination + felicity are
   type-theoretic and our cost is additive-from-leaves, so no head is consulted. `(cat_shape,
   ENF-prov)` is sufficient — *unless* ENF-prov or `Cost` ever start branching on a head (guard-test
   would catch it).
3. **Lazy-unpack cost under downstream type-checking — MITIGATED.** Packing is exact/polynomial, but
   the per-pop felicity check is the expensive op; a dense pocket of kernel-rejected pops could spin
   the queue. Mitigation: the `max_pops` bound (§5) — yield the partial k-best and `log()` rather
   than stall. A k-best-per-signature cap during extraction remains available if needed.
4. **`cat_group` non-pack-safe — RESOLVED (excluded).** See §4: coordination reads the sem; routed
   off the packed path. The metamorphic audit found it; the guard-test keeps it (and any future
   sem-reading rule) out of the packed classes.

## 10. Implementation status (2026-06-30)

- **Stage 1 (owned split) — DONE, committed.** `Item { category: CategoryPayload, semantics:
  SemanticPayload }` + accessors. Caught a latent bug (a `lexicon_order` write silently no-op'd via a
  `cost()` copy).
- **Stage 2 (sem-blind decision/build) — DONE, committed.** `combinable(&CategoryPayload,
  &CategoryPayload) -> Option<SemRecipe>` (the compiler-enforced sem-blind decision) + `build(&Item,
  &Item)` + `apply_group` carve-out. The literal `apply(&CategoryPayload)->CategoryPayload` proved
  impossible (CN-as-types: nominal rules build the result category from the modifier sem); the
  achievable form is decision/build, which still gives the guarantee packing needs.
- **Stage 3a — DONE, committed.** Option A recorded (§4); `category::cat_has_selectional_slot`
  predicate (the index-independence gate) implemented.
- **NOTE — the runtime metamorphic audit (§6 layer b) is SUBSUMED** by Stage 2's compile-time
  guarantee (`combinable` cannot see a sem), so it is not needed as a separate test.
- **CORRECTION to §8 / the "delete widen-on-failure" note:** widen is **retained** for the UNPACKED
  path (selectional lexicons + the differential-oracle baseline); the packed path is beam/widen-free
  *by construction* (packing never drops the needed constituent). "Delete widen" was premised on
  packing replacing the path — but Option A keeps both, gated.

## 10b. Phase-3 RESULT (2026-06-30) — packing works and pays off

Stages 3b–3d are implemented and **validated end-to-end**:
- **Correctness (3f.1 differential oracle):** on index-independent, construct-free demo sentences
  (determiners, attributive, compound, application), the packed path produces the **identical**
  closed+open forests as the unpacked path (`packed_forest_equals_unpacked_on_core_grammar`). This
  also confirms felicity ⊇ `unify_type` (deferring selectional pruning to the felicity pop-filter is
  sound). The oracle **caught a real bug**: `cat_shape` erased the `cat_forall` determiner λ-body,
  collapsing subject- and object-GQ readings into one node — fixed by shaping the `Lam` body so the
  signature is a combinability congruence.
- **Win (3f.4), full 7.6M WordNet+UMLS lexicon:** on a *packable* pile sentence ("DNA repair
  processes are attractive synthetic lethal targets") packing is **~8× faster with identical output**
  — unpacked open×232 @ 11.1s vs packed open×232 @ 1.4s; a light sentence is unchanged (no
  regression). The sense-product pile collapses to O(nodes).

**Honest reach:** this win is on sentences the router packs — no relatives / commas / coordination.
The *worst* corpus piles contain those and still route unpacked, so extending the win to them is the
**pack-relative-clauses** follow-up (§11 3g.3), now justified by this measured payoff.

## 10c. 3g.3 — pack RELATIVE CLAUSES (2026-06-30): done + correct, but the win is same-shape-scoped

Implemented restrictive `that`/`which` relatives as a packed `Edge::Relativize` (decision cat-based
via `relativize`; result embeds `body.sem`, materialised per item-pair by the same cube extractor).
Unguarded **`that`** (its restrictive-relative use is packed; its complementizer use is a lexical
leaf; appositive `, that` stays comma-guarded; pied-piping uses `which`, kept guarded). Differential
oracle **extended with `that`-relative sentences — passes** (packed ≡ unpacked, incl.
`a/every gene that affects HeLa is large`); battery 107.

**But the win did NOT extend to the hard relative pile sentence.** Measured on the full lexicon,
"We analysed these data sets for genes that are selectively essential in cancer cells with MSI":
**unpacked 221.6s vs packed 210.6s (~5%)**, both GAP. The 8× on "DNA repair processes…" was a
*same-`cat_shape` sense-product* pile (collapses to one node); this sentence's cost is **structural**
— PP-attachment ambiguity (`for … in … with …`) spawns many *distinct* cat-shapes, so the node count
itself is large and packing (which collapses *same*-shape) doesn't help. **Finding: packing's win is
scoped to same-shape piles; structural (multi-shape) ambiguity is a different bottleneck that packing
does not address.** So relative-packing is correct and free coverage, but the worst prep-heavy
corpus sentences need a *different* lever (PP-attachment control), not packing.

**Reserved-token refactor (§11 follow-up, done in-code):** the parser's reserved construct tokens
(`and`/`or`/`that`/`which`/`but`/`not`/`each`/`other`/`,`) are now centralised in
[`dcg/reserved.rs`](../../kernel/src/dcg/reserved.rs) — the guard, the relative rule,
`coord_connective`, and the CKY special-construct rules all classify tokens there instead of
hard-coding literals. Behaviour-identical (battery 107). **Follow-up (reseed-gated):** back the table
with an ontology declaration so the set is *data*, not code.

## 11. Phase-3 burn-down — STATUS (2026-06-30) + deviations from plan

**3b — Guard + router + flag** — ✅ DONE, committed.
- [x] **3b.1** `packing` field + `with_packing(bool)`.
- [x] **3b.2** the guard — landed as **`parse_needs_unpacked`** (renamed; **wider than planned**: also
  routes any *token-keyed construct* — relatives `that`/`which`, `but`, `,`, `each other` — to
  unpacked, not just coordination). Uses `cat_has_selectional_slot`; uncapped, fail-closed.
- [x] **3b.3** router in `parse_scoped_open` + `parse_packed` stub + body renamed `parse_unpacked`.
- [x] **3b.4** 4 predicate unit tests + `packing_flag_router_is_a_safe_noop`.

**3c — Packed forest** — ✅ DONE, committed. *Deviations:* `sig` field dropped from `PNode` (the cell
map is the source of truth); `rep` is an `Item` (not `CategoryPayload`); **`Group` edge dropped**
(3c.5) — the guard keeps coordination off the packed path; construction uses **`apply` on reps** (not
`combinable` directly) — same O(1)/node-pair, and `apply` internally is the sem-blind `combinable`.
- [x] **3c.1** `Sig`, `node_sig`. **3c.2** `NodeId`, `Edge{Leaf,Combine,Unary}`, `PNode{span,rep,edges}`,
  `Forest` + get-or-create; unit tests.
- [x] **3c.3** shared **`seed_leaves`** extraction (a refactor beyond the plan — unpacked stays
  byte-identical) + leaf grouping into `Leaf` edges.
- [x] **3c.4a** node-level binary CKY (`Combine` edges). **3c.4b** composed-cell **`Unary`** edges
  (bare-NP shift / type-raise / fronted participial) — a distinct sub-unit added for oracle parity.

**3d — Cube-pruning extractor** — ✅ DONE, committed.
- [x] **3d.1** `CubeCandidate` + `(cost, li, ri)` `Ord` (unit-tested). **3d.2** memoized `kbest`
  (`Leaf`/`Combine` cube/`Unary`). **3d.4** real `parse_packed` (top-span kbest + felicity Closed/Open).
- [x] **3d.3** `max_pops` bound **+ shortfall `log()`** (the cube logs a partial extraction; no silent
  truncation).

**Also landed (unplanned):** `cat_shape` **`Lam`-arm soundness fix** — the oracle caught that the
`cat_forall` determiner body was erased, collapsing subject/object GQ into one node.

**3f — Validation** — ✅ DONE, committed (result in §10b).
- [x] **3f.1** differential oracle (`packed_forest_equals_unpacked_on_core_grammar`).
- [x] **3f.2** guard fail-closed — the router decision is now **directly asserted** via the exposed
  `routes_packed` (`packing_router_decision_is_correct`): packable ⇒ packed; selectional / token-keyed
  construct / flag-off ⇒ not packed.
- [x] **3f.3** regression: battery 107 + workspace green, flag off. **3f.4** win probe (~8×, §10b).

## Deferred (open) — nothing silently dropped

- **3g.3 restrictive `that`-relatives — DONE** (§10c): packed + oracle-verified. But the win is
  same-shape-scoped; the hard prep-heavy relative pile sentences are bottlenecked by **structural
  PP-attachment ambiguity**, not the same-shape sense-product, so packing gives ~5% there.
- **3g.3 ALL token-keyed constructs — DONE** (§10d): the `Edge::Relativize` special-case was
  generalized to `Edge::Binary { left, right, rule: BinRule }` (a cat-based decision on
  representatives, sem materialised per item-pair at extraction via `apply_bin_rule`), covering
  relatives, coordination (`and`/`or` + list comma → the `Coordinate` edge + group-distribution
  `Combine`), the reciprocal (`each other` → `Reciprocal`), `but not` (`ButNot`), and appositives
  (`Appositive{Subj,Obj}`); the fronted-modifier comma is a new `UnaryKind::AbsorbComma`; the
  wh-determiner `which` is an ordinary lexical leaf. `dcg/reserved.rs::keys_unpacked_construct` was
  **removed** (no token-keyed construct is unmirrored). Oracle extended with coordination /
  reciprocal / but-not / which-relative / wh-det / comma sentences — all packed ≡ unpacked; battery
  107, workspace clippy/fmt clean.
  - **DEVIATION (recorded):** the plan's "pack `pied_pipe` (ternary)" item was NOT done as a ternary
    edge. Pied-piping (`[prep] which [subj] [VP]`) is genuinely ternary and forms **no pile**, so a
    ternary edge (or a `cat_n/(S\NP)` intermediate category existing nowhere else) is disproportionate
    complexity for zero packing benefit. Instead the router (`parse_needs_unpacked`) detects it
    **structurally** — a `which` immediately after a VP-adjunct preposition — and routes it to the
    proven unpacked path. This is the router doing its designed job (divert constructs not
    beneficially packable), not a wedge; plain which-relatives and the wh-determiner `which` ARE
    packed, capturing ~all of `which`-usage.
- **PP-attachment control** (L, NEW — the real lever for the hard corpus piles): the worst sentences
  spawn many *distinct* cat-shapes via `for … in … with …` attachment, which packing does not
  collapse. This — not more packing — is what those sentences need. (Separate from packing.)
- **reserved-token ontology backing** (M, reseed-gated): move the `dcg/reserved.rs` table into a
  `closed-class.esl` declaration loaded at index build, so the reserved set is data. Ride the next
  reseed.
- **3g.1** (M): `apply_core` (combinatory-core spike) into `combinable`/`build` — packing currently
  requires `!combinatory_core` (flag off by default, so not on the critical path).
- **3g.2** (S): make packed the *default* for index-independent grammars once broadly validated.
