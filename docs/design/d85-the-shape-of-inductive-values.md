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

An inductive **value** is a `Resource`. It states its own type, names its constructor, carries its
arguments, and is subject to the rules every resource is subject to:

```json
{
  "urn:eigenius:core:is_a":                ["urn:eigenius:eigentt:Term-App"],
  "urn:eigenius:eigentt:Term-App-fn":     { … },
  "urn:eigenius:eigentt:Term-App-arg":    { … }
}
```

**The constructor is what `is_a` names**, and each argument is a **declared property** on the
constructor's class. There is no `core:ctor` and no `core:args` — §6.1 settles why, and the whole of
it follows: arity is Rule 1, argument types are Rules 5 and 6, and a constructor class is
`subclass_of` the inductive so a slot declaring `class_types eigentt:Term` accepts the value
unchanged. An earlier draft of this section put the constructor in `core:ctor` and the arguments in a
positional `core:args` array; that shape is gone, and the question of how `core:args` declared its
element types is what removing it dissolved.

Inline, it is `Value::Embedded`. Named, it is **the resource's IRI** — and the design deliberately
does not say which variant carries that IRI, because nothing may depend on it. Reading a reference
goes through `Value::as_iri`, never through a match.

> **Written when there were two candidates.** The parser and CBOR both produced `Value::String`,
> while `canonicalise_resource_refs` could upgrade it to `Value::ResourceRef`, and `as_iri` accepted
> either. **`Value::ResourceRef` was retired on `2026-08-31`** (§6.2), so a reference is now always a
> `Value::String`. The rule is unchanged and the passages below that name the variant are the
> argument that produced that retirement, kept for it.

`Value` gains **no** variants. That is the point, and it is what makes the two forms above the same
two forms every resource-valued slot already has.

Five rules follow. They are the type discipline; the representation above is a consequence of them,
and every place the code disagreed with them is listed in §4.

---

## 2. The rules

### R1 — A value states its own type

`is_a` names the class the value inhabits. For an inductive value that is the **constructor's**
class, which is `subclass_of` the inductive — see §6.1, decided after this rule was first written.
The holding property's `class_types` is a **constraint to check it against**, not the source of
truth, and it is satisfied through `subclass_of` by the machinery that already resolves it.

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

*Why an injection and not subsumption.* `eigentt:Term-App` declares both arguments `eigentt:Term`
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

§6.1 is what discharges this rule rather than restating it: each argument is a **declared property**
carrying its own `data_type` and `class_types`, so "the types come from the constructor" is a fact
about where the property is declared, and the three readings above are the three ordinary
`data_type`s a property may have. Nothing needs to derive an argument's type at encode time — it is
looked up the way every other property's type is.

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

**This is why §1 declines to name a variant for the reference form.** `ResourceRef` **was** the
existing instance of the same mistake, so building on it would have imported the defect this rule
exists to avoid. A reference is an IRI, and which variant holds it is not part of the design — which
is what made retiring the variant (§6.2, done) a consequence of the rule rather than a separate
decision.

