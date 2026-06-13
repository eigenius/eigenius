#!/usr/bin/env python3
# Copyright 2026 The Eigenius Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
"""Canonical extraction recipes for the WRN Phase-1 SampleSets (Tier 1 pin).

This module is the SINGLE SOURCE OF TRUTH for the numeric arrays inlined as
`stats:sample_set_value` in ../wrn-phase1-recompute.esl. Each recipe states
exactly which pinned slice + column + filter + sort + grouping produces the
array, enforces the slice's sha256 before reading, and supports:

    python3 extract_samplesets.py --check    # re-derive + diff vs the ESL (default)
    python3 extract_samplesets.py --emit      # print ESL-ready arrays (regenerate)

The slices live under ../data/slices/ and are gitignored (≈235 MB; see
../data/MANIFEST.md). This script + the structured `bench:extracted_from_*`
provenance fields on each SampleSet are the committed, content-addressed
record; `--check` is the mechanical pin that fails loudly if an inlined
number drifts from the raw data or a slice changes.

The audit step this closes (the only previously-uncommitted link):
    raw checksummed slice  ──[column + filter + sort + group]──>  inlined SampleSet

Tier 2 (follow-up) will lift these recipes into a kernel data-ingestion
institution so the SampleSet becomes a DerivedResource witnessed by the
slice content-hash, rather than an Observed node with a recipe sidecar.
"""

import csv
import hashlib
import os
import re
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
SLICES = os.path.join(HERE, "..", "data", "slices")
ESL = os.path.join(HERE, "..", "wrn-phase1-recompute.esl")

# Pin anchors — full sha256 of each slice this extraction depends on.
# (MANIFEST.md carries the truncated forms; these are the enforced values.)
SHA256 = {
    "wrn_supplementary_table_1.csv":
        "eebd460257982a98cf6ce9f14e189ae0c4398a686f4181bc037c5591e87243f2",
    "achilles_18Q4_gene_effect.csv":
        "2186669de8ade17bfbf7f2bc3e67e8af59d644800bf793ef103c67a4692eb68b",
    "achilles_18Q4_sample_info.csv":
        "c5778e66e6c62c94386a39924be50f24086d5f0d5401117b065c3e6d7fbdb498",
    "wrn_sourcedata_EDFig3_MOESM6.xlsx":
        "506d7ac0f2517cb6b1e7277dfb175675044ab4b6fbc4a628f8c9ad843ba41fd6",
    "wrn_sourcedata_EDFig4_MOESM7.xlsx":
        "bba867f2778ee2ad0c7be4bdd4613e8711614da1c9592d32e90a471855178549",
    "wrn_sourcedata_EDFig10_MOESM12.xlsx":
        "3fc08ebadbc282ac7bdb4f87d73e704e79e216b611e5357302d17e4e32cdb33c",
    "wrn_sourcedata_Fig2_MOESM3.xlsx":
        "e9c006d12398d87a36801d4d2ecee56a9932c3536d3ba780f1bb6234a560c89e",
}


def _verify_sha256(fname):
    """Enforce the pin: the slice on disk must hash to the recorded value."""
    path = os.path.join(SLICES, fname)
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    got = h.hexdigest()
    if got != SHA256[fname]:
        raise SystemExit(
            f"PIN VIOLATION: {fname}\n  expected sha256 {SHA256[fname]}\n  got      sha256 {got}"
        )
    return path


def _realnum(s):
    """NaN-aware parse. The curated table uses BOTH `NA` (R default) and
    `NaN` (computed float columns); a value is present only if it parses as
    a real number (see recompute-findings.md F2)."""
    s = s.strip()
    if s == "" or s.upper() in ("NA", "NAN"):
        return None
    try:
        return float(s)
    except ValueError:
        return None


def _supp():
    return list(csv.DictReader(open(_verify_sha256("wrn_supplementary_table_1.csv"), newline="")))


def _truthy(s):
    return str(s).strip().upper() in ("TRUE", "T", "1", "YES")


