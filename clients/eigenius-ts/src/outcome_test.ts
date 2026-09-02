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

//! `Outcome` narrowing — the shape that made a refused load readable as success.
//!
//! The kernel reports refusal IN the response: 21 of its response messages carry
//! `bool success`, and `Load` answers `Ok(LoadResponse { success: false, errors, layer_id: "" })`.
//! A caller reading `resp.layerId` off that gets `""`, which renders as nothing and reads as
//! success. That shipped: a notebook reported "landed as layer " for an artifact the validator
//! had refused in full.

import { assert, assertEquals } from "@std/assert";
import type { Outcome } from "./client.ts";

Deno.test("the success payload is unreachable without narrowing", () => {
  // This is a TYPE-level guarantee, so the test's job is to pin the runtime shape the type
  // describes — that `value` exists only on the `ok` arm, and `message` only on the other.
  const refused: Outcome<{ layerId: string }> = {
    ok: false,
    message: "[UnresolvedClassReference] prov:was_generated_by references …",
    errors: [],
  };
  assert(!refused.ok);
  assertEquals(
    "value" in refused,
    false,
    "a refusal carries no success payload",
  );
  assert(refused.message.length > 0, "and always says why");

  const landed: Outcome<{ layerId: string }> = {
    ok: true,
    value: { layerId: "0123456789ab" },
  };
  assert(landed.ok);
  assertEquals(landed.value.layerId, "0123456789ab");
  assertEquals(
    "message" in landed,
    false,
    "a success carries no error message",
  );
});

Deno.test("a refusal can carry the raw response for structured detail", () => {
  // `message` cannot distinguish a tag-name collision from any other failure; `response` can.
  const refused: Outcome<{ alreadyExists: boolean }> = {
    ok: false,
    message: "tag exists",
    errors: [],
    response: { alreadyExists: true },
  };
  assert(!refused.ok);
  assertEquals(refused.response?.alreadyExists, true);

  // Optional, because a COMPOSED outcome has no response of the mapped type —
  // `publishNotebook` maps `Outcome<LoadResponse>` to `Outcome<{ publish, load }>`.
  const composed: Outcome<{ publish: number }> = {
    ok: false,
    message: "the load it wraps was refused",
    errors: [],
  };
  assert(!composed.ok);
  assertEquals(composed.response, undefined);
});
