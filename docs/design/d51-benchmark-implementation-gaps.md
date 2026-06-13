# D51 — Benchmark Implementation Gaps

*Status: implementation-planning memo · June 2026 · **status-reviewed 2026-06-11***

*Companion to [D50 benchmark evaluation approach](d50-benchmark-evaluation-approach.md). This memo enumerates the implementation work that must close before D50's pilot can be scheduled. Each gap is named, sized roughly, and located in the codebase. Items are ordered along the critical path: items earlier in the list block items later in the list.*

*Companion design documents the gaps consume: [D39 v2 justification logic](d39-justification-logic.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D48 indexed inductive families](d48-indexed-inductive-families.md), [D14 institution realisation](d14-institution-realisation.md).*

---

## 0. Status review (2026-06-11)

The kernel/ontology critical path is **closed**. Gap 1 (D49 ChainWitness machinery) and gap 3 (the D39 v2 Reasoning institution) landed in `#76 Justification logic` (be687e3); the witness emitter, the `JustifiedBy` / `ReasoningSentence` inductives, the `eigenius-reasoning` crate, and the `ValidateJustification` AutoOnLoad gate are all in the tree and tested. The remaining critical path is **experimental infrastructure** (gaps 4–8) plus the deferred Lean direction (gap 2).

| # | Gap | Status | What remains |
|---|---|---|---|
| 1 | D49 `ChainWitness` machinery (excl. Lean) | ✅ **Done** | — |
| 2 | Lean → Reasoning comorphism + `VerifiedPropositionView` | 🟡 **Partial** | View class + `IsVerifiedAs` lookup/coercion exist; the comorphism resource, the `lean_to_reasoning` transform, the `VerificationTrace` emit branch, and tests are outstanding (deferred to "Phase 8" in `reasoning.esl`). |
| 3 | D39 v2 institutional artifacts | ✅ **Done** | `ValidateJustification` is load-bearing; `EntailmentQuery` is v1 lookup-only and `ConsistencyCheck` is a v1 stub — honest dispatch-bound placeholders, not gaps in the institutional surface. |
| 4 | MCP surface extensions | ❌ **Not started** | `eigenius_load` `format` param + `eigenius_institution_dispatch` tool. |
| 5 | Base ontologies (`bench-core` + data-shape modules) | ❌ **Not started** | `experiments/` tree does not exist. **Reshaped to a `bench-core` spine + `mol`/`materials`/`singlecell` modules** — see §6. |
| 6 | Agent skill update | 🟡 **Partial** | `.claude/skills/eigenius.md` exists as a generic platform guide; the reasoning-discipline sections + worked model-then-reason example are unwritten. |
| 7 | Three-condition benchmark harness | ❌ **Not started** | **Now scoped to ScienceAgentBench chem+bio only** — see §8. `benchmark:TaskOutput` is not yet declared anywhere. |
| 8 | Per-pilot-task wiring | ❌ **Not started** | **Now scoped to 8 SAB tasks** — see §9. Reference repos (`references/ScienceAgentBench/`, `references/EngiBench/`) are present. |

**Pilot scope narrowed (2026-06-11): chem + bio.** The pilot is now the 4 computational-chemistry + 4 bioinformatics ScienceAgentBench tasks (8 total, 2 base ontologies). GIS, psychology, and all of EngiBench move to the scale-up tail. This is reflected below in gaps 5, 7, and 8, and in D50 §3/§4/§7. The deferred families' design content is retained — the narrowing is a scheduling decision, not a deletion of the broader design.

The original per-gap build-site inventories below are **preserved as the design record**; each now carries a `**Status (2026-06-11):**` line reconciling it against the code, including the handful of intentional deviations from the original plan (ESL rather than JSON authoring; registration via `cli/src/main.rs`; the D39 v2 `SpecStr` extension to `JustificationTerm`).

---

## 1. The critical path at a glance

Eight gaps, ordered top-to-bottom by dependency. Items 1–4 are kernel / institutional work; items 5–8 are experimental-infrastructure work. The kernel work must land before the infrastructure work can be exercised end-to-end, but the experimental infrastructure can be drafted (file layouts, scoring scripts, base-ontology authoring) in parallel with kernel work. The `Status` column reflects the 2026-06-11 review (§0); effort estimates are the *remaining* effort.

| # | Gap | Type | Status | Remaining effort | Blocked by |
|---|---|---|---|---|---|
| 1 | D49 `ChainWitness` machinery — witness table, synthesis, trace dispatch (excl. Lean) | Kernel | ✅ Done | — | nothing |
| 2 | Lean → Reasoning comorphism + `VerifiedPropositionView` class (D49 §7) | Ontology + Lean worker | 🟡 Partial | ~1 week | (1) |
| 3 | D39 v2 institutional artifacts — ontologies (`JustifiedBy`, `ReasoningSentence`, `Asserts(iri)`, `canonical_proposition`) + `crates/eigenius-reasoning/` (new crate parallel to `eigenius-lean`). The benchmark-scoped `TaskOutput` class lives with the harness (gap 7), not in the reasoning ontology. | Ontology + new crate | ✅ Done | — | (1) |
| 4 | MCP surface extensions — `eigenius_load` ESL parameter, `eigenius_institution_dispatch` generic tool | Orchestrator | ❌ Not started | ~0.5 weeks | (3) |
| 5 | Base ontologies — **`bench-core` spine + `mol`/`materials`/`singlecell`** (gis/psych/mfg/opt deferred) | ESL authoring | ❌ Not started | ~1 day | (3) |
| 6 | Agent skill update for the model-then-reason discipline | Documentation + worked examples | 🟡 Partial | ~1 week | (3), (4), (5) |
| 7 | Three-condition benchmark harness — **ScienceAgentBench only** for the pilot | Experimental infrastructure | ❌ Not started | ~1.5 weeks | (4), (5), (6) |
| 8 | Per-pilot-task wiring — **8 SAB tasks**; task fetching, eval-script integration | Experimental infrastructure | ❌ Not started | ~3 days | (7) |

