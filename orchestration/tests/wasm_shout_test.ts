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
 * Smoke test for M3: register the wasm-http-shout component via the TS API,
 * invoke it, and verify that the guest's dispatch-component call reaches a
 * mock CompleteText handler and the transformed text flows back out.
 *
 * Run with:
 *   deno test --allow-read --allow-ffi --allow-env --allow-sys \
 *     --unstable-node-globals --unstable-detect-cjs tests/wasm_shout_test.ts
 */

import { assertEquals } from "@std/assert";
import {
  type ComponentHandler,
  ComponentRegistry,
} from "../src/components/registry.ts";
import { tryLoadWasmAddon } from "../src/wasm/loadAddon.ts";
import { WasmComponentRegistry } from "../src/wasm/registry.ts";
import { createHostBridge } from "../src/wasm/hostBridge.ts";

const SHOUT_IRI = "urn:example:shout:Shout";
const COMPLETE_TEXT_IRI = "urn:eigenius:program:components:CompleteText";
const FIXTURE_URL = new URL(
  "../../kernel/tests/fixtures/eigenius_wasm_http_shout.wasm",
  import.meta.url,
);

/**
 * Mock CompleteText: extracts the user_prompt from the argument and returns
 * it uppercased wrapped in a value resource. This matches what the real
 * handler in `complete_text.ts` would return in shape.
 */
const mockCompleteText: ComponentHandler = ({ argument }) => {
  // The shout guest wraps the text in an instruction prompt of the form:
  //   "Rewrite the following text in ALL CAPS … Text: <actual>"
  // A real LLM would return just the uppercased <actual>. We emulate that
  // by pulling out whatever follows the last "Text: " marker.
  const userPrompt = argument[
    "urn:eigenius:program:components:completion:user_prompt"
  ] as string ?? "";
  const marker = "Text: ";
  const idx = userPrompt.lastIndexOf(marker);
  const payload = idx >= 0
    ? userPrompt.slice(idx + marker.length).trim()
    : userPrompt;
  return Promise.resolve({
    output: {
      "urn:eigenius:program:value": payload.toUpperCase(),
    },
  });
};

Deno.test("wasm-http-shout round-trip via orchestrator API", async () => {
  const addon = tryLoadWasmAddon();
  if (!addon) {
    console.warn(
      "Skipping WASM smoke test: native addon not built " +
        "(cd orchestration/native && ./node_modules/.bin/napi build --platform)",
    );
    return;
  }

  const binary = await Deno.readFile(FIXTURE_URL);

  const registry = new ComponentRegistry();
  registry.register(COMPLETE_TEXT_IRI, mockCompleteText);

  const wasmRegistry = new WasmComponentRegistry(addon);
  const bridge = createHostBridge({
    addon,
    registry,
    wasmRegistry,
  });

  // Simulate the kernel's RegisterWasmComponent path.
  await wasmRegistry.register(SHOUT_IRI, binary);
  const { createWasmComponentHandler } = await import(
    "../src/wasm/hostBridge.ts"
  );
  registry.register(
    SHOUT_IRI,
    createWasmComponentHandler(SHOUT_IRI, { addon, wasmRegistry, bridge }),
  );

  // Invoke as the kernel would via ComponentExecutor.execute.
  const result = await registry.execute(SHOUT_IRI, {
    input: {
      "urn:example:shout:text": "hello from wasm",
    },
    argument: {},
  });

  const shouted = result.output["urn:example:shout:shouted"] as
    | string
    | undefined;
  assertEquals(
    shouted,
    "HELLO FROM WASM",
    `expected uppercased text, got: ${JSON.stringify(result.output)}`,
  );

  // Clean up.
  wasmRegistry.clear();
});
