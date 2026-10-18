#!/usr/bin/env python3
"""D79 §2.1 — every property whose `class_types` names a `core:InductiveType`,
grouped by its declared `core:data_type`.

The inventory and P2's gate share this one source deliberately: an earlier scan
resolved ESL qualified names by local-name suffix and reported
`objective:options -> core:Option`, which is `objective:Option`, a *class*. Full-IRI
resolution only.

Exit 1 if any such property is still declared `core:resource` / `core:resource_array`
— that is P2's gate, so the next one added is caught rather than sampled for.
"""
import re, glob, os, json, sys, collections

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FILES = [p for p in glob.glob(os.path.join(ROOT, 'ontologies/**/*'), recursive=True)
         if p.endswith(('.json', '.esl')) and os.path.isfile(p)]

def json_resources(text):
    try:
        docs = json.loads(text)
    except Exception:
        return []
    if isinstance(docs, dict):
        docs = docs.get('resources', docs.get('@graph', []))
    return docs if isinstance(docs, list) else []

def inductive_iris():
    out = set()
    for p in FILES:
        t = open(p, errors='replace').read()
        if p.endswith('.esl'):
            for m in re.finditer(r'^\s*data\s+([a-zA-Z_]\w*):(\w+)', t, re.M):
                out.add(f"urn:eigenius:{m.group(1)}:{m.group(2)}")
        else:
            for r in json_resources(t):
                if isinstance(r, dict) and 'InductiveType' in str(r.get('urn:eigenius:core:is_a', '')):
                    out.add(r.get('@id', ''))
    return out

def qualify(name):
    return name if name.startswith('urn:') else 'urn:eigenius:' + name

def scan():
    ind = inductive_iris()
    rows = []
    for p in FILES:
        t = open(p, errors='replace').read()
        rel = os.path.relpath(p, ROOT)
        if p.endswith('.json'):
            for r in json_resources(t):
                if not isinstance(r, dict):
                    continue
                ct = r.get('urn:eigenius:core:class_types') or []
                ct = [ct] if isinstance(ct, str) else ct
                hit = [c for c in ct if c in ind]
                if hit:
                    dt = (r.get('urn:eigenius:core:data_type') or '?').split(':')[-1]
                    rows.append((r.get('@id'), dt, hit[0], rel))
        else:
            for m in re.finditer(r'^property\s+([\w]+):(\w+)\s*:\s*([\w:]+)\s*\{(.*?)^\}', t, re.M | re.S):
                cm = re.search(r'class_types\s+([\w:,\s]+);', m.group(4))
                if not cm:
                    continue
                for nm in (x.strip() for x in cm.group(1).split(',') if x.strip()):
                    if qualify(nm) in ind:
                        rows.append((f"urn:eigenius:{m.group(1)}:{m.group(2)}",
                                     m.group(3).split(':')[-1], qualify(nm), rel))
    return ind, rows

def main():
    ind, rows = scan()
    by_dt = collections.defaultdict(list)
    for iri, dt, ct, rel in rows:
        by_dt[dt].append((iri, ct, rel))
    print(f"InductiveType declarations: {len(ind)}")
    for dt in sorted(by_dt):
        print(f"\ndata_type = core:{dt}  ({len(by_dt[dt])})")
        for iri, ct, rel in sorted(by_dt[dt]):
            print(f"    {iri.split('eigenius:')[-1]:46s} -> {ct.split('eigenius:')[-1]:34s} {rel}")
    bad = [r for dt in ('resource', 'resource_array') for r in by_dt.get(dt, [])]
    if bad:
        print(f"\nFAIL: {len(bad)} propert{'y' if len(bad)==1 else 'ies'} ranged at an "
              f"InductiveType still declared core:resource(_array). D79 §2.1: a term-valued "
              f"property declared core:resource is accepted by the indexer and by Rule 22(b), "
              f"and both then do nothing.")
        return 1
    print("\nOK: every InductiveType-ranged property is declared core:inductive.")
    return 0

if __name__ == '__main__':
    sys.exit(main())
