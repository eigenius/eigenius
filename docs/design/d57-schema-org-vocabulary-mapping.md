# D57 — schema.org Vocabulary Mapping

*Status: **complete** (June 2026) · the full schema.org V30.0 vocabulary maps to `urn:schema_org:` (~2114 resources, loads + validates) via `crates/eigenius-schemaorg`; mapping discipline (§3) + cut accounting (§3.7) settled · design memo. The objective is discharged on-chain (`experiments/objectives/d57-schema-org/`): m1–m4 + the thesis `SchemaOrgMapped` all Hold; well-posedness gates pass.*

*Scope correction (2026-06-19): the deliverable is the **whole** vocabulary — a generated mapping of every *mappable* schema.org term plus an explicit, justified cut of what cannot be mapped — not the §2.5 ten-property slice (that is now the proof-of-shape probe). The mapping discipline below (§3) is settled; the generator (§3.6) and the cut accounting are the remaining work, tracked as objective `obj-d57` (`experiments/objectives/d57-schema-org/`).*

*Companion documents: [D53 large-data tracking](d53-large-data-tracking.md) (the caller), [core ontology](../../ontologies/core/core-ontology.json), [D26 runtime substrate](d26-runtime-substrate.md).*

*This memo specifies how the [schema.org](https://schema.org) vocabulary is brought into Eigenius as a typed descriptive layer — schema.org classes and properties translated, mostly as-is, into Eigenius resources under their own `urn:schema_org:` namespace. It is the shared substrate for D53's file-level metadata (§9), D53's §4 dataset-schema vocabulary, and any future RO-Crate interchange tooling. **Stub:** the decisions below are settled; the type-mapping details and the generation mechanism are open.*

---

## 1. Motivation

Several places want a **standard descriptive vocabulary** rather than bespoke Eigenius terms:

- D53 §9 — `PinnedExternalFile` should carry `license`, `creator`, `contentSize`, `encodingFormat`, `identifier` (DOI/URL), etc., instead of stranding them in `MANIFEST.md` prose.
- D53 §4 — describing a file's structure (datasets, files, measured variables) overlaps schema.org's `Dataset` / `PropertyValue` / `variableMeasured`.
- RO-Crate tooling (D53 §9, a boundary converter) is *built on* schema.org/JSON-LD — sharing the same term IRIs makes the boundary trivial.

schema.org is the lingua franca for describing datasets, files, software, people, organizations, and licenses. Reinventing those terms inside `urn:eigenius:` would be wasteful and would not round-trip with the FAIR-data ecosystem. So: **adopt schema.org as a vocabulary, expressed in Eigenius's type system.**

## 2. Decisions (settled)

1. **Own namespace.** schema.org terms live under **`urn:schema_org:`** — a sibling top-level vocabulary, *not* under `urn:eigenius:`. It is an external, adopted vocabulary, and keeping it in its own namespace marks it as such and keeps the mapping mechanical (`schema.org/Dataset` → `urn:schema_org:Dataset`, `schema.org/license` → `urn:schema_org:license`).
2. **Mostly translate as-is.** A schema.org **Class** becomes an Eigenius `core:Class`; a schema.org **Property** becomes an Eigenius `core:Property`. The translation is structural and largely 1:1 — names, descriptions, and the `subClassOf` hierarchy carry over directly (schema.org `Thing → CreativeWork → Dataset` → an Eigenius `is_a` chain).
3. **Descriptive layer, not domain layer.** `urn:schema_org:` supplies *generic descriptive* metadata (who made it, what format, what license, how big). Eigenius **domain** types (`onco:Gene`, `stats:SampleSet`, …) stay separate and are the *binding* targets for D53 §4's typed axes. A `PinnedExternalFile` can be `is_a urn:schema_org:Dataset` (descriptive) *and* carry Eigenius-typed schema bindings (semantic) — the two layers coexist.
4. **Open-world by default.** schema.org classes have no required properties; their Eigenius translations therefore land everything under `recommends`, nothing under `requires` — schema.org's open-world stance preserved.

## 2.5 Minimum slice for D53 (the only part D53 needs now)

D53 needs schema.org for exactly **one** thing: the *recommended* file-level
descriptive metadata on `PinnedExternalFile` (D53 §10) — name, license, creator,
size, DOI, etc. — so it lives as typed fields rather than `MANIFEST.md` prose.
**Everything else D53 uses is Eigenius-native:** its *required* fields
(`ingest:reference`, `ingest:content_hash`, `ingest:media_type`) are deliberately
`ingest:` properties, not schema.org, to keep D53 off D57's critical path; and the
§4 cube binds to `onco:` / `ingest:` types, not schema.org. So D53 **functionally
needs zero of D57** — implementation-plan Phases 0–3 never touch it; this slice is
purely the optional descriptive enrichment.

**The slice: ~10 hand-authored properties, no classes, no machinery.** Under
`urn:schema_org:`, each a `core:Property` with `data_type = core:string` — the
union-range simplification (a DOI, a license URL, a creator name are all strings),
which **sidesteps §3's hard type-mapping entirely**:

| `urn:schema_org:` property | role on `PinnedExternalFile` | note |
|---|---|---|
| `name` | dataset/file name | |
| `description` | human description | |
| `contentSize` | byte size | string now; integer later |
| `encodingFormat` | media type | aligns with `ingest:media_type` (keep both, or alias later) |
| `license` | license URL / SPDX id | string |
| `creator` | author | string (defer the `Person` range) |
| `sourceOrganization` | producing org | string (defer the `Organization` range) |
| `identifier` | DOI / accession / URL | string (defer the `PropertyValue` range) |
| `datePublished` | ISO-8601 date | string |
| `isPartOf` | parent collection | string / URL (defer the `CreativeWork` range) |

**Deferred (all of D57's hard parts, none needed for D53):** no classes
(`Dataset`/`Person`/`Organization`/`CreativeWork`) — values are strings, not typed
entities; no generation-from-JSON-LD (ten hand-authored properties); no
union/range mapping (§3); no `subClassOf` hierarchy; no IRI reconciliation; no
RO-Crate. Those land when a *consumer* needs them (typed authorship, RO-Crate
export), not for D53.

**Deliberately *not* in the slice:** `content_hash` stays `ingest:content_hash`
(schema.org has no sha256, and it's the correctness root — Eigenius-owned);
`reference` / `media_type` stay `ingest:` (required fields, off D57's critical path).

> **Superseded by §3 / the metamodel note.** The "~10 string properties"
> simplification predates the construct-correspondence analysis. Under the settled
> discipline only `name`/`description`/`contentSize` are cleanly `string`;
> `encodingFormat`/`datePublished` are `string`-by-collapse (format-spanning unions, §3.2);
> and `creator`/`publisher`/`sourceOrganization`/`license`/`identifier`/`isPartOf`
> are **entity- or union-typed** (`class_types`, entity-first per §3.3) — *not*
> strings. The m2 probe confirmed this on a real file: descriptive string fields
> bind directly, and `creator` binds to a `Person` resource (entity-first). So the
> file-level descriptive metadata is a *mix* of scalar and entity-valued fields,
> as the meta-ontology dictates — not a flat string bag.

## 3. Mapping discipline (settled)

The cut between what maps and what does not, **derived construct-by-construct from
the two meta-ontologies** — see the analysis in
[docs/notes/d57-schemaorg-vs-core-metamodel.md](../notes/d57-schemaorg-vs-core-metamodel.md),
which is the principled basis for the rules below (the "tiers" are consequences of
the `rangeIncludes` correspondence, not independent heuristics). The friction is
entirely in property *ranges*. The translator (§3.6) implements the correspondence.

### 3.1 The constructs

| schema.org | Eigenius target | Tier |
|---|---|---|
| **Class** (`rdfs:Class` + `rdfs:subClassOf`) | `core:Class` + **`core:subclass_of`** chain (`Thing → CreativeWork → Dataset`) | clean |
| **DataType** (Text, Number→Integer/Float, Boolean, Date, DateTime, Time, URL⊂Text) | core scalars (§3.2) | clean |
| **Enumeration** (Class ⊂ Enumeration + fixed member individuals) | `core:Class` + each member a `reflection:DeclaredResource` instance; **a property ranging over it → `class_types=[E]` + `allows_only=[members]`**, which *enforces* the closed set at commit (the `core:DataType`/`reflection:EpistemicStatus` idiom; metamodel note §5). Open enumerations (also admitting `DefinedTerm`/`Text`) widen `class_types` and drop `allows_only`. | clean |
| **Property** (`rdf:Property` + `rangeIncludes`) | `core:Property` (§3.3) | the crux |
| **`domainIncludes`** (advisory) | each domain class's **`core:recommends`** (inverted; subclasses inherit) — *not* `core:domain` (which restricts; schema.org doesn't) | clean |

### 3.2 DataType alignment (via `core:Format`)

schema.org's literal DataType *subtypes* map onto `core` scalars refined by
**`core:format`** — they mirror core's `Format` vocabulary almost 1:1 (metamodel
note §5.1), and the kernel *validates* the refinement (a fidelity gain):

- `Text` → `string`; `Integer` → `integer`; `Number`/`Float` → `float`;
  `Boolean` → `boolean`.
- `URL` (⊂ `Text`) → `string` + `format = iri`.
- `Date` / `DateTime` / `Time` → `string` + `format = date`/`datetime`/`time`.

`core:format` applies only when the range is a **single** refined DataType and all
values conform. A union spanning distinct formats does **not** refine: `Date |
DateTime` → plain `string` (no single format covers both), and `Text | URL` is
degenerate (`URL ⊂ Text`) with possibly non-IRI values (MIME strings) → plain
`string`. The DataType subclass hierarchy is informational — Eigenius does not
infer over it.

### 3.3 Property ranges — three tiers

`rangeIncludes` is multi-valued in schema.org; Eigenius `core:Property` has a
*single* `data_type` (a scalar) **or** `data_type = resource` + a `class_types`
set. The mapping by range shape:

- **Tier 1 — clean (1:1):** range is exactly one DataType → that scalar; range is
  exactly one Class → `resource` + `class_types = [that class]`.
- **Tier 2 — by documented convention:**
  - *all Classes* (e.g. `{Person, Organization}`) → `resource` +
    `class_types = [all]`. Lossless (`class_types` is already a set).
  - *all DataTypes* (e.g. `{Number, Text}`) → `string` (the broadest literal; §3.2
    governs any `core:format`).
  - *mixed literal-or-entity* (e.g. `author = {Person, Organization, Text}`,
    `license = {CreativeWork, URL}`) → **entity-first**: `resource` +
    `class_types = [the Classes]`; the literal option is dropped from the active
    type. *(Decision 2026-06-19: entity-first over literal-first — typed-entity
    binding is the platform's value; a bare-string value is the degenerate case
    schema.org itself tolerates. The opposite choice, all-`string`, was rejected.)*
- **No on-chain range cache.** The original `rangeIncludes` is recorded canonically
  by `core:source_irl` (and the source JSON-LD the generator reads); it is **not**
  duplicated in a bespoke property. `class_types` carries the class members,
  `core:format` carries the validated literal refinement, and `source_irl` carries
  the full provenance — so the dropped literal of an entity-first union is
  recoverable without an extra annotation. *(Earlier drafts added a
  `schema_org:range_literals` string; removed as redundant with `source_irl` +
  `core:format`.)*

### 3.4 Tier 3 — not mapped, recorded with reason (the residual)

Eigenius adopts a *vocabulary*, not a reasoner (§4), so schema.org's
inference/relational semantics are **not** imported as active relations — only as
inert provenance annotations, enumerated in the cut accounting:

- `supersededBy`, `rdfs:subPropertyOf`, `owl:equivalentClass`, `inverseOf` — no
  Eigenius inference consumes these.
- the **Role** superimposition pattern — no analog.

### 3.5 Scope, layer, identity

- **Scope** *(decision 2026-06-19)*: **core + hosted extensions**
  (health-lifesci, bib, auto, …; ~800 Classes / ~1.4k Properties). Realized by
  taking the **`current`** distribution (which excludes the deprecated `attic/`
  retired terms — `all` would include them) and **filtering out `pending`** terms,
  which ship inside `current` marked `schema:isPartOf https://pending.schema.org`
  (marker to confirm against the V30.0 file in m3). Stable IRIs for round-trip;
  re-runnable, so expanding later is cheap.
- **Identity / round-trip**: fixed prefix substitution
  `https://schema.org/<Term>` ↔ `urn:schema_org:<Term>`; the original https IRI is
  retained on each resource as `core:source_irl`. (No per-term `sameAs` needed —
  the substitution is total and reversible.)
- **Layer placement**: `urn:schema_org:` is a **sibling vendored ontology stacked
  above core** (like `ingest`/`reference`/`obo`), not a root layer — it depends on
  core's scalar types and `reflection:DeclaredResource`.

### 3.6 The generator (implemented)

`crates/eigenius-schemaorg` (`--bin schemaorg-import`) — a deterministic
schema.org-JSON-LD → Eigon-JSON translator implementing §3.1–3.5 (modeled on the
obograph importer). **Input (pinned):** `schemaorg-current-https.jsonld`
**V30.0 (2026-03-19)**, content-hashed (`data/MANIFEST.md`), *not* "latest".
JSON-LD over the CSV/Turtle/NT/RDF-XML distributions: it carries the full graph
(`subClassOf`, `domainIncludes`/`rangeIncludes`, DataType/Enumeration membership)
in one file, parseable without an RDF library; `https` + `current` match §3.5.
Every emitted resource is `is_a [..., reflection:DeclaredResource]` with
`reflection:declared_by = "urn:schema_org"` + `core:source_irl` — adopted, never
re-minted as native. Deterministic (byte-identical per input). Unit-tested across
every tier; **the full output (~2114 resources) loads + validates in the kernel.**

### 3.7 Coverage — the cut, accounted (m4; from V30.0)

The generator emits a coverage report (`data/coverage.json`). For V30.0:

- **Mapped (2114 resources):** 683 classes, 51 enumeration classes, 250
  enumeration members, 1130 properties.
- **Property ranges by tier:** Clean 867, by-convention 188 (entity-first unions /
  format-spanning literal unions), enumeration 66 (`class_types` + `allows_only` over
  the transitive member closure — 62 closed + 4 genuinely-open member-less enums,
  `coverage.enumeration_open`), defaulted 9 (no in-scope range → `string`).
- **DataTypes folded → core scalars (15):** `Text`/`URL`/`Number`/`Integer`/
  `Float`/`Boolean`/`Date`/`DateTime`/`Time` + the `Quantity` family.
- **Excluded by layer (848):** `pending` + `meta` (attic already absent from
  `current`). Hosted extensions (health-lifesci, auto, bib) are kept.
- **Tier-3 residual — recorded, not mapped (the cut):** `equivalentClass` 48,
  `equivalentProperty` 89, `subPropertyOf` 142, `inverseOf` 45, `supersededBy` 82
  — inert provenance only (no reasoner; §4).

## 4. Out of scope

- RO-Crate import/export itself — a **tooling** concern outside Eigenius proper (D53 §9); this memo only ensures the *vocabulary* it needs exists as typed resources.
- schema.org's RDFS/OWL inference semantics — Eigenius adopts the term *vocabulary*, not a reasoner over it.
- Eigenius domain ontologies — unaffected; `urn:schema_org:` is additive.