# ── Recipe 1: wrn_dep_sampleset (IID, 37 MSI / 91 MSS) ───────────────────
#   slice  : wrn_supplementary_table_1.csv
#   column : avg_WRN_dep  (curated mean of CRISPR-CERES + RNAi-DEMETER2)
#   filter : common_MSI_lineage = 1 ∧ CCLE_MSI ∈ {MSI, MSS} ∧ value real
#   group  : A = MSI, B = MSS; each sorted ascending
def extract_wrn_dep():
    supp = _supp()

    def grp(label):
        vals = [
            _realnum(r["avg_WRN_dep"])
            for r in supp
            if _truthy(r["common_MSI_lineage"]) and r["CCLE_MSI"].strip() == label
        ]
        return sorted(v for v in vals if v is not None)

    return {"kind": "IID", "group_a": grp("MSI"), "group_b": grp("MSS")}


# ── Recipe 2: wrn_corr_sampleset (Paired, 51 (x,y) pairs) ────────────────
#   slice  : wrn_supplementary_table_1.csv
#   columns: x = ms_deletions_normed, y = avg_WRN_dep
#   filter : CCLE_MSI = MSI (ALL lineages) ∧ both x,y real   (n = 51, finding F1)
#   sort   : by x ascending; emitted interleaved [x0, y0, x1, y1, ...]
def extract_wrn_corr():
    supp = _supp()
    pairs = []
    for r in supp:
        if r["CCLE_MSI"].strip() != "MSI":
            continue
        x = _realnum(r["ms_deletions_normed"])
        y = _realnum(r["avg_WRN_dep"])
        if x is not None and y is not None:
            pairs.append((x, y))
    pairs.sort()
    flat = [v for p in pairs for v in p]
    return {"kind": "Paired", "flat": flat, "n_pairs": len(pairs)}


# ── Recipe 3: wrn_recq_sampleset (IID, 32 MSI / 413 MSS) ─────────────────
#   slice  : achilles_18Q4_gene_effect.csv, column "WRN (7486)" (CRISPR-CERES,
#            the Achilles screen only — NOT the curated avg)
#   join   : DepMap_ID → CCLE_name (achilles_18Q4_sample_info.csv) → CCLE_MSI (Supp T1)
#   filter : CCLE_MSI ∈ {MSI, MSS} (ALL lineages) ∧ WRN value real
#   group  : A = MSI, B = MSS; each sorted ascending
def extract_wrn_recq():
    supp = _supp()
    msi_lab = {r["CCLE_ID"].strip(): r["CCLE_MSI"].strip() for r in supp}
    d2c = {
        r["DepMap_ID"].strip(): r["CCLE_name"].strip()
        for r in csv.DictReader(open(_verify_sha256("achilles_18Q4_sample_info.csv"), newline=""))
    }
    path = _verify_sha256("achilles_18Q4_gene_effect.csv")
    with open(path, newline="") as f:
        wi = next(csv.reader(f)).index("WRN (7486)")
    groups = {"MSI": [], "MSS": []}
    with open(path, newline="") as f:
        rd = csv.reader(f)
        next(rd)
        for row in rd:
            ccle = d2c.get(row[0].strip())
            if ccle is None:
                continue
            g = msi_lab.get(ccle)
            if g not in ("MSI", "MSS"):
                continue
            v = _realnum(row[wi])
            if v is not None:
                groups[g].append(v)
    return {"kind": "IID", "group_a": sorted(groups["MSI"]), "group_b": sorted(groups["MSS"])}


# ── Recipe 4: p53_dep_sampleset (IID, 23 p53-intact / 13 p53-impaired) ───
#   slice  : wrn_supplementary_table_1.csv
#   columns: avg_WRN_dep, TP53_status
#   filter : common_MSI_lineage=1 ∧ CCLE_MSI=MSI ∧ avg_WRN_dep real;
#            group A = TP53_proficient (n=23), B = TP53_null (n=13)
#   group  : each sorted ascending (the 1 NA-TP53 line of the 37 dropped)
def extract_p53_dep():
    supp = _supp()

    def grp(status):
        vals = [
            _realnum(r["avg_WRN_dep"])
            for r in supp
            if _truthy(r["common_MSI_lineage"])
            and r["CCLE_MSI"].strip() == "MSI"
            and r["TP53_status"].strip() == status
        ]
        return sorted(v for v in vals if v is not None)

    return {"kind": "IID", "group_a": grp("TP53_proficient"), "group_b": grp("TP53_null")}


