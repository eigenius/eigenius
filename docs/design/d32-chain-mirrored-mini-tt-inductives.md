# D32: Chain-Mirrored EigenTT Inductives + the `FormulaTerm` Language

**Date:** 2026-05-06
**Status:** Implemented (Phase 19d.0; `formulas:FormulaTerm` bootstrap layer + ESL `formula(...)` Pratt sublanguage live)
**Prerequisites:** D14 (Institution Realisation), D19 (EigenTT Inductive Types), D26 (Runtime Substrate), D27 (Julia Institutions), D29 (Eigon-Julia Mirror)
**Drives:** Phase 19d (Symbolics institution), the comorphism story across Phase 19e–19h, Phase 20 (Lean) cross-institution surface.

## 1. Motivation

### 1.1 The cross-institution formula transfer problem

Phase 19's reference institutions — `Symbolics`, `JuMP`, `IntervalArithmetic`, `Catalyst`, `DiffEq` — all consume and emit *formulas*: trees of operators applied to numeric variables, constants, and other operators. The same formula `x² + sin(x)` must be expressible as a `SymbolicExpression` (input to `qc_symb_simplify`), as the integrand of a `qc_intv_compute_bounds` call (interval extension), as the objective of a `qc_jump_solve`, and as the right-hand side of an `OdeProblem`. D14's Comorphism mechanic — the `(s, m, t)` triple where `m` is a EigenTT Component — assumes a typed payload language exists that the source `s` extracts into and the target `t` reifies from. Without a shared formula representation, every cross-institution path reinvents its own ad-hoc serialisation; comorphisms become impossible to type-check at the chain boundary; and EigenQL FIBER queries that traverse formulas through multiple institutions can't produce a stable shape.

### 1.2 Why now: Phase 19d (Symbolics) needs the chain shape

Phase 19a established the substrate and proved the institution dispatch path with `IntervalArithmetic`. Its sole resource class (`BoundedBy(value, lower, upper)`) is shallow — three floats. Phase 19d's Symbolics institution is the first one that demands a *typed expression tree* on the chain ([D27 §4.1](d27-julia-institutions.md)): `SymbolicExpression`, `SymbolicallyReducesTo`, `SimplifiesTo`, `Substitutes`, `SatisfiesEquation` all carry `SymbolicTerm` values whose shape is recursive (`Add` takes two `SymbolicTerm`s) and whose ctors take typed arguments (`Sym` takes a name string, `Const` takes a float, `Term(head, args)` takes a head IRI plus a list of subterms). The chain ontology *already supports* declaring such inductives — `core:InductiveType`, `core:InductiveCtor`, and `core:InductiveArgType` are all in place and ESL's compiler emits them — but the surface is unused outside the ESL compile path: no validator rule type-checks inductive *values* at commit, no mirror generator emits Julia for them, and no JSON-authored ontology layer references them. Closing those gaps is what unblocks 19d.

### 1.3 What's missing today

Three holes between the kernel's existing capability and the chain's representational power:

1. **The chain can't declare what constructor arguments look like.** [`core:InductiveCtor`](../../ontologies/core/core-ontology.json) requires only `ctor_name`. Its description says "an ordered list of argument types" but the corresponding property doesn't exist.
2. **The validator can't type-check inductive values.** Resources whose property values are inductive trees (e.g. `SymbolicExpression.term`) aren't validated against any ctor schema today — they'd land as `Value::Json` and the validator would pass them through unchecked.
3. **The mirror generator can't emit Julia for inductive types.** [`JuliaMirrorGenerator`](../../crates/eigenius-julia/src/mirror_gen.rs) emits structs for chain `Class` resources only. An `InductiveType` walked by the closure today is silently skipped.

This document specifies the design that closes those three holes, and pins how the load-bearing first consumer — a chain-shared `FormulaTerm` language with a typed operator catalog — sits on top.

## 2. Background

### 2.1 EigenTT inductives in the kernel work today (D19)

The kernel's EigenTT layer ([`kernel/src/nbe/term.rs`](../../kernel/src/nbe/term.rs)) has full inductive-type support: `Exp::InductiveType(decl, args)`, `Exp::InductiveCtor(decl, ctor_name, args)`, declarations carry typed parameters and ctor arg types via `Exp` shapes, and the NBE recursor ([`kernel/src/nbe/recursor.rs`](../../kernel/src/nbe/recursor.rs)) dispatches case branches against ctors. A user program written in ESL can declare `data Nat = Zero | Succ(Nat)` and pattern-match on it. **EigenTT itself is not the problem**; the problem is bringing this expressivity onto the chain so chain-committed Resources can carry typed inductive values.

### 2.2 EigenTT `Exp` is already the right term language

