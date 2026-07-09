# NbE module analysis — inventory, reorganization, ancestry alignment

Working report (not a design doc). Basis for deciding the `kernel/src/nbe` reorganization
and ancestry consolidation. Plan: analysis first, review, then restructure.

- Branch: `nbe-cleanup`, HEAD `123087d09c6c75d76dd3079bee8f1cac13bceb88`, clean tree at analysis start.
- Reference pins (vendored clones, git-ignored): `references/nanoda_lib` @ `f58f2f6`,
  `references/miniagda` @ `b838f00` (v0.2025.7.23), `references/Mini-TT` @ `442b08f`.
- Status: analysis **complete** (§0 baseline, §1 inventory, §2 coupling, §3 reorganization
  proposal, §4 ancestry alignment, §5 ranked backlog). Soundness findings **F-1–F-4 fixed**
  on this branch (2026-07-07, backlog items 1–3); F-5 (trace-tree completeness) rides with
  the §3.2 evaluator consolidation. Restructuring (§5 items 4+) awaits joint review.

## 0. Baseline (grade: Derived — commands run 2026-07-07 at HEAD above)

| Check | Command | Result |
| --- | --- | --- |
| Build | `cargo build` | ok |
| Full test suite | `cargo test --workspace` | exit 0, no failures |
| Kernel unit tests | `cargo test -p eigenius-kernel --lib` | 1611 passed, 0 failed, 0 ignored |
| NbE unit tests | `cargo test -p eigenius-kernel --lib nbe::` | 333 passed, 0 failed |
| Formatting | `cargo fmt --all -- --check` | clean |
| Lint | `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` | exit 0 |

## 1. Inventory

### 1.1 Files (reproduce: `wc -l kernel/src/nbe/*.rs`; test counts: `grep -c '#[test]'`)

| File | Lines | `#[test]`s | `mod tests` at | Role |
| --- | --- | --- | --- | --- |
| check.rs | 6,701 | 159 | 2796 | Bidirectional type checker |
| eval.rs | 4,251 | 55 | 2353 | Evaluator + D14/IO dispatch |
| sized.rs | 1,047 | 42 | 559 | Size-constraint solver (Floyd–Warshall) |
| unify.rs | 735 | 17 | 463 | First-order pattern unification (D48) |
| term.rs | 735 | 8 | 652 | `Exp` syntax |
| recursor.rs | 600 | 9 | 268 | Recursor minor-premise derivation |
| val.rs | 580 | 5 | 541 | `Val`/`Neut`/`Clos` semantic domain |
| readback.rs | 533 | 16 | 369 | Quotation values → normal forms |
| positivity.rs | 455 | 7 | 250 | Strict-positivity check |
| sized_rigid.rs | 346 | 10 | 216 | Rigid-size hypothesis tracker (TSO) |
| env.rs | 216 | 5 | 174 | `Rho`/`Gamma` environments |
| mod.rs | 33 | 0 | — | Module root |
| **Total** | **16,232** | **333 run** | | |

check.rs is 58% tests (lines 2796–6701). eval.rs is 45% tests (2353–4251).

### 1.2 Cluster map (line anchors verified by `grep -nE '^(pub )?(fn|enum|struct) '`)

#### check.rs

| Lines | Cluster |
| --- | --- |
| 43–176 | `CheckCtx` (rho, gamma, layer, type_cache, size_tso, institution index/runtime) + constructors, `eval_ctx()`, `extend`, `resolve_class_cached` |
| 177–326 | `check_decl`, `check_type` |
| 327–689 | `check` — checking mode: core arms (Lam/Pi/Sig/Pair/Con/Case/Unit/One/Sort/Refl/Id/Dec), size arms, Eigenius arms (Codata/Inductive/Match/CoRecord/EigonResource) |
| 690–1143 | `check_infer` — inference mode: core (Var/Ann/App/Fst/Snd/…) + Eigenius (PropAccess/Observe/Construct/Template/NativeDecide/DecEq/IdJ/Map/Reduce/Lit*/…) |
| 1144–1231 | Codata resolution: `resolve_full_codata_decl`, `lookup_codata_observation` |
| 1232–1288 | `eq_nf` — definitional equality via readback |
| 1289–1373 | Large elimination: `large_elim_admitted`, `ctor_args_pass_singleton_b` |
| 1374–1460 | Syntax utilities: `exp_mentions_var`, `patt_binds`, `is_syntactically_propositional_type` |
| 1461–1558 | `def_eq_at_type`, `infer_dependent_sort`, propositionality (`is_propositional_in_ctx`, `_structural`) |
| 1559–1641 | Subtyping/cumulativity: `subtype_of`, `subtype_of_with_hyps` |
| 1642–1898 | Guardedness: `collect_pattern_names`, `has_forbidden_head`, `check_guarded` |
| 1899–1993 | Sigma/Pi helpers: `find_sigma_field`, `advance_sigma`, `ext_pi`, `CtorArg` |
| 1994–2186 | Indexed-inductive validation: `validate_indexed_ctor_conclusions`, `ctx_with_param_and_arg_binders`, `peel_ctor_telescope` |
| 2187–2249 | ChainWitness synthesis (D49): `try_synthesize_chain_witness`, `chain_witness_category_for_short_name` |
| 2250–2769 | Inductive checking: `check_inductive_ctor_args`, `check_infer_inductive_rec`, `check_match` |
| 2770–2795 | `ext_sig`, `extract_list_element_type` |
| 2796–6701 | tests (159; includes integration-style D14/Eigon tests importing the institution registry/runtime) |

#### eval.rs

