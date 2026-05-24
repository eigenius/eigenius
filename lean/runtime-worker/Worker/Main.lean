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

import Worker.Ffi

/-!
# `lean-runtime-worker` entry point

Process layout:

1. Substrate spawns this binary with the UDS path as the first
   argv (or via `EIGENIUS_LEAN_WORKER_UDS_PATH` for tests).
2. We call [`Worker.Ffi.listen`] which binds + accepts the
   substrate's connection.
3. The polling loop reads the next request kind, dispatches into
   the matching handler, sends the response.
4. On `Evict` (kind = 4) or transport failure (kind < 0), the
   loop exits and the binary returns.

Per-verb dispatch:
- `Health` (0) → echo a default `Response::Health`.
- `Instantiate` (1) → reply `ready = true`. (v1 has no per-env
  setup beyond what `worker_listen` already did.)
- `RegisterMirror` (2) → reply `MirrorRegistered`. The Rust side
  doesn't currently retain the mirror payload — this verb lights
  up in 20a.6 alongside the mirror generator. For 20a.5b.2 we
  acknowledge so the substrate's connection loop doesn't stall.
- `DispatchMethod` (3) → route by `function_name`:
  - `lean_export` → 20a.5b.3 will land the `lake exe lean4export`
    shell-out; for 20a.5b.2 we reply `DispatchFailed` with a
    clear "pending" diagnostic so a substrate-side smoke test
    sees the dispatch fire end-to-end.
  - Any other name → `DispatchFailed{error_kind="not_implemented"}`.
- `Evict` (4) → send `Response::Evicted`, exit loop.
- `UnsupportedScriptKind` (-3) / `MalformedMethodInvocation` (-4)
  → send `DispatchFailed` with `method_signature_mismatch`.
- `Closed` (-1) / `TransportError` (-2) → exit loop silently
  (peer is gone; nothing to send).
-/

namespace Worker.Main

open Worker.Ffi

/-- Read the worker's UDS path. Lookup order:
1. argv[0] — explicit override (the e2e tests use this).
2. `EIGENIUS_TEST_WORKER_UDS` — the substrate's universal worker
   UDS env var (set by both `LocalServiceSpawner` and
   `DockerServiceSpawner`). Both Julia's worker and the substrate's
   reference test worker read this same name; consistent across
   languages.
3. `EIGENIUS_LEAN_WORKER_UDS_PATH` — Lean-specific fallback for
   ad-hoc invocations.
4. A temp-dir default for the most casual local testing. -/
def resolveUdsPath (args : List String) : IO String := do
  match args with
  | path :: _ => return path
  | [] =>
    match ← IO.getEnv "EIGENIUS_TEST_WORKER_UDS" with
    | some path => return path
    | none =>
      match ← IO.getEnv "EIGENIUS_LEAN_WORKER_UDS_PATH" with
      | some path => return path
      | none => return "/tmp/eigenius-lean-worker.sock"

/-- Convert a Lean `String` to a `ByteArray` for FFI use. Lean's
stdlib provides `String.toUTF8` for this — alias here so the
polling loop reads more declaratively. -/
@[inline] def asBytes (s : String) : ByteArray := s.toUTF8

/-- Inverse of [`asBytes`] — for accessor returns we want to
inspect as text (function_name, env_iri, etc.). Lossy on
ill-formed UTF-8 (returns the replacement-char string), which is
acceptable since the substrate always sends valid UTF-8 IRIs and
function names. -/
@[inline] def asString (b : ByteArray) : String :=
  String.fromUTF8! b

/-- Mint a unique temporary directory under `$TMPDIR` (or `/tmp`)
for one `lean_export` invocation. Uses the worker's PID + the
caller's counter so concurrent invocations on the same worker
don't collide.

We don't use Lean's `IO.FS.createTempDirectory` because Lean 4.29
doesn't expose one — Lake's own tooling builds this from scratch
when it needs it. Our impl is intentionally small: generate a
path, `IO.FS.createDirAll`, return the path. The caller cleans up
with `IO.FS.removeDirAll` after `lake` exits. -/
def mintTempDir (counter : Nat) : IO System.FilePath := do
  let baseDir ← match ← IO.getEnv "TMPDIR" with
    | some d => pure (System.FilePath.mk d)
    | none => pure (System.FilePath.mk "/tmp")
  let pid ← IO.Process.getPID
  let path := baseDir / s!"eigenius-lean-export-{pid}-{counter}"
  IO.FS.createDirAll path
  return path

