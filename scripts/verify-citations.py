#!/usr/bin/env python3

# Copyright 2026 The Eigenius Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Verify BibTeX citations in docs/references/*.bib against authoritative sources.

For each entry, the script picks the strongest identifier present and
checks the bibliographic fields against what the source actually returns:

  - DOI               -> Crossref REST API (api.crossref.org/works/{doi})
  - eprint (arXiv)    -> arXiv export API (export.arxiv.org/api/query)
  - URL only          -> HTTP HEAD/GET to confirm the resource resolves
  - Title only        -> Crossref title search; reports best match for
                         manual review
  - Nothing usable    -> reported as NO_IDENTIFIER (e.g. @misc software
                         with only a software URL)

Compared fields: title (normalized), year, first-author surname.
A mismatch on any of those is reported with both sides shown.

Stdlib only — no third-party dependencies. Polite to the APIs (default
0.5s between requests). Crossref recommends supplying a contact email
in the User-Agent for the polite pool, which we do.

Exit status: 0 if no entries were MISMATCH/NOT_FOUND/UNREACHABLE, else 1.

Examples:
  scripts/verify-citations.py
  scripts/verify-citations.py docs/references/eigenius_related_work.bib
  scripts/verify-citations.py --only-flagged
  scripts/verify-citations.py --limit 5 --verbose
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

USER_AGENT = (
    "eigenius-bib-verify/1.0 "
    "(https://github.com/eigenius/eigenius; mailto:hans.martin.will@gmail.com)"
)
CROSSREF_WORKS = "https://api.crossref.org/works"
ARXIV_API = "http://export.arxiv.org/api/query"
HTTP_TIMEOUT = 15


# --------------------------------------------------------------------------- #
# BibTeX parsing
# --------------------------------------------------------------------------- #


@dataclass
class Entry:
    kind: str  # "article", "book", ...  (lowercased, no @)
    key: str
    fields: dict[str, str] = field(default_factory=dict)
    source_file: Path = field(default_factory=lambda: Path("."))
    line: int = 0


def parse_bib(path: Path) -> list[Entry]:
    """Tolerant BibTeX parser sufficient for our hand-written files.

    Handles:
      - {...} and "..." quoted values
      - nested braces inside values
      - whitespace and line breaks anywhere
      - @comment, @string, @preamble are skipped (not used in our files)
    """
    text = path.read_text(encoding="utf-8")
    entries: list[Entry] = []
    i = 0
    n = len(text)
    line_no = 1

    def advance(j: int) -> int:
        nonlocal line_no
        line_no += text.count("\n", i, j)
        return j

    while i < n:
        if text[i] != "@":
            if text[i] == "\n":
                line_no += 1
            i += 1
            continue

        m = re.match(r"@(\w+)\s*\{", text[i:])
        if not m:
            i += 1
            continue
        kind = m.group(1).lower()
        if kind in ("string", "preamble", "comment"):
            i = advance(i + m.end())
            continue
        start_line = line_no
        i = advance(i + m.end())

        comma = text.find(",", i)
        if comma < 0:
            break
        key = text[i:comma].strip()
        i = advance(comma + 1)

        fields: dict[str, str] = {}
        while i < n:
            while i < n and text[i] in " \t\n\r":
                if text[i] == "\n":
                    line_no += 1
                i += 1
            if i >= n or text[i] == "}":
                if i < n:
                    i += 1
                break
            fm = re.match(r"(\w+)\s*=\s*", text[i:])
            if not fm:
                # Skip to next comma or closing brace
                next_brace = text.find("}", i)
                next_comma = text.find(",", i)
                cands = [x for x in (next_brace, next_comma) if x >= 0]
                if not cands:
                    i = n
                    break
                i = advance(min(cands))
                continue
            fname = fm.group(1).lower()
            i = advance(i + fm.end())

            if i >= n:
                break
            if text[i] == "{":
                depth = 0
                start = i
                while i < n:
                    c = text[i]
                    if c == "{":
                        depth += 1
                    elif c == "}":
                        depth -= 1
                        if depth == 0:
                            i += 1
                            break
                    elif c == "\n":
                        line_no += 1
                    i += 1
                value = text[start + 1 : i - 1]
            elif text[i] == '"':
                start = i + 1
                i += 1
                depth = 0
                while i < n:
                    c = text[i]
                    if c == "{":
                        depth += 1
                    elif c == "}":
                        depth -= 1
                    elif c == '"' and depth == 0:
                        break
                    elif c == "\n":
                        line_no += 1
                    i += 1
                value = text[start:i]
                if i < n:
                    i += 1
            else:
                vm = re.match(r"[^,}\n]+", text[i:])
                value = vm.group(0).strip() if vm else ""
                i += len(vm.group(0)) if vm else 0

            fields[fname] = collapse_ws(value)

            while i < n and text[i] in " \t\n\r":
                if text[i] == "\n":
                    line_no += 1
                i += 1
            if i < n and text[i] == ",":
                i += 1

        entries.append(
            Entry(
                kind=kind,
                key=key,
                fields=fields,
                source_file=path,
                line=start_line,
            )
        )
    return entries


