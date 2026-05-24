/-
Copyright 2026 The Eigenius Authors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-/

import Lake
open Lake DSL

/-!
# Lake project for the Eigenius Lean runtime worker

Builds `lean-runtime-worker` — a Lake-driven Lean executable that
links against the `eigenius-lean-worker` Rust cdylib for substrate
transport (UDS + CBOR framing) and dispatches the per-verb worker
handlers in Lean.

## Link discipline

`extraLinkArgs` does three things:
1. `-L../../target/debug` — search path for the Rust cdylib.
   The Cargo workspace puts it under `<root>/target/debug/`;
   the lakefile sits at `<root>/lean/runtime-worker/`.
2. `-leigenius_lean_worker` — link the cdylib.
3. `-Wl,-rpath,$ORIGIN/../../../../../target/debug` — embed the
   runtime search path so the produced binary finds the cdylib
   without `LD_LIBRARY_PATH` plumbing. The five `..` segments
   walk back from the binary's installed location
   (`.lake/build/bin/<exe>` → workspace root → `target/debug`).

## Why we don't `-lLean` here

Lake builds executables that always link against the Lean runtime
shared library (`libleanshared.so`), which exports the
`lean_register_external_class` / `lean_alloc_object` /
`lean_internal_panic_*` symbols our cdylib references. So the
cdylib's undefined Lean symbols resolve transitively through
Lake's standard link line — no extra args needed.
-/

package «eigenius-lean-worker-lake» where
  -- 20a.5b.2: skip precompilation of the Worker module so iterative
  -- development doesn't waste cycles building the lib target before
  -- the exe. Lake will still compile + link normally on `lake build`.
  precompileModules := false

@[default_target]
lean_exe «lean-runtime-worker» where
  root := `Worker.Main
  -- Pinned for the Rust cdylib link. Production deployments running
  -- against a non-debug build set LEAN_WORKER_LIB_DIR + manually
  -- relink; for 20a.5b.2's local-mode test the debug path is what
  -- the Rust workspace produces.
  moreLinkArgs := #[
    "-L../../target/debug",
    "-leigenius_lean_worker",
    "-Wl,-rpath,$ORIGIN/../../../../../target/debug"
  ]

lean_lib Worker where
  -- All sources under `Worker/`. Lake auto-discovers them via the
  -- module-path-to-file mapping.
