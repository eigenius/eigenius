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
 * The consolidated `kinase-institutions` notebook's **Part A**
 * (cells 1-13) exercises every supported chart kind (grouped-bar /
 * donut / vertical-bar / horizontal-bar / line / area), making it
 * the natural regression target. Parts B and C of that notebook
 * require the Julia institutions setup (`kinase-institutions-setup.sh`)
 * and are intentionally not exercised here — CI doesn't install the
 * Julia stack. We open the notebook, then run the **Part A cells
 * individually** rather than clicking Run all so the institutions
 * cells never fire.
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
  "../examples/kinase-institutions.json",
);

// Part A is small (3 ESL cells + 2 EigenQL cells + 6 chart cells +
// 1 topology cell — 12 runnable cells total). Each cell click waits
// for completion before the next runs. The 120s budget covers cold
// kernel boot + the ESL commits + the chart queries.
test.setTimeout(120_000);

test("kinase-institutions Part A: open → run cells 1-13 → six chart cells render", async ({
  page,
}) => {
  await page.goto("/notebooks/");

  // 1. SPA up; the patent demo is what App.tsx auto-loads on first
  //    mount, so wait for its title to know the page is interactive.
  await expect(
    page.getByRole("heading", { name: /Patent Analysis/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 2. Open the consolidated kinase-institutions notebook through the
  //    hidden file input wired to the toolbar's "Import…" button.
  await page.locator('input[type="file"]').setInputFiles(KINASE_PATH);

  // 3. Title swaps — confirms the file was parsed and loaded. The
  //    consolidated notebook's title starts with "Kinase Inhibitor
  //    Screening — From Flat Data to Typed Institutions".
  await expect(
    page.getByRole("heading", { name: /Kinase Inhibitor Screening/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 4. Seven chart-cell type badges visible — Part A has six (cells
  //    7-12: grouped-bar, donut, vertical-bar, horizontal-bar, line,
  //    area); Part B's Verdict donut adds a seventh that we don't
  //    run. The badge's DOM text is "Chart"; CSS uppercases it
  //    visually.
  await expect(page.getByText("Chart", { exact: true })).toHaveCount(7);

  // 5. Run only Part A's cells (1-13) by clicking each cell's per-cell
  //    Run button. Cells 1, 14, 15, 19, 20, 25, 28 are markdown — no
  //    Run button on those. Cell 13 is the topology graph TS cell;
  //    cells 14+ would invoke the Julia institutions and fail without
  //    the setup script having run, so we stop after cell 13.
  //
  //    The notebook UI exposes a Run button per runnable cell; we
  //    click them in order to mirror "Run all but only up to cell 13"
  //    semantics without a built-in Run-to-here helper.
  const runButtons = page.locator('[aria-label="Run cell"], button:has-text("Run")').filter({
    hasNot: page.locator('button:has-text("Run all")'),
  });

  // The runnable cells in Part A: 2, 3, 4 (ESL), 5, 6 (EigenQL),
  // 7-12 (charts), 13 (TypeScript) = 12 cells.
  const PART_A_RUNNABLE_CELLS = 12;

  for (let i = 0; i < PART_A_RUNNABLE_CELLS; i += 1) {
    await runButtons.nth(i).click();
    // Brief settle so the next click doesn't race the previous run's
    // pending state. The actual completion assertions follow below.
    await page.waitForTimeout(200);
  }

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
  //    in renderChart's switch. Only Part A's charts run; Part B's
  //    Verdict donut (cell 24) is not exercised here.
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
