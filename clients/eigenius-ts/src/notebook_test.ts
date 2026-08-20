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
 * The publish round-trip for the D71 `formalize` cell.
 *
 * `notebook:cell_type` is closed by `allows_only` precisely so a new cell type
 * cannot appear without a deliberate ontology edit — and that edit costs a
 * reseed. What the enumeration cannot catch is a cell type the ontology knows
 * and the SDK does not: publishing would then mint the wrong IRI, or drop the
 * cell's own fields, and the notebook would come back subtly different from the
 * one that was saved. That is what this covers.
 */

import {
  assertEquals,
  assertNotEquals,
} from "jsr:@std/assert@^1.0.19";
import {
  type CellJson,
  notebookJsonToResources,
  type NotebookJson,
  resourcesToNotebookJson,
} from "./notebook.ts";

function notebook(cells: CellJson[]): NotebookJson {
  return {
    format_version: 1,
    meta: { title: "Formalization round-trip" },
    cells,
  };
}

const CELL: CellJson = {
  id: "cell-1",
  type: "formalize",
  source: "MSI cancer models required the helicase activity of WRN.",
  doc_id: "wrn-first-page",
  structure_iri: "urn:eigenius:demo:v2:structure",
  lexicon_profile: "urn:eigenius:lexicon:profile:biomed",
  land: true,
};

Deno.test("a formalize cell survives publish and re-read", async () => {
  const out = await notebookJsonToResources(notebook([CELL]));
  const back = resourcesToNotebookJson(out.notebookIri, out.resources);

  assertEquals(back.cells.length, 1);
  const cell = back.cells[0];
  assertEquals(cell.type, "formalize");
  if (cell.type !== "formalize") throw new Error("unreachable");
  assertEquals(cell.source, CELL.source as string);
  // doc_id is the WORKING BRANCH: losing it on a round-trip would silently
  // re-point a re-run at a fresh branch, which re-asks the model instead of
  // replaying that run's recorded draws.
  assertEquals(cell.doc_id, "wrn-first-page");
  assertEquals(cell.structure_iri, "urn:eigenius:demo:v2:structure");
  assertEquals(cell.lexicon_profile, "urn:eigenius:lexicon:profile:biomed");
  // `land` decides whether `Run all` commits. Losing it on a round-trip would
  // silently turn a reproducible notebook into one that only produces artifacts.
  assertEquals(cell.land, true);
});

Deno.test("a formalize cell publishes as the formalize CellType, not a fallback", async () => {
  const out = await notebookJsonToResources(notebook([CELL]));
  const cellRes = out.resources.find((r) =>
    String(r["@id"] ?? "").includes(":cell:")
  );
  // An unknown cell type decodes as `markdown` by design (the reader degrades
  // rather than dropping a cell), so asserting the round-trip alone would pass
  // even if the SDK had never learned the type. Assert the IRI it published.
  assertEquals(
    cellRes?.["urn:eigenius:notebook:cell_type"],
    "urn:eigenius:notebook:formalize",
  );
});

Deno.test("cell identity ignores structure_iri but tracks doc_id", async () => {
  const base = await notebookJsonToResources(notebook([CELL]));
  const ranAgain = await notebookJsonToResources(
    notebook([{ ...CELL, structure_iri: "urn:eigenius:demo:v2:structure2" }]),
  );
  const differentBranch = await notebookJsonToResources(
    notebook([{ ...CELL, doc_id: "other-doc" }]),
  );

  // `structure_iri` is what the last RUN produced, not what the cell IS —
  // including it in the content hash would give the same prose a new identity
  // after every run and defeat cell de-duplication.
  assertEquals(base.cellIris[0], ranAgain.cellIris[0]);
  // `doc_id` DOES change identity: a different working branch is a different
  // cell, because it replays a different set of draws.
  assertNotEquals(base.cellIris[0], differentBranch.cellIris[0]);

  // So does `land`: two cells over the same prose that differ on whether they
  // commit are different cells, not one cell in two moods.
  const noLand = await notebookJsonToResources(
    notebook([{ ...CELL, land: false }]),
  );
  assertNotEquals(base.cellIris[0], noLand.cellIris[0]);
});
