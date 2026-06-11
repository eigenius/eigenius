// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

// The site has a hand-authored landing page at /src/pages/index.astro
// and a Starlight-driven docs section under /docs/.
// Concepts, examples, and research are also rendered through
// Starlight so the sidebar nav, search, mobile drawer, and
// dark-mode handling come for free.

export default defineConfig({
  site: "https://eigenius.io",
  integrations: [
    starlight({
      title: "Eigenius",
      description:
        "A typed knowledge graph that records how scientific claims are " +
        "reached — not just what is claimed. A compiler for AI thought.",
      logo: {
        src: "./src/assets/eigenius_logo_400x400.png",
        replacesTitle: false,
      },
      favicon: "/favicon.png",
      social: {
        github: "https://github.com/eigenius/eigenius",
      },
      // The landing page lives at /src/pages/index.astro and overrides
      // Starlight's default. The four sections below are the primary
      // nav; each is a docs collection rooted at its own slug.
      sidebar: [
        {
          label: "Concepts",
          autogenerate: { directory: "concepts" },
        },
        {
          label: "Examples",
          autogenerate: { directory: "examples" },
        },
        {
          label: "Docs",
          autogenerate: { directory: "docs" },
        },
        {
          label: "Research",
          autogenerate: { directory: "research" },
        },
      ],
      customCss: ["./src/styles/custom.css"],
    }),
  ],
});