| Lines | Cluster |
| --- | --- |
| 25–95 | `EvalError` |
| 96–190 | `EvalCtx` — `Pure`/`Read`/`IO`/`Check` |
| 191–677 | `eval` (thin, hard-codes `Pure`) + `eval_ctx` — the master match; core arms ≈196–330, Eigenius arms ≈330–677 |
| 678–713 | `match_dispatch` |
| 714–1111 | `eval_traced` — parallel evaluation match threading `Option<Trace>` (near-duplicate of `eval_ctx`, see §2.5) |
| 1112–1222 | `eval_map`, `eval_reduce` |
| 1223–1400 | Recursor runtime: `iota_reduce`, `extract_ctor_arg_types`, `is_recursive_arg_type`, `build_recursor_ih`, `deterministic_run_output_iri` |
| 1401–2262 | D14/IO dispatch engine (all private): `try_d14_institution_invoke` (1401), `dispatch_component` (1586), `now_millis`, `val_to_resource`, `resolve_component_schemas` (1858), `resource_payload` (1983), `decide_constraint` (2008), `try_d14_decide` (2093), `parse_verdict`, `ground_values_equal` |
| 2263–2352 | Resource⇄Val marshalling: `resource_value_to_val`, `val_to_resource_value` |
| 2353–4251 | tests (55) |

**Other files** (single-purpose; cluster = file): term.rs `Exp`/`Patt`/`Decl`/`Constraint`/
`PrimitiveType`/`InductiveDecl`/`CodataDecl` + builtin `list_decl`/`option_decl` builders;
val.rs `Val` (30–172), `Neut` (183–255), `Clos` + elimination methods (`app*`, `vfst`,
`vsnd`, `vobserve*`); env.rs `Rho`/`Gamma`/`gen_val`/`up_gamma`; readback.rs
`readback_val`/`readback_neut`; recursor.rs `derive_minor_types`; positivity.rs
`check_positivity`; unify.rs `MetaCtx`/`unify`/`solve_meta`/occurs-check/`zonk_val`;
sized.rs constraint graph + `solve`/`size_le*`; sized_rigid.rs `Tso`.

### 1.3 Core type theory vs Eigenius extensions

**Core TT** (would exist in any Mini-TT-descended kernel):

- Whole files: env.rs, readback.rs, recursor.rs, positivity.rs.
- term.rs/val.rs core arms (Lam/Sort/Pi/Sig/One/Unit/Pair/Con/Data/Case/Fst/Snd/App/Ann/Var/Dec; Gen/Meta/App/Fst/Snd/NtFun).
- eval.rs core arms (≈196–330); check.rs `check`/`check_infer` core arms, `eq_nf`,
  `def_eq_at_type`, `subtype_of*` (D46 `Sort(n)`, Prop=0).
- unify.rs (elaboration extension for D48 indexed families; standard machinery, no Eigenius coupling).
- sized.rs/sized_rigid.rs (MiniAgda ports; std-only, self-contained — the `SizeExpr`/`Rigid`
  abstraction is independent of `Val`).

**Eigenius extensions**:

- term.rs "Eigenius extensions" arms (line 70+): `Id`/`Refl`/`IdJ`, `NativeDecide`/`DecEq`,
  `EigonClass`/`EigonAxiom`/`EigonPrimitive`/`EigonResource`, `Lit*`, `PropAccess`,
  `Template`/`Construct`, Codata (D11), `Map`/`Reduce`, `Inductive*` (D19/D48), `Size*`/`SizedPi`,
  `InstitutionInvoke`, `Constraint::Institution`.
- val.rs counterparts + `ResourceVal`, `ChainWitness` (D49), `TemplateVal`, `List`.
- eval.rs: Eigenius arms of the match; the entire D14/IO dispatch cluster (1401–2262);
  marshalling (2263–2352); `eval_traced` (tracing is a D21/observability concern).
- check.rs: `CheckCtx` layer/institution fields, `resolve_class_cached`, ChainWitness
  synthesis (2187–2249), Eigon arms of `check`/`check_infer`.

Note: inductives, codata, and sized types are "extensions" relative to Mini-TT but are
standard type-theory features; only the Eigon/institution/witness/trace material is
platform-specific. The §3 layering question applies to the latter.

## 2. Coupling

### 2.1 Intra-nbe dependencies

