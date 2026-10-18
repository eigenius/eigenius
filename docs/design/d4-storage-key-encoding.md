# D4: Storage Key Encoding

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 3; RocksDB backend)
**Required before:** Phase 3 implementation
**Resolves:** Key encoding scheme for RocksDB/TiKV, column families, layer chain persistence, index layout

---

## 1. Overview

Eigenius uses an ordered key-value store for persistent storage. The primary backend is RocksDB (embedded, single-node). The same key encoding translates directly to TiKV when multi-node scalability is needed, since TiKV uses RocksDB as its local storage engine.

### 1.1 Storage progression

| Backend | Use case | Characteristics |
|---------|----------|----------------|
| In-memory | Testing, development | BTreeMap-based, no persistence |
| RocksDB | Production single-node | Embedded, persistent, ordered, concurrent |
| TiKV | Production multi-node | Distributed, replicated, same key encoding as RocksDB |

### 1.2 Design principles

**Ordered keys enable prefix scans.** All resources in a layer share a key prefix, so listing a layer's contents is a single range scan.

**Layer-prefixed keys ensure isolation.** Each layer's data is keyed under its `LayerId`, preventing cross-layer interference.

**Canonical encoding.** Resource values are stored in RFC 8785 canonical JSON form, ensuring content-addressed hashing is consistent between storage and computation.

---

## 2. Key Scheme

All keys are UTF-8 strings. Values are bytes (typically JSON or MessagePack).

### 2.1 Layer metadata

```
layer:<layer_id_hex>:meta
```

Value: JSON object with layer metadata.

```json
{
  "name": "animals",
  "parent_id": "a1b2c3...",
  "created_at": "2026-04-12T14:30:00Z"
}
```

For the root layer (no parent), `parent_id` is `null`.

### 2.2 Resources

```
layer:<layer_id_hex>:res:<iri>
```

Value: Eigon-JSON canonical form of the resource.

Example key: `layer:ee0b8aa99441f1ac:res:urn:eigenius:example:rex`

### 2.3 Layer chain

```
chain:<layer_id_hex>
```

Value: Parent layer ID (hex string), or empty for the root layer.

This enables reconstructing the parent-pointer chain on startup without loading all layer metadata.

### 2.4 Head pointer

```
head
```

Value: The `LayerId` (hex) of the current head layer.

Updated atomically on commit. On restart, the service reads this key to find the top of the chain.

### 2.5 Triple indexes (future)

For query optimization, SPO/POS/OPS triple indexes can be stored:

```
layer:<layer_id_hex>:idx:spo:<subject_iri>:<property_iri>:<object_hash>
layer:<layer_id_hex>:idx:pos:<property_iri>:<object_hash>:<subject_iri>
layer:<layer_id_hex>:idx:ops:<object_hash>:<property_iri>:<subject_iri>
```

Values: empty (existence-only) or the object value for non-IRI objects.

These indexes are not required for Phase 3 — the evaluator scans resources directly. They become important when query performance on large datasets requires indexed lookups.

---

## 3. Column Families

RocksDB supports column families for logical separation with independent compaction and caching.

| Column Family | Contents | Access pattern |
|---------------|----------|---------------|
| `default` | Head pointer, configuration | Point reads |
| `layers` | Layer metadata | Point reads by layer ID |
| `resources` | Resource data | Prefix scans by layer ID, point reads by (layer, IRI) |
| `chain` | Parent pointers | Point reads for chain reconstruction |

Column families are optional — all keys can coexist in the default column family with prefix-based separation. Column families provide better isolation and tuning but add operational complexity.

**Recommendation for Phase 3:** Use a single column family (default) with key prefixes. Add column families as an optimization if performance profiling shows benefit.

---

## 4. Operations

### 4.1 Store a committed layer

1. Write layer metadata: `layer:<id>:meta → {name, parent_id, created_at}`
2. For each resource: `layer:<id>:res:<iri> → canonical_json`
3. Write chain pointer: `chain:<id> → parent_id`
4. Update head: `head → id`

Steps 1-3 should be in a single write batch (atomic).

### 4.2 Load a layer

1. Read `layer:<id>:meta` for metadata
2. Prefix scan `layer:<id>:res:` for all resources
3. Construct the `Layer` with resources in a `BTreeMap`

### 4.3 Reconstruct the layer chain on startup

1. Read `head` to get the current head layer ID
2. Read `chain:<head_id>` to get the parent ID
3. Repeat until parent is empty (root layer)
4. Load each layer in order (root first, then children)
5. Reconstruct the `Arc<Layer>` chain with parent pointers

### 4.4 List all layers

Prefix scan on `layer:` keys, extracting layer IDs.

