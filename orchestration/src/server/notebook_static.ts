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
 * Static-file route for the notebook SPA (D22 §6.10).
 *
 * When `EIGENIUS_NOTEBOOK_STATIC` points at a directory containing the
 * Vite build output (`index.html` + `assets/`), the orchestrator serves
 * the notebook UI under `/notebooks/*` from that directory. SPA routing
 * is handled by serving `index.html` for any path that doesn't resolve
 * to a real file inside the static root.
 *
 * Path traversal is blocked by resolving each request against the
 * static root and refusing anything that escapes it.
 */

import { contentType } from "@std/media-types/content-type";
import { extname, join, resolve, SEPARATOR } from "@std/path";

const NOTEBOOK_PREFIX = "/notebooks";

export interface NotebookStaticHandler {
  /** Try to serve the request from the static root. Returns `null` if
   *  the path doesn't fall under `/notebooks/*` (so the caller can
   *  fall through to RPC routing). */
  tryServe: (req: Request) => Promise<Response | null>;
}

/**
 * Build a static-file handler rooted at `staticRoot`. The directory must
 * already contain a Vite-style `index.html`; Phase 2's notebook build
 * (`vite build`) emits one with `base: "/notebooks/"`.
 */
export function createNotebookStaticHandler(
  staticRoot: string,
): NotebookStaticHandler {
  const root = resolve(staticRoot);

  return {
    async tryServe(req: Request): Promise<Response | null> {
      const url = new URL(req.url);
      if (
        url.pathname !== NOTEBOOK_PREFIX &&
        !url.pathname.startsWith(`${NOTEBOOK_PREFIX}/`)
      ) {
        return null;
      }
      // Only GET / HEAD reach the static handler — POST/PUT/etc. fall
      // through to RPC routing in case a future RPC ends up under
      // /notebooks (none today).
      if (req.method !== "GET" && req.method !== "HEAD") return null;

      // Strip the prefix to get the path relative to the static root.
      let rel = url.pathname.slice(NOTEBOOK_PREFIX.length) || "/";
      if (rel === "/" || rel === "") rel = "/index.html";

      const filePath = resolve(join(root, rel));
      // Refuse path-traversal attempts: the resolved file must remain
      // inside the static root.
      if (
        filePath !== root && !filePath.startsWith(root + SEPARATOR)
      ) {
        return new Response("Forbidden", { status: 403 });
      }

      try {
        const file = await Deno.open(filePath, { read: true });
        const stat = await file.stat();
        if (!stat.isFile) {
          file.close();
          return spaFallback(root);
        }
        const headers = new Headers({
          "Content-Type": contentType(extname(filePath)) ??
            "application/octet-stream",
          "Content-Length": String(stat.size),
        });
        if (req.method === "HEAD") {
          file.close();
          return new Response(null, { status: 200, headers });
        }
        return new Response(file.readable, { status: 200, headers });
      } catch (err) {
        if (err instanceof Deno.errors.NotFound) {
          // SPA fallback — Vite-built apps handle their own routing
          // client-side, so any unknown path under /notebooks/ should
          // serve index.html and let the React router resolve it.
          return spaFallback(root);
        }
        throw err;
      }
    },
  };
}

async function spaFallback(root: string): Promise<Response> {
  const indexPath = join(root, "index.html");
  try {
    const file = await Deno.open(indexPath, { read: true });
    const stat = await file.stat();
    return new Response(file.readable, {
      status: 200,
      headers: {
        "Content-Type": "text/html; charset=utf-8",
        "Content-Length": String(stat.size),
      },
    });
  } catch {
    return new Response("Notebook SPA not found", { status: 404 });
  }
}