Layering (each file's `use crate::nbe::` imports, production code):

```text
term ──> (leaf)
env ──> term, val
val ──> term, env, eval (EvalError/EvalCtx in method sigs)
eval ──> term, val, env
readback ──> term, val, env
recursor ──> term, val, env, eval, readback
positivity ──> term
sized ──> (std only)          sized_rigid ──> (std only)
unify ──> val, readback, check::eq_nf   ←──┐  cycle
check ──> env, eval, val, readback, recursor, unify ──┘
```

The one cycle: `unify.rs:61` imports `check::eq_nf` (rigid-rigid fallback); check.rs calls
`unify::unify` at 2442 (`check_inductive_ctor_args`) and 2749 (`check_match`), each with a
fresh `MetaCtx` — unification is used only for inductive-index equations.

### 2.2 Reverse coupling: nbe → rest of kernel (production code only)

Reproduce: per file, `awk 'NR<{mod tests line}'` then grep `crate::<module>::`.
Doc-comment-only references excluded (term.rs's `institution`/`program` mentions are
doc links only — verified at term.rs:96,264,382–383).

| nbe file | Couples to | Sites | What for |
| --- | --- | --- | --- |
| term.rs | `ontology::{iri,resource,well_known}` | `use` + option_decl | `Exp::EigonResource`, `EigonClass(Iri)`, well-known OPTION IRI |
| val.rs | `ontology::{iri,resource}`; `witness::WitnessKey` (163); `program::trace::Trace` (297, 399, 435) | 3 | `Val::ResourceVal`, `Val::ChainWitness`; traced-eval method signatures |
| eval.rs | `institution::{registry,runtime,DecResult,marshal,dispatch}`; `layer`; `program::{component,trace,schema}`; `task::{TaskContext,Checkpoint}`; `context::{ExecutionContext,ExecutionMode}` (1451–1454, 2152–2155); `observability`; `ontology::{iri,resource,well_known,eigon_cbor}` | ~50 | `EvalCtx::{IO,Check}` payloads + the D14/IO dispatch engine |
| check.rs | `layer::{Layer,synthesize_chain_witness}`; `institution::{registry,runtime}`; `program::ground::resolve_class_type` (168); `witness::WitnessCategory` (2239); `ontology::{iri,well_known,resource}` | ~12 | `CheckCtx` fields; EigonClass resolution; D49 witness synthesis |
| env, readback, recursor, positivity, unify, sized, sized_rigid | — (none) | 0 | pure |

### 2.3 External API surface: who consumes nbe

`nbe` is `pub mod` at kernel/src/lib.rs:36; no crate-root re-exports — the module paths are
the API. Reference counts per kernel module (reproduce:
`grep -rn "crate::nbe" kernel/src --include=*.rs | grep -v src/nbe/`):
program 125, dcg 25, layer 22, witness 11, validation 8, institution 5, query 4, commit 4,
server 3, esl 3, capability 2, ontology 1.

External crates importing `eigenius_kernel::nbe`: cli, eigenius-reasoning (4 src files),
eigenius-lean, eigenius-statistics, plus tests in eigenius-julia, eigenius-wordnet,
storage/rocksdb.

De-facto public API by item:

| Item | Principal consumers | Purpose |
| --- | --- | --- |
| `val::Val` (esp. `ResourceVal`, `ChainWitness`, `InductiveVal`) | institution trait (`Institution::invoke` returns `Val`), program/eval_io, witness, query/evaluate, commit, layer/witness_index, dcg; external: reasoning, lean, statistics | Universal value across the institution/component boundary — the hottest coupling point |
| `term::Exp` + decls | esl/compile, dcg (parser/pretty/category/lexicon), program/{expr,ground,eigentt_type_mirror}, layer/witness_index; external: reasoning | Syntax construction/inspection |
| `eval::{eval, eval_ctx, eval_traced, EvalCtx, EvalError}` | program/eval_io (sole `EvalCtx::IO` builder, eval_io.rs:106), server/parse, dcg, validation, layer/merge; external: reasoning, cli | Evaluation; IO program execution |
| `check::{check, check_infer, check_type, CheckCtx, eq_nf, …}` | program/{ground,axiom_env,expr}, dcg, witness, validation/rules/eigentt_value; external: reasoning/validate | Type checking of decoded/parsed terms |
| `readback::readback_val` | server/parse, program/{ground,expr}, dcg, witness | Normal forms for storage/encoding |
| `env::{Rho, Gamma, gen_val, up_gamma}` | every eval/check caller | Environments |
| `eval::{resource_value_to_val, val_to_resource_value}` | layer/merge, institution/marshal convention; external: lean | Resource⇄Val marshalling |

### 2.4 EvalCtx / capability threading (D9)

`EvalCtx` (eval.rs:96): `Pure`, `Read { layer }`, `IO { layer, registry, trace_store,
dispatched_traces, produced_resources, task_context, institution_index, institution_runtime }`,
`Check { layer, institution_index, institution_runtime }`.

- `Pure`: default; `eval()` (eval.rs:191) hard-codes it.
- `Read`: **no production construction site** (verified: workspace grep for `EvalCtx::Read`
  finds only match arms and doc mentions). Dead-variant candidate → §3.
- `IO`: built at one production site, program/eval_io.rs:106 (`execute_program_nbe*`),
  reached from server/programs.rs:98 and server/lifecycle.rs:246; also built directly by two
  integration tests (kernel/tests/d14_dock_assay_demo{,_wasm}.rs).
- `Check`: built at one site, check.rs:121 (`CheckCtx::eval_ctx()`), when institution
  index+runtime are attached via `with_institutions_d14`; else falls back to `Pure`.

Capability is threaded on **two axes**: `EvalCtx` (what the evaluator may do) and
`context::ExecutionMode` (what a dispatched institution may do) — nbe pins the latter to
`ReadOnly` at both construction sites (eval.rs:1451–1454, 2152–2155). `TaskContext` rides in
`EvalCtx::IO` for per-task positional trace keys (D21); `None` for synchronous runs and checking.

### 2.5 Duplication and internal notes

- **eval_ctx / eval_traced** (eval.rs:196 vs 714): two parallel ~400-line evaluation matches,
  the latter threading `Option<Trace>`. Mirrored in val.rs: `app_ctx`/`app_ctx_traced`,
  `vobserve_ctx`/`vobserve_ctx_traced`. Primary consolidation candidate (§3).
- **`gen_val` duplicated**: env.rs:169 (`gen_val(rho)` — level from env depth) and
  readback.rs:359 (`gen_val(level)`, private). Same concept, two signatures.
- **`Arrow`→`Pi` / `Times`→`Sig` desugaring** appears in both `check` and `check_infer`.
- **Name-based `PartialEq`** on `InductiveDecl` (term.rs:442) and `CodataDecl` (term.rs:473)
  — deliberate (self-reference support), correctness-sensitive; flag for §4 review.
- **No TODO/FIXME/unimplemented! markers** in the module (grep: 0 matches).
- check.rs's 159 tests include integration-style D14/institution tests (import the full
  registry/runtime); a file split should consider relocating those (§3).

## 3. Reorganization assessment

### 3.1 Mechanical splits (no semantic change)

**check.rs → `check/`** (production lines from §1.2; tests move with their subject):

