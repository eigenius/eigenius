# D83 — The wire format for inductive values

**Status: proposed.** Written `2026-08-30`, during P6.2 of the judgements-and-warrants refactor
([`docs/notes/judgements-warrants-build-plan.md`](../notes/judgements-warrants-build-plan.md)).

**Superseded in part by [D84](d84-an-inductive-value-is-a-resource.md)**, which makes an inductive
value a `Resource`. D84 supersedes §3.1, §3.4 and §4.2, and withdraws §4.1; §3.2 and §3.3 stand and
become the general case rather than the special one they are written as here. The sections below are
left as written, each carrying a pointer, because the reasoning that produced them is what led to
D84 and deleting it would hide the path.

**Supersedes** D32 §3.7 and the encoding half of D47 §4. Both stay in force for everything else
they say; this note replaces only the question *what does an inductive value look like on the
chain*.

**Why it exists.** P6.2 needs a value to be able to REFERENCE another inductive value rather than
inline it. Adding that to the format as it stands would mean adding it twice, because the format as
it stands is two formats that no document distinguishes.

---

## 1. What the format is today

### 1.1 One format is documented

D32 §3.7 specifies a single shape for a property whose `data_type` is `core:inductive`:

```json
{ "ctor": "<ctor_name>", "args": [<arg₁>, <arg₂>, …] }
```

D47 §1.2 says it *"reuses all of"* D32's infrastructure, "value encoding" included. So the
documented answer is: one shape, the tagged dict.

### 1.2 Two formats exist

`encode_type` does not emit that shape for a constructor application. Its own comment
(`kernel/src/program/eigentt_type_mirror.rs`) states the alternative:

```
D.c(a₁, …, aₙ)   encodes as   App(App(…App(CtorApp(D.iri, c), a₁)…, aₙ₋₁), aₙ)
```

Both forms are on the chain now. The 22 hand-authored ontology values — `formulas:` operator
signatures, `core:Asserts.result_sort`, `reflection:canonical_proposition.expected_type` — use the
tagged dict. Everything written through ESL `type_expr(…)` uses the `App`-spine.

### 1.3 Validation is split to match, and neither half knows about the other

`walk_inductive_value` (Rule 16) reads `obj["ctor"]` and looks the name up on the declared
inductive's ctor list. It contains **no** occurrence of `CtorApp`. Handed a spine it reports

```
ctor `App` not declared on InductiveType `eigentt:Judgement`
```

which is exactly the defect P5 found on every judgement on every chain, and "fixed" by giving
Rule 21 ownership of `eigentt:Judgement` slots. That patched the symptom. The cause is that a
constructor application has two shapes and each rule understands one.

### 1.4 `App` is overloaded

For `eigentt:Term`, `App` is a declared constructor of the inductive — a term language has
application — *and* the encoding's currying device. Same wire node, two meanings. Decode
disambiguates structurally: fold the argument into the head's arg list if the spine bottoms out at
`CtorApp`, otherwise build a plain `Exp::App`.

It works, it is undocumented, and it costs something real: a saturated constructor application
applied to a further argument can never round-trip, because the fold always wins. `D.c(a) b` is
inexpressible.

---

## 2. What we need

1. **One shape.** Two shapes with no stated rule for which applies is what produced §1.3.
2. **Self-describing enough to hand-author.** Eigon-JSON is an authoring surface, not only a
   serialisation target.
3. **A reference form.** A value must be able to name another value instead of containing it —
   P6.2's requirement, and the asymmetry D32 §3.7 already leaves: it gives *class*-typed arguments
   both "a `ResourceRef` … or an embedded resource map", and gives inductive-typed arguments only
   the inline form.
4. **`App` means one thing.**
5. **No new machinery where the kernel already has some.** `class_types` already resolves to an
   `InductiveType` (`class_types_inductive_target`), and Rule 22 already admits an `is_a` target
   that is an `InductiveType`. Both are load-bearing below.