def collapse_ws(value: str) -> str:
    return re.sub(r"\s+", " ", value).strip()


# --------------------------------------------------------------------------- #
# Normalisation for comparison
# --------------------------------------------------------------------------- #


def normalize_text(t: str) -> str:
    """Lowercase, strip diacritics, collapse non-alphanumerics to spaces."""
    t = re.sub(r"[{}\\]", "", t)
    t = unicodedata.normalize("NFKD", t)
    t = "".join(c for c in t if not unicodedata.combining(c))
    t = re.sub(r"[^a-z0-9]+", " ", t.lower()).strip()
    return t


def first_author_surname(authors_field: str) -> str:
    """Return the (normalised) surname of the first listed author/editor."""
    if not authors_field:
        return ""
    first = re.split(r"\s+and\s+", authors_field, maxsplit=1)[0]
    if "," in first:
        last = first.split(",", 1)[0].strip()
    else:
        toks = re.findall(r"[^\s{}]+", first)
        last = toks[-1] if toks else ""
    return normalize_text(last)


def is_corporate_or_placeholder(authors_field: str) -> bool:
    """Crossref doesn't return a person surname for corporate/anonymous
    entries; skip the surname check in those cases."""
    if not authors_field:
        return True
    a = authors_field.lower()
    return (
        "anonymous" in a
        or "others" == a.strip()
        or a.strip().startswith("{") and a.strip().endswith("}")
    )


# --------------------------------------------------------------------------- #
# HTTP helpers
# --------------------------------------------------------------------------- #


def http_request(
    url: str, *, method: str = "GET", accept: str = "*/*"
) -> tuple[int, bytes]:
    req = urllib.request.Request(
        url,
        method=method,
        headers={"User-Agent": USER_AGENT, "Accept": accept},
    )
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as r:
            body = b"" if method == "HEAD" else r.read()
            return r.status, body
    except urllib.error.HTTPError as e:
        return e.code, b""
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        return 0, str(e).encode()


# --------------------------------------------------------------------------- #
# Lookups
# --------------------------------------------------------------------------- #


def lookup_doi(doi: str) -> Optional[dict]:
    url = f"{CROSSREF_WORKS}/{urllib.parse.quote(doi, safe='/:.')}"
    status, body = http_request(url, accept="application/json")
    if status != 200 or not body:
        return None
    try:
        msg = json.loads(body).get("message", {})
    except json.JSONDecodeError:
        return None
    title = (msg.get("title") or [""])[0]
    year = None
    # Prefer published-print over issued: Crossref's `issued` is the
    # earliest known publication date, which for journal articles is
    # often the online date. The conventional citation year is the
    # print year, so we look there first.
    for k in ("published-print", "published", "issued", "published-online", "created"):
        parts = (msg.get(k) or {}).get("date-parts")
        if parts and parts[0]:
            year = parts[0][0]
            break
    authors = msg.get("author") or msg.get("editor") or []
    first_surname = ""
    if authors:
        first_surname = (authors[0].get("family") or "").strip()
    return {"title": title, "year": year, "first_surname": first_surname}


