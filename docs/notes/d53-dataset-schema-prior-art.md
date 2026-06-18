# D53 §4 dataset schema — prior-art survey

> Cited survey of existing data models for binding a dataset file's internal
> structure (columns/rows/dimensions) to typed graph entities, to inform the
> design of [D53](../design/d53-large-data-tracking.md) §4 ("Dataset
> schema — binding file axes to the graph"). Produced via the `deep-research`
> harness (fan-out web search → fetch → 3-vote adversarial verification →
> synthesis), June 2026. Confidence/votes are the verification outcome per claim.
>
> **Bottom line:** two complementary families. The **W3C RDF Data Cube (QB)**
> family supplies the *conceptual model* (dimension/measure/attribute split, a
> reusable schema separated from instances, IRI-bound coded dimensions);
> **MLCommons Croissant** supplies the *file-binding mechanics* (a logical schema
> superimposed over unmodified files, foreign-key references, IRI-typed fields).
> A life-science member of the QB family — **Allotrope ADF-DCO** — already does
> the exact thing D53 §4 needs: an abstract data cube mapped to a physical binary
> file layout.

## 1. W3C RDF Data Cube (QB) family — the conceptual model

- **Dimension / measure / attribute decomposition** *(high, 3-0)*. A `qb:DataStructureDefinition` "defines the dimensions, attributes and measures used in the dataset." Dimensions "identify the observations… a set of values for all the dimension components is sufficient to identify a single observation"; measures "represent the phenomenon being observed"; attributes "qualify and interpret… units of measure… status." Roles are `qb:DimensionProperty` / `qb:MeasureProperty` / `qb:AttributeProperty`, enumerated in the DSD via `qb:component → qb:ComponentSpecification`. — `w3.org/TR/vocab-data-cube`
- **DSD = a reusable abstract schema, separated from instances** *(high, 3-0)*. `qb:structure` (DataSet → DSD): "define that structure once and then reuse it for each publication." A clean semantic-schema-vs-instance split — though QB describes *RDF observations*, not external file layouts. — `w3.org/TR/vocab-data-cube`
- **Axes bind to typed concept IRIs, not free-text labels** *(high, 3-0)*. `qb:concept` links a component property to a `skos:Concept`; `qb:codeList` ties a coded property to a `skos:ConceptScheme` / `skos:Collection` / `qb:HierarchicalCodeList`. So a dimension binds to an ontology/code-list IRI. — `w3.org/TR/vocab-data-cube`, `w3.org/TR/2017/NOTE-qb4st`
- **Limitation — attributes can't attach to a specific component** *(high, 2-1)*. Base QB attaches an attribute (e.g. unit) "to a complete Observation, or a higher level aggregation (DataSet or Slice), but not unambiguously to a specific component." *Design lesson: allow per-component (per-axis) units/attributes from the start.* — `w3.org/TR/2017/NOTE-qb4st`
- **Limitation — no OLAP hierarchies** *(high, 3-0)*. Base QB "lacks dimension hierarchies, hierarchy levels, level-to-level relationships, and aggregate functions." — `arxiv.org/abs/1512.06080`, `github.com/lorenae/qb4olap/wiki`
- **QB4OLAP — hierarchies + a schema added over existing instances** *(high, 3-0)*. Adds level hierarchies + rollups + per-measure aggregate functions, enabling rollup/slice/dice via plain SPARQL, and — notably — "QB4OLAP cube schemas can be built on top of data cube instances already published using QB… the cost is the cost of building the new schema." *A schema layer can be added over data without rewriting it.* — `arxiv.org/abs/1512.06080`
- **★ Allotrope ADF-DCO — QB for scientific matrices, mapped to physical storage** *(high, 3-0)*. A life-science ontology that "imports and thus extends QB," keeping `qb:DataSet` / `qb:DataStructureDefinition` / `qb:ComponentSpecification` central, with explicit `adf-dc:Dimension` (independent variables, identify observations) and `adf-dc:Measure` (dependent variables, store values). Crucially, a companion **mapping ontology** "maps the abstract data cubes… to their concrete HDF5 representations," "defin[ing] a mapping between the business perspective and the physical representation of n-dimensional data in HDF5." *This is the clearest existing precedent for binding an abstract cube to a physical file layout — exactly D53 §4's problem, for a CRISPR cell-line × gene matrix.* — `docs.allotrope.org/ADF Data Cube Ontology.html`

## 2. MLCommons Croissant — the file-binding mechanics

- **Logical schema superimposed over unmodified files** *(high, 3-0)*. Four layers — Dataset Metadata / Resource (`FileObject`/`FileSet`) / Structure (`RecordSet`/`Field`) / Semantic. A `RecordSet` is "a view on top of one or more FileObjects/FileSets"; each `Field` declares its physical `source` (a fileObject + **column / jsonPath / fileProperty**, optional regex/JSON transforms). Croissant "does not require changing the existing layout of data… describes data as it already exists," and "handling all data formatting in one layer abstracts away format heterogeneity." *The cleanest logical-vs-physical separation surveyed.* — `docs.mlcommons.org/croissant`, `arxiv.org/html/2403.19546v2`
- **Foreign keys via `references`** *(high, 3-0)*. "The equivalent of a foreign key reference in a relational database… values in the referencing Field are taken from the values of the target Field" of another RecordSet (or a fileObject + key column). *The transferable mechanism for row-key/column → an external entity table (a gene-column referencing a Gene RecordSet/class).* — `docs.mlcommons.org/croissant`
- **`dataType` binds Fields to vocabulary IRIs** *(high, 3-0)*. A Field's `dataType` points at external IRIs (schema.org `sc:Text` / `sc:ImageObject`, or Wikidata `wd:Q515`); multiple dataTypes allowed (≥1 atomic, others add semantics). Descriptions are "based on schema.org/Dataset." *Columns bind to ontology IRIs — ties directly to [D57](../design/d57-schema-org-vocabulary-mapping.md).* — `docs.mlcommons.org/croissant`
- **Weakness to avoid — implicit name-suffix subfield mapping** *(medium, 2-1)*. Subfields are mapped to a class's properties "because their names match by suffix" (e.g. `coordinates/latitude` → `latitude`); an explicit `equivalentProperty` escape hatch exists. *Design lesson: bind cells/subfields to property IRIs **explicitly**, never by name matching.* — NeurIPS 2024 Croissant paper

## 3. CSVW / Frictionless — the CSV mechanics

- **CSVW has a native CSV→RDF transform** *(high, 3-0)*. CSVW provides column descriptors + datatypes + primary/foreign keys, and `csv2rdf` is a W3C Recommendation ("Generating RDF from Tabular Data on the Web"). Frictionless Table Schema has **no** native RDF transform (only converters to DataCite/DCAT) — though its `rdfType` is a lightweight field-level annotation, and its field + key model is a simple column-descriptor baseline. — `w3.org/TR/2015/REC-csv2rdf`, `datapackage.org/guides/csvw-data-package`

## 4. Lineage note (FuGE → modern)

FuGE's "data cube" (functional-genomics object model, ~2007) is the genomics ancestor of the dimension/measure idea; its descriptive-experiment role passed to the **ISA model** (Investigation/Study/Assay) and **OBI**, which today carry *experiment context* (sample/assay/platform) — relevant to D53 §4.2's opaque/`ReadSet` profile, less to the cube binding. The cube concept itself was superseded by the QB family above (verified findings centered there).

## 5. Recommendation for D53 §4

**Adopt a DSD-style reusable semantic schema + a per-file physical layout binding, Eigenius-IRI-bound throughout.**

Take:
- **From QB / ADF-DCO** — the **dimension / measure / attribute** decomposition; a reusable **`DatasetSchema`** (DSD analog) declared once and referenced by many files; **IRI-bound coded dimensions** (the `concept`/`codeList` idea → bind to Eigenius classes/code-lists); and ADF-DCO's **abstract-cube → physical-file mapping** as the model for separating semantics from layout.
- **From Croissant** — the **logical-schema-over-unmodified-file** superimposition; per-field **`source`** (axis/column/regex) as the physical layout binding (incl. the entity-per-column header rule as an *explicit* source mapping); and **`references`** as the row-key/foreign-key mechanism.
- **From CSVW** — `csv2rdf` as prior art for the tabular→typed mechanics.

Avoid:
- Croissant's **implicit name-suffix** subfield→property mapping — bind explicitly by IRI.
- Base-QB's inability to attach an **attribute to a specific component** — allow per-axis units/qualifiers from the start.

The decisive pattern, validated by ADF-DCO (scientific data) and Croissant (file mechanics): **separate the semantic cube (dimensions/measures, graph-bound, reusable) from the physical layout (which axis/column, per-file).**