Partial application is **not** among the needs. It is the only thing the spine buys, and it buys it
for a case that does not arise: a stored value is a complete value, and the kernel's
`Exp::InductiveCtor(iri, name, args)` represents a partially applied constructor by holding fewer
args, with no currying required.

---

## 3. The format

**One shape: the tagged dict.** The `App`-spine is retired.

### 3.0 One shape, two readers

The shape is uniform. What reads it is not, and conflating the two is what made §1 hard to see.

| reader | scope | produces |
|---|---|---|
| `walk_inductive_value` (Rule 16) | **any** `core:inductive` value, against its declared inductive | validation errors |
| `decode_type` | the `eigentt:Term` family only | a kernel `Exp` |

`decode_type` is not a general inductive decoder, and reading it as one is a mistake this note
originally made. `formulas:FormulaTerm` values are never decoded to `Exp` at all — Rule 16 validates
them structurally and the numerical institutions consume the JSON directly. That is why FormulaTerm
has always used the tagged dict and nobody noticed the split: the split lives entirely inside the
eigentt family, which is the only family with a second reader.

### 3.1 Inline

> **Superseded by D84 §3.2.** The inductive type IS carried: an inductive value is a resource
> and declares its `is_a`. The slot's `class_types` becomes a constraint to check it against.


```json
{ "ctor": "<ctor_name>", "args": [<arg₁>, …, <argₙ>] }
```

The inductive type is not carried; it is recovered from the slot — the property's `class_types` at
the top level, the enclosing constructor's `arg_types[i]` below it. This is D32 §3.7 unchanged.

Each `argᵢ` is encoded by its declared `arg_type`:

| declared arg type | encoding |
|---|---|
| primitive | the existing primitive Eigon-CBOR/JSON encoding |
| class | a resource IRI string, or an embedded resource object |
| **inductive** | **an inline value (§3.1), or a value IRI string (§3.3)** |
| `cardinality: list` | a JSON array of the element encoding |

The inductive row is the only change to D32 §3.7, and it makes the inductive row say what the class
row already said.

### 3.2 As a resource

An inductive value becomes chain-resident by acquiring an identity and declaring its type:

```json
{
  "@id": "urn:eigenius:pub:wrn:cellcycle_recompute_proof",
  "urn:eigenius:core:is_a":  ["urn:eigenius:justification:Certificate"],
  "urn:eigenius:core:ctor":  "app",
  "urn:eigenius:core:args":  [ … ]
}
```

`is_a` names the **InductiveType**, not a Class — **exactly one, with no Class alongside it**. A
value has one type; the array shape is inherited from `is_a` generally, not a licence to give a
value two.

This needs no new class and no wrapper. It is the same relationship `Embedded` and `ResourceRef`
already have for resources — an embedded resource is an object without an `@id`, a referenced one is
the same object with one — lifted to inductive values.

**Two new properties**, neither of which collides with anything (`core:ctors`, `core:ctor_type` and
`core:ctor_name` exist, and belong to the DECLARATION vocabulary; `core:ctor` and `core:args` are
free):

| property | `data_type` | meaning |
|---|---|---|
| `core:ctor` | `core:string` | the constructor this value applies |
| `core:args` | `core:value_array` | its arguments, each encoded per §3.1's table |

**Why IRI keys and not bare `ctor` / `args`.** D1 §2.1 states that *"`@id` is the only reserved key
in Eigon-JSON"*, and D1 §3.1 that *"all property keys in Eigon-JSON are full IRIs."* Reserving two
more keys would make both sentences false and require amending D1. Declaring two ordinary properties
requires amending nothing, and they then validate like any other property.

Note this costs no consistency with §3.1. A resource's KEYS are IRIs; a VALUE's internal structure
is not a resource, so the bare `ctor` / `args` inside `core:args`' elements are not property keys and
never were — which is why D32 §3.7's tagged dict has always coexisted with D1 §3.1 without
contradiction. The boundary is exactly where §3.1 stops and §3.2 begins.

