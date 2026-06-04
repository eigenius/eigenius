# 10. Result format

Query execution produces an **Eigon document** — a `Vec<Resource>` shaped per [D2 Appendix A](../../design/d2-eigenql-specification.md) so it's self-describing: the row schema, row class, and result set all come back together. The shaping code is [`kernel/src/query/document.rs`](../../../kernel/src/query/document.rs); it runs as the final stage of [`execute_with`](../../../kernel/src/query/mod.rs).

## 11.1. The two result shapes

Every query returns one of two shapes:

### With `RETURN` items — self-describing result set

For queries like:

```eigenql
USING "urn:eigenius:core:Class"

MATCH Class(?c) {
    short_name: ?name
}
RETURN [] {
    short_name: ?name
}
```

the result vector contains:

1. **One `Property` resource per `RETURN` item**. Each describes one column — its short name, inferred data type, and synthesized IRI.
2. **One `Class` resource** for the rows. It carries the `is_a` classes from `RETURN [Classes, ...]`, and lists which properties each row has.
3. **One `ResultSet` resource** with the row count and the embedded row values.

Row resources themselves are **embedded inside the ResultSet** — they have no stable `@id`. This matches D1's usage of `Value::Embedded` for resources that are conceptually part of their parent.

### Without `RETURN` content — guard-style

For queries like:

```eigenql
MATCH ?x {}
WHERE ?x = "urn:eigenius:test:alice"
RETURN [] {}
```

the result vector contains just **one `ResultSet`** resource with:

- `is_a: [urn:eigenius:query:ResultSet]`
- `matched: <boolean>` — true if any binding survived
- `row_count: <integer>` — number of surviving bindings

No Property, no row Class — nothing to describe. Useful for existence checks.

## 11.2. Synthesized IRIs

Every query's result resources live under `urn:eigenius:query:gen:<hash>:*`, where `<hash>` is the first 8 bytes (16 hex chars) of SHA-256 over the query text. The construction helper is [`QueryFingerprint`](../../../kernel/src/query/document.rs):

```rust
pub struct QueryFingerprint { hash: String }

impl QueryFingerprint {
    pub fn of(query_text: &str) -> Self
    fn base(&self) -> String  // "urn:eigenius:query:gen:<hash>"
    pub fn result_set_iri(&self) -> Iri    // <base>:result
    pub fn row_class_iri(&self) -> Iri     // <base>:row_class
    pub fn row_property_iri(&self, short_name: &str) -> Iri  // <base>:row:<short>
    pub fn fiber_response_iri(&self, clause_idx: usize, binding_idx: usize) -> Iri
}
```

Running the same query text against the same layer produces identical IRIs. Consumers that cache results can key on the query hash directly.

## 11.3. Walk-through: a full result

Query:

```eigenql
USING "urn:eigenius:example:Dog"

MATCH Dog(?d) {
    name: ?name,
    breed: ?breed
}
RETURN [Prediction] {
    name: ?name,
    breed: ?breed
}
```

Produces (conceptually) four resources:

