// Probe what the SDK throws for invalid ESL — mirrors the browser's
// connect-web call path so we can see the same error shape.

import { Eigen } from "../mod.ts";

const eigen = new Eigen({ endpoint: "http://localhost:8080" });

try {
  await eigen.load("this is not valid ESL syntax {{{", {
    contentType: "application/x-esl",
    autoCommit: true,
  });
  console.log("unexpected success");
} catch (err) {
  console.log("=== caught ===");
  console.log("typeof:", typeof err);
  console.log("constructor:", err?.constructor?.name);
  console.log("message:", err instanceof Error ? err.message : String(err));
  if (err && typeof err === "object") {
    for (const k of Object.keys(err as object)) {
      // deno-lint-ignore no-explicit-any
      console.log(`  .${k}:`, (err as any)[k]);
    }
  }
  // Connect-specific properties
  // deno-lint-ignore no-explicit-any
  const ce = err as any;
  if (ce.code !== undefined) console.log("  .code:", ce.code);
  if (ce.rawMessage !== undefined) console.log("  .rawMessage:", ce.rawMessage);
  if (ce.details !== undefined) console.log("  .details:", ce.details);
}
