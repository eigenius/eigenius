# D47: Chain-Mirrored EigenTT Type Fragment

*Design document for the Eigenius project — June 2026*

**Status:** Draft
**Required before:** D46 Phase H (axiom-as-Resource framework)
**Depends on:** D32 (chain-mirrored EigenTT inductives — provides the infrastructure this reuses)
**Unblocks:** D46 Phase H, any future feature that needs to commit EigenTT *types* to the chain as queryable values (axiom statements, theorem statements, schema-level constraints, propositional institutions other than D39)

---

## 1. Motivation

### 1.1 The hole D46 found

D46 §10 introduces a `core:Axiom` Resource class whose key property — `core:axiom_statement` — must carry a **EigenTT type**: the proposition the axiom inhabits. Examples:

- `propext : ∀ {P Q : Prop}, (P ↔ Q) → P = Q`
- `Quot.sound : ∀ {α : Type} {r : α → α → Prop} {a b : α}, r a b → Quot.mk r a = Quot.mk r b`

These are closed EigenTT terms of type `Sort(n)` for some `n`. They mention universe sorts (`Prop`, `Type`), dependent binders (`∀`), application (`P ↔ Q`), propositional equality (`P = Q`), and references to declared constants (`Iff`, `Quot`, `Quot.mk`).

Eigenius's existing chain mirrors fit the type-language poorly:

- **D32's `formulas:FormulaTerm`** covers the symbol-algebra fragment of EigenTT (`Var`, `App`, `Lam`, `Pi`, `LitFloat`, `OpRef`). It has `Pi`, but no `Sort` and no `Id`. D32 §4.3 explicitly scopes FormulaTerm as "the minimum common subset across numerical institutions" — stretching it to cover Sort/Id would violate that charter.
- **D40's `lean:LeanExpr`** mirrors Lean's term language. Wrong target — uses Lean's `Const`/`Proj`/`Local` semantics, dotted Names, and `LeanLevel` with `Max`/`IMax`/`Param`. Forcing Eigenius's EigenTT types through a Lean-shaped mirror would import substantial Lean-specific accidents.

So we add a third chain mirror, scoped to EigenTT's *type-level subset* — exactly what's needed to express axiom statements, theorem statements, and any other Prop/Type-valued EigenTT term that wants to live on the chain as a queryable artifact.

### 1.2 Why this is small

D32 brought the chain-inductive infrastructure online (declaration mechanism, validator-walks-against-ADT-schema, value encoding, Eigon-CBOR persistence shape). D47 reuses all of it. The new work is:

- An ontology declaration for `core:EigenTTType` with its ctors.
- An encoder `Exp → EigenTTType value` covering the type-level subset.
- A decoder `EigenTTType value → Exp` (inverse).
- Tests.

Estimated total: ~1 week. The same pattern as FormulaTerm, but the chain-inductive mechanism is no longer being built for the first time.

### 1.3 Scope

In scope: a chain inductive `core:EigenTTType` covering the subset of `kernel/src/nbe/term.rs::Exp` that can appear in **closed type expressions** suitable for axiom and theorem statements. Plus encoder/decoder + validator integration.

Out of scope:
- Proof terms (axiom statements are types, not proofs — proofs aren't chain-mirrored, matching D40's "propositions only" choice).
- Open terms with free variables outside the binder structure (axiom statements are closed).
- EigenTT term forms that only make sense in proofs (`Refl`, `IdJ`, `NativeDecide`, `DecEq` — these construct or eliminate inhabitants, not types).
- Inductive-declaration ctors / recursor forms (`InductiveCtor`, `InductiveRec`, `Match`) — these are term-level, not type-level. (`InductiveType(iri, args)` *is* in scope because it appears in types, e.g., `List Nat`.)
- Codata-declaration ctors / corecord forms (same reason; `CodataType(iri, args)` is in scope.)
- Universe polymorphism (literal `usize` levels; matches D46 §4.3).

---

## 2. Today's state

### 2.1 What Exp variants are type-level

Inspecting `kernel/src/nbe/term.rs`, the variants that can appear in a closed type expression are:

