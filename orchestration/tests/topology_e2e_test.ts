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
 * End-to-end topology test against the SDK.
 *
 * Spawns a real kernel + orchestrator pair, drives `eigen.layerTopology`
 * through `@eigenius/client`, and asserts that **edges actually come
 * back over the wire**. Phase 1c notebook UI rendered an empty graph
 * even though the kernel-side walker's unit tests pass; this test
 * pins the SDK-visible behaviour so a regression in either the wire
 * layer, the orchestrator proxy, or the proto codegen surfaces here.
 *
 * Run with:
 *   deno test --allow-read --allow-ffi --allow-env --allow-sys \
 *     --allow-net --allow-run --unstable-node-globals --unstable-detect-cjs \
 *     tests/topology_e2e_test.ts
 *
 * Prerequisites (test skips with a message if any are missing):
 *   • `cargo build` has produced target/debug/eigenius
 *   • `deno task build:addon` has produced native/index.js + .node
 */

import { assert, assertGreater } from "@std/assert";
import { EdgeKind, Eigen, NodeKind } from "../../clients/eigenius-ts/mod.ts";

const REPO_ROOT = new URL("../../", import.meta.url);
const KERNEL_BIN = new URL("./target/debug/eigenius", REPO_ROOT).pathname;
const ADDON_JS = new URL("./orchestration/native/index.js", REPO_ROOT).pathname;
const ORCH_ENTRY = new URL("./orchestration/src/main.ts", REPO_ROOT).pathname;

async function checkPrerequisites(): Promise<string | null> {
  for (
    const [label, path] of [
      ["kernel binary (run `cargo build`)", KERNEL_BIN],
      ["native addon (run `deno task build:addon`)", ADDON_JS],
    ]
  ) {
    try {
      await Deno.stat(path);
    } catch {
      return `missing ${label}: ${path}`;
    }
  }
  return null;
}

function pickPort(): number {
  const listener = Deno.listen({ port: 0 });
  const port = (listener.addr as Deno.NetAddr).port;
  listener.close();
  return port;
}

