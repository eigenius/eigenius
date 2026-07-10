import { getCollection, type CollectionEntry } from "astro:content";

export type Post = CollectionEntry<"blog">;

/** A run of posts that read in a fixed order, e.g. a three-part series. */
export interface SeriesGroup {
  kind: "series";
  name: string;
  /** Ascending by `part`. */
  posts: Post[];
  /** Newest `pubDate` in the group — its position on the index. */
  sortDate: Date;
}

/** A post that belongs to no series. */
export interface SinglePost {
  kind: "post";
  post: Post;
  sortDate: Date;
}

export type IndexEntry = SeriesGroup | SinglePost;

/**
 * Published posts, newest first.
 *
 * Drafts are excluded from production builds only, so `npm run dev`
 * still renders them. GitHub Pages serves a static build, so there is
 * no request-time gate — a draft that reaches `dist/` is public.
 *
 * The index page, the post pages, and the RSS feed all read through
 * this function. If they each filtered and sorted independently, a
 * draft could disappear from the index while remaining reachable at
 * its URL and in the feed.
 */
export async function getPublishedPosts(): Promise<Post[]> {
  const posts = await getCollection("blog", ({ data }) =>
    import.meta.env.PROD ? data.draft === false : true,
  );
  return posts.sort(
    (a, b) => b.data.pubDate.valueOf() - a.data.pubDate.valueOf(),
  );
}

/** Published posts of one series, ascending by `part`. */
export async function getSeriesPosts(name: string): Promise<Post[]> {
  const posts = (await getPublishedPosts()).filter(
    (p) => p.data.series?.name === name,
  );
  assertDistinctParts(name, posts);
  return posts.sort((a, b) => a.data.series!.part - b.data.series!.part);
}

/**
 * What the index renders, top to bottom.
 *
 * Series collapse into one entry ordered by `part` ascending, so Part I
 * precedes Part II. The entry as a whole is placed by its newest post,
 * which keeps a freshly-finished series above older standalone posts
 * without letting its final part jump ahead of its first.
 */
export async function getIndexEntries(): Promise<IndexEntry[]> {
  const posts = await getPublishedPosts();
  const bySeries = new Map<string, Post[]>();
  const entries: IndexEntry[] = [];

  for (const post of posts) {
    const series = post.data.series;
    if (!series) {
      entries.push({ kind: "post", post, sortDate: post.data.pubDate });
      continue;
    }
    const group = bySeries.get(series.name) ?? [];
    group.push(post);
    bySeries.set(series.name, group);
  }

  for (const [name, group] of bySeries) {
    assertDistinctParts(name, group);
    entries.push({
      kind: "series",
      name,
      posts: [...group].sort((a, b) => a.data.series!.part - b.data.series!.part),
      sortDate: new Date(
        Math.max(...group.map((p) => p.data.pubDate.valueOf())),
      ),
    });
  }

  return entries.sort((a, b) => b.sortDate.valueOf() - a.sortDate.valueOf());
}

/**
 * The one entry to feature on the landing page, or null if nothing is
 * published.
 *
 * Deliberately *not* "the newest post". The newest post in a series is
 * its last part, which assumes the parts before it — featuring Part III
 * drops a first-time visitor into the conclusion. Featuring the entry
 * instead means a series is presented as a series, entered at Part I.
 *
 * Reads through `getIndexEntries`, so the landing page and the blog
 * index can never disagree about what is newest.
 */
export async function getFeaturedEntry(): Promise<IndexEntry | null> {
  const entries = await getIndexEntries();
  return entries[0] ?? null;
}

/** The post a featured entry should link to first. */
export function entryEntryPoint(entry: IndexEntry): Post {
  return entry.kind === "series" ? entry.posts[0] : entry.post;
}

/** Where a post sits in its series, and its neighbours. */
export interface SeriesContext {
  name: string;
  part: number;
  total: number;
  prev?: Post;
  next?: Post;
}

export async function getSeriesContext(
  post: Post,
): Promise<SeriesContext | null> {
  const series = post.data.series;
  if (!series) return null;

  const siblings = await getSeriesPosts(series.name);
  const index = siblings.findIndex((p) => p.id === post.id);

  return {
    name: series.name,
    part: series.part,
    total: siblings.length,
    prev: index > 0 ? siblings[index - 1] : undefined,
    next: index < siblings.length - 1 ? siblings[index + 1] : undefined,
  };
}

/**
 * Two posts claiming the same part number have no defined order, and
 * whichever the sort happens to emit first would silently become the
 * earlier part. Fail the build instead.
 */
function assertDistinctParts(name: string, posts: Post[]): void {
  const seen = new Map<number, string>();
  for (const post of posts) {
    const part = post.data.series!.part;
    const clash = seen.get(part);
    if (clash) {
      throw new Error(
        `Series "${name}" has two posts numbered part ${part}: ` +
          `"${clash}" and "${post.id}". Part numbers must be unique.`,
      );
    }
    seen.set(part, post.id);
  }
}

/**
 * Part numbers are stored as integers so they sort, but displayed as
 * Roman numerals to match how the posts title themselves ("Part III —
 * Making Reasoning Checkable"). A generated chip reading "Part 3" above
 * a headline reading "Part III" is the kind of mismatch readers notice.
 *
 * Numbers above the series lengths we plausibly write fall back to the
 * integer rather than producing nonsense.
 */
const ROMAN = [
  "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X",
  "XI", "XII", "XIII", "XIV", "XV", "XVI", "XVII", "XVIII", "XIX", "XX",
];

export function formatPart(part: number): string {
  return ROMAN[part - 1] ?? String(part);
}

/** `3` → `three`, for prose like "a three-part introduction". */
const WORDS = [
  "zero", "one", "two", "three", "four", "five",
  "six", "seven", "eight", "nine", "ten",
];

export function spellCount(n: number): string {
  return WORDS[n] ?? String(n);
}

/** `2026-07-09` → `9 July 2026`. */
export function formatDate(date: Date): string {
  return date.toLocaleDateString("en-GB", {
    year: "numeric",
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  });
}

/** Machine-readable form for `<time datetime>`, e.g. `2026-07-09`. */
export function isoDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}
