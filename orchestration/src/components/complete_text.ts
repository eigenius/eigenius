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
import type {
  ComponentHandler,
  ComponentInput,
  ComponentMetrics,
  ComponentOutput,
} from "./registry.ts";

/** Default request parameters. */
const DEFAULTS = {
  model: "claude-sonnet-4-20250514",
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
 *     "urn:eigenius:program:request:model": "claude-sonnet-4-20250514",
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
  const params =
    argument[
      "urn:eigenius:program:components:completion:request_parameters"
    ] ?? {};

  return {
    userPrompt,
    systemPrompt,
    model: params["urn:eigenius:program:request:model"] ?? DEFAULTS.model,
    temperature:
      params["urn:eigenius:program:request:temperature"] ??
        DEFAULTS.temperature,
    maxTokens:
      params["urn:eigenius:program:request:max_tokens"] ?? DEFAULTS.maxTokens,
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

    const result = await generateText({
      model: anthropic(model),
      system: systemPrompt,
      prompt,
      temperature,
      maxOutputTokens: maxTokens,
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
export const COMPLETE_TEXT_IRI =
  "urn:eigenius:program:components:CompleteText";