| Target | Content (current lines) | ~Size |
| --- | --- | --- |
| `check/mod.rs` | `CheckCtx` (43–176), `check_decl`/`check_type` (177–326), `check` (327–689), `check_infer` (690–1143), `ext_pi`/`ext_sig`/sigma helpers | ~1,150 |
| `check/conv.rs` | `eq_nf` (1232), `def_eq_at_type`/propositionality (1461–1558), `subtype_of*` (1559–1641), `exp_mentions_var`/`patt_binds` (1374–1460) | ~330 |
| `check/inductive.rs` | large elim (1289–1373), `CtorArg`/telescopes (1962–2186), `check_inductive_ctor_args`/`check_infer_inductive_rec`/`check_match` (2250–2769), `extract_list_element_type` | ~900 |
| `check/codata.rs` | codata resolution (1144–1231), guardedness (1642–1898) | ~340 |
| `check/witness.rs` | D49 ChainWitness synthesis (2187–2249) — Eigenius-specific; absorbed by layering Option B if taken | ~60 |

nanoda precedent: `inductive.rs` separate from `tc.rs`. The 3,900-line test block splits by
subject; the D14-institution integration tests (import registry/runtime, check.rs:5501–5503
region) belong at the `check/mod.rs` level or in `kernel/tests/`.

**eval.rs → `eval/`**:

| Target | Content | ~Size |
| --- | --- | --- |
| `eval/mod.rs` | `EvalError`, `EvalCtx`, consolidated evaluator (§3.2) | ~700 after dedup |
| `eval/iota.rs` | `iota_reduce` + ctor-arg helpers (1223–1400) | ~180 |
| `eval/mapreduce.rs` | `eval_map`/`eval_reduce` (1112–1222) | ~110 |
| `eval/marshal.rs` | `resource_value_to_val`/`val_to_resource_value` (2263–2352) | ~90 |
| D14/IO engine (1401–2262) | → out of nbe entirely (§3.3 Option A); fallback: `eval/dispatch.rs` | ~860 |

### 3.2 `eval_ctx`/`eval_traced` consolidation (grade: Observed — full structural diff)

Facts (verified by side-by-side reading of eval.rs:196–713 vs 714–1105 and the val.rs pairs):

- `eval_traced` special-cases exactly **8 arms** (`Dec`, `App`, `PropAccess`, `Construct`,
  `Observe`, `Map`, `Reduce`, `InstitutionInvoke`); all ~40 other arms fall through a
  catch-all to untraced `eval_ctx` (eval.rs:1103). **No arm produces a different value** than
  `eval_ctx` — consolidation cannot change observable behavior. `Map`/`Reduce` re-implement
  `eval_map`/`eval_reduce` inline (case-for-case identical) rather than delegating.
- Real trace producers: `dispatch_component` (pushes `ComponentTrace` into the shared
  `dispatched_traces` vec — fires in **both** modes, eval.rs:1746/1800), the traced `App` arm
  (wraps `dispatched_traces.last()`, 783–790), the traced `InstitutionInvoke` arm
  (`Trace::Comorphism`, 1093–1099), and `Val::app_ctx_traced` (`Trace::Case`, val.rs:410–414).
  The rest are structural combinators. `Trace::Pure` is never produced by the evaluator.
- **The tree trace is lossy** (the flat `dispatched_traces` vec keeps layer commits correct;
  only the D6b tree loses nodes): `Reduce` drops one of two per-step traces
  (`t1.or(t2)`, eval.rs:975/994/1018); `Case` drops one of two and hardcodes
  `scrutinee_trace: None` (val.rs:410–414); `App` evaluates function/argument untraced
  (795–797); `Observe`'s receiver untraced (884); every catch-all arm with effectful children
  (`Pair`, `Con`, `Match`, `InductiveRec`, `NativeDecide`, …) is invisible in the tree.
  Recorded as **Finding F-5** (§4.3-adjacent; trace-completeness, not soundness).
  **F-5 FIXED 2026-07-07** with the consolidation (§5 item 5): every arm routes children
  through the tracer; `Trace::Seq` joins multiple effectful siblings; regression tests
  `f5_*` in eval/mod.rs.

