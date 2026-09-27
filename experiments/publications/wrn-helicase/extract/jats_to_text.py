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
"""The WRN author manuscript (PMC6580861, JATS) as citable text.

One line per paragraph, prefixed by a stable locator:
  [abstract.pN]           the abstract
  [body.pN]               the letter body (results and discussion, no section titles in a Letter)
  [methods/<title>.pN]    a methods subsection
  [fig1 p0] / [ed-fig5 pN] a figure's title (p0) and caption paragraphs
  [data-availability.pN], [<other back section>.pN]
Inline cross-references keep their labels (`Fig. 1a`), and citations their numbers (`[10]`).

Usage: jats_to_text.py <PMC6580861.xml> [> PMC6580861.txt]
With no argument (as `data/fetch.sh` runs it, from `data/slices/`), reads `PMC6580861.xml` and
writes `PMC6580861.txt` beside it.
"""

import re
import sys
import xml.etree.ElementTree as ET


def text_of(el):
    """An element's text with its descendants', whitespace collapsed; a citation xref keeps its
    number in brackets."""
    parts = []

    def walk(e):
        if e.tag == "xref" and e.get("ref-type") == "bibr":
            parts.append("[" + "".join(e.itertext()).strip() + "]")
        elif e.tag in ("fig", "table-wrap", "supplementary-material"):
            pass  # floats are printed on their own
        else:
            if e.text:
                parts.append(e.text)
            for c in e:
                walk(c)
                if c.tail:
                    parts.append(c.tail)
            return
        if e.tail is not None and e is not el:
            pass

    walk(el)
    return re.sub(r"\s+", " ", "".join(parts)).strip()


def title_of(el):
    """An element's <title>, with any markup inside it (`<italic>WRN</italic>`)."""
    t = el.find("title")
    return text_of(t) if t is not None else ""


def slug(title):
    return re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")[:48] or "untitled"


def paragraphs(sec, path, out):
    """Every <p> directly under `sec`, then its subsections, depth-first."""
    n = 0
    for child in sec:
        if child.tag == "p":
            t = text_of(child)
            if t:
                out.append((f"{path}.p{n}", t))
                n += 1
        elif child.tag == "sec":
            paragraphs(child, f"{path}/{slug(title_of(child))}", out)


def figure_label(fig):
    label = (fig.findtext("label") or fig.get("id") or "fig").strip()
    m = re.match(r"(Extended Data )?Fig(?:ure)?\.?\s*(\d+)", label, re.I)
    if m:
        return ("ed-fig" if m.group(1) else "fig") + m.group(2)
    return slug(label)


def main():
    src = sys.argv[1] if len(sys.argv) > 1 else "PMC6580861.xml"
    if len(sys.argv) == 1:
        sys.stdout = open("PMC6580861.txt", "w", encoding="utf-8")
    root = ET.parse(src).getroot()
    art = root.find(".//article")
    out = []
    for i, p in enumerate(art.findall("./front/article-meta/abstract//p")):
        out.append((f"abstract.p{i}", text_of(p)))
    body = art.find("./body")
    methods = None
    for sec in body.findall("./sec"):
        if "method" in title_of(sec).lower():
            methods = sec
    # The Letter's text is the body's own paragraphs and any non-methods sections.
    n = 0
    for child in body:
        if child.tag == "p":
            out.append((f"body.p{n}", text_of(child)))
            n += 1
        elif child.tag == "sec" and child is not methods:
            paragraphs(child, f"body/{slug(title_of(child))}", out)
    if methods is not None:
        paragraphs(methods, "methods", out)
    for fig in art.iter("fig"):
        lab = figure_label(fig)
        cap = fig.find("caption")
        if cap is None:
            continue
        title = title_of(cap)
        if title:
            out.append((f"{lab} p0", title))
        for i, p in enumerate(cap.findall("p"), start=1):
            out.append((f"{lab} p{i}", text_of(p)))
    back = art.find("./back")
    if back is not None:
        for sec in back.findall(".//sec"):
            for i, p in enumerate(sec.findall("p")):
                out.append((f"{slug(title_of(sec))}.p{i}", text_of(p)))
    for loc, t in out:
        print(f"[{loc}] {t}")


if __name__ == "__main__":
    main()
