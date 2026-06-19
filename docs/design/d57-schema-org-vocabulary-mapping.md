# D57 — schema.org Vocabulary Mapping

*Status: **mapping discipline settled** (June 2026) · generator + full mapping in progress on objective branch `obj-d57` · design memo*

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

## 3. Mapping discipline (settled)

The cut between what maps and what does not. Four constructs; the friction is
entirely in property *ranges*. The translator (§3.6) encodes these rules.

### 3.1 The four constructs

| schema.org | Eigenius target | Tier |
|---|---|---|
| **Class** (`rdfs:Class` + `rdfs:subClassOf`) | `core:Class` + `is_a` chain (`Thing → CreativeWork → Dataset`) | clean |
| **DataType** (Text, Number→Integer/Float, Boolean, Date, DateTime, Time, URL⊂Text) | core scalars (§3.2) | clean |
| **Enumeration** (Class ⊂ Enumeration + fixed member individuals) | `core:Class` + each member a `reflection:DeclaredResource` instance (a code list) | clean |
| **Property** (`rdf:Property` + multi-valued `domainIncludes` / `rangeIncludes`) | `core:Property` (§3.3) | the crux |

### 3.2 DataType alignment

`Text` → `string`; `URL` → `string` (format `iri`); `Integer` → `integer`;
`Number`/`Float` → `float`; `Boolean` → `boolean`; `Date`/`DateTime`/`Time` →
`string` (ISO-8601, with a format hint). The DataType subclass hierarchy is
carried as `is_a`/annotation but is informational — Eigenius does not infer over
it.

### 3.3 Property ranges — three tiers

`rangeIncludes` is multi-valued in schema.org; Eigenius `core:Property` has a
*single* `data_type` (a scalar) **or** `data_type = resource` + a `class_types`
set. The mapping by range shape:

- **Tier 1 — clean (1:1):** range is exactly one DataType → that scalar; range is
  exactly one Class → `resource` + `class_types = [that class]`.
- **Tier 2 — by documented convention:**
  - *all Classes* (e.g. `{Person, Organization}`) → `resource` +
    `class_types = [all]`. Lossless (`class_types` is already a set).
  - *all DataTypes* (e.g. `{Number, Text}`) → `string` (the broadest literal).
  - *mixed literal-or-entity* (e.g. `author = {Person, Organization, Text}`,
    `license = {CreativeWork, URL}`) → **entity-first**: `resource` +
    `class_types = [the Classes]`; the literal option is dropped from the active
    type. *(Decision 2026-06-19: entity-first over literal-first — typed-entity
    binding is the platform's value; a bare-string value is the degenerate case
    schema.org itself tolerates. The opposite choice, all-`string`, was rejected.)*
- In **every** Tier-2 case the generator preserves the **full original
  `rangeIncludes` verbatim** as a `schema_org:range_includes` provenance
  annotation, so nothing is lost and the JSON-LD round-trip is exact even when the
  active `data_type` picks one interpretation.

### 3.4 Tier 3 — not mapped, recorded with reason (the residual)

Eigenius adopts a *vocabulary*, not a reasoner (§4), so schema.org's
inference/relational semantics are **not** imported as active relations — only as
inert provenance annotations, enumerated in the cut accounting:

- `supersededBy`, `rdfs:subPropertyOf`, `owl:equivalentClass`, `inverseOf` — no
  Eigenius inference consumes these.
- the **Role** superimposition pattern — no analog.

### 3.5 Scope, layer, identity

- **Scope** *(decision 2026-06-19)*: **core + hosted extensions**
  (health-lifesci, bib, auto, …; ~800 Classes / ~1.4k Properties). The unstable
  `pending/` staging layer and deprecated `attic/` layer are **excluded** —
  stable IRIs for round-trip. (Re-runnable, so expanding later is cheap.)
- **Identity / round-trip**: fixed prefix substitution
  `https://schema.org/<Term>` ↔ `urn:schema_org:<Term>`; the original https IRI is
  retained on each resource as `core:source_irl`. (No per-term `sameAs` needed —
  the substitution is total and reversible.)
- **Layer placement**: `urn:schema_org:` is a **sibling vendored ontology stacked
  above core** (like `ingest`/`reference`/`obo`), not a root layer — it depends on
  core's scalar types and `reflection:DeclaredResource`.

### 3.6 Remaining open (the generator)

- **The translator.** A deterministic schema.org-JSON-LD → Eigon-JSON generator
  implementing §3.1–3.5 (in the spirit of the obograph importer / mirror
  generator). Input: schema.org's published `schemaorg-current-https.jsonld`.
  Output: the `urn:schema_org:` ontology + a **coverage report** (mapped clean /
  mapped-by-convention / Tier-3 residual counts + the per-term residual list).
  Home: `crates/eigenius-schemaorg/` (proposed), `--bin schemaorg_import`.
- **Adopted grade.** Every emitted resource is
  `is_a [..., reflection:DeclaredResource]` with `reflection:declared_by =
  "urn:schema_org"` + `core:source_irl` — adopted, never re-minted as native.

## 4. Out of scope

- RO-Crate import/export itself — a **tooling** concern outside Eigenius proper (D53 §9); this memo only ensures the *vocabulary* it needs exists as typed resources.
- schema.org's RDFS/OWL inference semantics — Eigenius adopts the term *vocabulary*, not a reasoner over it.
- Eigenius domain ontologies — unaffected; `urn:schema_org:` is additive.
