/**
 * Program Execution Coordinator
 *
 * Coordinates program execution between the kernel (which walks the
 * expression tree) and the orchestrator (which handles IO component
 * dispatch). The kernel drives execution; the orchestrator provides
 * component handlers.
 *
 * Architecture reference: D6 (execution architecture)
 */

import { KernelClient } from "../client/kernel_client.ts";
import { ComponentRegistry } from "../components/registry.ts";
import type { RunProgramResponse } from "../gen/eigenius_pb.ts";

export class ProgramExecutor {
  private client: KernelClient;
  private components: ComponentRegistry;

  constructor(client: KernelClient, components: ComponentRegistry) {
    this.client = client;
    this.components = components;
  }

  /**
   * Run a program via the kernel.
   *
   * The kernel evaluates the expression tree. IO component calls are
   * dispatched back to the orchestrator's component registry.
   */
  async run(
    programJson: string,
    inputJson: string,
  ): Promise<RunProgramResponse> {
    return await this.client.runProgram(programJson, inputJson);
  }

  /** Get the component registry for handler registration. */
  getComponents(): ComponentRegistry {
    return this.components;
  }
}
