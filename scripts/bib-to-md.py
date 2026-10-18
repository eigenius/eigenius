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

"""Render docs/references/*.bib as a four-file reference guide under
docs/guides/references/. Entries within each file are sorted by citation key.

Each output markdown file gets a short header derived from the bib file's
purpose and a flat list of `### \\`key\\`` entries rendered in a compact
academic-citation style. Notes are preserved as blockquotes.

Stdlib only --- no third-party dependencies. The bibtex parser is the same
one used by verify-citations.py; it's duplicated here to keep both scripts
independently runnable rather than introducing a shared helper module.

Usage:
  scripts/bib-to-md.py                 # render all four bib files
  scripts/bib-to-md.py --check         # exit 1 if any output would change
  scripts/bib-to-md.py --stdout key.bib  # print one file to stdout instead
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------- #
# Bibtex parser (lifted from verify-citations.py).
# --------------------------------------------------------------------------- #


@dataclass
class Entry:
    kind: str
    key: str
    fields: dict[str, str] = field(default_factory=dict)
    source_file: Path = field(default_factory=lambda: Path("."))
    line: int = 0


def parse_bib(path: Path) -> list[Entry]:
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

            fields[fname] = re.sub(r"\s+", " ", value).strip()

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


# --------------------------------------------------------------------------- #
# LaTeX-to-Unicode for human-readable rendering.
# --------------------------------------------------------------------------- #

# Two-character accent commands first so they don't get partially matched.
LATEX_ACCENTS = {
    # Umlauts
    r'\"a': "ä", r'\"o': "ö", r'\"u': "ü", r'\"A': "Ä", r'\"O': "Ö", r'\"U': "Ü",
    r'\"e': "ë", r'\"i': "ï", r'\"y': "ÿ",
    # Acute
    r"\'a": "á", r"\'e": "é", r"\'i": "í", r"\'o": "ó", r"\'u": "ú",
    r"\'A": "Á", r"\'E": "É", r"\'I": "Í", r"\'O": "Ó", r"\'U": "Ú",
    r"\'c": "ć", r"\'C": "Ć", r"\'n": "ń", r"\'N": "Ń", r"\'s": "ś", r"\'S": "Ś",
    # Grave
    r"\`a": "à", r"\`e": "è", r"\`i": "ì", r"\`o": "ò", r"\`u": "ù",
    # Circumflex
    r"\^a": "â", r"\^e": "ê", r"\^i": "î", r"\^o": "ô", r"\^u": "û",
    # Tilde
    r"\~a": "ã", r"\~n": "ñ", r"\~o": "õ",
    # Caron / hacek (\v X)
    r"\v c": "č", r"\v C": "Č", r"\v s": "š", r"\v S": "Š",
    r"\v z": "ž", r"\v Z": "Ž", r"\v n": "ň", r"\v r": "ř",
    # Breve (\u X)  -- e.g. Romanian a-breve
    r"\u a": "ă", r"\u A": "Ă", r"\u g": "ğ", r"\u G": "Ğ",
    # Cedilla (\c X)
    r"\c c": "ç", r"\c C": "Ç", r"\c s": "ş", r"\c S": "Ş",
    r"\c t": "ţ", r"\c T": "Ţ",
    # Stroke / slash
    r"\o": "ø", r"\O": "Ø", r"\l": "ł", r"\L": "Ł",
    # Ring
    r"\r a": "å", r"\r A": "Å",
    # Special letters
    r"\ss": "ß", r"\ae": "æ", r"\AE": "Æ", r"\oe": "œ", r"\OE": "Œ",
    # Common escaped punctuation
    r"\&": "&", r"\#": "#", r"\_": "_", r"\%": "%", r"\$": "$",
}

# Compile a single regex that matches any LaTeX accent in priority order.
_ACCENT_RE = re.compile(
    "|".join(re.escape(k) for k in sorted(LATEX_ACCENTS, key=len, reverse=True))
)


def latex_to_unicode(s: str) -> str:
    """Convert common LaTeX accent commands and dashes to Unicode, then drop
    protective braces. Best-effort: anything we don't know about is left as-is."""
    if not s:
        return ""
    # Apply accent substitutions before brace stripping so that {\\"a} works.
    s = _ACCENT_RE.sub(lambda m: LATEX_ACCENTS[m.group(0)], s)
    # Em-dash and en-dash
    s = s.replace("---", "—").replace("--", "–")
    # Non-breaking space
    s = s.replace("~", " ")
    # Strip protective braces (e.g. {ACM}). Repeat in case of {{X}}.
    prev = None
    while prev != s:
        prev = s
        s = re.sub(r"\{([^{}]*)\}", r"\1", s)
    # Collapse any whitespace runs introduced by replacements.
    s = re.sub(r"\s+", " ", s).strip()
    return s