Eigon-JSON uses full property IRIs as keys; `@id` is the only reserved short key. This convention is inherited from [JSON-AD](https://docs.atomicdata.dev/), the JSON serialization of Atomic Data from which Eigon-JSON descends ([D1 §1.1–1.2](../../design/d1-eigon-serialization-format.md)).

**Property 1: `name` column**

```json
{
    "@id": "urn:eigenius:query:gen:3f7a…:row:name",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "name",
    "urn:eigenius:core:data_type": "urn:eigenius:core:string"
}
```

**Property 2: `breed` column**

Same shape, different IRI and short name.

**Row Class**

```json
{
    "@id": "urn:eigenius:query:gen:3f7a…:row_class",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:short_name": "ResultRow",
    "urn:eigenius:core:subclass_of": ["urn:eigenius:example:Prediction"],
    "urn:eigenius:core:requires": [
        "urn:eigenius:query:gen:3f7a…:row:name",
        "urn:eigenius:query:gen:3f7a…:row:breed"
    ]
}
```

The row class inherits from the classes named in `RETURN [Prediction]` via `urn:eigenius:core:subclass_of` and requires each result property.

**ResultSet**

```json
{
    "@id": "urn:eigenius:query:gen:3f7a…:result",
    "urn:eigenius:core:is_a": ["urn:eigenius:query:ResultSet"],
    "urn:eigenius:query:result_class": "urn:eigenius:query:gen:3f7a…:row_class",
    "urn:eigenius:query:row_count": 2,
    "urn:eigenius:query:rows": [
        {
            "urn:eigenius:core:is_a": ["urn:eigenius:query:gen:3f7a…:row_class"],
            "urn:eigenius:query:gen:3f7a…:row:name": "Rex",
            "urn:eigenius:query:gen:3f7a…:row:breed": "German Shepherd"
        },
        {
            "urn:eigenius:core:is_a": ["urn:eigenius:query:gen:3f7a…:row_class"],
            "urn:eigenius:query:gen:3f7a…:row:name": "Buddy",
            "urn:eigenius:query:gen:3f7a…:row:breed": "Golden Retriever"
        }
    ]
}
```

Each row is an embedded resource tagged with the row class. Property values use the synthesized row-property IRIs as keys.

## 11.4. Why synthesize Property and Class resources?

EigenQL results aren't generic JSON — they're Eigon resources that describe themselves through the same class/property mechanism the rest of the system uses. Three consequences:

1. **Post-query querying**: a consumer can run a second EigenQL query against the result document to iterate rows by row-class.
2. **Validation**: the synthesized row class obligates the rows to carry the columns named in `RETURN`, which the validator can enforce if needed.
3. **Stability**: the IRI scheme is reproducible, so caches and traces can reference specific result sets without ambiguity.

## 11.5. Data-type inference for properties

When synthesizing each Property resource, the wrapper inspects the `RETURN` expression to pick a `data_type`:

- `Literal(Integer)` / `Integer`-returning ops → `urn:eigenius:core:integer`
- `Literal(Float)` / `Float`-returning ops → `urn:eigenius:core:float`
- `Literal(Boolean)` / comparisons / logical ops → `urn:eigenius:core:boolean`
- `Aggregate(Count)` → `urn:eigenius:core:integer`
- `Aggregate(Avg)` / `Aggregate(Pow)` → `urn:eigenius:core:float`
- Everything else → `urn:eigenius:core:string` (default for variables and unresolved expressions)

This is a syntactic inference — if a `RETURN` expression's runtime type differs from the static guess, the value is still stored correctly (Eigon's `Value` is dynamic), but the declared `data_type` in the Property resource may be less specific than reality. The inference is good enough for round-tripping through a consumer that reads the Property metadata.

## 11.6. Match-only result details

The minimal match-only result:

```json
{
    "@id": "urn:eigenius:query:gen:3f7a…:result",
    "urn:eigenius:core:is_a": ["urn:eigenius:query:ResultSet"],
    "urn:eigenius:query:matched": true,
    "urn:eigenius:query:row_count": 5
}
```

Use cases:

- **Existence check**: "Is there at least one Alice?" — `MATCH ?x {} WHERE ?x = "alice" RETURN [] {}` produces `matched: true/false`.
- **Counting without shaping**: `row_count` tells you the binding count without needing a GROUP BY.
- **Debugging**: `RETURN [] {}` is a quick way to confirm a query binds anything before writing the full RETURN.

## 11.7. Consuming results programmatically

Server-side Rust code can destructure the `Vec<Resource>`:

```rust
let results = execute_with(query, &layer, runtime)?;
let result_set = results.iter()
    .find(|r| r.is_a().iter().any(|i| i.as_str() == RESULT_SET_CLASS))
    .ok_or("no ResultSet")?;

let matched = result_set.get(&iri(MATCHED_PROP))
    .and_then(|v| v.as_boolean());
let row_count = result_set.get(&iri(ROW_COUNT_PROP))
    .and_then(|v| v.as_integer());
let rows = result_set.get(&iri(ROWS_PROP))
    .and_then(|v| match v { Value::Array(a) => Some(a), _ => None });
```

For queries with `RETURN` items, the Property resources preceding the ResultSet describe each column and can be used to reflect on result shape without parsing the ResultSet.

## 11.8. Serialization

The result vector is serializable through the standard Eigon pipelines:

- [`eigon_json::emit_document`](../../../kernel/src/ontology/eigon_json/mod.rs) — JSON output
- [`eigon_cbor::serialize_document`](../../../kernel/src/ontology/eigon_cbor/mod.rs) — binary (wire format for gRPC)

Both preserve the synthesized IRIs and the embedded row structure.

## 11.9. Reference constants

From [`kernel/src/query/document.rs`](../../../kernel/src/query/document.rs):

| Constant | IRI |
|---|---|
| `RESULT_SET_CLASS` | `urn:eigenius:query:ResultSet` |
| `RESULT_CLASS_PROP` | `urn:eigenius:query:result_class` |
| `ROWS_PROP` | `urn:eigenius:query:rows` |
| `ROW_COUNT_PROP` | `urn:eigenius:query:row_count` |
| `MATCHED_PROP` | `urn:eigenius:query:matched` |

These are **ephemeral** — they're not part of the core ontology, they live only in query result documents.

---

Next: **[12. Error messages →](12-error-messages.md)**