### 3.3 As a reference

A bare JSON string holding the IRI of a §3.2 resource:

```json
{ "ctor": "app", "args": [ "urn:eigenius:pub:wrn:cellcycle_recompute_proof", … ] }
```

**The two forms are unambiguous by JSON type.** An inline value is always an object; a reference is
always a string. No sentinel, no discriminator field.

**Identity is by expansion.** A referencing value and its fully inlined twin are the same value:
they must be equal under `values_equal`, and must produce the same
`hash_proposition_exp`. Anything else would make deduplication change witness keys and break live
citations — the reference is a sharing device, not a distinct term.

That ordering has one consequence worth stating: **cycle detection must run before hashing**, since
expansion of a cyclic reference does not terminate. P6.1's well-foundedness pass
(`kernel/src/justification/wellfounded.rs`) is the natural place, and its condition is already
evaluated over decoded terms.

---

### 3.4 A value from another inductive, embedded in an eigentt term

> **Superseded by D84 §3.1.** `CtorApp` is deleted, not widened. It exists to name an inductive
> when the slot cannot; a value that carries its own `is_a` names itself. The retirement of the
> `App` spine stands — `App` means application and nothing else.


`eigentt:Term` is a universal term language, and a slot declared `eigentt:Term` routinely holds a
value belonging to some other inductive. `eigentt:Judgement.holds` declares
`term : eigentt:Term` and `type : eigentt:Term`, while the values stored there are
`justification:Certificate` applications.

So the inductive cannot always come from the slot, and §3.1's rule has one exception. The exception
is already declared: `eigentt:Term` carries `CtorApp(decl_iri, ctor_name)` as its twentieth
constructor — the escape hatch into other inductives. That is not an encoding hack bolted onto the
codec; it is part of the type.

**The escape hatch stays. The currying goes.**

```
CtorApp(D, c) + n × App        ⟶        CtorApp(D, c, [a₁, …, aₙ])
```

| ctor | args |
|---|---|
| `CtorApp` | `decl_iri : core:string`, `ctor_name : core:string`, `args : eigentt:Term` with `cardinality: list` |

Same information — `D` is carried, as it must be — with the fold removed, so `App` means
application and nothing else. `cardinality: list` is already in D32 §3.7's arg table.

**The ctor keeps its name and gains a third argument.** An earlier draft minted `CtorVal` instead,
on the reasoning that old values carry two args and new ones three, so reusing the name would leave
arity as the only discriminator — the silent ambiguity §1 is about. That reasoning is void: the two
never coexist (§4.4). `CtorApp` is what the thing is, and there is no second meaning to distinguish
it from.

**Threading and the escape hatch are not alternatives.** Threading (§4.2) removes the wrapper where
the slot names the inductive; the escape hatch covers the case where it cannot, because the declared
type is `eigentt:Term` and the value's type is something else. Each does a job the other cannot.

## 4. What changes

### 4.1 `Value` splits three ways

> **WITHDRAWN — see D84 §5.** Implemented and reverted the same day. The cited live consequence
> does not exist: `json_mentions_of_value` has one caller and it is already gated on `data_type`.
> And the variants are `Embedded` and `ResourceRef` rediscovered on a schema discriminant instead
> of a shape one, so neither the parser nor CBOR can carry them.


```rust
Json(serde_json::Value)   // genuinely opaque — `core:json`
Inductive(InductiveValue) // a tagged dict — `core:inductive`
InductiveRef(Iri)         // a reference to a §3.2 resource
```

Today one variant carries both jobs, and its doc comment — *"Opaque JSON value, not validated by the
ontology"* — describes only the first while Rule 16 walks it, Rule 21 type-checks it and the witness
hash canonicalises it.

**The ontology already draws this line**; the Rust enum is what lags. `core:json` and
`core:inductive` are distinct data types, and `core-ontology.json`'s own description of
`core:ctor_type` records the confusion being fixed once already:

