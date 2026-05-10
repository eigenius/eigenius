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
 * Program Execution Coordinator
 *
 * Coordinates program execution between the kernel (which walks the
 * expression tree) and the orchestrator (which handles IO component
 * dispatch). The kernel drives execution; the orchestrator provides
 * component handlers.
 *
 * Architecture reference: D6 (execution architecture)
 */

import type { KernelClient } from "../client/kernel_client.ts";
import type { ComponentRegistry } from "../components/registry.ts";
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