# --------------------------------------------------------------------------- #
# Author formatting.
# --------------------------------------------------------------------------- #


def format_authors(raw: str) -> str:
    """Format a BibTeX author/editor field for human reading.

    Keeps the "Last, First" form for each author (it's the canonical academic
    style). Joins with commas plus 'and' before the last name to avoid Oxford
    comma debates.

    Special cases:
      - Corporate authors wrapped in {} are kept as-is.
      - "others" becomes "et al."
      - Empty input returns empty string.
    """
    if not raw:
        return ""
    parts = re.split(r"\s+and\s+", raw)
    cleaned = []
    for p in parts:
        p = p.strip()
        if p.lower() == "others":
            cleaned.append("et al.")
            continue
        cleaned.append(latex_to_unicode(p))
    if len(cleaned) == 0:
        return ""
    if len(cleaned) == 1:
        return cleaned[0]
    # If the last token is "et al." (i.e. the field used "and others"),
    # join without "and" before it: "Smith, J., et al." not "Smith, J. and et al."
    last_is_etal = cleaned[-1] == "et al."
    if len(cleaned) == 2:
        if last_is_etal:
            return f"{cleaned[0]}, et al."
        return f"{cleaned[0]} and {cleaned[1]}"
    if last_is_etal:
        return ", ".join(cleaned[:-1]) + ", et al."
    return ", ".join(cleaned[:-1]) + ", and " + cleaned[-1]


# --------------------------------------------------------------------------- #
# Entry rendering.
# --------------------------------------------------------------------------- #


def render_entry(e: Entry) -> str:
    """Render one entry as a markdown subsection.

    The shape varies by entry kind, but the general structure is:
        ### `key`
        Authors (year). "Title". *Venue*, vol(no):pp.
        [DOI](...) · [arXiv](...) · [URL](...)
        > Note (if present)
    """
    f = e.fields
    parts: list[str] = [f"### `{e.key}`", ""]

    title = latex_to_unicode(f.get("title", ""))
    # Year may be a range like "2020--2026"; convert dashes for display.
    year = f.get("year", "").strip().replace("---", "—").replace("--", "–")

    # Author or editor (with "(ed.)" marker if it's an editor)
    if "author" in f:
        people = format_authors(f["author"])
    elif "editor" in f:
        people = format_authors(f["editor"])
        if people:
            people += " (ed.)"
    else:
        people = ""

    # The lead line: who wrote it and when.
    lead_bits: list[str] = []
    if people:
        lead_bits.append(people)
    if year:
        lead_bits.append(f"({year}).")
    elif people:
        lead_bits[-1] = lead_bits[-1] + "."

    # Title presentation: books and theses get italics, everything else
    # gets quoted-string treatment. Software (@misc with no journal/url
    # context) gets italics too.
    italic_kinds = {"book", "phdthesis", "mastersthesis"}
    if e.kind in italic_kinds or (e.kind == "misc" and "journal" not in f):
        title_str = f"*{title}*."
    else:
        title_str = f'"{title}".'
    if title:
        lead_bits.append(title_str)
    lead = " ".join(lead_bits)

    # Venue line: assembled from kind-appropriate fields.
    venue = render_venue(e)

    if lead and venue:
        parts.append(f"{lead} {venue}")
    elif lead:
        parts.append(lead)
    elif venue:
        parts.append(venue)
    parts.append("")

    # Identifier links
    links: list[str] = []
    if doi := f.get("doi"):
        links.append(f"[DOI: {doi}](https://doi.org/{doi})")
    if eprint := f.get("eprint"):
        archive = f.get("archivePrefix", "arXiv")
        links.append(f"[{archive}:{eprint}](https://arxiv.org/abs/{eprint})")
    if url := f.get("url"):
        # Skip the URL if it duplicates an identifier we already emitted.
        is_doi_dupe = doi and url.endswith(doi)
        is_arxiv_dupe = eprint and eprint in url
        if not (is_doi_dupe or is_arxiv_dupe):
            links.append(f"[Link]({url})")
    if links:
        parts.append(" · ".join(links))
        parts.append("")

    # Note as blockquote
    if note := f.get("note"):
        cleaned = latex_to_unicode(note)
        # Wrap each line of the note in '> ' for the blockquote.
        for line in cleaned.split("\n"):
            parts.append(f"> {line}")
        parts.append("")

    return "\n".join(parts)


