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

/-- Read the worker's UDS path from argv or fall back to an env
var, with a final temp-dir default for ad-hoc local testing. -/
def resolveUdsPath (args : List String) : IO String := do
  match args with
  | path :: _ => return path
  | [] =>
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

/-- The lean_export handler stub. 20a.5b.3 will replace this with
the real flow:

1. Read the `LeanProject` resource from `requestInput h 0` (CBOR-
   encoded by the substrate via the kernel's eigon_cbor codec).
2. Decode the project's `lakefile.lean` / `lake-manifest.json` /
   `source_tree` fields.
3. Stage the files under a temp directory.
4. Spawn `lake exe lean4export` via `IO.Process.spawn`.
5. Read the resulting export file, return its bytes as the
   `DispatchOk.output` payload.

For 20a.5b.2 we reply with a clearly-marked failure so a
substrate-side smoke test can see the routing land end-to-end
without depending on a working Lake invocation. -/
def runLeanExport (h : WorkerHandle) : IO Unit := do
  let errorKind := asBytes "not_implemented"
  let message := asBytes "lean_export shell-out lands in Phase 20a.5b.3 — Rust FFI bridge is wired but the Lake invocation isn't yet"
  sendDispatchFailed h errorKind message

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
  else if kind == kindClosed || kind == kindTransportError then
    -- Peer closed cleanly or transport broke. Nothing to send;
    -- exit the loop.
    return
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
