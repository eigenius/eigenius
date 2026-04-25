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
