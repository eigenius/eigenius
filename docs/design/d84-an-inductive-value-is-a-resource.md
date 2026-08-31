# D84 — An inductive value is a resource

**Status: proposed.** Written `2026-08-30`, during P6.2 of the judgements-and-warrants refactor
([`docs/notes/judgements-warrants-build-plan.md`](../notes/judgements-warrants-build-plan.md)).

**Supersedes** D83 §3.1, §3.4 and §4.2, and the encoding half of D47. **Withdraws** D83 §4.1.
D83 §3.2 and §3.3 stand and turn out to be the general case rather than the special one they were
written as.

---

## 1. The claim

`eigentt:Term` is an ordinary inductive, and an inductive VALUE is a `Resource`:

```json
{
  "urn:eigenius:core:is_a":   ["urn:eigenius:eigentt:Term"],
  "urn:eigenius:core:ctor":   "App",
  "urn:eigenius:core:args":   [ … ]
}
```

Inline, it is `Value::Embedded`. Chain-resident, it acquires an `@id` and is named by
`Value::ResourceRef`. `Value` gains no variants, because the two forms an inductive value can take
are the two forms every resource-valued slot already has.

Everything below follows from that sentence plus one rule the platform already enforces.

## 2. Why — the rule that decides it

`Value` should be the Rust reflection of `core:DataType`, as `Resource` is the reflection of
`core:Class`. The ontology declares the data model's own type system — `string`, `integer`,
`float`, `boolean`, `json`, `resource`, `resource_array`, `value_array`, `inductive` — and a
self-describing platform's in-memory value type is the inhabitants of that system, with the
recursion never leaving it.

D32 §3.7 already specified constructor arguments in exactly that vocabulary:

> - Primitive types: as the existing primitive Eigon-CBOR encoding.
> - Class refs: as a **`ResourceRef`** (the existing canonical class-ref encoding) or an embedded resource map.
> - Inductive refs: recursively as `{ "ctor": ..., "args": [...] }`.
> - `cardinality: list`: as a CBOR array of the encoded element shape.

That is `Value::String` / `Integer` / `Float` / `Boolean`, `Value::ResourceRef` / `Value::Embedded`,
the recursive case, and `Value::Array`. The specification names `ResourceRef` outright. The same
section then says why it did not land that way:

> *"`Value::Json` in the existing kernel handling already round-trips it … with no shape change."*

Reuse of an available carrier, not a design decision. The semantics were specified in `Value` and
the implementation was written in `serde_json::Value`, and the gap between those two sentences is
the whole of the debt this note pays.

**The asymmetry that makes it visible.** `Value::Embedded(Box<Resource>)` keeps the recursion inside
the data model; an inductive value in `serde_json::Value` leaves it. So every facility the data
model has acquired a second implementation for the JSON side:

| data-model side | `serde_json` twin |
|---|---|
| `value_refs` (declaration_order), `collect_refs_from_value` (supporting) | `json_mentions` (term_mentions) |
| `values_equal` (query/functions) | `alpha_canonicalize_proposition_json` (witness) |
| `value_to_cbor` | `json_value_to_cbor` |
| `canonicalise_resource_refs` | *nothing* — a term's interior is never canonicalised |

## 3. Values self-describe, and that is not a choice

**Rule 0: every resource declares at least one `is_a` class**, described in `rules/is_a.rs` as a
semantic of the data model rather than of the wire. Every embedded resource in the shipped core
ontology follows it: `InductiveCtor` and `InductiveArgType` both carry `is_a`.

So if an inductive value is a resource, it carries `is_a` naming its InductiveType. There is no
"resource without a type" to fall back on. Four things follow, and none of them is a preference.

### 3.1 `CtorApp` disappears

D47's twentieth `eigentt:Term` constructor exists to name an inductive when the SLOT cannot — a
`justification:Certificate` value standing in a slot declared `eigentt:Term`. A value that carries
its own `is_a` names itself, so the escape hatch has nothing left to do. It is deleted, not widened.

### 3.2 D83 §3.1 inverts

§3.1 says *"the inductive type is not carried; it is recovered from the slot."* The opposite holds:
the value carries its type, and the property's `class_types` becomes a CONSTRAINT to check it
against — which is exactly what `class_types` already means for a `core:resource` slot. One rule for
both, rather than two rules that disagree about where a type comes from.

### 3.3 D83 §4.2's threading is unnecessary

Threading the declared inductive from the slot into the encoder existed so a value could be written
at a type it did not state. A self-describing value states it. The property → declared-inductive and
ctor → arg-types seeds added to the ESL compiler, and the printer's slot map, all go.

### 3.4 Rule 16 folds into Rule 24

Validating an inductive value IS validating a resource against its `is_a`. Rule 24 already does
this (D83 §3.2, committed). `walk_inductive_value`'s parallel traversal, its hand-threaded
reference-cycle set, and its `Value::Json` unwrapping are all subsumed.

## 4. What this supersedes from D83, and why it was reached anyway

Two changes committed earlier today solve problems that do not exist once values self-describe:

| commit | what it did | why it is superseded |
|---|---|---|
| `ad51d25` | `CtorApp` gains a third argument; the `App` spine retired | `CtorApp` is deleted entirely (§3.1) |
| `438de92` | the declared inductive threaded into compiler and printer | a self-describing value needs no threading (§3.3) |

The spine retirement stands on its own — `App` meaning application and nothing else is right
independent of this note, and it is what made `D.c(a) b` expressible. What is superseded is the
constructor that carried the arguments, not the decision to stop currying them.

