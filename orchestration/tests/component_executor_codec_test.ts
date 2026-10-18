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
 * Phase 18e codec branching tests for `executeComponentRequest`.
 *
 * The kernel ↔ orchestrator path is end-to-end Eigon-CBOR by default,
 * with a JSON fallback for backward compatibility during a rolling
 * deploy where the kernel and orchestrator might be at different
 * Phase-18e adoption levels. These tests pin both branches:
 *
 * - **CBOR happy path** — `content_type = "application/eigon+cbor"`
 *   round-trips through `encodeResource` / `decodeResource`.
 * - **JSON fallback** — `content_type = "application/eigon+json"`
 *   round-trips through `JSON.parse` / `JSON.stringify`.
 * - **Empty content_type** — pre-18e clients didn't set the field;
 *   treated as JSON for safety.
 *
 * The third one is the load-bearing regression guard: if a future
 * refactor removes the JSON branch, an unupgraded kernel suddenly
 * sees decode failures in production. This test fires first.
 */

import { assertEquals } from "@std/assert";
import { create } from "@bufbuild/protobuf";
import {
  type ComponentRequest,
  ComponentRequestSchema,
} from "../src/gen/eigenius_pb.ts";
import { executeComponentRequest } from "../src/server/component_executor.ts";
import {
  type ComponentHandler,
  type ComponentInput,
  ComponentRegistry,
} from "../src/components/registry.ts";
import { decodeResource, encodeResource } from "../src/codec/cbor.ts";

const ECHO_IRI = "urn:test:components:CodecEcho";
const TEXT_DECODER = new TextDecoder();
const TEXT_ENCODER = new TextEncoder();

/** Test handler: surfaces the input + argument shape so the test can
 * confirm what the executor handed it. */
function echoHandler(): ComponentHandler {
  return (req: ComponentInput) =>
    Promise.resolve({
      output: {
        "urn:test:saw_input": req.input,
        "urn:test:saw_argument": req.argument,
      },
    });
}

function buildRegistry(): ComponentRegistry {
  const registry = new ComponentRegistry();
  registry.register(ECHO_IRI, echoHandler());
  return registry;
}

const noopMark = { fail(_: string) {} };

function buildRequest(
  contentType: string,
  input: Uint8Array,
  argument: Uint8Array,
): ComponentRequest {
  return create(ComponentRequestSchema, {
    componentIri: ECHO_IRI,
    contentType,
    input,
    argument,
  });
}

Deno.test("executeComponentRequest CBOR path round-trips", async () => {
  const registry = buildRegistry();
  const inputObj = { "urn:test:k": "input-value" };
  const argumentObj = { "urn:test:k": "arg-value" };
  const req = buildRequest(
    "application/eigon+cbor",
    encodeResource(inputObj),
    encodeResource(argumentObj),
  );

  const resp = await executeComponentRequest(req, registry, noopMark);
  assertEquals(resp.success, true);
  const decoded = decodeResource(resp.output);
  assertEquals(decoded["urn:test:saw_input"], inputObj);
  assertEquals(decoded["urn:test:saw_argument"], argumentObj);
});

Deno.test(
  "executeComponentRequest JSON fallback decodes/encodes via JSON when content_type is application/eigon+json",
  async () => {
    const registry = buildRegistry();
    const inputObj = { "urn:test:k": "input-value" };
    const argumentObj = { "urn:test:k": "arg-value" };
    const req = buildRequest(
      "application/eigon+json",
      TEXT_ENCODER.encode(JSON.stringify(inputObj)),
      TEXT_ENCODER.encode(JSON.stringify(argumentObj)),
    );

    const resp = await executeComponentRequest(req, registry, noopMark);
    assertEquals(resp.success, true);
    // JSON branch encodes output as JSON, not CBOR.
    const decoded = JSON.parse(TEXT_DECODER.decode(resp.output));
    assertEquals(decoded["urn:test:saw_input"], inputObj);
    assertEquals(decoded["urn:test:saw_argument"], argumentObj);
  },
);

Deno.test(
  "executeComponentRequest treats empty content_type as JSON (pre-18e clients)",
  async () => {
    const registry = buildRegistry();
    const inputObj = { "urn:test:k": "v" };
    const argumentObj = {};
    const req = buildRequest(
      "",
      TEXT_ENCODER.encode(JSON.stringify(inputObj)),
      TEXT_ENCODER.encode(JSON.stringify(argumentObj)),
    );

    const resp = await executeComponentRequest(req, registry, noopMark);
    assertEquals(resp.success, true);
    // Should be JSON-encoded since fallback path was taken.
    const decoded = JSON.parse(TEXT_DECODER.decode(resp.output));
    assertEquals(decoded["urn:test:saw_input"], inputObj);
    assertEquals(decoded["urn:test:saw_argument"], argumentObj);
  },
);

Deno.test(
  "executeComponentRequest returns success=false for an unknown component",
  async () => {
    const registry = new ComponentRegistry();
    let failedKind: string | undefined;
    const captureMark = {
      fail(kind: string) {
        failedKind = kind;
      },
    };
    const req = buildRequest(
      "application/eigon+cbor",
      new Uint8Array(),
      new Uint8Array(),
    );

    const resp = await executeComponentRequest(req, registry, captureMark);
    assertEquals(resp.success, false);
    assertEquals(failedKind, "unknown_component");
  },
);
