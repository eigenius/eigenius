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
 * Golden e2e for the notebook MVP (D22 §6.12).
 *
 * Exercises the LLM-free critical path through the patent-analysis
 * demo: the static SPA serves, the demo loads on first mount, the
 * ESL ontology cell commits a layer, and the EigenQL cell queries
 * the patent namespace and renders rows in the result table.
 *
 * The LLM-driven program-run cell isn't asserted here — it costs
 * money + is slow. A separate gated test (post-MVP) covers it with
 * mock LLM enabled.
 */

import { expect, test } from "@playwright/test";

test("patent demo: load → ESL load → EigenQL query → results", async ({ page }) => {
  await page.goto("/notebooks/");

  // 1. SPA loaded; demo title is visible.
  await expect(
    page.getByRole("heading", { name: /Patent Analysis/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 2. All 6 cells render. The cell-type label is the most stable hook
  //    — six cells, six labels (markdown / esl / eigenql / esl /
  //    program-run / typescript). We assert at least the four
  //    distinct types are present.
  await expect(page.getByText("MARKDOWN").first()).toBeVisible();
  await expect(page.getByText("ESL").first()).toBeVisible();
  await expect(page.getByText("EIGENQL").first()).toBeVisible();
  await expect(page.getByText("PROGRAM RUN").first()).toBeVisible();
  await expect(page.getByText("TYPESCRIPT").first()).toBeVisible();

  // 3. Click "Run all" in the notebook toolbar — runs every runnable
  //    cell top-to-bottom, halting on first error. The patent demo's
  //    last cell hits Anthropic, but we don't gate on its completion;
  //    the assertions below cover the LLM-free critical path that
  //    finishes well before the program-run cell does.
  await page.getByRole("button", { name: "Run all", exact: true }).click();

  // 4. Both ESL cells should commit a layer; the load-output panel
  //    surfaces "Loaded N resource(s)" for each. The first is the
  //    ontology (multiple resources); the second is the patent
  //    instance (1 resource).
  await expect(
    page.getByText(/Loaded \d+ resources?/).first(),
  ).toBeVisible({ timeout: 15_000 });

  // 5. The EigenQL cell renders a result table; assert at least one
  //    row matches the patent namespace prefix.
  await expect(
    page.getByRole("gridcell", { name: /urn:eigenius:demo:patent:/ }).first(),
  ).toBeVisible({ timeout: 15_000 });
});
