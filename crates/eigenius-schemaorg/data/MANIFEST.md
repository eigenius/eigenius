# schema.org generator input (pinned)

| field | value |
|---|---|
| file | `schemaorg-current-https.jsonld` (renamed `schemaorg-current-https-v30.0.jsonld`) |
| version | **V30.0** (2026-03-19) |
| origin | https://schema.org/version/30.0/schemaorg-current-https.jsonld |
| sha256 | `0f0c97a4f666b2f8563573fe48453782fd51b87a504523cf0c9aff6a71c3eec4` |
| bytes | 1551177 |

The input JSON-LD and the generated `schema-org.eigon.json` are gitignored
(large, reproducible). `coverage.json` (the m4 cut accounting) is committed.

## Fetch + generate
```bash
curl -fsSL https://schema.org/version/30.0/schemaorg-current-https.jsonld \
  -o crates/eigenius-schemaorg/data/schemaorg-current-https-v30.0.jsonld
# verify sha256 matches the table above, then:
cargo run -p eigenius-schemaorg --bin schemaorg-import -- \
  --input  crates/eigenius-schemaorg/data/schemaorg-current-https-v30.0.jsonld \
  --output crates/eigenius-schemaorg/data/schema-org.eigon.json \
  --report crates/eigenius-schemaorg/data/coverage.json
```
Deterministic: byte-identical output across runs for a given input.
