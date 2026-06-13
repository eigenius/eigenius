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


# resource name in the ESL → recipe
RECIPES = {
    "wrn:wrn_dep_sampleset": extract_wrn_dep,
    "wrn:wrn_corr_sampleset": extract_wrn_corr,
    "wrn:wrn_recq_sampleset": extract_wrn_recq,
    "wrn:p53_dep_sampleset": extract_p53_dep,
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
