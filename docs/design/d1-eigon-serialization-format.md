# D1: Eigon Serialization Format

*Design document for the Eigenius project — April 2026*

**Status:** Draft
**Required before:** Phase 0 implementation
**Resolves:** Eigon JSON schema, property value encoding, URI representation, blob references, resource identity

---

## 1. Overview

Eigon is the canonical data format for the Eigenius platform. All data — ontology definitions, instance resources, processing pipelines, reasoning traces — is represented as Eigon resources.

This document specifies the JSON serialization of Eigon resources (Eigon-JSON), the datatype system, resource identity and embedding rules, validation semantics, and canonical form for content-addressed hashing.

### 1.1 Design influences

The Eigon format is inspired by [Atomic Data](https://docs.atomicdata.dev/) and adapts its core ideas — typed properties, self-describing schemas, JSON serialization — with key differences:

- **URIs, not URLs.** Identifiers are URIs that may be URNs (`urn:eigenius:...`). They are not required to be fetchable over HTTP. Type information is resolved from the loaded ontology in the layer stack, not by dereferencing the identifier.
- **No `@context`.** Namespace resolution is handled by the layer stack, not by document-level declarations.

### 1.2 Relationship to Atomic Data

| Concept | Atomic Data | Eigon |
|---------|------------|-------|
| Subject identity | Fetchable URL | URI (may be URN) |
| Property keys | Fetchable URL | URI (may be URN) |
| Type discovery | Fetch the property URL | Resolve from loaded ontology |
| Class membership | `is-a` property (single) | `is_a` property (array — multiple class membership) |
| Shortnames | Built-in, class-scoped | Stored as data on resources; not used as keys in core format |
| Namespace context | `@context` | None; full URIs always |
| Property typing | `range` on property | `datatype` on property, optional `elementtype` for arrays |
| Property ownership | Property declares its class | Class declares its required/recommended properties |

---

## 2. Resource identity

### 2.1 Top-level resources

A **top-level resource** has a globally unique identity expressed as a URI in the `@id` field. Top-level resources are independently addressable and can be referenced from other resources.

```json
{
  "@id": "urn:eigenius:example:alice",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Person"],
  "urn:eigenius:example:name": "Alice"
}
```

`@id` is the only reserved key in Eigon-JSON.

### 2.2 Embedded resources

An **embedded resource** is a JSON object without an `@id` field, nested as a property value of a top-level resource (or of another embedded resource). Embedded resources have no independent identity and are addressable only by navigating through their owning top-level resource.

```json
{
  "@id": "urn:eigenius:example:alice",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Person"],
  "urn:eigenius:example:name": "Alice",
  "urn:eigenius:example:address": {
    "urn:eigenius:core:is_a": ["urn:eigenius:example:Address"],
    "urn:eigenius:example:city": "Berlin",
    "urn:eigenius:example:country": "Germany"
  }
}
```

Embedded resources may appear as values of properties whose datatype is `resource` or as elements in a `resource_array`.

Embedded resources may or may not carry `is_a` — an embedded object without `is_a` is an untyped embedded resource.

### 2.3 URI conventions

All Eigenius-internal identifiers use the URN scheme:

```
urn:eigenius:<namespace>:<local-name>
```

- **Core Ontology:** `urn:eigenius:core:` (immutable, baked into the kernel)
- **Foundation Layer:** `urn:eigenius:foundation:`
- **User/domain ontologies:** `urn:eigenius:<domain>:<local-name>`

URIs from external systems (URLs, other URN schemes) are also valid as identifiers.

---

## 3. Property keys and values

### 3.1 Property keys

All property keys in Eigon-JSON are full URIs. There are no abbreviated forms within the core format. Shortnames are stored as data on Property resources (via the `shortname` property) for use by external integrations, but are never used as keys in Eigon-JSON documents.

```json
{
  "@id": "urn:eigenius:example:alice",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Person"],
  "urn:eigenius:example:name": "Alice",
  "urn:eigenius:example:age": 30
}
```

### 3.2 Property definitions

A Property is itself a resource. Properties do not declare which class they belong to — instead, classes declare which properties they require or recommend (see §5.1). A Property definition carries:

| Property | URI | Datatype | Description |
|----------|-----|----------|-------------|
| is_a | `urn:eigenius:core:is_a` | resource_array | Must include the Property class |
| description | `urn:eigenius:core:description` | string | Human-readable description |
| shortname | `urn:eigenius:core:shortname` | string | Short identifier for external use |
| datatype | `urn:eigenius:core:datatype` | resource | Datatype constraining values |
| elementtype | `urn:eigenius:core:elementtype` | resource | Element datatype for array-typed properties |

Example:

```json
{
  "@id": "urn:eigenius:example:attachments",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Optional attachments (multiple allowed).",
  "urn:eigenius:core:shortname": "attachments",
  "urn:eigenius:core:datatype": "urn:eigenius:core:value_array",
  "urn:eigenius:core:elementtype": "urn:eigenius:core:string"
}
```

### 3.3 Datatypes and value encoding

Property values are typed according to the property's `datatype` declaration. The JSON encoding for each datatype:

| Datatype | URI | JSON type | Constraints | Example |
|----------|-----|-----------|-------------|---------|
| string | `urn:eigenius:core:string` | `string` | UTF-8 | `"hello"` |
| markdown | `urn:eigenius:core:markdown` | `string` | CommonMark | `"# Title\n\nBody"` |
| integer | `urn:eigenius:core:integer` | `number` | Signed 64-bit, no decimal | `42` |
| float | `urn:eigenius:core:float` | `number` | 64-bit IEEE 754 | `3.14` |
| boolean | `urn:eigenius:core:boolean` | `boolean` | | `true` |
| datetime | `urn:eigenius:core:datetime` | `string` | ISO 8601 with timezone | `"2026-04-09T14:30:00Z"` |
| date | `urn:eigenius:core:date` | `string` | `YYYY-MM-DD` | `"2026-04-09"` |
| uri | `urn:eigenius:core:uri` | `string` | Valid URI | `"urn:eigenius:example:foo"` |
| resource | `urn:eigenius:core:resource` | `string` or `object` | String = reference, object = embedded | `"urn:eigenius:example:bob"` |
| resource_array | `urn:eigenius:core:resource_array` | `array` | Of strings (refs) or objects (embedded) | `["urn:...", {"...": "..."}]` |
| value_array | `urn:eigenius:core:value_array` | `array` | Homogeneous primitives; element type declared via `elementtype` | `[1, 2, 3]` |
| blob | `urn:eigenius:core:blob` | `string` | URI reference to blob storage | `"urn:eigenius:blob:abc123"` |
| json | `urn:eigenius:core:json` | any | Opaque, not validated by ontology | `{"arbitrary": true}` |

### 3.4 Resource references vs. embedded resources

When a property has datatype `resource` or `resource_array`:

- A **string value** is a URI reference to another top-level resource: `"urn:eigenius:example:bob"`
- An **object value** is an embedded resource (no `@id`): `{ "urn:eigenius:core:is_a": [...], ... }`

This distinction is unambiguous from the JSON type alone.

### 3.5 Class membership (`is_a`)

The `is_a` property is always an **array of resource references**, supporting multiple class membership:

```json
"urn:eigenius:core:is_a": ["urn:eigenius:example:Dog", "urn:eigenius:example:Pet"]
```

A resource may be an instance of multiple classes simultaneously. Validation applies the requirements of all declared classes (see §5.2).

### 3.6 Absence and null

- A **missing key** means the property has no value on this resource.
- Explicit `null` values are **not allowed** in Eigon-JSON. Omit the key instead.
- Empty arrays and empty objects are not allowed.

---

## 4. Documents

### 4.1 Single-resource document

A JSON object with an `@id` field:

```json
{
  "@id": "urn:eigenius:example:alice",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Person"],
  "urn:eigenius:example:name": "Alice"
}
```

### 4.2 Multi-resource document

A JSON array of top-level resources. This is a convenience for loading and authoring. The underlying store always deals in individual resources.

```json
[
  {
    "@id": "urn:eigenius:example:Person",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:description": "A person",
    "urn:eigenius:core:shortname": "Person",
    "urn:eigenius:core:requires": ["urn:eigenius:example:name"]
  },
  {
    "@id": "urn:eigenius:example:name",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:description": "A person's name",
    "urn:eigenius:core:shortname": "name",
    "urn:eigenius:core:datatype": "urn:eigenius:core:string"
  }
]
```

A multi-resource document file may be associated with a specific node in the namespace hierarchy, providing context to the resources within it.

---

## 5. Class definitions and validation

### 5.1 Class structure

A Class is a resource that declares which properties its instances must or should provide. Classes do not own properties — properties are independent resources. The class-to-property relationship is expressed through `requires` and `recommends`.

| Property | URI | Datatype | Description |
|----------|-----|----------|-------------|
| is_a | `urn:eigenius:core:is_a` | resource_array | Must include `urn:eigenius:core:Class` |
| description | `urn:eigenius:core:description` | string | Human-readable description |
| shortname | `urn:eigenius:core:shortname` | string | Short identifier |
| parent_classes | `urn:eigenius:core:parent_classes` | resource_array | Parent classes in the inheritance hierarchy |
| requires | `urn:eigenius:core:requires` | resource_array | Properties that instances must provide |
| recommends | `urn:eigenius:core:recommends` | resource_array | Properties that instances should provide |

Example:

```json
{
  "@id": "urn:eigenius:example:Dog",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
  "urn:eigenius:core:description": "A dog",
  "urn:eigenius:core:shortname": "Dog",
  "urn:eigenius:core:parent_classes": ["urn:eigenius:example:Animal"],
  "urn:eigenius:core:requires": ["urn:eigenius:example:breed"]
}
```

### 5.2 Validation rules

1. **Required properties:** A resource must provide values for all properties listed in `requires` on each of its classes (all entries in `is_a`).
2. **Inherited requirements:** A subclass inherits the `requires` and `recommends` lists from all ancestor classes. An instance of `Dog` must satisfy requirements from both `Dog` and `Animal`.
3. **Type checking:** Each property value must conform to the property's declared `datatype`. For `value_array` properties, each element must conform to the property's `elementtype`.
4. **Open world:** Extra properties — those not declared in `requires` or `recommends` on any of the resource's classes or their ancestors — are **allowed**. Their presence is not an error.

### 5.3 Self-description

The Core Ontology is self-describing:

- `urn:eigenius:core:Class` is an instance of `urn:eigenius:core:Class` (its `is_a` includes itself)
- `urn:eigenius:core:Property` is an instance of `urn:eigenius:core:Class`
- `urn:eigenius:core:is_a` is an instance of `urn:eigenius:core:Property`

This bootstrap circularity is resolved by hardcoding the Core Ontology in the kernel.

---

## 6. Canonical form

For content-addressed hashing (used by the layer system for layer identifiers), Eigon-JSON is serialized in canonical form following RFC 8785 (JSON Canonicalization Scheme):

1. All keys sorted lexicographically (Unicode code point order)
2. No insignificant whitespace
3. No empty objects, empty arrays, or null values
4. Deterministic number representation (no trailing zeros, no positive sign, exponential notation only when required by RFC 8785)

The canonical form of a resource produces a deterministic byte sequence suitable for hashing (e.g., SHA-256) to produce content-addressed identifiers.

---

## 7. MIME type

Eigon-JSON documents use the MIME type `application/eigon+json`.

---

## 8. Examples

### 8.1 Defining an ontology

```json
[
  {
    "@id": "urn:eigenius:example:Animal",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:description": "An animal",
    "urn:eigenius:core:shortname": "Animal",
    "urn:eigenius:core:requires": ["urn:eigenius:example:name"]
  },
  {
    "@id": "urn:eigenius:example:Dog",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:description": "A dog",
    "urn:eigenius:core:shortname": "Dog",
    "urn:eigenius:core:parent_classes": ["urn:eigenius:example:Animal"],
    "urn:eigenius:core:requires": ["urn:eigenius:example:breed"]
  },
  {
    "@id": "urn:eigenius:example:name",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:description": "Name of the animal",
    "urn:eigenius:core:shortname": "name",
    "urn:eigenius:core:datatype": "urn:eigenius:core:string"
  },
  {
    "@id": "urn:eigenius:example:breed",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:description": "Breed of the dog",
    "urn:eigenius:core:shortname": "breed",
    "urn:eigenius:core:datatype": "urn:eigenius:core:string"
  }
]
```

### 8.2 Creating an instance

```json
{
  "@id": "urn:eigenius:example:rex",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Dog"],
  "urn:eigenius:example:name": "Rex",
  "urn:eigenius:example:breed": "German Shepherd"
}
```

This resource is valid because:
- `Dog` requires `breed` — present
- `Dog` inherits from `Animal`, which requires `name` — present
- Open-world: additional properties would also be allowed

### 8.3 Embedded resources

```json
{
  "@id": "urn:eigenius:example:alice",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Person"],
  "urn:eigenius:example:name": "Alice",
  "urn:eigenius:example:address": {
    "urn:eigenius:core:is_a": ["urn:eigenius:example:Address"],
    "urn:eigenius:example:street": "Unter den Linden 1",
    "urn:eigenius:example:city": "Berlin",
    "urn:eigenius:example:country": "Germany"
  }
}
```

The address has no `@id` and exists only as part of Alice's resource.

### 8.4 DAG pipeline (real-world pattern)

A DAG pipeline with embedded step and binding resources, demonstrating deeply nested embedded resources and multiple class membership:

```json
{
  "@id": "urn:eigenius:example:my-pipeline",
  "urn:eigenius:core:is_a": ["urn:eigenius:dag:Dag"],
  "urn:eigenius:dag:input": "urn:eigenius:example:InputBundle",
  "urn:eigenius:dag:output": "urn:eigenius:example:OutputData",
  "urn:eigenius:dag:steps": [
    {
      "urn:eigenius:core:is_a": ["urn:eigenius:dag:Step"],
      "urn:eigenius:dag:step_label": "extract",
      "urn:eigenius:dag:component": "urn:eigenius:dag:components:CompleteJson",
      "urn:eigenius:dag:bindings": [
        {
          "urn:eigenius:core:is_a": ["urn:eigenius:dag:Binding"],
          "urn:eigenius:dag:step_label": "input",
          "urn:eigenius:dag:component_property": "urn:eigenius:common:string",
          "urn:eigenius:dag:context_property": "urn:eigenius:example:letter"
        }
      ]
    }
  ]
}
```

---

## 9. Decisions log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Property keys | Full URIs always | URNs are not fetchable; shortnames stored as data, not used as keys |
| System fields | `@id` only | Class membership via `is_a` property, not a system field |
| `is_a` cardinality | Always an array | Supports multiple class membership |
| Property ownership | Classes declare `requires`/`recommends` | Properties are independent; not scoped to a single class |
| Property typing | `datatype` + optional `elementtype` | Clearer than overloaded `range`; explicit array element typing |
| Shortnames | Stored as `shortname` property on resources | Available for external integrations; never used as keys in core format |
| Description | `description` property (not `label`) | Human-readable description of the resource |
| Namespace context | None | Full URIs eliminate ambiguity; layer stack handles resolution |
| Null handling | Missing key = no value; explicit null forbidden | Simplicity; follows Atomic Data precedent |
| Extra properties | Allowed (open world) | Flexibility for extension without schema changes |
| Class inheritance | Subclass inherits `requires`/`recommends` from ancestors | Natural expectation; Dog must satisfy Animal requirements |
| Blob encoding | URI reference to blob storage | Keeps documents lightweight; blob storage is a separate concern |
| Identity model | Top-level resources have `@id`; embedded resources do not | Embedded resources are addressed through their parent |
| Embedded `is_a` | Optional on embedded resources | Untyped embedded objects are allowed |
| Canonical form | RFC 8785 (JCS) | Enables content-addressed hashing for layer identifiers |