def render_venue(e: Entry) -> str:
    """Render the venue / volume / pages line appropriate to the entry kind."""
    f = e.fields

    def opt(field: str, prefix: str = "", suffix: str = "") -> str:
        v = f.get(field, "").strip()
        return f"{prefix}{latex_to_unicode(v)}{suffix}" if v else ""

    vol = f.get("volume", "").strip()
    num = f.get("number", "").strip()
    pages = f.get("pages", "").replace("--", "–").strip()

    if e.kind == "article":
        bits = [f"*{latex_to_unicode(f['journal'])}*"] if f.get("journal") else []
        if vol and num:
            bits.append(f"{vol}({num})")
        elif vol:
            bits.append(vol)
        if pages:
            bits.append(f"pp. {pages}")
        return ", ".join(bits) + "." if bits else ""

    if e.kind in ("inproceedings", "incollection", "conference"):
        bits = []
        bt = f.get("booktitle", "")
        if bt:
            bits.append(f"In *{latex_to_unicode(bt)}*")
        ed = f.get("editor", "")
        if ed:
            bits.append(f"ed. {format_authors(ed)}")
        ser = f.get("series", "")
        if ser and vol:
            bits.append(f"{latex_to_unicode(ser)} {vol}")
        elif ser:
            bits.append(latex_to_unicode(ser))
        elif vol:
            bits.append(f"vol. {vol}")
        if pages:
            bits.append(f"pp. {pages}")
        if pub := f.get("publisher"):
            bits.append(latex_to_unicode(pub))
        return ", ".join(bits) + "." if bits else ""

    if e.kind == "book":
        bits = []
        if ser := f.get("series"):
            bits.append(latex_to_unicode(ser))
        if vol:
            bits.append(f"vol. {vol}")
        if pub := f.get("publisher"):
            bits.append(latex_to_unicode(pub))
        if addr := f.get("address"):
            bits.append(latex_to_unicode(addr))
        return ", ".join(bits) + "." if bits else ""

    if e.kind == "phdthesis":
        sch = latex_to_unicode(f.get("school", ""))
        return f"PhD thesis, {sch}." if sch else "PhD thesis."

    if e.kind == "mastersthesis":
        sch = latex_to_unicode(f.get("school", ""))
        return f"Master's thesis, {sch}." if sch else "Master's thesis."

    if e.kind == "techreport":
        bits = []
        if inst := f.get("institution"):
            bits.append(latex_to_unicode(inst))
        if typ := f.get("type"):
            bits.append(latex_to_unicode(typ))
        if num:
            bits.append(num)
        return ", ".join(bits) + "." if bits else ""

    if e.kind == "misc":
        bits = []
        if how := f.get("howpublished"):
            bits.append(latex_to_unicode(how))
        return (", ".join(bits) + ".") if bits else ""

    # Fallback: dump publisher/howpublished if present.
    bits = []
    for k in ("howpublished", "publisher"):
        if v := f.get(k):
            bits.append(latex_to_unicode(v))
    return (", ".join(bits) + ".") if bits else ""


