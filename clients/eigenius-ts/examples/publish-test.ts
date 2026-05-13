// Quick verification of Phase 3.5 publish flow.
// Run via: deno run --allow-net --allow-read=/home/hm/src/eigenius examples/publish-test.ts
//
// The notebook ontology is now part of the kernel boot chain (D22 §6.5 +
// kernel/src/bootstrap/mod.rs), so no explicit ontology load is required —
// the kernel knows about Notebook + Cell from startup.

import { Eigen } from "../mod.ts";

const eigen = new Eigen({ endpoint: "http://localhost:8080" });

// 1. Confirm the kernel has the notebook ontology baked in (sanity check).
const sanity = await eigen.inspect("urn:eigenius:notebook:Notebook");
console.log(
  `[sanity] Notebook class resolvable: ${sanity.found} (${sanity.resource.length} CBOR bytes)`,
);
if (!sanity.found) {
  console.error(
    "kernel does not have the notebook ontology — bootstrap may not have included it",
  );
  Deno.exit(1);
}

// 2. Load the patent-analysis notebook (cold — no explicit ontology load)
const notebookFile = await Deno.readTextFile(
  "/home/hm/src/eigenius/notebooks/examples/patent-analysis.json",
);
const notebook = JSON.parse(notebookFile);

const { publish, load } = await eigen.publishNotebook(notebook);
console.log(
  `[publish] notebookIri=${publish.notebookIri}\n          ${publish.cellIris.length} cell IRI(s):`,
);
for (const iri of publish.cellIris) {
  console.log(`            ${iri}`);
}
console.log(
  `[publish] load success=${load.success} resources=${load.resourceCount} layer=${
    load.layerId.slice(0, 12)
  }…`,
);
if (!load.success) {
  console.error("errors:", load.errors);
  Deno.exit(1);
}

// 3. Verify by querying for Notebook resources
const q1 = await eigen.query(`
  USING "urn:eigenius:notebook:Notebook"
  MATCH Notebook(?n) {
    "urn:eigenius:notebook:title": ?title
  }
  RETURN [] {
    title: ?title,
    iri:   ?n
  }
`);
console.log(
  `[query Notebook] success=${q1.success} bytes=${q1.document.length}`,
);
if (!q1.success) console.error("error:", q1.error);

// 4. Verify by querying for Cell resources
const q2 = await eigen.query(`
  USING "urn:eigenius:notebook:Cell"
  MATCH Cell(?c) {
    "urn:eigenius:notebook:cell_type": ?type
  }
  RETURN [] {
    iri:  ?c,
    type: ?type
  }
`);
console.log(`[query Cell] success=${q2.success} bytes=${q2.document.length}`);
if (!q2.success) console.error("error:", q2.error);

// 5. Idempotence check: publish again, expect same IRI
const second = await eigen.publishNotebook(notebook);
console.log(
  `[idempotence] second publish IRI=${second.publish.notebookIri}`,
);
if (second.publish.notebookIri === publish.notebookIri) {
  console.log("            ✓ matches first publish");
} else {
  console.log("            ✗ DIFFERS — content addressing is broken");
}