**Remaining total**: ~4 working weeks if serialised, less with parallelisation. The kernel bottleneck (gaps 1–3) is cleared; what remains is the orchestrator extension (gap 4), the chem+bio base ontologies (gap 5), the skill discipline sections (gap 6), the SAB-only harness (gap 7), and the 8-task wiring (gap 8). Gaps 4–6 can proceed in parallel against the now-stable D39 surface; gap 2 (the Lean direction) is off the pilot critical path — it only matters once a pilot task needs a `JustifiedBy.verified` warrant, which the chem+bio subset does not.

The rest of this memo covers each gap in turn: what needs to be built, where it lives in the tree, what it depends on, and the design references that already specify the shape.

## 2. Gap 1 — D49 `ChainWitness` machinery (excl. Lean)

**Status (2026-06-11): ✅ Done** (landed in `#76`, be687e3). Every build site below is in the tree:
- `kernel/src/layer/witness_index.rs` — `WitnessKey { category, iri, prop_hash }`, `WitnessCategory` (4 variants), `build_witness_index`, `lookup_chain_witness` (parent-chain walk), `synthesize_chain_witness` (with the D49 §5 diagnostic), the D39 §4.1 `Asserts(iri)` default-proposition helper, and the D49 §4 `IsVerifiedAs → IsDerivedAs` lookup-time coercion. 10 unit tests.
- `kernel/src/layer/mod.rs` — `OnceLock<BTreeMap<WitnessKey, ()>>` on `Layer`, lazily built via `chain_witness_index()`.
- `kernel/src/nbe/val.rs` — `Val::ChainWitness(WitnessKey)` opaque variant, key-based definitional equality.
- `kernel/src/nbe/check.rs` — witness synthesis hook (`try_synthesize_chain_witness`) over the four `Is*As` predicates at `JustifiedBy.*` constructor check time; elidable witness slots.
- `kernel/src/ontology/well_known.rs` — `CANONICAL_PROPOSITION` and `ASSERTS` IRI constants plus the three trace-class constants.
- `kernel/src/validation/rules/canonical_proposition.rs` — Rule 20 validates `reflection:canonical_proposition` through the D47 codec at commit; 3 tests.

The only deferred sub-item is the `VerificationTrace` emit branch (the `IsVerifiedAs` *producer*), which D51 itself assigns to gap 2's end-to-end wiring; the `WitnessCategory::Verified` slot, the lookup hook, and the coercion are already present, so gap 1's non-Lean scope is complete.

**Specified in**: D49 §3-§6 (table location, witness key, synthesis algorithm, trace-emission dispatch for the three non-Lean witness families).

**Build sites**:

- `kernel/src/layer/witness_index.rs` (new) — `WitnessKey` struct (`category` × `iri` × `prop_hash`), `BTreeMap<WitnessKey, ()>` materialised per `Layer`, `build_witness_index(&Layer)` builder, `OnceLock` for lazy construction.
- `kernel/src/layer/mod.rs` — wire `OnceLock<BTreeMap<WitnessKey, ()>>` into `Layer`; expose `lookup_chain_witness(&Layer, &WitnessKey) -> bool` walking the parent chain.
- `kernel/src/nbe/val.rs` — add `Val::ChainWitness { key: WitnessKey }` variant per D49 §8.
- `kernel/src/nbe/check.rs` — when type-checking a `JustifiedBy.declared` / `.observed` / `.derived` constructor, synthesise the witness via `lookup_chain_witness`; on miss, emit `TypeError::NoAdmittedChainWitness { … }` with the diagnostic shape D49 §5 specifies.
- `kernel/src/ontology/well_known.rs` — add the `reflection:canonical_proposition` IRI constant.
- `kernel/src/validation/rules/` — extend the validator with the per-resource `canonical_proposition` type-check at `Prop`.

**Test surface**: hand-built `Layer` carrying mock `DeclarationTrace` / `ObservationTrace` / `ProgramTrace` resources; smoke-test the synthesis algorithm catches the witness, the negative diagnostic on misses, the parent-chain walk admits an ancestor's witness for a descendant Layer.

