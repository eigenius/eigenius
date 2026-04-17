# D8: CompleteJson Component — Structured LLM Output

*Design document for the Eigenius project — April 2026*

**Status:** Draft
**Required before:** CompleteJson implementation
**Depends on:** D1 (Eigon format), D3 (program model), D6 (execution architecture)

---

## 1. Overview

CompleteJson is an IO component that calls an LLM with a JSON Schema derived from an Eigenius ontology class, receives structured JSON using short names, and converts it back to a fully typed Eigon resource. Unlike CompleteText (which returns raw text), CompleteJson produces typed, validated resources that integrate directly into the knowledge graph.

The core challenge: the LLM sees JSON Schema with short property names (human-readable), but Eigenius uses full IRI property keys. The mapping must be **bijective** — every short name maps to exactly one IRI and vice versa within the scope of a single class.

---

## 2. Data Flow

```
                                  JSON Schema                          
Ontology class  ───► Schema      (short names)     ───► LLM
(full IRIs)         Generator    ───────────────────►    (generateObject)
                                                         │
                                                         ▼
Eigon resource  ◄─── Resource    simple JSON       ◄─── JSON response
(full IRIs)         Converter    (short names)           (short names)
```

1. **Schema generation**: walk the class definition in the layer chain, collect `requires`/`recommends` properties (including inherited), map each to a JSON Schema property using `short_name` as the key.
2. **LLM call**: send the schema + prompt to the LLM via Vercel AI SDK's `generateObject()`.
3. **Resource conversion**: map the JSON response back from short names to full IRIs, wrapping as a typed Eigon resource with `is_a`.

---

## 3. Schema Generation

### 3.1 Property Collection

Given a class IRI, walk the class definition and all ancestor classes (via `subclass_of`) to collect:
- **required**: all properties from `requires` (including inherited)
- **optional**: all properties from `recommends` (including inherited)

Exclude meta-properties that are part of the ontology infrastructure: `is_a`, `description`, `short_name`, `subclass_of`, `requires`, `recommends`, `conditional_requires`, `domain`, `source_irl`.

### 3.2 Data Type Mapping

| Eigon data_type | JSON Schema type | Notes |
|----------------|-----------------|-------|
| `string` | `{ "type": "string" }` | |
| `integer` | `{ "type": "integer" }` | |
| `float` | `{ "type": "number" }` | |
| `boolean` | `{ "type": "boolean" }` | |
| `resource` | `{ "type": "string" }` or `{ "type": "object" }` | See §3.4 |
| `resource_array` | `{ "type": "array", "items": ... }` | See §3.5 |
| `value_array` | `{ "type": "array", "items": ... }` | Element type from `element_type` |
| `json` | `{}` | Any JSON value |

### 3.3 Constraint Mapping

| Eigon constraint | JSON Schema | Example |
|-----------------|-------------|---------|
| `min_value` | `minimum` | `{ "type": "integer", "minimum": 0 }` |
| `max_value` | `maximum` | `{ "type": "number", "maximum": 100.0 }` |
| `min_length` | `minLength` / `minItems` | String or array |
| `max_length` | `maxLength` / `maxItems` | String or array |
| `pattern` | `pattern` | `{ "type": "string", "pattern": "^[A-Z]" }` |
| `format` | `format` | `date`, `date-time`, `time`, `uri`, `uuid` |

Format mapping:

| Eigon format | JSON Schema format |
|-------------|-------------------|
| `urn:eigenius:core:formats:date` | `date` |
| `urn:eigenius:core:formats:datetime` | `date-time` |
| `urn:eigenius:core:formats:time` | `time` |
| `urn:eigenius:core:formats:iri` | `uri` |
| `urn:eigenius:core:formats:uuid` | `uuid` |

### 3.4 Resource References and allows_only → Enums

When a property has `data_type: resource` and `allows_only`, the allowed values map to a JSON Schema `enum` using their `short_name`:

