# D57 schema.org mapping — decision log

> The decision points and insights that shaped the mapping document
> ([metamodel note](d57-schemaorg-vs-core-metamodel.md), [D57](../design/d57-schema-org-vocabulary-mapping.md))
> and the conversion program (`crates/eigenius-schemaorg`), compiled from the work.
> Origin tags: **[steer]** = a steering correction during the work; **[bug]** =
> surfaced by a failed load / wrong result; **[analysis]** = derived from the
> meta-ontology comparison; **[probe]** = found by the m2 proof-of-shape probe.

## Sources consulted (grounding)

Two schema.org documents were **fetched** and used to anchor these decisions (per
the grounding discipline — real cited sources, not assumption):

- **[schema.org Developers / distribution](https://schema.org/docs/developers.html)**
  — grounded **input selection (§D, #14)** and **scope (#2)**: the `current` vs
  `all` distinction (`all` adds *retired/attic* terms), the `http` vs `https` URI
  variants, the available formats (JSON-LD, Turtle, Triples, Quads, RDF/XML, CSV —
  CSV split into *Types* + *Properties*), the current release **V30.0
  (2026-03-19)**, and that older releases live under `data/releases/` on GitHub.
- **[schema.org Data Model](https://schema.org/docs/datamodel.html)** — grounded the
  **conformance analysis (#4)** and the advisory-vs-enforced mapping choices
  (`recommends` not `requires`/`domain`; enforce only the enumeration closed set).
  Confirmed verbatim: *"a multiple inheritance hierarchy where each type may be a
  sub-class of multiple types"*; *"one or more types as its domains … ranges"*; and
  the permissive stance — *"we expect a property value of type Person … [but] will
  get a text string, even if our schemas don't formally document that expectation"*;
  tools *"are not obliged to treat unexpected structures as errors."*

(The `schema.org/DataType` and `schema.org/Enumeration` term pages and Eigenius's
`core` ontology were the other anchors — see the
[metamodel note](d57-schemaorg-vs-core-metamodel.md). The **`pending`/`meta` layer
markers** (#2, #17) came not from these docs but from inspecting the V30.0 file
itself — `schema:isPartOf` values: 842 pending, 385 health-lifesci, 27 auto, 26
bib, 6 meta.)

## The cut — what maps vs what is out of scope

The whole point of the mapping is an *explicit* boundary. This is it; the numbered
decisions below give the rationale, and the generator's `coverage.json` gives the
live counts.

### Mapped (in scope)

| schema.org construct | → Eigenius `core` | decision |
|---|---|---|
| Type (`rdfs:Class`) | `core:Class` | #5, #13 |
| `rdfs:subClassOf` (in-scope parents) | `core:subclass_of` | — |
| `rdfs:label` / `rdfs:comment` | `core:short_name` / `core:description` | — |
| DataType (`Text`,`URL`,`Number`,`Integer`,`Float`,`Boolean`,`Date`,`DateTime`,`Time` + `Quantity` family) | a core scalar + `core:format` for `URL`/`Date`/`DateTime`/`Time`; **folded, not emitted as classes** | #7, #8, #16 |
| Enumeration (subclass of `Enumeration`) | `core:Class` | #6 |
| Enumeration member instances | `DeclaredResource` instances (`is_a [E, DeclaredResource]`) | #6, #18 |
| Property (`rdf:Property`) | `core:Property` | — |
| `rangeIncludes` — classes / single DataType / all-Class union / mixed / enumeration | `class_types` / scalar+format / `class_types` / entity-first / `class_types`+`allows_only` | #5–#8, #11 |
| `domainIncludes` (advisory) | each domain class's `core:recommends` (subclasses inherit) | #9 |
| provenance | `core:source_irl` + `reflection:declared_by` on every resource | #13 |

**Scope layers mapped:** core (no `isPartOf`) + hosted extensions
(health-lifesci, auto, bib).

### Out of scope (not mapped)

| schema.org construct / layer | disposition | why | decision |
|---|---|---|---|
| `pending` layer | **excluded** | unstable, may change/be removed | #2, #17 |
| attic / retired terms | **excluded** | absent from the `current` distribution (would be in `all`) | #2, #14 |
| `meta` layer | **excluded** | schema.org's own metamodel (`rdfs:Class`, `domainIncludes`, …) — Eigenius maps *to* `core`, doesn't import it | #2 |
| `rdfs:subPropertyOf`, `schema:inverseOf`, `schema:supersededBy`, `owl:equivalentClass`, `owl:equivalentProperty` | **recorded, not mapped** (inert provenance in `coverage.json`) | inference/relational semantics; Eigenius adopts a vocabulary, not a reasoner | #12 |
| the **Role** reification pattern | **not mapped** | no Eigenius analog | #12 |
| other annotations: `schema:sameAs` (e.g. → Wikidata), `schema:contributor`, `schema:source`, … | **ignored** (not carried, not recorded) | out of the vocabulary-typing scope; candidates for future provenance enrichment | — |
| external-vocabulary references (`dcat:`, `dct:`, `owl:`, `gs1:`, …) as `@id`s or as `subClassOf`/range/`domainIncludes` targets | **dropped** from structural refs | not `schema:` terms; only in-scope `schema:` targets are kept (open-world) | #17 |
| DataType internal structure (a `Quantity`'s unit + value) | **collapsed to `string`** | the `Quantity` family folds to a scalar; structured quantities lose their unit structure | #16 |
| exact on-chain reconstruction of a union range | **not stored** | the dropped literal of an entity-first union is recoverable via `source_irl`, not duplicated on-chain | #10 |
| `core:domain` (the restrictive construct) | **never emitted** | no schema.org analog — `domainIncludes` is advisory → `recommends` | #9 |

**One-line statement.** *Everything schema.org expresses as a typed term — Classes,
DataTypes, Enumerations (+ members), and Properties with their ranges — is mapped,
in scope (core + hosted extensions); everything it expresses as cross-term
inference/equivalence/versioning relations, plus the unstable/retired/meta layers
and external-vocabulary links, is out of scope (recorded as residual where it is a
known relation, ignored otherwise).*

## A. Framing & scope

1. **The deliverable is the whole vocabulary, not a slice.** [steer] D57 began as a
   ~10-property descriptive "minimum slice" (§2.5). Reframed: map *all* of
   schema.org — the mappable part generated, the rest enumerated with reasons. The
   slice was demoted to the m2 proof-of-shape probe.
2. **Scope = core + hosted extensions; exclude pending, attic, meta.** [analysis]
   Realized by taking the `current` distribution (attic absent) and filtering
   `schema:isPartOf` = `pending`/`meta`. Hosted extensions (health-lifesci, auto,
   bib) are kept. (V30.0: 848 terms excluded by layer.)

## B. The method shift — ground the mapping in a meta-ontology comparison

3. **Derive the mapping construct-by-construct from the two meta-ontologies, not
   ad-hoc "tiers."** [steer] The tiers (clean / by-convention / enumeration) became
   *consequences* of the `rangeIncludes` correspondence, documented in a dedicated
   analysis ([metamodel note](d57-schemaorg-vs-core-metamodel.md)).
4. **The defining difference is conformance stance.** [analysis] schema.org is
   *descriptive / recommendation-based* (`domainIncludes`/`rangeIncludes` are
   expectations; nothing required; no validator). Eigenius `core` is *prescriptive /
   structurally validated* (`requires`/`allows_only`/types enforced at commit).
   Confirmed verbatim from schema.org's data-model doc ("tools … are not obliged to
   treat unexpected structures as errors"). **This single difference drives most
   mapping choices** — map advisory schema.org constructs to advisory Eigenius ones
   (`recommends`, not `requires`/`domain`); enforce only where schema.org itself
   closes a set (enumerations).

## C. The correspondence (the discipline)

5. **`rangeIncludes` class members → `core:class_types`.** [steer] `class_types`
   *is* the faithful representation of the class part of a range; an early separate
   provenance string duplicated it.
6. **Enumeration → `class_types` + `allows_only` (closed set).** [steer] A schema.org
   Enumeration is the one closed-world construct: class `E` + fixed member instances.
   Maps to `core:Class E` + member `DeclaredResource`s, and a property ranging over
   it gets `class_types=[E]` + `allows_only=[members]` — the **same idiom `core`
   itself uses** for `core:data_type` (over `core:DataType`) and
   `reflection:epistemic_status`. Eigenius *enforces* the set (fidelity gain;
   schema.org only recommends). Open enumerations widen `class_types`, drop
   `allows_only`. *(Verified live: a member loads; a non-member → `AllowedValueViolation`.)*
7. **DataType subtypes → `string` + `core:format` (validated).** [steer] schema.org's
   literal DataType subtypes mirror core's `Format` vocabulary almost 1:1:
   `URL→iri`, `Date→date`, `DateTime→datetime`, `Time→time`; `Text→string`;
   `Number/Integer/Float/Boolean→` the scalar. `core:format` *validates* (a gain),
   subsuming what a provenance annotation would have recorded.
8. **`core:format` applies only to a single refined DataType where all values
   conform.** [probe] A **format-spanning union** does *not* refine: `Date|DateTime`
   → plain `string` (no single format covers both; year-only values exist and would
   fail strict `date`); `Text|URL` is degenerate (`URL ⊂ Text`) and holds MIME
   strings → plain `string`. Over-applying a format would reject data schema.org
   accepts.
9. **`domainIncludes` → each domain class's `core:recommends` (inverted), NOT
   `core:domain` — restriction vs recommendation.** [steer] The load-bearing
   insight: `core:domain` and `core:recommends` are *different in kind*, not just
   direction.
   - **`core:domain`** (on a property) is an **enforced restriction** — "this
     property may *only* be used on these classes"; using it elsewhere fails
     validation.
   - **`core:recommends`** (on a class) is **advisory** — "instances *should*
     provide these properties"; open-world, never rejected.

   schema.org `domainIncludes` is advisory ("a property is *expected* on these
   types", and routinely used elsewhere). So the faithful target is
   **`core:recommends`**, not `core:domain` — mapping to `domain` would *invent* an
   enforcement schema.org never asserts and reject valid data. Emit on the direct
   domain class only; subclasses inherit `recommends`, mirroring schema.org's
   apply-to-subtypes. **`core:domain` has no schema.org analog and is unused by the
   import.**

   This is the concrete instance of the general adoption rule (a corollary of #4):
   `core` pairs an *enforcing* construct with an *advisory* one — `requires` ↔
   `recommends`, `domain` ↔ (advisory has no single name, but `recommends` plays it),
   `allows_only` ↔ (unconstrained). **Adopt an advisory source vocabulary onto the
   advisory members; reserve the enforcing members for where the source itself is
   closed** (the enumeration `allows_only`, #6, is the one such place).
10. **No on-chain range cache.** [steer] A bespoke `range_literals`/`range_includes`
    annotation was added then **removed as redundant** — `core:source_irl` is the
    canonical record of the original range, `class_types` carries the class members,
    and `core:format` carries the validated literal refinement. Nothing duplicated.
11. **Mixed literal-or-entity union → entity-first.** [steer] `class_types=[the
    classes]`, `data_type=resource`; the literal option is dropped from the active
    type (recoverable via `source_irl`). Typed-entity binding is the platform's
    value; a bare string is the degenerate case schema.org tolerates.
12. **Tier-3 relational/inference vocabulary is not mapped.** [analysis]
    `subPropertyOf`, `inverseOf`, `supersededBy`, `equivalentClass|Property`, the
    Role pattern → recorded as inert provenance in the coverage report, never as
    active relations (Eigenius adopts a vocabulary, not a reasoner).
13. **Adopted grade + identity.** [analysis] Every emitted resource is
    `is_a [..., reflection:DeclaredResource]` with `declared_by = urn:schema_org` +
    `core:source_irl`. Round-trip identity = total prefix substitution
    `https://schema.org/<T>` ↔ `urn:schema_org:<T>`. `urn:schema_org:` is a sibling
    vendored ontology above core.

## D. Input selection

14. **`schemaorg-current-https.jsonld`, pinned V30.0, content-hashed.** [steer]
    `current` (excludes attic) over `all`; `https` (the identity rule) over `http`;
    **JSON-LD** over CSV/Turtle/NT/RDF-XML because it carries the full graph
    (`subClassOf`, `domain/rangeIncludes`, DataType/Enumeration membership) in one
    file, parseable without an RDF library. Pin a fixed release + content-hash
    (`sha256:0f0c97a4…`) for reproducibility — not "latest".

## E. Conversion-program implementation

15. **Parse to `serde_json::Value` + accessors, not rigid serde structs.** [analysis]
    schema.org's JSON-LD is irregular: a field may be `{@id}`, a list of them, or a
    bare string; `@type` is a string or a list. Helpers normalise these shapes.
16. **DataType set seeds from every dual-typed datatype, not just `DataType`'s
    subclasses.** [bug] First load dangled on `subClassOf Number/Text/Quantity`. Most
    datatypes carry `@type [rdfs:Class, schema:DataType]` but **no `subClassOf
    DataType` edge**, so their subtypes (`Integer⊂Number`, `URL⊂Text`,
    `Distance⊂Quantity`) are only reachable by seeding descendants from each dual
    node. (Consequence: the `Quantity` family folds to `string` — a structured
    quantity collapses to a scalar; acceptable for v1, recorded.)
17. **Filter every outgoing reference against the kept set.** [bug] First load
    dangled on non-excluded terms referencing **pending** terms (and folded
    DataTypes used as parents). Compute the set of emitted ids; drop any
    `subclass_of`/`class_types`/`recommends`/`allows_only` reference to a
    non-kept target. This is the correct open-world behaviour and is *why the full
    output loads cleanly*.
18. **Enumeration members are identified by `@type ∈ enum_set`** (the member's type
    is the enum class), not a separate marker; skipped if their enum class is out of
    scope. `allows_only` is emitted only when *every* entity range is an enumeration
    (a mixed enum+class range can't close), and its value is the **transitive member
    closure** over the enumeration's subclass subtree — schema.org's enumerations are
    a subclass hierarchy (members of `QualitativeValue` / `NonprofitType` live under
    their subtypes), so direct-only collection under-populated the closed set
    (Finding F1). When the closure is empty (a genuinely member-less enumeration) the
    range stays *open* — `class_types` only — and is counted in
    `coverage.enumeration_open` (Finding F2).
19. **Deterministic by construction.** [analysis] `BTreeMap`/`BTreeSet` throughout →
    byte-identical output across runs (verified); a unit test guards it.
20. **The coverage report is the m4 cut accounting.** [analysis] The generator emits
    per-tier property counts, folded DataTypes, excluded-by-layer counts, and the
    Tier-3 residual — so "what can/can't be mapped, and why" falls out of the run
    rather than being a separate exercise.

## Validation outcome (V30.0)

2114 resources (683 classes, 51 enumeration classes, 250 members, 1130 properties)
**load + validate in the kernel** (0 errors); property tiers Clean 867 /
by-convention 188 / enumeration 66 / defaulted 9; 15 DataTypes folded; 848 excluded;
Tier-3 residual (equivalentClass 48, equivalentProperty 89, subPropertyOf 142,
inverseOf 45, supersededBy 82) recorded but not mapped. The 66 Enumeration-tier
properties split into **62 closed** (carry `allows_only`) **+ 4 genuinely-open**
(member-less enums, Finding F2).

## Mechanical evidence & fail-closed findings (Level 1)

The claims above are no longer asserted in prose — they are witnessed by
`cargo test -p eigenius-schemaorg -- --ignored`
(`crates/eigenius-schemaorg/tests/output_validates.rs`), which converts the
content-pinned V30.0 input (sha256 `0f0c97a4…`) and checks: the output loads + the
kernel `Validator` reports 0 errors (Expressible); no resource carries `core:domain`;
every closable enumeration carries `allows_only`; `source_irl` round-trips to the
`@id`; an independent recount reproduces the coverage partition; conversion is
deterministic. The objective chain itself is kernel-type-checked by
`tests/d57_chain_validates.rs`. See
[d57-mechanical-evidence-plan.md](d57-mechanical-evidence-plan.md).

Two findings surfaced **because** we moved from asserting to witnessing — fail
closed, recorded, not routed around:

- **F1 — `allows_only` was direct-members-only (generator bug, fixed).** The enum
  check first failed 43 ≠ 66: 23 Enumeration-tier properties ranged on a *parent*
  enumeration whose members live in subclass enums (e.g. `QualitativeValue`,
  `NonprofitType`, `EnergyEfficiencyEnumeration`), so direct-only collection left
  their closed set empty and the range silently leaked open. Fixed by computing the
  **transitive member closure** (decision #18); regression-tested in
  `convert.rs::enumeration_closure_is_transitive_and_open_enums_accounted`.
- **F2 — 4 enumerations are genuinely member-less (modeling reality, accounted).**
  `BusinessFunction`, `BusinessEntityType`, `BedType`, `WarrantyScope` have no members
  anywhere in the vocabulary (members defined in external vocabularies, e.g.
  GoodRelations). A closed set cannot be formed, so the range stays open
  (`class_types` only — `allows_only=[]` would wrongly forbid all values). These are
  counted in `coverage.enumeration_open` (=4) with examples, so the gap is explicit,
  not silent.
