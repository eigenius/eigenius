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

"""Convert the WRN paper's Supplementary Table 1 workbook to the pinned CSV.

This is the committed counterpart of the previously-undocumented "stdlib
zipfile + xml.etree parser" referenced in data/MANIFEST.md. It closes the one
remaining hole in the provenance chain: the raw paper supplement
(`wrn_supplementary_table_1.xlsx`, the NIHMS1522798 supplement) -> the
machine-readable `wrn_supplementary_table_1.csv` that every Phase-1 SampleSet
projects from.

Faithful reproduction: cell values are emitted verbatim (shared-string text or
the raw stored `<v>` numeric text — never reformatted), empty cells become
`NA`, and the standard library `csv` writer (minimal quoting, CRLF line
endings) matches the pinned content hash byte-for-byte.

Stdlib only — no pandas/openpyxl — so it runs anywhere `python3` does.

Usage (from data/slices/):
    python3 ../../extract/supp_table_1_to_csv.py
    python3 ../../extract/supp_table_1_to_csv.py --check   # verify, don't write

The pinned content hash (data/sources.tsv) is
    sha256(wrn_supplementary_table_1.csv) = eebd460257982a98cf6ce9f14e189ae0c4398a686f4181bc037c5591e87243f2
"""

import argparse
import csv
import hashlib
import io
import sys
import xml.etree.ElementTree as ET
import zipfile

NS = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"

# Empty cells render as this token (matches the original derivation + R's NA).
NA = "NA"


def col_index(cell_ref):
    """'A1' -> 0, 'B2' -> 1, 'AA3' -> 26 (zero-based column index)."""
    letters = "".join(ch for ch in cell_ref if ch.isalpha())
    idx = 0
    for ch in letters:
        idx = idx * 26 + (ord(ch) - ord("A") + 1)
    return idx - 1


def load_shared_strings(zf):
    """Return the workbook's shared-string table (index -> concatenated text)."""
    try:
        raw = zf.read("xl/sharedStrings.xml")
    except KeyError:
        return []
    root = ET.fromstring(raw)
    strings = []
    for si in root.findall(f"{NS}si"):
        # A shared string is either a single <t> or several <r><t> rich-text
        # runs; concatenate every <t> descendant to recover the full value.
        strings.append("".join(t.text or "" for t in si.iter(f"{NS}t")))
    return strings


def first_sheet_path(zf):
    """Resolve the path of the workbook's first sheet via the rels map."""
    wb = ET.fromstring(zf.read("xl/workbook.xml"))
    rels = ET.fromstring(zf.read("xl/_rels/workbook.xml.rels"))
    rid = wb.find(f"{NS}sheets/{NS}sheet").get(
        "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"
    )
    rel_ns = "{http://schemas.openxmlformats.org/package/2006/relationships}"
    for rel in rels.findall(f"{rel_ns}Relationship"):
        if rel.get("Id") == rid:
            target = rel.get("Target")
            return target if target.startswith("xl/") else "xl/" + target
    return "xl/worksheets/sheet1.xml"


def cell_value(cell, shared):
    """Resolve one <c> element to its string value (shared/inline/raw numeric)."""
    t = cell.get("t")
    if t == "s":  # shared string
        v = cell.find(f"{NS}v")
        return shared[int(v.text)] if v is not None and v.text is not None else ""
    if t == "inlineStr":
        is_el = cell.find(f"{NS}is")
        return "".join(x.text or "" for x in is_el.iter(f"{NS}t")) if is_el is not None else ""
    # Numeric, boolean, or formula-string cell: emit the stored <v> verbatim.
    v = cell.find(f"{NS}v")
    return v.text if v is not None and v.text is not None else ""


def rows_from_sheet(zf, shared):
    """Yield each sheet row as a list of cell strings. Each row spans only its
    own occupied columns (per-row width = its rightmost cell + 1); internal gaps
    and blank cells become NA. Trailing empty columns are simply absent — so a
    row with no Comments value is one field shorter than the header, matching the
    original derivation exactly."""
    sheet = ET.fromstring(zf.read(first_sheet_path(zf)))
    out = []
    for row in sheet.find(f"{NS}sheetData").findall(f"{NS}row"):
        cells = {}
        for c in row.findall(f"{NS}c"):
            cells[col_index(c.get("r"))] = cell_value(c, shared)
        width = max(cells) + 1 if cells else 0
        out.append([cells[i] if cells.get(i, "") != "" else NA for i in range(width)])
    return out


def build_csv_bytes(xlsx_path):
    with zipfile.ZipFile(xlsx_path) as zf:
        shared = load_shared_strings(zf)
        rows = rows_from_sheet(zf, shared)
    buf = io.StringIO(newline="")
    writer = csv.writer(buf)  # default dialect: minimal quoting, CRLF terminators
    writer.writerows(rows)
    return buf.getvalue().encode("utf-8")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", default="wrn_supplementary_table_1.xlsx",
                    help="source workbook (default: %(default)s, in the cwd)")
    ap.add_argument("--output", default="wrn_supplementary_table_1.csv",
                    help="destination CSV (default: %(default)s, in the cwd)")
    ap.add_argument("--check", action="store_true",
                    help="derive in memory and diff against --output; do not write")
    args = ap.parse_args()

    data = build_csv_bytes(args.input)
    digest = hashlib.sha256(data).hexdigest()

    if args.check:
        try:
            existing = open(args.output, "rb").read()
        except FileNotFoundError:
            print(f"--check: {args.output} not present to compare against", file=sys.stderr)
            return 1
        if existing == data:
            print(f"OK: re-derived {args.output} byte-identical (sha256 {digest})")
            return 0
        print(f"DRIFT: re-derived {args.output} differs from the committed file "
              f"(got sha256 {digest})", file=sys.stderr)
        return 1

    with open(args.output, "wb") as f:
        f.write(data)
    n_rows = data.count(b"\r\n")
    print(f"wrote {args.output}: {n_rows} rows, sha256 {digest}")


if __name__ == "__main__":
    sys.exit(main())