`66213fc` (§3.2 value-as-resource, §3.3 reference form, Rule 24) is not superseded. It was written
as a special case for chain-resident values and is the general case for all of them.

## 5. `Value::Json` splitting — withdrawn

D83 §4.1 proposed `Json` / `Inductive(serde_json::Value)` / `InductiveRef(Iri)`. It was implemented
and reverted the same day. Two reasons, both measured.

**Its cited live consequence does not exist.** The claim was that `json_mentions_of_value` matches
`Value::Json` unconditionally, so opaque JSON holding a `urn:`-shaped string becomes a spurious
`core:mentions` edge. It has exactly one caller, `layer::index`, and that call already sits inside
the `wk::INDUCTIVE` arm of a match on the property's `data_type`.

**The variants were `Embedded` and `ResourceRef` rediscovered, badly.** `Inductive` and
`InductiveRef` are discriminated by the property's `data_type` — a schema fact — so the parser
cannot produce them (it has no `Layer`, because `bootstrap` parses `core-ontology.json` with
`parent: None` and that parse CREATES `core:data_type`), and CBOR cannot preserve them (`Text` →
`String`, tagged map → `Json`). They exist only between `LayerBuilder::build` and the next
serialisation. `Embedded` and `ResourceRef` are the same two ideas anchored on a SHAPE, which is why
they survive both boundaries.

`ResourceRef` is the pre-existing instance of the same mistake and this note does not fix it: it is
`String` plus a schema lookup, needs `as_iri_str` / `as_iri` / `as_iri_array` across 142 call sites
to reconcile, carries 28 "accept both" comments, has a special case in `values_equal` that derived
`PartialEq` does not share, and `as_iri_str` exists because its absence "produced an entirely-empty
topology graph for canonicalised chains". Recorded here; out of scope.

## 6. Open, and deliberately not decided here

1. **`is_a` on embedded resources is conventional, not enforced.** `Validator::validate` iterates
   `layer.iter_resources()` — top-level only — so Rule 0 never reaches an embedded resource. The
   shipped ontology follows the convention uniformly, and under this note Rule 24 enforces it for
   inductive values specifically, since it resolves `is_a` to find the inductive. Whether Rule 0
   should descend generally is a separate question.
2. **`core:args` element typing.** It is declared `value_array` with `element_type: core:json`, and
   is the only one of the 18 `value_array` properties in the tree whose elements are not uniform —
   the other 17 are all `string`, `integer` or `float`. `core:element_type` is `then_requires`d
   whenever `data_type` is `value_array`, and admits only the five primitives. A positional argument
   list whose element types come from `core:ctor` is not a homogeneous array, and forcing it into
   one produced the `core:json` workaround. Either `element_type` becomes optional when the types
   are determined elsewhere, or `core:args` is not a `value_array`.
3. **The wire abbreviation.** Whether `ctor` / `args` may appear as short keys on the wire, expanded
   to their IRIs on read, the way a `ResourceRef` already serialises as a bare IRI string rather
   than `{"@id": …}`. This is a codec table entry, reversible, and NOT a design fork — an earlier
   draft of this note treated it as one on a size argument that does not survive contact with the
   storage layer, which sets `DBCompressionType::Lz4`, and against which repeated
   `urn:eigenius:core:ctor` keys compress to nearly nothing.

## 7. Getting there

Mostly deletion, which is the sign the shape is right.

**Phase 1 is one cut-over, and almost everything is in it.** An earlier draft of this section split
the deletions into a second phase. They are not separable: each of them reads `Value::Json`, and
`CtorApp` is worse than that — leaving it one phase longer would force `encode_type` to CHOOSE
between emitting a self-describing value and emitting a `CtorApp`, which is the two-shapes problem
D83 §2.1 exists to prevent.

| phase | work |
|---|---|
| 1 | `encode_type` / `decode_type` become `Exp` ↔ `Value` bridges — the only substantive new code, and smaller than what it replaces (`arg_string` becomes `as_str`; the ctor arms read `&[Value]`). Inductive values become `Embedded` / `ResourceRef`. **In the same cut-over**, because each reads the shape being retired: delete `CtorApp` from `eigentt:Term` and the codec; delete §4.2's threading (compiler seeds, printer slot map); fold Rule 16's walker into Rule 24; replace `json_mentions` in the triple index's `wk::INDUCTIVE` arm with the `Embedded` descent `value_refs` already does; and take α-canonicalisation off JSON — `hash_proposition_value` and `WitnessKey::from_encoded` have three callers between them, two of which are tests. |
| 2 | `Vector` leaves `Value` — it inhabits no `core:DataType`, which is why serialising one panics. The query engine takes a domain that extends the data model. 7 sites in `resource.rs`, 17 across the tree; genuinely independent of phase 1. |

The compiler enumerates the cut-over: the §4.1 experiment broke exactly six exhaustive matches plus
about eight substantive readers.

**`core:cardinality` loses its only user.** It was built for `CtorApp`'s argument list and nothing
else in the tree declares a list-valued constructor argument, so deleting `CtorApp` leaves it a
declared, tested capability with no production consumer. It is not wrong — D32 §3.7 specified it
from the start and it was documentation-only until now — but it should be kept or dropped
deliberately rather than by inertia. Kept, for now, on the grounds that a sequence-valued
constructor argument is a real thing D32 promised and the ESL surface `[T]` plus six tests keep it
honest.

One reseed, at the end of phase 1, folded into the one already owed for P4, P5 and D83 §3.4.