> *"A D47-encoded `eigentt:Term`, so `core:inductive` rather than `core:json`: until D79 §2.1 this
> was declared `core:json`, which is 'not validated by the ontology', so no rule checked it and the
> mutual-inductive walker in `layer::declaration_order` had to descend into it by hand."*

**Where the split happens: `canonicalise_resource_refs`, not the parser.** `eigon_json::parse_value`
takes a property NAME and a JSON value and nothing else — it has no `Layer`, because `bootstrap`
parses `core-ontology.json` with `parent: None` and that is the parse which CREATES `core:data_type`
and `core:json`. Every distinction above is a schema distinction — `core:json` and `core:inductive`
values are both objects with bare keys, a §3.3 reference and a `core:resource` reference are both
bare strings — so none of them can be decided at parse time.

They are decided one step later, by a pass that already does exactly this for the resource case.
`LayerBuilder::build` calls `canonicalise_resource_refs`, which upgrades every parsed `Value::String`
naming a `data_type: resource` property to a `Value::ResourceRef` so that *"downstream readers
(validator, triple index, query evaluator) can then assume one shape per data_type."* It resolves
each property's `data_type` against the layer being built and its parents, and consults the layer's
own resources first — which is why bootstrap is unaffected, the core ontology's property
declarations being visible to their own canonicalisation pass. `core:inductive` is one more arm.

**The live consequence this section used to cite does not exist.** The claim was that
`json_mentions_of_value` matches `Value::Json` unconditionally, so opaque JSON holding a
`urn:`-shaped string becomes a spurious `core:mentions` edge. It has exactly one caller,
`layer::index`, and that call already sits inside the `wk::INDUCTIVE` arm of a match on the
property's `data_type`. The argument for the split is the doc comment above, not a live defect.

### 4.2 The expected inductive is threaded through decode

> **Superseded by D84 §3.3.** Threading exists so a value can be written at a type it does not
> state. A self-describing value states it.


`decode_type(value, layer)` gains the declared inductive: at the top of a slot it is the property's
single `class_types` entry; below, it is the enclosing constructor's `arg_types[i]`. That is what
D32 §3.7 always said — *"resolves the property's declared inductive type (via `class_types`)"* — and
what Rule 16 already does. `decode_type` simply never received it.

With it threaded, a slot whose `class_types` is a user inductive carries that inductive's ctors
directly: `justification:judgement` becomes `{"ctor": "holds", "args": […]}` with no wrapper at all.

**Cost, measured:** 65 call sites across 16 files. Most already hold the type they must pass; the
one that does not is `hash_stored_proposition`, which reads a `canonical_proposition` off a resource
and would need the property's `class_types` plumbed alongside it.

**What threading does NOT reach** is §3.4's case, and that is why the escape hatch exists.

Decode also stops spine-folding: `{"ctor": "App", …}` becomes `Exp::App` and nothing else. It does
**not** accept the old spine alongside — see §4.4, there is nothing to be compatible with.

### 4.3 Rule 16 becomes the single structural validator

With one shape, `walk_inductive_value` reads every inductive value including `eigentt:Term` and
`eigentt:Judgement`. Rule 21 keeps its own job — decoding and NbE type-checking eigentt-typed slots
— but the two no longer disagree about the shape, and P5's exemption of `eigentt:Judgement` from
Rule 16 can be withdrawn.

**Rule 16 does NOT get a `CtorApp` arm.** This section called for one — switch the expected
inductive to `decl_iri`, resolve `ctor_name` on it, check the arg list against THAT ctor's
`arg_types` — on the reasoning that the escape hatch would otherwise go unvalidated. It was
written and then withdrawn, for two reasons found by writing it.

It is **wrong for a typed constructor.** `core:ctor_type` carries a full Π-telescope and, when
present, `arg_types` is absent — the form every ctor of an indexed inductive uses. Arity read off
`arg_types` is therefore `0` for `justification:Certificate.declared`, and the arm rejected 107
well-formed values across the WRN chains on its first run.

