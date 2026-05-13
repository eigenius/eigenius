// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/**
 * Decode an Eigon-CBOR program-trace resource (D11 / kernel/src/program/trace.rs)
 * into a renderable hierarchy. Each trace variant maps to a node with
 * a `kind` (LetTrace, ComponentTrace, PureTrace, MapTrace, ReduceTrace,
 * CaseTrace, ConstructTrace, ...) plus a short label and any attached
 * children.
 */

import { decode as cborDecode } from "cbor-x";

const IS_A = "urn:eigenius:core:is_a";
const NS = "urn:eigenius:reflection";

const ID = "@id";

const PROP = {
  name: `${NS}:name`,
  component: `${NS}:component`,
  output: `${NS}:output`,
  cached: `${NS}:cached`,
  inputHash: `${NS}:input_hash`,
  argumentHash: `${NS}:argument_hash`,
  provider: `${NS}:provider`,
  model: `${NS}:model`,
  promptTokens: `${NS}:prompt_tokens`,
  completionTokens: `${NS}:completion_tokens`,
  latencyMs: `${NS}:latency_ms`,
  branchTaken: `${NS}:branch_taken`,
  // ProgramTrace metadata + root-trace embedding
  program: `${NS}:program`,
  startedAt: `${NS}:started_at`,
  completedAt: `${NS}:completed_at`,
  totalTokens: `${NS}:total_tokens`,
  executedSteps: `${NS}:executed_steps`,
  traceTree: `${NS}:trace_tree`,
  // child-trace edges
  valueTrace: `${NS}:value_trace`,
  bodyTrace: `${NS}:body_trace`,
  scrutineeTrace: `${NS}:scrutinee_trace`,
  branchTrace: `${NS}:branch_trace`,
  sourceTrace: `${NS}:source_trace`,
  elementTraces: `${NS}:element_traces`,
  stepTraces: `${NS}:step_traces`,
  fieldTraces: `${NS}:field_traces`,
  argumentTrace: `${NS}:argument_trace`,
} as const;

export interface TraceNode {
  /** Last segment of the is_a IRI: "LetTrace", "ComponentTrace", … */
  kind: string;
  /** Short label for display (typically the bound name or component IRI). */
  label: string;
  /** Free-form key/value summary lines (token counts, latency, etc.). */
  summary: readonly { key: string; value: string }[];
  children: TraceNode[];
}

interface CborResource {
  [key: string]: unknown;
}

