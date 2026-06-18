# Patent Analysis Demo

This demo takes a patent document and produces a structured analysis plus a
plain-language summary, all type-checked end to end. It exercises two LLM
components in sequence — **CompleteJson** for structured extraction and
**CompleteText** for narrative generation — and combines their results into a
single typed output.

The patent used is US10452978B2: "Attention-based sequence transduction neural
networks" (Google LLC, 2018) — the patent behind the *Attention Is All You
Need* paper that introduced the Transformer architecture.

## Running the demo

Start the kernel and orchestrator first (Docker Compose or three terminals —
see the top-level README), then:

```bash
./demo/patent/run.sh
```

The script performs three steps:

1. **Load the patent ontology** — the ESL file that defines the types
2. **Load the patent document** — a JSON resource describing the patent
3. **Run the analysis pipeline** — the ESL program that chains two LLM calls

With a real API key the pipeline takes around 10-20 seconds (two LLM round
trips). In mock mode it returns placeholder text instantly.

## The files

### patent-ontology.esl — Type definitions

This file defines three classes and their properties:

```
PatentClaim
  requires: title, patent_number, abstract_text
  recommends: assignee, filing_date

PatentAnalysis
  requires: invention_category, technical_domain, key_innovations, practical_applications
  recommends: prior_art_references, limitations

PatentBrief
  requires: summary, analysis
```

**`requires`** means a resource of that class *must* have these properties.
**`recommends`** means they are expected but optional. The type system treats
required properties as guaranteed and recommended properties as optional
values (present or absent).

Each property has a data type. Most are `core:string`. The list properties
(`key_innovations`, `practical_applications`, etc.) use `core:value_array`
with `element_type = core:string` — a typed list of strings. The `analysis`
property on `PatentBrief` has type `core:resource` with
`class_types patent:PatentAnalysis`, meaning it holds a nested resource that
must conform to the `PatentAnalysis` class.

### transformer-patent.json — Input data

A JSON resource describing the Transformer patent:

```json
{
  "@id": "urn:eigenius:demo:patent:US10452978B2",
  "urn:eigenius:core:is_a": ["urn:eigenius:demo:patent:PatentClaim"],
  "urn:eigenius:demo:patent:title": "Attention-based sequence transduction neural networks",
  "urn:eigenius:demo:patent:patent_number": "US10452978B2",
  "urn:eigenius:demo:patent:assignee": "Google LLC",
  "urn:eigenius:demo:patent:filing_date": "2018-06-28",
  "urn:eigenius:demo:patent:abstract_text": "Methods, systems, and apparatus..."
}
```

Every resource in Eigenius has an IRI identity (`@id`) and declares its class
via `is_a`. All property keys are full IRIs — there are no implicit names that
could collide across domains.

### analyze-patent.esl — The program

```esl
program patent:analyze_patent : patent:PatentClaim -> patent:PatentBrief {
    // Step 1: Extract structured analysis via CompleteJson
    let analysis : patent:PatentAnalysis = CompleteJson(input) {
        completion:user_prompt = "Analyze this patent...
            Title: {{urn:eigenius:demo:patent:title}}
            Patent Number: {{urn:eigenius:demo:patent:patent_number}}
            Abstract: {{urn:eigenius:demo:patent:abstract_text}}";
        completion:system_prompt = "You are a patent analyst...";
        completion:output_schema = patent:PatentAnalysis;
        completion:request_parameters = {
            request:model = "claude-sonnet-4-6";
            request:temperature = 0.0;
            request:max_tokens = 2000;
        };
    };

    // Step 2: Generate plain-language summary
    let summary : core:string = CompleteText(analysis) {
        completion:user_prompt = "Based on this patent analysis...
            Category: {{urn:eigenius:demo:patent:invention_category}}
            Domain: {{urn:eigenius:demo:patent:technical_domain}}
            Key Innovations: {{urn:eigenius:demo:patent:key_innovations}}
            Applications: {{urn:eigenius:demo:patent:practical_applications}}";
        completion:system_prompt = "You are a technical writer...";
        completion:request_parameters = {
            request:model = "claude-sonnet-4-6";
            request:temperature = 0.3;
            request:max_tokens = 1000;
        };
    };

    // Step 3: Combine into the final brief
    Construct patent:PatentBrief {
        patent:summary = summary,
        patent:analysis = analysis
    }
}
```

The program signature `patent:PatentClaim -> patent:PatentBrief` declares that
it takes a `PatentClaim` and produces a `PatentBrief`. Inside, the variable
`input` is bound to the incoming patent resource.

