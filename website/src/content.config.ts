import { defineCollection, z } from "astro:content";
import { glob } from "astro/loaders";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

// Starlight expects a single docs collection. Sections in the
// sidebar (Concepts, Examples, Docs, Research) are subdirectories
// inside src/content/docs/, configured in astro.config.mjs's
// sidebar block.
const docs = defineCollection({ loader: docsLoader(), schema: docsSchema() });

// Blog posts are deliberately not part of the docs collection: the
// docs sidebar orders entries alphabetically and its schema carries
// no date, so neither a reverse-chronological index nor an RSS feed
// can be derived from it. Posts render through src/pages/blog/*
// wearing the hand-authored chrome instead of the docs sidebar.
//
// `pubDate` is required on purpose. An undated post would sort
// arbitrarily and reach the feed with a garbage timestamp; requiring
// it fails the build instead.
const blog = defineCollection({
  loader: glob({ pattern: "**/*.{md,mdx}", base: "./src/content/blog" }),
  schema: z.object({
    title: z.string(),
    // Deck line under the headline, e.g. "Institutions, Part I — …".
    // Lives here rather than as a heading in the body: the layout
    // already renders `title` as the page's only <h1>, and a second
    // one in the markdown would give the page two.
    subtitle: z.string().optional(),
    description: z.string(),
    pubDate: z.coerce.date(),
    updatedDate: z.coerce.date().optional(),
    author: z.string().default("Eigenius"),
    tags: z.array(z.string()).default([]),
    draft: z.boolean().default(false),
    // Posts that read in a fixed order. The index groups them and
    // orders by `part` ascending, overriding the reverse-chronological
    // default — otherwise the last part published sits on top, above
    // the parts it depends on.
    series: z
      .object({
        name: z.string(),
        part: z.number().int().positive(),
      })
      .optional(),
  }),
});

export const collections = { docs, blog };