`Exp` ([term.rs:28](../../kernel/src/nbe/term.rs#L28)) is a fully-elaborated dependent term language with `Var(Name)` for variables, `Lam(Patt, body)` and `Pi(Patt, ty, body)` for binders (the binder's type slot is where variable types live — patterns themselves are unannotated, [term.rs:274](../../kernel/src/nbe/term.rs#L274)), `App(head, arg)` for application, plus the codata, identity-type, and Eigon-aware extensions. The "symbol-algebra fragment" (Var, App, Lam, Pi, plus literal and operator-reference primitives) is a structural subset of `Exp`. Defining `FormulaTerm` to mirror that subset means **the formula language is *literally a fragment of EigenTT*** — no impedance mismatch with the kernel's evaluator, and a comorphism's `m` can be written in actual EigenTT.

### 2.3 The existing chain ontology

[`core:InductiveType`](../../ontologies/core/core-ontology.json) requires `is_a`, `short_name`, and `ctors` (an ordered list of `InductiveCtor` resources). [`core:InductiveCtor`](../../ontologies/core/core-ontology.json) requires only `ctor_name`. The ontology already declares `core:type_params` (currently unused — Verdict has none), reserved for parametric inductives. The single existing inductive on the chain — `urn:eigenius:institution:Verdict` — has three ctors with no arguments.

### 2.4 The mirror generator (D29) emits Julia for `Class` only

[`JuliaMirrorGenerator`](../../crates/eigenius-julia/src/mirror_gen.rs) walks the closure of a `Class` resource (following `requires` → `Property` definitions → `data_type` / `class_types` → referenced classes), emitting a Julia `struct` per class with `decode_<C>` and `encode_<C>` functions plus a `_eigenius_decoders` registry. `InductiveType` resources are not visited, and there's no Julia emission story for them. The resource type closure walker would need a parallel branch for `InductiveType` references.

## 3. Design — EigenTT inductives reach the chain

### 3.1 Existing ontology surface — already in place

A first-pass survey of [`ontologies/core/core-ontology.json`](../../ontologies/core/core-ontology.json) reveals the chain ontology *already declares* the inductive-ctor-arg surface, used internally by ESL's compiler ([`kernel/src/esl/compile.rs`](../../kernel/src/esl/compile.rs)) when lowering `data` declarations like `data Nat = zero | succ(Nat)`. The shape:

| Resource | Required properties | Recommended properties |
|---|---|---|
| `core:InductiveType` | `short_name`, `ctors` | `type_params` |
| `core:InductiveCtor` | `ctor_name` | (today: nothing — `arg_types` is *not* required, so nullary ctors like `Verdict::Holds` validate without it) |
| `core:InductiveArgType` | `type_name` | `type_args` |
| `core:InductiveParam` | `param_name`, `param_kind` | (`param_kind` is currently `core:Set` only — Phase 11b admits one universe) |

Plus the corresponding properties: `core:ctors`, `core:type_params`, `core:ctor_name`, `core:arg_types`, `core:type_name`, `core:type_args`, `core:param_name`, `core:param_kind`.

**The shape is more powerful than what an earlier draft of this document proposed.** `InductiveArgType.type_name` is a string that's *overloaded* — it carries either a class IRI (`"urn:eigenius:example:Nat"`) or a bare type-parameter name (`"A"`) — and `type_args` carries parametric application (`List(A)` is `{type_name: "...:List", type_args: [{type_name: "A"}]}`). Parametric inductives are already supported on the chain; an earlier draft of this design (which proposed deferring them) was reinventing surface that already existed.

The ESL compiler tests in [`kernel/src/esl/compile.rs`](../../kernel/src/esl/compile.rs) (`compile_data_nat_yields_inductive_type_with_one_param`, `compile_data_list_parametric_records_param_references_as_bare_names`) exercise both the monomorphic and parametric forms end-to-end and round-trip through this Resource shape. The surface is ESL-compiler-only today — no chain-validator rule consumes it, no mirror generator emits Julia for it, no JSON-authored ontology layer uses it — but the *declaration* shape itself is fully baked.

### 3.2 The one missing addition — `core:arg_name`

The existing `InductiveArgType` shape is **positional only**: `arg_types: [InductiveArgType, InductiveArgType, ...]` is an ordered list with no per-slot name. ESL's compiler consumes this fine because it carries the source-position pattern alongside (`succ(n)` → `arg_types[0]`'s ESL-side slot is named `n`); but a JSON-authored ontology layer (e.g. the `formulas:` layer this design drives) has no such side channel, and a mirror generator emitting Julia structs would have to fall back to positional names (`arg_0`, `arg_1`, …) that obscure the ctor's intent.

The fix is small: add an optional `core:arg_name` property to `InductiveArgType`.

```json
{
  "@id": "urn:eigenius:core:arg_name",
  "core:is_a": ["core:Property"],
  "core:short_name": "arg_name",
  "core:description":
    "Optional readable name for an InductiveArgType slot (e.g. `head`, `tail`, `name`, `body`). Mirror generators use it for typed-language struct field names; chain readers use it for diagnostics. ESL-compiled inductives may omit it (the compiler carries the source-position pattern through other channels).",
  "core:data_type": "core:string",
  "core:domain": ["core:InductiveArgType"]
}
```

Recommended-not-required, for two reasons: (1) ESL's compiler-emitted ctors don't set it and shouldn't have to, (2) a mirror generator or JSON author can leave it off and accept positional fallback names if they don't care.

### 3.3 The `type_name` discipline at v1

`type_name` is a string that the validator must classify before recursing:

- **Class IRI** (resolves to a `core:Class` resource): the value is an embedded resource or `ResourceRef` whose `is_a` matches the Class.
- **InductiveType IRI** (resolves to a `core:InductiveType` resource): the value is itself an inductive value (the tagged-dict shape — see §3.5 below). Self-reference (the parent inductive's own IRI) is permitted — this is recursion. Cross-inductive cycles are also permitted (mutually recursive types).
- **Primitive type IRI** (`core:float`, `core:integer`, `core:string`, `core:boolean`, `core:iri`, `core:bytes`, `core:json`): the value is the corresponding primitive `Value` shape.
- **Bare type-parameter name** (e.g. `"A"`, no `urn:` scheme): the inductive carries a matching `InductiveParam`. Type-parameter resolution happens at the use site (when the parametric inductive is *applied* to a type argument); the validator records the parameter name and substitutes when it walks an `InductiveArgType` carrying `type_args`.

The validator's classification rule is "if `type_name` parses as an IRI and resolves on the chain, dispatch on the resolved resource's `is_a`; otherwise treat as a parameter name." This matches ESL's compiler convention.

### 3.4 Concrete example — `FormulaTerm`'s `App` ctor

Using the existing surface plus the new `arg_name`:

```json
{
  "@id": "urn:eigenius:formulas:ctor:App",
  "core:is_a": ["core:InductiveCtor"],
  "core:ctor_name": "App",
  "core:arg_types": [
    {
      "core:is_a": ["core:InductiveArgType"],
      "core:arg_name": "head",
      "core:type_name": "urn:eigenius:formulas:FormulaTerm"
    },
    {
      "core:is_a": ["core:InductiveArgType"],
      "core:arg_name": "arg",
      "core:type_name": "urn:eigenius:formulas:FormulaTerm"
    }
  ]
}
```

For an `Add` ctor that takes a variadic list of subterms, the existing `type_args`-based parametric-application surface lets us write it without inventing new shorthand:

```json
{
  "@id": "urn:eigenius:formulas:ctor:Add",
  "core:is_a": ["core:InductiveCtor"],
  "core:ctor_name": "Add",
  "core:arg_types": [
    {
      "core:is_a": ["core:InductiveArgType"],
      "core:arg_name": "args",
      "core:type_name": "urn:eigenius:core:List",
      "core:type_args": [
        {
          "core:is_a": ["core:InductiveArgType"],
          "core:type_name": "urn:eigenius:formulas:FormulaTerm"
        }
      ]
    }
  ]
}
```

This requires `core:List` to be a chain-committed parametric `InductiveType` (a hand-rolled `data List(A) = nil | cons(A, List(A))`). v1 declares it once and consumes it everywhere — no separate cardinality discipline needed.

### 3.5 Validator semantics

The kernel's commit-time validator gains one new rule: **inductive-value type checking**.

#### 3.2.1 The rule

When a property's `data_type` is `core:inductive` (a new primitive type — see §3.7) and its value is an inductive-value tree, the validator:

1. Reads the value's top-level `ctor` field.
2. Resolves the property's declared inductive type (via `class_types`, which now accepts `InductiveType` references) and looks up the ctor on that type's `ctors` list.
3. If the ctor isn't declared on the inductive, errors with `InductiveCtorMismatch`.
4. For each argument declared in the ctor's `ctor_args`, recursively validates the corresponding `args[i]` value:
   - **Primitive type:** match the value's `Value` shape against the primitive's wire form. Same checks as today's `data_type` validation.
   - **Class type:** value is `ResourceRef` or embedded resource matching the Class. Reuses the existing class-types rule.
   - **InductiveType:** recurse into rule (1).
   - Cardinality `list`: the value is `Value::Array`; each element matches `arg_type`. Cardinality `single`: the value is a single value matching `arg_type`.
5. Errors aggregate with structured field paths so the user sees `term.args[0].args[1]: expected FormulaTerm.Var, got FormulaTerm.LitFloat`.

#### 3.2.2 Termination

The validator's recursion is structural over the value tree. The chain-side termination guarantee is value-side, not type-side: a value tree with N nodes induces at most N validator calls. Type-level positivity (the kernel-layer guarantee that the recursor terminates) is checked when the value is reified into EigenTT `Exp` for evaluation — out of scope for the validator, which is purely structural.

### 3.6 Mirror generator extensions

The mirror generator's closure walker gains an `InductiveType` branch.

#### 3.3.1 Closure walk

When walking the closure for a seed `Class` (or, eventually, a seed `InductiveType`), the walker visits every `Class` and `InductiveType` reachable through `requires` → `Property` → `class_types` and `requires` → `Property` → `arg_type` references. For each visited `InductiveType`, it pulls the type's `ctors` list and, for each ctor, the `ctor_args` references — terminating at primitive types and classes already in the closure.

#### 3.3.2 Emitted Julia for an `InductiveType`

For an inductive `T` with ctors `C₁(...args), C₂(...args), ...`:

```julia
abstract type T end

struct C₁ <: T
    arg_name_1::Julia(arg_type_1)
    arg_name_2::Julia(arg_type_2)
    ...
end

struct C₂ <: T
    ...
end
```

`Julia(arg_type)` follows the existing primitive-type ladder for primitives, the existing struct emission for classes, and recurses for inductives. `cardinality: list` produces `Vector{Julia(arg_type)}`.

For self-referential inductives (e.g. `FormulaTerm` containing `FormulaTerm`s), Julia's forward-reference discipline applies: the abstract type comes first, all ctors reference the abstract type via `<: T`, recursive fields are typed `T` (not the concrete ctor) so any concrete sub-ctor satisfies the field type.

#### 3.3.3 Decode / encode

```julia
function decode_T(d::Dict)::T
    ctor = d["ctor"]
    args = d["args"]
    if ctor == "C₁"
        return C₁(decode_arg_type_1(args[1]), decode_arg_type_2(args[2]), ...)
    elseif ctor == "C₂"
        ...
    else
        error("unknown ctor `$ctor` for inductive T")
    end
end

function encode_T(v::T)::Dict
    if v isa C₁
        return Dict("ctor" => "C₁", "args" => [encode_arg_type_1(v.arg_name_1), ...])
    elseif v isa C₂
        ...
    end
end
```

Both functions register into the existing `_eigenius_decoders` / `_eigenius_encoders` global maps so the worker dispatcher resolves inductive-typed inputs the same way it resolves class-typed ones today.

### 3.7 Eigon-CBOR persistence shape for inductive values

A new primitive type `core:inductive` joins the existing primitive ladder. A property with `data_type: core:inductive` and `class_types: [<InductiveType IRI>]` carries a value whose CBOR/JSON shape is:

```json
{
  "ctor": "<ctor_name>",
  "args": [<arg₁>, <arg₂>, ...]
}
```

Each `argᵢ` is itself encoded according to the corresponding `ctor_args[i].arg_type`:

- Primitive types: as the existing primitive Eigon-CBOR encoding.
- Class refs: as a `ResourceRef` (the existing canonical class-ref encoding) or an embedded resource map.
- Inductive refs: recursively as `{ "ctor": ..., "args": [...] }`.
- `cardinality: list`: as a CBOR array of the encoded element shape.

This keeps the wire format JSON-shaped — Eigon-JSON producers can author it by hand, and `Value::Json` in the existing kernel handling already round-trips it through the validator and storage layer with no shape change. The new primitive type `core:inductive` is what triggers the *type-checking* of that JSON value against the declared inductive schema.

## 4. The first consumer — `FormulaTerm`

### 4.1 Constructor list

`FormulaTerm` is committed under the **`urn:eigenius:formulas:`** namespace — *not* under `urn:eigenius:symbolics:` — because it is the shared formula language across every numerical institution. The Symbolics handler is its first user, but `IntervalArithmetic`, `JuMP`, `DiffEq`, and `Catalyst` all consume the same shape.

```json
{
  "@id": "urn:eigenius:formulas:FormulaTerm",
  "core:is_a": ["core:InductiveType"],
  "core:short_name": "FormulaTerm",
  "core:description":
    "Symbol-algebra-relevant fragment of EigenTT Exp, lifted to the chain. The shared formula language across every numerical institution. Constructors mirror Exp::Var, Exp::App, Exp::Lam, Exp::Pi one-for-one. Variables are introduced by Lam/Pi binders whose type slot carries the variable's type — same discipline as EigenTT itself. Free vars in an open expression are typed by the institution's ambient context (the dispatch's input typing or the operator catalog).",
  "core:ctors": [
    "urn:eigenius:formulas:ctor:Var",
    "urn:eigenius:formulas:ctor:LitFloat",
    "urn:eigenius:formulas:ctor:OpRef",
    "urn:eigenius:formulas:ctor:App",
    "urn:eigenius:formulas:ctor:Lam",
    "urn:eigenius:formulas:ctor:Pi"
  ]
}
```

The ctors:

| Ctor | Args | Mirrors | Use |
|---|---|---|---|
| `Var` | `name: string` | `Exp::Var(Name)` | Free or binder-bound variable. |
| `LitFloat` | `value: float` | `Exp::EigonResource(Float-typed Resource)` (effectively) | Numeric literal. Future ctors `LitInt`, `LitRational` extend the numeric ladder; v1 ships float only. |
| `OpRef` | `iri: iri` | `Exp::Var` resolving to a chain-committed operator | Reference to an entry in the operator catalog (§5). The reason institutions agree on `add`, `sin`, etc. without baking them into the term language. |
| `App` | `head: FormulaTerm`, `arg: FormulaTerm` | `Exp::App(head, arg)` | Application. Multi-arg operators land via curried application: `add(x, 2)` is `App(App(OpRef("add"), Var("x")), LitFloat(2.0))`. |
| `Lam` | `name: string`, `ty: FormulaTerm`, `body: FormulaTerm` | `Exp::Lam(Patt::Var(name), body)` with type carried alongside | Typed binder. The `ty` slot is a `FormulaTerm` rather than a separate `TypeExpr` because operator catalog entries (Real, Int, Real → Real) are themselves FormulaTerms — see §5. |
| `Pi` | `name: string`, `ty: FormulaTerm`, `body: FormulaTerm` | `Exp::Pi(Patt::Var(name), ty, body)` | Dependent function type. Used to declare operator signatures (`Pi(_, Real, Real)` is `Real → Real`). |

### 4.2 Naming convention follows EigenTT

Variable names are EigenTT `Name`s — strings. No de Bruijn indices on the chain. `Lam`/`Pi` introduce a name; nested `Var(name)`s in the body are bound by the nearest enclosing binder of the same name (EigenTT's α-equivalence discipline). This matches what the kernel evaluator expects when it reifies a `FormulaTerm` value into an `Exp` for type-checking or evaluation.

### 4.3 Why `FormulaTerm` is not declared under Symbolics

Putting `FormulaTerm` under `urn:eigenius:formulas:` rather than `urn:eigenius:symbolics:` is the comorphism-readiness move:

- Every institution that consumes formulas imports the same ontology layer, gets the same Julia struct after mirror generation, and dispatches on the same typed Julia type. No per-institution mirror translation.
- A comorphism's `m` (EigenTT Component) is a function `FormulaTerm → FormulaTerm` — typed input, typed output, both endpoints validated against the same chain-committed `InductiveType`. Without a shared declaration, comorphisms would have to type their `m`s against institution-specific types (Symbolics' `SymbolicTerm`, JuMP's `JumpExpr`, etc.) and the chain validator couldn't cross-check the two ends.
- Future Lean integration (Phase 20) gets the same surface — `FormulaTerm` decodes into Lean's `Expr`-like representation without going through a bespoke Symbolics-specific bridge.

Symbolics-specific resources (`SymbolicExpression`, `SymbolicallyReducesTo`, etc.) live under `urn:eigenius:symbolics:` and reference `formulas:FormulaTerm` for their term carrier. Same pattern for every other formula-consuming institution.

## 5. The operator catalog

### 5.1 The pinned design question

Should the operator catalog (`formulas:ops:add`, `formulas:ops:mul`, `formulas:ops:sin`, ...) carry on-chain EigenTT type signatures? Or are operators just IRIs whose semantics each institution's handler interprets opaquely?

### 5.2 Pinned answer — yes, operators carry typed signatures

**Yes — operators carry on-chain EigenTT type signatures.** That's what makes `FormulaTerm` a *typed* term language rather than a naive S-expression. Three concrete benefits:

- **Validator-side rank check at commit time.** When a `SymbolicExpression.term` arrives carrying `App(App(OpRef("formulas:ops:add"), Var("x")), LitFloat(2.0))`, the validator looks up `formulas:ops:add`, reads its declared signature `Real → Real → Real`, and checks the `App` chain matches: argument 1 has type `Real` (free `Var("x")` deferred to the ambient context's type binding; the validator allows it but flags it), argument 2 has type `Real` (`LitFloat` matches). Mismatched arity (`add(x, y, z)`) and mismatched arg types (`add(x, "hello")`) reject at commit time, not at dispatch time.
- **Cross-institution type discipline.** A comorphism `Symbolics → IntervalArithmetic` carrying `m : FormulaTerm → FormulaTerm` is type-checked against the operator signatures both institutions declare. If Symbolics' handler produces `App(OpRef("formulas:ops:my_obscure"), x)` and IntervalArithmetic doesn't have an interval-extension for `my_obscure`, the chain validator (or the substrate-side dispatch) catches it before the worker spawns.
- **Generated Julia gets typed dispatch.** The mirror generator emits `Julia(formulas:ops:add)` as a Julia method specialised on `(Real, Real) → Real`. Symbolics' handler dispatches on the typed inputs naturally; IntervalArithmetic's handler dispatches on `(Interval, Interval) → Interval`. The institution's job is to provide *one method per operator per signature*, not to parse strings.

### 5.3 `Operator` resource shape

```json
{
  "@id": "urn:eigenius:formulas:Operator",
  "core:is_a": ["core:Class"],
  "core:short_name": "Operator",
  "core:description":
    "A function symbol in the FormulaTerm language. Carries a typed EigenTT signature so chain validation can rank-check App invocations and so cross-institution comorphisms can type-check at the chain boundary.",
  "core:requires": [
    "core:short_name",
    "formulas:operator_signature"
  ],
  "core:recommends": [
    "core:description",
    "formulas:operator_arity",
    "formulas:operator_associativity",
    "formulas:operator_commutativity"
  ]
}
```

Properties:

| Property | Type | Description |
|---|---|---|
| `formulas:operator_signature` | `core:inductive`, `class_types: [formulas:FormulaTerm]` | The operator's EigenTT type, encoded as a `FormulaTerm` value. For `add`: `Pi(_, Real, Pi(_, Real, Real))`. The `Pi` ctor's binders may be unnamed (`_`) when the operator is non-dependent. |
| `formulas:operator_arity` | `core:integer` | Convenience property; redundant with `operator_signature` for non-dependent operators. The validator may use it as a fast-path check before walking the full signature. |
| `formulas:operator_associativity` | `core:string` (`allows_only: ["left", "right", "none", "n_ary"]`) | Algebraic discipline. `n_ary` lets institutions flatten `App(App(add, x), y), z` into `Add([x, y, z])` internally; the chain shape stays curried. |
| `formulas:operator_commutativity` | `core:boolean` | Convenience for normal-form computation; not enforced by the validator. |

The signature is itself a `FormulaTerm`. That's intentional — it dogfoods the type language. When the validator looks up `formulas:ops:add.operator_signature` and gets back `Pi(_, OpRef("formulas:types:Real"), Pi(_, OpRef("formulas:types:Real"), OpRef("formulas:types:Real")))`, it walks that signature using the same recursion that validates terms. The "type catalog" (`formulas:types:Real`, `formulas:types:Int`, etc.) is just operators with arity-zero signatures — types are nullary functions in this view, the same trick EigenTT uses internally.

### 5.4 Validator's `App` rank check

When the validator descends into a `FormulaTerm` value and encounters an `App(head, arg)` node:

1. Walk left-spine to find the top-level operator: the `head` of the leftmost `App`. If it's an `OpRef(iri)`, resolve `iri` to an `Operator` resource, read its `operator_signature`. If it's a `Var(name)`, the operator is bound in the ambient context (free var) and rank-checking defers to dispatch.
2. Count the App-spine arguments — `App(App(App(OpRef, a₁), a₂), a₃)` carries three.
3. For each argument, descend the operator's `Pi` chain, taking the binder's `ty` slot as the expected type and the body as the remaining signature for the next argument. Recursively validate `aᵢ` against the expected type.
4. If the App spine has more arguments than the signature has Pis (or fewer), reject with `OperatorArityMismatch`.

### 5.5 Initial operator + type catalog

Committed in the `urn:eigenius:formulas:` ontology layer alongside the `FormulaTerm` declaration. v1 set, sized to what Symbolics + IntervalArithmetic need:

**Type-as-operator entries** (arity-zero):
- `formulas:types:Real` — signature `Real` (terminal).
- `formulas:types:Int` — signature `Int` (terminal). Reserved.
- `formulas:types:Bool` — signature `Bool`. Reserved.

**Arithmetic** (`Real → Real → Real`):
- `formulas:ops:add`, `formulas:ops:sub`, `formulas:ops:mul`, `formulas:ops:div`, `formulas:ops:pow`.

**Unary numeric** (`Real → Real`):
- `formulas:ops:neg`, `formulas:ops:exp`, `formulas:ops:log`, `formulas:ops:sin`, `formulas:ops:cos`, `formulas:ops:tan`, `formulas:ops:sqrt`, `formulas:ops:abs`.

**Comparisons** (`Real → Real → Bool`, reserved for the `Decidable` role and for institutions that need them):
- `formulas:ops:eq`, `formulas:ops:lt`, `formulas:ops:le`.

**Calculus** (`(Real → Real) → Real → Real` for `derivative` evaluated at a point; future expansion gives the structurally-derivative form):
- `formulas:ops:derivative` — signature `Pi(f, Real → Real, Pi(x, Real, Real))`. Symbolics' AD interprets `derivative` symbolically; IntervalArithmetic interprets it via interval-Newton.

The catalog grows incrementally — when DiffEq lands (Phase 19g), it adds `formulas:ops:int_definite` and friends. When Catalyst lands, it adds `formulas:ops:rate_law_term`. Adding an operator is a chain commit, not a code change.

### 5.6 Institutions add operators by committing

An institution authoring its handler package commits new `Operator` resources to the chain at registration time, alongside its `Institution` resource. The institution's handler implements one Julia method per operator-IRI per operand-type-signature — exactly the same discipline as the existing intervals handler today, just dispatched on a typed operator IRI rather than implicit operator names. Operators that lack a method on the dispatching institution surface a typed `OperatorNotImplemented` at dispatch time (not at validator time — the validator type-checks the *shape*, not coverage).

## 6. Comorphism implications

### 6.1 Comorphism `m` becomes a EigenTT function

[D14 §5](d14-institution-realisation.md) defines a comorphism as a triple `(s, m, t)`: source ExportFormat, EigenTT Component `m`, target ImportFormat. With `FormulaTerm` as the shared payload, `m`'s chain-side type becomes `FormulaTerm → FormulaTerm` — a EigenTT function written in actual EigenTT. Two consequences:

- **The kernel type-checks `m`.** Since `m` is a `Component` (EigenTT term), the kernel's existing component-typing machinery handles it; no new boundary-typing rule needed.
- **Composition is free.** Two comorphisms `m₁: A → B` and `m₂: B → C` compose into `m₂ ∘ m₁: A → C` by EigenTT's existing function composition. EigenQL FIBER queries that traverse comorphism chains can simply compose the `m`s symbolically.

### 6.2 Concrete example — Symbolics → IntervalArithmetic

A `Comorphism` that lets an interval-arithmetic handler accept a Symbolics expression as its function argument:

```json
{
  "@id": "urn:eigenius:formulas:comorphisms:symbolics_to_interval_function",
  "core:is_a": ["institution:Comorphism"],
  "institution:source_export": "urn:eigenius:symbolics:export_formats:ef_symb_expr",
  "institution:target_import": "urn:eigenius:intervals:import_formats:if_intv_function",
  "institution:component": "urn:eigenius:formulas:comorphisms:identity_term"
}
```

Both ExportFormat and ImportFormat carry `FormulaTerm` payloads, so `m` can be the *identity function* on FormulaTerm. The Symbolics handler extracted a `SymbolicExpression` to a `FormulaTerm`; IntervalArithmetic's handler reifies the same `FormulaTerm` as a callable `function` for `qc_intv_compute_bounds`. The chain validator confirms both ends typecheck, and the comorphism passes through with no transformation. **This is the cleanest cross-institution transfer the design admits** — and it falls out of having a shared term language with typed operators, not from any per-institution glue.

## 7. Migration path

### 7.1 Deliverables, in order

Work breaks into four landings, each independently shippable:

1. **`core:arg_name` ontology addition.** Add the optional `arg_name` property on `core:InductiveArgType` (§3.2). Existing ESL-emitted ctors validate unchanged (the property is recommended-not-required). Triggers a `seed_manifest_v1` bump; existing dev DBs need a `docker compose down -v` re-seed.
2. **Validator + persistence — `core:inductive` primitive type.** Adds the new primitive type, the inductive-value type-check rule against the existing `arg_types` schema (§3.5 + §3.7), and the `class_types: [InductiveType]` extension. Smoke-tested against a hand-rolled `Nat = zero | succ(Nat)` end-to-end.
3. **Mirror generator extension.** Closure walker visits `InductiveType` references through `arg_types[].type_name` and `arg_types[].type_args`, emit Julia abstract type + per-ctor structs + decode/encode/registry. Smoke-tested against `Nat` (round-trips a `succ(succ(zero))` value through the substrate's image-build pipeline).
4. **Structural inductives + `formulas:` ontology layer.** Commits the structural parametric inductives (`core:List`, `core:Option`, `core:Pair`) under the core layer alongside `FormulaTerm`, `Operator`, the type-as-operator entries, and the arithmetic/unary/comparison/calculus operator set under the new `urn:eigenius:formulas:` layer. First consumer (Symbolics, Phase 19d) follows in its own crate.

The first three landings are dependency-ordered (each unblocks the next). Landing four depends on all three. Symbolics (Phase 19d) and the comorphism wiring depend on landing four.

### 7.2 Backward compatibility

**Verdict still works.** Its three ctors declare no `ctor_args`; the validator's new rule treats an empty `ctor_args` list as "no per-arg checks." Existing Verdict-emitting institutions (intervals + future) need no changes.

**Existing Class mirrors still work.** The mirror generator's `InductiveType` branch is additive — Class closure walking is unchanged.

**ESL programs still work.** ESL's compile path produces `Exp` directly (it does not round-trip through the chain's inductive-value shape) — the chain-side `core:inductive` shape is for *resource property values*, not for ESL program sources.

## 8. Decisions

These three were carried as open questions through the first design pass and are now settled. Each is stated firmly with the trigger that would justify revisiting.

### 8.1 Parametric inductives stay deferred

**Decision.** Parametric inductives are *already supported* on the chain via the existing `core:type_params` + `core:type_args` surface (§3.1). `List<T>`, `Option<T>`, `Pair<A, B>` etc. are expressible by declaring a parametric `core:InductiveType` (e.g. `data List(A) = nil | cons(A, List(A))`) and referencing its parametric application in any consumer's `core:arg_types`. v1 commits a small set of structural parametric inductives (`core:List`, `core:Option`, `core:Pair`) alongside the `formulas:` layer so consumers don't need to re-declare them.

**Why this is now a "non-question".** An earlier draft of this document deferred parametric inductives behind a `cardinality` shorthand on `InductiveCtorArg`. The chain ontology survey (§3.1) revealed that ESL's compiler already produces `InductiveArgType` resources with `type_args` for parametric application — the surface was reinventable but *also already done*. Using the existing shape eliminates the deferral entirely.

**Revisit when.** Never expected for parametric inductives themselves. The follow-on questions (kind polymorphism beyond `Set`, higher-rank parameters) remain open at the kernel layer per [D19](d19-inductive-types.md) — the chain shape doesn't constrain them.

### 8.2 Dispatch stays QueryClass-keyed; no operator-level dispatch

**Decision.** The substrate dispatches on the `QueryClass` IRI (and its `query_handler` → `RuntimeMethodSignature` chain), not on the `OpRef` content of the `FormulaTerm` payload. A `qc_symb_simplify(SymbolicExpression)` query goes to Symbolics; `qc_intv_compute_bounds(SymbolicExpression, domain)` goes to IntervalArithmetic. The same `FormulaTerm` value can flow into both, but each institution sees it under its own QueryClass.

**Why.** This is the discipline [D14 §6](d14-institution-realisation.md) establishes for institution dispatch and the substrate already implements it ([D26 §5.3](d26-runtime-substrate.md)). Adding an "operator-level" routing layer that picks an institution from the term's `OpRef` content would (a) duplicate dispatch state across two surfaces, (b) make multi-operator terms (`add(sin(x), mul(y, z))`) ambiguous to route, and (c) break the `Comorphism`-as-typed-translation story — comorphisms exist precisely because the same operator means different things in different institutions, and the institution context is what disambiguates.

**Revisit when.** Never expected. The decision is structural to D14, not a v1 simplification.

### 8.3 `FormulaTerm` is data; no `quote`/`unquote` in v1

**Decision.** A `FormulaTerm` value is data on the chain. The kernel does not evaluate it directly; institutions decode it into their target language and operate there. Comorphism `m`s are EigenTT functions over `FormulaTerm` data values (the `Data` shape from §3.7), not over `Exp`.

**Why.** The data-only stance keeps the chain-side semantics narrow and well-defined: a `FormulaTerm` is exactly what its constructor schema says it is. Adding `quote`/`unquote` would couple `FormulaTerm` to EigenTT's evaluation discipline (positivity checks, neutral-form handling, NBE termination) and force the chain validator to reason about syntax-vs-value distinctions that aren't load-bearing for the institution dispatch path. Institutions already own evaluation in their target language; the kernel doesn't need a parallel evaluator over `FormulaTerm`.

**Revisit when.** A use case appears that genuinely benefits from kernel-side normalisation of `FormulaTerm` values. The most plausible trigger is a `Decidable` QueryClass that wants to reduce a `FormulaTerm`-shaped predicate during type-check evaluation without round-tripping through an institution worker — e.g., constant folding (`add(LitFloat(1.0), LitFloat(2.0))` → `LitFloat(3.0)`) inside `Exp::NativeDecide`. We'd add `quote: Exp → FormulaTerm` and `unquote: FormulaTerm → Exp` then.

**Forward-compatibility.** Adding `quote`/`unquote` later is strictly additive — existing `FormulaTerm` data values stay valid, the institution dispatch path is unchanged, and the new pair is opt-in for kernel-side evaluators that want it.

## 9. References

- [D14 — Institution Realisation](d14-institution-realisation.md) — Comorphism mechanic, EigenTT Component as `m`.
- [D19 — Inductive Types](d19-inductive-types.md) — EigenTT inductive support, recursor, positivity.
- [D26 — Runtime Substrate](d26-runtime-substrate.md) — RuntimePackageMirror, env image, ServiceSpawner.
- [D27 — Julia Institutions §4.1 — Symbolics](d27-julia-institutions.md) — first consumer's resource classes.
- [D29 — Eigon-Julia Mirror Spec](d29-eigon-julia-mirror-spec.md) — current Class-only mirror discipline; this design extends it.
- [D24 — Schema Versioning](d24-schema-versioning.md) — ontology bump procedure for the `core:` extension.
