# D85 — The shape of inductive types and their values

**Status: proposed.** Written `2026-08-31`.

**Replaces [D83](d83-inductive-value-wire-format.md) and
[D84](d84-an-inductive-value-is-a-resource.md) entirely.** Both were written across one day while
the facts were still being discovered, and each carries claims the other disproves. They are kept
for their history; nothing should be read out of them.

**Supersedes** D32 §3.7 and the encoding half of D47 §4.

**Every claim about the code below carries a citation, anchored at `5187cfc`.** Where an earlier
note asserted something about the implementation it had not checked, that is called out — seven such
claims are what made D83 and D84 unusable, and the discipline is the point of this one.

---

## 1. What is decided

An inductive **value** is a `Resource`. It states its own type, carries its constructor and its
arguments, and is subject to the rules every resource is subject to:

```json
{
  "urn:eigenius:core:is_a":  ["urn:eigenius:eigentt:Term"],
  "urn:eigenius:core:ctor":  "App",
  "urn:eigenius:core:args":  [ … ]
}
```

Inline, it is `Value::Embedded`. Named, it is **the resource's IRI** — and the design deliberately
does not say which variant carries that IRI, because nothing may depend on it: the parser and CBOR
both produce `Value::String`, `canonicalise_resource_refs` may have upgraded it to
`Value::ResourceRef`, and `Value::as_iri` accepts either. Reading a reference goes through that
accessor, never through a match.

`Value` gains **no** variants. That is the point, and it is what makes the two forms above the same
two forms every resource-valued slot already has.

Five rules follow. They are the type discipline; the representation above is a consequence of them,
and every place the code disagreed with them is listed in §4.

---

## 2. The rules

### R1 — A value states its own type

`is_a` names the `core:InductiveType` the value inhabits. The holding property's `class_types` is a
**constraint to check it against**, not the source of truth.