# ── Recipe 5: viab_{KM12,OVK18}_sampleset (IID nested, C-VAL competition assay) ─
#   slice  : wrn_sourcedata_EDFig3_MOESM6.xlsx (Nature Source Data, ED Fig 3b)
#   sheet  : "ED Fig 3b" → "Relative ratio" block, Day 10 (Firefly:Renilla /
#            mean-of-negatives, 6 reps per guide)
#   group A: sgWRN1,sgWRN2,sgWRN3 (flat by guide); group B: sgCh2-2,sgCh2-4
#   (pan-essential controls sgPolR2D/sgMYC excluded — "sgWRNs vs neg controls")
def _read_xlsx_cells(fname, sheet=1):
    """Minimal xlsx reader → sheet `sheet` rows as {col_idx: value}. Stdlib only."""
    import xml.etree.ElementTree as ET

    path = _verify_sha256(fname)
    z = zipfile.ZipFile(path)
    ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    sst = []
    if "xl/sharedStrings.xml" in z.namelist():
        sst = [(n.text or "") for n in ET.fromstring(z.read("xl/sharedStrings.xml")).iter(f"{{{ns}}}t")]

    def colnum(ref):
        m = re.match(r"([A-Z]+)", ref)
        s = 0
        for ch in m.group(1):
            s = s * 26 + (ord(ch) - 64)
        return s - 1

    rows = []
    for row in ET.fromstring(z.read(f"xl/worksheets/sheet{sheet}.xml")).iter(f"{{{ns}}}row"):
        cur = {}
        for c in row.findall(f"{{{ns}}}c"):
            v = c.find(f"{{{ns}}}v")
            if v is None:
                continue
            cur[colnum(c.get("r"))] = sst[int(v.text)] if c.get("t") == "s" else v.text
        rows.append(cur)
    return rows


# ── Recipe 6: cc_{KM12,SW48,OVK18}_sampleset (cell-cycle %S, ED Fig 4b) ──
#   slice  : wrn_sourcedata_EDFig4_MOESM7.xlsx, sheet "ED Fig 4b" (sheet1)
#   layout : 6-row cell-line blocks (block b at rows 6b..6b+5; values 6b+3..+5).
#            S-phase columns per guide: sgCh2-2 c3, sgWRN2 c6, sgWRN3 c9.
#   block  : KM12 b=2, SW48 b=3, OVK18 b=5.
#   group A: sgWRN2,sgWRN3 (3 reps each, flat by guide); group B: sgCh2-2 (3)
# ── Recipe 7: apop_{KM12,SW48,OVK18}_sampleset (total apoptosis, ED Fig 4c) ─
#   slice  : same file, sheet "ED Fig 4c" (sheet2)
#   layout : same 6-row blocks; total-apoptosis columns: sgCh2-2 c4, sgWRN2 c8,
#            sgWRN3 c12.  group A: sgCh2-2 (3); group B: sgWRN2,sgWRN3 (3 each)
#            (control first — WRN loss RAISES apoptosis, so mean_a < mean_b).
def _ed4_block_vals(rows, block, col):
    base = block * 6
    return [round(float(rows[base + 3 + r][col]), 6) for r in range(3)]


def _extract_cc(block):
    rows = _read_xlsx_cells("wrn_sourcedata_EDFig4_MOESM7.xlsx", sheet=1)
    wrn = _ed4_block_vals(rows, block, 6) + _ed4_block_vals(rows, block, 9)  # sgWRN2, sgWRN3
    ctl = _ed4_block_vals(rows, block, 3)  # sgCh2-2
    return {"kind": "IID", "group_a": wrn, "group_b": ctl}


