# schema.org meta-ontology vs Eigenius `core` meta-ontology

> The principled basis for the D57 mapping discipline. Rather than ad-hoc
> "tiers", the schema.org → Eigenius mapping is derived **construct by
> construct** from a comparison of the two meta-ontologies. The D57 §3 rules are
> consequences of the correspondence table below.
>
> schema.org side anchored in its canonical data model:
> [schema.org/docs/datamodel.html](https://schema.org/docs/datamodel.html),
> [DataType](https://schema.org/DataType), [Enumeration](https://schema.org/Enumeration).
> Eigenius side: [`ontologies/core/core-ontology.json`](../../ontologies/core/core-ontology.json).

## 1. The two meta-ontologies at a glance

Both are **reflective** (the metamodel is expressed in itself: schema.org Types
are `rdfs:Class`es; `core:Class` is an instance of itself). They differ most in
**conformance stance**:

- **schema.org is descriptive / recommendation-based.** `domainIncludes` and
  `rangeIncludes` are *expectations*, not constraints; nothing is required;
  multiple types per property and per instance are allowed; there is no validator
  that rejects non-conforming data. It is a shared *vocabulary*, not a schema
  language. (Confirmed verbatim at [datamodel.html](https://schema.org/docs/datamodel.html):
  "a **multiple inheritance hierarchy** where each type may be a sub-class of
  multiple types"; "we expect a property value of type Person … [but] will get a
  text string, even if our schemas don't formally document that expectation";
  tools "are not obliged to treat unexpected structures as errors".)
- **Eigenius `core` is prescriptive / structurally validated.** `requires`,
  `allows_only`, `data_type`, and `class_types` are enforced at commit — a
  non-conforming resource does not load. It is a *type system*.

This single difference drives most mapping decisions: schema.org constructs that
*recommend* generally become Eigenius `recommends` (preserving open-world), and
schema.org's one genuinely *closed* construct — the **Enumeration** — becomes the
one place we *enforce* (`allows_only`).

## 2. schema.org meta-ontology (the constructs)

| Construct | Role |
|---|---|
| **Type** (`rdfs:Class`), root `Thing` | a class in the multiple-inheritance type hierarchy |
| **`rdfs:subClassOf`** (multi) | superclass links (multiple inheritance) |
| **DataType** hierarchy | the literal value types: `Text` (⊃ `URL`), `Number` (⊃ `Integer`, `Float`), `Boolean`, `Date`, `DateTime`, `Time` |
| **Property** (`rdf:Property`), *global* | a named relation, not owned by a class |
| **`schema:domainIncludes`** (multi) | the Types a property is *expected* on |
| **`schema:rangeIncludes`** (multi) | the Types a property's value is *expected* to be — **a union** |
| **Enumeration** + member individuals | a Type `subClassOf Enumeration` whose valid values are a *fixed set of typed instances* (e.g. `DayOfWeek` → `Monday`…`Sunday`) |
| `rdfs:subPropertyOf`, `schema:inverseOf`, `schema:supersededBy`, `owl:equivalentClass`, `sameAs` | relational / inference semantics |
| **Role** reification, `additionalType` | qualified-value pattern; instance-level extra typing |

## 3. Eigenius `core` meta-ontology (the constructs)

| Construct | Role |
|---|---|
| **`core:Class`** (instance of itself) | an ontological type |
| **`core:subclass_of`** (multi) | superclass links; subclass inherits `requires`/`recommends` |
| **`core:DataType`** | the primitive value types: `string`, `integer`, `float`, `boolean`, `resource`, `resource_array`, `value_array`, `json`, `inductive`, `template` |
| **`core:Property`** | a named attribute |
| **`core:data_type`** (single) | the property's value type |
| **`core:class_types`** (set) | for `resource`(`_array`): the allowed classes/inductives of the value |
| **`core:allows_only`** (set) | restricts a resource-valued property to a fixed set of resources *by IRI identity* |
| **`core:domain`** (multi) | the classes a property may be used on |
| **`core:requires`** / **`core:recommends`** (set) | per-class mandatory / advisory properties |
| **`core:element_type`**, **`core:format`**, `pattern`, `min/max_value`, `min/max_length` | finer value constraints |
| **`core:ConditionalRequirement`** (`condition` → `then_requires`/`then_recommends`) | conditional cardinality |
| **`core:InductiveType`** | constructor-defined inductive values |

## 4. Construct correspondence (the core of the mapping)

| schema.org | Eigenius `core` | Fidelity |
|---|---|---|
| Type / `rdfs:Class` | `core:Class` | **direct** |
| `rdfs:subClassOf` (multi) | `core:subclass_of` (multi) | **direct** (ESL surface allows one parent; JSON allows many) |
| `rdf:Property` (global) | `core:Property` | direct |
| `schema:domainIncludes` (multi, **advisory**) | each domain class's **`core:recommends`** (inverted) | **not `core:domain`** — domainIncludes says "expected on", advisory; `core:domain` *restricts* usage and would over-enforce. Subclasses inherit `recommends`, matching schema.org's apply-to-subtypes. |
| `rangeIncludes` — **Class members** | **`core:class_types`** | **direct** — this *is* the mapping for the class part of a range |
| `rangeIncludes` — **DataType members** | `core:data_type` (+ `core:format`, §5.1) | a union of distinct literal types collapses to one scalar (§6) |
| `rangeIncludes` — **mixed (class + literal)** | `class_types=[classes]`, `data_type=resource` | **entity-first** (D57 §3.3); the dropped literal option is recoverable from `source_irl` (not duplicated on-chain) |
| **Enumeration** + members | `core:Class` + `DeclaredResource` member instances; **consumed via `class_types` + `allows_only`** | **§5** — Eigenius *enforces* the set (stricter) |
| DataType: `Text` | `string` | direct |
| DataType: `URL` (⊂ Text) | `string` + **`core:format = iri`** | validated (§5.1) |
| DataType: `Integer` / `Number`,`Float` / `Boolean` | `integer` / `float` / `boolean` | direct |
| DataType: `Date` / `DateTime` / `Time` | `string` + **`core:format = date`/`datetime`/`time`** | validated when the range is a single temporal type (§5.1) |
| `additionalType` (instance-level) | `core:is_a` (multi) | partial |
| *(no required-property concept)* | `core:requires` | schema.org props → **`recommends`** (preserve open-world) |
| *(none)* | `core:ConditionalRequirement`, `format`, `pattern`, `min/max` | Eigenius-only; unused by the import |
| `rdfs:subPropertyOf`, `inverseOf`, `supersededBy`, `equivalentClass`, `sameAs` | *(inert provenance only — no reasoner)* | **not mapped as active relations** (Tier 3) |
| **Role** reification | *(none)* | not mapped |

## 5. Enumeration ↔ `allows_only` (the closed-set correspondence)

A schema.org **Enumeration** is the one schema.org construct with *closed-world*
intent: a Type `E subClassOf Enumeration` whose valid values are a **fixed set of
typed member individuals** `{m₁…mₙ}` (each `mᵢ a E`). A property `p` with
`rangeIncludes E` should take a value from exactly that set.

That is precisely Eigenius's **enumeration idiom** — the one `core` itself uses for
`core:data_type` (a `core:resource` property with `class_types core:DataType`,
`allows_only [string, integer, …]` over the `DataType` instances) and that
`reflection:epistemic_status` uses over `EpistemicStatus`. So:

```
schema.org                          Eigenius core
─────────────                       ─────────────
E  (subClassOf Enumeration)    →    core:Class  E      (subclass_of the enum parent)
m₁ … mₙ  (instances of E)      →    DeclaredResource m₁ … mₙ   (is_a E)
property p  (rangeIncludes E)  →    core:Property p {
                                        data_type   = resource
                                        class_types = [E]
                                        allows_only = [m₁ … mₙ]   ← the enumeration's closed set
                                    }
```

`class_types=[E]` says "a member of E"; **`allows_only=[m₁…mₙ]` enforces the closed
set** at commit. This is a **fidelity gain**: schema.org only *recommends* the
member set; Eigenius *rejects* a non-member (a typo'd enumeration value won't
load). The member set lives in exactly one place (the instances), and `allows_only`
references them — no duplication.

*Open enumerations.* Some schema.org enumerations also admit `DefinedTerm`/`Text`
(an "open" enumeration). Those map with `class_types` widened (e.g.
`[E, DefinedTerm]`) and **no `allows_only`** — the set isn't closed, so don't
enforce it. The closed/open distinction is read from whether the enumeration
permits open terms.

### 5.1 DataType subtypes ↔ `core:Format` (the literal-refinement correspondence)

schema.org's literal **DataType subtypes** are essentially *format refinements of a
string*, and they mirror Eigenius's `core:Format` vocabulary almost one-to-one:

| schema.org DataType | Eigenius | validated? |
|---|---|---|
| `Text` | `string` | — |
| `URL` (⊂ `Text`) | `string` + `core:format = iri` | ✓ RFC 3987 |
| `Date` | `string` + `core:format = date` | ✓ ISO-8601 `YYYY-MM-DD` |
| `DateTime` | `string` + `core:format = datetime` | ✓ |
| `Time` | `string` + `core:format = time` | ✓ |

So a property whose range is a *single* refined DataType maps to `string` + the
matching `core:format`, and the kernel **validates** the value — the same
fidelity-gain pattern as enumeration `allows_only` (§5). `core:format` therefore
*subsumes* what a separate provenance annotation would have recorded for these
cases, and validates on top.

**Two caveats, both seen in the m2 probe:**
- *Unions of distinct formats don't refine.* `datePublished` = `Date | DateTime`
  spans two formats; no single `core:format` covers it (`date` rejects datetimes,
  `datetime` rejects date-only), so it stays plain `string`. Likewise `Text | URL`
  is degenerate (`URL ⊂ Text`) and its values may be non-IRI (`encodingFormat`
  holds MIME strings like `text/csv`), so it stays plain `string`.
- *No on-chain range cache.* The original `rangeIncludes` is canonically recorded
  by `core:source_irl` (and the source JSON-LD the generator reads); we do **not**
  duplicate it in a bespoke property. `core:format` carries the *validated*
  refinement; `source_irl` carries the *full* provenance.

## 6. Where the metamodels genuinely diverge

1. **Enforcement vs recommendation.** schema.org `domainIncludes` is advisory, so
   it maps to **`core:recommends`** (advisory) on each domain class — *never*
   `core:domain`, which *restricts* a property's usage (schema.org has no such
   restriction; `core:domain` is therefore unused by the import). Likewise
   properties are `recommends`, never `requires`. The one place we *do* enforce is
   the enumeration's closed set (`allows_only`) — the one set schema.org means to
   close. (Class members of `rangeIncludes` → `class_types` is typing of the value,
   a deliberate entity-binding gain, not a usage restriction.)
2. **Union ranges have no single-type analog.** `core:data_type` is single. A
   class-union → `class_types` (lossless set). A mixed union → entity-first
   (`class_types` + `data_type=resource`; the literal option dropped). An
   all-DataType union of distinct formats (`Date|DateTime`) → one scalar, no
   `core:format`. The original union is recoverable from `source_irl`.
3. **DataType refinement via `core:format` (§5.1).** A single refined DataType
   (`URL`, `Date`, `DateTime`, `Time`) → `string` + the matching `core:format`
   (validated). `Text` and format-spanning unions → plain `string`.
4. **No-analog constructs.** schema.org has no `requires`/`ConditionalRequirement`/
   `format`/`pattern` (Eigenius-only, unused by the import). Eigenius has no
   `subPropertyOf`/`inverseOf`/`supersededBy`/`Role` (schema.org-only) — kept as
   inert provenance, never active relations (no reasoner; D57 §4).

## 7. Implication for D57 §3

The §3 "tiers" are **consequences** of row 4's `rangeIncludes` correspondence, not
independent rules:

- single DataType → scalar, refined by `core:format` where applicable (§5.1);
  single Class → `class_types=[C]`;
- all-Class union → `class_types=[…]`; mixed union → entity-first
  (`class_types` + `data_type=resource`; `source_irl` is the full-range record);
- **Enumeration range → `class_types=[E]` + `allows_only=[members]`** (§5) — closed-set
  enforcement.

The generator (m3) implements the **correspondence table (§4)** directly; the cut
(§6 divergences, plus Tier-3 inert relations) is the residual it reports.
