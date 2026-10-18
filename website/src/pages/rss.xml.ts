import rss from "@astrojs/rss";
import type { APIContext } from "astro";
import { getPublishedPosts } from "../lib/posts";

// Prerendered to a static rss.xml at build time — GitHub Pages has no
// server, so this endpoint cannot run on demand.
export async function GET(context: APIContext) {
  const posts = await getPublishedPosts();

  return rss({
    title: "Eigenius",
    description:
      "Notes and articles on typed knowledge graphs, warranted reasoning, " +
      "and building auditable AI research agents.",
    // Non-null: `site` is set in astro.config.mjs, and @astrojs/rss
    // needs an absolute base to resolve each item's link.
    site: context.site!,
    items: posts.map((post) => ({
      title: post.data.title,
      description: post.data.description,
      pubDate: post.data.pubDate,
      link: `/blog/${post.id}/`,
      categories: post.data.tags,
      author: post.data.author,
    })),
    customData: "<language>en</language>",
  });
}