**Not in scope for this gap**: the Lean institution's `IsVerifiedAs` path (gap 2); the `JustifiedBy` inductive's authoring as a chain artifact (gap 3, since `JustifiedBy` is itself a `data` declaration that consumes D49's machinery).

## 3. Gap 2 — Lean → Reasoning comorphism + `VerifiedPropositionView`

**Status (2026-06-11): 🟡 Partial — off the pilot critical path.** What exists:
- `ontologies/reasoning/reasoning.esl` — `reasoning:VerifiedPropositionView : reflection:DerivedResource` is declared (requires `source_verified_resource` + `reflection:canonical_proposition`), ready to receive comorphism-reified views. The file comment notes "Phase 8 wires the comorphism; this declaration goes ahead of it."
- `kernel/src/layer/witness_index.rs` / `nbe/check.rs` — the `IsVerifiedAs` lookup hook and the `IsVerifiedAs → IsDerivedAs` coercion are wired; no further kernel change is needed on the *consumer* side once the producer lands.

What remains (the load-bearing pieces, all deferred to "Phase 8"):
- The `lean_to_reasoning` comorphism *resource* declaration (source `lean:LeanProofTerm`, target `reasoning:VerifiedPropositionView`, `AutoOnLoad`). Note: `reasoning.esl` §330 defers the *Reasoning → Lean* direction to gh #73 pending a real D30 term/type translation; the *Lean → Reasoning* direction here is the one gap 2 needs.
- `crates/eigenius-lean-worker/src/lean_to_reasoning.rs` — the inverse-D30 transformation. Does not exist (the crate's `src/` holds only `lean_project.rs`, `lean_ffi.rs`, `lean_sys.rs`, `lib.rs`).
- The `VerificationTrace` branch of the witness emitter in `witness_index.rs` (reads `canonical_proposition` off the reified view). The file explicitly marks this "deferred to the Phase-7 / D49 §7 integration."
- The 2+2=4 round-trip test plus the universe-polymorphism negative test.

**Pilot relevance**: none of the 8 chem+bio SAB tasks need a `JustifiedBy.verified` warrant (those warrant Lean-proved propositions), so gap 2 does **not** block the chem+bio pilot. It is required only for the four-gate concrete demo (D50 §9) and any future task family that cites a Lean verdict. Recommend completing it in parallel with, not ahead of, gaps 4–8.

**Specified in**: D49 §7 (comorphism-reify pattern; no new D14 trait surface).

This gap intentionally adds *no kernel trait surface* — the cross-institution translation rides on D14's existing comorphism machinery and chain-reinsertion path (D14 §9.3 step 4). An earlier draft of D49 introduced a new `Institution::export_proposition` trait method; that shape was over-engineered and was dropped in favour of the comorphism pattern. The build sites below reflect the current design.

**Build sites**:

- `ontologies/reasoning/reasoning-ontology.json` (in the same authoring pass as gap 3) — declare the `reasoning:VerifiedPropositionView` class. `is_a [reflection:DerivedResource]`; requires `reasoning:source_verified_resource` (IRI of the user-authored `VerifiedResource`) and `reflection:canonical_proposition` (D47-encoded EigenTT `Prop` term). The view's `derivation` invariant is satisfied by the comorphism's reify trace.
- `ontologies/lean/lean-ontology.json` (or wherever existing Lean comorphisms are declared) — declare the `lean_to_reasoning` comorphism per D14 §3-§5. Source class: `lean:LeanProofTerm`. Target class: `reasoning:VerifiedPropositionView`. Transformation: a reference to the inverse-D30 transformation Component (below). Dispatch role: `AutoOnLoad` on `lean:LeanProofTerm` commits. `exact: false` — not faithful for the full Lean fragment.
- `crates/eigenius-lean-worker/src/lean_to_reasoning.rs` (new) — the comorphism's transformation implementation. Reads the chain-mirrored `lean:LeanExpr` proposition from the source `VerifiedResource`, runs the inverse of D30's forward translation on the trivially-mappable `Prop` fragment, returns the EigenTT `Exp` as the comorphism's typed payload to be reified. Propositions outside the v1 fragment (universe polymorphism, Lean-specific definitional unfolding rules not mirrored in EigenTT) cause the transformation to fail with a `Verdict::Fails` whose diagnostic names the inexpressible feature — the reify step does not commit a view, and no `IsVerifiedAs` witness becomes admissible.
- `kernel/src/layer/witness_index.rs` — the `VerificationTrace` branch of the witness emitter (gap 1) reads `canonical_proposition` from the *reified* `VerifiedPropositionView` (looked up by `source_verified_resource = trace.resource`) rather than from the user-authored VerifiedResource. **No special dispatch path** — the same code that reads the property for `IsDeclaredAs` / `IsObservedAs` / `IsDerivedAs` reads it for `IsVerifiedAs`, just from a different chain resource. This branch should land as part of gap 1's witness-emitter implementation; gap 2 makes it work end-to-end by providing the comorphism that produces the view.

**Test surface**: a hand-authored `VerifiedResource` with a small Lean proof (e.g., `2 + 2 = 4` in Nat). Confirm: (a) on commit, the Lean → Reasoning comorphism's AutoOnLoad fires and reifies a `VerifiedPropositionView` with the EigenTT-form proposition; (b) the witness `IsVerifiedAs iri (Eq Nat (2+2) 4)` is admissible at the next type-check; (c) a separate `VerifiedResource` whose proposition uses universe polymorphism fails the comorphism reify with a diagnostic, no view is committed, and the witness is correctly absent. The diagnostic surfaces both at comorphism-dispatch time (a Verdict resource) and at downstream `JustifiedBy.verified` type-check time (the witness lookup misses with a hint pointing back at the Verdict).

**Independent of gap 3** in principle (the comorphism produces the view as soon as gap 1's witness emitter is in place; the absence of `JustifiedBy.verified` consumers just means no one looks up the witness yet). Easier to land *after* gap 3 because the `JustifiedBy.verified` consumer needed for end-to-end testing exists only once gap 3 has authored the `JustifiedBy` inductive.

**Why no kernel trait extension**: the inverse-D30 transformation is a pure function over `lean:LeanExpr` returning an `Exp`. Wrapping it in the comorphism transformation pattern (where comorphisms are *declared* as ontology resources and the transformation is the source-export step) reuses D14's commit-time AutoOnLoad dispatch, its content-addressed reify, its diagnostic shape, and its query-class registration without writing any new trait or dispatch code. The Reasoning institution does not call into the Lean institution directly — it consumes a chain resource the comorphism committed.

## 4. Gap 3 — D39 v2 institutional artifacts

**Status (2026-06-11): ✅ Done** (landed in `#76`, be687e3). The institutional surface is in the tree and tested, with a few intentional deviations from the original build-site list below:
- **`ontologies/reasoning/reasoning.esl`** (authored as ESL, *not* `reasoning-ontology.json` — the bootstrap loader uses the ESL surface so a content-hash of the file drives manifest-drift detection). Declares: `JustificationTerm` with **7** ctors (the planned 4 groundings + `App` + `Sum`, plus a D39 v2 addition `SpecStr` for universal specialization); `JustifiedBy` with **8** ctors (`declared`/`observed`/`derived`/`verified` + `app` + `sum_l`/`sum_r` + `spec_str`); the four zero-ctor `witness:Is*As` predicates; `ReasoningSentence : reflection:DerivedResource` (`proposition`, `justification`, `certificate`, + recommended `subject_iri`, `refutes`); the Reasoning `institution:Institution` resource; and the three query classes (`ValidateJustification` AutoOnLoad, `EntailmentQuery` OnDemand, `ConsistencyCheck` Decidable).
- **`ontologies/core/core-ontology.json`** — `Asserts(iri) : Prop` (zero-ctor inductive) and `reflection:canonical_proposition` as an optional property on the three reflection resource classes.
- **`kernel/src/bootstrap/mod.rs`** — reasoning is wired as a bootstrap layer parent (between the Lean layers and statistics); `embedded_ontologies()` is now `[…; 14]`.
- **`crates/eigenius-reasoning/`** — `lib.rs`, `institution.rs` (`ReasoningInstitution` + `impl Institution`), `extract.rs`, `validate.rs`, `entailment.rs`, `consistency.rs`, `startup.rs`. No `reify.rs` (v1 declares no `ImportFormat`; the trait method returns `NotImplemented`) and no `chain_mirror.rs` (the inductives ride the existing kernel decode path) — both omissions are correct per the original "not needed" notes.
- **Registration**: via the in-process institution list in **`cli/src/main.rs`** (`eigenius_reasoning::ReasoningInstitution::arc()`), not `kernel/src/capability/registration.rs` as originally sketched — same chain-scan auto-registration shape the Lean and Statistics institutions use.
- **Tests**: `tests/validate_handler.rs`, `tests/drug_screening.rs` (end-to-end axiom + measurement + derived claim + sentence citing them via `App(DeclaredEvidence, DerivedEvidence)`, validated to `Verdict::Holds`), `tests/universal_rule.rs` (the `SpecStr`/`spec_str` universal-application path).

**Maturity caveat (not a gap in the surface, but load-bearing for D50's metrics):** only `ValidateJustification` is fully implemented. `EntailmentQuery` (`entailment.rs`) is **v1 lookup-only** — it matches a candidate against already-committed `ReasoningSentence` propositions (syntactic `Exp` equality) and returns `Undecidable` on a miss; it does *not* do the spec's bounded-depth `JustificationTerm` proof search. `ConsistencyCheck` (`consistency.rs`) is a **v1 stub** — `Holds` on the empty set, `Undecidable` otherwise; the decision procedure is unimplemented. Both are dispatch-bound with reserved input shapes, so callers and the MCP surface (gap 4) can target them now, but D50 §6.2's "before committing, check whether the chain already entails it" agent guidance will only catch the exact-restatement case until the entailment search is built out. Track this as Reasoning-institution follow-on, sequenced by whether Phase 0 shows the agent actually reaching for entailment/consistency.

**Specified in**: D39 v2 §3–§5. (The `TaskOutput` class previously specified in D39 §4.4 was relocated to D50 §5b on review — it is benchmark-scoped, not Reasoning-scoped. Its build moved to gap 7, the benchmark harness.)

**Build sites**:

- `ontologies/reasoning/reasoning-ontology.json` (new) — declares:
  - `JustificationTerm` indexed inductive (6 ctors per D39 §3: 4 groundings + `App` + `Sum`). Authored using the eigenius#72 Layer 2 ESL surface (`data` with indices, typed ctors).
  - `JustifiedBy` indexed inductive over `(JustificationTerm × Prop)` with 6 ctors per D39 §5 (`declared` / `observed` / `derived` / `verified` consuming `ChainWitness` witnesses + `app` / `sum_l` / `sum_r` composition). Same surface.
  - `ReasoningSentence` Resource class. `is_a: [reflection:DerivedResource, reasoning:ReasoningSentence]` per the D39 §4.2 update. Property declarations: `proposition`, `justification`, `certificate`, `subject_iri` (with index hint), `refutes` (optional). The `derivation` invariant from `DerivedResource` is satisfied by pointing at the `certificate`.
  - (`TaskOutput` was previously listed here per D39 §4.4. It has been relocated to D50 §5b — it is benchmark-scoped, not Reasoning-scoped — and now lives with the harness in gap 7.)
  - The Reasoning institution declaration (`institution:Institution` resource) with `extract_typed` / `reify` shapes and three query class declarations (`ValidateJustification` AutoOnLoad, `EntailmentQuery` OnDemand, `ConsistencyCheck` Decidable).
- `ontologies/core/core-ontology.json` — add `Asserts(iri) : Prop` declaration (uniform-parameter no-ctor inductive in `Sort(0)`) per D39 §4.1. Also add `reflection:canonical_proposition` as an optional property on `DeclaredResource` / `ObservedResource` / `DerivedResource` (the latter two carry it as a forward-compat property even when not yet authored on most resources).
- `kernel/src/bootstrap/mod.rs` — add the reasoning ontology as a new bootstrap layer parent (after `core`, `program`, `reflection`, `institution`, and the `eigentt-type-fragment` layer). Update `embedded_ontologies` count.
- `crates/eigenius-reasoning/` (new crate, parallel to `crates/eigenius-lean/`) — the Reasoning institution's `Institution` trait implementation. Single crate (no worker / runtime sub-crates needed) because the validator IS the kernel's NbE checker and there's no external runtime. Cargo deps: `eigenius-kernel` (for the `Institution` trait + `Resource` / `Layer` / `Val` / `Exp` / NbE checker types) plus the usual workspace utilities. File layout mirrors `crates/eigenius-lean/src/`:
  - `lib.rs` — top-level exports.
  - `institution.rs` — `ReasoningInstitution` struct + `impl Institution` wiring.
  - `extract.rs` — `extract_typed`: decode `ReasoningSentence` resource → `JustifiedBy J P` typed payload via the D47 codec.
  - `reify.rs` — the inverse.
  - `validate.rs` — `query(ValidateJustification, …)` handler: thin wrapper that type-checks the certificate against `JustifiedBy justification proposition` via the kernel's NbE checker; returns Verdict. Wired through D14's existing AutoOnLoad dispatch.
  - `entailment.rs` — `query(EntailmentQuery, …)` handler: given Γ and A, bounded-depth search for a `JustificationTerm` whose certificate type-checks; returns Verdict.
  - `consistency.rs` — `query(ConsistencyCheck, …)` handler: propositional-fragment consistency over the committed-sentence set.
  - `startup.rs` — chain-scan registration hook (parallel to `eigenius-lean/src/startup.rs`).

  No `chain_mirror.rs` (parallel to `eigenius-lean/src/chain_mirror.rs`) is needed because `JustificationTerm` and `JustifiedBy` are authored via the eigenius#72 Layer 2 ESL surface and decoded by existing kernel inductive machinery. No `checker.rs` is needed because there's no external term checker to delegate to — the validation runs through `eigenius-kernel`'s NbE machinery directly.

- `kernel/src/capability/registration.rs` — register the Reasoning institution at chain-scan time using the same auto-registration shape the Lean institution already uses (D14 §3, plus the existing in-kernel registration path that handles `eigenius-lean`).

**Test surface**: hand-authored `ReasoningSentence` resources with each `JustificationTerm` shape; confirm commit-time validation fires per D39 §4.3; confirm gate firings are recorded as `Verdict` resources alongside the sentences. End-to-end: a small chain (axiom + observed measurement + derived value + reasoning sentence citing them) round-trips through commit / lookup / EntailmentQuery.

## 5. Gap 4 — MCP surface extensions

**Status (2026-06-11): ❌ Not started.** `eigenius_load` exists (`orchestration/src/mcp/server.ts`) but takes no `format` parameter; the kernel client (`orchestration/src/client/kernel_client.ts`) hardcodes `CONTENT_TYPE_CBOR`. There is no `eigenius_institution_dispatch` tool — the current MCP surface is `eigenius_query`, `eigenius_inspect`, `eigenius_list_branches`, `eigenius_list_tags`, `eigenius_list_institutions`, `eigenius_get_schema`, `eigenius_layer_topology`, `eigenius_load`, `eigenius_validate_program`, `eigenius_run_program`, `eigenius_run_program_by_iri`, `eigenius_health`, `eigenius_list_tasks`, `eigenius_get_task_status`. This is the smallest remaining critical-path gap and unblocks the agent's access to `EntailmentQuery`/`ConsistencyCheck` (even in their v1 form — see gap 3).

**Specified in**: the conversation thread on `orchestration/src/mcp` review.

**Build sites**:

- `orchestration/src/mcp/server.ts` — extend `eigenius_load` (around line 281) with an optional `format: "json" | "esl"` parameter; thread through to `client.load(args.json, { … format })` which passes through to the kernel's existing `content_type` handling. ~30 lines.
- `orchestration/src/mcp/server.ts` — add a new `eigenius_institution_dispatch(institution_iri, query_class_iri, payload, branch?, atLayer?)` tool under the Explore group. Calls the kernel's institution-dispatch RPC (which already backs `eigenius_query`'s `FIBER` clause; the new tool exposes the standalone-dispatch path). ~40 lines.
- `orchestration/src/client.ts` (or wherever the `client.load` signature lives) — propagate the `format` parameter through the typed RPC surface.
- `proto/eigenius.proto` if a new RPC method is needed for standalone institution dispatch (likely it isn't — the existing surface that backs FIBER should suffice; verify before adding a new proto method).

**Test surface**: a Deno orchestration test that loads an ESL file via `eigenius_load(format: "esl")` and confirms the resulting chain layer matches the equivalent JSON-loaded layer; a second test that dispatches `EntailmentQuery` on a small Reasoning-institution chain and confirms the Verdict.

**Out of scope**: per-Reasoning-institution-query convenience MCP tools (e.g., `eigenius_check_entailment`). The generic `eigenius_institution_dispatch` covers them; convenience wrappers are added later if agent ergonomics in Phase 0 show the agent struggling with the institution-IRI / query-class-IRI parameters.

## 6. Gap 5 — Base ontologies: `bench-core` + data-shape modules (chem + bio pilot)

**Status (2026-06-12): ❌ Not started (authoring in progress).** The `experiments/` tree does not exist; no `bench:*` ESL is in the repo. **Reshaped 2026-06-12** from the original flat per-family `chem`/`bio` plan to a shared **`bench-core`** spine + thin **data-shape modules** (`mol`, `materials`, `singlecell`), after grounding against the eight pilot tasks (D50 §4; per-task sketches in `docs/notes/chem-bio-pilot-execution-plan.md`). Rationale: the typed-tool-boundary + `Measurement` + `Dataset` spine is domain-agnostic (factor it once, no drift), and the molecule nouns straddle SAB chem *and* bio (tasks 8/18), so modules cut by data shape rather than SAB domain label.

**Specified in**: D50 §4 (the module table) + D50 §9 / §12 (typed-tool-boundary workaround the spine encodes).

**Build sites** (pilot scope; authoring order `bench-core` + `mol` first — they carry the SAB 16 tracer):

- `experiments/benchmark/base-ontologies/bench-core.esl` (extends `reflection`):
  - `class bench:ToolArtifact : reflection:DerivedResource { requires bench:tool, bench:produced_from; }` — the typed tool boundary; every RDKit/pymatgen/sklearn output is one.
  - `class bench:Measurement : reflection:ObservedResource { requires bench:value, bench:unit, bench:quantity; }` — reuses the `statistics.esl` value+unit shape.
  - `class bench:Dataset : reflection:ObservedResource { requires bench:dataset_path; }` — input-data anchor.
  - `data bench:concerns : core:string -> core:string -> Prop {}` — shared linking-predicate convention (ties a tool result/measurement to the subject it is about; used when statistical-entity and domain-entity identities differ, as in the `sample_for` pattern of `stats-and-reasoning.json`).
- `experiments/benchmark/base-ontologies/mol.esl` (extends `bench-core`) — covers SAB 16, 17, 94, 8, 18: `mol:Compound` (`requires mol:smiles`), `mol:Fingerprint : bench:ToolArtifact`, `mol:ActivityMeasurement : bench:Measurement`, `mol:Target`.
- `experiments/benchmark/base-ontologies/materials.esl` (extends `bench-core`) — covers SAB 28: `materials:CrystalStructure`; density fields/differences/profiles are `bench:ToolArtifact`s.
- `experiments/benchmark/base-ontologies/singlecell.esl` (extends `bench-core`) — covers SAB 69, 98: `sc:Cell`, `sc:Gene`, `sc:ExpressionMatrix`, `sc:CellType`, `sc:ChainPairing`.
- *(optional `ml` facet on `bench-core` — `FeatureSet`, `Classifier`, `CVScore`, `Prediction` for SAB 8/18; fold into `mol` if it stays small.)*

Each module is 5-10 ESL declarations. Pilot authoring effort ~1 day; mine the patent demo + `statistics.esl` for the class/property and value+unit shapes.

**Deferred to scale-up** (further modules on `bench-core`, authored only if D50 §7's criteria are met): `gis.esl` (`SpatialFeature`, `RasterLayer`, `CRS`, `Buffer`, `Polygon`, `TemperatureSeries`, `Glacier`), `psych.esl` (`Signal`, `ECGRecord`, `HRVIndex`, `Subject`, `QuestionnaireResponse`, `ValidatedScore`), `mfg.esl` (`Component`, `Process`, `Decision`, `Cost`, `ConfidenceLevel`, `HypothesisTest`, `InspectionPolicy`, `DefectRate`), `opt.esl` (`Variable`, `Constraint`, `Objective`, `FeasibleRegion`, `Solution`, `ProbabilityModel`, `OptimizationProblem`).

**Quality check**: each base ontology must round-trip through the kernel's commit pipeline cleanly (no validator failures); each base must be loadable as a layer parent in the benchmark harness without conflicting with the bootstrap layers; each base's classes must support being subclassed by per-task agent vocabulary (test: hand-author a per-task vocabulary for one pilot task per family and confirm it commits cleanly on top of the base).

**Independent of kernel gaps 1-3** in principle (ESL authoring is supported today), but the agent's per-task vocabulary will need the eigenius#72 surface to author `axiom` declarations naturally — and the Reasoning institution's `ReasoningSentence` shape must exist before the agent can cite axioms in reasoning sentences. So gap 5 is *authoring-ready* now but *useful* only after gap 3.

## 7. Gap 6 — Agent skill update for the model-then-reason discipline

**Status (2026-06-11): 🟡 Partial.** `.claude/skills/eigenius.md` exists but is a generic platform guide (MCP tool surface, vocabulary, minimal shapes, workflow recipes, pitfalls). None of the reasoning-discipline sections below are written, and `docs/guides/platform/14-notebook.md` has no model-then-reason worked example (its three notebook examples are patent-analysis, kinase-institutions, lean-verification). Two housekeeping items surfaced in the review: the skill's tool list needs reconciling with the actual MCP surface once gap 4 lands, and the worked example should use a chem or bio pilot task so it doubles as the publication's introductory example under the narrowed scope.

**Specified in**: D39 §4.5 (two-phase agent surface), §6.4 (trade-off pattern), the agent-skill summary in the conversation thread on MCP review.

**Build sites**:

- `.claude/skills/eigenius.md` — extend the existing skill with:
  - **Section: "Reasoning loop overview"** — the two-phase discipline (vocabulary, then reasoning), why it matters, when it engages.
  - **Section: "Authoring vocabulary"** — patterns for `class` / `property` / `axiom` / indexed `data` declarations in ESL; common shapes per domain (chemistry, GIS, manufacturing) with worked examples; how to recover from validator failures on vocabulary commits (most common: malformed `requires` lists, mis-typed axiom statements).
  - **Section: "Authoring `ReasoningSentence`s"** — the canonical Eigon-JSON / ESL shape; how to construct `JustificationTerm`s for each of the four grounding patterns from D39 §6; the trade-off pattern from D39 §6.4 with a worked example.
  - **Section: "Querying past reasoning"** — three canonical EigenQL templates: "my conclusions about subject X", "what does sentence Y depend on", "what axioms / inference rules are in scope for predicate P". Each template is copy-pasteable.
  - **Section: "When to use `eigenius_institution_dispatch`"** — operational guidance for `EntailmentQuery` (before committing a derivative conclusion, check whether the chain already entails it) and `ConsistencyCheck` (before committing a contradicting sentence). Concrete examples.
  - **Section: "Recovery from commit failures"** — the kernel's diagnostic shape for the common failure modes (missing prior, ungrounded justification, ill-typed proposition, vocabulary error) and the canonical revise-and-retry pattern for each.
  - **Section: "Common anti-patterns"** — vacuous justifications (`DeclaredEvidence` citing the agent's own assertion as the only ground), predicate-name proliferation (10 ad-hoc predicates where 3 would do), trying to reason in untyped prose first and lift to ESL later.

- `docs/guides/platform/14-notebook.md` or a new chapter — a worked end-to-end example: one pilot task taken through the model-then-reason discipline, with the chain artifacts at each step shown. This doubles as the publication's introductory worked example and as the skill's reference example.

**Test surface**: the Phase 0 shakedown (D50 §7) is the test for this gap. If the three Phase 0 tasks run smoothly with the updated skill, the discipline is teachable; if the agent fights the surface or produces vacuous chains, the skill needs more work.

## 8. Gap 7 — Three-condition benchmark harness (ScienceAgentBench only)

**Status (2026-06-11): ❌ Not started.** `experiments/benchmark-harness/` does not exist; no runners, no scoring scripts, and `benchmark:TaskOutput` is not declared anywhere. `references/ScienceAgentBench/` and `references/EngiBench/` are present as reference trees. **Scoped 2026-06-11 to ScienceAgentBench only** — the chem+bio pilot draws entirely from SAB, so the EngiBench native-runner path and the LLM-judge scoring are dropped from the pilot harness and deferred to scale-up. This removes the harness's hardest single integration (the EngiBench LLM-judge with inter-judge calibration), which is the main reason the remaining effort drops from ~2 weeks to ~1.5.

**Specified in**: D50 §5 (harness architecture sketch).

**Build sites** (pilot scope):

- `experiments/benchmark-harness/` (new tree, separate from production code):
  - `harness-ontology.esl` — declares the benchmark-scoped `bench:TaskOutput` class + its properties (`task`, `deliverable_kind`, `payload`, `reasoning_chain`, `deliverable_resources`, required `reflection:derivation`) per D50 §5b. Loaded as a sibling layer to the per-family base ontologies; the Reasoning institution stays unaware of it. **✅ Authored 2026-06-12** at `experiments/benchmark/harness-ontology.esl` (ahead of the rest of the harness — it's the prerequisite for the corrected SAB-16 deliverable model); validated by the base-ontology smoke test. Path note: consolidated under `experiments/benchmark/` (this gap's tree), reconciling the earlier `experiments/benchmark-harness/` path. Follow-on: a commit-time validator rule that every `reasoning_chain` IRI resolves to a `ReasoningSentence`.
  - `conditions/baseline_runner.py` — wraps SAB's native agent (`agent.py` in `references/ScienceAgentBench/ScienceAgentBench_github/`). Produces the deliverable in the format the SAB eval script expects.
  - `conditions/cot_runner.py` — same agent, but with a chain-of-thought instruction added to the system prompt. Records the agent's reasoning trace in a separate field alongside the deliverable.
  - `conditions/eigenius_runner.py` — drives the Eigenius MCP surface for the structured-reasoning condition. Loads the per-family base ontology (chem or bio) as a layer parent, loads per-task vocabulary hints into the agent's context, runs the agent loop with access to the MCP tools (`eigenius_load`, `eigenius_query`, `eigenius_inspect`, `eigenius_institution_dispatch`), extracts the final `benchmark:TaskOutput.payload` as the deliverable.
  - `tasks/sab/<task-id>/{task.json, hints.esl}` — per-task config: task instruction, dataset path, eval script reference, vocabulary hints.
  - `scoring/sab_score.py` — wraps the SAB per-task eval scripts (which live under `references/ScienceAgentBench/ScienceAgentBench_github/evaluation/`); produces VER / SR / CBS for each (condition × task × replicate) triple.
  - `scoring/derived_metrics.py` — gate-firing tally, vocabulary size, reasoning chain depth, citation density, trade-off pattern usage (per D50 §6.3).
  - `runs/<run-id>/<condition>/<task>/<replicate>/` — per-cell run artifacts: the agent's transcript, the deliverable, the scoring output, the timing data, the Eigenius chain artifacts (for condition C).
  - `analyze/headline.py` — produces the per-condition table; runs significance tests; emits the publication-ready figures.

**Deferred to scale-up** (built only if D50 §7's scale-up criteria pull EngiBench back in): the EngiBench direct-prompt path inside the three runners, `tasks/engibench/<task-id>/{task.json, hints.esl}`, and `scoring/engibench_score.py` (the pinned LLM-judge rubric scorer with inter-judge calibration).

**Effort estimate**: ~1.5 weeks Python-side for the SAB-only harness, including the SAB native-runner integration. Shorter if the SAB agent runner can be wrapped without modification.

**Operational considerations**:

- Run conditions one at a time per task (not parallelised across conditions for one task), so the conditions don't compete for LLM API rate-limit budget.
- Run different tasks in parallel only if the LLM API rate limit allows; otherwise serialise.
- Hard per-task per-condition timeout: 30 minutes. Tasks that time out are reported separately, not treated as failures.
- All runs use the same LLM model / version, pinned in the harness config. Mid-pilot model upgrades are not allowed.

## 9. Gap 8 — Per-pilot-task wiring (8 SAB chem+bio tasks)

**Status (2026-06-11): ❌ Not started.** No `tasks/` configs exist. **Scoped 2026-06-11 to the 8 chem+bio SAB tasks** (chem: 16, 17, 28, 94; bio: 8, 18, 69, 98). EngiBench wiring and its LLM-judge calibration are deferred to scale-up — dropping the calibration is the main effort reduction here.

**Build sites** (pilot scope):

- For each of the 8 SAB tasks (chem 16/17/28/94, bio 8/18/69/98): confirm the eval script in `references/ScienceAgentBench/ScienceAgentBench_github/evaluation/` runs cleanly on the gold program; package the dataset for the harness; author the per-task vocabulary hints file (~5 suggested predicate names, subclassing the `chem` or `bio` base).
- One pilot dry-run: the Phase 0 shakedown (D50 §7) covers the operational test of this wiring.

**Deferred to scale-up**: the 11 EngiBench tasks' problem-statement packaging, hints, and the per-pilot LLM-judge calibration (2 problems cross-scored with a second judge family). Pulled back in only if D50 §7's scale-up criteria add EngiBench.

**Effort estimate**: ~3 days for the 8 SAB tasks (no LLM-judge calibration in the pilot scope).

## 10. Sequencing recommendation

**Revised 2026-06-11.** Gaps 1 and 3 (the kernel + Reasoning-institution critical path) are done. What remains is experimental infrastructure for the narrowed chem+bio pilot, plus the optional Lean direction (gap 2). The remaining sequence:

**Week 1** (orchestrator): Gap 4 (MCP extensions — `format` param + `eigenius_institution_dispatch`). Smallest gap; unblocks the agent's institution-dispatch access.
**Week 1** (parallel): Gap 5 (author `chem.esl` + `bio.esl` — ~1 day against the now-stable D39 surface). Gap 6 (draft the reasoning-discipline skill sections; the worked example uses a chem or bio pilot task once gap 4 lands).

**Week 2** (infrastructure): Gap 7 (SAB-only harness — declare `benchmark:TaskOutput` in `harness-ontology.esl` first, then the three runners + `sab_score.py` + `derived_metrics.py`). Gap 8 (wire the 8 SAB tasks).
**Week 2** (parallel, off critical path): Gap 2 (Lean → Reasoning comorphism + `lean_to_reasoning` transform + `VerificationTrace` emit branch). Needed only for the four-gate concrete demo, not the chem+bio pilot — schedule it whenever convenient.

**Diagnostic-quality buffer** (per §11): budget a few days after gap 7 lands to iterate on kernel diagnostics if Phase 0 shows the agent fighting the surface.

**Phase 0 shakedown** per D50 §7 (3 chem+bio tasks: SAB 16 shortest-chem, SAB 17 medium-chem, SAB 18 bio).

**Phase 1 full pilot** per D50 §7 (8 tasks × 3 conditions × 3 replicates = 72 runs). Wall-clock ~12 hours of agent time; calendar time depends on LLM rate limits.

Total calendar time to a publishable chem+bio pilot result: roughly 2–3 weeks of remaining infrastructure work, down from the original ~7 weeks now that the kernel path is cleared and the scope is narrowed. Scale-up to the deferred families (GIS, psychology, EngiBench, full SAB) follows D50 §7's scale-up criteria and pulls in the deferred build sites in gaps 5, 7, and 8.

## 11. Risks specific to the implementation work

These are the implementation-side risks. Architectural-soundness risks are in D49 §9 / D39 §10. Experimental-design risks are in D50 §8.

**D39's first-wave UX may need iteration before the agent loop works.** The agent's experience of kernel diagnostics determines whether the discipline is teachable or feels like fighting the system. The Phase 0 shakedown is supposed to surface this, but if the first-wave diagnostics are too cryptic for an LLM agent to act on, expect to spend a week on diagnostic-quality iteration before Phase 1. **Mitigation**: budget an explicit "diagnostic-quality iteration" buffer week between gap 3 landing and Phase 0 starting.

**The Lean → Reasoning comorphism transformation may be narrower than expected.** Gap 2 commits to the trivially-mappable `Prop` fragment of Lean. If the demo theorems (or future use cases) need universe polymorphism or Lean-specific definitional unfolding, gap 2's v1 transformation is insufficient and a v2 with broader coverage is needed. The v2 path is purely additive to the comorphism's transformation implementation — no architectural reshape required, since the comorphism is the right shape; just more cases handled in the transformation. **Mitigation**: pick the demo Lean theorems early (parallel to gap 2 implementation) and sanity-check they fall in the v1-mappable fragment before committing the time.

**The agent may need more than the canonical EigenQL templates for self-recall.** Phase 0 will reveal whether the three templates in the skill (gap 6) are enough or whether more sophisticated queries are needed. **Mitigation**: track which queries the agent reaches for during Phase 0 and add them to the canonical-template list before Phase 1.

**Base ontology drift between authoring and pilot use.** Gap 5 authors the six base ontologies up front; gap 3's eventual D39 surface may impose constraints (e.g., the `canonical_proposition` property shape, the exact `Asserts(iri)` declaration shape) that require revising the base ontologies. **Mitigation**: re-validate the base ontologies against the D39 surface as the final step of gap 3; treat any base-ontology revisions as part of gap 3's effort, not gap 5's re-work.

**MCP surface ergonomics with the generic dispatch tool.** Gap 4 chooses generic `eigenius_institution_dispatch` over per-query-class convenience tools. If the agent struggles to remember institution IRIs and query class IRIs, convenience wrappers may be needed. **Mitigation**: include "agent successfully invokes EntailmentQuery via the generic tool in ≥80% of Phase 0 attempts" as a Phase 0 success criterion; add convenience wrappers if the criterion fails.

**Wall-clock and token-cost overhead in condition C.** The discipline adds friction. If condition C wall-clock blows past 30 min per task on most pilot tasks, the agent is fighting the surface rather than using it productively. **Mitigation**: report per-task wall-clock as part of Phase 0 and treat outliers as failure modes to debug before Phase 1.

## 12. What's *not* in scope for this gap inventory

- **Soundness-tally measurement infrastructure.** D50 §1 reframes this as a secondary finding rather than the headline. Gap 7's `derived_metrics.py` covers what's actually needed (gate-firing tally is a per-run statistic, not a comparison-harness output).
- **Wider domain institutional coverage.** Geopandas / DeepChem / BioPsyKit institutions don't exist in Eigenius today and are not built for this pilot. The pilot works around this by treating each tool invocation as a typed Component the agent declares (input type, output type) and the kernel checks the boundary, not the internal computation. The Python-bridge typed-Component shape may need a small extension to D14 / D26 to be authored cleanly; this is in scope for gap 5 (the base ontologies define what typed-Component shapes are available for each domain family) but the production-quality Python bridge is *not* on the critical path.
- **The four-gate concrete demo** (drug-candidate, dock_to_assay, Lean verdict). Worked example for the publication's introduction, not benchmark infrastructure. Authored in parallel; not blocking the pilot.
- **EigenQL surface for `subject_iri` indexing.** D39 §4.2 declares `subject_iri` as a first-class query index; D23's per-class triple index should auto-cover it, but if Phase 0 shows the query is slow, kernel-side index hints may be needed. Treated as a follow-up rather than a critical-path item.

---

*This is an implementation-planning memo. The eight gaps, their dependencies, and the sequencing recommendation are the load-bearing decisions; the per-gap effort estimates are first-draft proposals expected to slip as the work progresses. The risks in §11 should be re-reviewed before each gap is started.*
