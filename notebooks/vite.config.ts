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

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

// Phase 2 — static viewer. The notebook is served from /notebooks/* in
// Phase 4 (per D22 §6.10 / §6.11) once the orchestrator's static-file
// route is added; for Phase 2 the dev server runs standalone on 5173.
export default defineConfig({
  plugins: [react()],
  base: "/notebooks/", // the eventual mount point
  resolve: {
    alias: {
      // The TypeScript SDK lives at clients/eigenius-ts/. Aliasing it
      // for development means the notebook can `import { Eigen } from
      // "@eigenius/client"` without a JSR/npm round-trip.
      "@eigenius/client": path.resolve(
        here,
        "..",
        "clients",
        "eigenius-ts",
        "mod.ts",
      ),
    },
  },
  server: {
    port: 5173,
    // Proxy Connect-RPC traffic to the orchestrator so the browser
    // hits a single origin (D22 §6.10). The two services we currently
    // talk to are EigeniusKernel and NotebookService; both are mounted
    // under their fully-qualified proto package on the orchestrator.
    proxy: {
      "/eigenius.v1.EigeniusKernel": "http://localhost:8080",
      "/eigenius.v1.NotebookService": "http://localhost:8080",
    },
  },
});
