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
 * Cross-library CBOR interop test: cbor-x ↔ ciborium.
 *
 * Pipeline per case:
 *   JS object
 *     → cbor-x encode (our `encodeResource`)
 *     → WASM execute (guest decodes via ciborium, re-encodes via ciborium)
 *     → cbor-x decode (our `decodeResource`)
 *     → compare with the original
 *
 * Covers value types the main suites don't exercise: floats (several
 * widths' worth of values), booleans, null, large and negative integers,
 * empty collections, non-ASCII strings, and nested shapes.
 *
 * Uses the [`wasm-cbor-echo`](../../examples/wasm-cbor-echo) fixture.
 *
 * Run with:
 *   deno test --allow-read --allow-ffi --allow-env --allow-sys \
 *     --unstable-node-globals --unstable-detect-cjs tests/cbor_roundtrip_test.ts
 */

import { assertEquals } from "@std/assert";
import { ComponentRegistry } from "../src/components/registry.ts";
import { tryLoadWasmAddon } from "../src/wasm/loadAddon.ts";
import { WasmComponentRegistry } from "../src/wasm/registry.ts";
import {
  createHostBridge,
  createWasmComponentHandler,
} from "../src/wasm/hostBridge.ts";
import { decodeResource } from "../src/wasm/cbor.ts";
import { encode as rawCborEncode, Tag } from "cbor-x";

const ECHO_IRI = "urn:test:components:CborEcho";
const FIXTURE_URL = new URL(
  "../../kernel/tests/fixtures/eigenius_wasm_cbor_echo.wasm",
  import.meta.url,
);

// deno-lint-ignore no-explicit-any
type Obj = Record<string, any>;

// Cases cover the value types our routine tests don't touch.
const CASES: Array<{ name: string; value: Obj }> = [
  // Note: the Eigon model treats null as "property absent" rather than a
  // first-class value. The SDK's `Resource::from_cbor` rejects null-valued
  // properties with `null values not allowed`. We test that separately at
  // the end; nulls are excluded from the round-trip cases here.
  { name: "boolean true", value: { "urn:t:b": true } },
  { name: "boolean false", value: { "urn:t:b": false } },
  { name: "small positive int", value: { "urn:t:i": 42 } },
  { name: "zero", value: { "urn:t:i": 0 } },
  { name: "negative int", value: { "urn:t:i": -123 } },
  { name: "large positive int (< 2^32)", value: { "urn:t:i": 3_500_000_000 } },
  { name: "max safe int", value: { "urn:t:i": Number.MAX_SAFE_INTEGER } },
  { name: "float 0.5", value: { "urn:t:f": 0.5 } },
  { name: "float pi-ish", value: { "urn:t:f": 3.141592653589793 } },
  { name: "float negative", value: { "urn:t:f": -2.718281828 } },
  { name: "float very small", value: { "urn:t:f": 1e-20 } },
  { name: "float very large", value: { "urn:t:f": 1.234e30 } },
  { name: "empty string", value: { "urn:t:s": "" } },
  { name: "unicode string", value: { "urn:t:s": "héllo 🚀 世界" } },
  { name: "array of strings", value: { "urn:t:a": ["a", "b", "c"] } },
  { name: "empty array", value: { "urn:t:a": [] } },
  {
    name: "mixed-type array (no null)",
    value: { "urn:t:a": [1, "two", true] },
  },
  {
    name: "nested embedded resource",
    value: {
      "urn:t:outer": {
        "urn:t:inner": {
          "urn:t:deep": "value",
        },
      },
    },
  },
  {
    name: "multi-property resource",
    value: {
      "urn:eigenius:core:is_a": ["urn:test:Shape"],
      "urn:t:name": "mixed",
      "urn:t:count": 3,
      "urn:t:ratio": 0.75,
      "urn:t:enabled": true,
    },
  },
];

Deno.test("cbor-x ↔ ciborium round-trip via WASM echo", async (t) => {
  const addon = tryLoadWasmAddon();
  if (!addon) {
    console.warn(
      "Skipping: native addon not built (run `deno task build:addon`)",
    );
    return;
  }

  const binary = await Deno.readFile(FIXTURE_URL);

  const registry = new ComponentRegistry();
  const wasmRegistry = new WasmComponentRegistry(addon);
  const bridge = createHostBridge({ addon, registry, wasmRegistry });

  await wasmRegistry.register(ECHO_IRI, binary);
  registry.register(
    ECHO_IRI,
    createWasmComponentHandler(ECHO_IRI, { addon, wasmRegistry, bridge }),
  );

  try {
    for (const { name, value } of CASES) {
      await t.step(name, async () => {
        const result = await registry.execute(ECHO_IRI, {
          input: value,
          argument: {},
        });
        assertEquals(
          result.output,
          value,
          `round-trip mismatch for '${name}': got ${
            JSON.stringify(result.output)
          }`,
        );
      });
    }

    // Positive assertion of the documented constraint: null-valued
    // properties are rejected cleanly at the guest (not silently dropped).
    await t.step("null property is rejected with a clear error", async () => {
      let err: Error | undefined;
      try {
        await registry.execute(ECHO_IRI, {
          input: { "urn:t:n": null },
          argument: {},
        });
      } catch (e) {
        err = e as Error;
      }
      if (!err) throw new Error("expected null-value rejection");
      if (!/null values not allowed/i.test(err.message)) {
        throw new Error(`unexpected error text: ${err.message}`);
      }
    });
  } finally {
    wasmRegistry.clear();
  }
});

/**
 * Regression for the Phase 18d/e bug where the kernel would tag
 * `Value::Json` payloads with `EIGENIUS_JSON_TAG = 27182` (per
 * `kernel/src/ontology/eigon_cbor.rs`) but cbor-x would surface the
 * decoded value as a `Tag { value, tag }` wrapper rather than the
 * inner JS object — breaking handler-side shape checks like
 * `CompleteJson`'s `isShortNameTable`. The decoder hook in
 * `wasm/cbor.ts` unwraps the tag.
 */
Deno.test("decodeResource unwraps EIGENIUS_JSON_TAG (27182) to the inner value", () => {
  const inner = {
    class_iri: "urn:test:Class",
    properties: { foo: "urn:test:foo" },
    enums: [],
  };
  // Encode using the bare cbor-x API (no extension installed for the
  // outer call) wrapping the value in Tag(27182). The orchestrator's
  // `decodeResource` must unwrap.
  const taggedBytes = rawCborEncode(new Tag(inner, 27182));
  const decoded = decodeResource(taggedBytes);
  assertEquals(decoded, inner);
});

Deno.test("decodeResource unwraps EIGENIUS_JSON_TAG nested inside a property", () => {
  // Wire shape the kernel produces when a Resource has a
  // `Value::Json` property — the tagged value sits as a property
  // value inside an outer Eigon-CBOR map.
  const tableValue = {
    class_iri: "urn:test:Class",
    properties: { name: "urn:test:name" },
    enums: [],
  };
  const taggedTable = new Tag(tableValue, 27182);
  const argument = {
    "urn:test:k": "v",
    "urn:eigenius:program:components:short_name_table": taggedTable,
  };
  const argumentBytes = rawCborEncode(argument);
  const decoded = decodeResource(argumentBytes);
  // The tagged property must appear as a plain object after decode,
  // not a Tag wrapper.
  assertEquals(decoded["urn:test:k"], "v");
  assertEquals(
    decoded["urn:eigenius:program:components:short_name_table"],
    tableValue,
  );
});
