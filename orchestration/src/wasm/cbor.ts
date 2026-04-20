/**
 * CBOR ↔ Eigon-JSON bridge.
 *
 * The WASM side speaks Eigon-CBOR (kernel's `eigon_cbor` module, ciborium
 * under the hood). TS component handlers speak plain JS objects keyed by
 * IRI strings. cbor-x's default encoding round-trips these cleanly: a CBOR
 * map with text keys decodes to `Record<string, any>` and vice versa.
 *
 * The kernel encoder sorts keys for deterministic encoding; cbor-x preserves
 * insertion order. That's fine — the decoder on both sides is order-agnostic
 * per ciborium's `cbor_to_resource`.
 */

import { decode as cborDecode, encode as cborEncode } from "cbor-x";

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
