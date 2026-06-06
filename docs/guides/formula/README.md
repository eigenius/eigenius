# Formula language user guide

The formula language is a **chain-mirrored fragment of EigenTT** that lives at
`urn:eigenius:formulas:` — a small typed expression-tree language shared by
every numerical institution on the platform. A single payload shape that
[`Symbolics.jl`](https://juliasymbolics.org/),
[`IntervalArithmetic.jl`](https://juliaintervals.github.io/),
[`Catalyst.jl`](https://docs.sciml.ai/Catalyst/stable/),
[`OrdinaryDiffEq.jl`](https://docs.sciml.ai/DiffEqDocs/stable/), and
[`JuMP+HiGHS`](https://jump.dev/) all consume directly.

This guide is grounded in [D32 — Chain-Mirrored EigenTT
Inductives](../../design/d32-chain-mirrored-mini-tt-inductives.md) and the
implementation in [`ontologies/formulas/`](../../../ontologies/formulas/),
[`kernel/src/esl/parser.rs`](../../../kernel/src/esl/parser.rs), and
[`kernel/src/validation/`](../../../kernel/src/validation/). Every claim in
this guide can be exercised against the kinase-institutions notebook
([`notebooks/examples/kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json)).

## How to read this guide

Read sequentially if it's your first encounter. The chapters build on each
other:

1. **[Introduction](01-introduction.md)** — what the formula language is,
   why it exists at `urn:eigenius:formulas:` rather than under any one
   institution, and the three-surface mental model (EigenTT fragment /
   chain encoding / ESL `formula(...)` sublanguage).
2. **[The EigenTT fragment](02-mini-tt-fragment.md)** — the six
   constructors (`Var`, `LitFloat`, `OpRef`, `App`, `Lam`, `Pi`), what they
   correspond to in EigenTT, and why two binders (`Lam` / `Pi`) sit
   alongside the four expression-shaped ones.
3. **[Eigon-JSON embedding](03-eigon-json-embedding.md)** — the tagged-dict
   shape `{"ctor", "args"}`, left-spined `App` currying for multi-arg
   operators, the validator's inductive-value rule, and how every chain
   commit gets type-checked.
4. **[The operator catalog](04-operator-catalog.md)** —
   `formulas:Operator` resources, the on-chain `operator_signature`
   (Pi-spine), the App-spine arity check, and how to author a new operator.
5. **[The ESL `formula(...)` sublanguage](05-esl-sublanguage.md)** —
   Pratt-parsed surface for `+ - * / ^`, function calls, parens, unary
   minus. Operator precedence, the lexer's unary-minus subtlety, and
   how the result is a `Value::CtorApp` literal mirroring the FormulaTerm
   tree.
6. **[Sharing across institutions](06-sharing-across-institutions.md)** —
   why FormulaTerm-speaking institutions can comorphism-bridge each other
   with mostly identity transformations; how Symbolics, IntervalArithmetic,
   Catalyst, DiffEq, and JuMP-HiGHS all consume the same shape.
7. **[Common failure modes](07-failure-modes.md)** — operator arity
   mismatch, unknown ctor, free variable in handler, lexer surprises.
8. **[Appendix](08-appendix.md)** — references, source index, related
   design docs.

## Most important chapters

- **[1. Introduction](01-introduction.md)** for the framing.
- **[3. Eigon-JSON embedding](03-eigon-json-embedding.md)** for hand-authoring
  FormulaTerm values.
- **[5. ESL `formula(...)` sublanguage](05-esl-sublanguage.md)** for
  authoring values inside ESL programs.

## Cross-references

- [**ESL user guide**](../esl/README.md) — surface syntax for ontologies
  and programs; `formula(...)` sublanguage detailed reference is here.
- [**EigenQL user guide**](../eigenql/README.md) — surface syntax for
  queries; FIBER param coercion across institutions sharing FormulaTerm.
- [**Platform §11 — Runtime substrate**](../platform/11-runtime-substrate.md)
  — the hosting layer for institutions that consume FormulaTerm.
- [**Symbolics tutorial**](../platform/julia-institutions/symbolics-institution-tutorial.md)
  — the most thorough worked example of FormulaTerm end-to-end.
- [**D32 — Chain-Mirrored EigenTT
  Inductives**](../../design/d32-chain-mirrored-mini-tt-inductives.md) —
  the canonical design specification.

---

Ready to start? → **[1. Introduction](01-introduction.md)**
