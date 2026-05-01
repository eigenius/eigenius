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
 * Smoke test for `@eigenius/client` (Phase 1 + Phase 3a + Phase 14g).
 *
 * Exercises every method the SDK currently exposes:
 *   - layerTopology   (NotebookService)
 *   - inspect         (EigeniusKernel passthrough)
 *   - query           (EigeniusKernel passthrough)
 *   - listInstitutions (EigeniusKernel passthrough)
 *   - health          (EigeniusKernel passthrough)
 *   - load            (EigeniusKernel passthrough — Phase 3a)
 *   - validateProgram (EigeniusKernel passthrough — Phase 3a)
 *   - listBranches / getBranch / createBranch / deleteBranch +
 *     per-call branch routing on load          (Phase 14g)
 *
 * Requires a running stack (kernel + orchestrator). Easiest is:
 *   EIGENIUS_MOCK_LLM=true docker compose up --build -d
 *
 * Exits 0 on success, non-zero on any RPC failure.
 */

import { Eigen, NodeKind } from "../mod.ts";

const ENDPOINT = Deno.env.get("EIGENIUS_ORCHESTRATOR") ??
  "http://localhost:8080";

const eigen = new Eigen({ endpoint: ENDPOINT });

console.log(`SDK smoke test against ${ENDPOINT}\n`);

let stepNum = 0;
const TOTAL_STEPS = 8;
function step(label: string): void {
  stepNum++;
  console.log(`[${stepNum}/${TOTAL_STEPS}] ${label}`);
}
function fail(msg: string): never {
  console.error(`  ✗ ${msg}`);
  Deno.exit(1);
  throw new Error("unreachable");
}
function ok(msg: string): void {
  console.log(`  ✓ ${msg}`);
}

// ---------------------------------------------------------------------
// 1. health — kernel liveness
// ---------------------------------------------------------------------
step("health()");
const health = await eigen.health();
if (!health.healthy) fail(`kernel is unhealthy: ${JSON.stringify(health)}`);
ok(
  `kernel v${health.version}, ${health.layerCount} layer(s), ${health.resourceCount} resource(s)`,
);
console.log();

// ---------------------------------------------------------------------
// 2. inspect — fetch a known core resource
// ---------------------------------------------------------------------
step('inspect("urn:eigenius:core:Class")');
const inspectResp = await eigen.inspect("urn:eigenius:core:Class");
if (!inspectResp.found) fail("urn:eigenius:core:Class should always exist");
if (inspectResp.resource.length === 0) {
  fail("response has found=true but empty resource bytes");
}
ok(`fetched Class resource (${inspectResp.resource.length} CBOR bytes)`);

// Negative case: a missing IRI should return found=false, not error
const missing = await eigen.inspect("urn:eigenius:nonexistent:zzz");
if (missing.found) fail("expected found=false for nonexistent IRI");
ok("nonexistent IRI returns found=false (not an error)");
console.log();

// ---------------------------------------------------------------------
// 3. query — execute an EigenQL query
// ---------------------------------------------------------------------
step("query() — list all classes");
const queryResp = await eigen.query(`
  USING "urn:eigenius:core:Class"
  MATCH Class(?c) { short_name: ?n }
  RETURN [] { name: ?n }
`);
if (!queryResp.success) fail(`query failed: ${queryResp.error}`);
if (queryResp.document.length === 0) fail("query returned empty document");
ok(`query succeeded (${queryResp.document.length} CBOR bytes returned)`);
console.log();

// ---------------------------------------------------------------------
// 4. listInstitutions — registered institution list
// ---------------------------------------------------------------------
step("listInstitutions()");
const institutions = await eigen.listInstitutions();
ok(`${institutions.length} institution(s) registered`);
for (const inst of institutions) {
  console.log(
    `    - ${inst.iri} (${inst.queryTypes.length} query type(s))`,
  );
}
console.log();

// ---------------------------------------------------------------------
// 5. layerTopology — both modes
// ---------------------------------------------------------------------
step("layerTopology() — taxonomy then full");
const taxonomy = await eigen.layerTopology();
const layerNodes = taxonomy.nodes.filter((n) => n.kind === NodeKind.LAYER);
const classNodes = taxonomy.nodes.filter((n) => n.kind === NodeKind.CLASS);
if (layerNodes.length === 0) fail("expected at least one layer node");
if (classNodes.length === 0) fail("expected at least one class node");
ok(
  `taxonomy: ${taxonomy.nodes.length} nodes (${layerNodes.length} layer / ${classNodes.length} class), ${taxonomy.edges.length} edges`,
);

