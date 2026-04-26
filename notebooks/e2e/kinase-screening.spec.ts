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
 * E2E for the chart cell type (D22 Phase 5d).
 *
 * The chart cell is form-based: pick a chart kind, write an EigenQL
 * query, bind axis columns. Execution runs the query against the
 * kernel and renders the result as a Fluent chart.
 *
 * The kinase-screening demo exercises every supported chart kind
 * (grouped-bar / donut / vertical-bar / horizontal-bar / line / area),
 * making it the natural regression target. This test asserts that
 * after a single Run all click, every chart cell renders an SVG and
 * none of the cells surface a "Cell failed" message.
 *
 * The categorical-x → numeric-index → tickText mapping for line/area
 * is the most fragile path (Fluent's LineChart/AreaChart only support
 * numeric or Date x-axes natively); covering it here protects the
 * fix in `renderChart` from silent regressions.
 */

import { expect, test } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const KINASE_PATH = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../examples/kinase-screening.json",
);

// Run All in this notebook does ESL load + EigenQL queries + six
// chart-cell queries + topology graph fetch — comfortably more than
// Playwright's default 30s budget on first kernel boot.
test.setTimeout(120_000);

test("kinase-screening: open → run all → six chart cells render", async ({ page }) => {
  await page.goto("/notebooks/");

  // 1. SPA up; the patent demo is what App.tsx auto-loads on first
  //    mount, so wait for its title to know the page is interactive.
  await expect(
    page.getByRole("heading", { name: /Patent Analysis/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 2. Open the kinase-screening notebook through the hidden file
  //    input wired to the toolbar's "Open…" button.
  await page.locator('input[type="file"]').setInputFiles(KINASE_PATH);

  // 3. Title swaps — confirms the file was parsed and loaded.
  await expect(
    page.getByRole("heading", { name: /Kinase Inhibitor Screening/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 4. Six chart-cell type badges visible — confirms the parser
  //    accepted every chart cell shape (catches regressions in
  //    parseCell or the CellType union). The badge's DOM text is
  //    "Chart"; CSS uppercases it visually.
  await expect(page.getByText("Chart", { exact: true })).toHaveCount(6);

  // 5. Run all — ESL cells commit layers, EigenQL cells query, chart
  //    cells query + render, topology cell calls layerTopology.
  await page.getByRole("button", { name: "Run all", exact: true }).click();

  // 6. ESL load completes (matches Cell 2/3/4). The kinase ontology
  //    has many resources; the data cells have ~24.
  await expect(
    page.getByText(/Loaded \d+ resources?/).first(),
  ).toBeVisible({ timeout: 30_000 });

  // 7. EigenQL DataGrid shows at least one cell with a kinase
  //    compound ID — confirms the kernel returned rows and the
  //    result table mounted. The kinase queries RETURN plain values
  //    (compound_id, target_name, ic50_nm), not IRIs, so we match on
  //    the EIG_NNNN compound-id format.
  await expect(
    page.getByRole("gridcell", { name: /^EIG_\d{4}$/ }).first(),
  ).toBeVisible({ timeout: 30_000 });

  // 8. Chart cells produce SVGs. Fluent splits its chart family
  //    across three className roots:
  //      - `.fui-cart__root`  — cartesian family (4 of ours:
  //        grouped-bar, vertical-bar, line, area)
  //      - `.fui-hbc__root`   — horizontal-bar (own implementation)
  //      - `.fui-donut__root` — donut (non-cartesian)
  //    Asserting each per-class count catches a silent mis-mapping
  //    in renderChart's switch.
  await expect(page.locator(".fui-cart__root")).toHaveCount(4, {
    timeout: 60_000,
  });
  await expect(page.locator(".fui-hbc__root")).toHaveCount(1);
  await expect(page.locator(".fui-donut__root")).toHaveCount(1);

  // 9. No chart (or any other) cell surfaced an error message bar.
  //    This catches: malformed EigenQL, missing columns, render
  //    exceptions (e.g. the categorical-x line/area regression).
  await expect(page.getByText("Cell failed")).toHaveCount(0);
});
