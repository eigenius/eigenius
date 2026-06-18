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
 * CompleteText Component Handler
 *
 * Implements the urn:eigenius:program:components:CompleteText component
 * using Vercel AI SDK. Sends a text prompt to an LLM and returns the
 * completion as a string resource.
 *
 * Architecture reference: D6 (IO components), Phase 4 plan §4
 */

import { generateText } from "ai";
import { anthropic } from "@ai-sdk/anthropic";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";
import type {
  ComponentHandler,
  ComponentInput,
  ComponentMetrics,
  ComponentOutput,
} from "./registry.ts";

/** Default request parameters. */
const DEFAULTS = {
  model: "claude-sonnet-4-6",
  temperature: 0.3,
  maxTokens: 4000,
};

/**
 * Extract prompt and parameters from the argument resource.
 *
 * Expected argument structure:
 * ```
 * {
 *   "urn:eigenius:program:components:completion:user_prompt": "...",
 *   "urn:eigenius:program:components:completion:system_prompt": "...",
 *   "urn:eigenius:program:components:completion:request_parameters": {
 *     "urn:eigenius:program:request:model": "claude-sonnet-4-6",
 *     "urn:eigenius:program:request:temperature": 0.3,
 *     "urn:eigenius:program:request:max_tokens": 4000
 *   }
 * }
 * ```
 */
// deno-lint-ignore no-explicit-any
function parseArgument(argument: Record<string, any>): {
  userPrompt: string;
  systemPrompt?: string;
  model: string;
  temperature: number;
  maxTokens: number;
} {
  const userPrompt =
    argument["urn:eigenius:program:components:completion:user_prompt"] ?? "";
  const systemPrompt =
    argument["urn:eigenius:program:components:completion:system_prompt"];
  const params = argument[
    "urn:eigenius:program:components:completion:request_parameters"
  ] ?? {};

  return {
    userPrompt,
    systemPrompt,
    model: params["urn:eigenius:program:request:model"] ?? DEFAULTS.model,
    temperature: params["urn:eigenius:program:request:temperature"] ??
      DEFAULTS.temperature,
    maxTokens: params["urn:eigenius:program:request:max_tokens"] ??
      DEFAULTS.maxTokens,
  };
}

/**
 * Format the prompt by interpolating the input resource.
 *
 * Replaces `{{string}}` with the JSON-serialized input.
 * Replaces `{{property_iri}}` with specific property values.
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
 * Create the CompleteText component handler using Vercel AI SDK.
 *
 * Requires ANTHROPIC_API_KEY environment variable.
 */
export function createCompleteTextHandler(): ComponentHandler {
  return async (req: ComponentInput): Promise<ComponentOutput> => {
    const { userPrompt, systemPrompt, model, temperature, maxTokens } =
      parseArgument(req.argument);

    const prompt = formatPrompt(userPrompt, req.input);
    const startTime = Date.now();

    log.debug(operation.LLM_COMPLETE_TEXT, "LLM call starting", {
      provider: "anthropic",
      model,
      prompt_chars: prompt.length,
    });

    let result;
    try {
      result = await generateText({
        model: anthropic(model),
        system: systemPrompt,
        prompt,
        temperature,
        maxOutputTokens: maxTokens,
      });
    } catch (e) {
      log.warn(operation.LLM_COMPLETE_TEXT, "LLM call failed", {
        provider: "anthropic",
        model,
        error_kind: "provider_error",
        error_message: e instanceof Error ? e.message : String(e),
        latency_ms: Date.now() - startTime,
      });
      throw e;
    }

    const latencyMs = Date.now() - startTime;

    log.info(operation.LLM_COMPLETE_TEXT, "LLM call completed", {
      provider: "anthropic",
      model,
      prompt_tokens: result.usage.inputTokens ?? 0,
      completion_tokens: result.usage.outputTokens ?? 0,
      token_count: (result.usage.inputTokens ?? 0) +
        (result.usage.outputTokens ?? 0),
      latency_ms: latencyMs,
    });

    const metrics: ComponentMetrics = {
      provider: "anthropic",
      model,
      promptTokens: result.usage.inputTokens ?? 0,
      completionTokens: result.usage.outputTokens ?? 0,
      latencyMs,
    };

    return {
      output: { "urn:eigenius:program:value": result.text },
      metrics,
    };
  };
}

/**
 * Create a mock CompleteText handler for testing.
 *
 * Returns deterministic text without an API call.
 */
export function createMockCompleteTextHandler(
  responseText = "This is a mock LLM response.",
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
      output: { "urn:eigenius:program:value": responseText },
      metrics,
    });
  };
}

/** The component IRI for CompleteText. */
export const COMPLETE_TEXT_IRI = "urn:eigenius:program:components:CompleteText";
