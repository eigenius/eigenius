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
 * Exercises the auto-loaded stats-and-reasoning demo: the static SPA
 * serves, the demo loads on first mount, "Run all" commits its ESL
 * cells, and the statistics institution's AutoOnLoad gate recomputes
 * the potency claim from the raw reads — the first EigenQL cell finds
 * its `Holds` verdict.
 */

import { expect, test } from "@playwright/test";

// Cold kernel boot + nine ESL commits (one of them triggering the
// statistics recomputation) + three queries.
test.setTimeout(60_000);

test("stats demo: load → run all → statistics verdict Holds", async ({ page }) => {
  await page.goto("/notebooks/");

  // 1. SPA loaded; demo title is visible.
  await expect(
    page.getByRole("heading", { name: /Statistics \+ Reasoning/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 2. The notebook's three cell types render.
  await expect(page.getByText("MARKDOWN").first()).toBeVisible();
  await expect(page.getByText("ESL").first()).toBeVisible();
  await expect(page.getByText("EIGENQL").first()).toBeVisible();

  // 3. "Run all" runs every runnable cell top-to-bottom, halting on
  //    the first error.
  await page.getByRole("button", { name: "Run all", exact: true }).click();

  // 4. The ESL cells commit layers; the load-output panel surfaces
  //    "Loaded N resource(s)" for each.
  await expect(
    page.getByText(/Loaded \d+ resources?/).first(),
  ).toBeVisible({ timeout: 15_000 });

  // 5. The first EigenQL cell queries the Verdict the statistics
  //    institution emitted when the StatisticalAnalysisPlan committed.
  await expect(
    page.getByRole("gridcell", { name: "Holds", exact: true }).first(),
  ).toBeVisible({ timeout: 30_000 });
});
