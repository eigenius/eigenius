# D1: Eigon Serialization Format

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 0)
**Required before:** Phase 0 implementation
**Resolves:** Eigon JSON schema, property value encoding, IRI representation, blob references, resource identity

---

## 1. Overview

Eigon is the canonical data format for the Eigenius platform. All data — ontology definitions, instance resources, processing pipelines, reasoning traces — is represented as Eigon resources.

This document specifies the JSON serialization of Eigon resources (Eigon-JSON), the data_type system, resource identity and embedding rules, validation semantics, and canonical form for content-addressed hashing.

### 1.1 Design influences

The Eigon format is inspired by [Atomic Data](https://docs.atomicdata.dev/) and adapts its core ideas — typed properties, self-describing schemas, JSON serialization — with key differences:

- **IRIs, not URLs.** Identifiers are IRIs (Internationalized Resource Identifiers, RFC 3987) using the `urn:` scheme. They are not required to be fetchable over HTTP. Type information is resolved from the loaded ontology in the layer stack, not by dereferencing the identifier.
- **No `@context`.** Namespace resolution is handled by the layer stack, not by document-level declarations.
- **Three-layer type system.** Primitive data_types, format constraints, and content types are separated (inspired by JSON Schema).

### 1.2 Relationship to Atomic Data

| Concept | Atomic Data | Eigon |
|---------|------------|-------|
| Subject identity | Fetchable URL | IRI (typically URN) |
| Property keys | Fetchable URL | IRI (typically URN) |
| Type discovery | Fetch the property URL | Resolve from loaded ontology |
| Class membership | `is-a` property (single) | `is_a` property (array — multiple class membership) |
| Shortnames | Built-in, class-scoped | Stored as data on resources; not used as keys in core format |
| Namespace context | `@context` | None; full IRIs always |
| Property typing | `range` on property | `data_type` + optional `format`, `content_type`, `element_type` |
| Property ownership | Property declares its class | Class declares its required/recommended properties |

---

## 2. Resource identity

### 2.1 Top-level resources

A **top-level resource** has a globally unique identity expressed as an IRI in the `@id` field. Top-level resources are independently addressable and can be referenced from other resources.

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

Embedded resources may appear as values of properties whose data_type is `resource` or as elements in a `resource_array`.

Embedded resources may or may not carry `is_a` — an embedded object without `is_a` is an untyped embedded resource.

### 2.3 IRI conventions

All Eigenius-internal identifiers use the `urn:` scheme (a registered IANA scheme per RFC 8141), which is a valid IRI:

```
urn:eigenius:<namespace>:<local-name>
```

- **Core Ontology:** `urn:eigenius:core:` (immutable, baked into the kernel)
- **Foundation Layer:** `urn:eigenius:foundation:`
- **User/domain ontologies:** `urn:eigenius:<domain>:<local-name>`

Namespace depth is unrestricted — colons may nest arbitrarily (e.g., `urn:eigenius:example:animals:properties:breed`). The total IRI length must not exceed 512 characters.

IRIs from external systems (URLs, other URN namespaces) are also valid as identifiers.

---

## 3. Property keys and values

### 3.1 Property keys

All property keys in Eigon-JSON are full IRIs. There are no abbreviated forms within the core format. Shortnames are stored as data on Property resources (via the `short_name` property) for use by external integrations, but are never used as keys in Eigon-JSON documents.

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

| Property | IRI | DataType | Description |
|----------|-----|----------|-------------|
| is_a | `urn:eigenius:core:is_a` | resource_array | Must include the Property class |
| description | `urn:eigenius:core:description` | string | Human-readable description |
| short_name | `urn:eigenius:core:short_name` | string | Short identifier for external use |
| data_type | `urn:eigenius:core:data_type` | resource | Primitive data_type constraining values (see §3.3) |
| format | `urn:eigenius:core:format` | resource | Format constraint for string values (see §3.4) |
| pattern | `urn:eigenius:core:pattern` | string (format: regex) | Regular expression constraint for string values (ECMA 262 syntax, full-match) |
| content_type | `urn:eigenius:core:content_type` | string | MIME type for string content interpretation (see §3.5) |
| content_encoding | `urn:eigenius:core:content_encoding` | resource | Encoding for binary-in-string content (see §3.5). Constrained to core encodings via `allows_only` |
| element_type | `urn:eigenius:core:element_type` | resource | Element data_type for array-typed properties |
| class_types | `urn:eigenius:core:class_types` | resource_array | Allowed classes for `resource` or `resource_array` values |
| allows_only | `urn:eigenius:core:allows_only` | resource_array | Restricts `resource` or `resource_array` values to a specific set of resources |
| min_value | `urn:eigenius:core:min_value` | float | Minimum value (inclusive) for integer or float values |
| max_value | `urn:eigenius:core:max_value` | float | Maximum value (inclusive) for integer or float values |
| min_length | `urn:eigenius:core:min_length` | integer | Minimum length for string values or array properties |
| max_length | `urn:eigenius:core:max_length` | integer | Maximum length for string values or array properties |
| domain | `urn:eigenius:core:domain` | resource_array | Restricts which class types this property may be used on. If omitted, the property may be used on any resource |

The following core property is available on any resource (not specific to Property definitions):

| Property | IRI | DataType | Description |
|----------|-----|----------|-------------|
| source_irl | `urn:eigenius:core:source_irl` | string (format: iri) | Optional fetchable IRL where the resource can be found or was sourced from |

### 3.3 Primitive data_types

Primitive data_types determine the JSON-level representation of a value:

| DataType | IRI | JSON type | Constraints |
|----------|-----|-----------|-------------|
| string | `urn:eigenius:core:string` | `string` | UTF-8. May carry `format`, `content_type`, and/or `content_encoding` |
| integer | `urn:eigenius:core:integer` | `number` | Signed 53-bit safe range (-(2^53-1) to 2^53-1), no decimal point |
| float | `urn:eigenius:core:float` | `number` | 64-bit IEEE 754 |
| boolean | `urn:eigenius:core:boolean` | `boolean` | |
| resource | `urn:eigenius:core:resource` | `string` or `object` | String = IRI reference, object = embedded resource. May carry `class_types` |
| resource_array | `urn:eigenius:core:resource_array` | `array` | Of strings (IRI refs) or objects (embedded). May carry `class_types` |
| value_array | `urn:eigenius:core:value_array` | `array` | Homogeneous primitives. Requires `element_type` |
| json | `urn:eigenius:core:json` | any | Opaque JSON value, not validated by the ontology |

### 3.4 Formats

Formats are validation constraints on `string` values. They are declared on the Property definition via the `format` property. The kernel validates values against the declared format.

| Format | IRI | Constraint |
|--------|-----|-----------|
| date | `urn:eigenius:core:formats:date` | `YYYY-MM-DD` (ISO 8601 date) |
| datetime | `urn:eigenius:core:formats:datetime` | ISO 8601 date-time with timezone |
| time | `urn:eigenius:core:formats:time` | ISO 8601 time |
| iri | `urn:eigenius:core:formats:iri` | Valid IRI (RFC 3987) |
| uuid | `urn:eigenius:core:formats:uuid` | RFC 4122 UUID |
| regex | `urn:eigenius:core:formats:regex` | Valid ECMA 262 regular expression |

Formats are extensible — domain ontologies may define additional formats in their own namespaces.

Example — a date property:

```json
{
  "@id": "urn:eigenius:example:birthdate",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Date of birth",
  "urn:eigenius:core:short_name": "birthdate",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:format": "urn:eigenius:core:formats:date"
}
```

An instance using it: `"urn:eigenius:example:birthdate": "1990-03-15"`

### 3.5 Content types and content encoding

Content types declare the MIME type of content embedded in a string value, following standard media types (RFC 2046). Content encoding declares how binary content is encoded within the string.

These are declared on the Property definition via `content_type` and optionally `content_encoding`.

Example — a Markdown property:

```json
{
  "@id": "urn:eigenius:example:bio",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Biographical information in Markdown format",
  "urn:eigenius:core:short_name": "bio",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:content_type": "text/markdown"
}
```

Example — an inline binary image:

```json
{
  "@id": "urn:eigenius:example:avatar",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Profile photo",
  "urn:eigenius:core:short_name": "avatar",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:content_type": "image/png",
  "urn:eigenius:core:content_encoding": "urn:eigenius:core:encodings:base64"
}
```

Example — a reference to an externally stored blob:

```json
{
  "@id": "urn:eigenius:example:document",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Reference to an uploaded document",
  "urn:eigenius:core:short_name": "document",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:format": "urn:eigenius:core:formats:iri",
  "urn:eigenius:core:content_type": "application/pdf"
}
```

In this case, `format: iri` validates that the value is a valid IRI, and `content_type: application/pdf` indicates what the IRI points to.

### 3.6 Resource references vs. embedded resources

When a property has data_type `resource` or `resource_array`:

- A **string value** is an IRI reference to another top-level resource: `"urn:eigenius:example:bob"`
- An **object value** is an embedded resource (no `@id`): `{ "urn:eigenius:core:is_a": [...], ... }`

This distinction is unambiguous from the JSON type alone.

### 3.7 Class membership (`is_a`)

The `is_a` property is always an **array of resource references**, supporting multiple class membership:

```json
"urn:eigenius:core:is_a": ["urn:eigenius:example:Dog", "urn:eigenius:example:Pet"]
```

A resource may be an instance of multiple classes simultaneously. Validation applies the requirements of all declared classes (see §5.3).

### 3.8 Absence and null

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
    "urn:eigenius:core:short_name": "Person",
    "urn:eigenius:core:requires": ["urn:eigenius:example:name"]
  },
  {
    "@id": "urn:eigenius:example:name",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:description": "A person's name",
    "urn:eigenius:core:short_name": "name",
    "urn:eigenius:core:data_type": "urn:eigenius:core:string"
  }
]
```

A multi-resource document file may be associated with a specific node in the namespace hierarchy, providing context to the resources within it.

---

## 5. Class definitions and validation

### 5.1 Class structure

A Class is a resource that declares which properties its instances must or should provide. Classes do not own properties — properties are independent resources. The class-to-property relationship is expressed through `requires` and `recommends`.

| Property | IRI | DataType | Description |
|----------|-----|----------|-------------|
| is_a | `urn:eigenius:core:is_a` | resource_array | Must include `urn:eigenius:core:Class` |
| description | `urn:eigenius:core:description` | string | Human-readable description |
| short_name | `urn:eigenius:core:short_name` | string | Short identifier |
| subclass_of | `urn:eigenius:core:subclass_of` | resource_array | Parent classes in the inheritance hierarchy |
| requires | `urn:eigenius:core:requires` | resource_array | Properties that instances must provide |
| recommends | `urn:eigenius:core:recommends` | resource_array | Properties that instances should provide |

Example:

```json
{
  "@id": "urn:eigenius:example:Dog",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
  "urn:eigenius:core:description": "A dog",
  "urn:eigenius:core:short_name": "Dog",
  "urn:eigenius:core:subclass_of": ["urn:eigenius:example:Animal"],
  "urn:eigenius:core:requires": ["urn:eigenius:example:breed"]
}
```

### 5.2 Conditional requirements

Classes may declare **conditional requirements** — properties that are required or recommended only when another property has a specific value. This is expressed via the `conditional_requires` property on a Class, whose value is an array of embedded `ConditionalRequirement` resources.

A `ConditionalRequirement` declares:

| Property | IRI | DataType | Description |
|----------|-----|----------|-------------|
| when_property | `urn:eigenius:core:when_property` | resource | The property whose value triggers the condition |
| has_value | `urn:eigenius:core:has_value` | resource_array | The condition matches if the property's value is any of these |
| then_requires | `urn:eigenius:core:then_requires` | resource_array | Properties that become required when the condition matches |
| then_recommends | `urn:eigenius:core:then_recommends` | resource_array | Properties that become recommended when the condition matches |

Example — `element_type` is required when `data_type` is `value_array`:

```json
{
  "urn:eigenius:core:when_property": "urn:eigenius:core:data_type",
  "urn:eigenius:core:has_value": ["urn:eigenius:core:value_array"],
  "urn:eigenius:core:then_requires": ["urn:eigenius:core:element_type"]
}
```

### 5.3 Shortname uniqueness

Within a class's effective property set (the union of its own `requires`/`recommends` and those inherited from all ancestor classes), short_names must be unique.

If two ancestor classes contribute properties with the same short_name (e.g., through multiple inheritance), the conflict is resolved by **declaration order**: the property appearing first in the `requires`/`recommends` list wins. When the conflict arises across inheritance levels, the more specific (deeper) class's declaration is processed first.

### 5.4 Validation rules

1. **Required properties:** A resource must provide values for all properties listed in `requires` on each of its classes (all entries in `is_a`).
2. **Inherited requirements:** A subclass inherits the `requires` and `recommends` lists from all ancestor classes. An instance of `Dog` must satisfy requirements from both `Dog` and `Animal`.
3. **Type checking:** Each property value must conform to the property's declared `data_type`. For `value_array` properties, each element must conform to the property's `element_type`.
4. **Format checking:** If a property declares a `format`, the string value must conform to the format's validation rules.
5. **Pattern checking:** If a property declares a `pattern`, the string value must fully match the regular expression (ECMA 262 syntax, implicitly anchored as a full match).
6. **Range checking:** If a property declares `min_value` or `max_value`, the numeric value must fall within the specified range (inclusive).
7. **Length checking:** If a property declares `min_length` or `max_length`, the string length (in characters) or array length (in elements) must fall within the specified range.
8. **Class type checking:** If a property declares `class_types`, any resource value (reference or embedded) must be an instance of at least one of the listed classes (including subclasses).
9. **Allowed values checking:** If a property declares `allows_only`, any resource value must be one of the listed resources (by IRI identity).
10. **Domain checking:** If a property declares `domain`, it may only be used on resources that are instances of at least one of the listed classes (including subclasses).
11. **Conditional requirements:** If a class declares `conditional_requires`, and a resource's property value matches the `has_value` condition, the properties listed in `then_requires` become required and those in `then_recommends` become recommended.
12. **Open world:** Extra properties — those not declared in `requires` or `recommends` on any of the resource's classes or their ancestors — are **allowed**. Their presence is not an error.

### 5.5 Self-description

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
    "urn:eigenius:core:short_name": "Animal",
    "urn:eigenius:core:requires": ["urn:eigenius:example:name"]
  },
  {
    "@id": "urn:eigenius:example:Dog",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:description": "A dog",
    "urn:eigenius:core:short_name": "Dog",
    "urn:eigenius:core:subclass_of": ["urn:eigenius:example:Animal"],
    "urn:eigenius:core:requires": ["urn:eigenius:example:breed"]
  },
  {
    "@id": "urn:eigenius:example:name",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:description": "Name of the animal",
    "urn:eigenius:core:short_name": "name",
    "urn:eigenius:core:data_type": "urn:eigenius:core:string"
  },
  {
    "@id": "urn:eigenius:example:breed",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:description": "Breed of the dog",
    "urn:eigenius:core:short_name": "breed",
    "urn:eigenius:core:data_type": "urn:eigenius:core:string"
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

### 8.4 Property with format constraint

```json
{
  "@id": "urn:eigenius:example:birthdate",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Date of birth",
  "urn:eigenius:core:short_name": "birthdate",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:format": "urn:eigenius:core:formats:date"
}
```

Instance value: `"urn:eigenius:example:birthdate": "1990-03-15"`

### 8.5 Property with content type

```json
{
  "@id": "urn:eigenius:example:bio",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Biographical information in Markdown format",
  "urn:eigenius:core:short_name": "bio",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:content_type": "text/markdown"
}
```

Instance value: `"urn:eigenius:example:bio": "# Alice\n\nAlice is a **software engineer** based in Berlin."`

### 8.6 Inline binary data

```json
{
  "@id": "urn:eigenius:example:avatar",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Profile photo",
  "urn:eigenius:core:short_name": "avatar",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:content_type": "image/png",
  "urn:eigenius:core:content_encoding": "urn:eigenius:core:encodings:base64"
}
```

Instance value: `"urn:eigenius:example:avatar": "iVBORw0KGgoAAAANSUhEUg..."`

### 8.7 Blob reference

```json
{
  "@id": "urn:eigenius:example:document",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Reference to an uploaded document",
  "urn:eigenius:core:short_name": "document",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:format": "urn:eigenius:core:formats:iri",
  "urn:eigenius:core:content_type": "application/pdf"
}
```

Instance value: `"urn:eigenius:example:document": "urn:eigenius:blob:abc123"`

### 8.8 Property with regex pattern

```json
{
  "@id": "urn:eigenius:example:phone",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "US phone number",
  "urn:eigenius:core:short_name": "phone",
  "urn:eigenius:core:data_type": "urn:eigenius:core:string",
  "urn:eigenius:core:pattern": "^\\([0-9]{3}\\)[0-9]{3}-[0-9]{4}$"
}
```

Instance value: `"urn:eigenius:example:phone": "(555)123-4567"`

### 8.9 Class-constrained resource property

```json
{
  "@id": "urn:eigenius:example:author",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
  "urn:eigenius:core:description": "Author of the document",
  "urn:eigenius:core:short_name": "author",
  "urn:eigenius:core:data_type": "urn:eigenius:core:resource",
  "urn:eigenius:core:class_types": ["urn:eigenius:example:Person"]
}
```

---

## 9. Decisions log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Identifier terminology | IRI (RFC 3987); `urn:` scheme for Eigenius identifiers | IRIs are the modern standard; `urn:` is a registered IANA scheme with universal tooling support |
| Property keys | Full IRIs always | URNs are not fetchable; short_names stored as data, not used as keys |
| System fields | `@id` only | Class membership via `is_a` property, not a system field |
| `is_a` cardinality | Always an array | Supports multiple class membership |
| Property ownership | Classes declare `requires`/`recommends` | Properties are independent; not scoped to a single class |
| Type system | Three layers: primitive data_types, formats, content types | Separates JSON representation, validation constraints, and content interpretation (inspired by JSON Schema) |
| Primitive data_types | `string`, `integer`, `float`, `boolean`, `resource`, `resource_array`, `value_array`, `json` | Only types that affect JSON-level representation |
| Formats | `date`, `datetime`, `time`, `iri`, `uuid` (extensible) | Validation constraints on string values; kernel-enforced |
| Pattern | Regex constraint on string properties (ECMA 262, full match) | Custom validation without defining new formats; follows JSON Schema |
| Content types | Standard MIME types (`text/markdown`, `text/html`, `image/png`, etc.) | Interpretation hints for string content; extensible without core changes |
| Content encoding | `base64` for binary-in-string | Follows JSON Schema pattern; enables inline binary data |
| Integer range | 53-bit safe range (-(2^53-1) to 2^53-1) | JSON numbers are IEEE 754 doubles; this is the safe integer range |
| `blob` data_type | Removed; use `string` + `format: iri` + `content_type` | Blob references and inline binary are both expressible via the three-layer type system |
| `markdown`, `uri`, `date`, `datetime` data_types | Removed as primitives; expressed via `format` and `content_type` on `string` | Avoids unbounded growth of primitive types for what are really string variants |
| Shortnames | Stored as `short_name` property on resources | Available for external integrations; never used as keys in core format |
| Naming | `short_name` + `description` (no `label`) | Follows Atomic Data; short_name for identifiers, description for human-readable text |
| Namespace depth | Unrestricted, max 512 chars total IRI length | Supports hierarchical organization (e.g., `urn:eigenius:example:animals:properties:breed`) |
| Shortname uniqueness | Unique within a class's effective property set; declaration order resolves conflicts | Prevents ambiguity in external integrations using short_names |
| Property `required` field | Not used; classes declare `requires` instead | Avoids redundancy between property-level and class-level declarations |
| Namespace context | None | Full IRIs eliminate ambiguity; layer stack handles resolution |
| Null handling | Missing key = no value; explicit null forbidden | Simplicity; follows Atomic Data precedent |
| Extra properties | Allowed (open world) | Flexibility for extension without schema changes |
| Class inheritance | Subclass inherits `requires`/`recommends` from ancestors | Natural expectation; Dog must satisfy Animal requirements |
| Identity model | Top-level resources have `@id`; embedded resources do not | Embedded resources are addressed through their parent |
| Embedded `is_a` | Optional on embedded resources | Untyped embedded objects are allowed |
| Canonical form | RFC 8785 (JCS) | Enables content-addressed hashing for layer identifiers |