**Design: generic `eval_impl<T: Tracer>`** (recommended over an `Option<&mut TraceSink>`
parameter). A `Tracer` trait with ~8 node-building methods + associated `type Node`;
`NoTrace` (`Node = ()`, all no-ops) monomorphizes to codegen equivalent to today's
`eval_ctx` — the type-checker's hot path stays zero-cost. `TreeTracer` (`Node =
Option<Trace>`) reproduces `eval_traced`. The three val.rs method pairs
(`app_ctx`/`app_ctx_traced`, `vobserve_ctx*`, `Clos::apply_ctx*`) unify under the same
parameter, which also removes `program::trace::Trace` from val.rs signatures. A sink design
can't reconstruct the nested tree without returning child nodes anyway, so it degenerates to
always-allocating returns on the hot path. Churn: **one** production caller of `eval_traced`
(program/eval_io.rs:128) + the val.rs pairs + tests. The rewrite is the natural point to fix
F-5 (route the missing arms/children through `T`; replace `or`-merges with multi-child
nodes) — under `NoTrace` all of it compiles away.

### 3.3 Layering: factoring the Eigenius runtime out of the NbE core

The verified hook surface (§2.2 + call-graph): the evaluator enters the D14/IO engine at
exactly **three points** — `InstitutionInvoke` arm → `try_d14_institution_invoke` (eval.rs:381),
`NativeDecide` arm → `decide_constraint` (393), `App`-component interception →
`dispatch_component` (272). Everything else in the 860-line engine is internal to it.

**Option A (recommended) — extract the engine behind an effect-hook trait.** nbe defines
(sketch):

```rust
pub trait EffectHooks {
    fn institution_invoke(&self, comorphism: &Iri, source: &Val, target: Option<&Iri>)
        -> Result<Option<Val>, EvalError>;          // None → stay neutral
    fn dispatch_component(&self, name: &str, input: &Val, arg: Option<&Val>)
        -> Result<Val, EvalError>;
    fn decide(&self, iri: &Iri, args: &[Val], rho: &Rho) -> Result<Decision, EvalError>;
}
```

The engine (try_d14_*, dispatch_component, schema resolution, verdict parsing, marshalling
into resources) moves to `kernel/src/institution/` (it already imports
`institution::{marshal,dispatch,DecResult}`); the `EvalCtx::IO`/`Check` payloads collapse to
`layer + Arc<dyn EffectHooks>` (+ tracing via §3.2). `Decision` is an nbe-owned verdict enum
mirroring `institution::DecResult` so nbe stops importing it. Effects are inherently
heavyweight (WASM/network/R/Lean), so dyn dispatch is noise. Consumer impact:
program/eval_io.rs:106 constructs the hooks impl instead of raw fields; the two integration
tests likewise; nothing else changes (verified: those are the only `EvalCtx::IO` build
sites). Removes `institution`, `program::{component,schema}`, `task`, `context`,
`observability` from nbe's imports (task/context/observability travel with the engine).

**Option A-fallback — `eval/dispatch.rs` submodule, no inversion.** Pure file move, zero
API churn, but nbe keeps all seven Eigenius imports. Choose only if the trait is rejected at
review.

**Option B (recommended, same pattern) — check-side hooks.** `CheckCtx`'s three Eigenius
couplings — `program::ground::resolve_class_type` (check.rs:168), `layer::
synthesize_chain_witness` + `witness::WitnessCategory` (2230–2240), institution
index/runtime fields — become a `CheckHooks` trait (`resolve_class`, `synthesize_witness`)
plus the same `EffectHooks` for check-time deciding. Public constructors (`new`,
`with_layer`, `with_institutions_d14`) keep their signatures as kernel-level convenience
wrappers that wire the default impls, so the 12+ `CheckCtx` construction sites and external
crates (eigenius-reasoning) see no change.

**Option C — `Val`/`Exp` ground-type embeddings: keep (documented).** `ResourceVal`,
`ChainWitness(WitnessKey)`, `EigonClass`, `Lit*` are **data** — ground values of the type
theory (D9/D18); `Val` is the marshalling type every institution crate depends on (§2.3).
The effects move (A/B); the data stays. Post-A/B/§3.2, nbe's external imports reduce to
`ontology` (data model) + `witness::WitnessKey` (a key type). `Exp`'s doc-comments citing
institution types (term.rs:96,264,382) get rewritten against the trait.

**Also:** `EvalCtx::Read` has no production constructor (§2.4) — remove it, or fold
`Read`/`Check` into one "layer + hooks, no component registry" shape under Option A (the
natural outcome: `Pure` vs `WithEffects{layer, hooks, registry?}`).

### 3.4 Recommended target tree

```text
kernel/src/nbe/
  mod.rs  term.rs  env.rs  readback.rs  recursor.rs  positivity.rs
  unify.rs  sized.rs  sized_rigid.rs          (unchanged)
  val.rs                                      (Tracer-unified methods; no Trace in sigs)
  eval/{mod,tracer,iota,mapreduce,marshal}.rs (EffectHooks trait defined in eval/mod.rs)
  check/{mod,conv,inductive,codata}.rs        (CheckHooks trait in check/mod.rs)