*The debt itself*, recorded and not fixed here. `ResourceRef` is
`String` plus a schema lookup, produced only by
[`canonicalise_resource_refs`](../../kernel/src/layer/mod.rs#L1305) at
[build time](../../kernel/src/layer/mod.rs#L1073) and lost on the next serialisation. Hence 142 call
sites of `as_iri_str` / `as_iri` / `as_iri_array` to reconcile it, and a special case in
[`values_equal`](../../kernel/src/query/functions.rs#L149) so `ResourceRef("urn:x") == String("urn:x")`
— which derived `PartialEq` does not share. The data model already states the principle, in
[`as_iri_array`'s own doc](../../kernel/src/ontology/resource.rs#L179): *"the distinction between
string literals and resource references is made by the property's data_type, not at parse time."*

### R5a — what a traversal does at each variant

R5 leaves this implicit and it should not be. Once an inductive value is `Embedded`, the two
container variants mean exactly one thing each, and a reader that walks a value tree for references
has no discretion:

| variant | meaning | descend? |
|---|---|---|
| `Embedded` | a resource — an inline inductive value, or any other typed instance | **yes**; its IRIs are references |
| `Json` | `core:json`, and only that: opaque, "not validated by the ontology" | **no**; an IRI inside opaque data is not a reference |

*This is not a style preference; the tree currently gives three different answers.* Eight functions
implement the same value-tree traversal — `declaration_order::value_refs`,
`supporting::collect_refs_from_value`, `merge/lca::collect_iri_refs_into`,
`merge/cascade::collect_orphaned_refs_in_value`, `merge/resolve::value_mentions_iri`,
`merge/resolve::substitute_iri_in_value`, `dcg/chart/attribute::value_refs`, and the JSON twin
`term_mentions::json_mentions` — about 240 lines. On `Value::Json` they variously descend, ignore,
or descend conditionally on whether the property is term-valued. D79 §2.2 had to make `value_refs`
call `json_mentions` so that `core:mentions` and `MutualInductives` would agree; that repaired one
pair of the eight.

The three-way split exists only because `Value::Json` today carries both jobs — genuinely opaque
data AND inductive values, whose interior IRIs *are* references. §1 separates them, so the
conditional (`if term_valued`) is not needed: the variant already says which is which.

**Consolidation follows this note; it must not precede it.** Six of the eight differ only in what
happens at a string leaf, so they collapse to one visitor plus one rebuilding variant — but doing
that before the `Json`/`Embedded` split would have to pick one of the three current answers and
would freeze it. `json_mentions` then disappears outright: an inductive value's interior is `Value`,
so the traversal that already handles `Embedded` handles it.

`dcg/chart/attribute::value_refs` is the other outlier — it does not descend into `Embedded` at all.
Not reachable today, because its one caller passes `core:is_a`, a flat array of IRI strings. It is
still wrong by the table above, and it is the kind of wrong that is invisible until someone reuses a
function whose name promises the general traversal.

---

## 3. What the rules give

**One validator.** Validating an inductive value IS validating a resource against its type. Rule 23
already recurses into every embedded resource that declares `is_a` and applies the full rule set
([`validation/mod.rs:572`](../../kernel/src/validation/mod.rs#L572)), using `is_a` presence as the
discriminator between a typed instance and an opaque carrier. So a value's arguments are validated
as resources in their own right. Under §6.1 there is no rule for a value **at all** — its arguments
are declared properties, so Rule 1 checks arity and Rules 5 and 6 check each argument's type.
`walk_inductive_value`'s parallel traversal
([`rules/inductive.rs:192`](../../kernel/src/validation/rules/inductive.rs#L192)) is subsumed, and
replaced by nothing.

**One traversal, one equality, one canonicalisation.** Today each has a `serde_json` twin, because
`Embedded(Box<Resource>)` keeps the recursion inside the data model and an inductive value in
`serde_json::Value` leaves it: `value_refs` / `collect_refs_from_value` against `json_mentions`,
`values_equal` against `alpha_canonicalize_proposition_json`, `value_to_cbor` against
`json_value_to_cbor`. Canonicalisation has no twin at all, which is why a term's interior is never
canonicalised.

**References for free.** A chain-resident value has an `@id`; naming it is that IRI. No sentinel,
no second reference kind, and `core:mentions` follows it because
[`value_refs`](../../kernel/src/layer/declaration_order.rs#L128) already descends `Embedded`. (This
paragraph said "naming it is a `ResourceRef`" until that variant was retired on `2026-08-31` per
§6.2 — R5 is what made the retirement available, and the paragraph reads the same without it, which
was the point.)

**α-canonicalisation moves to `Exp`,** where α-equivalence lives. It currently renames bound
variables in JSON, only because the mirror is JSON.

---

## 4. Where the code disagrees, and what it costs

| site | today | under §2 |
|---|---|---|
| [`encode_type` / `decode_type`](../../kernel/src/program/eigentt_type_mirror.rs#L75) | `serde_json::Value` tagged dicts | `Exp` ↔ `Value` bridges |
| `eigentt:Term.CtorApp` | 2 args + an `App` spine | `Embed(value)`, 1 arg |
| [Rule 16 walker](../../kernel/src/validation/rules/inductive.rs#L192) | walks JSON against the SLOT's inductive | deleted, and replaced by NOTHING — Rule 23 recurses, Rule 1 checks arity, Rules 5 and 6 check argument types (§6.1) |
| `json_mentions` | JSON twin of `value_refs` | deleted |
| `alpha_canonicalize_proposition_json` | α-equivalence on JSON | on `Exp` |
| authored values in shipped ontologies | 114 tagged dicts in 5 slots | resource form |
| `Value::Vector` | in the persisted value type; [panics if serialised](../../kernel/src/ontology/eigon_cbor.rs#L217) | out of `Value` (§6) |

The **123** authored values sit in `core:type_name` (89), `formulas:operator_signature` (20),
`core:param_kind` (9), `core:result_sort` (4) and `eigentt:expected_type` (1) — a small, uniform set
migrable by script. It was 114 when this note was written; the three `witness:Is*As` predicates P7
moved into core carry 6 more `param_kind` values and 3 more `result_sort`.

**They do not all validate the same way**, which is what decides the step order above. Rule 21
fires only on slots ranged on `eigentt:Term` or `eigentt:Judgement`, and exempts `core:type_name`
and `core:param_kind` by name; `core:result_sort` is ranged on `core:Level` and
`formulas:operator_signature` on `formulas:FormulaTerm`, so neither reaches it either. Exactly one
authored slot — `eigentt:expected_type` — is Term-ranged and unexempt.

---

## 5. Retrofit

Each step lands green. There is no atomic cut-over: D84 §7 claimed one and the attempt ran with a
never-green tree for its whole length, so every fact arrived as a failure that could not be acted on.

| # | step | green after |
|---|---|---|
| 1 | `core:subclass_of` admits `core:InductiveType`, so a constructor class can name its inductive; derive the constructor classes and argument properties (§6.1), **with Rule 25**, the two-sided closedness check (§6.1) — a class `subclass_of` an inductive must be declared in that inductive's own layer AND correspond to an entry in its `core:ctors`. Without it §6.1 silently converts a closed type into an open one. **No `core:ctor`, no `core:args`, and no new value rule** otherwise — arity is Rule 1, argument types are Rules 5 and 6. Nothing produces the shape yet, so this is additive. | yes |
| 2 | `decode_type` READS a value resource as well as a tagged dict; `encode_type` still emits the dict. Expand before migrate: nothing is rewritten, so nothing can break, and the codec stops being the reason the new shape is refused. | yes |
| 3 | Migrate the 123 authored values — each `{"ctor": C, "args": [a…]}` becomes a resource whose `is_a` names the constructor class step 1 derived and whose arguments are the named properties on it — then `encode_type` emits the resource form and `CtorApp` → `Embed`. | yes |
| 4 | Delete the twins: `json_mentions`, the Rule 16 walker, α-canonicalisation on JSON. | yes |
| 5 | `Vector` leaves `Value` — the query engine takes a domain extending the data model, and serialising a transient becomes a type error rather than a panic. | yes |

One reseed, after step 3, folded into the one already owed for P4 and P5.

**Steps 2 and 3 were the other way round until `2026-09-01`, and that order cannot land green.**
Migrating a value first means writing the resource form into a slot whose validation still reads
`Value::Json`: Rule 21 routes every `eigentt:Term`- or `eigentt:Judgement`-ranged slot through the
D47 codec, and the codec rejects `Value::Embedded` outright. It would have looked survivable,
because Rule 21 exempts exactly two properties by name — `is_declaration_internal` covers
`core:type_name` and `core:param_kind` — and those two hold **98 of the 123** authored values, so a
migration that started there would have passed and the remaining 25 would have failed at the end.
Expand first, migrate second: the codec learns to read both shapes while nothing has changed, and
the migration then rewrites values a reader already accepts.

---

## 6. Open

0. **`eigentt:Term` declares a constructor the D47 codec cannot decode.** The inductive declares
   20 constructors including `SizeSort`; `decode_type_json` has no arm for it, so
   `{"ctor": "SizeSort", "args": []}` fails with *"unknown eigentt:Term ctor"*. Found while
   testing step 2, and **pre-existing** — the two shapes disagree about what the type has, in
   whichever encoding. Nothing authored uses it (`SizeSort` appears in no chain value), which is
   why it has gone unnoticed. Either the codec gains the arm or the declaration loses the
   constructor; both are small, and which one is right depends on whether sized types are coming
   back after eigenius#218 retired them.



1. **`core:args` element typing — DECIDED `2026-08-31`: there is no `core:args`.** A constructor
   gets a **class**; each argument becomes a **named property** on that class. The question dissolves
   rather than being answered, which is why this beats both options the note originally offered.

   ```
   class     eigentt:Term-App        subclass_of eigentt:Term
                                    requires eigentt:Term-App-fn, eigentt:Term-App-arg
   property  eigentt:Term-App-fn    data_type core:resource   class_types eigentt:Term
                                    domain eigentt:Term-App
   value     { "is_a": ["urn:eigenius:eigentt:Term-App"],
               "urn:eigenius:eigentt:Term-App-fn":  { … },
               "urn:eigenius:eigentt:Term-App-arg": { … } }
   ```

   **Everything that had to be built is already built.** Arity is Rule 1 (required properties).
   Argument types are Rules 5 and 6 (`data_type`, `class_types`) — so R4 is *implemented by the
   ontology* rather than threaded through the encoder. Membership works because
   [`core:InductiveType` is itself `is_a: [core:Class]`](../../ontologies/core/core-ontology.json), so
   a constructor class may be `subclass_of` its inductive, and
   [`is_instance_of_any`](../../kernel/src/validation/mod.rs#L629) already resolves `class_types`
   through `is_subclass_of` — a slot declaring `class_types eigentt:Term` accepts a value whose
   `is_a` is `eigentt:Term-App` with no change. Argument ORDER lives in the declaration's ordered
   `core:arg_types`, so a value cannot get it wrong; positional indexing disappears from the value.

   **`core:ctor` goes too.** The constructor is what `is_a` names. Step 1 below was going to declare
   two properties; it declares neither.

   *Why not the two options this item used to offer.* Making `core:element_type` optional weakens a
   checkable invariant on all 18 `value_array` properties to accommodate one, and leaves an absent
   `element_type` ambiguous between *heterogeneous by design* and *omitted by mistake*. Giving
   `core:args` a data type of its own is correct as far as it goes, but data types are matched by IRI
   in [`type_check.rs`](../../kernel/src/validation/rules/type_check.rs) under a
   `_ => true, // Unknown data type, skip` arm, so a new one whose arm is forgotten accepts
   everything silently.

   **The classes are DERIVED, not authored.** `core:ctors` stays the single declaration; the
   constructor classes and their argument properties are a projection of it, materialised into the
   layer that declares the inductive. Nothing writes one by hand, in ESL or in JSON, so the two
   declaration surfaces stay as they are and cannot disagree with each other.

   This is what keeps closedness STRUCTURAL rather than merely checked. Deriving them means there
   is no authoring step to police: a constructor class exists because `core:ctors` has an entry,
   and there is no other way for one to come into being. Rule 25 below still holds — it is what
   answers a class someone writes by hand anyway — but on the normal path nothing ever trips it.

   Materialised at **layer build**, before the content hash, so they are ordinary persisted
   resources rather than an in-memory convenience. That distinction is load-bearing and was learned
   the hard way: `canonicalise_resource_refs` was a build-time rewrite that did NOT survive CBOR,
   so a reloaded chain read a shape no reader could rely on (§6.2). These are resources in the
   layer, hashed with it, and identical on reload.

   **How an argument's type becomes a property's.** `core:type_name` holds a D47-encoded term, and
   measured across the 88 JSON-declared arguments it takes exactly two shapes: **86 `ConstRef`** and
   **2 `Var`**.

   | `type_name` | property declares |
   |---|---|
   | `ConstRef(core:string / integer / float / boolean)` — 26 | that primitive as `data_type` |
   | `ConstRef(X)` for any other X — 60 | `data_type: core:resource`, `class_types: [X]`. Subsumption then accepts a value whose `is_a` is one of X's constructor classes, which is the whole mechanism |
   | `Var(A)` — 2 | `data_type: core:resource`, and **no `class_types`** |

   The two `Var` cases are `core:List.cons.head` and `core:Option.some.value` — the element type of
   a PARAMETRIC inductive. A property cannot carry `A`, and inventing a parametric-property
   mechanism for two arguments would be absurd. It is also unnecessary, because R4 already says
   where such a type comes from: for a parametric constructor the argument's type is the
   constructor **applied to its type arguments**, which is a typing fact, not a schema fact. The
   property declares what is structurally checkable — that the argument is a value — and the
   instantiation is checked where instantiation is checked, by Rule 21 and the NbE checker. This is
   the same split R5a draws between what a structural walk decides and what the type checker
   decides, and R3 between a closed value and an open term.

   **A constructor class is not an ordinary subclass, and the difference is closedness.**
   An inductive type is CLOSED: its constructors are exactly the entries in `core:ctors`, which is
   what makes exhaustiveness checking sound (`non-exhaustive match: missing case for …`,
   [`program/expr.rs:742`](../../kernel/src/program/expr.rs#L742)), makes the eliminator total, and
   makes "no user-constructible inhabitant" true of the zero-ctor witness types. A `core:Class` with
   `subclass_of` is OPEN: any later layer may add one. This rule borrows the Class mechanism, so it
   must shut the openness off explicitly.

   *What is given away, precisely.* Today `core:ctors` holds **embedded** `InductiveCtor` resources
   — no `@id`, inside the inductive's own resource — so closedness is **structural**: there is
   nowhere to add a constructor. Moving constructors to top-level classes is what creates the
   opening, because they need IRIs to be named by `is_a` and `domain`, and a top-level resource is
   addable by anyone. Nothing today would stop it: `subclass_of` is validated to reference a
   `core:Class` ([`rules/is_a.rs:221`](../../kernel/src/validation/rules/is_a.rs#L221)) and
   `core:InductiveType` *is* `is_a: [core:Class]`. A later layer could declare
   `eigentt:Term.Bogus subclass_of eigentt:Term`, and a value `is_a: [eigentt:Term.Bogus]` would
   satisfy every slot declaring `class_types eigentt:Term` — `is_instance_of_any` walks
   `subclass_of` — while no match arm covers it and no eliminator handles it.

   **So the rule — RULE 25, two-sided.** Rules 0–24 are in use (2 and 11 are historical gaps);
   this is the next number, and it needs one because steps 2 onward and §5's table refer to it.

   1. A class whose `subclass_of` names a `core:InductiveType` may be declared **only in the layer
      that declares that inductive**. A lower layer cannot reference a higher one, so same-layer is
      the only locality that admits anything at all; the content of the rule is the refusal of every
      layer above.
   2. It must correspond to an entry in that inductive's `core:ctors`. This is the load-bearing
      half: `core:ctors` stays the authority, and it is what exhaustiveness already reads
      ([`program/expr.rs:742`](../../kernel/src/program/expr.rs#L742)), so the class cannot introduce
      a constructor the eliminator does not know about even within the declaring layer.

   Both are commit-time checks on the shape of a declaration, in the same family as Rule 22's
   same-or-lower resolution. Without them §6.1 converts a closed type into an open one silently,
   which is the one thing the inductive/class distinction exists to prevent — and the distinction is
   why an inductive is NOT declared `subclass_of core:Class`, which would have made every inductive
   open by the same mechanism.

   **How the two shapes are admitted — and the answer that was tried first and withdrawn.**
   `core:is_a` and `core:subclass_of` now accept EITHER kind of type, a `core:Class` or a
   `core:InductiveType`, which is what `core:class_types` has done since D32 §3.5. An inductive
   is **not** declared `subclass_of core:Class`.

   *The first attempt was to declare exactly that*, on the reading that one subsumption statement
   serves both shapes. It does, and it type-checked — but a class owes `core:description`, so
   every inductive would owe one too, and **122** declared inductives had none: 28 in the shipped
   ontologies and 94 across experiment chains, demo files and test fixtures, 45 of them domain
   biology in one WRN chain. That cost is the argument against the claim rather than an obstacle
   to it. **An inductive is not a class.** Their contracts differ — an inductive owes `ctors`, a
   class owes `requires` / `recommends` — and, decisively for §6.1, a class is OPEN to subclassing
   by any later layer where an inductive is closed. Making one a subclass of the other asserts a
   containment the design spends the rest of this section denying.

   The 28 ontology descriptions were kept: a declared type with no description is a gap whichever
   way this resolved, and `core:description` is now a `recommends` on `core:InductiveType` so the
   intent is declared rather than incidental.

   **What `is_a` admits, and why it is NOT widened.** A value names its **constructor's class** —
   `is_a: [eigentt:Term-App]` — which is a `core:Class`, so `core:is_a` needs no change at all.
   Only `core:subclass_of` does, so the constructor class can name the inductive.

   Step 1 first widened `core:is_a` too, on the reading that a value names the inductive. It does
   not, and permitting it is worse than redundant: `is_a: [eigentt:Term]` says *some Term,
   constructor unspecified*, which has no arity and no argument types, so Rule 1 and Rules 5 and 6
   have nothing to check. That is the underspecified shape this design exists to make
   inexpressible, and `a_value_may_not_name_the_inductive_itself` now pins its rejection. The
   widening was withdrawn.

   The distinction is worth stating once: `subclass_of` and `class_types` name a **type**, and a
   type is a `core:Class` or a `core:InductiveType`; `is_a` names the **class an instance
   inhabits**, and an instance of an inductive inhabits a constructor's class.

   **Two rejections, not one, and the second was invisible from the first.** `is_a: [eigentt:Term]`
   failed Rule 8 (`ClassTypeMismatch`) — correctly, as it turns out. `subclass_of: [eigentt:Term]`
   failed a *different* rule — Rule 22's `ReferenceCheck` — which held ONE expected class, which is
   why `class_types` had open-coded its Class-or-InductiveType case around it. `ReferenceCheck` now
   holds a LIST, and that open-coded case folded back into it: three fields that reference a type
   share one walk instead of two plus a special case.

   **Naming: `-` is the separator, and it is forced.** A constructor class is
   `<inductive>-<Ctor>`; an argument property is `<inductive>-<Ctor>-<arg>`, fully qualified.

   *Not `.`, which an earlier draft used.* ESL admits `[A-Za-z0-9_]` in a bare identifier and
   `[A-Za-z0-9_-]` in the quoted form `'…'` — **`.` is in neither**, so a dotted `eigentt:Term.App`
   is unspellable in ESL however it is written, and
   [`print.rs`](../../kernel/src/esl/print.rs#L96) hard-errors on such a local name rather than
   emitting something unreadable. A derived class carrying a dot would break ESL printing of every
   chain that contained one.

   *Not `_` either.* It is in the bare charset, but it is ambiguous: constructor names in the tree
   are full of underscores (`cat_np`, `conn_and`, `m_app`), so an inductive `A_B` with constructor
   `C` and an inductive `A` with constructor `B_C` would both derive `A_B_C`. For a MECHANICAL
   projection that is a collision waiting to happen.

   `-` is unambiguous because **no component can contain one**: measured across every
   JSON-declared inductive, constructor and argument name in the tree, all match `[A-Za-z0-9_]+`
   exactly. Splitting on `-` therefore recovers the parts. And it is spellable — `eigentt:'Term-App'`
   — through the quoted form, which exists for precisely this: a name outside the bare charset. An earlier draft scoped properties per
   inductive so that a name reused across constructors at the same type shared one declaration —
   17 of the 88 could have — and named them `eigentt:App.fn`. That is prettier and wrong for a
   DERIVED scheme: sharing requires a same-name-same-type analysis across constructors, and the two
   cases where a name recurs at DIFFERENT types would have to be split out by hand. A projection of
   `core:ctors` should be mechanical, so every property is qualified by its constructor and `domain`
   names exactly one class.

   **Measured, once built.** Across the bootstrap chain's **44** inductives the derivation
   materialises **179 constructor classes and 136 argument properties**. An earlier estimate of
   96 / 112 counted only `ontologies/**`; the chain declares more. All 88 JSON-declared arguments **already carry `core:arg_name`**,
   though it is only a `recommends`. ESL's positional form emits none
   (`a_positional_ctor_arg_carries_no_arg_name`), so the compiler names them — the `arg_0` / `arg_1`
   fallback it already defines for the Julia mirror generator, emitting a dictionary of named
   arguments where it now emits a positional list.
2. **`ResourceRef` — DONE `2026-08-31`**, in two hash-neutral commits before the P7 reseed, as
   planned below. R5 is what makes the decision available: once a reference is an IRI
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

   It was the same question as item 1 while item 1 still had a `core:args` to ask it of: whether a
   bare IRI in an argument position is a reference or a `core:string` literal. Item 1's answer
   removes the position — an argument is a declared property now, so its `data_type` decides,
   exactly as for every other slot. One answer, reached by deleting the second question.

3. **The wire abbreviation — MOOT as posed, `2026-08-31`.** It asked whether `ctor` and `args` may
   appear as short wire keys, expanded on read. Item 1 removed both, so there is nothing left to
   abbreviate here.

   What remains underneath is a **different and much broader question**: an inductive value now
   carries full property IRIs (`urn:eigenius:eigentt:Term-App-fn`), and so does every other resource in
   the system. Whether Eigon-JSON and Eigon-CBOR should abbreviate property IRIs is a codec question
   about ALL resources, not about inductive values, and it does not belong in this note. The size
   argument that made an earlier draft treat it as a design fork does not survive the storage layer
   setting `DBCompressionType::Lz4` either way.

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
