# eigenius-schemaorg

schema.org JSON-LD → Eigon-JSON generator (D57 m3). Translates schema.org's
published vocabulary into typed Eigenius resources under `urn:schema_org:`,
implementing the **meta-ontology correspondence** in
[`docs/notes/d57-schemaorg-vs-core-metamodel.md`](../../docs/notes/d57-schemaorg-vs-core-metamodel.md)
(spec: [D57](../../docs/design/d57-schema-org-vocabulary-mapping.md)).

- **Input (pinned):** `schemaorg-current-https.jsonld` V30.0 — see
  [`data/MANIFEST.md`](data/MANIFEST.md) (URL + sha256). `current` excludes attic;
  `pending`/`meta` layers filtered; hosted extensions kept.
- **Output:** the `urn:schema_org:` ontology (Eigon-JSON) + a coverage report
  (`data/coverage.json`, committed — the D57 m4 cut accounting).
- **Mapping:** Class→`core:Class`+`subclass_of`; DataType→scalar+`core:format`
  (URL/Date…); Enumeration→class + member `DeclaredResource`s + `allows_only`;
  Property→`core:Property` with `domain` and `rangeIncludes` per §3.3. Tier-3
  relations (`subPropertyOf`/`inverseOf`/`supersededBy`/`equivalentClass`) are
  recorded in coverage, not mapped (no reasoner).

```bash
cargo run -p eigenius-schemaorg --bin schemaorg-import -- \
    --input data/schemaorg-current-https-v30.0.jsonld \
    --output ../../ontologies/schema-org/schema-org.eigon.json --report data/coverage.json
```

Deterministic (byte-identical output per input). The full generated ontology
(2114 resources) loads + validates into the kernel; it is committed as a
first-class ontology at [`ontologies/schema-org/`](../../ontologies/schema-org/).