# --------------------------------------------------------------------------- #
# File-level rendering.
# --------------------------------------------------------------------------- #

# bib basename -> (output filename, page title, one-paragraph blurb)
FILE_MAP: dict[str, tuple[str, str, str]] = {
    "eigenius": (
        "01-cited.md",
        "Cited references",
        "References that are explicitly cited from the design documents, "
        "papers, or guides. The single source of truth for `\\cite{...}` "
        "calls in the LaTeX papers; new entries follow the same lowercase-"
        "key convention.",
    ),
    "eigenius_additional": (
        "02-foundational.md",
        "Foundational works the system relies on",
        "Foundational publications that Eigenius depends on conceptually --- "
        "type theory, codata, Datalog, knowledge representation, LLMs, "
        "WebAssembly, SMT, RPC --- but does not yet cite from any design doc, "
        "paper, or guide. Listed here so future bibliography passes have a "
        "single place to draw from.",
    ),
    "eigenius_precursors": (
        "03-precursors.md",
        "Philosophical and methodological precursors",
        "Works that situate Eigenius within a longer arc of research on "
        "Mathematical Knowledge Management, verified-at-scale formalization, "
        "Suppes-style structuralism, formal ontologies in science, the "
        "reproducibility movement, and adjacent contemporary projects.",
    ),
    "eigenius_related_work": (
        "04-related-work.md",
        "Contemporary related work",
        "Contemporary work in applied formal reasoning for science and "
        "engineering: institution theory in physics and systems engineering, "
        "higher-order logic for the natural sciences, formal ontologies for "
        "engineering and chemistry, Homotopy Type Theory and its directed "
        "and dynamic extensions, and the epistemology of formal proof.",
    ),
}


def render_file(bib_path: Path, entries: list[Entry]) -> str:
    """Build the full markdown document for one bib file."""
    base = bib_path.stem
    if base not in FILE_MAP:
        raise SystemExit(f"unknown bib basename: {base}")
    _, title, blurb = FILE_MAP[base]

    out = [f"# {title}", "", blurb, ""]
    out.append(
        f"_Generated from `docs/references/{bib_path.name}` by "
        f"`scripts/bib-to-md.py`. Do not edit by hand._"
    )
    out.append("")
    out.append(f"Total entries: **{len(entries)}**.")
    out.append("")
    out.append("---")
    out.append("")

    for e in sorted(entries, key=lambda x: x.key.lower()):
        out.append(render_entry(e))

    return "\n".join(out).rstrip() + "\n"


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n", 1)[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument(
        "--check",
        action="store_true",
        help="exit 1 if any output file would change (CI mode)",
    )
    ap.add_argument(
        "--stdout",
        type=Path,
        metavar="FILE.bib",
        help="render one bib file to stdout instead of writing to disk",
    )
    args = ap.parse_args(argv)

    repo_root = Path(__file__).resolve().parents[1]
    refs_dir = repo_root / "docs/references"
    out_dir = repo_root / "docs/guides/references"
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.stdout is not None:
        bib = args.stdout if args.stdout.is_absolute() else refs_dir / args.stdout.name
        entries = parse_bib(bib)
        sys.stdout.write(render_file(bib, entries))
        return 0

    differences = []
    for base, (out_name, _, _) in FILE_MAP.items():
        bib = refs_dir / f"{base}.bib"
        if not bib.exists():
            print(f"warning: missing {bib}", file=sys.stderr)
            continue
        entries = parse_bib(bib)
        rendered = render_file(bib, entries)
        out_path = out_dir / out_name

        if args.check:
            existing = out_path.read_text(encoding="utf-8") if out_path.exists() else ""
            if existing != rendered:
                differences.append(out_path)
        else:
            out_path.write_text(rendered, encoding="utf-8")
            print(f"wrote {out_path.relative_to(repo_root)} ({len(entries)} entries)")

    if args.check and differences:
        print(f"\n{len(differences)} file(s) out of date:", file=sys.stderr)
        for p in differences:
            print(f"  {p.relative_to(repo_root)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