def search_crossref_title(title: str, year: Optional[str]) -> Optional[dict]:
    qs = {"query.title": title, "rows": "1"}
    if year:
        qs["filter"] = f"from-pub-date:{year},until-pub-date:{year}"
    url = f"{CROSSREF_WORKS}?{urllib.parse.urlencode(qs)}"
    status, body = http_request(url, accept="application/json")
    if status != 200 or not body:
        return None
    try:
        items = json.loads(body).get("message", {}).get("items") or []
    except json.JSONDecodeError:
        return None
    if not items:
        return None
    item = items[0]
    item_title = (item.get("title") or [""])[0]
    parts = (item.get("issued") or {}).get("date-parts")
    item_year = parts[0][0] if parts and parts[0] else None
    authors = item.get("author") or []
    first_surname = (authors[0].get("family") or "").strip() if authors else ""
    return {
        "title": item_title,
        "year": item_year,
        "first_surname": first_surname,
        "doi": item.get("DOI"),
    }


def lookup_arxiv(eprint: str) -> Optional[dict]:
    url = f"{ARXIV_API}?id_list={urllib.parse.quote(eprint)}"
    status, body = http_request(url, accept="application/atom+xml")
    if status != 200 or not body:
        return None
    try:
        root = ET.fromstring(body)
    except ET.ParseError:
        return None
    ns = {"a": "http://www.w3.org/2005/Atom"}
    entry = root.find("a:entry", ns)
    if entry is None:
        return None
    title_el = entry.find("a:title", ns)
    title = collapse_ws(title_el.text or "") if title_el is not None else ""
    pub_el = entry.find("a:published", ns)
    year = None
    if pub_el is not None and pub_el.text and len(pub_el.text) >= 4:
        try:
            year = int(pub_el.text[:4])
        except ValueError:
            year = None
    name_els = entry.findall("a:author/a:name", ns)
    first_surname = ""
    if name_els and name_els[0].text:
        toks = name_els[0].text.strip().split()
        first_surname = toks[-1] if toks else ""
    # arXiv returns the title with a leading "Title: " sometimes; normalise
    return {"title": title, "year": year, "first_surname": first_surname}


# --------------------------------------------------------------------------- #
# Verification
# --------------------------------------------------------------------------- #


@dataclass
class Result:
    entry: Entry
    status: str  # OK | MISMATCH | NOT_FOUND | UNREACHABLE | NO_IDENTIFIER
    detail: str = ""


def compare_meta(
    entry: Entry, meta: dict, *, source: str
) -> Result:
    bib_title = normalize_text(entry.fields.get("title", ""))
    bib_year = entry.fields.get("year", "").strip()
    bib_first = first_author_surname(
        entry.fields.get("author") or entry.fields.get("editor", "")
    )
    skip_author = is_corporate_or_placeholder(
        entry.fields.get("author") or entry.fields.get("editor", "")
    )

    issues: list[str] = []
    ref_title = normalize_text(meta.get("title") or "")
    if bib_title and ref_title and ref_title != bib_title:
        # tolerate subtitle differences
        if not (ref_title.startswith(bib_title) or bib_title.startswith(ref_title)):
            issues.append(f"title: bib={bib_title!r} src={ref_title!r}")
    if bib_year and meta.get("year") and str(meta["year"]) != bib_year:
        # bib_year may be a range like "2020--2026"; accept if year falls inside
        if not year_matches_range(bib_year, meta["year"]):
            issues.append(f"year: bib={bib_year} src={meta['year']}")
    if not skip_author and bib_first and meta.get("first_surname"):
        ref_first = normalize_text(meta["first_surname"])
        if ref_first and ref_first != bib_first:
            issues.append(f"author: bib={bib_first!r} src={ref_first!r}")

    if issues:
        return Result(entry, "MISMATCH", f"{source}: " + "; ".join(issues))
    return Result(entry, "OK", source)


def year_matches_range(bib_year: str, ref_year: int) -> bool:
    """Tolerate `--`-separated ranges (e.g. 2020--2026) and 'YYYY/MM' forms."""
    nums = [int(x) for x in re.findall(r"\d{4}", bib_year)]
    if not nums:
        return False
    return min(nums) <= ref_year <= max(nums)