def _extract_apop(block):
    rows = _read_xlsx_cells("wrn_sourcedata_EDFig4_MOESM7.xlsx", sheet=2)
    ctl = _ed4_block_vals(rows, block, 4)  # sgCh2-2 (control first)
    wrn = _ed4_block_vals(rows, block, 8) + _ed4_block_vals(rows, block, 12)  # sgWRN2, sgWRN3
    return {"kind": "IID", "group_a": ctl, "group_b": wrn}


# ── Recipe 8: mmr_{rescue,resens1,resens2}_sampleset (ED Fig 10c, n=6) ────
#   slice  : wrn_sourcedata_EDFig10_MOESM12.xlsx, sheet "ED Fig 10c" (sheet2)
#   readout: relative viability (NORMALIZED rows), shWRN1 (c5) + shWRN2 (c6),
#            6 reps each — the bars the figure's ∗†‡§ symbols mark.
#   blocks : four HCT116 derivatives, each a 16-row block (6 raw, mean,
#            mean-norm, 6 norm) — the normalized per-replicate rows are:
#            ∗ Ch2            rows 10-15 (0-indexed); † Ch3+5+sgCh2-2  rows 26-31
#            ‡ Ch3+5+sgMLH1-1 rows 43-48;             § Ch3+5+sgMLH1-2 rows 59-64
#   model  : crossed `lm(value ~ CL + guide)`; group_a = the lower-viability
#            (MMR-deficient / re-deficient) arm, group_b = the rescued arm.
_ED10C_NORM_ROWS = {
    "star": range(11, 17),  # ∗ HCT116 Ch2
    "dag": range(27, 33),  # † HCT116 Ch3+5+sgCh2-2
    "ddag": range(44, 50),  # ‡ HCT116 Ch3+5+sgMLH1-1
    "sect": range(60, 66),  # § HCT116 Ch3+5+sgMLH1-2
}


def _ed10c_block(key):
    rows = _read_xlsx_cells("wrn_sourcedata_EDFig10_MOESM12.xlsx", sheet=2)
    rr = _ED10C_NORM_ROWS[key]
    w1 = [round(float(rows[ri][5]), 6) for ri in rr]  # shWRN1
    w2 = [round(float(rows[ri][6]), 6) for ri in rr]  # shWRN2
    return w1 + w2


def _extract_mmr(group_a_key, group_b_key):
    return {"kind": "IID", "group_a": _ed10c_block(group_a_key), "group_b": _ed10c_block(group_b_key)}


# ── Recipe 9: rescue_{wt,e84a}_sampleset (Fig 2c, KM12 cDNA rescue, n=6) ──
#   slice  : wrn_sourcedata_Fig2_MOESM3.xlsx, sheet "Fig 2c" (sheet2)
#   readout: "Relative ratio" (firefly/renilla, normalized to sgCh2-2=1), rows
#            27-32 (6 biological replicates). sgWRN-EIJ arm column per cDNA
#            background: GFP c5, WRN-WT c12, WRN-E84A c19.
#   model  : two-sample t-test (IID), group A = GFP+sgWRN-EIJ (no-rescue),
#            group B = cDNA+sgWRN-EIJ (rescued). Rescue ⇒ group A below B.
def _fig2c_eij(col):
    rows = _read_xlsx_cells("wrn_sourcedata_Fig2_MOESM3.xlsx", sheet=2)
    return [round(float(rows[ri][col]), 6) for ri in range(27, 33)]


def _extract_rescue(cdna_eij_col):
    return {"kind": "IID", "group_a": _fig2c_eij(5), "group_b": _fig2c_eij(cdna_eij_col)}


def _extract_viab(first_guide_col):
    # ED Fig 3b cell-line blocks: ES2 first-guide col 2, OVK18 11, SW620 20,
    # KM12 29. Day-10 "Relative ratio" Value 1-6 are rows 92..97. Guide order:
    # sgCh2-2,sgCh2-4,sgPolR2D,sgMYC,sgWRN1,sgWRN2,sgWRN3 (offsets 0..6).
    rows = _read_xlsx_cells("wrn_sourcedata_EDFig3_MOESM6.xlsx")

    def vals(offset):
        col = first_guide_col + offset
        return [round(float(rows[ri][col]), 6) for ri in range(92, 98) if rows[ri].get(col) not in (None, "")]

    wrn = vals(4) + vals(5) + vals(6)  # sgWRN1,2,3
    ctl = vals(0) + vals(1)  # sgCh2-2, sgCh2-4
    return {"kind": "IID", "group_a": wrn, "group_b": ctl}