**Ontology:**
```json
{
  "@id": "urn:eigenius:example:severity",
  "urn:eigenius:core:data_type": "urn:eigenius:core:resource",
  "urn:eigenius:core:allows_only": [
    "urn:eigenius:example:severity:low",
    "urn:eigenius:example:severity:medium",
    "urn:eigenius:example:severity:high"
  ]
}
```

Where each allowed value has a `short_name`:
```json
{ "@id": "urn:eigenius:example:severity:low", "urn:eigenius:core:short_name": "low" }
{ "@id": "urn:eigenius:example:severity:medium", "urn:eigenius:core:short_name": "medium" }
{ "@id": "urn:eigenius:example:severity:high", "urn:eigenius:core:short_name": "high" }
```

**Generated JSON Schema:**
```json
{
  "severity": {
    "type": "string",
    "enum": ["low", "medium", "high"]
  }
}
```

**Conversion back:** `"severity": "high"` → `"urn:eigenius:example:severity": "urn:eigenius:example:severity:high"` by looking up which allowed value has `short_name = "high"`.

**Uniqueness requirement:** all values in `allows_only` must have distinct `short_name` values. If two share a short name, schema generation fails with an error.

### 3.5 Nested Objects (class_types)

When a property has `data_type: resource` and `class_types` (but no `allows_only`), the property value is a nested object. The schema generator recurses into the referenced class:

**Ontology:**
```json
{
  "@id": "urn:eigenius:example:address",
  "urn:eigenius:core:data_type": "urn:eigenius:core:resource",
  "urn:eigenius:core:class_types": ["urn:eigenius:example:Address"]
}
```

**Generated JSON Schema:**
```json
{
  "address": {
    "type": "object",
    "properties": {
      "street": { "type": "string" },
      "city": { "type": "string" },
      "zip": { "type": "string" }
    },
    "required": ["street", "city"]
  }
}
```

Recursion depth is bounded (default: 4 levels). Circular references are detected and rejected.

When `class_types` lists multiple classes, this is a **union type** — see §3.6.

### 3.6 Union Types (multiple class_types)

When a property's `class_types` lists multiple classes, the JSON Schema uses `oneOf` with a discriminator. Each variant includes a `_type` field set to the class's `short_name`:

**Ontology:** property `result` has `class_types: [Success, Failure]`

**Generated JSON Schema:**
```json
{
  "result": {
    "oneOf": [
      {
        "type": "object",
        "properties": {
          "_type": { "type": "string", "const": "Success" },
          "value": { "type": "string" }
        },
        "required": ["_type", "value"]
      },
      {
        "type": "object",
        "properties": {
          "_type": { "type": "string", "const": "Failure" },
          "error": { "type": "string" }
        },
        "required": ["_type", "error"]
      }
    ]
  }
}
```

**Conversion back:** the `_type` field determines which class to use for `is_a`. The `_type` field itself is not stored as a property — it is consumed by the converter.

**Uniqueness requirement:** all classes in `class_types` must have distinct `short_name` values. The `_type` discriminator field name is reserved and must not collide with any property short name on the variant classes.

### 3.7 Arrays

**`value_array`** with `element_type`:
```json
{
  "tags": {
    "type": "array",
    "items": { "type": "string" }
  }
}
```

**`resource_array`** with `class_types`:
```json
{
  "items": {
    "type": "array",
    "items": {
      "type": "object",
      "properties": { ... }
    }
  }
}
```

**`resource_array`** with `allows_only`:
```json
{
  "categories": {
    "type": "array",
    "items": {
      "type": "string",
      "enum": ["cat_a", "cat_b", "cat_c"]
    }
  }
}
```

### 3.8 Property Descriptions

Each JSON Schema property includes a `description` field from the property definition's `description`. This gives the LLM context about what each field means:

```json
{
  "employee_name": {
    "type": "string",
    "description": "Full name of the employee filing the complaint."
  }
}
```

---

## 4. Type-Level Guarantees

CompleteJson is not just a runtime component — it has a **type-level contract** enforced by the Mini-TT type checker during program validation. When the type checker encounters `Apply(CompleteJson, arg)` with `output_schema: C`, it invokes `schema_for_class(C)` and verifies that the class admits a bijective short-name mapping. If it does not, the program is **ill-typed** and rejected before execution.

