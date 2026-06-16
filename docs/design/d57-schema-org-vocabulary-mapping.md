# D57 — schema.org Vocabulary Mapping

*Status: **stub** · design memo · June 2026*

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

## 3. Open questions

- **Type/range mapping.** schema.org property ranges (`Text`, `URL`, `Number`, `Integer`, `Boolean`, `Date`/`DateTime`, or a class) → Eigenius `data_type` (`string`, `float`, `integer`, `boolean`, `resource`+`class_types`). Conventions needed for: `Date`/`DateTime` (→ `string`, ISO-8601), `URL` (→ `string`), and especially schema.org's **multi-range (union) properties** — a single property whose value may be `Text` *or* a `Class` — which Eigenius's single-`data_type` model doesn't express directly (candidates: pick the broadest, fall back to `string`, or `json`). This is the bulk of the real design work.
- **Generation, not hand-authoring.** schema.org publishes its full vocabulary as downloadable JSON-LD/RDF. The `urn:schema_org:` ontology should be **generated** from that (a deterministic translator, in the spirit of the mirror generator), not transcribed — so it stays in sync and the "mostly as-is" mapping is mechanized. The translator encodes the §3 type conventions.
- **Scope / subset.** schema.org has ~800 types; Eigenius needs the dataset/file/provenance/agent slice first (`Dataset`, `DataDownload`/`MediaObject`, `File`, `PropertyValue`, `DefinedTerm`, `Person`, `Organization`, `SoftwareSourceCode`/`SoftwareApplication`, `CreativeWork`, `Thing`, plus the properties they hang off). Generate-on-demand vs vendor-the-slice.
- **Identity reconciliation.** How `urn:schema_org:` IRIs relate to schema.org's own `https://schema.org/...` IRIs (for JSON-LD round-trip): a fixed prefix substitution, or a recorded `sameAs`. Needed by the RO-Crate boundary tool.
- **Layer placement.** Whether `urn:schema_org:` is a root layer (like the core ontology) or a sibling vendored ontology stacked above core.

## 4. Out of scope

- RO-Crate import/export itself — a **tooling** concern outside Eigenius proper (D53 §9); this memo only ensures the *vocabulary* it needs exists as typed resources.
- schema.org's RDFS/OWL inference semantics — Eigenius adopts the term *vocabulary*, not a reasoner over it.
- Eigenius domain ontologies — unaffected; `urn:schema_org:` is additive.