| `Exp` variant | Appears in types? | Notes |
|---|---|---|
| `Sort(usize)` (post-D46) | yes | universes |
| `Pi(p, A, B)` | yes | dependent function type |
| `Sig(p, A, B)` | yes | dependent pair type |
| `Arrow(A, B)` | yes | sugar for `Pi(_, A, B)` — desugared at parse |
| `Times(A, B)` | yes | sugar for `Sig(_, A, B)` — desugared at parse |
| `One` | yes | unit type |
| `Id(A, x, y)` | yes | propositional equality |
| `Var(name)` | yes | bound type/term variable inside a binder |
| `App(h, a)` | yes | type application (`List Nat`, `Iff P Q`) |
| `Lam(p, body)` | rarely | type-level lambda (motives, indexed family parameters) |
| `EigonClass(iri)` | yes | resolved ground class type |
| `EigonPrimitive(iri)` | yes | primitive (`String`, `Integer`, …) |
| `InductiveType(decl, args)` | yes | applied inductive type former |
| `CodataType(decl, args)` | yes | applied codata type former |
| `Data(_)`, `Codata(_)` | yes | type-former declarations (anonymous Sum / Codata in a type position) |
| Everything else | no | term-level only |

Note `Pi`/`Sig`/`Lam` carry `Patt` rather than bare names. For chain-mirroring we restrict to `Patt::Var(name)` (single name) and `Patt::Unit` (no binder name, for `Arrow`/`Times`-style anonymous binders). Pattern bindings like `Patt::Pair(...)` don't appear in type expressions.

### 2.2 Reference to declared things

Closed axiom statements name declared constants (`Iff`, `Quot.mk`, `Asserts`, …). Three reference kinds occur:

1. **EigonClass** — `Exp::EigonClass(iri)` already resolves through the chain.
2. **EigonPrimitive** — `Exp::EigonPrimitive(iri)` likewise.
3. **InductiveType / CodataType** — `Exp::InductiveType(decl, args)` / `Exp::CodataType(decl, args)` carry an `Arc<InductiveDecl>`/`Arc<CodataDecl>` in-memory. On the chain we only need the *IRI* of the declared type plus its `args`; the rehydration walks the chain and re-resolves the `Arc`.

D47's `ConstRef(iri)` ctor unifies the three: a single chain ctor that carries an IRI plus a positional argument list, decoded based on the resolved resource's class.

---

## 3. The `core:EigenTTType` inductive

### 3.1 Ctor table

| Ctor | Args | Decodes to `Exp` | Notes |
|---|---|---|---|
| `Sort` | `level: integer` | `Exp::Sort(level)` | Universe. Level 0 = Prop, 1 = Set, n+1 = Type(n). Per D46 §3.1. |
| `Var` | `name: string` | `Exp::Var(name)` | Bound variable reference. Validity (binder is in scope) is checked at decode/type-check, not on chain commit. |
| `ConstRef` | `iri: string` | `Exp::EigonClass(iri)`, `Exp::EigonPrimitive(iri)`, `Exp::InductiveType(decl, [])`, or `Exp::CodataType(decl, [])` depending on the resolved resource's `is_a` | Reference to a chain-declared type former. Always nullary — multi-arg references are built by `App` currying (e.g. `List Nat` = `App(ConstRef("core:List"), ConstRef("core:Nat"))`). The decoder dispatches on the resolved resource's class, and for parameterised types walks the enclosing `App` spine to collect the args. Choosing currying avoids needing a chain-side `core:List<EigenTTType>` declaration — `App` does the work. |
| `App` | `head: EigenTTType`, `arg: EigenTTType` | `Exp::App(head, arg)` | Type application. Multi-arg via currying (`Iff P Q` = `App(App(Iff, P), Q)`). |
| `Pi` | `name: string`, `dom: EigenTTType`, `body: EigenTTType` | `Exp::Pi(Patt::Var(name), dom, body)` | Dependent function type. Empty `name` (the zero-length string) → `Patt::Unit` (anonymous binder; equivalent to `Arrow`). |
| `Sig` | `name: string`, `dom: EigenTTType`, `body: EigenTTType` | `Exp::Sig(Patt::Var(name), dom, body)` | Dependent pair type. Empty `name` → `Patt::Unit`. |
| `Lam` | `name: string`, `dom: EigenTTType`, `body: EigenTTType` | `Exp::Lam(Patt::Var(name), body)` with type annotation carried alongside | Type-level lambda (motives, parametric definitions). Rare in axiom statements but cheap to include. The `dom` is the binder's type annotation; the kernel's `Exp::Lam` doesn't carry the type slot directly (annotations live in surrounding `Pi`), so decoding pairs the `Lam` with an inferred or supplied context. See §4.2 for the decode rule. |
| `One` | (no args) | `Exp::One` | Unit type. |
| `Id` | `ty: EigenTTType`, `lhs: EigenTTType`, `rhs: EigenTTType` | `Exp::Id(ty, lhs, rhs)` | Propositional equality. Per D46 §9, `Id` lives in `Prop`. |