### 4.1 What the Type Checker Verifies

1. The class `C` exists in the layer chain.
2. All properties in `requires`/`recommends` (including inherited) have a `short_name`.
3. All property short names are unique within scope (no collisions from inheritance).
4. All `allows_only` values have `short_name` and they are distinct.
5. All `class_types` union variants have distinct `short_name`.
6. No circular class references in nested `class_types`.
7. The reserved discriminator `_type` does not collide with any property short name.

A program like:

```esl
let analysis : Analysis = CompleteJson(input, prompt);
```

type-checks only if `Analysis` satisfies all seven conditions. If two inherited properties share a short name, or an `allows_only` value lacks a `short_name`, the program fails validation with a clear error pointing at the ontology conflict.

### 4.2 The Guarantee

The type-level check ensures that for any valid JSON conforming to the generated schema, there exists a **unique, total** conversion back to a well-typed Eigon resource. This is a static guarantee — verified once at program validation time, not re-checked at every execution.

In type-theoretic terms: `schema_for_class(C)` produces a witness that the JSON Schema type and the Eigon resource type for class `C` are **isomorphic** in the category of types with short-name-keyed projections. The type checker verifies the existence of this witness; the runtime relies on it.

### 4.3 Relationship to CIC

The Mini-TT core implements a fragment of the Calculus of Inductive Constructions. Class definitions with `requires` correspond to dependent record types (Sigma types). The schema generation check extends ground type resolution: when resolving `C` as a ground type for CompleteJson, the type checker additionally verifies the bijectivity condition. This is an **intensional** check on the ground type — it inspects the ontology structure, not just the type's identity.

---

## 5. Bijectivity Invariant

The short-name-to-IRI mapping must be **bijective within the scope of a single schema generation**. This means:

1. **Property short names must be unique** across all `requires` and `recommends` of a class (including inherited). If class `Dog` inherits from `Animal`, and both define a property with `short_name: "name"`, that is an error (unless it is the same property IRI).

2. **Enum short names must be unique** within a single `allows_only` set.

3. **Union discriminator short names must be unique** within a single `class_types` set.

4. **The `_type` field** is reserved as the union discriminator. No property in any variant class may have `short_name: "_type"`.

Schema generation **fails with an error** if any uniqueness violation is detected. The error message identifies the conflicting short names and their IRIs.

### 4.1 Short Name Lookup Table

During schema generation, the generator builds a `ShortNameTable`:

```
ShortNameTable {
  properties: Map<short_name, property_iri>,
  enums: Map<(property_iri, short_name), value_iri>,
  unions: Map<(property_iri, short_name), class_iri>,
}
```

This table is passed to the resource converter for the reverse mapping.

---

## 6. Resource Conversion (JSON → Eigon)

Given the LLM's JSON response and the `ShortNameTable`:

1. Create a new embedded `Resource`.
2. Set `is_a` to the target class IRI.
3. For each key-value pair in the JSON object:
   a. Look up the property IRI from `properties[key]`.
   b. Convert the value based on the property's `data_type`:
      - Primitive types: use directly as `Value::String`, `Value::Integer`, etc.
      - `resource` with `allows_only`: look up `enums[(prop_iri, value)]` → resource IRI, store as `Value::String(iri)`.
      - `resource` with `class_types`: recurse into the nested object. If it has a `_type` field, use `unions[(prop_iri, _type_value)]` to determine the class.
      - Arrays: convert each element according to `element_type` or `class_types`.
4. Return the typed resource.

### 5.1 Example Round-Trip

**Ontology:**
```json
[
  {
    "@id": "urn:ex:Analysis",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:short_name": "Analysis",
    "urn:eigenius:core:requires": ["urn:ex:employee_name", "urn:ex:severity", "urn:ex:facts"]
  },
  {
    "@id": "urn:ex:employee_name",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "employee_name",
    "urn:eigenius:core:description": "Full name of the employee.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:string"
  },
  {
    "@id": "urn:ex:severity",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "severity",
    "urn:eigenius:core:description": "Severity level of the complaint.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:resource",
    "urn:eigenius:core:allows_only": ["urn:ex:sev:low", "urn:ex:sev:medium", "urn:ex:sev:high"]
  },
  {
    "@id": "urn:ex:facts",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "facts",
    "urn:eigenius:core:description": "Key facts extracted from the complaint.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:value_array",
    "urn:eigenius:core:element_type": "urn:eigenius:core:string"
  }
]
```

