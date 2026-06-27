# Chain-scaling audit — full-chain scans (`iter_all_resources`) and O(chain) hot spots

**Status:** living audit. **Context:** D23 (out-of-core layers). Large knowledge-graph
sources (UMLS Level-0 ≈ 281k resources, more to come) live on the *interactive* chain
as real KG content — that is the design target, not a stress hack. So any operation
that walks the whole chain is fine on a small chain and seconds-slow on a large one;
these are first-class scaling bugs to fix structurally, never to route around by moving
data off-chain.

## The anti-pattern

`Layer::iter_all_resources()` (and `iter_resources()`) materialise resource bodies —
under D23 each one pages cache→backend. Using them to *find a typed subset* (e.g. "all
`Institution` resources", "all `InductiveType`s") is O(chain) when it should be
O(matches). Likewise, re-`resolve`-ing the same definitions per resource is O(chain)
per call.

## Two fix techniques (and when each is safe)

1. **Index-driven discovery** — `crate::layer::resolve_typed_resources(layer,
   &[metaclass_iri, …])` finds resources of a type without materialising the chain,
   then resolves each to its top view (which also filters DAG-wide index hits to this
   chain). O(matches). It walks the chain consulting **each layer's own storage** in
   either state: the **triple index** for stored layers (populated at `store_layer`)
   and the **`pending` stage** for in-flight (built-but-not-stored) layers — so it is
   correct against freshly-built layers too (bootstrap, build-then-compile), not just
   committed chains. Triple indexes are deduped by `Arc` identity, so a shared-storage
   chain (production) scans once; per-layer-storage chains (some tests) are also
   covered. `is_a` must be indexable (`data_type = resource_array`) for the stored
   path — true on any core-rooted chain.
2. **Per-pass resolve memo** — `crate::layer::ResolveMemoScope` memoises
   `Layer::resolve` for the duration of an immutable-chain pass (used by validation).
   Collapses millions of redundant deep resolves into one per `(layer, iri)`.

## Site inventory (`kernel/src` `iter_all_resources` callers)

Legend: ✅ fixed · ⚠️ confirmed bottleneck, open · 🔵 legitimate (inherently needs all
resources) · 🔎 needs review/measurement.

| Site | What it finds | Disposition |
|---|---|---|
| `institution/registry.rs` `from_layer` | Institution/QueryClass/Comorphism/Export·ImportFormat | ✅ commit hot path now uses `from_layer_indexed` (index-driven); `from_layer` kept for tests + non-hot callers. Equivalence-tested. |
| `capability/registration.rs` `build_wasm_institution_runtime` | WASM `Institution`s | ✅ commit hot path uses `build_wasm_institution_runtime_indexed`; full-scan kept for other callers. |
| `capability/registration.rs` `validate_external_institution_chain` | external `Institution`s | ✅ switched to `resolve_typed_resources` (single hot-path caller). |
| `server/topology.rs` (was 2×: counts + nodes) | per-layer kind counts / nodes | ✅ counts via triple index; resource nodes only when `include_resources=true`; stack view is layers-only. |
| `esl/compile.rs` `collect_ctors_from_layer` | `InductiveType` (+ embedded ctors) | ✅ **fixed** (was the ESL-compile ~7s). Index-driven via `resolve_typed_resources`. The in-flight gap (compile runs against not-yet-stored layers during bootstrap → `lexicon:Cat` not in the index) is handled by `resolve_typed_resources` itself: it consults each chain layer's **own storage** — triple index for stored layers + `pending` for in-flight ones — so no build-time persisted artifact is needed. |
| `esl/compile.rs` `collect_macros_from_layer` | `core:Macro` | ✅ **fixed** — same mechanism (`resolve_typed_resources` with `core:Macro`). |
| `query/evaluate/pattern.rs` `collect_candidates_via_scan` (untyped `MATCH ?r {}`) | all resources | ✅ **fixed** for subject-bound WHERE (`LIKE`/`=`/`IN`) via subject-predicate pushdown — was the `query` RPC's 66s. Remaining untyped+non-subject-WHERE still scans (🔎, secondary). |
| `query/evaluate/pattern.rs` `resolve_name` (ShortName) | class by `short_name` | ✅ **fixed** — namespace-scoped, index-driven (`crate::query::resolve::resolve_scoped_name`); see section below. |
| `query/type_check.rs` `resolve_short_name_to_query_class`, `resolve_property_name` | class/property by `short_name` (compile) | ✅ **fixed** — same `resolve_scoped_name`. `query/evaluate/similarity.rs` `resolve_property_name` too. |
| `query/evaluate/{fiber,similarity}.rs`, type_check other sites | EigenQL eval | 🔵/🔎 genuine all-resource MATCH (🔵) vs find-by-type (🔎); review per operator under real workloads. |
| `server/inspect.rs` `resource_count = head.iter_all_resources().count()` | total count for inspect/health | 🔎 O(chain) just to count. Prefer summing `LayerHandle.resource_count` over the chain (metadata, no bodies). |
| `program/axiom_env.rs` | axiom resources for a program run | 🔎 per-program-run; review if it's find-by-type. |
| `nbe/check.rs` | resources during NbE checking | 🔎 review. |
| `validation/retroactive.rs` `scan_chain_for_property_carriers` (case 2) | lower-layer carriers of a newly-declared property | ✅ **fixed** — was `iter_all_resources()` over the whole chain (OOM-killed the kernel at 7.6M resources when a commit declared a property). Now gated behind `redefines`: a brand-new property provably has no lower carriers once property **keys** must be declared (Rule 22 §c), so only the rare redefinition scans. See "Retroactive carrier scan" below. |
| `dcg/lookup.rs` | lexical entries for DCG | 🔎 parse path; the lazy `LexicalIndex` (value index) is meant to cover this — confirm this site isn't a redundant full scan. |
| `layer/consolidate.rs` | range consolidation | 🔵 bulk maintenance op, inherently whole-range. |
| `layer/mod.rs` (test helpers) | — | 🔵 tests. |

## Confirmed measurements (UMLS chain, kernel `duration_ms` + orchestrator `latency_ms`)

- Commit pipeline: **2–27 ms** (was fine).
- Post-commit institution rebuild: **9.3 s → ~0.4 ms** after index-driven fix.
- `layer_topology` (stack view): **57 ms** after lazy fix.
- `load` (ESL compile): **~7 s** ← `collect_ctors`/`collect_macros` full scans. ✅ fixed.
- `query` (EigenQL): untyped `MATCH ?r {}` + `WHERE LIKE` **66 s → 0.01 s**; `WHERE =`
  3.5 s → 0.00 s; typed/indexed query 3.6 s → 0.00 s. Two fixes: subject-predicate
  pushdown (below) + `type_check` using the index-driven institution rebuild (below).
  All ✅ — queries are effectively instant on the UMLS chain now.
- `run_program_by_iri`: ~19 s, dominated by an ~11 s LLM call (not a chain scan).

### Full-scale load (2026-06-27, fresh DB): WordNet full + UMLS `--all`

Loaded **27 layers / 7,615,963 resources** clean (no errors/OOM) via the partitioned
`--out-dir` chained-load path — ~27× the prior ~281k on-chain UMLS baseline, the largest
chain loaded to date.

- **WordNet full**: 104 s, 3 layers (484k resources). Previously *unloadable* — the 164 MB
  single document exceeds the 128 MiB gRPC Load limit. Fixed by adding partitioned
  `--out-dir`/`--split-bytes` to `wordnet-import` (parity with `umls-import`): base layer
  (descriptor + all synset classes/axioms, ~20 MB) + `LexicalEntry` chunks; entries resolve
  their class + descriptor against the base below (no cross-chunk dependency).
- **UMLS `--all`**: 976 s, 24 layers (~7.1M resources: 2.38M concept classes + 4.75M lexical
  entries). The single-file `--out` + in-memory `--validate` path OOM-kills at this size; the
  `--out-dir` path streams chunks and the kernel validates each layer at load (no OOM).
- **Per-commit `duration_ms`** (concept chunks, ~320k resources each): avg **33.3 s**;
  first-5 **29.4 s** → last-5 **36.2 s** — only **+27%** while the resident chain grew **~14×**
  (0.5M → 7M). Strongly **sublinear in chain length**: commit cost tracks *layer size*, not
  chain depth → the O(chain) fixes hold at 27× scale. (A surviving O(chain) cost would make
  chunk 22 ~14× slower, not 1.27×.)
- **Residual (minor, OPEN)**: the +27% slope — likely index-insert / persist cost / D23 cache
  pressure growing with total resident data, not a per-commit scan. Optimization, not a
  regression. Also: per-320k-chunk commit is ~33 s (~9k resources/s) — dominated by
  per-resource structural validation + index population + persist; revisit if it matters.

Two readiness gaps this load surfaced and fixed: `provision-wordnet.sh` passed a bare
endpoint (no `http://`) → transport error; `wordnet-import` had no partitioned emit. Both
provision scripts now use `--out-dir` + chained load on the `--endpoint` path.

### Retroactive carrier scan — the OOM the 7.6M chain exposed (and the Rule-12 hole behind it)

Running a notebook cell that **declares a property** against the 7.6M-resource chain
**SIGKILL'd the kernel** (exit 137, host OOM) in `commit.retroactive`. Root cause:
`scan_chain_for_property_carriers` (retroactive case 2) called `Layer::iter_all_resources()`,
which materialises **every resource body in the chain into one `BTreeMap`** (multi-GB at
7.6M) to find lower-layer resources carrying the newly-declared property. It ran for
**brand-new** properties, justified by an *open-world property-key hole*: Rule 12 admitted
property keys that resolve to nothing, and Rule 22 skipped them (`unknown property — not our
concern`), so a lower layer could in principle carry an undeclared key that a later
declaration must retroactively validate.

**Fix (two coupled changes):**
1. **Close the hole — `reference_integrity.rs` Rule 22 §(c):** every property *key* must
   resolve to a declared `core:Property` same-or-lower at commit (the committing layer
   counts, so intra-layer declarations are fine). Open-world (Rule 12) still frees a
   resource from its classes' `requires`/`recommends` sets — it does **not** permit an
   undeclared property IRI as a key. This makes "a layer only writes property keys known
   at its commit" a real, enforced invariant.
2. **Gate the scan — `retroactive.rs`:** case (2) now runs only for `redefines` (like
   cases 1/3). With keys-declared enforced, a brand-new property was unwritable in any
   lower layer ⇒ no lower carriers ⇒ no scan. Additive imports (and notebook cells
   declaring new properties) do **zero** chain scans; only the rare redefinition pays.

**Fallout fixed:** closing the hole revealed **4** core vocab gaps — inductive/macro
declaration fields used as keys but never declared as `core:Property`: `result_sort`,
`indices`, `ctor_type`, `macro_decl_json` (now declared in `core-ontology.json`). And
~47 `lattice`/`gc` unit tests validated **coreless** layer fragments (no core in chain →
`is_a`/`description` keys don't resolve); migrated to build on real core (lattice: a
committed core base; gc: a self-contained core-carrying root, count-neutral). Per the
chosen approach (build on real core, not an exemption list) the enforcement is exercised
by the tests so the hole can't silently reopen. Full kernel + workspace suites green.

## EigenQL query — what was fixed vs still open

**Fixed — per-query institution-rebuild floor** (`type_check.rs`): every query's
`type_check` called `InstitutionIndex::from_layer(layer)` — the full-chain-scan version —
to build the index for FIBER / qualified-call checks. That was a ~3.5s per-query floor on
the UMLS chain, hidden *inside* a helper call (not a direct `iter_all_resources`, so it
didn't show in the grep — a reminder that O(chain) hides behind helpers like `from_layer`).
Switched to `from_layer_indexed` (index-driven; the query head is stored, so the triple
index covers it; equivalence-tested). Floor gone (3.5s → ~0).

**Fixed — subject-predicate pushdown** (`evaluate/pattern.rs`): an untyped `MATCH ?r {}`
no longer scans + materialises the whole chain when a top-level WHERE conjunct constrains
the subject. `extract_subject_constraint` recognises `?r LIKE "p%"` (pure trailing-`%`
prefix), `?r = "iri"`, and `?r IN [...]`; `collect_candidates_via_subject` then gathers
candidates by IRI — a prefix-filtered walk of `defined_iris` (metadata, no body paging)
for `LIKE`, or direct `resolve` for `=`/`IN`. The WHERE re-applies every condition, so
pushdown only ever pre-filters (never drops a row); proven by
`subject_pushdown_equals_scan` (pushdown result == full-scan+filter). Not applied to
negated patterns (would change `NOT` semantics).

**Still open (lower priority — not on the patent demo's hot path):**
- **Untyped `MATCH ?r {}` with a non-subject or non-pushable WHERE** still falls back to
  the full scan (`collect_candidates_via_scan`). Could push down property predicates via
  the value/triple index when the brace carries a concrete predicate.
- **Large subclass trees** — a typed `MATCH ?x : C` where `C` has a huge subclass tree
  (e.g. a UMLS semantic type ≈ 50k concept-class subclasses) does
  `class_with_subclass_closure` + one `scan_chain(is_a, sub)` per subclass → O(subclasses).
  Wants a transitive `subclass_of` index, or accept the inherently large result.
- **Per-candidate property clone** in the indexed path (`r.properties().clone()` per row)
  — project only the requested properties.

## Short-name resolution — ✅ FIXED (`USING NAMESPACE` + implicit core)

Was: `resolve_name` (`pattern.rs`), `resolve_short_name_to_query_class` +
`resolve_property_name` (`type_check.rs`), and `resolve_property_name`
(`evaluate/similarity.rs`) resolved a **bare short name** by `iter_all_resources`
full-scan, at both compile and execute, once per reference — O(chain) × refs.

**Fix (design + impl):**
- **Grammar:** new `USING NAMESPACE "<prefix>"` clause (lexer `Namespace` token; parser
  `parse_using_namespace`; AST `MatchPart.using_namespaces: Vec<String>`). Declares the
  vocabulary namespaces a query's bare short names resolve within.
- **Implicit core:** `wk::CORE_NAMESPACE` (`urn:eigenius:core:`) is **always in scope** —
  core is the root layer on every chain, so its vocabulary (`Class`, `Property`,
  `short_name`, `domain`, …) is the prelude; no `USING NAMESPACE` needed for core terms.
  Non-core short names require an explicit `USING NAMESPACE`.
- **Resolver:** `crate::query::resolve::resolve_scoped_name(layer, namespaces,
  metaclasses, short)` — index-driven via the new `crate::layer::typed_resource_iris`
  (discovers candidate IRIs of a metaclass from the triple index / pending **without
  resolving bodies**), filters candidates by namespace prefix **first**, then resolves
  only the survivors. Cost = O(imported-namespace vocab), independent of chain size. A
  global `short_name` value index was rejected (would index 281k UMLS CUIs + risk false
  matches).
- **Ambiguity = error** (`ambiguous_short_name`): a short name matching >1 imported-namespace
  resource fails rather than first-wins. **Fail closed:** an unresolvable short-name
  *pattern class* is a type-check error (`unknown_class`) — no silent degradation to an
  untyped match-all (which would be slower AND drop the class filter). DEFINE relation
  names are exempt (they reference a derived relation, not a chain class).

In-repo short-name queries were migrated to declare their non-core namespaces: kernel +
demo tests, plus the executable demo/notebook artifacts — `demo/symbolics/run.sh` (FIBER
short query-class), `notebooks/src/runtime/publishedNotebooks.ts` (`MATCH Notebook`), and
`notebooks/examples/kinase-institutions.json` (10 cells: `AssayResult`/`Compound` →
`urn:eigenius:demo:assay:`, `OptimisesTo` → `urn:eigenius:jump:`). Other demo `run.sh`
scripts already use full IRIs (no change). Tests: `query::resolve::tests::*`
(scoped/ambiguous/fail-closed/implicit-core), `query::parser::tests::using_namespace_parses`.

**Deferred (clean extension, not a half-feature):** `USING NAMESPACE "<prefix>" AS <p>`
+ CURIE `p:local` names — the AST/parser leave room; not needed yet.

## Next actions (priority order)

1. ~~ESL compile~~ ✅ done.
2. ~~EigenQL untyped-match-all subject pushdown~~ ✅ done.
3. ~~Short-name resolution scoping~~ ✅ done (`USING NAMESPACE` + implicit core; see section above).
4. **`inspect`/health count**: sum `LayerHandle.resource_count` instead of materialising.
5. EigenQL secondary items (property-predicate pushdown for untyped patterns; transitive
   subclass index for large class trees; per-candidate projection).
6. Review the 🔎 sites under real demo workloads.
