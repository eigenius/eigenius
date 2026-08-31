# D83 — The wire format for inductive values

**Status: proposed.** Written `2026-08-30`, during P6.2 of the judgements-and-warrants refactor
([`docs/notes/judgements-warrants-build-plan.md`](../notes/judgements-warrants-build-plan.md)).

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

### 3.1 Inline

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
  "@id":  "urn:eigenius:pub:wrn:cellcycle_recompute_proof",
  "core:is_a": ["urn:eigenius:justification:Certificate"],
  "ctor": "app",
  "args": [ … ]
}
```

`is_a` names the **InductiveType**, not a Class. The resource *is* the value: `ctor` and `args` are
reserved keys of the Eigon-JSON object model when `is_a` names an inductive, exactly as `@id` and
`core:is_a` are reserved everywhere.

This needs no new class and no wrapper property. It is the same relationship `Embedded` and
`ResourceRef` already have for resources — an embedded resource is an object without an `@id`; a
referenced one is the same object with one — lifted to inductive values.

**Rejected alternative:** a wrapper, `{"@id": …, "is_a": [D], "core:inductive_value": {…}}`. It
avoids reserving `ctor`/`args`, but it states the type twice — once in `is_a` and once in the
wrapper property's `class_types` — with nothing keeping the two honest.

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

## 4. What changes

### 4.1 `Value` splits three ways

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

There is a live consequence too. `json_mentions_of_value` matches `Value::Json` **unconditionally**,
not gated on the property being inductive, so any genuinely opaque JSON holding a `urn:`-shaped
string becomes a spurious `core:mentions` edge. Latent only because nothing stores such data; the
type does not prevent it.

### 4.2 The codec stops folding

`encode_type` emits `{"ctor": name, "args": [...]}` for `Exp::InductiveCtor`. `decode_type` stops
spine-folding: `{"ctor":"App", …}` becomes `Exp::App` and nothing else. `CtorApp` is retired as a
wire ctor.

`App` then means one thing, and `D.c(a) b` becomes expressible.

### 4.3 Rule 16 becomes the single structural validator

With one shape, `walk_inductive_value` reads every inductive value including `eigentt:Term` and
`eigentt:Judgement`. Rule 21 keeps its own job — decoding and NbE type-checking eigentt-typed slots
— but the two no longer disagree about the shape, and P5's exemption of `eigentt:Judgement` from
Rule 16 can be withdrawn.

### 4.4 Migration is a codec change, not a data migration

Every `App`-spine value on the chain is regenerated from source: ESL is recompiled on load, and the
lexicon's millions of `core:inductive` values are rebuilt by the importers. **A reseed is already
owed** for P4 and P5, so the format change rides it at no additional cost. That is the argument for
doing it now rather than after: the window in which it is free is open and will close.

---

## 5. What is retired

| retired | replaced by |
|---|---|
| the `App`-spine encoding of constructor application | §3.1, the tagged dict |
| the `CtorApp` wire ctor | the `ctor` field |
| spine folding in `decode_type` | nothing — `App` is a term ctor only |
| `Value::Json` as the carrier for inductive values | `Value::Inductive` (§4.1) |
| Rule 21's exemption of `eigentt:Judgement` from Rule 16 | one shape, both rules read it |
| the inline-only restriction on inductive arguments | §3.3, the reference form |

---

## 6. Open

1. **Reserving `ctor` and `args`.** §3.2 makes them reserved keys when `is_a` names an inductive.
   No current resource uses either as a property IRI (they are unprefixed, and every property key on
   the chain is a `urn:`), so the collision risk is nil today — but it is a change to the object
   model and should be stated in D1 rather than only here.
2. **Whether `is_a` on a value resource may name more than one inductive.** A value has one type;
   the array shape is inherited from `is_a` generally. Proposed: exactly one InductiveType, and no
   Class alongside it.
3. **Rule 22's diagnostic.** It already admits an `is_a` target that is an `InductiveType` but its
   error text still says *"resolves to a resource that is not a core:Class"*.