def verify_entry(e: Entry, *, delay: float) -> Result:
    fields = e.fields

    if doi := fields.get("doi"):
        meta = lookup_doi(doi)
        time.sleep(delay)
        if not meta:
            return Result(e, "NOT_FOUND", f"DOI not found: {doi}")
        return compare_meta(e, meta, source=f"DOI {doi}")

    if eprint := fields.get("eprint"):
        meta = lookup_arxiv(eprint)
        time.sleep(delay)
        if not meta:
            return Result(e, "NOT_FOUND", f"arXiv not found: {eprint}")
        return compare_meta(e, meta, source=f"arXiv {eprint}")

    if url := fields.get("url"):
        # Try HEAD first; if it 405s or 0s, fall back to GET.
        status, _ = http_request(url, method="HEAD")
        if status in (0, 403, 405):
            status, _ = http_request(url, method="GET")
        time.sleep(delay)
        if status == 0:
            return Result(e, "UNREACHABLE", f"connection error: {url}")
        if status >= 400:
            return Result(e, "UNREACHABLE", f"HTTP {status}: {url}")
        return Result(e, "OK", f"URL {status}: {url}")

    # No identifier — try a Crossref title search for context.
    title = fields.get("title", "").strip()
    year = fields.get("year", "").strip() or None
    if title:
        guess = search_crossref_title(title, year)
        time.sleep(delay)
        if guess:
            return Result(
                e,
                "NO_IDENTIFIER",
                f"no doi/eprint/url; nearest Crossref hit: "
                f"{guess.get('first_surname','?')} ({guess.get('year','?')}) "
                f"doi={guess.get('doi','?')}",
            )
    return Result(e, "NO_IDENTIFIER", "no doi/eprint/url and no Crossref hit")


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


STATUS_ORDER = ("OK", "MISMATCH", "NOT_FOUND", "UNREACHABLE", "NO_IDENTIFIER")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n", 1)[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("\n", 2)[2] if "\n" in __doc__ else "",
    )
    ap.add_argument(
        "paths",
        nargs="*",
        type=Path,
        help="bib files to verify (default: docs/references/*.bib)",
    )
    ap.add_argument(
        "--only-flagged",
        action="store_true",
        help="restrict to entries whose `note` contains 'verif' "
        "(matches our 'to be verified' tags)",
    )
    ap.add_argument(
        "--limit",
        type=int,
        default=0,
        help="stop after N entries (0 = all)",
    )
    ap.add_argument(
        "--delay",
        type=float,
        default=0.5,
        help="seconds between HTTP requests (default 0.5)",
    )
    ap.add_argument(
        "--key",
        action="append",
        default=[],
        help="restrict to specific citation key(s); repeatable",
    )
    ap.add_argument(
        "--verbose",
        "-v",
        action="store_true",
        help="print OK lines too (default: only non-OK lines and summary)",
    )
    args = ap.parse_args(argv)

    if not args.paths:
        repo_root = Path(__file__).resolve().parents[1]
        args.paths = sorted((repo_root / "docs/references").glob("*.bib"))

    entries: list[Entry] = []
    for p in args.paths:
        if not p.exists():
            print(f"warning: missing file {p}", file=sys.stderr)
            continue
        entries.extend(parse_bib(p))

    if args.key:
        wanted = set(args.key)
        entries = [e for e in entries if e.key in wanted]
    if args.only_flagged:
        entries = [
            e for e in entries if "verif" in e.fields.get("note", "").lower()
        ]
    if args.limit:
        entries = entries[: args.limit]

    if not entries:
        print("no entries to verify")
        return 0

    print(f"Verifying {len(entries)} entries...\n")
    results: list[Result] = []
    for i, e in enumerate(entries, 1):
        r = verify_entry(e, delay=args.delay)
        results.append(r)
        rel = e.source_file.name
        if args.verbose or r.status != "OK":
            print(f"[{i:3d}/{len(entries)}] {r.status:14s} {e.key:38s} {rel}")
            if r.detail:
                print(f"               {r.detail}")

    print()
    print("Summary:")
    by_status: dict[str, list[Result]] = {}
    for r in results:
        by_status.setdefault(r.status, []).append(r)
    for s in STATUS_ORDER:
        print(f"  {s:14s} {len(by_status.get(s, []))}")

    bad = sum(
        len(by_status.get(s, []))
        for s in ("MISMATCH", "NOT_FOUND", "UNREACHABLE")
    )
    if bad:
        noun = "entry needs" if bad == 1 else "entries need"
        print(f"\n{bad} {noun} attention")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