**Generated JSON Schema (sent to LLM):**
```json
{
  "type": "object",
  "properties": {
    "employee_name": {
      "type": "string",
      "description": "Full name of the employee."
    },
    "severity": {
      "type": "string",
      "enum": ["low", "medium", "high"],
      "description": "Severity level of the complaint."
    },
    "facts": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Key facts extracted from the complaint."
    }
  },
  "required": ["employee_name", "severity", "facts"]
}
```

**LLM returns:**
```json
{
  "employee_name": "Jane Smith",
  "severity": "high",
  "facts": ["Workplace safety violation", "Repeated incidents", "No corrective action taken"]
}
```

**Converted Eigon resource:**
```json
{
  "urn:eigenius:core:is_a": ["urn:ex:Analysis"],
  "urn:ex:employee_name": "Jane Smith",
  "urn:ex:severity": "urn:ex:sev:high",
  "urn:ex:facts": ["Workplace safety violation", "Repeated incidents", "No corrective action taken"]
}
```

Note: `severity` was converted from the short name `"high"` back to the full IRI `"urn:ex:sev:high"`.

---

## 7. Component Argument Structure

CompleteJson uses the same `component_argument` pattern as CompleteText, with an additional `output_schema` field specifying the target class:

```json
{
  "urn:eigenius:program:component_argument": {
    "urn:eigenius:program:components:completion:user_prompt": "Extract the employee name, severity, and key facts from this complaint:\n\n{{urn:ex:complaint_text}}",
    "urn:eigenius:program:components:completion:system_prompt": "You are a structured data extractor.",
    "urn:eigenius:program:components:completion:output_schema": "urn:ex:Analysis",
    "urn:eigenius:program:components:completion:request_parameters": {
      "urn:eigenius:program:request:model": "claude-sonnet-4-20250514",
      "urn:eigenius:program:request:temperature": 0.0,
      "urn:eigenius:program:request:max_tokens": 2000
    }
  }
}
```

The `output_schema` IRI is resolved against the layer chain to obtain the class definition. Schema generation walks from there.

---

## 8. Implementation Plan

### 8.1 Where Each Piece Lives

| Concern | Location | Rationale |
|---------|----------|-----------|
| Schema generation | Kernel (`schema.rs`) | Has the layer chain and property resolution |
| Bijectivity check | Kernel (type checker + `schema.rs`) | Verified at program validation time |
| JSON Schema → LLM call | Orchestrator (`complete_json.ts`) | Vercel AI SDK `generateObject()` |
| JSON → Eigon conversion | Kernel (`schema.rs`) | Has the `ShortNameTable`, can validate immediately |

The orchestrator is a thin pass-through: it receives the JSON Schema from the kernel, calls the LLM, and returns the raw simple JSON. **All conversion logic lives in the kernel.**

### 8.2 Execution Flow

1. Kernel executor hits `Apply(CompleteJson, input)` with `component_argument` containing `output_schema: C`.
2. Kernel calls `schema_for_class(C, layer)` → produces `(json_schema, ShortNameTable)`.
3. Kernel serializes the JSON Schema into the `ComponentRequest.argument` alongside the prompt config.
4. Orchestrator receives the request, extracts the JSON Schema, calls `generateObject(schema, prompt)`.
5. Orchestrator returns the raw LLM JSON in `ComponentResponse.output`.
6. Kernel receives the simple JSON, calls `convert_json_to_resource(json, &table, C)` → typed Eigon `Resource`.
7. The resource passes validation (guaranteed by the type-level bijectivity check).

### 8.3 Kernel Modules

**`kernel/src/program/schema.rs`:**