Nine ctors. Matches FormulaTerm's order of magnitude (six) and is much smaller than LeanExpr's eleven.

### 3.2 Concrete example — encoding `propext`

The proposition, in standard mathematical notation:

```
propext : ∀ {P Q : Prop}, (P ↔ Q) → P = Q
```

EigenTT has no primitive `↔`; it's sugar for `(P → Q) × (Q → P)`. Expanded:

```
∀ {P : Prop}, ∀ {Q : Prop}, ((P → Q) × (Q → P)) → Id Prop P Q
```

…which uses only D47 ctors plus the EigenTT sugar `→` (`Pi` with anonymous binder) and `×` (`Sig` with anonymous binder). Encoded as a `EigenTTType` value (Eigon-JSON):

```json
{
  "ctor": "Pi",
  "args": [
    "P",
    { "ctor": "Sort", "args": [0] },
    {
      "ctor": "Pi",
      "args": [
        "Q",
        { "ctor": "Sort", "args": [0] },
        {
          "ctor": "Pi",
          "args": [
            "",
            {
              "ctor": "Sig",
              "args": [
                "",
                {
                  "ctor": "Pi",
                  "args": [
                    "",
                    { "ctor": "Var", "args": ["P"] },
                    { "ctor": "Var", "args": ["Q"] }
                  ]
                },
                {
                  "ctor": "Pi",
                  "args": [
                    "",
                    { "ctor": "Var", "args": ["Q"] },
                    { "ctor": "Var", "args": ["P"] }
                  ]
                }
              ]
            },
            {
              "ctor": "Id",
              "args": [
                { "ctor": "Sort", "args": [0] },
                { "ctor": "Var", "args": ["P"] },
                { "ctor": "Var", "args": ["Q"] }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

No `ConstRef` appears in this example because `propext`'s statement uses only built-in EigenTT constructions (universes, dependent binders, `Id`). A more complex axiom (e.g., `Quot.sound`, which references `Quot` and `Quot.mk`) would reference those via `ConstRef` against chain-committed declarations — and would only be encodable after those declarations exist.

### 3.3 Why no `Iff`, `And`, `Or`, etc. as primitive ctors

EigenTT defines these inline from its primitives:

- `P ∧ Q ≡ P × Q ≡ Sig(_, P, Q)`
- `P ∨ Q` requires a declared `Sum`-typed inductive (the core ontology can ship one, but it isn't a EigenTT primitive).
- `P ↔ Q ≡ (P → Q) × (Q → P)` (see §3.2).
- `¬ P ≡ P → False` (needs `False` declared as an inductive with no ctors — see §3.4).

If a future ontology wants `Iff`, `And`, `Or` as *named* inductive types (so they're queryable by IRI on the chain), they're declared as `core:InductiveType` resources, then referenced via `ConstRef`. `EigenTTType` is the *fragment*; the *vocabulary* lives in the chain. This matches D32's posture: `FormulaTerm` is the language; specific operators live in an operator catalog, not as ctor primitives.

### 3.4 What declarations D46 Phase H needs in the core ontology

For D46's two default-admitted axioms to encode, the core ontology needs the following declarations *prior to* committing the axiom Resources (in the same core-ontology layer, ordered before the axioms):

- `core:False` — `core:InductiveType` with zero ctors. Needed for `¬ P` if any axiom statement uses negation. (`propext` and `Quot.sound` don't, but it's the obvious next addition.)
- `core:Quot`, `core:Quot.mk`, `core:Quot.lift` — `core:InductiveType` plus ctors and a recursor for the quotient construction. Needed so `Quot.sound`'s statement can reference them via `ConstRef`. These are not added by D47; they're D46 Phase H's responsibility.

`propext` (as shown in §3.2) encodes without any new ontology declarations beyond what D47 itself adds.

### 3.5 Why no literals

For axiom statements, we don't need `LitInt` / `LitString` ctors — closed propositions don't typically embed concrete numeric or string literals at the *type* level. If a future axiom needs to mention a literal (e.g., a sized type bound), we add a `LitInt(integer)` and `LitString(string)` ctor; that's a one-line extension. For v1, omit.

### 3.6 Why no `Patt::Pair` / nested patterns

Type-expression binders don't need them in practice. ESL's surface syntax already disallows complex patterns on `Pi` / `Sig` binders for type expressions, and the kernel `Exp::Pi` is happy with `Patt::Var` / `Patt::Unit` exclusively for types. If a future axiom needs a destructuring binder in a type, we revisit.

### 3.7 The ontology declaration

```json
{
  "@id": "urn:eigenius:core:EigenTTType",
  "is_a": ["urn:eigenius:core:InductiveType"],
  "core:short_name": "EigenTTType",
  "core:description": "Chain-mirrored EigenTT type fragment. Encodes closed EigenTT type expressions for committing axiom statements, theorem statements, and other Prop/Type-valued artifacts to the chain.",
  "core:ctors": [
    "urn:eigenius:core:EigenTTType:Sort",
    "urn:eigenius:core:EigenTTType:Var",
    "urn:eigenius:core:EigenTTType:ConstRef",
    "urn:eigenius:core:EigenTTType:App",
    "urn:eigenius:core:EigenTTType:Pi",
    "urn:eigenius:core:EigenTTType:Sig",
    "urn:eigenius:core:EigenTTType:Lam",
    "urn:eigenius:core:EigenTTType:One",
    "urn:eigenius:core:EigenTTType:Id"
  ]
}
```

Each ctor declaration follows D32 §3.4's shape — `core:InductiveCtor` with `core:ctor_name` and `core:arg_types`. Recursive args use `core:type_name: urn:eigenius:core:EigenTTType`. List-typed args (only `ConstRef.args`) use the existing `core:List` parametric inductive wrapper per D32 §3.4's `Add` example.

---

## 4. Codec — `Exp ↔ EigenTTType value`

Lives in a new module `kernel/src/program/eigentt_type_mirror.rs`.

### 4.1 Encoder `encode_type(exp: &Exp) -> Result<Value, EncodeError>`

Structural recursion over the type-level subset of `Exp`:

```rust
fn encode_type(exp: &Exp) -> Result<Value, EncodeError> {
    match exp {
        Exp::Sort(n) => ctor("Sort", vec![Value::Integer(*n as i64)]),
        Exp::Var(name) => ctor("Var", vec![Value::String(name.clone())]),
        Exp::App(h, a) => ctor("App", vec![encode_type(h)?, encode_type(a)?]),
        Exp::Pi(Patt::Var(n), dom, body) =>
            ctor("Pi", vec![Value::String(n.clone()), encode_type(dom)?, encode_type(body)?]),
        Exp::Pi(Patt::Unit, dom, body) =>
            ctor("Pi", vec![Value::String("".into()), encode_type(dom)?, encode_type(body)?]),
        Exp::Sig(Patt::Var(n), dom, body) =>
            ctor("Sig", vec![Value::String(n.clone()), encode_type(dom)?, encode_type(body)?]),
        Exp::Sig(Patt::Unit, dom, body) =>
            ctor("Sig", vec![Value::String("".into()), encode_type(dom)?, encode_type(body)?]),
        Exp::Arrow(a, b) =>
            // desugar to Pi-anonymous
            encode_type(&Exp::Pi(Patt::Unit, a.clone(), b.clone())),
        Exp::Times(a, b) =>
            encode_type(&Exp::Sig(Patt::Unit, a.clone(), b.clone())),
        Exp::Lam(Patt::Var(n), body) => {
            // Lam at type level requires a type annotation; encoder requires it as context.
            // Reject if not available; type-level Lam is rare and the caller can pre-annotate.
            return Err(EncodeError::LamWithoutAnnotation);
        }
        Exp::One => ctor("One", vec![]),
        Exp::Id(ty, x, y) =>
            ctor("Id", vec![encode_type(ty)?, encode_type(x)?, encode_type(y)?]),
        Exp::EigonClass(iri) | Exp::EigonPrimitive(iri) =>
            ctor("ConstRef", vec![Value::Iri(iri.clone()), Value::Array(vec![])]),
        Exp::InductiveType(decl, args) | Exp::CodataType(decl, args) => {
            let arg_vals: Vec<Value> = args.iter().map(encode_type).collect::<Result<_, _>>()?;
            ctor("ConstRef", vec![Value::Iri(decl.iri.clone()), Value::Array(arg_vals)])
        }
        other => Err(EncodeError::NotATypeLevelExp(format!("{other:?}"))),
    }
}