const full = await eigen.layerTopology({ includeResources: true });
if (full.nodes.length < taxonomy.nodes.length) {
  fail(
    `includeResources=true produced fewer nodes (${full.nodes.length}) than default (${taxonomy.nodes.length})`,
  );
}
ok(
  `full: ${full.nodes.length} nodes (+ ${
    full.nodes.length - taxonomy.nodes.length
  } instance resources), ${full.edges.length} edges`,
);

// ---------------------------------------------------------------------
// 6. load — compile + commit a small ESL ontology
// ---------------------------------------------------------------------
step("load() — small ESL ontology");
const SMOKE_ESL = `
namespace smoke = "urn:eigenius:smoke";
namespace core  = "urn:eigenius:core";

class smoke:Widget {
    description = "A widget the smoke test loads.";
    requires smoke:label;
}

property smoke:label : core:string {
    description = "Human-readable widget label.";
}
`;
const loadResp = await eigen.load(SMOKE_ESL);
if (!loadResp.success) {
  fail(
    `load failed: ${loadResp.errors.map((e) => e.message).join("; ")}`,
  );
}
if (!loadResp.layerId) fail("load succeeded but returned empty layerId");
ok(
  `loaded ${loadResp.resourceCount} resource(s) → layer ${
    loadResp.layerId.slice(0, 12)
  }…`,
);
console.log();

// ---------------------------------------------------------------------
// 7. validateProgram — check a tiny ESL program against the layer chain
// ---------------------------------------------------------------------
step("validateProgram() — identity program over Widget");
const SMOKE_PROGRAM = `
namespace smoke = "urn:eigenius:smoke";

program smoke:identity_widget : smoke:Widget -> smoke:Widget {
    input
}
`;
const validateResp = await eigen.validateProgram(SMOKE_PROGRAM);
if (!validateResp.valid) {
  fail(
    `validateProgram failed: ${
      validateResp.errors.map((e) => e.message).join("; ")
    }`,
  );
}
ok(
  `program validated as ${validateResp.programType || "(no type returned)"}`,
);
console.log();

// ---------------------------------------------------------------------
// 8. branch CRUD + per-call branch routing (Phase 14g)
// ---------------------------------------------------------------------
step("branch CRUD + per-call routing");

const branches = await eigen.listBranches();
const mainBranch = branches.find((b) => b.name === "main");
if (!mainBranch) fail("expected `main` in listBranches output");
ok(
  `listBranches: ${branches.length} branch(es); main at ${
    mainBranch.headLayer.slice(0, 12)
  }…`,
);

const mainGet = await eigen.getBranch("main");
if (!mainGet.found) fail("getBranch(main) returned found=false");
if (mainGet.headLayer !== mainBranch.headLayer) {
  fail(
    `getBranch head ${
      mainGet.headLayer.slice(0, 12)
    }… disagrees with listBranches ${mainBranch.headLayer.slice(0, 12)}…`,
  );
}
ok("getBranch(main) head matches listBranches");

const branchName = `smoke-${Date.now()}`;
const created = await eigen.createBranch(branchName, {
  fromLayer: mainBranch.headLayer,
});
if (!created.success) fail(`createBranch failed: ${created.error}`);
ok(`createBranch(${branchName}) at ${created.headLayer.slice(0, 12)}…`);

// Load a tiny resource into the new branch and verify main is unchanged.
const ISOLATE_ESL = `
namespace branchsmoke = "urn:eigenius:branchsmoke:${stepNum}";
namespace core = "urn:eigenius:core";

class branchsmoke:Marker {
    description = "Marker resource that proves per-branch routing.";
}
`;
const loadIntoBranch = await eigen.load(ISOLATE_ESL, { branch: branchName });
if (!loadIntoBranch.success) {
  fail(
    `load into ${branchName} failed: ${
      loadIntoBranch.errors.map((e) => e.message).join("; ")
    }`,
  );
}
if (loadIntoBranch.branch !== branchName) {
  fail(`load echoed branch=${loadIntoBranch.branch}; expected ${branchName}`);
}
ok(
  `load committed to ${branchName} (layer ${
    loadIntoBranch.layerId.slice(0, 12)
  }…)`,
);

const mainAfter = await eigen.getBranch("main");
if (mainAfter.headLayer !== mainBranch.headLayer) {
  fail(
    `main advanced to ${
      mainAfter.headLayer.slice(0, 12)
    }… while loading into ${branchName}`,
  );
}
ok("main head unchanged after load into feature branch");

const deleted = await eigen.deleteBranch(branchName);
if (!deleted.success || !deleted.deleted) {
  fail(`deleteBranch failed: ${deleted.error}`);
}
ok(
  `deleteBranch(${branchName}) reclaimed ref (was at ${
    deleted.previousHead.slice(0, 12)
  }…)`,
);
console.log();

console.log("\n✓ smoke test passed");
