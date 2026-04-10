/**
 * LLM Adapter Interface
 *
 * Defines the contract for LLM provider adapters. Each adapter translates
 * Eigon-typed requests into provider-specific API calls and wraps responses
 * as typed Eigon resources.
 *
 * Architecture reference: §2.3 (AI Integration Model)
 */

export interface LlmRequest {
  systemMessage?: string;
  userMessage: string;
  model?: string;
  temperature?: number;
  maxTokens?: number;
}

export interface LlmResponse {
  content: string;
  model: string;
  tokenUsage: {
    promptTokens: number;
    completionTokens: number;
    totalTokens: number;
  };
}

export interface LlmAdapter {
  readonly providerId: string;
  invoke(request: LlmRequest): Promise<LlmResponse>;
}

// TODO: Phase 4 — Implement adapters for Anthropic, OpenAI, etc.
// using Vercel AI SDK for provider abstraction.