---

## 5. Serialization

The system uses two serialization formats at different layers:

| Layer | Format | Rationale |
|-------|--------|-----------|
| Authoring / files | Eigon-JSON | Human-readable, editable |
| Storage (RocksDB values) | CBOR | Compact, fast parsing, deterministic encoding |
| gRPC wire | CBOR | Compact, matches storage format |
| Content-addressed hashing | CBOR deterministic encoding | Replaces RFC 8785; simpler and unambiguous |
| CLI output / debugging | Eigon-JSON | Human-readable |

The `Resource` type is the internal representation. Serialization format is a boundary concern:
- **Ingest:** Eigon-JSON → parse → Resource → CBOR → store
- **Retrieve:** CBOR → parse → Resource → Eigon-JSON (if human output)
- **Hash:** Resource → CBOR deterministic encoding → SHA-256

### 5.1 CBOR encoding (storage and wire)

Resources and layer metadata are stored as CBOR (RFC 8949). Benefits over JSON:

- **Compact** — no string quoting, binary numbers, length-prefixed strings
- **Fast** — no UTF-8 escape parsing, direct binary representation
- **Deterministic** — Core Deterministic Encoding (RFC 8949 §4.2) provides canonical form:
  - Map keys sorted by encoded byte string
  - Shortest encoding for each value
  - No indefinite-length containers in stored form
- **Round-trip safe** — no floating-point precision issues from JSON number representation

CBOR type mapping:

| Eigon type | CBOR major type |
|-----------|----------------|
| String | Major 3 (text string) |
| Integer | Major 0/1 (unsigned/negative integer) |
| Float | Major 7 (IEEE 754 double) |
| Boolean | Major 7 (simple values true/false) |
| Resource (embedded) | Major 5 (map) |
| Array | Major 4 (array) |
| IRI (as key) | Major 3 (text string) |
| `@id` | Key string `"@id"` in the map |

A resource is encoded as a CBOR map where:
- Keys are property IRI strings (text, major 3)
- Values are typed according to the property's data type
- `@id` is included for top-level resources
- Maps are sorted by key in deterministic encoding

Rust library: `ciborium` (serde-based, supports deterministic encoding).
TypeScript library: `cbor-x` (fast, streaming support).

### 5.2 Eigon-JSON (files and human output)

Eigon-JSON remains the authoring and debugging format as specified in D1. Files on disk (ontology definitions, example data, program specifications) use Eigon-JSON. The CLI outputs Eigon-JSON by default.

RFC 8785 (JSON Canonicalization Scheme) is retained for backward compatibility with existing content-addressed hashes. New layers may use CBOR deterministic encoding for hashing; existing layers retain their original hash.

### 5.3 Layer metadata

Layer metadata is stored as CBOR:

```
{
  "name": "animals",
  "parent_id": "ee0b8aa9...",
  "created_at": "2026-04-12T14:30:00Z"
}
```

(Shown as JSON for readability; stored as CBOR in the database.)

### 5.4 Key encoding

Layer IDs are encoded as lowercase hex strings (64 characters for SHA-256). IRIs are stored as-is (UTF-8). The `:` separator is unambiguous because:
- Layer IDs are hex (no colons)
- The fixed prefix `layer:`, `chain:`, `head` provides namespace separation
- IRIs contain colons but always appear after the `res:` prefix

---

## 6. TiKV Compatibility

The key scheme is directly compatible with TiKV:
- Keys are UTF-8 byte strings — TiKV uses byte-ordered keys
- Prefix scans map to TiKV range queries
- Write batches map to TiKV transactions
- Column families map to TiKV key prefixes (TiKV does not use RocksDB column families directly)

Migration from RocksDB to TiKV: export all key-value pairs, import into TiKV. The key encoding is identical.

---

## 7. Decisions Log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Primary backend | RocksDB (Phase 3), TiKV (future) | Same key encoding; RocksDB is simpler for single-node |
| Key format | UTF-8 strings with `:` separators | Human-readable, debuggable, ordered |
| Column families | Single default CF for Phase 3 | Simpler; add CFs as optimization later |
| Storage value format | CBOR (RFC 8949) | Compact, fast parsing, deterministic encoding built-in |
| Authoring format | Eigon-JSON (unchanged) | Human-readable; CBOR is a storage/wire optimization |
| Content-addressed hashing | CBOR deterministic encoding | Replaces RFC 8785; simpler, no floating-point edge cases |
| Layer chain storage | Separate `chain:` keys + `head` pointer | Efficient startup reconstruction without loading all metadata |
| Triple indexes | Deferred | Not needed until query performance requires indexed lookups |
