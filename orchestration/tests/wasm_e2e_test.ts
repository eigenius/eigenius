/**
 * End-to-end WASM IO component test: spawns a real kernel and orchestrator
 * as subprocesses, installs `wasm-http-shout` via the kernel's Load RPC,
 * invokes it via RunProgram, and verifies the dispatch → mock-CompleteText
 * round-trip flows all the way back.
 *
 * This is the most integrated test we have — unlike `wasm_shout_test.ts`
 * (which stays in-process) this exercises the kernel-side scan, the
 * `RegisterWasmComponent` gRPC call, `RemoteComponent` dispatch, and the
 * Eigon-JSON ↔ CBOR transcoding at the orchestrator boundary.
 *
 * Run with:
 *   deno test --allow-read --allow-ffi --allow-env --allow-sys \
 *     --allow-net --allow-run --unstable-node-globals --unstable-detect-cjs \
 *     tests/wasm_e2e_test.ts
 *
 * Prerequisites (test skips with a message if any are missing):
 *   • `cargo build` has produced target/debug/eigenius
 *   • `deno task build:addon` has produced native/index.js + .node
 *   • wasm-http-shout fixture is in kernel/tests/fixtures/
 */

import { assert, assertEquals, assertStringIncludes } from "@std/assert";
import { encodeBase64 } from "jsr:@std/encoding@^1/base64";
import { KernelClient } from "../src/client/kernel_client.ts";
import { decodeResource } from "../src/wasm/cbor.ts";

const SHOUT_IRI = "urn:example:components:HttpShout";
const MOCK_RESPONSE = "This is a mock LLM response.";

const REPO_ROOT = new URL("../../", import.meta.url);
const KERNEL_BIN = new URL("./target/debug/eigenius", REPO_ROOT).pathname;
const ADDON_JS = new URL("./orchestration/native/index.js", REPO_ROOT).pathname;
const FIXTURE = new URL(
  "./kernel/tests/fixtures/eigenius_wasm_http_shout.wasm",
  REPO_ROOT,
).pathname;
const ORCH_ENTRY = new URL("./orchestration/src/main.ts", REPO_ROOT).pathname;

/** Skip the test with a clear message if a prerequisite is missing. */
async function checkPrerequisites(): Promise<string | null> {
  for (const [label, path] of [
    ["kernel binary (run `cargo build`)", KERNEL_BIN],
    ["native addon (run `deno task build:addon`)", ADDON_JS],
    ["wasm-http-shout fixture", FIXTURE],
  ]) {
    try {
      await Deno.stat(path);
    } catch {
      return `missing ${label}: ${path}`;
    }
  }
  return null;
}

/** Grab an ephemeral free port by briefly opening a listener. */
function pickPort(): number {
  const listener = Deno.listen({ port: 0 });
  const port = (listener.addr as Deno.NetAddr).port;
  listener.close();
  return port;
}

