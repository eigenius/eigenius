# 8. Appendix

## 8.1. Source index

| File | Purpose |
|---|---|
| [`ontologies/formulas/formulas-ontology.json`](../../../ontologies/formulas/formulas-ontology.json) | Chain declaration of the `FormulaTerm` inductive type, the `Operator` class, and the v1 catalog (`add`, `mul`, `pow`, etc.) |
| [`kernel/src/esl/lexer.rs`](../../../kernel/src/esl/lexer.rs) | Lexer; tokenises `+ - * / ^` and the unary-minus rules described in [chapter 5 §5.3](05-esl-sublanguage.md#53-the-unary-minus-subtlety-lexer-note) |
| [`kernel/src/esl/parser.rs`](../../../kernel/src/esl/parser.rs) | `formula(...)` Pratt parser; lowers the math sublanguage to `Value::CtorApp` literals |
| [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs) | Lowers `Value::CtorApp` to chain-bound `Value::Json` carrying the canonical tagged-dict shape |
| [`kernel/src/validation/`](../../../kernel/src/validation/) | The validator's inductive-value rule (chapter 3 §3.4); the App-spine arity check (chapter 4 §4.3) |
| [`crates/runtime-substrate/src/mirror_generator.rs`](../../../crates/runtime-substrate/src/mirror_generator.rs) | Closure walker; emits per-ctor decoder/encoder code for `FormulaTerm` and other inductives |
| [`crates/eigenius-julia/src/mirror_gen.rs`](../../../crates/eigenius-julia/src/mirror_gen.rs) | Julia-specific mirror generator; emits `decode_FormulaTerm` and per-ctor structs |
| [`julia/institutions/symbolics/EigeniusSymbolics/src/EigeniusSymbolics.jl`](../../../julia/institutions/symbolics/EigeniusSymbolics/src/EigeniusSymbolics.jl) | `formula_to_num` walker — the most thorough per-handler example |
| [`julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl`](../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl) | `formula_to_interval` walker |
| [`julia/institutions/diffeq/EigeniusDiffEq/src/EigeniusDiffEq.jl`](../../../julia/institutions/diffeq/EigeniusDiffEq/src/EigeniusDiffEq.jl) | `formula_to_value` walker; ODE RHS interpretation |
| [`julia/institutions/jump/EigeniusJuMPHiGHS/src/EigeniusJuMPHiGHS.jl`](../../../julia/institutions/jump/EigeniusJuMPHiGHS/src/EigeniusJuMPHiGHS.jl) | `formula_to_jump` walker; smart-pow rule |
| [`julia/comorphisms/symbolics-to-intervals.eigon.json`](../../../julia/comorphisms/symbolics-to-intervals.eigon.json), [`catalyst-to-diffeq.eigon.json`](../../../julia/comorphisms/catalyst-to-diffeq.eigon.json), [`symbolics-to-jump.eigon.json`](../../../julia/comorphisms/symbolics-to-jump.eigon.json) | The three v1 cross-institution comorphisms; identity transformations on FormulaTerm |

## 8.2. Worked-example references

The kinase-institutions notebook is the canonical end-to-end exercise of
the formula language across institutions:

- [`notebooks/examples/kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json)
  — twelve cells exercising FormulaTerm in DiffEq RHS, JuMP objective,
  `OptimisesTo` claims, and comorphism dispatch through both ESL and
  EigenQL surfaces.
- [`notebooks/examples/kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh)
  — installs the five institutions and three comorphisms.

The per-institution Julia tutorials (under
[`platform/julia-institutions/`](../platform/julia-institutions/)) each
exercise FormulaTerm in their own institution's vocabulary:

- [Symbolics](../platform/julia-institutions/symbolics-institution-tutorial.md)
  — most thorough, exercises validator + AutoOnLoad + OnDemand + Decidable
- [IntervalArithmetic](../platform/julia-institutions/intervals-institution-tutorial.md)
  — substrate plumbing slow-walk
- [Catalyst](../platform/julia-institutions/catalyst-institution-tutorial.md)
- [DiffEq](../platform/julia-institutions/diffeq-institution-tutorial.md)
- [JuMP-HiGHS](../platform/julia-institutions/jump-highs-institution-tutorial.md)

## 8.3. Related design documents

- [**D32 — Chain-Mirrored EigenTT
  Inductives**](../../design/d32-chain-mirrored-mini-tt-inductives.md) —
  the canonical specification for the formula language.
- [**D14 — Institution Realisation**](../../design/d14-institution-realisation.md)
  — institution model; §4 covers `ExportFormat` / `ImportFormat` /
  `Comorphism` shapes.
- [**D19 — Inductive Types**](../../design/d19-inductive-types.md) —
  the inductive-types-on-the-chain mechanism `formulas:FormulaTerm` is
  declared with.
- [**D27 — Julia institutions**](../../design/d27-julia-institutions.md)
  — the v1 Julia institution suite that consumes FormulaTerm.
- [**D29 — Mirror Generator**](../../design/d29-runtime-mirror-generator.md)
  — closure walker; emits the per-ctor decoder/encoder code that walks
  FormulaTerm values.

## 8.4. Cross-language guides

- [**ESL user guide**](../esl/README.md) — surface syntax for
  ontologies and programs; `formula(...)` shows up in [§5
  Expressions](../esl/05-expressions.md).
- [**EigenQL user guide**](../eigenql/README.md) — surface syntax for
  queries; FIBER param coercion across FormulaTerm-speaking
  institutions in [§7](../eigenql/07-fiber-clauses.md).
- [**Platform §11 — Runtime substrate**](../platform/11-runtime-substrate.md)
  — hosting layer for the institutions that consume FormulaTerm.

## 8.5. Phase status

The formula language is currently complete through Phase 19f.3:

- **Phase 19f.1** — `formulas:FormulaTerm` chain declaration; v1
  operator catalog; validator's inductive-value rule.
- **Phase 19f.2** — ctor-call literals on the ESL surface
  (`App(...)`, `OpRef(...)`, `Var(...)`, `LitFloat(...)`); typed
  inline authoring without raw `eigen.load(JSON.stringify(...))`
  workarounds.
- **Phase 19f.3** — Pratt-parsed `formula(...)` sublanguage in ESL;
  lexer-level unary-minus refactor.
- **Phase 19h.1** — Catalyst → DiffEq comorphism (identity on
  `OdeProblem`).
- **Phase 19f.1** — Symbolics → JuMP comorphism (identity on
  `OptimisationProblem`).
- **Phase 19d.2** — Symbolics → IntervalArithmetic comorphism (identity
  on `FormulaTerm`).
- **Phase 19i** — Comorphism chain reinsertion (D14 §9.3); both ESL
  program-invoke (`Exp::InstitutionInvoke`) and EigenQL `FIBER ... INTO`
  surfaces.

Next: structured-clause cross-institution flows under the Curry–Howard
reading from [chapter 2 §2.2](02-mini-tt-fragment.md#22-why-pi-and-lam-are-chain-resident);
medium-term as application demand grows.

---

Return to **[README](README.md)**.
