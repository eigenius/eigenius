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
 * Eigenius Orchestration Layer
 *
 * The Deno/TypeScript orchestration layer sits above the kernel service API
 * and handles IO component execution, LLM adapter management, and the MCP
 * server surface.
 *
 * Architecture reference: §2.2 (Host Layer)
 */

import { KernelClient } from "./client/kernel_client.ts";
import { ComponentRegistry } from "./components/registry.ts";
import {
  COMPLETE_TEXT_IRI,
  createCompleteTextHandler,
  createMockCompleteTextHandler,
} from "./components/complete_text.ts";
import {
  COMPLETE_JSON_IRI,
  createCompleteJsonHandler,
  createMockCompleteJsonHandler,
} from "./components/complete_json.ts";
import * as log from "./observability/mod.ts";
import { operation } from "./observability/mod.ts";
import { ProgramExecutor } from "./program/executor.ts";
import { startServer } from "./server/mod.ts";
import { tryLoadRuntimeSubstrateAddon } from "./runtime/loadAddon.ts";
import {
  createRunRuntimeScriptHandler,
  RUN_RUNTIME_SCRIPT_IRI,
} from "./components/run_runtime_script.ts";
import {
  CALL_RUNTIME_METHOD_IRI,
  createCallRuntimeMethodHandler,
} from "./components/call_runtime_method.ts";
import { createMcpServer } from "./mcp/server.ts";
import { createMcpHttpHandler } from "./mcp/http.ts";

const KERNEL_ENDPOINT = Deno.env.get("EIGENIUS_KERNEL_ENDPOINT") ??
  "http://localhost:50051";
const ORCHESTRATOR_PORT = parseInt(
  Deno.env.get("EIGENIUS_ORCHESTRATOR_PORT") ?? "8080",
);
const USE_MOCK_LLM = Deno.env.get("EIGENIUS_MOCK_LLM") === "true";

/** Path to the substrate's `eigenius-test-worker` binary, used by the
 * default bash-c TestLanguageRuntime. Optional — if unset, the
 * substrate has no language registered and `RunRuntimeScript`
 * dispatches surface a typed `UnknownLanguage` error. Phase 19+
 * replaces this with per-language runtime registration. */
const TEST_WORKER_BIN = Deno.env.get("EIGENIUS_TEST_WORKER_BIN");

/** Configuration for the Julia language runtime. All three must be
 * set for Julia to register; if any is missing the runtime stays
 * unregistered and Julia dispatches surface a typed `UnknownLanguage`
 * error. The defaults are tailored to the production orchestrator
 * container layout (`/opt/eigenius/julia-runtime-worker/` for the
 * worker source, `/var/lib/eigenius/substrate-depot/` for the shared
 * depot). */
const JULIA_WORKER_PROJECT_DIR = Deno.env.get(
  "EIGENIUS_JULIA_WORKER_PROJECT_DIR",
);
const JULIA_BASE_IMAGE_REF = Deno.env.get("EIGENIUS_JULIA_BASE_IMAGE_REF");
const JULIA_DEPOT_PATH = Deno.env.get("EIGENIUS_JULIA_DEPOT_PATH");

/** Configuration for the Lean language runtime. All five must be set
 * for Lean to register; if any is missing the runtime stays
 * unregistered and Lean dispatches surface a typed `UnknownLanguage`
 * error. The substrate path is wider than Julia's because Lean stages
 * three host-side artifacts: the Lake worker project, the cdylib the
 * worker links against, and the EigeniusLeanCommon Lake package the
 * generated mirrors depend on. Phase 20a.5 wiring. */
const LEAN_WORKER_PROJECT_DIR = Deno.env.get(
  "EIGENIUS_LEAN_WORKER_PROJECT_DIR",
);
const LEAN_CDYLIB_PATH = Deno.env.get("EIGENIUS_LEAN_CDYLIB_PATH");
const LEAN_COMMON_DIR = Deno.env.get("EIGENIUS_LEAN_COMMON_DIR");
const LEAN_BASE_IMAGE_REF = Deno.env.get("EIGENIUS_LEAN_BASE_IMAGE_REF");
const LEAN_DEPOT_PATH = Deno.env.get("EIGENIUS_LEAN_DEPOT_PATH");

/** Configuration for the R language runtime (D55 / D56). All four must
 * be set for R to register; if any is missing the runtime stays
 * unregistered and R dispatches surface a typed `UnknownLanguage`
 * error. `DRIVER_PATH`/`CDYLIB_PATH` are the host paths to
 * `EigeniusRWorker.R` + `libeigenius_r_worker.so` baked into the image;
 * they must match the recipe that built the R worker image so the boot
 * cross-check passes (D26 §9.3). The image itself is resolved per
 * dispatch from the env Resource's `image_digest`. */