# resource name in the ESL → recipe
RECIPES = {
    "wrn:wrn_dep_sampleset": extract_wrn_dep,
    "wrn:wrn_corr_sampleset": extract_wrn_corr,
    "wrn:wrn_recq_sampleset": extract_wrn_recq,
    "wrn:p53_dep_sampleset": extract_p53_dep,
    "wrn:viab_KM12_sampleset": lambda: _extract_viab(29),
    "wrn:viab_OVK18_sampleset": lambda: _extract_viab(11),
    "wrn:cc_KM12_sampleset": lambda: _extract_cc(2),
    "wrn:cc_SW48_sampleset": lambda: _extract_cc(3),
    "wrn:cc_OVK18_sampleset": lambda: _extract_cc(5),
    "wrn:apop_KM12_sampleset": lambda: _extract_apop(2),
    "wrn:apop_SW48_sampleset": lambda: _extract_apop(3),
    "wrn:apop_OVK18_sampleset": lambda: _extract_apop(5),
    "wrn:mmr_rescue_sampleset": lambda: _extract_mmr("star", "dag"),
    "wrn:mmr_resens1_sampleset": lambda: _extract_mmr("ddag", "dag"),
    "wrn:mmr_resens2_sampleset": lambda: _extract_mmr("sect", "dag"),
    "wrn:rescue_wt_sampleset": lambda: _extract_rescue(12),
    "wrn:rescue_e84a_sampleset": lambda: _extract_rescue(19),
}


def _derived_flat(result):
    if result["kind"] == "Paired":
        return result["flat"]
    return result["group_a"] + result["group_b"]


def _inlined_flat(resource):
    """Parse the floats inside a resource's stats:sample_set_value(...) block."""
    text = open(ESL).read()
    start = text.index(f"resource {resource} ")
    sv = text.index("sample_set_value", start)
    end = text.index("BiologicalReplication", sv)
    return [float(x) for x in re.findall(r"-?\d+\.\d+", text[sv:end])]


def _fmt(vals, indent):
    out, row = [], []
    pad = " " * indent
    for v in vals:
        row.append(f"{v:.6f}")
        if len(row) == 8:
            out.append(pad + ", ".join(row) + ",")
            row = []
    if row:
        out.append(pad + ", ".join(row) + ",")
    return "\n".join(out)


def cmd_check():
    ok = True
    for resource, recipe in RECIPES.items():
        derived = [round(x, 6) for x in _derived_flat(recipe())]
        inlined = [round(x, 6) for x in _inlined_flat(resource)]
        same = derived == inlined
        ok = ok and same
        print(
            f"  {resource:<28} derived={len(derived):>3} inlined={len(inlined):>3}  "
            f"{'IDENTICAL' if same else 'DRIFT ✗'}"
        )
    if not ok:
        raise SystemExit("FAIL: inlined SampleSet array(s) drifted from the pinned slices")
    print("OK: all inlined SampleSets reproduce exactly from the pinned slices")


def cmd_emit():
    for resource, recipe in RECIPES.items():
        r = recipe()
        print(f"// {resource}")
        if r["kind"] == "Paired":
            print(_fmt(r["flat"], 12))
        else:
            print(f"// group_a (n={len(r['group_a'])}):")
            print(_fmt(r["group_a"], 12))
            print(f"// group_b (n={len(r['group_b'])}):")
            print(_fmt(r["group_b"], 12))
        print()


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "--check"
    if mode == "--emit":
        cmd_emit()
    elif mode == "--check":
        cmd_check()
    else:
        raise SystemExit(f"usage: {sys.argv[0]} [--check|--emit]")
