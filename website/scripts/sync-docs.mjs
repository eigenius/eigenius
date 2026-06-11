// Sync docs/guides/ into website/src/content/docs/docs/ as a build step.
//
// The repo-side guides are authoritative; the website-side copy is a
// build artifact (gitignored). On every build we:
//   1. Wipe the destination (except the hand-authored index.mdx).
//   2. Copy each guide markdown file, prepending YAML frontmatter
//      extracted from the file's H1 (the guides have no frontmatter;
//      Starlight requires `title:`).
//   3. Rewrite any markdown link whose target resolves outside
//      docs/guides/ — design docs, source-tree paths — into an absolute
//      GitHub URL. Links that stay within guides/ are left alone so
//      Starlight's intra-collection link handling works.
//   4. Copy the assets/ directory verbatim.

import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = path.resolve(__dirname, "..");
const REPO_ROOT = path.resolve(WEBSITE_ROOT, "..");
const GUIDES_SRC = path.join(REPO_ROOT, "docs", "guides");
const DOCS_DST = path.join(
  WEBSITE_ROOT,
  "src",
  "content",
  "docs",
  "docs",
);
const GITHUB_BASE = "https://github.com/eigenius/eigenius";

// Files in DOCS_DST that the sync must not touch.
const PRESERVE = new Set(["index.mdx"]);

// The top-level docs/guides/README.md is skipped — we have our own
// index.mdx that introduces the docs section.
const SKIP_RELATIVE = new Set(["README.md"]);

async function* walk(dir) {
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(full);
    else yield full;
  }
}

function rewriteHref(href, sourceFilePath) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(href)) return href; // http:, https:, mailto:, ...
  if (href.startsWith("#") || href.startsWith("/")) return href;

  const [pathPart, anchor] = href.split("#");
  const anchorPart = anchor !== undefined ? `#${anchor}` : "";

  if (!pathPart) return href;

  const sourceDir = path.dirname(sourceFilePath);
  const resolved = path.resolve(sourceDir, pathPart);

  if (resolved === GUIDES_SRC || resolved.startsWith(GUIDES_SRC + path.sep)) {
    // Stays within guides. Convert to an absolute /docs/... URL —
    // relative resolution is fragile because Starlight serves each
    // `X.md` as URL `X/` (a virtual directory), which shifts the
    // base for every sibling link. Computing the URL directly from
    // the resolved file path avoids that whole class of bug.
    let rel = path
      .relative(GUIDES_SRC, resolved)
      .split(path.sep)
      .join("/");
    // README.md / index.md → the directory itself.
    rel = rel.replace(/(^|\/)(README|index)\.md$/, "$1");
    // Other .md → strip extension and treat as a directory.
    if (rel.endsWith(".md")) rel = rel.slice(0, -3);
    // Always end with a slash for directory-style URLs.
    if (rel && !rel.endsWith("/")) rel += "/";
    const url = "/docs/" + rel;
    return `${url}${anchorPart}`;
  }

  const fromRepo = path.relative(REPO_ROOT, resolved).split(path.sep).join("/");
  const isDir = pathPart.endsWith("/");
  const kind = isDir ? "tree" : "blob";
  const cleanPath = isDir && fromRepo.endsWith("/")
    ? fromRepo.slice(0, -1)
    : fromRepo;
  return `${GITHUB_BASE}/${kind}/main/${cleanPath}${anchorPart}`;
}

function rewriteLinks(content, sourceFilePath) {
  // Inline markdown links: [text](url) and [text](url "title")
  const linkRe = /\[([^\]]*)\]\(([^)\s]+)(\s+"[^"]*")?\)/g;
  return content.replace(linkRe, (_match, text, href, titleSuffix) => {
    const newHref = rewriteHref(href, sourceFilePath);
    return `[${text}](${newHref}${titleSuffix ?? ""})`;
  });
}

function addFrontmatter(content) {
  if (/^---\r?\n/.test(content)) return content; // already has frontmatter

  const h1Match = content.match(/^# (.+?)\s*$/m);
  const title = h1Match ? h1Match[1].replace(/"/g, '\\"') : "Untitled";
  const stripped = h1Match
    ? content.replace(/^# .+\r?\n+/m, "")
    : content;
  return `---\ntitle: "${title}"\n---\n\n${stripped}`;
}

async function processMarkdown(srcPath) {
  const rel = path.relative(GUIDES_SRC, srcPath);
  if (SKIP_RELATIVE.has(rel)) return false;

  // Starlight only routes index.{md,mdx} as a section index — not
  // README.md. Rename each subsection's README.md to index.md on the
  // way in so /docs/<section>/ resolves to its overview page.
  const dstRel = rel.replace(/(^|\/)README\.md$/, "$1index.md");
  const dstPath = path.join(DOCS_DST, dstRel);

  let content = await fs.readFile(srcPath, "utf8");
  content = rewriteLinks(content, srcPath);
  content = addFrontmatter(content);

  await fs.mkdir(path.dirname(dstPath), { recursive: true });
  await fs.writeFile(dstPath, content);
  return true;
}

async function copyAssets() {
  const srcAssets = path.join(GUIDES_SRC, "assets");
  try {
    await fs.access(srcAssets);
  } catch {
    return 0;
  }
  const dstAssets = path.join(DOCS_DST, "assets");
  await fs.mkdir(dstAssets, { recursive: true });
  let count = 0;
  for await (const file of walk(srcAssets)) {
    const rel = path.relative(srcAssets, file);
    const dst = path.join(dstAssets, rel);
    await fs.mkdir(path.dirname(dst), { recursive: true });
    await fs.copyFile(file, dst);
    count++;
  }
  return count;
}

async function clean() {
  try {
    const entries = await fs.readdir(DOCS_DST, { withFileTypes: true });
    for (const entry of entries) {
      if (PRESERVE.has(entry.name)) continue;
      const full = path.join(DOCS_DST, entry.name);
      await fs.rm(full, { recursive: true, force: true });
    }
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
  }
}

async function main() {
  const relSrc = path.relative(REPO_ROOT, GUIDES_SRC);
  const relDst = path.relative(REPO_ROOT, DOCS_DST);
  console.log(`sync-docs: ${relSrc} → ${relDst}`);

  await fs.mkdir(DOCS_DST, { recursive: true });
  await clean();

  let mdCount = 0;
  for await (const file of walk(GUIDES_SRC)) {
    if (!file.endsWith(".md")) continue;
    const rel = path.relative(GUIDES_SRC, file);
    if (rel.startsWith("assets" + path.sep)) continue;
    if (await processMarkdown(file)) mdCount++;
  }
  const assetCount = await copyAssets();
  console.log(`sync-docs: ${mdCount} markdown files, ${assetCount} assets`);
}

main().catch((e) => {
  console.error("sync-docs failed:", e);
  process.exit(1);
});