**Step 1** calls `CompleteJson` — an LLM component that returns structured
JSON conforming to a schema. The `output_schema = patent:PatentAnalysis` tells
the system to generate a JSON Schema from the `PatentAnalysis` class
definition and instruct the LLM to return data matching that schema. The
result is bound to `analysis` with type `patent:PatentAnalysis`.

**Step 2** calls `CompleteText` — an LLM component that returns plain text.
It receives the structured `analysis` as input and produces a narrative
summary. The result is bound to `summary` with type `core:string`.

**Step 3** uses `Construct` to build the final `PatentBrief` resource from
the two intermediate results.

### Prompt templates

The `{{...}}` markers in prompt strings are **template references**. They
name properties that must exist on the resource being passed to the component.
For example, the Step 1 prompt references `{{urn:eigenius:demo:patent:title}}`
— at runtime, this is replaced with the actual title from the input patent
document.

The type system validates that the referenced properties are available on the
input type. If you wrote `{{urn:eigenius:demo:patent:nonexistent}}`, the
system would flag that the property doesn't exist on `PatentClaim`. This
catches prompt/data mismatches before the program runs.

## Example output

Running the demo against the Transformer patent produces output like this
(see `example-output.json` for the full result):

```json
{
  "urn:eigenius:core:is_a": [
    "urn:eigenius:demo:patent:PatentBrief"
  ],
  "urn:eigenius:demo:patent:analysis": {
    "urn:eigenius:core:is_a": [
      "urn:eigenius:demo:patent:PatentAnalysis"
    ],
    "urn:eigenius:demo:patent:invention_category": "system",
    "urn:eigenius:demo:patent:key_innovations": [
      "Attention-based sequence transduction without recurrent or convolutional layers",
      "Encoder neural network with sequence of encoder subnetworks",
      "Encoder self-attention sub-layer mechanism",
      "Query-based attention mechanism over encoder subnetwork inputs",
      "Transformer architecture for sequence-to-sequence tasks",
      "Parallelizable attention mechanism replacing sequential processing"
    ],
    "urn:eigenius:demo:patent:limitations": [
      "Computational complexity scales quadratically with sequence length",
      "Memory requirements increase significantly for long sequences",
      "Requires large amounts of training data for optimal performance"
    ],
    "urn:eigenius:demo:patent:practical_applications": [
      "Natural language processing and translation",
      "Large language model architectures (GPT, BERT, PaLM, Claude)",
      "Sequence-to-sequence learning tasks",
      "Text generation and understanding",
      "Machine translation systems",
      "Language modeling applications"
    ],
    "urn:eigenius:demo:patent:prior_art_references": [
      "Recurrent neural networks for sequence processing",
      "Convolutional neural networks for sequence tasks",
      "Traditional attention mechanisms in neural networks"
    ],
    "urn:eigenius:demo:patent:technical_domain": "artificial intelligence and machine learning"
  },
  "urn:eigenius:demo:patent:summary": "This patent describes a breakthrough..."
}
```

Every field uses a full IRI as its key. The `is_a` declaration marks the
result as a `PatentAnalysis`. The required fields (`invention_category`,
`technical_domain`, `key_innovations`, `practical_applications`) are all
present. The recommended fields (`prior_art_references`, `limitations`) were
also filled in by the LLM — but the system would accept a result without them.

## What happens under the hood

### 1. ESL compilation

When you write `.esl` files, the CLI compiles them to **Eigon-JSON** before
sending them to the kernel. Eigon-JSON is the canonical data format — every
resource, class definition, property, and program is represented as a JSON
object with IRI keys.

The ontology file `patent-ontology.esl` compiles to 16 Eigon-JSON resources:
3 class definitions, 11 property definitions, and 2 namespace declarations
that get folded into the IRI prefixes. For example, the class declaration:

```esl
class patent:PatentClaim {
    description = "A patent claim or abstract to be analyzed.";
    requires patent:title, patent:patent_number, patent:abstract_text;
    recommends patent:assignee, patent:filing_date;
}
```

becomes:

```json
{
  "@id": "urn:eigenius:demo:patent:PatentClaim",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
  "urn:eigenius:core:description": "A patent claim or abstract to be analyzed.",
  "urn:eigenius:core:requires": [
    "urn:eigenius:demo:patent:title",
    "urn:eigenius:demo:patent:patent_number",
    "urn:eigenius:demo:patent:abstract_text"
  ],
  "urn:eigenius:core:recommends": [
    "urn:eigenius:demo:patent:assignee",
    "urn:eigenius:demo:patent:filing_date"
  ],
  "urn:eigenius:core:short_name": "PatentClaim"
}
```

