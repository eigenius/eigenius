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
 * WasmComponentRegistry — tracks IRI ↔ native-addon handle mappings.
 *
 * The kernel ships each IO WASM binary via `RegisterWasmComponent` gRPC.
 * We compile it through the addon, store the resulting handle here, and
 * plug a handler into the parent `ComponentRegistry` that dispatches back
 * through the addon on each Execute call.
 *
 * Binaries are held in memory only. If the orchestrator restarts, the
 * kernel must re-register (tracked in issue #11).
 */

import type { WasmAddon } from "./loadAddon.ts";

export interface WasmRegistration {
  handle: number;
  componentIri: string;
  fuelLimit: number;
  memoryLimitPages: number;
}

export class WasmComponentRegistry {
  private byIri = new Map<string, WasmRegistration>();

  constructor(private addon: WasmAddon) {}

  /** Whether a WASM component has been registered for this IRI. */
  has(componentIri: string): boolean {
    return this.byIri.has(componentIri);
  }

  /** List all registered component IRIs. */
  list(): string[] {
    return [...this.byIri.keys()];
  }

  /**
   * Compile + register a WASM component. Returns the new handle.
   * Re-registering the same IRI unloads the previous handle first.
   */
  async register(
    componentIri: string,
    binary: Uint8Array,
    opts: { fuelLimit?: number; memoryLimitPages?: number } = {},
  ): Promise<number> {
    const fuelLimit = opts.fuelLimit ?? 0;
    const memoryLimitPages = opts.memoryLimitPages ?? 0;

    if (this.byIri.has(componentIri)) {
      this.unregister(componentIri);
    }

    const handle = await this.addon.loadComponent(binary, {
      fuelLimit,
      memoryLimitPages,
    });

    this.byIri.set(componentIri, {
      handle,
      componentIri,
      fuelLimit,
      memoryLimitPages,
    });
    return handle;
  }

  /** Release the handle for a component IRI. Returns true if something was removed. */
  unregister(componentIri: string): boolean {
    const reg = this.byIri.get(componentIri);
    if (!reg) return false;
    this.addon.unloadComponent(reg.handle);
    this.byIri.delete(componentIri);
    return true;
  }

  /** Look up the handle for a registered IRI. */
  getHandle(componentIri: string): number | undefined {
    return this.byIri.get(componentIri)?.handle;
  }

  /** Release every registered component. Idempotent. */
  clear(): void {
    for (const reg of this.byIri.values()) {
      this.addon.unloadComponent(reg.handle);
    }
    this.byIri.clear();
  }
}
