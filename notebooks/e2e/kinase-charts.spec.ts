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
 * Julia stack. We use the notebook's per-cell **"Run to here…"**
 * affordance on cell 13 to run cells 2-13 in source order via the
 * notebook's own scheduler (which serialises cell execution against
 * the kernel-side commit pipeline, avoiding the data races a
 * per-cell click loop with fixed delays would hit).
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

// "Run to here" on cell 13 fires 12 runnable cells (3 ESL + 2 EigenQL
// + 6 charts + 1 topology TS cell) in sequence. Cold kernel boot + ESL
// commits + chart queries comfortably fit in 120s.
test.setTimeout(120_000);

test("kinase-institutions Part A: open → run to cell 13 → six chart cells render", async ({ page }) => {
  await page.goto("/notebooks/");

  // 1. SPA up; the patent demo is what App.tsx auto-loads on first
  //    mount, so wait for its title to know the page is interactive.
  await expect(
    page.getByRole("heading", { name: /Patent Analysis/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 2. Open the consolidated kinase-institutions notebook through the
  //    hidden file input wired to the toolbar's "Import…" button.
  await page.locator('input[type="file"]').setInputFiles(KINASE_PATH);

  // 3. Title swaps — confirms the file was parsed and loaded.
  await expect(
    page.getByRole("heading", { name: /Kinase Inhibitor Screening/i }),
  ).toBeVisible({ timeout: 10_000 });

  // 4. Seven chart-cell type badges visible — Part A has six (cells
  //    7-12: grouped-bar, donut, vertical-bar, horizontal-bar, line,
  //    area); Part B's Verdict donut (cell 28) adds a seventh that we
  //    don't run. The badge's DOM text is "Chart"; CSS uppercases it
  //    visually.
  await expect(page.getByText("Chart", { exact: true })).toHaveCount(7);

  // 5. Use cell 13's per-cell SplitButton menu to invoke "Run to
  //    here…", which runs cells 2-13 in source order through the
  //    notebook's own scheduler. The index pill on every cell carries
  //    `aria-label="Cell N"`; the SplitButton's menu trigger
  //    (a chevron next to the primary "Run" button) sits in the same
  //    toolbar row and is identifiable by `aria-haspopup="menu"`.
  const cell13Toolbar = page
    .getByLabel("Cell 13", { exact: true })
    .locator("xpath=ancestor::div[1]");
  await cell13Toolbar.locator('[aria-haspopup="menu"]').click();
  await page
    .getByRole("menuitem", { name: /Run to here/i })
    .click();

  // 6. The "Run all" toolbar button is disabled while *any* cell is
  //    running — re-enabled when none are. Wait for it to leave and
  //    then re-enter the enabled state, which brackets the entire
  //    cells-2-through-13 sweep without us having to track per-cell
  //    completions.
  const runAll = page.getByRole("button", { name: "Run all", exact: true });
  // Confirm the run actually started (button entered disabled state).
  await expect(runAll).toBeDisabled({ timeout: 10_000 });
  // Then wait for the entire sweep to finish.
  await expect(runAll).toBeEnabled({ timeout: 90_000 });

  // 7. ESL load completes (matches Cell 2/3/4). The kinase ontology
  //    has many resources; the data cells have ~24.
  await expect(
    page.getByText(/Loaded \d+ resources?/).first(),
  ).toBeVisible({ timeout: 5_000 });

  // 8. EigenQL DataGrid shows at least one cell with a kinase
  //    compound ID — confirms the kernel returned rows and the
  //    result table mounted. The kinase queries RETURN plain values
  //    (compound_id, target_name, ic50_nm), not IRIs, so we match on
  //    the EIG_NNNN compound-id format.
  await expect(
    page.getByRole("gridcell", { name: /^EIG_\d{4}$/ }).first(),
  ).toBeVisible({ timeout: 5_000 });

  // 9. Chart cells produce SVGs. Fluent splits its chart family
  //    across three className roots:
  //      - `.fui-cart__root`  — cartesian family (4 of ours:
  //        grouped-bar, vertical-bar, line, area)
  //      - `.fui-hbc__root`   — horizontal-bar (own implementation)
  //      - `.fui-donut__root` — donut (non-cartesian)
  //    Asserting each per-class count catches a silent mis-mapping
  //    in renderChart's switch. Only Part A's charts run; Part B's
  //    Verdict donut (cell 28) is not exercised here.
  await expect(page.locator(".fui-cart__root")).toHaveCount(4);
  await expect(page.locator(".fui-hbc__root")).toHaveCount(1);
  await expect(page.locator(".fui-donut__root")).toHaveCount(1);

  // 10. No cell surfaced an error message bar. This catches: malformed
  //     EigenQL, missing columns, render exceptions (e.g. the
  //     categorical-x line/area regression), and ESL-validation
  //     failures from cells 2-4.
  await expect(page.getByText("Cell failed")).toHaveCount(0);
});