/-- Capture stdout + stderr from a child process. Returns
`(exitCode, stdout, stderr)`. Used to drive `lake build` /
`lake exe lean4export`. -/
def captureProcess (cmd : String) (args : Array String) (cwd : System.FilePath) :
    IO (UInt32 × ByteArray × ByteArray) := do
  let output ← IO.Process.output {
    cmd := cmd
    args := args
    cwd := some cwd.toString
  }
  return (output.exitCode, output.stdout.toUTF8, output.stderr.toUTF8)

/-- Process-local counter used to disambiguate temp-dir paths
across concurrent `lean_export` invocations on the same worker. -/
initialize tempDirCounter : IO.Ref Nat ← IO.mkRef 0

/-- Real `lean_export` handler (Phase 20a.5b.3).

Flow:
1. Mint a temp dir.
2. Read `input[1]` as the target module name (UTF-8 bytes).
3. Stage the `LeanProject` (from `input[0]`) into the temp dir via
   [`Worker.Ffi.stageLeanProject`].
4. Run `lake build` to compile the project's sources.
5. Run `lake exe lean4export <Module>` to dump the environment.
6. Send the captured stdout as `DispatchOk.output`.
7. Clean up the temp dir.

Each failure point emits `DispatchFailed` with a clear
`error_kind` — `not_implemented` is reserved for handlers that
don't exist at all; here we use `lean_export_failed` for any of
the staging / build / export steps. -/
def runLeanExport (h : WorkerHandle) : IO Unit := do
  -- 1. Temp dir.
  let counter ← tempDirCounter.modifyGet (fun n => (n, n + 1))
  let tempDir ← try
    mintTempDir counter
  catch e =>
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes s!"failed to create temp dir: {e.toString}")
    return

  -- 2. Target module + constant name from input[1] + input[2].
  -- (input[0] is the LeanProject.) Each input ships as an
  -- Eigon-CBOR Resource (the cross-runtime wire format the
  -- substrate uses for every `call_method` input); we ask the
  -- cdylib's Eigon decoder to extract the relevant string
  -- property out of each. Without an explicit constant name,
  -- `lake exe lean4export <Module>` dumps the entire imported
  -- environment — hundreds of MB for any project that imports
  -- Lean stdlib. Requiring the caller to pin a constant keeps
  -- the export bounded.
  let inputCount ← requestInputCount h
  if inputCount < 3 then
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes s!"lean_export requires inputs=[LeanProject, targetModule, targetConstant]; got {inputCount} inputs")
    IO.FS.removeDirAll tempDir
    return
  let moduleCbor ← requestInput h 1
  let moduleBytes ← decodeEigonStringProperty moduleCbor
    (asBytes "urn:eigenius:lean:module_name")
  if moduleBytes.size == 0 then
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes "lean_export input[1] missing `urn:eigenius:lean:module_name` string property")
    IO.FS.removeDirAll tempDir
    return
  let targetModule := asString moduleBytes
  let constantCbor ← requestInput h 2
  let constantBytes ← decodeEigonStringProperty constantCbor
    (asBytes "urn:eigenius:lean:constant_name")
  if constantBytes.size == 0 then
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes "lean_export input[2] missing `urn:eigenius:lean:constant_name` string property")
    IO.FS.removeDirAll tempDir
    return
  let targetConstant := asString constantBytes

  -- 3. Stage the project files to disk.
  let stagingError ← stageLeanProject h 0 (asBytes tempDir.toString)
  if stagingError.size != 0 then
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes s!"staging failed: {asString stagingError}")
    IO.FS.removeDirAll tempDir
    return

  -- 4. lake build compiles the project's sources so lean4export
  -- has .olean artifacts to read from.
  let (buildExit, _buildStdout, buildStderr) ← captureProcess "lake" #["build"] tempDir
  if buildExit != 0 then
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes s!"lake build failed (exit {buildExit}): {asString buildStderr}")
    IO.FS.removeDirAll tempDir
    return

  -- 5. `lake exe lean4export <Module> -- <Constant>` — the `--`
  -- separates lean4export's module-name args from its
  -- constant-name args; without an explicit constant, lean4export
  -- dumps the entire imported environment.
  let (exportExit, exportStdout, exportStderr) ←
    captureProcess "lake" #["exe", "lean4export", targetModule, "--", targetConstant] tempDir
  if exportExit != 0 then
    sendDispatchFailed h (asBytes "lean_export_failed")
      (asBytes s!"lake exe lean4export failed (exit {exportExit}): {asString exportStderr}")
    IO.FS.removeDirAll tempDir
    return

  -- 6. Send the export bytes back to the substrate.
  sendDispatchOk h exportStdout (ByteArray.mk #[])

  -- 7. Clean up the temp dir.
  IO.FS.removeDirAll tempDir

/-- Dispatch table for `Request::DispatchMethod`. Lean reads
`function_name` from the in-flight slot and routes to the matching
handler. Unknown functions surface as `DispatchFailed`. -/
def dispatchMethod (h : WorkerHandle) : IO Unit := do
  let fnNameBytes ← requestFunctionName h
  let fnName := asString fnNameBytes
  if fnName == "lean_export" then
    runLeanExport h
  else
    let errorKind := asBytes "not_implemented"
    let message := asBytes s!"Lean worker has no handler for function `{fnName}`"
    sendDispatchFailed h errorKind message

/-- Discriminator values matching the Rust `RequestKind` enum in
[`crates/eigenius-lean-worker/src/lib.rs`](../../crates/eigenius-lean-worker/src/lib.rs).
Lean's `Int32` doesn't support pattern-matching against integer
literals directly, so we expose the values as named constants the
`runLoop` if-chain compares against. -/
def kindHealth : Int32 := 0
def kindInstantiate : Int32 := 1
def kindRegisterMirror : Int32 := 2
def kindDispatchMethod : Int32 := 3
def kindEvict : Int32 := 4
def kindClosed : Int32 := -1
def kindTransportError : Int32 := -2
def kindUnsupportedScriptKind : Int32 := -3
def kindMalformedMethodInvocation : Int32 := -4

/-- The main polling loop. Each iteration: read the next request
kind, dispatch on it, send a response, repeat. Exits when
`Evict` is sent or the peer disconnects. -/
partial def runLoop (h : WorkerHandle) : IO Unit := do
  let kind ← nextRequestKind h
  if kind == kindHealth then
    sendHealth h
    runLoop h
  else if kind == kindInstantiate then
    sendInstantiated h true
    runLoop h
  else if kind == kindRegisterMirror then
    -- 20a.5b.2: ack the registration without retaining the
    -- archive. 20a.6's mirror generator + 20a.7's correspondence
    -- check will light this up properly. The mirror_iri we echo
    -- back must match what the substrate sent — read it out and
    -- use it.
    let iriBytes ← requestMirrorIri h
    sendMirrorRegistered h iriBytes
    runLoop h
  else if kind == kindDispatchMethod then
    dispatchMethod h
    runLoop h
  else if kind == kindEvict then
    sendEvicted h
    -- Loop exits — substrate has signalled shutdown.
    return
  else if kind == kindUnsupportedScriptKind || kind == kindMalformedMethodInvocation then
    -- The Rust side stashed the invocation_id in the in-flight
    -- slot so we can still build a DispatchFailed response.
    let errorKind := asBytes "method_signature_mismatch"
    let message := asBytes (
      if kind == kindUnsupportedScriptKind then
        "Lean worker only handles target_kind = Method"
      else
        "MethodInvocation decode failed"
    )
    sendDispatchFailed h errorKind message
    runLoop h
  else if kind == kindClosed then
    -- Peer closed cleanly. The substrate opens one UDS connection
    -- per RPC (Health and DispatchMethod are separate dials per
    -- D26 §8.1), so an EOF here means "this RPC is done, wait for
    -- the next." Accept the next connection on the same listener
    -- and loop back; a non-zero acceptNext return means the
    -- listener itself is dead and the worker should exit.
    let rc ← acceptNext h
    if rc != 0 then
      IO.eprintln s!"eigenius-lean-worker: acceptNext returned {rc}; exiting"
      return
    runLoop h
  else if kind == kindTransportError then
    -- CBOR decode or I/O failure on the current connection. Same
    -- recovery as a clean close: roll over to the next connection.
    -- A transport-level error on one RPC shouldn't tear down the
    -- worker — the next RPC is a fresh frame on a fresh stream.
    let rc ← acceptNext h
    if rc != 0 then
      IO.eprintln s!"eigenius-lean-worker: acceptNext (after transport error) returned {rc}; exiting"
      return
    runLoop h
  else
    IO.eprintln s!"eigenius-lean-worker: unknown request kind {kind}; exiting"
    return

/-- Worker entry point. Resolves the UDS path, binds + accepts via
[`listen`], runs the polling loop until exit. -/
def run (args : List String) : IO Unit := do
  let udsPath ← resolveUdsPath args
  IO.eprintln s!"eigenius-lean-worker: binding UDS at {udsPath}"
  let h ← listen (asBytes udsPath)
  runLoop h

end Worker.Main

/-- The `lean_exe` target's `main`. Lean's runtime hands argv to
`main : List String → IO UInt32` (or `IO Unit`); we delegate to
the worker's `run`. -/
def main (args : List String) : IO Unit := Worker.Main.run args