function isResource(v: unknown): v is CborResource {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function localKind(resource: CborResource): string {
  const tags = resource[IS_A];
  if (!Array.isArray(tags)) return "Trace";
  const first = tags.find((t): t is string => typeof t === "string");
  if (!first) return "Trace";
  const colon = first.lastIndexOf(":");
  return colon >= 0 ? first.slice(colon + 1) : first;
}

function shortenIri(iri: string): string {
  const last = iri.lastIndexOf(":");
  if (last < 0) return iri;
  const local = iri.slice(last + 1);
  const before = iri.slice(0, last);
  const prev = before.lastIndexOf(":");
  return prev >= 0
    ? `${before.slice(prev + 1)}:${local}`
    : `${before}:${local}`;
}

function asStr(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function buildNode(resource: CborResource): TraceNode {
  const kind = localKind(resource);
  const summary: { key: string; value: string }[] = [];
  let label = "";
  const children: TraceNode[] = [];

  switch (kind) {
    case "ProgramTrace": {
      const program = asStr(resource[PROP.program]);
      label = program ? shortenIri(program) : "program";
      const traceIri = asStr(resource[ID]);
      if (traceIri) summary.push({ key: "trace", value: traceIri });
      const totalTokens = resource[PROP.totalTokens];
      const executedSteps = resource[PROP.executedSteps];
      if (typeof totalTokens === "number") {
        summary.push({ key: "tokens", value: String(totalTokens) });
      }
      if (typeof executedSteps === "number") {
        summary.push({ key: "steps", value: String(executedSteps) });
      }
      const startedAt = asStr(resource[PROP.startedAt]);
      const completedAt = asStr(resource[PROP.completedAt]);
      if (startedAt) summary.push({ key: "started", value: startedAt });
      if (completedAt) summary.push({ key: "completed", value: completedAt });
      if (startedAt && completedAt) {
        const dur = Date.parse(completedAt) - Date.parse(startedAt);
        if (Number.isFinite(dur)) {
          summary.push({ key: "duration", value: `${dur}ms` });
        }
      }
      // The kernel emits `Trace::Let { value, body }` as a right-leaning
      // AST: `let A = e1 in (let B = e2 in e3)` becomes a Let nested
      // inside a Let inside the body. That reads as "B is inside A"
      // which is the inverse of dataflow expectation. Flatten the chain
      // into siblings so each binding's evaluation reads top-to-bottom.
      pushFlattenedLetChain(resource[PROP.traceTree], children);
      break;
    }
    case "LetTrace": {
      // Same flattening as ProgramTrace: a Let that's encountered as
      // a child (e.g., inside a CaseTrace branch) still benefits from
      // siblings-not-nested rendering. The current Let becomes the
      // first emitted node; the (possibly chained) body unrolls beside
      // it as later siblings of the parent.
      const name = asStr(resource[PROP.name]) ?? "(let)";
      label = `let ${name}`;
      pushChild(resource[PROP.valueTrace], "= value", children);
      // Walk the body chain inline; each Let in the chain becomes a
      // sibling node appended to `children` rather than a deeper nest.
      let cursor: CborResource | undefined = (() => {
        const b = resource[PROP.bodyTrace];
        return isResource(b) ? b : undefined;
      })();
      while (cursor && localKind(cursor) === "LetTrace") {
        const innerName = asStr(cursor[PROP.name]) ?? "(let)";
        const innerValueRes = cursor[PROP.valueTrace];
        const innerLetChildren: TraceNode[] = [];
        if (isResource(innerValueRes)) {
          innerLetChildren.push(buildNode(innerValueRes));
        }
        children.push({
          kind: "LetTrace",
          label: `let ${innerName}`,
          summary: [],
          children: innerLetChildren,
        });
        const next = cursor[PROP.bodyTrace];
        cursor = isResource(next) ? next : undefined;
      }
      if (cursor) children.push(buildNode(cursor));
      break;
    }
    case "ComponentTrace": {
      const component = asStr(resource[PROP.component]) ?? "(component)";
      label = shortenIri(component);
      const cached = resource[PROP.cached];
      if (cached === true) summary.push({ key: "cached", value: "true" });
      const provider = asStr(resource[PROP.provider]);
      const model = asStr(resource[PROP.model]);
      if (provider || model) {
        summary.push({
          key: "via",
          value: [provider, model].filter(Boolean).join(" / "),
        });
      }
      const inT = resource[PROP.promptTokens];
      const outT = resource[PROP.completionTokens];
      if (typeof inT === "number" || typeof outT === "number") {
        summary.push({
          key: "tokens",
          value: `${inT ?? "?"} in / ${outT ?? "?"} out`,
        });
      }
      const latency = resource[PROP.latencyMs];
      if (typeof latency === "number") {
        summary.push({ key: "latency", value: `${latency}ms` });
      }
      const inputHash = asStr(resource[PROP.inputHash]);
      if (inputHash) {
        summary.push({
          key: "input_hash",
          value: inputHash.slice(0, 16) + "…",
        });
      }
      const argumentHash = asStr(resource[PROP.argumentHash]);
      if (argumentHash) {
        summary.push({
          key: "argument_hash",
          value: argumentHash.slice(0, 16) + "…",
        });
      }
      pushChild(resource[PROP.argumentTrace], "argument", children);
      break;
    }
    case "PureTrace": {
      const component = asStr(resource[PROP.component]) ?? "(pure)";
      label = `pure ${shortenIri(component)}`;
      break;
    }
    case "MapTrace": {
      const traces = resource[PROP.elementTraces];
      label = Array.isArray(traces) ? `map (${traces.length})` : "map";
      pushChildArray(traces, "element", children);
      break;
    }
    case "ReduceTrace": {
      const traces = resource[PROP.stepTraces];
      label = Array.isArray(traces) ? `reduce (${traces.length})` : "reduce";
      pushChildArray(traces, "step", children);
      break;
    }
    case "CaseTrace": {
      const branch = asStr(resource[PROP.branchTaken]);
      label = `case → ${branch ?? "?"}`;
      pushChild(resource[PROP.scrutineeTrace], "scrutinee", children);
      pushChild(resource[PROP.branchTrace], "branch", children);
      break;
    }
    case "ConstructTrace": {
      label = "construct";
      const fields = resource[PROP.fieldTraces];
      pushChildArray(fields, "field", children);
      break;
    }
    default: {
      // Generic fallback — pick up any *_trace embedded, surface key
      // string properties as summary lines.
      label = kind;
      for (const [k, v] of Object.entries(resource)) {
        if (k === IS_A || k === "@id") continue;
        if (k.endsWith("_trace") && isResource(v)) {
          children.push({ ...buildNode(v), label: shortenIri(k) });
        } else if (k.endsWith("_traces") && Array.isArray(v)) {
          pushChildArray(v, shortenIri(k), children);
        } else if (typeof v === "string" || typeof v === "number") {
          summary.push({ key: shortenIri(k), value: String(v) });
        }
      }
    }
  }

  return { kind, label, summary, children };
}

function pushChild(
  value: unknown,
  fallbackLabel: string,
  out: TraceNode[],
): void {
  if (!isResource(value)) return;
  const child = buildNode(value);
  if (!child.label) child.label = fallbackLabel;
  out.push(child);
}

function pushChildArray(
  array: unknown,
  fallbackLabelPrefix: string,
  out: TraceNode[],
): void {
  if (!Array.isArray(array)) return;
  array.forEach((entry, idx) => {
    if (!isResource(entry)) return;
    const child = buildNode(entry);
    if (!child.label) child.label = `${fallbackLabelPrefix} ${idx}`;
    out.push(child);
  });
}

/**
 * Walk a (possibly singleton) Let-chain and emit one sibling per
 * binding plus the final non-Let body. Each emitted Let has the value
 * trace as its single child; the final body is appended as itself.
 *
 * If `value` is not a LetTrace, falls back to a single child via
 * `pushChild`. If `value` is missing, no-ops.
 */
function pushFlattenedLetChain(value: unknown, out: TraceNode[]): void {
  if (!isResource(value)) return;
  if (localKind(value) !== "LetTrace") {
    out.push(buildNode(value));
    return;
  }
  let cursor: CborResource | undefined = value;
  while (cursor && localKind(cursor) === "LetTrace") {
    const name = asStr(cursor[PROP.name]) ?? "(let)";
    const valueRes = cursor[PROP.valueTrace];
    const letChildren: TraceNode[] = [];
    if (isResource(valueRes)) {
      letChildren.push(buildNode(valueRes));
    }
    out.push({
      kind: "LetTrace",
      label: `let ${name}`,
      summary: [],
      children: letChildren,
    });
    const next: unknown = cursor[PROP.bodyTrace];
    cursor = isResource(next) ? next : undefined;
  }
  if (cursor) {
    out.push(buildNode(cursor));
  }
}

/**
 * Decode a CBOR-encoded trace resource into a TraceNode tree.
 */
export function decodeTraceResource(bytes: Uint8Array): TraceNode {
  const decoded = cborDecode(bytes) as unknown;
  if (!isResource(decoded)) {
    throw new Error("trace document did not decode to a resource");
  }
  return buildNode(decoded);
}
