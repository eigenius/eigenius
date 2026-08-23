# P2 · N4 — Should the EigenTT term representation live in core?

Written `2026-08-23` from `c271213`. Arose while implementing
[#188](https://github.com/eigenius/eigenius/issues/188) but is **not part of it** — universe
polymorphism needs none of this.

**Answer: yes, and the bootstrap-cycle objection does not hold.** The decoder is hard-coded, not
schema-driven, so nothing needs a `TypeExpr` value to decode a `TypeExpr` value. What remains is one
narrower question about the *validator*, in §4.

## 1. The symptom, twice

`core` owns the inductive metamodel — `InductiveType`, `InductiveCtor`, `InductiveArgType`,
`InductiveParam`, and the properties `param_kind`, `type_name`, `result_sort`, `indices`,
`type_params`. It does **not** own the term language those slots need to describe a type. So every
type-valued slot in core degrades to a string, and each has decoded lossily:

| slot | encoding | consequence |
|---|---|---|
| `core:result_sort` | was `"Prop"` / `"Set"` / `"Type:N"` | could not express a level variable — **fixed** by #188 slice 5b, now a `core:Level` value |
| `core:param_kind` | IRI \| bare param name \| sort keyword | a class-typed parameter falls through `decode_param_kind_str` to `Sort(1)` and is **silently typed `Set`**, which accepts anything |
| `core:type_name` (on `InductiveArgType`) | IRI \| bare param name | handled correctly — `decode_arg_type` has the `EigonClass` arm `param_kind` lacks |

Two of the three were defective, in the same way, for the same reason. `decode_arg_type`'s doc
records the asymmetry without naming its cause: *"Any other class IRI: emitted as
`Exp::EigonClass(iri)`"* — the arm the parameter path never got. #199's comment names it too, from
the other side.

**The `param_kind` `EigonClass` arm is a live bug and should be fixed regardless of this note.** It
needs no layering change and no ontology edit.

## 2. What the layer actually contains

`eigentt-type-fragment` is not one thing. Its dependency edges run one way:

| resource | kind | depends on |
|---|---|---|
| `eigentt:TypeExpr` | `core:InductiveType` | — |
| `eigentt:Axiom` | `core:Class` | — |
| `eigentt:axiom_statement` | property | `class_types [eigentt:TypeExpr]` |
| `eigentt:axiom_justification` | property | — |
| `eigentt:Definition` | `core:Class` | — |
| `eigentt:definition_type` | property | `class_types [eigentt:TypeExpr]` |
| `eigentt:definition_body` | property | `class_types [eigentt:TypeExpr]` |
| `eigentt:definition_opaque` | property | — |

So the layer bundles **the term language** with **resources that carry terms**. `Axiom` and
`Definition` are consumers of `TypeExpr`; nothing depends the other way. Moving `TypeExpr` down does
not break a cohesive layer — it separates two strata that were bundled.

That also disposes of the one argument this note started with against the move. There is no cohesion
to protect.

## 3. Why `core` is already the type-theory layer

The objection "core should stay a minimal, type-theory-free metamodel" describes a `core` that does
not exist:

- it owns the entire inductive **declaration** metamodel (§1);
- it **declares inductives** — `core:Asserts`, `core:Option`;
- it now carries a **universe level algebra**, `core:Level`, because `result_sort` needed one and a
  lower layer cannot reference a higher one (#188 slice 5b);
- D47 §211 already reasons about what must be in core *and in what order* — *"the core ontology needs
  the following declarations prior to committing the axiom Resources (in the same core-ontology
  layer, ordered before the axioms)"*.

No recorded rationale places `TypeExpr` above `core`. On the evidence the boundary is where it is by
accident of when D47 was written, not by design.

## 4. The bootstrap-cycle question, answered

The objection worth taking seriously: `eigentt:TypeExpr` is itself declared **as a
`core:InductiveType`**, its constructor arguments described by `core:type_name`. Retype `type_name`
to carry `TypeExpr` values — which is the reason to move it — and TypeExpr's own declaration
describes itself in its own language. Does that cycle?

**For the decoder, no.** `decode_type_json` (`program/eigentt_type_mirror.rs:496`) dispatches on
hard-coded ctor-name strings — `match ctor { "Sort" => …, "Var" => …, … }`. It never reads
`TypeExpr`'s chain declaration. Decoding a `TypeExpr` value therefore does not require having
decoded `TypeExpr`'s schema, and the self-reference is inert at decode time.

The recursion the design *does* worry about is already solved, and by a mechanism that generalises:
`decode_arg_type` resolves a cross-inductive reference to a **stub** rather than the full decl,
*"which would risk infinite recursion for mutually-referential inductives"*. A `TypeExpr`-valued
`type_name` would resolve through the same stub path.

**For the validator, open.** Rule 16 (`validation/rules/inductive.rs:245`) *is* schema-driven — it
reads the target `InductiveType`'s `ctors` array and checks the value's ctor name and arity against
it. Validating `TypeExpr`'s own `type_name` values would mean reading `TypeExpr`'s `ctors` to check
values that are themselves inside `TypeExpr`'s `ctors`. That is not obviously a cycle — the read is
of a resource, not of a decoded term — but it is the one place to look before committing.

**The narrow question to settle first:** does Rule 16 terminate when the value under validation is
part of the very declaration supplying the schema? Answerable by reading `check_inductive_value`'s
recursion, or by an experiment: give one `InductiveArgType` a `TypeExpr` value and validate the core
layer.

## 5. If it goes ahead

- **Move the declaration, keep the IRI.** `urn:eigenius:eigentt:TypeExpr` declared in
  `core-ontology.json` is legal — the namespace prefix is a naming convention, not a layer
  assertion. That leaves **all 22 chain references and 59 Rust references untouched**. Renaming to
  `core:TypeExpr` would change 81 references and break a widely-referenced IRI for cosmetics.
- `Axiom` / `Definition` and their four properties stay where they are. They are the higher stratum.
- Retype `param_kind` and `type_name` to `data_type core:resource` / `class_types [TypeExpr]`, the
  pattern `reflection:canonical_proposition` and now `core:result_sort` both use.
- **`TypeExpr` needs a `SizeSort` ctor** — `param_kind` accepts `Size`, and `TypeExpr`'s 19
  constructors have no representation for it.
- Both string unions disappear, and with them the `param_kind` silent default.

## 6. Scope

**Not part of #188.** Universe polymorphism needed `core:Level`, which is done. It does not need
`param_kind` or `type_name` retyped; only the sort case of `param_kind` would benefit, and that is
what made this look like #188 work when it is not.

Do **not** fold this into the #188 reseed as a matter of convenience. It is a change to what `core`
is, it wants its own gate, and #188 is already carrying two core-layer moves. The cost of a separate
reseed is real but bounded; the cost of discovering a validator cycle inside a reseed that is also
carrying universe polymorphism is not.
