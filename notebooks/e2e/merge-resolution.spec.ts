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
 * D36 §14 (PR 4) — Merge resolution E2E skeleton.
 *
 * Drives the cell-commit-race entry path end-to-end: load a fixture
 * notebook whose two ESL cells write the same IRI with conflicting
 * bodies, run the second cell against a branch that already has the
 * first, click through the resolution flow, and verify the cell
 * badge clears on success.
 *
 * **Why this is `fixme`d.** The other notebook e2e specs run against
 * the bundled patent / kinase fixtures, which exercise a single
 * monotonic chain. Reproducing `NEEDS_WITNESSED_MERGE` requires either
 * (a) a fixture that pre-seeds a branch at layer L_A and runs a cell
 * whose computed parent is L_A but whose contribution overlaps with
 * a second branch tip L_B, or (b) two parallel test pages racing
 * commits to the same branch. Neither fixture exists yet; the
 * orchestrator's test-mode endpoints don't currently expose a way to
 * pre-seed a divergent branch from inside the SPA.
 *
 * The selectors below are stable against the current
 * `MergeResolutionFlow` / `StrategyPicker` / `CascadePreviewPane`
 * markup — once the fixture lands (tracked as follow-up), drop the
 * `fixme` and the assertions below should pass without further edits
 * to the spec.
 */

import { expect, test } from "@playwright/test";

// Cold kernel boot + two ESL commits + a preview + a commit
// comfortably fits in 60s; bump only if the fixture grows.
test.setTimeout(60_000);

test.fixme(
  "cell commit race → resolve via Rename → cell badge clears",
  async ({ page }) => {
    // 1. SPA up. Patent demo is the auto-loaded fixture; we'll swap
    //    it for the merge fixture in step 2.
    await page.goto("/notebooks/");
    await expect(
      page.getByRole("heading", { name: /Patent Analysis/i }),
    ).toBeVisible({ timeout: 10_000 });

    // 2. TODO(fixture): import a notebook with the two-divergent-cell
    //    setup. The fixture needs:
    //      - Cell 1: an ESL block defining `urn:project:Patient` as
    //        a medical-records type. Runs first, commits layer L_A.
    //      - Cell 2: an ESL block defining the same IRI as a billing
    //        type. Runs second; because both cells branched from the
    //        same parent, this hits `NEEDS_WITNESSED_MERGE`.
    //    Replace the path below once the fixture exists at
    //    `notebooks/examples/merge-resolution-fixture.json`.
    //
    // const FIXTURE_PATH = path.resolve(
    //   path.dirname(fileURLToPath(import.meta.url)),
    //   "../examples/merge-resolution-fixture.json",
    // );
    // await page.locator('input[type="file"]').setInputFiles(FIXTURE_PATH);

    // 3. Run all. Cell 2 fails with the witnessed-merge banner.
    await page.getByRole("button", { name: "Run all", exact: true }).click();

    // 4. Cell 2's commit-status badge should show the
    //    `NEEDS_WITNESSED_MERGE` message bar with the resolution CTA.
    await expect(
      page.getByText(/needs witnessed merge/i),
    ).toBeVisible({ timeout: 30_000 });
    const resolveBtn = page.getByRole("button", {
      name: /Resolve in Merge rail/i,
    });
    await expect(resolveBtn).toBeVisible();
    await resolveBtn.click();

    // 5. Merge rail switches to resolution mode; the picking header
    //    surfaces the conflict count.
    await expect(
      page.getByRole("heading", { name: /Resolve 1 conflict/i }),
    ).toBeVisible({ timeout: 5_000 });

    // 6. Pick `Rename` for the single conflict. The strategy radios
    //    are labelled by their human-readable name.
    await page.getByRole("radio", { name: /Rename/ }).check();

    // 7. Fill the rename editor: side B, new IRI in the billing
    //    namespace.
    await page.getByRole("radio", { name: /Side B/ }).check();
    await page.getByLabel(/New IRI/).fill("urn:project:billing:Patient");

    // 8. Preview cascade. The flow advances to `acknowledging`.
    await page.getByRole("button", { name: /Preview cascade/i }).click();
    await expect(
      page.getByRole("heading", { name: /Acknowledge consequences/i }),
    ).toBeVisible({ timeout: 10_000 });

    // 9. Tick every cascade-item checkbox. The fixture's exact item
    //    count depends on what pre-existing references the seeded
    //    branch has; the assertion shape works for any non-zero
    //    cascade. (If the cascade is empty, step 10 skips ticking
    //    and the Commit button is enabled by default.)
    const ackBoxes = page.getByRole("checkbox");
    const ackCount = await ackBoxes.count();
    for (let i = 0; i < ackCount; i++) {
      await ackBoxes.nth(i).check();
    }

    // 10. Commit. The flow advances to `done` and renders the
    //     success card.
    await page.getByRole("button", { name: /Commit merge/i }).click();
    await expect(
      page.getByRole("heading", { name: /Merge committed/i }),
    ).toBeVisible({ timeout: 10_000 });

    // 11. Cell 2's NEEDS_WITNESSED_MERGE badge should clear (the
    //     store rewrites its commit meta to `TRIVIAL_MERGE` per
    //     D36 §15.3 / Decisions log).
    await expect(page.getByText(/needs witnessed merge/i)).toHaveCount(0);
  },
);