/** Poll a predicate until it returns truthy, or give up. */
async function waitFor(
  check: () => Promise<boolean>,
  { timeoutMs = 15_000, intervalMs = 100, label = "condition" } = {},
): Promise<void> {
  const start = performance.now();
  while (performance.now() - start < timeoutMs) {
    try {
      if (await check()) return;
    } catch {
      // swallow — we'll retry until timeout
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  throw new Error(`timed out waiting for ${label}`);
}

interface Spawned {
  child: Deno.ChildProcess;
  shutdown: () => Promise<void>;
}

/**
 * Spawn a subprocess, capture stdio to files in a temp dir (useful for
 * diagnostics when the test fails), and return a shutdown handle that
 * SIGTERM-s and waits for exit.
 */
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

  // Drain the streams so the child doesn't stall on full buffers; echo to
  // parent stderr with a label prefix so the test log is interleaved.
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

  const shutdown = async () => {
    try {
      child.kill("SIGTERM");
    } catch {
      // already exited
    }
    await child.status;
  };

  return { child, shutdown };
}

Deno.test({
  name: "e2e: kernel + orchestrator install & invoke wasm-http-shout",
  // Exclusive — avoids port collisions with other tests in the suite.
  sanitizeOps: false,
  sanitizeResources: false,
  async fn() {
    const missing = await checkPrerequisites();
    if (missing) {
      console.warn(`skipping e2e: ${missing}`);
      return;
    }

    const orchPort = pickPort();
    const kernelPort = pickPort();

    const denoFlags = [
      "run",
      "--allow-net",
      "--allow-read",
      "--allow-ffi",
      "--allow-env",
      "--allow-sys",
      "--unstable-node-globals",
      "--unstable-detect-cjs",
      ORCH_ENTRY,
    ];
    const orch = spawn(
      "orch",
      Deno.execPath(),
      denoFlags,
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
      // Orchestrator first — the kernel will connect to it on startup.
      await waitFor(
        async () => {
          const resp = await fetch(`http://localhost:${orchPort}/health`);
          await resp.body?.cancel();
          return resp.ok;
        },
        { label: "orchestrator /health", timeoutMs: 30_000 },
      );

      kernel = spawn(
        "kern",
        KERNEL_BIN,
        [
          "serve",
          "--port",
          String(kernelPort),
          "--orchestrator",
          `http://localhost:${orchPort}`,
        ],
        { ...Deno.env.toObject() },
      );

      const client = new KernelClient(`http://localhost:${kernelPort}`);
      await waitFor(async () => (await client.health()).healthy, {
        label: "kernel Health RPC",
        timeoutMs: 30_000,
      });

      // ---------------------------------------------------------------
      // Install: feed a layer containing the wasm-http-shout component
      // resource. Kernel's scan picks out `capability_level=io` and
      // forwards to the orchestrator.
      // ---------------------------------------------------------------
      const wasmBytes = await Deno.readFile(FIXTURE);
      const resourceDoc = [{
        "@id": SHOUT_IRI,
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
        "urn:eigenius:core:short_name": "HttpShout",
        "urn:eigenius:program:component:input_type": "urn:eigenius:core:Class",
        "urn:eigenius:program:component:output_type": "urn:eigenius:core:Class",
        "urn:eigenius:program:component:capability_level":
          "urn:eigenius:program:capability_levels:io",
        "urn:eigenius:program:component:implementation": "wasm",
        "urn:eigenius:program:component:wasm_binary": encodeBase64(wasmBytes),
      }];

      const loadResp = await client.load(JSON.stringify(resourceDoc));
      assert(
        loadResp.success,
        `load failed: ${JSON.stringify(loadResp.errors.map((e) => e.message))}`,
      );

      // ---------------------------------------------------------------
      // Invoke: run a trivial program that applies the component to our
      // input. Mirrors what `eigenius capability test` does.
      // ---------------------------------------------------------------
      const programJson = JSON.stringify({
        "@id": "urn:test:e2e:program",
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
        "urn:eigenius:program:input_type": "urn:eigenius:core:Class",
        "urn:eigenius:program:output_type": "urn:eigenius:core:Class",
        "urn:eigenius:program:body": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
          "urn:eigenius:program:function": SHOUT_IRI,
          "urn:eigenius:program:argument": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "input",
          },
        },
      });
      const inputJson = JSON.stringify({
        "@id": "urn:test:e2e:input",
        "urn:example:shout:text": "hello from e2e",
      });

      const runResp = await client.runProgram(programJson, inputJson);
      assert(
        runResp.success,
        `runProgram failed: ${JSON.stringify(runResp.errors.map((e) => e.message))}`,
      );

      // RunProgram returns output as Eigon-CBOR (see kernel server
      // serialize_resource). Decode with the same codec the orchestrator
      // uses at the WASM boundary.
      const output = decodeResource(runResp.output);
      const shouted = output["urn:example:shout:shouted"];

      assertEquals(
        typeof shouted,
        "string",
        `expected shouted string, got: ${JSON.stringify(output)}`,
      );
      // Mock CompleteText returns this fixed string regardless of input.
      assertStringIncludes(shouted as string, MOCK_RESPONSE);
    } finally {
      if (kernel) await kernel.shutdown();
      await orch.shutdown();
    }
  },
});