The program `analyze-patent.esl` compiles to a single Eigon-JSON resource
with nested expression nodes (see the [Appendix](#appendix-compiled-eigon-json)
for the full output). Each ESL expression (`let`, function call, `Construct`)
maps to a corresponding expression node type (`Let`, `Apply`, `Construct`)
from the program ontology.

### 2. Loading into the kernel

When the kernel receives a `load` request, it:

1. **Parses** the resources from Eigon-JSON (or compiles from ESL first)
2. **Validates** each resource against the ontology — checking required
   properties, data types, constraints, and class conformance
3. **Commits** the resources as a new immutable **layer** in the knowledge
   graph, returning a content-addressed layer ID (SHA-256 hash)

Layers are immutable and form a chain via parent pointers. The kernel starts
with three bootstrap layers (core ontology, program ontology, reflection
ontology). Each `load` adds a new layer on top. Resolution walks the chain
from top to bottom, so later layers can extend or shadow earlier definitions.

### 3. Type checking

Before execution, the kernel type-checks the program. The type system is
based on **dependent types** — a technique from programming language theory
where types can depend on values. In practice this means:

- The program signature `PatentClaim -> PatentBrief` is checked: the input
  type and output type must be valid classes in the ontology.
- Each `let` binding is checked: the declared type must match what the
  expression actually produces. `CompleteJson(input)` with
  `output_schema = PatentAnalysis` produces a `PatentAnalysis`, and that must
  match the declared type `patent:PatentAnalysis`.
- The `Construct` at the end builds a `PatentBrief`, which requires `summary`
  (string) and `analysis` (PatentAnalysis). The type checker verifies that
  `summary` is indeed a string and `analysis` is indeed a `PatentAnalysis`.

**Where dependent types matter:** The prompt templates create a connection
between the *type* of data flowing through the program and the *content* of
the prompts. The template `"Title: {{urn:eigenius:demo:patent:title}}"` is a
**template type** — a type that carries information about which properties it
references. The type checker uses this to verify that the input resource
(of type `PatentClaim`) actually has a `title` property. If the template
referenced a property that doesn't exist on the input type, type checking
would fail.

This is the key safety property: the system guarantees at type-check time
that every `{{...}}` reference in every prompt will resolve to an actual
property at runtime. No "undefined variable" surprises when the LLM call
happens.

### 4. Components and the orchestrator

`CompleteJson` and `CompleteText` are not built into the kernel. They are
**components** — extension points that run in the orchestrator (a separate
Deno/TypeScript service). The kernel knows about them through the program
ontology, which declares:

- **CompleteText**: takes a resource, returns a string. Uses the Anthropic API
  via the Vercel AI SDK to generate text.
- **CompleteJson**: takes a resource, returns a structured resource. Uses the
  Anthropic API with a JSON Schema constraint so the LLM returns structured
  data matching the target class.

When the kernel encounters a component call during execution, it:

1. Serializes the input resource and component arguments to JSON
2. Sends a gRPC request to the orchestrator's `ComponentExecutor` service
3. The orchestrator routes the request to the appropriate handler
4. The handler calls the LLM API, collects the response and usage metrics
5. The response comes back to the kernel as JSON

For `CompleteJson` specifically, the kernel generates a **JSON Schema** from
the target class (`PatentAnalysis`) and includes it in the component
arguments. The orchestrator passes this schema to the LLM as a structured
output constraint. The LLM returns short-name JSON keys (`invention_category`
instead of the full IRI), and the kernel maps them back to IRIs using the
schema's name table.

This architecture means the kernel stays focused on types, validation, and
program logic, while the orchestrator handles external I/O. New components
can be added to the orchestrator without changing the kernel.

### 5. Program execution

The kernel executes programs using an evaluator based on **Normalization by
Evaluation** (NbE) — a technique where programs are evaluated by
alternating between two representations:

- **Syntax** (expressions): the program as written — `let`, function calls,
  variables
- **Values** (semantic domain): the results of evaluation — actual resources,
  strings, numbers

The evaluator walks the expression tree. When it hits a `let` binding, it
evaluates the right-hand side, binds the result to the variable name, and
continues with the body. When it hits an `Apply` (function call) to a
component like `CompleteJson`, it dispatches the call to the orchestrator
and waits for the result.

For the patent demo, execution proceeds as:

1. `input` is bound to the patent document resource
2. `CompleteJson(input)` dispatches to the orchestrator with the patent data
   and the `PatentAnalysis` schema. The LLM returns structured JSON. The
   kernel converts it to a typed `PatentAnalysis` resource and binds it to
   `analysis`.
3. `CompleteText(analysis)` dispatches again, this time with the structured
   analysis. The template references (`{{invention_category}}`, etc.) are
   filled from the analysis resource. The LLM returns narrative text, bound
   to `summary`.
4. `Construct PatentBrief { summary, analysis }` assembles the final resource.

The kernel also records a **trace** for each component call — which component
was called, what input it received, what output it produced, and LLM usage
metrics (tokens, latency). These traces are committed as resources in the
knowledge graph, providing a full audit trail.

## Appendix: Compiled Eigon-JSON

The ESL program `analyze-patent.esl` compiles to this Eigon-JSON
representation. This is what the kernel actually processes — ESL is syntactic
sugar over this structure.

```json
[
  {
    "@id": "urn:eigenius:demo:patent:analyze_patent",
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
    "urn:eigenius:program:input_type": "urn:eigenius:demo:patent:PatentClaim",
    "urn:eigenius:program:output_type": "urn:eigenius:demo:patent:PatentBrief",
    "urn:eigenius:program:body": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
      "urn:eigenius:program:name": "analysis",
      "urn:eigenius:program:type": "urn:eigenius:demo:patent:PatentAnalysis",
      "urn:eigenius:program:value": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
        "urn:eigenius:program:function": "urn:eigenius:program:components:CompleteJson",
        "urn:eigenius:program:argument": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
          "urn:eigenius:program:name": "input"
        },
        "urn:eigenius:program:component_argument": {
          "urn:eigenius:program:components:completion:user_prompt":
            "Analyze this patent...\n\nTitle: {{urn:eigenius:demo:patent:title}}\nPatent Number: {{urn:eigenius:demo:patent:patent_number}}\n\nAbstract:\n{{urn:eigenius:demo:patent:abstract_text}}",
          "urn:eigenius:program:components:completion:system_prompt":
            "You are a patent analyst...",
          "urn:eigenius:program:components:completion:output_schema":
            "urn:eigenius:demo:patent:PatentAnalysis",
          "urn:eigenius:program:components:completion:request_parameters": {
            "urn:eigenius:program:request:model": "claude-sonnet-4-6",
            "urn:eigenius:program:request:temperature": 0.0,
            "urn:eigenius:program:request:max_tokens": 2000
          }
        }
      },
      "urn:eigenius:program:body": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
        "urn:eigenius:program:name": "summary",
        "urn:eigenius:program:type": "urn:eigenius:core:string",
        "urn:eigenius:program:value": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
          "urn:eigenius:program:function": "urn:eigenius:program:components:CompleteText",
          "urn:eigenius:program:argument": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "analysis"
          },
          "urn:eigenius:program:component_argument": {
            "urn:eigenius:program:components:completion:user_prompt":
              "Based on this patent analysis...\n\nCategory: {{urn:eigenius:demo:patent:invention_category}}\nDomain: {{urn:eigenius:demo:patent:technical_domain}}\nKey Innovations: {{urn:eigenius:demo:patent:key_innovations}}\nApplications: {{urn:eigenius:demo:patent:practical_applications}}",
            "urn:eigenius:program:components:completion:system_prompt":
              "You are a technical writer...",
            "urn:eigenius:program:components:completion:request_parameters": {
              "urn:eigenius:program:request:model": "claude-sonnet-4-6",
              "urn:eigenius:program:request:temperature": 0.3,
              "urn:eigenius:program:request:max_tokens": 1000
            }
          }
        },
        "urn:eigenius:program:body": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
          "urn:eigenius:program:class": "urn:eigenius:demo:patent:PatentBrief",
          "urn:eigenius:program:fields": {
            "urn:eigenius:demo:patent:summary": {
              "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
              "urn:eigenius:program:name": "summary"
            },
            "urn:eigenius:demo:patent:analysis": {
              "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
              "urn:eigenius:program:name": "analysis"
            }
          }
        }
      }
    }
  }
]
```

Key observations:

- **Everything is a resource.** The program itself is a resource with
  `is_a: Program`. Each expression node (`Let`, `Apply`, `Var`, `Construct`)
  is an embedded resource with its own `is_a` declaration.
- **Nesting mirrors scope.** The `Let` nodes nest: the outer `Let` (binding
  `analysis`) contains an inner `Let` (binding `summary`), which contains
  the `Construct`. This mirrors how variable scoping works — `analysis` is
  visible in the inner `Let` and the `Construct`.
- **Components are references, not calls.** `CompleteJson` appears as
  `"urn:eigenius:program:components:CompleteJson"` — a string reference to a
  component defined in the program ontology. The kernel looks it up and
  dispatches to the orchestrator.
- **Component arguments are data, not code.** The `component_argument` block
  holds the prompts, schema reference, and model parameters as plain data
  values. This is what gets sent to the orchestrator alongside the input
  resource.
- **Template strings carry IRI references.** The `{{urn:eigenius:demo:patent:title}}`
  markers use full IRIs, making the connection between prompts and data
  explicit and machine-checkable.