It is **redundant.** Every `CtorApp` on the chain sits inside a slot ranged at `eigentt:Term` or
`eigentt:Judgement`, and Rule 21 owns both: it decodes and NbE-checks them, which validates
constructor names and arities against the environment. D76 Phase B removed this same check from
the decoder for this same reason — *"the type checker does anyway (`check_ctor_unknown_name`), and
does with the environment in hand rather than at decode time"* — and Rule 16 already skips
`eigentt:Term` so the two do not produce duplicate diagnostics.

What Rule 16 does check is the escape hatch's own SHAPE, and §3.4 is what made that possible:
`CtorApp`'s third argument is declared `[eigentt:Term]`, so a non-array argument list or a
non-string `decl_iri` is caught by the generic walk. Before §3.4 the arguments were not part of the
node at all, and the surrounding `App` spine was reported as a malformed value.

### 4.4 There is no migration

The reseed **drops the database**. Nothing carrying the old form survives it, so no compatibility
window is needed and decode accepts exactly one shape.

Verified rather than assumed: **no checked-in source carries the spine.** `grep` for a `CtorApp`
wire ctor across every `.json` and `.esl` in the tree returns nothing. The one file with an `App`
wire ctor, `notebooks/examples/lean-verification-demo.eigon.json`, is a `lean:LeanExpr` value —
ctors `Const` / `App` / `Nil`, a different inductive that already uses the tagged dict and is
untouched by this note.

The spine therefore exists in exactly two places: the persisted database, which the reseed deletes,
and memory, which does not outlive the process. Everything else is regenerated — ESL recompiles on
load, and `LexicalEntry`'s two `core:inductive` slots — `lexicon:cat` and `lexicon:sem_type` — are
rebuilt by the importers.

A reseed is already owed for P4 and P5. Riding it costs nothing; deferring past it would mean either
writing the compatibility path this section says is unnecessary, or migrating stored values. The
window is open and will close.

## 5. What is retired

| retired | replaced by |
|---|---|
| the `App`-spine encoding of constructor application | §3.1's tagged dict at a slot boundary; §3.4's 3-arg `CtorApp` inside an eigentt term |
| `CtorApp`'s 2-arg form + currying | `CtorApp(D, c, args)` — same ctor, third argument, no fold |
| spine folding in `decode_type` | nothing — `App` is a term ctor only |
| `decode_type`'s context-free signature | the threaded declared inductive (§4.2) |
| `Value::Json` as the carrier for inductive values | `Value::Inductive` (§4.1) |
| Rule 21's exemption of `eigentt:Judgement` from Rule 16 | one shape, both rules read it |
| `decode_type`'s two-argument `CtorApp` + the `App` fold | §3.4's three-argument form |
| the inline-only restriction on inductive arguments | §3.3, the reference form |

---

## 6. Settled while drafting

1. **Reserved keys — resolved, and the reason is D1.** An earlier draft made `ctor` and `args`
   reserved keys of the object model. D1 §2.1 says `@id` is the only reserved key and §3.1 says
   every property key is a full IRI; reserving two more would falsify both and require amending D1.
   §3.2 declares two ordinary properties instead, so D1 stands untouched. The collision risk was
   measured and nil either way — every property key on every resource today is `urn:`-prefixed, and
   all 173 `ctor` / `args` occurrences sit inside value objects — but "nil risk" was never the
   objection; a document stating something that would become false was.
2. **One inductive per value resource.** `is_a` names exactly one InductiveType and no Class.
3. **Rule 22's diagnostic — fixed.** It already admitted an `is_a` target that is an InductiveType,
   while its rejection message still read *"resolves to a resource that is not a core:Class"*. The
   check permitted inductives; only the message never caught up. Left alone it would have misled
   the first person to debug a §3.2 value resource into believing the shape was illegal.
