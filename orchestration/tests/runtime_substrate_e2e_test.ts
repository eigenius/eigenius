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
 * Phase 18a end-to-end: ComponentInput → orchestrator handler →
 * runtime-substrate napi addon → SubstrateDispatcher →
 * TestLanguageRuntime → bash worker → Eigon-CBOR Resource → back.
 *
 * In-process, no spawned kernel — the test directly invokes the
 * handler the orchestrator's main.ts wires against the
 * `RunRuntimeScript` component IRI.
 *
 * Skipped if either prerequisite is missing:
 *   • runtime-substrate-native built (run `napi build` from the addon
 *     dir, or `deno task build:addon` in a future task)
 *   • eigenius-test-worker built (run `cargo build`)
 */

import { assert, assertEquals } from "@std/assert";
import { tryLoadRuntimeSubstrateAddon } from "../src/runtime/loadAddon.ts";
import { createRunRuntimeScriptHandler } from "../src/components/run_runtime_script.ts";
import { createCallRuntimeMethodHandler } from "../src/components/call_runtime_method.ts";

const REPO_ROOT = new URL("../../", import.meta.url);
const ADDON_JS = new URL(
  "./orchestration/runtime-substrate-native/index.js",
  REPO_ROOT,
).pathname;
const TEST_WORKER_BIN = new URL(
  "./target/debug/eigenius-test-worker",
  REPO_ROOT,
).pathname;

async function checkPrerequisites(): Promise<string | null> {
  for (
    const [label, path] of [
      [
        "runtime-substrate native addon (run `cd orchestration/runtime-substrate-native && napi build --platform`)",
        ADDON_JS,
      ],
      ["eigenius-test-worker (run `cargo build`)", TEST_WORKER_BIN],
    ] as const
  ) {
    try {
      await Deno.stat(path);
    } catch {
      return `missing ${label}: ${path}`;
    }
  }
  return null;
}

Deno.test(
  "runtime-substrate e2e: RunRuntimeScript dispatches bash echo through the orchestrator handler",
  async (t) => {
    const skip = await checkPrerequisites();
    if (skip) {
      console.log(`Skipping: ${skip}`);
      return;
    }

    const addon = tryLoadRuntimeSubstrateAddon();
    assert(addon, "addon load");

    // Idempotency-tolerant: if the addon was already loaded by another
    // test (the OnceLock + Mutex inside the addon is process-singleton),
    // re-registering the same language errors with AlreadyRegistered.
    // Treat that as a no-op so test ordering doesn't matter.
    try {
      addon.registerTestLanguageRuntime(TEST_WORKER_BIN);
    } catch (e) {
      const msg = (e as Error).message;
      if (!msg.includes("already registered")) throw e;
    }

    await t.step(
      "RunRuntimeScript round-trips through bash worker",
      async () => {
        const handler = createRunRuntimeScriptHandler(addon);
        const result = await handler({
          input: {},
          argument: {
            "urn:eigenius:runtime:language": "test",
            "urn:eigenius:runtime:source": "echo orchestrator-validated",
          },
        });
        const stdout = result.output["urn:eigenius:test:bash_stdout"];
        assert(
          typeof stdout === "string",
          "expected bash_stdout to be a string",
        );
        assertEquals((stdout as string).trim(), "orchestrator-validated");
        assertEquals(
          result.output["urn:eigenius:runtime:language"],
          "test",
        );
      },
    );

    await t.step(
      "CallRuntimeMethod against TestLanguageRuntime errors with method_signature_mismatch",
      async () => {
        const handler = createCallRuntimeMethodHandler(addon);
        let caught: Error | undefined;
        try {
          await handler({
            input: {},
            argument: {
              "urn:eigenius:runtime:language": "test",
              "urn:eigenius:runtime:source": "echo unused",
            },
          });
        } catch (e) {
          caught = e as Error;
        }
        assert(caught, "expected handler to throw");
        // The substrate's MethodSignatureMismatch surfaces verbatim
        // through the napi boundary.
        assert(
          caught.message.toLowerCase().includes("method") ||
            caught.message.toLowerCase().includes("signature"),
          `expected method-signature error, got: ${caught.message}`,
        );
      },
    );
  },
);
