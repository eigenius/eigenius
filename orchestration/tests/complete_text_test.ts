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

import { assertEquals, assertExists } from "@std/assert";
import {
  COMPLETE_TEXT_IRI,
  createMockCompleteTextHandler,
} from "../src/components/complete_text.ts";
import { ComponentRegistry } from "../src/components/registry.ts";

const ARGUMENT = {
  "urn:eigenius:program:components:completion:user_prompt":
    "Summarize: {{string}}",
  "urn:eigenius:program:components:completion:system_prompt":
    "You are a helpful assistant.",
  "urn:eigenius:program:components:completion:request_parameters": {
    "urn:eigenius:program:request:model": "claude-sonnet-4-6",
    "urn:eigenius:program:request:temperature": 0.3,
    "urn:eigenius:program:request:max_tokens": 100,
  },
};

const INPUT = {
  "urn:eigenius:example:text": "Hello world",
};

Deno.test("mock CompleteText returns deterministic response", async () => {
  const handler = createMockCompleteTextHandler("Mock summary.");
  const result = await handler({ input: INPUT, argument: ARGUMENT });

  assertEquals(result.output["urn:eigenius:program:value"], "Mock summary.");
  assertExists(result.metrics);
  assertEquals(result.metrics!.provider, "mock");
  assertEquals(result.metrics!.promptTokens, 10);
  assertEquals(result.metrics!.completionTokens, 5);
});

Deno.test("mock CompleteText default response", async () => {
  const handler = createMockCompleteTextHandler();
  const result = await handler({ input: INPUT, argument: ARGUMENT });

  assertEquals(
    result.output["urn:eigenius:program:value"],
    "This is a mock LLM response.",
  );
});

Deno.test("ComponentRegistry dispatches to CompleteText", async () => {
  const registry = new ComponentRegistry();
  registry.register(COMPLETE_TEXT_IRI, createMockCompleteTextHandler("OK"));

  assertEquals(registry.has(COMPLETE_TEXT_IRI), true);
  assertEquals(registry.listComponents(), [COMPLETE_TEXT_IRI]);

  const result = await registry.execute(COMPLETE_TEXT_IRI, {
    input: INPUT,
    argument: ARGUMENT,
  });

  assertEquals(result.output["urn:eigenius:program:value"], "OK");
});

Deno.test("ComponentRegistry throws for unknown component", async () => {
  const registry = new ComponentRegistry();
  try {
    await registry.execute("urn:unknown:component", {
      input: {},
      argument: {},
    });
    throw new Error("should have thrown");
  } catch (e) {
    assertEquals(
      (e as Error).message,
      "No handler registered for component: urn:unknown:component",
    );
  }
});
