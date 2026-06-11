import { defineCollection } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

// Starlight expects a single docs collection. Sections in the
// sidebar (Concepts, Examples, Docs, Research) are subdirectories
// inside src/content/docs/, configured in astro.config.mjs's
// sidebar block.

export const collections = {
  docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),
};