const R_DRIVER_PATH = Deno.env.get("EIGENIUS_R_DRIVER_PATH");
const R_CDYLIB_PATH = Deno.env.get("EIGENIUS_R_CDYLIB_PATH");
const R_BASE_IMAGE_REF = Deno.env.get("EIGENIUS_R_BASE_IMAGE_REF");
const R_DEPOT_PATH = Deno.env.get("EIGENIUS_R_DEPOT_PATH");
// Generic OCI tool runtime (D60). The worker binary is a pinned Eigenius worker
// (e.g. eigenius-schemaorg-worker) staged into the orchestrator image.
const OCI_WORKER_BINARY_PATH = Deno.env.get("EIGENIUS_OCI_WORKER_BINARY_PATH");
const OCI_BASE_IMAGE_REF = Deno.env.get("EIGENIUS_OCI_BASE_IMAGE_REF");
const OCI_DEPOT_PATH = Deno.env.get("EIGENIUS_OCI_DEPOT_PATH");

function main() {
  // Install the structured-logging subscriber before anything else
  // emits an event. Reads `EIGENIUS_LOG_LEVEL` and
  // `EIGENIUS_LOG_FORMAT` from env (same envelope as the kernel).
  log.init();

  log.info(operation.SERVER_START, "Eigenius orchestration layer starting", {
    kernel_endpoint: KERNEL_ENDPOINT,
    mock_llm: USE_MOCK_LLM,
  });

  const client = new KernelClient(KERNEL_ENDPOINT);
  const components = new ComponentRegistry();

  // Register LLM components
  if (USE_MOCK_LLM) {
    log.info(
      operation.COMPONENT_REGISTER,
      "registered LLM components in mock mode",
      { mock_llm: true },
    );
    components.register(COMPLETE_TEXT_IRI, createMockCompleteTextHandler());
    components.register(COMPLETE_JSON_IRI, createMockCompleteJsonHandler());
  } else {
    components.register(COMPLETE_TEXT_IRI, createCompleteTextHandler());
    components.register(COMPLETE_JSON_IRI, createCompleteJsonHandler());
  }

  const _executor = new ProgramExecutor(client, components);

  // Native addon for the runtime substrate (RunRuntimeScript /
  // CallRuntimeMethod). Optional — skipped if not built. Phase 18a.
  const substrateAddon = tryLoadRuntimeSubstrateAddon();
  if (substrateAddon) {
    if (TEST_WORKER_BIN) {
      try {
        substrateAddon.registerTestLanguageRuntime(TEST_WORKER_BIN);
        log.info(
          operation.COMPONENT_REGISTER,
          "registered bash-c TestLanguageRuntime",
          { worker_binary: TEST_WORKER_BIN },
        );
      } catch (e) {
        log.warn(
          operation.COMPONENT_REGISTER,
          "failed to register TestLanguageRuntime",
          {
            error_kind: "test_runtime_register_failed",
            error_message: e instanceof Error ? e.message : String(e),
          },
        );
      }
    }
    if (
      JULIA_WORKER_PROJECT_DIR && JULIA_BASE_IMAGE_REF && JULIA_DEPOT_PATH
    ) {
      try {
        substrateAddon.registerJuliaLanguageRuntime(
          JULIA_WORKER_PROJECT_DIR,
          JULIA_BASE_IMAGE_REF,
          JULIA_DEPOT_PATH,
        );
        log.info(
          operation.COMPONENT_REGISTER,
          "registered JuliaLanguageRuntime",
          {
            worker_project_dir: JULIA_WORKER_PROJECT_DIR,
            base_image_ref: JULIA_BASE_IMAGE_REF,
            depot_path: JULIA_DEPOT_PATH,
          },
        );
      } catch (e) {
        log.warn(
          operation.COMPONENT_REGISTER,
          "failed to register JuliaLanguageRuntime",
          {
            error_kind: "julia_runtime_register_failed",
            error_message: e instanceof Error ? e.message : String(e),
          },
        );
      }
    }
    if (
      LEAN_WORKER_PROJECT_DIR && LEAN_CDYLIB_PATH && LEAN_COMMON_DIR &&
      LEAN_BASE_IMAGE_REF && LEAN_DEPOT_PATH
    ) {
      try {
        substrateAddon.registerLeanLanguageRuntime(
          LEAN_WORKER_PROJECT_DIR,
          LEAN_CDYLIB_PATH,
          LEAN_COMMON_DIR,
          LEAN_BASE_IMAGE_REF,
          LEAN_DEPOT_PATH,
        );
        log.info(
          operation.COMPONENT_REGISTER,
          "registered LeanLanguageRuntime",
          {
            worker_project_dir: LEAN_WORKER_PROJECT_DIR,
            cdylib_path: LEAN_CDYLIB_PATH,
            lean_common_dir: LEAN_COMMON_DIR,
            base_image_ref: LEAN_BASE_IMAGE_REF,
            depot_path: LEAN_DEPOT_PATH,
          },
        );
      } catch (e) {
        log.warn(
          operation.COMPONENT_REGISTER,
          "failed to register LeanLanguageRuntime",
          {
            error_kind: "lean_runtime_register_failed",
            error_message: e instanceof Error ? e.message : String(e),
          },
        );
      }
    }
    if (
      R_DRIVER_PATH && R_CDYLIB_PATH && R_BASE_IMAGE_REF && R_DEPOT_PATH
    ) {
      try {
        substrateAddon.registerRLanguageRuntime(
          R_DRIVER_PATH,
          R_CDYLIB_PATH,
          R_BASE_IMAGE_REF,
          R_DEPOT_PATH,
        );
        log.info(
          operation.COMPONENT_REGISTER,
          "registered RLanguageRuntime",
          {
            driver_path: R_DRIVER_PATH,
            cdylib_path: R_CDYLIB_PATH,
            base_image_ref: R_BASE_IMAGE_REF,
            depot_path: R_DEPOT_PATH,
          },
        );
      } catch (e) {
        log.warn(
          operation.COMPONENT_REGISTER,
          "failed to register RLanguageRuntime",
          {
            error_kind: "r_runtime_register_failed",
            error_message: e instanceof Error ? e.message : String(e),
          },
        );
      }
    }
    if (OCI_WORKER_BINARY_PATH && OCI_BASE_IMAGE_REF && OCI_DEPOT_PATH) {
      try {
        substrateAddon.registerOciToolRuntime(
          OCI_WORKER_BINARY_PATH,
          OCI_BASE_IMAGE_REF,
          OCI_DEPOT_PATH,
        );
        log.info(
          operation.COMPONENT_REGISTER,
          "registered OciToolRuntime",
          {
            worker_binary_path: OCI_WORKER_BINARY_PATH,
            base_image_ref: OCI_BASE_IMAGE_REF,
            depot_path: OCI_DEPOT_PATH,
          },
        );
      } catch (e) {
        log.warn(
          operation.COMPONENT_REGISTER,
          "failed to register OciToolRuntime",
          {
            error_kind: "oci_runtime_register_failed",
            error_message: e instanceof Error ? e.message : String(e),
          },
        );
      }
    }
    components.register(
      RUN_RUNTIME_SCRIPT_IRI,
      createRunRuntimeScriptHandler(substrateAddon),
    );
    components.register(
      CALL_RUNTIME_METHOD_IRI,
      createCallRuntimeMethodHandler(substrateAddon),
    );
    log.info(
      operation.COMPONENT_REGISTER,
      "runtime substrate components enabled",
      {
        languages: substrateAddon.listRegisteredLanguages(),
      },
    );
  } else {
    log.warn(
      operation.ADDON_LOAD,
      "runtime substrate disabled (addon not loaded; " +
        "RunRuntimeScript / CallRuntimeMethod will fail)",
      { enabled: false },
    );
  }

  log.info(
    operation.SERVER_START,
    "component registry initialised",
    {
      count: components.listComponents().length,
      components: components.listComponents(),
    },
  );

  // MCP server — exposes a curated subset of the kernel surface to
  // LLM agents over the orchestrator's HTTP port (`/mcp`). Stateless
  // JSON-response mode; see `mcp/http.ts`. The SDK requires a fresh
  // server + transport pair per request in stateless mode, so we pass
  // a builder rather than a built instance.
  const mcpHandler = createMcpHttpHandler(() => createMcpServer(client));

  // Start the orchestrator server (gRPC + NotebookService + health + MCP).
  // Pass the substrate addon explicitly so the `DispatchExternal` RPC
  // (D31 §6.2) can route into the same handle that powers
  // `RunRuntimeScript` / `CallRuntimeMethod`.
  startServer(
    components,
    client,
    ORCHESTRATOR_PORT,
    substrateAddon ?? undefined,
    mcpHandler,
  );
}

main();
