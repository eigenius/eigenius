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
 * CBOR ↔ Eigon-JSON bridge.
 *
 * The kernel speaks Eigon-CBOR (kernel's `eigon_cbor` module, ciborium
 * under the hood) on the component-executor and runtime-substrate wire.
 * TS component handlers speak plain JS objects keyed by IRI strings.
 * cbor-x's default encoding round-trips these cleanly: a CBOR map with
 * text keys decodes to `Record<string, any>` and vice versa.
 *
 * The kernel encoder sorts keys for deterministic encoding; cbor-x preserves
 * insertion order. That's fine — the decoder on both sides is order-agnostic
 * per ciborium's `cbor_to_resource`.
 */

import {
  addExtension,
  decode as cborDecode,
  encode as cborEncode,
  Tag,
} from "cbor-x";

/**
 * CBOR tag the kernel uses to mark `Value::Json` payloads on the wire
 * so they decode back to `Value::Json` rather than `Value::Embedded(Resource)`.
 * See `kernel/src/ontology/eigon_cbor.rs`'s `EIGENIUS_JSON_TAG` const —
 * the values must match exactly. Without this decoder hook, cbor-x
 * surfaces the tag as a `Tag { value, tag }` wrapper rather than the
 * inner JS object — which then fails handler-side shape checks
 * (e.g. `CompleteJson`'s `isShortNameTable`).
 */
const EIGENIUS_JSON_TAG = 27182;

addExtension({
  Class: Tag,
  tag: EIGENIUS_JSON_TAG,
  // Encode hook: if the JS side ever produces a `Tag` with this id
  // (we don't, but the hook is required for `addExtension`), pass
  // through the inner value. The kernel-side encoder is what stamps
  // the tag on the wire.
  encode(t: Tag, encodeFn: (v: unknown) => Uint8Array): Uint8Array {
    return encodeFn(t.value);
  },
  // Decode hook: unwrap to the inner JS value. The handlers
  // (`CompleteJson` shortNameTable, generic JSON-typed properties)
  // expect plain objects, not `Tag` wrappers.
  decode(value: unknown) {
    return value;
  },
});

// deno-lint-ignore no-explicit-any
export type EigonResource = Record<string, any>;

/** Encode a plain JS object to Eigon-CBOR bytes. */
export function encodeResource(resource: EigonResource): Uint8Array {
  return cborEncode(resource);
}

/** Decode Eigon-CBOR bytes to a plain JS object. */
export function decodeResource(bytes: Uint8Array): EigonResource {
  if (bytes.length === 0) return {};
  return cborDecode(bytes) as EigonResource;
}