```rust
/// Generate a JSON Schema and short-name lookup table for a class.
/// Fails if the bijectivity invariant is violated.
pub fn schema_for_class(
    class_iri: &Iri,
    layer: &Layer,
) -> Result<(serde_json::Value, ShortNameTable), SchemaError>

/// Convert a simple JSON object (short-name keys) back to an Eigon Resource
/// using the ShortNameTable. Infallible if the JSON conforms to the schema
/// (guaranteed by generateObject).
pub fn convert_json_to_resource(
    json: &serde_json::Value,
    table: &ShortNameTable,
    class_iri: &Iri,
) -> Result<Resource, ConversionError>
```

This module is useful beyond CompleteJson — any component needing JSON Schema from an ontology class can use it.

### 8.4 Type Checker Integration

Extend `kernel/src/program/ground.rs` or add a validation pass in `kernel/src/program/expr.rs`:

When type-checking `Apply(CompleteJson, ...)`, extract `output_schema` from the `component_argument` and call `schema_for_class`. If it returns `Err`, the program is ill-typed.

### 8.5 Orchestrator Component

`orchestration/src/components/complete_json.ts`:

Simpler than CompleteText — receives the JSON Schema in the argument, calls `generateObject()`, returns the raw JSON. No short-name table needed on the orchestrator side.

### 8.6 Proto Addition

```proto
// Optional: expose schema generation as a standalone RPC
// for tooling and debugging (not required for the execution flow).
rpc GetSchema(GetSchemaRequest) returns (GetSchemaResponse);

message GetSchemaRequest {
  string class_iri = 1;
}

message GetSchemaResponse {
  bool success = 1;
  string json_schema = 2;       // JSON Schema as JSON string
  string error = 3;
}
```

### 8.7 Steps

1. Implement `ShortNameTable` and `schema_for_class` in the kernel (property collection, constraint mapping, enum/union handling, uniqueness checks)
2. Implement `convert_json_to_resource` in the kernel
3. Integrate bijectivity check into the type checker
4. Pass JSON Schema through `ComponentRequest.argument` to orchestrator
5. Implement `complete_json.ts` in orchestrator (extract schema, call `generateObject`, return raw JSON)
6. Wire kernel-side conversion in the executor after receiving the component response
7. Add `GetSchema` RPC to proto and kernel server (for tooling)
8. End-to-end test: define a class with enums and nested objects, run a program that uses CompleteJson, verify the output is a properly typed Eigon resource with correct IRI-keyed properties

---

## 9. Edge Cases and Limitations

### 8.1 Properties Without short_name

Every property used in schema generation must have a `short_name`. Properties without one are skipped with a warning. This should not happen for well-formed ontologies (short_name is required on Property).

### 8.2 Deeply Nested Structures

Recursion depth is bounded at 4 levels. Deeper nesting produces `{ "type": "object" }` without further schema constraints. This is a practical limit — LLMs handle flat and shallow structures better than deeply nested ones.

### 8.3 Self-Referential Classes

A class whose `requires` includes a property with `class_types` pointing back to the same class is a circular reference. Detected during schema generation and rejected with an error.

### 8.4 LLM Hallucinated Keys

The LLM may return JSON keys not in the schema. These are silently ignored during conversion — they have no mapping in the `ShortNameTable`.

### 8.5 Missing Required Fields

If the LLM omits a required field, `generateObject()` will retry or fail. The schema's `required` array enforces this at the Vercel AI SDK level before Eigenius sees the result.

---

## 10. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Schema generation location | Kernel (via GetSchema RPC) | Kernel has layer chain + property resolution; avoid duplicating in orchestrator |
| Short name collisions | Fail with error | Silent resolution would break bijectivity; ontology author must fix |
| Union discriminator | `_type` field | Conventional, JSON Schema oneOf compatible, reserved name |
| Recursion depth | 4 levels | LLM practical limit; deeper structures should be decomposed |
| Circular references | Rejected | Cannot produce finite JSON Schema |
| Extra LLM keys | Silently ignored | Defensive; don't fail on LLM over-generation |
| Format for ShortNameTable over wire | CBOR | Consistent with other kernel → orchestrator data |
