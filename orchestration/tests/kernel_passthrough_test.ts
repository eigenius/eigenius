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
 * The browser's view of the kernel is a CURATED SUBSET.
 *
 * `registerEigeniusKernelPassthrough` lists the EigeniusKernel methods the
 * orchestrator forwards; anything absent is not routed and returns Connect's
 * default "…is not implemented" — regardless of how complete the kernel behind
 * it is. That is a good default (the browser should not reach everything), and
 * it is also a silent failure mode: the kernel gains a method, the SDK gains a
 * wrapper, the UI calls it, and it fails at runtime with an error that reads
 * like a missing kernel feature.
 *
 * That is exactly what happened with D71 `FormalizeDocument` (`2026-08-20`):
 * the RPC, the CLI, the MCP tools and the SDK were all in place, and the
 * notebook cell still failed at the first call because this list was not
 * updated. The MCP tool inventory has had a pinning test since the MVP; this
 * did not. Now it does.
 */

import { assertEquals } from "@std/assert";
import { createConnectRouter } from "@connectrpc/connect";
import { registerEigeniusKernelPassthrough } from "../src/notebook/eigenius_kernel_passthrough.ts";
import type { KernelClient } from "../src/client/kernel_client.ts";

// ---------------------------------------------------------------------------
// The canonical passthrough inventory. If you expose (or withdraw) a kernel
// method in `eigenius_kernel_passthrough.ts`, update this list — this test will
// fail otherwise. That failure is on purpose.
// ---------------------------------------------------------------------------

const EXPECTED_METHODS = [
  "cancelTask",
  "consolidateChain",
  "createBranch",
  "createTag",
  "deleteBranch",
  "deleteTag",
  "estimateConsolidation",
  "estimateGc",
  "formalizeDocument",
  "getBranch",
  "getFormalizationResult",
  "getSchema",
  "getTaskStatus",
  "health",
  "inspect",
  "layerTopology",
  "listBranches",
  "listInstitutions",
  "listTags",
  "listTasks",
  "load",
  "mergeBranches",
  "parseSentence",
  "prepareMerge",
  "previewCascade",
  "previewMerge",
  "query",
  "reflect",
  "runGc",
  "runProgram",
  "runProgramByIri",
  "submitResolution",
  "validateProgram",
].sort();

function routedMethods(): string[] {
  const router = createConnectRouter();
  registerEigeniusKernelPassthrough(router, {
    kernel: { raw: {} } as unknown as KernelClient,
  });
  return router.handlers.map((h) => h.method.localName).sort();
}

Deno.test("the passthrough exposes exactly the curated method set", () => {
  assertEquals(routedMethods(), EXPECTED_METHODS);
});

Deno.test("a formalize cell's whole call sequence is routed", () => {
  const routed = new Set(routedMethods());
  // The notebook's formalize cell makes exactly these calls: start, poll,
  // fetch. Missing any one of them fails the cell at runtime with an error
  // that reads like a missing kernel feature rather than a missing route.
  for (
    const m of ["formalizeDocument", "getTaskStatus", "getFormalizationResult"]
  ) {
    assertEquals(routed.has(m), true, `${m} is not routed to the browser`);
  }
});
