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
 * Phase 1 smoke test for `@eigenius/client`.
 *
 * Requires a running `docker compose up` (or local kernel + orchestrator):
 *   - kernel reachable from orchestrator at $EIGENIUS_KERNEL_ENDPOINT
 *   - orchestrator listening on $EIGENIUS_ORCHESTRATOR (default http://localhost:8080)
 *
 * Exits 0 on success, non-zero on any RPC failure.
 *
 * Usage:
 *   deno run --allow-net --allow-env clients/eigenius-ts/examples/smoke-test.ts
 *   deno task -c clients/eigenius-ts/deno.jsonc smoke
 */

import { Eigen, NodeKind } from "../mod.ts";

const ENDPOINT = Deno.env.get("EIGENIUS_ORCHESTRATOR") ??
  "http://localhost:8080";

const eigen = new Eigen({ endpoint: ENDPOINT });

console.log(`SDK smoke test against ${ENDPOINT}`);

// ---------------------------------------------------------------------
// 1. layerTopology — taxonomy only (default)
// ---------------------------------------------------------------------
console.log("\n[1/2] layerTopology() — taxonomy only");
const taxonomy = await eigen.layerTopology();
console.log(
  `  ${taxonomy.nodes.length} nodes, ${taxonomy.edges.length} edges`,
);

const layerNodes = taxonomy.nodes.filter((n) => n.kind === NodeKind.LAYER);
const classNodes = taxonomy.nodes.filter((n) => n.kind === NodeKind.CLASS);
console.log(`  ${layerNodes.length} layer node(s), ${classNodes.length} class node(s)`);
if (layerNodes.length === 0) {
  console.error("  ✗ expected at least one layer node from the bootstrap chain");
  Deno.exit(1);
}
if (classNodes.length === 0) {
  console.error(
    "  ✗ expected at least one class node from the core ontology",
  );
  Deno.exit(1);
}

// Sample a layer node and verify its attrs include the count fields.
const firstLayer = layerNodes[0];
for (const key of [
  "name",
  "class_count",
  "property_count",
  "resource_count",
  "institution_count",
]) {
  if (!(key in firstLayer.attrs)) {
    console.error(`  ✗ layer node missing attr: ${key}`);
    Deno.exit(1);
  }
}
console.log(
  `  ✓ first layer "${firstLayer.attrs.name}" has expected count attrs`,
);

// ---------------------------------------------------------------------
// 2. layerTopology — with includeResources=true
// ---------------------------------------------------------------------
console.log("\n[2/2] layerTopology({ includeResources: true })");
const full = await eigen.layerTopology({ includeResources: true });
console.log(
  `  ${full.nodes.length} nodes, ${full.edges.length} edges`,
);

if (full.nodes.length < taxonomy.nodes.length) {
  console.error(
    `  ✗ includeResources=true produced fewer nodes (${full.nodes.length}) than default (${taxonomy.nodes.length})`,
  );
  Deno.exit(1);
}
console.log("  ✓ includeResources=true produced ≥ default node count");

console.log("\n✓ smoke test passed");