async function waitFor(
  check: () => Promise<boolean>,
  { timeoutMs = 30_000, intervalMs = 100, label = "condition" } = {},
): Promise<void> {
  const start = performance.now();
  while (performance.now() - start < timeoutMs) {
    try {
      if (await check()) return;
    } catch {
      // swallow — retry
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  throw new Error(`timed out waiting for ${label}`);
}

interface Spawned {
  shutdown: () => Promise<void>;
}

function spawn(
  label: string,
  cmd: string,
  args: string[],
  env: Record<string, string>,
  cwd?: string,
): Spawned {
  const child = new Deno.Command(cmd, {
    args,
    env,
    cwd,
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const prefix = (chunk: Uint8Array) => {
    const text = new TextDecoder().decode(chunk);
    for (const line of text.split("\n")) {
      if (line.length > 0) console.error(`[${label}] ${line}`);
    }
  };
  (async () => {
    for await (const chunk of child.stdout) prefix(chunk);
  })();
  (async () => {
    for await (const chunk of child.stderr) prefix(chunk);
  })();
  return {
    shutdown: async () => {
      try {
        child.kill("SIGTERM");
      } catch {
        // already exited
      }
      await child.status;
    },
  };
}

/**
 * Histogram of edge kinds — used both for assertions and for
 * diagnostic output when the test fails. The kind names are the same
 * strings the `EdgeKind` enum uses so failure messages stay readable.
 */
function edgeKindHistogram(
  edges: ReadonlyArray<{ kind: EdgeKind }>,
): Record<string, number> {
  const hist: Record<string, number> = {};
  for (const e of edges) {
    const name = EdgeKind[e.kind] ?? `UNKNOWN(${e.kind})`;
    hist[name] = (hist[name] ?? 0) + 1;
  }
  return hist;
}

Deno.test({
  name: "e2e: layerTopology returns nodes + edges over the SDK wire",
  sanitizeOps: false,
  sanitizeResources: false,
  async fn() {
    const missing = await checkPrerequisites();
    if (missing) {
      console.warn(`skipping topology e2e: ${missing}`);
      return;
    }

    const orchPort = pickPort();
    const kernelPort = pickPort();
    const orch = spawn(
      "orch",
      Deno.execPath(),
      [
        "run",
        "--allow-net",
        "--allow-read",
        "--allow-ffi",
        "--allow-env",
        "--allow-sys",
        "--unstable-node-globals",
        "--unstable-detect-cjs",
        ORCH_ENTRY,
      ],
      {
        ...Deno.env.toObject(),
        EIGENIUS_ORCHESTRATOR_PORT: String(orchPort),
        EIGENIUS_KERNEL_ENDPOINT: `http://localhost:${kernelPort}`,
        EIGENIUS_MOCK_LLM: "true",
      },
      new URL("./orchestration/", REPO_ROOT).pathname,
    );

    let kernel: Spawned | null = null;
    try {
      await waitFor(
        async () => {
          const resp = await fetch(`http://localhost:${orchPort}/health`);
          await resp.body?.cancel();
          return resp.ok;
        },
        { label: "orchestrator /health" },
      );

      kernel = spawn("kern", KERNEL_BIN, [
        "serve",
        "--port",
        String(kernelPort),
        "--orchestrator",
        `http://localhost:${orchPort}`,
      ], { ...Deno.env.toObject() });

      const eigen = new Eigen({ endpoint: `http://localhost:${orchPort}` });
      await waitFor(async () => (await eigen.health()).healthy, {
        label: "kernel health via SDK",
      });

      // -----------------------------------------------------------------
      // Baseline: core ontology only. The kernel boots with it loaded,
      // so we should see classes, properties, parent-layer links, and
      // class-hierarchy edges out of the box.
      // -----------------------------------------------------------------
      const baseline = await eigen.layerTopology();
      console.log(
        `[baseline] nodes=${baseline.nodes.length}, edges=${baseline.edges.length}`,
      );
      console.log(
        `[baseline] node kinds:`,
        countByKind(baseline.nodes, (n) => NodeKind[n.kind] ?? `${n.kind}`),
      );
      console.log(`[baseline] edge kinds:`, edgeKindHistogram(baseline.edges));

      assertGreater(
        baseline.nodes.length,
        0,
        "baseline topology should have nodes (at least core layers + classes)",
      );
      assertGreater(
        baseline.edges.length,
        0,
        "baseline topology should have edges (parent_layer + subclass_of in core ontology)",
      );

      const baselineKinds = edgeKindHistogram(baseline.edges);
      assert(
        (baselineKinds[EdgeKind[EdgeKind.PARENT_LAYER]] ?? 0) > 0,
        `expected PARENT_LAYER edges in core chain, got: ${
          JSON.stringify(baselineKinds)
        }`,
      );
      assert(
        (baselineKinds[EdgeKind[EdgeKind.SUBCLASS_OF]] ?? 0) > 0,
        `expected SUBCLASS_OF edges in core ontology, got: ${
          JSON.stringify(baselineKinds)
        }`,
      );

      // -----------------------------------------------------------------
      // Add a small ESL ontology. After the load we should pick up new
      // SUBCLASS_OF + PROPERTY_REF + REQUIRES edges keyed on the freshly
      // committed layer.
      // -----------------------------------------------------------------
      const SMOKE_ESL = `
namespace smoke = "urn:eigenius:smoke";
namespace core  = "urn:eigenius:core";

class smoke:Widget {
    description = "A widget the topology test loads.";
    requires smoke:label;
}

class smoke:WidgetVariant : smoke:Widget {
    description = "A widget variant — exercises SUBCLASS_OF.";
}

property smoke:label : core:string {
    description = "Label string for a widget.";
}
`;
      const loadResp = await eigen.load(SMOKE_ESL, {
        contentType: "application/x-esl",
        autoCommit: true,
      });
      assert(
        loadResp.success,
        `load failed: ${JSON.stringify(loadResp.errors.map((e) => e.message))}`,
      );

      const withSmoke = await eigen.layerTopology();
      console.log(
        `[+smoke] nodes=${withSmoke.nodes.length}, edges=${withSmoke.edges.length}`,
      );
      console.log(`[+smoke] edge kinds:`, edgeKindHistogram(withSmoke.edges));

      assertGreater(
        withSmoke.nodes.length,
        baseline.nodes.length,
        "loading smoke ontology should add nodes",
      );
      assertGreater(
        withSmoke.edges.length,
        baseline.edges.length,
        "loading smoke ontology should add edges (SUBCLASS_OF + PROPERTY_REF + REQUIRES)",
      );

      const smokeKinds = edgeKindHistogram(withSmoke.edges);
      assert(
        (smokeKinds[EdgeKind[EdgeKind.REQUIRES]] ?? 0) > 0,
        `expected REQUIRES edges after smoke load, got: ${
          JSON.stringify(smokeKinds)
        }`,
      );
      assert(
        (smokeKinds[EdgeKind[EdgeKind.PROPERTY_REF]] ?? 0) > 0,
        `expected PROPERTY_REF edges after smoke load, got: ${
          JSON.stringify(smokeKinds)
        }`,
      );

      // -----------------------------------------------------------------
      // includeResources=true should produce more nodes (each Widget
      // instance becomes its own Resource node) but should NOT drop
      // any edges relative to the taxonomy view.
      // -----------------------------------------------------------------
      const full = await eigen.layerTopology({ includeResources: true });
      console.log(
        `[+resources] nodes=${full.nodes.length}, edges=${full.edges.length}`,
      );
      assertGreater(
        full.nodes.length,
        withSmoke.nodes.length,
        "includeResources should expose more nodes",
      );
      assertGreater(
        full.edges.length,
        0,
        "includeResources should still return edges",
      );
    } finally {
      if (kernel) await kernel.shutdown();
      await orch.shutdown();
    }
  },
});

function countByKind<T>(
  items: ReadonlyArray<T>,
  key: (item: T) => string,
): Record<string, number> {
  const hist: Record<string, number> = {};
  for (const item of items) {
    const k = key(item);
    hist[k] = (hist[k] ?? 0) + 1;
  }
  return hist;
}
