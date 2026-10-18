# schema.org ontology (`urn:schema_org:`)

`schema-org.eigon.json` is the [schema.org](https://schema.org/) vocabulary mapped into
Eigon-JSON under `urn:schema_org:` — 2114 resources (683 classes, 51 enumeration
classes, 250 enumeration members, 1130 properties). Unlike the hand-authored ontologies
in this tree, it is **generated** (deterministically) from schema.org's published
JSON-LD by the D57 meta-ontology correspondence, then committed here as a first-class,
loadable ontology.

- **Provenance.** Generated from schema.org **V30.0** (sha256 `0f0c97a4…`) by
  `crates/eigenius-schemaorg` (`--bin schemaorg-import`). Byte-identical across runs for
  a given input; sha256 `f4de231a3e32247509b000801e88a026a874bf3bf5a872a758f2227c5598c3fb`.
- **The mapping discipline** — what maps cleanly, by convention, and what is out of
  scope (the cut) — is documented in
  [`docs/design/d57-schema-org-vocabulary-mapping.md`](../../docs/design/d57-schema-org-vocabulary-mapping.md)
  and [`docs/notes/d57-mapping-decisions.md`](../../docs/notes/d57-mapping-decisions.md).
- **Regenerate / verify** (the input is fetched, not vendored — see
  `crates/eigenius-schemaorg/data/MANIFEST.md`):

  ```bash
  cargo run -p eigenius-schemaorg --bin schemaorg-import -- \
    --input  crates/eigenius-schemaorg/data/schemaorg-current-https-v30.0.jsonld \
    --output ontologies/schema-org/schema-org.eigon.json \
    --report crates/eigenius-schemaorg/data/coverage.json
  cargo test -p eigenius-schemaorg -- --ignored   # loads + validates the output (0 errors)
  ```

The D57 objective's Level-2 lift runs this generator *through the kernel* (the D60 `oci`
tool runtime) and pins this artifact as `obj:gen_output` — see
[`experiments/objectives/d57-schema-org/`](../../experiments/objectives/d57-schema-org/).
