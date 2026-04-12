/**
 * Program Execution Engine
 *
 * Executes validated programs (typed expressions) by walking the
 * expression tree and dispatching component calls.
 *
 * Architecture reference: §4 (Processing Programs)
 */

import { KernelClient } from "../client/kernel_client.ts";

export class ProgramExecutor {
  private client: KernelClient;

  constructor(client: KernelClient) {
    this.client = client;
  }

  execute(_program: unknown, _inputs: unknown): Promise<unknown> {
    // TODO: Phase 2 — program execution
    void this.client;
    return Promise.reject(new Error("Not implemented"));
  }
}