kernel/src/institution/eval_hooks.rs          (the 860-line engine + EffectHooks impl)
kernel/src/program/check_hooks.rs             (CheckHooks impl: ground resolution + D49 witness)
```

Dependency result: nbe → `ontology` + `witness::WitnessKey` only; all effect/runtime coupling
inverted. Consumers unchanged except the single `EvalCtx::IO` build site and the `CheckCtx`
constructor internals.

## 4. Ancestry alignment

### 4.1 Port fidelity vs nanoda_lib @ `f58f2f6` (D1–D3) — COMPLETE

**Citation check.** nanoda citations in nbe: positivity.rs:23–24 (`inductive.rs:666-787`,
`main`-branch URL), check.rs:38–39 (tc.rs, no line range), term.rs:402 (stylistic mention).
At the pin, `check_positivity1` = 666, `is_valid_ind_app` = 691, `has_ind_occ` = 749,
`which_valid_ind_app` = 775 (family ends ≤787): **the cited range matches the pin — no
drift** (grade: Observed). Backlog: rewrite the URL to a pin-anchored form
(`references/nanoda_lib/src/inductive.rs:666-787 @ f58f2f6`).

**Function correspondence** (each row verified by reading both sides; grade: Observed):

| nanoda @ f58f2f6 | Ours | Delta | Classification |
| --- | --- | --- | --- |
| `check_positivity1` (inductive.rs:666) | `check_constructor` / `check_arg_positivity` (positivity.rs:65/88) | nanoda checks per-binder, whnf-normalizes the cursor, and **accepts** higher-order positive args (rejects only ind-occurrence in a Pi *domain*, recurses the codomain); ours walks the telescope syntactically, no normalization, rejects higher-order | Intentional restriction (documented, positivity.rs:37–40: iota can't build higher-order IHs) |
| `is_valid_ind_app` (691) | `check_arg_positivity` case 2 + `check_result_type` (119) + `validate_indexed_ctor_conclusions` (check.rs:1994) | nanoda: head const match + level eq + arg count = params+indices + **index args I-free** + **params exactly the block params** (`ctor_app_params_ok`). Ours: head-IRI match; args-I-free (arg sites); arg *count* only (conclusions) | **Findings F-2, F-3** below |
| `has_ind_occ` (749) | `has_ind_occurrence` (positivity.rs:134) | nanoda: `find_const` by name over single-repr syntax. Ours: structural walk with per-variant arms; `Exp::Inductive(_)` arm returns `false` (positivity.rs:153) while `Exp::Inductive(d)` *evaluates* to the same `Val::InductiveType` as `Exp::InductiveType(d, [])` (eval.rs:534) | **Finding F-1** (ours-specific, no nanoda analogue) |
| `which_valid_ind_app` (775) | — | mutual-inductive-block support; we have no mutual inductives | Intentional restriction |
| `large_elim_test` (880) + `large_elim_test_aux` (845) | `large_elim_admitted` + `ctor_args_pass_singleton_b` (check.rs:1289/1314) | zero-ctor/single-ctor/multi-ctor skeleton matches. Arg admissibility: nanoda infers the binder's sort (semantic) and requires each non-Prop arg to **be a member of** the conclusion's applied params+indices; ours checks propositionality **syntactically** (conservative → safe) and accepts an arg **mentioned in** an index expression | Propositionality: safe simplification. Mentions-vs-membership: **Finding F-4** |
| `init_k_target` (947), `to_ctor_when_k` (tc.rs:985) | — | K-like reduction for structural eta / unit-like props: unimplemented here | Gap — borrow-idea candidate, not a soundness issue |
| `mk_minors1group` (1069) | `derive_minor_type` (recursor.rs:66) | Minor shape `Π args. Π ihs. C (c args)` — identical (nanoda: `abstr_pis(all_ctor_args, abstr_pis(ih_pis, c_app))`, inductive.rs:1085–1086). nanoda also builds higher-order IHs (`handle_rec_args_aux`); ours direct-only, consistent with the positivity restriction | Already-match (for our fragment) |
| `mk_rec_rule1` (1137) / `reduce_rec` (tc.rs:1041) | `iota_reduce` (eval.rs:1223) | Application order minor → ctor args (original order) → IHs (recursive-arg order) — consistent on both sides | Already-match; witnessed by the two new ordering tests |

### 4.2 Parity test matrix (grade: Derived — `cargo test -p eigenius-kernel --lib`, 1617 passed, 0 failed)

| Case | Ours | nanoda | Test |
| --- | --- | --- | --- |
| Nat / List / Bool | accept | accept | `positivity::accepts_*` (pre-existing) |
| Negative occurrence | reject | reject | `rejects_negative_occurrence` (pre-existing) |
| Higher-order positive | reject | **accept** | `rejects_higher_order_positive` (pre-existing; intentional restriction) |
| Nested occurrence | reject | reject | `rejects_nested_occurrence` (pre-existing) |
| Wrong result type | reject | reject | `rejects_wrong_result_type` (pre-existing) |
| Param-mismatched recursive arg | reject (fixed 2026-07-07) | reject | `rejects_param_mismatch_in_recursive_arg` (new) |
| Non-uniform conclusion params | reject (fixed 2026-07-07) | reject | `rejects_nonuniform_conclusion_params` (new) |
| Disguised `Exp::Inductive` negative occurrence | reject (fixed 2026-07-07) | n/a (single repr) | `rejects_disguised_inductive_negative_occurrence` (new) |
| Large elim: zero/multi ctor, all-Prop args, non-Prop arg, Eq-via-indices, arg-not-in-conclusion | match | match | `large_elim_*`, `d48_singleton_*` (pre-existing) |
| Large elim: index *mentions* but ≠ arg | reject (fixed 2026-07-07) | reject | `singleton_elim_rejects_index_that_only_mentions_arg` (new) |
| Large elim: index refers to a shadowed binder | reject | n/a (unique locals) | `singleton_elim_rejects_shadowed_arg_reference` (new) |
| Minor binder order (args → IHs, first-rec outermost) | ✓ | ✓ | `node_minor_binder_order_is_args_then_ihs_in_arg_order` (new) |
| Iota application order matches minor binders | ✓ | ✓ | `iota_two_recursive_args_ih_order_matches_minor_binders` (new) |

### 4.3 Findings — all four FIXED (2026-07-07, this branch; each parity test asserts rejection)

- **F-1 (fixed) — positivity evasion via `Exp::Inductive` form.** `Exp::Inductive(d)` and
  `Exp::InductiveType(d, [])` evaluate identically (eval.rs:534) but the former was invisible
  to `has_ind_occurrence`. Fix: the predicate now matches evaluation semantics —
  `Exp::Inductive(d)` is an occurrence on iri match, and embedded declarations' ctor types
  are scanned (self-reference stubs carry empty ctors, so recursion is bounded). Severity
  note: no production code outside nbe constructs `Exp::Inductive` (workspace grep) — the
  form was reachable via the kernel API only.
- **F-2 (fixed) — recursive-arg params not checked against block params.** `check_arg_positivity`
  now enforces full arity (params + indices) and parameter pass-through on recursive
  occurrences (`check_params_uniform`, port of nanoda's `ctor_app_params_ok`), shadow-aware:
  the ctor-prefix binder names are tracked and cleared when rebound.
- **F-3 (fixed) — conclusion params not checked for uniformity.** `check_result_type` applies
  the same `check_params_uniform` to the conclusion's parameter prefix (arity stays with
  `validate_indexed_ctor_conclusions`, which has the friendlier count diagnostics).
- **F-4 (fixed) — singleton-elim Case B "mentions" → "is one of".** `ctor_args_pass_singleton_b`
  now requires a non-Prop arg to *be* a conclusion index (`Exp::Var(name)`, unshadowed) —
  membership, matching `large_elim_test_aux` and Lean's recoverability rule. `Eq` stays
  admitted (its args are the indices). D46 §7 Case B and D48 §5.8 wording sharpened to the
  strict reading.

Upstream delta scan: not run. The vendored pin `f58f2f6` is current with upstream (grade:
Declared — user statement, 2026-07-07); no upstream fixes to the ported functions to review.

### 4.4 Remaining dimensions (D4–D12)

**D4 — def-eq caching (verdict: borrow-idea; act only on profiling evidence).**
Ours: `eq_nf` (check.rs:1232) reads back both values at the current level and compares `Exp`
syntactically; no caching of any kind exists around it (the only cache in `CheckCtx` is class
resolution). Call sites: 5 (`eq_nf`) + 9 (`subtype_of*`). nanoda: `TcCache` with split infer
caches, whnf caches, union-find success cache, failure cache — justified by re-reducing
syntax at Mathlib scale. Ours re-derives normal forms per comparison; the borrowable ideas,
in order of fit: (1) a syntactic fast path before readback (`Val` variants that are trivially
equal/unequal), (2) a per-`CheckCtx` def-eq success cache keyed on readback NFs. Neither is
justified today — no measured hotspot; record as profile-gated backlog items.

**D5 — proof irrelevance ordering (verdict: already-match).** `def_eq_at_type`
(check.rs:1461) short-circuits on propositionality *before* structural comparison, with a
structural fast path then inference (`is_propositional_in_ctx`) — same tactic order as
nanoda's `proof_irrel_eq` placement (tc.rs:1291–1309, before delta unfolding). `eq_nf` also
fast-paths `ChainWitness` key equality (D49 §8).

**D7 — context threading (verdict: borrow-idea, one concrete item).** `CheckCtx::extend`
(check.rs:142) deep-clones `type_cache` (a `BTreeMap<String, Val>` — `Val` clones are deep)
plus `size_tso` at every binder crossing; 24 production call sites. Two observations:
(1) the doc comment says the child "shares … the type_cache with the parent" but the code
*clones* — child inserts don't propagate back, so resolution work inside a binder is lost on
exit (doc/code mismatch + a real cache-effectiveness gap); (2) nanoda's borrow-split
(`&mut TcCtx` vs `&Env`) exists for borrow-checker reasons and is not our shape — the fitting
fix here is `Rc<RefCell<BTreeMap<…>>>` or moving the cache out of the cloned-per-binder
struct, keeping cache lifetime per check-invocation (which the current design already
intends). Concrete backlog item, independent of any nanoda alignment.

**D12 — InferFlag (verdict: not applicable).** nanoda's `InferOnly` skips re-checking during
reduction of known-well-typed terms; our evaluator is untyped (no checking during `eval`) and
the bidirectional checker doesn't re-infer checked subterms beyond the standard
check→infer+subtype fallback. Nothing to borrow without a profiling signal.

**Deliberate-divergence register** (the better reference per dimension; recorded so "why not
like nanoda" isn't re-litigated):

| Dim | Verdict | Register entry |
| --- | --- | --- |
| D6 universes | deliberate divergence | Concrete `Sort(n)` + cumulativity via `subtype_of` (Prop=0, D46). nanoda's `Zero/Succ/Max/IMax/Param` + `leq_core` serve universe *polymorphism*, which EigenTT does not have. If polymorphism ever lands, revisit `level.rs` wholesale; until then the simple scheme is correct and cheaper. |
| D9 term representation | deliberate divergence | `Exp`/`Val`/`Clos` + readback IS the NbE architecture (Mini-TT `Main.hs` is the reference). nanoda's hash-consed, flag-annotated, closure-free arena exists because it re-reduces syntax. Only per-`Exp` hash-consing is even discussable, and only with profiling evidence. |
| D10 errors | deliberate divergence | nanoda panics; our `Result` discipline is strictly better. The real item is ours alone: `check.rs` returns `Result<_, String>` while `eval.rs` has typed `EvalError` — a typed `CheckError` is a backlog item on our own terms, not an alignment one. |
| D11 metavariables / sized types | out of scope for nanoda | nanoda checks fully elaborated terms — no metavars, no sizes. References of record: MiniAgda (`Warshall.hs` → sized.rs, `TreeShapedOrder.hs` → sized_rigid.rs, both verified vendored) and the D48 unification literature. The thin `unify ↔ check::eq_nf` cycle (§2.1) is an internal design question. |

## 5. Ranked action backlog

Ordering rationale: soundness findings first (small, independent of structure); then the
structural work in dependency order (splits → evaluator consolidation → hooks extraction),
because the consolidation is easier inside already-split files and the hooks extraction
builds on the consolidated evaluator; small hygiene items ride along; profile-gated items
last.

| # | Action | Evidence | Cost | Risk / prerequisite |
| --- | --- | --- | --- | --- |
| 1 | ~~Fix F-1~~ **done 2026-07-07**: `has_ind_occurrence` matches evaluation semantics for `Exp::Inductive` (predicate correction, not a guard — the predicate was wrong about what the form means); no producer outside nbe emits the form (verified) | §4.3, `rejects_disguised_inductive_negative_occurrence` | — | — |
| 2 | ~~Fix F-4~~ **done 2026-07-07**: membership rule (arg *is* an unshadowed conclusion index), shadow-aware; D46 §7 + D48 §5.8 wording sharpened; `Eq` stays admitted | §4.3, `singleton_elim_rejects_index_that_only_mentions_arg`, `…_rejects_shadowed_arg_reference` | — | — |
| 3 | ~~Fix F-2 + F-3~~ **done 2026-07-07**: `check_params_uniform` (≈ nanoda `ctor_app_params_ok`) + arity on recursive occurrences and conclusions, shadow-aware prefix tracking in `check_constructor` | §4.3, `rejects_param_mismatch_in_recursive_arg`, `rejects_nonuniform_conclusion_params` | — | — |
| 4 | ~~Split check.rs and eval.rs~~ **done 2026-07-07** per §3.1. Result: `check/{mod 1228, inductive 883, codata 380, conv 341, witness 105, testutil}.rs` and `eval/{mod 1141, dispatch 906, iota 174, mapreduce 122, marshal 118, testutil}.rs` (production lines; tests moved with subjects, shared helpers in per-directory `testutil`). Public paths preserved via `pub use` re-exports; only-visibility changes (`pub(super)`) at the new internal boundaries. Verified: kernel lib 1618 passed (identical), workspace exit 0, clippy `-D warnings` clean, fmt clean | §3.1 | — | — |
| 5 | ~~Consolidate evaluators~~ **done 2026-07-07**: single `eval_impl<T: Tracer>` (`eval/tracer.rs`: `Tracer` trait, ZST `NoTrace`, `TreeTracer`); `eval_ctx`/`eval_traced` are thin wrappers; val.rs pairs unified into `apply_impl`/`app_impl`/`vobserve_impl` (one implementation each; `Trace` gone from val.rs); `eval_map`/`eval_reduce`/`iota_reduce` generic. **F-5 fixed**: all arms route children through `T`; new `Trace::Seq` node (+ `reflection:SeqTrace` class, D6b §2 updated) for multi-child structural joins; `Match` now emits `CaseTrace`; `Reduce` steps keep both application traces; App/Observe children traced. 4 F-5 regression tests; all 5 pre-existing trace-shape tests unchanged. Kernel lib 1622 passed, workspace clean | §3.2 | — | — |
| 6 | ~~Extract effect hooks~~ **done 2026-07-08** (Options A + B). Eval-side: `EffectHooks` trait (`nbe/eval/hooks.rs`) + `InstitutionEngine` (`institution/eval_hooks.rs`) holding the moved 860-line engine; `EvalCtx` collapsed to `Pure \| Effectful{layer, hooks}` (dead `Read` removed, `IO`/`Check` folded into hooks impls via `for_io`/`for_check`); `decide_constraint` split (structural stays pure in core, institution → hook); tracer takes the returned `ComponentTrace` (no shared-state read). Check-side: `CheckHooks` trait (`nbe/check/hooks.rs`) + `DefaultCheckHooks` (`program/check_hooks.rs`); `CheckCtx` gains an `Arc<dyn CheckHooks>` wired by the constructors (signatures unchanged). **Result: `nbe/eval` imports only `program::trace` data types; `nbe/check` only `layer::Layer`** — the `institution`/`program::{component,schema}`/`task`/`context`/`observability`/`witness` coupling is gone from the NbE core, now held by the two hook impls. 7 engine unit tests migrated. Kernel lib 1624 passed, workspace + clippy `-D warnings` + fmt clean | §3.3 | — | — |
| 7 | ~~Hygiene batch~~ **done 2026-07-08**: `EvalCtx::Read` removed (folded into `Pure`/`Effectful` by item 6); `is_recursive_arg_type`/`is_direct_recursive_ref` deduped into `InductiveDecl::is_direct_recursive_ref` (term.rs); nanoda citations re-anchored to `references/nanoda_lib/… @ f58f2f6`; `extend` doc-comment corrected (clones the cache — §4.4-D7). **Correction**: the flagged `gen_val` "duplication" is *not* one — the name tag is load-bearing (`Neut::Gen(j, name)` reads back as `Exp::Var("{name}{j}")`), so readback's `G#` (paired with `gen_patt`) and the checker's `TC#` are deliberately distinct; both call sites now document this. Kernel lib 1624, clippy `-D warnings` + fmt clean | §2.4, §2.5, §4.1, §4.4 | — | — |
| 8 | Typed `CheckError` replacing `Result<_, String>` in check/ (our own item, not nanoda alignment) | §4.4-D10 | medium | Best done after #4's splits |
| 9 | Profile-gated: `type_cache` sharing instead of clone-per-`extend` (24 sites); syntactic fast path / success cache around `eq_nf` | §4.4-D4/D7 | small each | Requires a profiling harness first — no measured hotspot today |
| 10 | Idea parking: K-target reduction (nanoda `init_k_target`/`to_ctor_when_k`); per-`Exp` hash-consing | §4.1, §4.4-D9 | large | Only with a concrete need (K: eta for unit-like props; hash-consing: profiling) |

