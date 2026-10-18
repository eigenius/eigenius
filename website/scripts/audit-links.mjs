// Walk the built `dist/` directory, extract every <a href> from every
// HTML file, and report any internal href that doesn't resolve to a
// built page or static asset.
//
// External links (http://, https://, mailto:) are reported as a
// summary count and skipped — verifying them is a separate job.
// Anchors-only (#foo) are also skipped.
//
// Run from website/: node scripts/audit-links.mjs

import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = path.resolve(__dirname, "..");
const DIST = path.join(WEBSITE_ROOT, "dist");

const HREF_RE = /<a\s+[^>]*href\s*=\s*"([^"]+)"[^>]*>/gi;
const ID_RE = /\sid\s*=\s*"([^"]+)"/gi;

async function* walk(dir) {
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(full);
    else yield full;
  }
}

function pageRouteForHtml(htmlFile) {
  // dist/foo/bar/index.html → /foo/bar/
  // dist/index.html         → /
  const rel = path.relative(DIST, htmlFile);
  const dir = path.dirname(rel);
  if (dir === "." && path.basename(rel) === "index.html") return "/";
  if (path.basename(rel) === "index.html") {
    return "/" + dir.split(path.sep).join("/") + "/";
  }
  return "/" + rel.split(path.sep).join("/");
}

async function pathExistsInDist(urlPath) {
  // Strip query string (we never use them, but be safe).
  urlPath = urlPath.split("?")[0];
  // Reject anything trying to escape dist via "../".
  if (urlPath.includes("..")) return false;
  // Try the URL as-is, then as a directory index.
  const cleanPath = urlPath.replace(/^\//, "");
  const candidates = [];
  if (urlPath.endsWith("/")) {
    candidates.push(path.join(DIST, cleanPath, "index.html"));
  } else {
    candidates.push(path.join(DIST, cleanPath));
    candidates.push(path.join(DIST, cleanPath, "index.html"));
    if (!path.extname(cleanPath)) {
      candidates.push(path.join(DIST, cleanPath + ".html"));
    }
  }
  for (const c of candidates) {
    try {
      const st = await fs.stat(c);
      if (st.isFile()) return true;
    } catch {
      // not found, try next
    }
  }
  return false;
}

// Find the file the URL resolves to (for anchor checks).
async function fileForUrl(urlPath) {
  urlPath = urlPath.split("?")[0];
  const cleanPath = urlPath.replace(/^\//, "");
  const candidates = urlPath.endsWith("/")
    ? [path.join(DIST, cleanPath, "index.html")]
    : [
        path.join(DIST, cleanPath, "index.html"),
        path.join(DIST, cleanPath),
        path.join(DIST, cleanPath + ".html"),
      ];
  for (const c of candidates) {
    try {
      const st = await fs.stat(c);
      if (st.isFile() && c.endsWith(".html")) return c;
    } catch {}
  }
  return null;
}

async function collectIds(htmlFile) {
  const html = await fs.readFile(htmlFile, "utf8");
  const ids = new Set();
  let m;
  ID_RE.lastIndex = 0;
  while ((m = ID_RE.exec(html))) ids.add(m[1]);
  return ids;
}

async function main() {
  console.log(`audit-links: scanning ${path.relative(WEBSITE_ROOT, DIST)}`);

  const broken = []; // { fromRoute, href, kind }
  let externalCount = 0;
  let internalCount = 0;
  let pagesScanned = 0;

  // Memoize id sets per file so repeated anchor checks are cheap.
  const idCache = new Map();

  for await (const file of walk(DIST)) {
    if (!file.endsWith(".html")) continue;
    pagesScanned++;
    const fromRoute = pageRouteForHtml(file);
    const html = await fs.readFile(file, "utf8");

    HREF_RE.lastIndex = 0;
    let match;
    while ((match = HREF_RE.exec(html))) {
      let href = match[1].trim();
      if (!href) continue;

      // External (and mailto:, tel:, javascript:): skip.
      if (/^[a-z][a-z0-9+.-]*:/i.test(href) && !href.startsWith("//")) {
        externalCount++;
        continue;
      }
      if (href.startsWith("//")) {
        externalCount++;
        continue;
      }
      // Pure anchor on the current page.
      if (href.startsWith("#")) {
        internalCount++;
        const ids = idCache.get(file) ?? (await collectIds(file));
        idCache.set(file, ids);
        const anchor = href.slice(1);
        if (anchor && !ids.has(anchor)) {
          broken.push({ fromRoute, href, kind: "anchor-on-self" });
        }
        continue;
      }

      internalCount++;

      // Split off the anchor for anchored cross-page links.
      const [pathPart, anchor] = href.split("#");

      // Resolve relative paths against fromRoute.
      let urlPath;
      if (pathPart.startsWith("/")) {
        urlPath = pathPart;
      } else if (pathPart === "") {
        // pure anchor handled above; defensive fallthrough
        continue;
      } else {
        // Relative — resolve against the current route's directory.
        const baseDir = fromRoute.endsWith("/") ? fromRoute : fromRoute + "/";
        urlPath = new URL(pathPart, "http://x" + baseDir).pathname;
      }

      const exists = await pathExistsInDist(urlPath);
      if (!exists) {
        broken.push({ fromRoute, href, kind: "page-not-found" });
        continue;
      }

      // If anchored, check the anchor exists in the target file.
      if (anchor) {
        const targetFile = await fileForUrl(urlPath);
        if (targetFile) {
          const ids =
            idCache.get(targetFile) ?? (await collectIds(targetFile));
          idCache.set(targetFile, ids);
          if (!ids.has(anchor)) {
            broken.push({
              fromRoute,
              href,
              kind: "anchor-not-found",
            });
          }
        }
      }
    }
  }

  console.log(
    `audit-links: scanned ${pagesScanned} pages, ${internalCount} internal hrefs, ${externalCount} external`,
  );

  if (!broken.length) {
    console.log("audit-links: no broken internal references ✓");
    return;
  }

  // Group by kind, then by href.
  const byHref = new Map();
  for (const b of broken) {
    const key = `${b.kind} :: ${b.href}`;
    if (!byHref.has(key)) byHref.set(key, []);
    byHref.get(key).push(b.fromRoute);
  }

  console.log(`audit-links: ${broken.length} broken reference(s):`);
  for (const [key, sources] of byHref) {
    console.log(`  ${key}`);
    const uniq = [...new Set(sources)];
    const sample = uniq.slice(0, 5);
    for (const s of sample) console.log(`      from ${s}`);
    if (uniq.length > sample.length) {
      console.log(`      ... and ${uniq.length - sample.length} more`);
    }
  }
  process.exitCode = 1;
}

main().catch((e) => {
  console.error("audit-links failed:", e);
  process.exit(1);
});