fn ctor(name: &str, args: Vec<Value>) -> Result<Value, EncodeError> {
    Ok(Value::Json(serde_json::json!({
        "ctor": name,
        "args": args,
    })))
}
```

Lam at the type level is rare enough that v1 rejects rather than guessing how to invent annotations. A future v2 can carry a parallel annotation env.

### 4.2 Decoder `decode_type(value: &Value, layers: &[Arc<Layer>]) -> Result<Exp, DecodeError>`

Structural recursion mirroring the encoder, with one wrinkle: `ConstRef(iri, args)` requires layer access to dispatch on the resolved resource's class.

```rust
fn decode_type(v: &Value, layers: &[Arc<Layer>]) -> Result<Exp, DecodeError> {
    let (ctor, args) = unpack_ctor(v)?;
    match ctor.as_str() {
        "Sort" => {
            let n = arg_integer(args, 0)? as usize;
            Ok(Exp::Sort(n))
        }
        "Var" => Ok(Exp::Var(arg_string(args, 0)?)),
        "App" => Ok(Exp::App(
            Box::new(decode_type(arg_value(args, 0)?, layers)?),
            Box::new(decode_type(arg_value(args, 1)?, layers)?),
        )),
        "Pi" => decode_binder(args, layers, |patt, dom, body| Exp::Pi(patt, dom, body)),
        "Sig" => decode_binder(args, layers, |patt, dom, body| Exp::Sig(patt, dom, body)),
        "Lam" => {
            // Lam decodes structurally; the dom annotation is discarded (Exp::Lam
            // has no type slot) but kept on the chain for round-trip fidelity.
            let name = arg_string(args, 0)?;
            let _dom = decode_type(arg_value(args, 1)?, layers)?;
            let body = decode_type(arg_value(args, 2)?, layers)?;
            let patt = if name.is_empty() { Patt::Unit } else { Patt::Var(name) };
            Ok(Exp::Lam(patt, Box::new(body)))
        }
        "One" => Ok(Exp::One),
        "Id" => Ok(Exp::Id(
            Box::new(decode_type(arg_value(args, 0)?, layers)?),
            Box::new(decode_type(arg_value(args, 1)?, layers)?),
            Box::new(decode_type(arg_value(args, 2)?, layers)?),
        )),
        "ConstRef" => {
            let iri: Iri = arg_iri(args, 0)?;
            let raw_args = arg_array(args, 1)?;
            let arg_exps: Vec<Exp> = raw_args.iter()
                .map(|a| decode_type(a, layers))
                .collect::<Result<_, _>>()?;
            resolve_const_ref(iri, arg_exps, layers)
        }
        unknown => Err(DecodeError::UnknownCtor(unknown.to_string())),
    }
}