Verification items closed with the fixes: (a) no production code outside nbe constructs
`Exp::Inductive` (workspace grep — the F-1 form was kernel-API-only); (b) F-2/F-3-shaped
declarations are now rejected at declaration time, so no recursor use site can receive them.

Addendum (2026-07-07, follow-on to item 5): the trace schema is now closed over a base
class — abstract `reflection:Trace`, all node classes `subclass_of` it, every trace-child
property + `trace_tree` `class_types`-constrained to it (Rule 8 matches transitively), and
the untyped empty-embedded-resource placeholder replaced by typed `reflection:EmptyTrace`
(`empty_trace_resource()` in program/trace.rs).

Addendum 2 (2026-07-07): **embedded-resource recursion (validation Rule 23) now landed** —
`validate_resource` descends into every embedded resource that declares an `is_a`, applying
the full rule set at every depth; embedded resources without `is_a` (opaque internal
program-/comorphism-mirror carriers) are skipped. Closing the recursion gap surfaced four
latent schema/impl bugs the non-recursion had hidden, all fixed here:

- `ConstructTrace.field_traces` was an untyped IRI-keyed embedded map (recursion can't type
  it) → restructured to a `resource_array` of typed `reflection:FieldTrace` entries
  (`property` + `trace`), serializer updated.
- Four trace-node classes listed structurally-optional children in `requires`
  (`LetTrace.value_trace`/`body_trace`, `ProjectTrace.source_trace`, `CaseTrace.branch_trace`,
  `ComponentTrace.timestamp` — the last never emitted at all) → moved to `recommends`.
- `reflection:property` domain didn't include `FieldTrace` → added.
- The kinase-notebook compile test asserted Part C's comorphism-dispatching `produce_problem`
  program "validates cleanly"; it never did — the dangling comorphism reference sat in an
  embedded Apply node the old validator skipped. Its reference closure (comorphism → formats
  → institution → `symbolics:env:v1`) bottoms at a Julia runtime-env build artifact
  unresolvable offline, so the test now compile-checks such cells but excludes them from the
  clean-validation chain (detected structurally, not by cell id).
Tests: `validation::tests::embedded_typed_instance_is_validated_at_depth` (deep node
validated), `…program_trace_tree_root_is_class_typed` (root typing), plus the rocksdb
`run_program_commits_*` integration path. Full workspace green (kernel lib 1624).
