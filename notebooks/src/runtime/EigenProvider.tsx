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

import { createContext, type ReactNode, useContext, useMemo } from "react";
import { Eigen } from "@eigenius/client";

const EigenContext = createContext<Eigen | null>(null);

/**
 * Endpoint resolution:
 *   1. `VITE_EIGENIUS_ORCHESTRATOR` build-time env var, if set.
 *   2. The current page origin (single-origin deployment, D22 §2.3 —
 *      the orchestrator serves both /notebooks/* and the RPC paths).
 *   3. In `vite dev`, requests on the same origin are routed to the
 *      orchestrator by the proxy entries in `vite.config.ts`.
 */
function resolveEndpoint(): string {
  const fromEnv = (import.meta.env.VITE_EIGENIUS_ORCHESTRATOR ?? "").trim();
  if (fromEnv) return fromEnv;
  return window.location.origin;
}

export interface EigenProviderProps {
  children: ReactNode;
  /** Override the auto-resolved endpoint. Useful for tests / Storybook. */
  endpoint?: string;
}

export function EigenProvider({ children, endpoint }: EigenProviderProps) {
  const client = useMemo(
    () => new Eigen({ endpoint: endpoint ?? resolveEndpoint() }),
    [endpoint],
  );
  return (
    <EigenContext.Provider value={client}>{children}</EigenContext.Provider>
  );
}

/**
 * Access the active `Eigen` SDK client. Throws if used outside a
 * `<EigenProvider>` — that would be a bug, not a missing-feature case.
 */
export function useEigen(): Eigen {
  const ctx = useContext(EigenContext);
  if (!ctx) {
    throw new Error(
      "useEigen() called outside an <EigenProvider>. Wrap the tree at the root.",
    );
  }
  return ctx;
}