*Why it is forced rather than chosen.* Rule 0 requires that every resource declare at least one
`is_a` class ([`validation/rules/is_a.rs:36`](../../kernel/src/validation/rules/is_a.rs#L36)), and
its doc calls this "a validator rule, not a wire-format rule" — a semantic of the data model. Every
embedded resource in the shipped core ontology follows it: `InductiveCtor` and `InductiveArgType`
both carry `is_a`. So a value that is a resource carries one; there is no untyped resource to fall
back on.

*This is the reverse of D83 §3.1*, which said "the inductive type is not carried; it is recovered
from the slot".

### R2 — `eigentt:Term` is the universal term language, and `Embed` is its injection

A value of any inductive may stand where an `eigentt:Term` is expected. `eigentt:Term` carries a
constructor for exactly that:

| ctor | arg |
|---|---|
| `Embed` | `value : core:inductive` |

`core:inductive` is a declared `core:DataType`, so "a value of any inductive" is expressible with
existing vocabulary — no new type former.

*Measured, not assumed.* A census over the demo, WRN and benchmark chains found **460** foreign
values inside terms, in exactly **one** slot: `justification:judgement`, carrying
`justification:Term` (337) and `justification:Certificate` (123). Zero anywhere else — not in the
bootstrap chain, not in `canonical_proposition`, not in `lexicon:cat`, not in `axiom_statement`.

*Why an injection and not subsumption.* `eigentt:Term.App` declares both arguments `eigentt:Term`
(core-ontology.json, `eigentt:Term`, 20 constructors). Without a constructor that makes a foreign
value *be* a term, every one of those 460 sites violates the declaration. The ontology has
`subclass_of` for classes and no analogue for inductives, so the injection is what the type system
can express.

*D84 §3.1 said `CtorApp` "disappears".* That is wrong, and the implementation is what disproved it:
`CtorApp` had two jobs, and only one — re-encoding the declaration IRI, constructor name and
arguments that R1 now makes the value carry itself — was redundant. It shrinks from three arguments
to one; it does not go away.

### R3 — Values are closed; terms may be open

A structural rule may check a **closed value** against its constructor's declared argument types. It
may **not** do so for an **open term**, which has variables in argument positions.

*The case that forces it.* `justification.esl` declares
`declared : forall (iri : core:string, P : Prop) => justification:Certificate(justification:Declared(iri), P)`.
Inside that telescope, `Declared`'s argument is the bound variable `iri`, not a string. A structural
rule that checks it against `core:string` rejects the ontology's own declarations — which is exactly
what happened.

So: an `eigentt:Term` value in any argument position is admitted structurally. Whether the term
inhabits the declared type is the type checker's question, answered with the environment in hand
(Rule 21 / NbE), not a structural walk's.

### R4 — Argument types come from the constructor, not the slot

To encode or validate a constructor's arguments, one needs their declared types. Those come from the
constructor — which the value names, per R1 — so they are derivable **from the value alone**.

This is not the slot-directed threading D84 §3.3 removed, and conflating the two is why the same
question surfaced three times: as `core:args` element typing, as literal-versus-reference, and as
primitive arguments encoded as terms. A `core:string` argument is a `Value::String`, a class-typed
argument is a reference, an inductive-typed argument is a value.

### R5 — `Value`'s variants are shapes, not interpretations

`Value` ([`ontology/resource.rs:29`](../../kernel/src/ontology/resource.rs#L29)) is the Eigon data
model's value type, distinct from the kernel's `Exp`/`Val` because most property values are not
terms. It serves three roles at once: the encoding-independent datum, the query language's value
domain (`values_equal`, `values_compare` drive `=`, `<`, `MIN`, `ORDER BY`), and the canonicalisation
input for content hashes.

A variant must be decidable **from the datum**. `String`, `Integer`, `Float`, `Boolean`, `Array`,
`Embedded`, `Json` are; that is why
[`parse_value`](../../kernel/src/ontology/eigon_json.rs#L183) can produce them with no `Layer`,
which bootstrap requires — `core-ontology.json` is parsed with `parent: None`, and that parse creates
`core:data_type`.

So there is **no `Value::Inductive` and no `Value::InductiveRef`.** They would be decided by the
property's `data_type` — a schema fact — so the parser could not produce them and CBOR could not
preserve them (`Text` decodes to `String`, a bare `Map` to `Embedded`,
[`eigon_cbor.rs:379`, `:402`](../../kernel/src/ontology/eigon_cbor.rs#L379)). They are `Embedded`
and `ResourceRef` rediscovered on the wrong discriminant.

**This is why §1 declines to name a variant for the reference form.** `ResourceRef` is the existing
instance of the same mistake, so building on it would import the defect this rule exists to avoid.
A reference is an IRI, and which variant holds it is not part of the design.

*The debt itself*, recorded and not fixed here. `ResourceRef` is
`String` plus a schema lookup, produced only by
[`canonicalise_resource_refs`](../../kernel/src/layer/mod.rs#L1305) at
[build time](../../kernel/src/layer/mod.rs#L1073) and lost on the next serialisation. Hence 142 call
sites of `as_iri_str` / `as_iri` / `as_iri_array` to reconcile it, and a special case in
[`values_equal`](../../kernel/src/query/functions.rs#L149) so `ResourceRef("urn:x") == String("urn:x")`
— which derived `PartialEq` does not share. The data model already states the principle, in
[`as_iri_array`'s own doc](../../kernel/src/ontology/resource.rs#L179): *"the distinction between
string literals and resource references is made by the property's data_type, not at parse time."*

---

## 3. What the rules give

**One validator.** Validating an inductive value IS validating a resource against its type. Rule 23
already recurses into every embedded resource that declares `is_a` and applies the full rule set
([`validation/mod.rs:572`](../../kernel/src/validation/mod.rs#L572)), using `is_a` presence as the
discriminator between a typed instance and an opaque carrier. So a value's arguments are validated
as resources in their own right, and the rule for a value need only check one level.
`walk_inductive_value`'s parallel traversal
([`rules/inductive.rs:192`](../../kernel/src/validation/rules/inductive.rs#L192)) is subsumed.

**One traversal, one equality, one canonicalisation.** Today each has a `serde_json` twin, because
`Embedded(Box<Resource>)` keeps the recursion inside the data model and an inductive value in
`serde_json::Value` leaves it: `value_refs` / `collect_refs_from_value` against `json_mentions`,
`values_equal` against `alpha_canonicalize_proposition_json`, `value_to_cbor` against
`json_value_to_cbor`. Canonicalisation has no twin at all, which is why a term's interior is never
canonicalised.

**References for free.** A chain-resident value has an `@id`; naming it is a `ResourceRef`. No
sentinel, no second reference kind, and `core:mentions` follows it because
[`value_refs`](../../kernel/src/layer/declaration_order.rs#L128) already descends `Embedded`.

**α-canonicalisation moves to `Exp`,** where α-equivalence lives. It currently renames bound
variables in JSON, only because the mirror is JSON.

---

## 4. Where the code disagrees, and what it costs

| site | today | under §2 |
|---|---|---|
| [`encode_type` / `decode_type`](../../kernel/src/program/eigentt_type_mirror.rs#L75) | `serde_json::Value` tagged dicts | `Exp` ↔ `Value` bridges |
| `eigentt:Term.CtorApp` | 2 args + an `App` spine | `Embed(value)`, 1 arg |
| [Rule 16 walker](../../kernel/src/validation/rules/inductive.rs#L192) | walks JSON against the SLOT's inductive | deleted; Rule 23 + a one-level rule |
| `json_mentions` | JSON twin of `value_refs` | deleted |
| `alpha_canonicalize_proposition_json` | α-equivalence on JSON | on `Exp` |
| authored values in shipped ontologies | 114 tagged dicts in 5 slots | resource form |
| `Value::Vector` | in the persisted value type; [panics if serialised](../../kernel/src/ontology/eigon_cbor.rs#L217) | out of `Value` (§6) |

The 114 authored values sit in `core:type_name` (89), `formulas:operator_signature` (20),
`core:param_kind` (3), `eigentt:expected_type` (1) and `core:result_sort` (1) — a small, uniform set
migrable by script.

---

## 5. Retrofit

Each step lands green. There is no atomic cut-over: D84 §7 claimed one and the attempt ran with a
never-green tree for its whole length, so every fact arrived as a failure that could not be acted on.

| # | step | green after |
|---|---|---|
| 1 | `core:is_a` admits `core:InductiveType`; declare `core:ctor` / `core:args`; add the one-level value rule. Nothing produces the shape yet, so this is additive. | yes |
| 2 | Migrate the 114 authored values; they parse as `Embedded` natively and round-trip through CBOR unchanged. | yes |
| 3 | `encode_type` emits value resources and `decode_type` reads them; `CtorApp` → `Embed`. The one irreducible step, and it is smaller than D84 §7's version because 1, 2, 4 and 5 are outside it. | yes |
| 4 | Delete the twins: `json_mentions`, the Rule 16 walker, α-canonicalisation on JSON. | yes |
| 5 | `Vector` leaves `Value` — the query engine takes a domain extending the data model, and serialising a transient becomes a type error rather than a panic. | yes |

One reseed, after step 3, folded into the one already owed for P4 and P5.

---

## 6. Open

1. **`core:args` element typing.** All 17 other `value_array` properties in the tree have a uniform
   element type (12 `string`, 2 `integer`, 3 `float`). A positional argument list whose element types
   come from the constructor is not a homogeneous array, and `core:element_type` is `then_requires`d
   for `value_array` and admits only the five primitives. Either `element_type` becomes optional when
   the types are determined elsewhere, or `core:args` is not a `value_array`. R4 says where the types
   come from; it does not say how the property declares that.
2. **`ResourceRef` — DECIDED: retire it, as its own change, after the reseed.** Not open, and not
   part of this note's retrofit. R5 is what makes the decision available: once a reference is an IRI
   read through an accessor and no variant is canonical, the variant has no job left. It was only
   ever justified by the promise in `LayerBuilder::build` that readers "can then assume one shape per
   data_type" — which the wire format cannot keep, because CBOR writes `Text` and reads back
   `String`.

   Measured cost of retiring: **90 reader sites** match it across 34 files, **~660** construction
   sites become `Value::String`, **140** calls to `as_iri_str` / `as_iri` / `as_iri_array` collapse
   to `as_str` plus a parse, and both `values_equal`'s special case and `canonicalise_resource_refs`
   are deleted. The 90 matches are not merely migration cost: the variant is not reliably produced,
   so each is a place a reloaded chain can silently read nothing — the bug class that already shipped
   once, as the empty topology graph `as_iri_str` exists to fix.

   Sequenced after the reseed because it moves equality and join semantics while the reseed moves
   hashing and the manifest, and two changes that can each silently produce wrong results should not
   land together.

   It is the same question as item 1: whether a bare IRI in `core:args` is a reference or a
   `core:string` literal is decided by the constructor's argument type (R4); whether a bare IRI in a
   `core:resource` slot is a reference is decided by `data_type`. One fix serves both.

3. **The wire abbreviation.** Whether `ctor` / `args` may appear as short keys, expanded on read, the
   way a `ResourceRef` already serialises as a bare IRI string. A codec table entry, reversible, and
   explicitly NOT a design fork — an earlier draft made it one on a size argument that does not
   survive the storage layer setting `DBCompressionType::Lz4`.

---

## 7. Why the two notes it replaces went wrong

Worth recording, because the failure was in method rather than in any one conclusion.

Seven claims about the implementation were asserted without being checked, and each drove work that
had to be undone: that `cardinality: list` was already implemented (it was in D32's prose only); that
the `Value` split "cannot be built" (`canonicalise_resource_refs` does exactly that); that
`json_mentions_of_value` produced spurious `core:mentions` (its sole caller is already gated,
[`layer/index.rs:341`](../../kernel/src/layer/index.rs#L341)); that `CtorApp` had one job; that `is_a`
on embedded resources was unenforced (Rule 23 enforces it); that the wire abbreviation was a size
decision; and that slot threading and constructor-argument typing were the same thing.

The common shape: a claim formed first, then checked narrowly to confirm it. The correction is the
citation discipline this note is written under — and the census in R2, which settled in one query a
question that had been revised three times.
