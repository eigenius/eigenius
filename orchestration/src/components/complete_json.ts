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
 * CompleteJson Component Handler
 *
 * Implements the urn:eigenius:program:components:CompleteJson component
 * using Vercel AI SDK's generateObject(). Receives a JSON Schema from
 * the kernel, sends it to the LLM with the prompt, and returns the
 * structured JSON response. The kernel handles schema generation and
 * JSON → Eigon conversion.
 *
 * Architecture reference: D8 (CompleteJson specification)
 */

import { generateObject } from "ai";
import { anthropic } from "@ai-sdk/anthropic";
import { jsonSchema } from "ai";
import type {
  ComponentHandler,
  ComponentInput,
  ComponentMetrics,
  ComponentOutput,
} from "./registry.ts";

/** Default request parameters. */
const DEFAULTS = {
  model: "claude-sonnet-4-20250514",
  temperature: 0.0,
  maxTokens: 4000,
};

/**
 * Extract prompt, schema, and parameters from the argument resource.
 */
// deno-lint-ignore no-explicit-any
function parseArgument(argument: Record<string, any>): {
  userPrompt: string;
  systemPrompt?: string;
  outputSchema?: object;
  model: string;
  temperature: number;
  maxTokens: number;
} {
  const userPrompt =
    argument["urn:eigenius:program:components:completion:user_prompt"] ?? "";
  const systemPrompt =
    argument["urn:eigenius:program:components:completion:system_prompt"];
  const outputSchema =
    argument["urn:eigenius:program:components:completion:output_schema"];
  const params = argument[
    "urn:eigenius:program:components:completion:request_parameters"
  ] ?? {};

  return {
    userPrompt,
    systemPrompt,
    outputSchema: typeof outputSchema === "object" ? outputSchema : undefined,
    model: params["urn:eigenius:program:request:model"] ?? DEFAULTS.model,
    temperature: params["urn:eigenius:program:request:temperature"] ??
      DEFAULTS.temperature,
    maxTokens: params["urn:eigenius:program:request:max_tokens"] ??
      DEFAULTS.maxTokens,
  };
}

/**
 * Format the prompt by interpolating the input resource.
 */
// deno-lint-ignore no-explicit-any
function formatPrompt(template: string, input: Record<string, any>): string {
  let result = template.replace("{{string}}", JSON.stringify(input));

  result = result.replace(/\{\{(\S+?)\}\}/g, (_match, key: string) => {
    if (input[key] !== undefined) {
      const val = input[key];
      return typeof val === "string" ? val : JSON.stringify(val);
    }
    return `{{${key}}}`;
  });

  return result;
}

/**
 * Create the CompleteJson component handler using Vercel AI SDK.
 *
 * Requires ANTHROPIC_API_KEY environment variable.
 */
export function createCompleteJsonHandler(): ComponentHandler {
  return async (req: ComponentInput): Promise<ComponentOutput> => {
    const {
      userPrompt,
      systemPrompt,
      outputSchema,
      model,
      temperature,
      maxTokens,
    } = parseArgument(req.argument);

    const prompt = formatPrompt(userPrompt, req.input);
    const startTime = Date.now();

    if (!outputSchema) {
      throw new Error(
        "CompleteJson requires output_schema in component argument",
      );
    }

    const result = await generateObject({
      model: anthropic(model),
      system: systemPrompt,
      prompt,
      temperature,
      maxOutputTokens: maxTokens,
      schema: jsonSchema(outputSchema),
    });

    const latencyMs = Date.now() - startTime;

    const metrics: ComponentMetrics = {
      provider: "anthropic",
      model,
      promptTokens: result.usage.inputTokens ?? 0,
      completionTokens: result.usage.outputTokens ?? 0,
      latencyMs,
    };

    return {
      output: result.object as Record<string, unknown>,
      metrics,
    };
  };
}

/**
 * Create a mock CompleteJson handler for testing.
 *
 * Returns a deterministic object matching the schema keys.
 */
export function createMockCompleteJsonHandler(
  // deno-lint-ignore no-explicit-any
  responseObject: Record<string, any> = { result: "mock" },
): ComponentHandler {
  return (req: ComponentInput): Promise<ComponentOutput> => {
    const { model } = parseArgument(req.argument);

    const metrics: ComponentMetrics = {
      provider: "mock",
      model,
      promptTokens: 10,
      completionTokens: 5,
      latencyMs: 1,
    };

    return Promise.resolve({
      output: responseObject,
      metrics,
    });
  };
}

/** The component IRI for CompleteJson. */
export const COMPLETE_JSON_IRI = "urn:eigenius:program:components:CompleteJson";