fn resolve_const_ref(iri: Iri, args: Vec<Exp>, layers: &[Arc<Layer>])
    -> Result<Exp, DecodeError>
{
    let resource = lookup_in_chain(&iri, layers)
        .ok_or(DecodeError::UnresolvedConstRef(iri.clone()))?;
    let class = resource.primary_class();
    match class.as_str() {
        "urn:eigenius:core:Class" =>
            if args.is_empty() { Ok(Exp::EigonClass(iri)) }
            else { Err(DecodeError::ClassWithArgs(iri)) },
        "urn:eigenius:core:DataType" /* primitive */ =>
            if args.is_empty() { Ok(Exp::EigonPrimitive(iri)) }
            else { Err(DecodeError::PrimitiveWithArgs(iri)) },
        "urn:eigenius:core:InductiveType" => {
            let decl = resolve_inductive_decl(&iri, layers)?;
            Ok(Exp::InductiveType(decl, args))
        }
        "urn:eigenius:core:CodataType" => {
            let decl = resolve_codata_decl(&iri, layers)?;
            Ok(Exp::CodataType(decl, args))
        }
        other => Err(DecodeError::ConstRefWrongClass(iri, other.to_string())),
    }
}
```

### 4.3 Round-trip property

For every type-level closed `Exp`, `decode_type(encode_type(e)) ≡ e` modulo:

- `Arrow(A, B)` round-trips as `Pi(Patt::Unit, A, B)` (the sugar is desugared at encode time).
- `Times(A, B)` likewise → `Sig(Patt::Unit, A, B)`.
- `Lam(Patt::Var(n), body)` round-trips as `Lam(Patt::Var(n), body)` with the type annotation preserved on the chain but not in the in-memory `Exp` (the decoder discards it; the encoder requires it as context).

Property-based tests (`proptest`) cover the closed type-level subset.

---

## 5. Validator integration

`core:EigenTTType` is a chain `InductiveType`; D32's existing validator handles the value-tree walk against the ctor schema for free. No new validator rule.

The one D47-specific check, added on top of the D32 walk, is **`ConstRef` resolution**: when a `ConstRef` value is encountered, the validator (a) verifies the `iri` resolves to a chain resource of an allowed class (`Class`, `DataType`, `InductiveType`, `CodataType`), and (b) verifies the `args.len()` matches the resolved resource's expected arity (0 for Class/DataType; matches parameter telescope for InductiveType/CodataType).

This check fires at commit time of any resource carrying a `EigenTTType`-valued property. If the resolution fails, the commit is rejected with `ConstRefUnresolved(iri)` or `ConstRefArityMismatch(iri, expected, actual)`.

Type-checking the *decoded* `Exp` against a universe (e.g., asserting the axiom statement is well-typed and inhabits `Sort(n)`) is the consumer's responsibility — D46 Phase H does this for axioms. The validator only guarantees the chain shape.

---

## 6. Eigon-CBOR persistence

Per D32 §3.7, `data_type: core:inductive` with `class_types: [core:EigenTTType]` triggers the standard chain-inductive Eigon-CBOR persistence shape:

```json
{ "ctor": "<ctor_name>", "args": [<arg₁>, <arg₂>, …] }
```

No new persistence work. The same Eigon-CBOR encoder that already handles FormulaTerm handles EigenTTType.

---

## 7. Open questions

### 7.1 Should `Lam` carry the type annotation in `Exp`?

Currently `Exp::Lam(Patt, body)` has no type slot — annotations live in the surrounding `Pi`. The chain-mirrored `Lam` does carry the annotation (for round-trip fidelity with type-level lambdas that aren't immediately surrounded by a `Pi`). If usage shows that type-level `Lam` is always paired with a context-determinable type, the chain ctor can drop the `dom` arg in v2. Defer until usage shows the pattern.

### 7.2 Should `ConstRef` also handle term-level constants?

A future consumer (e.g., a theorem-statement institution that wants to mirror theorem proofs in addition to statements) might want `ConstRef` to also resolve to `Exp::Refl(...)` or other term-level forms. For D47 v1, `ConstRef` resolves only to type-formers. Term-level references would be a separate ctor (`TermRef(iri)`?) added when needed.

### 7.3 Universe polymorphism

Sort levels are concrete `usize` integers (matching D46 §4.3's decision to defer universe polymorphism). If universe variables land later, `Sort` gains a polymorphic variant `SortVar(name: string)` referencing a binder introduced by a hypothetical universe-quantifier ctor. Not in scope here.

### 7.4 Shared subterm encoding

Closed axiom statements are small enough that DAG sharing isn't a performance concern (compare D40 §4.2, which explicitly chooses tree encoding for the same reason). EigenTTType values are trees. If a future use case commits large shared statements, we revisit.

---

## 8. Implementation plan

Estimated effort: ~1 week.

### 8.1 Phase 1 — Ontology + tests (~2 days)

- Add `core:EigenTTType` plus the 9 ctor declarations to `ontologies/core/core-ontology.json`. Triggers a `seed_manifest_v1` bump (pre-production posture per [project_posture_preproduction memory](#) — drop and re-seed).
- Verify D32's existing validator accepts and rejects sample `EigenTTType` values correctly using existing test infrastructure.

### 8.2 Phase 2 — Encoder (~2 days)

- Implement `encode_type` in `kernel/src/program/eigentt_type_mirror.rs`.
- Unit tests for each ctor.
- Sample tests encoding `propext`, `Quot.sound`, and a few D39-flavored statements.

### 8.3 Phase 3 — Decoder + `ConstRef` resolution (~2 days)

- Implement `decode_type` + `resolve_const_ref`.
- Round-trip property tests (proptest over the closed type-level subset).
- Negative tests: unresolved `ConstRef`, arity mismatch, wrong-class `ConstRef`, malformed ctor shape.

### 8.4 Phase 4 — Validator wiring (~1 day)

- Add the `ConstRef` resolution check to the commit-time validator.
- Tests verifying commits with malformed `EigenTTType` values are rejected with structured errors.

### 8.5 Phase 5 — Documentation + sample axioms (~1 day)

- Document the encoder/decoder API in module-level rustdoc.
- Commit `propext` and `Quot.sound` as encoded sample `core:EigenTTType` Eigon-JSON values in a test fixture for D46 Phase H to consume.

### 8.6 Sequencing relative to D46

D47 is independent of D46 Phases A–G (Sort ladder, impredicative Pi, proof irrelevance, singleton-elim, term repositioning). It blocks only D46 Phase H. So D47 and D46 Phases A–G can be implemented in parallel; D46 Phase H follows whichever lands last.

---

## 9. Risks

### 9.1 `Lam` type-annotation handling

The encode/decode asymmetry (encoder requires annotation context; decoder discards `dom`) is a slight wart. If type-level Lam shows up more than expected, we revisit and either add the annotation to `Exp::Lam` itself (significant kernel churn) or carry a parallel annotation map. For axiom statements specifically, type-level Lam is essentially unused — risk is low.

### 9.2 Chain-resource class drift

`ConstRef` dispatches on the resolved resource's primary class. If future ontology evolution renames or splits classes (e.g., `core:Class` becomes `core:Class` + `core:RecordClass`), the `resolve_const_ref` match table needs to track. Mitigation: keep the match table small and centralised; treat it as part of D47's surface API.

### 9.3 Decoder needs layer access

Unlike FormulaTerm's decoder (which resolves `OpRef` names through an operator catalog passed in), `decode_type` needs the full layer chain to look up `ConstRef` IRIs. This makes the decoder less convenient to call in isolation. Mitigation: D46 Phase H always has the layer chain available (it's running inside the kernel's environment builder); other future consumers will too.

---

## 10. References

### 10.1 Internal

- [D32 — Chain-mirrored EigenTT inductives](d32-chain-mirrored-mini-tt-inductives.md) — direct precedent for the chain-inductive declaration + value encoding pattern that D47 reuses.
- [D40 — Chain-mirrored Lean expressions](d40-chain-mirrored-lean-expressions.md) — parallel design for Lean's term language; informs the propositions-only / no-DAG-sharing choices.
- [D46 — Prop universe and proof irrelevance](d46-prop-universe-and-proof-irrelevance.md) — primary consumer; §10's axiom-as-Resource framework needs `core:EigenTTType` for `axiom_statement` values.
- [D1 — Eigon serialization format](d1-eigon-serialization-format.md) — the persistence shape D47 inherits via `data_type: core:inductive`.

### 10.2 Source

- `kernel/src/nbe/term.rs::Exp` — the source language D47 mirrors a subset of.
- `kernel/src/program/` — where the new `eigentt_type_mirror.rs` module lives.
