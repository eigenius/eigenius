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
"""What the SPECIALIST Lexicon would change in the imported lexicon (D97).

Joins three inputs by lemma and category:
  - the entries the WordNet importer emitted (`wordnet-chain/*.esl`, written by the reseed), which
    record each verb sense's frame tag (`v00254150_i`) and each adjective's governed preposition
    (`cat_pp_arg(lexicon:prep_on)`);
  - the SPECIALIST Lexicon (`references/specialist/LEXICON`, NLM, 2026 release);
  - the Wiktionary countability list (`references/wiktionary/uncountable-nouns.txt`).
and reports, over the whole lexicon and over the WRN texts plus the quantity corpus:
  - verbs SPECIALIST makes transitive that WordNet has only without an object;
  - verbs SPECIALIST has that WordNet lacks;
  - the preposition SPECIALIST names where WordNet's PP frame names none (`prep_any`);
  - object + PP frames (`ditran=np,pphr(p,np)`);
  - adjectives' governed prepositions, SPECIALIST against the importer;
  - countability, SPECIALIST against Wiktionary;
  - the entries a lemma-level union would add (one per existing sense x form).

Usage: experiments/lexicon-specialist/measure-specialist.py [--list]

Inputs, all gitignored: `wordnet-chain/` from a reseed; the Wiktionary list
(`scripts/provision-countability.sh`); and SPECIALIST, until D97 slice 1 scripts it:
  mkdir -p references/specialist && curl -o references/specialist/LEXICON \
    https://data.lhncbc.nlm.nih.gov/public/lsg/lexicon/2026/release/LEX_DOC/LEXICON
"""

import collections
import glob
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SPECIALIST = os.path.join(ROOT, "references/specialist/LEXICON")
WORDNET_CHAIN = os.path.join(ROOT, "wordnet-chain")
WIKTIONARY = os.path.join(ROOT, "references/wiktionary/uncountable-nouns.txt")
TEXTS = [
    os.path.join(ROOT, "references/publications/WRN-Helicase-Nature-OCR/methods.txt"),
    os.path.join(ROOT, "references/publications/WRN-Helicase-Nature-OCR/letter-body.txt"),
]
CORPUS = os.path.join(ROOT, "experiments/parsing/quantities/corpus.tsv")

# The prepositions `lexicon:Prep` names (less `prep_any`).
PREP_ENUM = {"to", "on", "in", "with", "from", "for", "at", "upon", "about", "against", "into", "of", "as"}
# Frame tags that give the verb an NP object (`FrameKind` in the WordNet importer).
OBJECT_TAGS = {"t", "d", "as"}
# The closed class's verbs, which no content importer emits.
CLOSED_VERBS = {"be", "have", "do"}


def read_specialist():
    """(base, cat) -> record; every spelling variant indexes the same record."""
    by_key = collections.defaultdict(list)
    text = open(SPECIALIST, encoding="utf-8").read()
    for block in text.split("{base=")[1:]:
        lines = block.split("\n")
        rec = {"base": lines[0].strip(), "spellings": [], "fields": collections.defaultdict(list)}
        for ln in lines[1:]:
            ln = ln.strip()
            if not ln or ln == "}":
                continue
            key, _, val = ln.partition("=")
            if key == "spelling_variant":
                rec["spellings"].append(val)
            else:
                rec["fields"][key].append(val)
        cat = rec["fields"].get("cat", [""])[0]
        for s in [rec["base"]] + rec["spellings"]:
            by_key[(s.lower(), cat)].append(rec)
    return by_key


def past_participles(rec):
    """The past participles of a SPECIALIST verb, from its variant codes."""
    base = rec["base"].lower()
    forms = set()
    for v in rec["fields"].get("variants", []):
        if v.startswith("irreg|"):
            parts = v.split("|")
            if len(parts) > 4 and parts[4]:
                forms.add(parts[4].lower())
        elif v == "regd":
            forms.add(base + base[-1] + "ed")
        elif v == "reg":
            if base.endswith("e"):
                forms.add(base + "d")
            elif base.endswith("y") and len(base) > 1 and base[-2] not in "aeiou":
                forms.add(base[:-1] + "ied")
            else:
                forms.add(base + "ed")
    return forms


def complements(rec):
    """SPECIALIST's complementation: object?, the governed prepositions, object+PP prepositions."""
    f = rec["fields"]
    comps = f.get("tran", []) + f.get("ditran", []) + f.get("cplxtran", [])
    has_object = any(c == "np" or c.startswith("np,") for c in comps)
    pp = {m for c in f.get("tran", []) for m in re.findall(r"^pphr\(([a-z ]+),", c)}
    obj_pp = {m for c in f.get("ditran", []) for m in re.findall(r"^np,pphr\(([a-z ]+),", c)}
    compl = {m for c in f.get("compl", []) for m in re.findall(r"^pphr\(([a-z ]+),", c)}
    return has_object, pp, obj_pp, compl


ENTRY = re.compile(
    r'lexicon:form\s*=\s*"([^"]*)";\s*lexicon:cat\s*=\s*type_expr\((.*?)\);\s*'
    r"lexicon:sem\s*=\s*([^;]+);.*?lexicon:sense\s*=\s*\"wn:([^\"]+)\"",
    re.S,
)


def read_wordnet():
    """Per (lemma, pos): senses, the frame tags per sense, emitted entries per tag, governed preps;
    and form -> {(lemma, pos)}."""
    verbs = collections.defaultdict(lambda: {"senses": set(), "tags": collections.Counter(), "sense_tags": collections.defaultdict(set)})
    adj_preps = collections.defaultdict(set)
    adjs = set()
    nouns = set()
    forms = collections.defaultdict(set)
    participles = set()
    for path in sorted(glob.glob(os.path.join(WORDNET_CHAIN, "*.esl"))):
        text = open(path, encoding="utf-8").read()
        for form, cat, sem, sense in ENTRY.findall(text):
            lemma, pos, offset = sense.rsplit(".", 2)
            lemma = lemma.replace("_", " ").lower()
            pos = {"s": "a"}.get(pos, pos)
            forms[form.lower()].add((lemma, pos))
            if pos == "v" and "lexicon:pss)" in cat:
                participles.add(form.lower())
            if pos == "v":
                tag = sem.strip().rsplit("_", 1)[-1]
                v = verbs[lemma]
                v["senses"].add(offset)
                v["tags"][tag] += 1
                v["sense_tags"][offset].add(tag)
            elif pos == "a":
                adjs.add(lemma)
                for p in re.findall(r"cat_pp_arg\(lexicon:prep_([a-z]+)\)", cat):
                    adj_preps[lemma].add(p)
            elif pos == "n":
                nouns.add(lemma)
    return verbs, adjs, adj_preps, nouns, forms, participles


def pct(n, d):
    return f"{n} ({100.0 * n / d:.1f}%)" if d else str(n)


def main():
    listing = "--list" in sys.argv
    spec = read_specialist()
    verbs, adjs, adj_preps, nouns, wn_forms, wn_participles = read_wordnet()

    # One record per verb, named by its base; its spelling variants join it to WordNet.
    spec_verbs = {}
    for (key, cat), rs in spec.items():
        if cat != "verb":
            continue
        for r in rs:
            spec_verbs.setdefault(r["base"].lower(), r)
    spec_adjs = {k[0]: r for k, rs in spec.items() if k[1] == "adj" for r in rs}

    def wordnet_lemma(rec):
        """The WordNet verb lemma a SPECIALIST verb is, by base or spelling variant."""
        for s in [rec["base"]] + rec["spellings"]:
            if s.lower() in verbs:
                return s.lower()
        return None

    # ── Verbs, whole lexicon ─────────────────────────────────────────────────────
    gain_object, new_verbs, pp_named, obj_pp_named = {}, {}, {}, {}
    for lemma, rec in spec_verbs.items():
        has_object, pp, obj_pp, _ = complements(rec)
        wn = wordnet_lemma(rec)
        if wn is None:
            if (has_object or pp) and lemma not in CLOSED_VERBS:
                new_verbs[lemma] = rec
            continue
        v = verbs[wn]
        if has_object and not (set(v["tags"]) & OBJECT_TAGS):
            # One transitive entry per existing sense x form: mirror the intransitive ones.
            gain_object[lemma] = v["tags"].get("i", 0) + v["tags"].get("p", 0)
        if pp and "p" in v["tags"]:
            pp_named[lemma] = pp
        if obj_pp:
            obj_pp_named[lemma] = obj_pp
    print("== verbs (whole lexicon) ==")
    joined = sum(1 for r in spec_verbs.values() if wordnet_lemma(r))
    print(f"SPECIALIST verb records: {len(spec_verbs)}; WordNet verb lemmas: {len(verbs)}; joined: {joined}")
    print(f"WordNet verbs SPECIALIST makes transitive (no object-taking sense in WordNet): {len(gain_object)}, "
          f"adding {sum(gain_object.values())} entries (one per existing sense x form)")
    print(f"SPECIALIST verbs with an object or a PP that WordNet lacks: {len(new_verbs)}")
    print(f"WordNet PP-oblique verbs (prep_any) SPECIALIST names a preposition for: {len(pp_named)}")
    print(f"verbs with an object + PP frame in SPECIALIST (ditran=np,pphr): {len(obj_pp_named)}")
    all_preps = collections.Counter(p for ps in list(pp_named.values()) + list(obj_pp_named.values()) for p in ps)
    outside = {p: n for p, n in all_preps.items() if p not in PREP_ENUM}
    print(f"  prepositions named: {len(all_preps)}; outside lexicon:Prep: {sorted(outside.items(), key=lambda x: -x[1])[:12]}")

    # ── Adjectives, whole lexicon ────────────────────────────────────────────────
    agree = adds = wn_only = 0
    spec_governed = 0
    for lemma in adjs:
        rec = spec_adjs.get(lemma)
        s = complements(rec)[3] if rec else set()
        w = adj_preps.get(lemma, set())
        if s:
            spec_governed += 1
        if s & w:
            agree += 1
        if s - w:
            adds += 1
        if w - s:
            wn_only += 1
    print("\n== adjectives (whole lexicon) ==")
    print(f"WordNet adjective lemmas: {len(adjs)}; with a governed preposition from the importer: {len(adj_preps)}; "
          f"with compl=pphr in SPECIALIST: {spec_governed}")
    print(f"lemmas where the two agree on a preposition: {agree}; SPECIALIST adds one: {adds}; "
          f"the importer has one SPECIALIST does not: {wn_only}")

    # ── Countability ─────────────────────────────────────────────────────────────
    wikt = {l.strip().lower() for l in open(WIKTIONARY, encoding="utf-8") if l.strip() and not l.startswith("#")}
    spec_mass = {k[0] for k, rs in spec.items() if k[1] == "noun"
                 for r in rs if any("uncount" in v for v in r["fields"].get("variants", []))}
    spec_mass_wn = spec_mass & nouns
    print("\n== countability (WordNet noun lemmas) ==")
    print(f"Wiktionary uncountable: {len(wikt)}; SPECIALIST uncount: {len(spec_mass_wn)}; both: {len(wikt & spec_mass_wn)}; "
          f"SPECIALIST only: {len(spec_mass_wn - wikt)}; Wiktionary only: {len(wikt - spec_mass_wn)}")

    # ── The WRN texts and the quantity corpus ────────────────────────────────────
    spec_participle = collections.defaultdict(set)
    for lemma, rec in spec_verbs.items():
        for f in past_participles(rec):
            spec_participle[f].add(lemma)
    text = " ".join(open(p, encoding="utf-8").read() for p in TEXTS if os.path.exists(p))
    text += " " + " ".join(l.split("\t")[0] for l in open(CORPUS, encoding="utf-8") if not l.startswith("#"))
    tokens = re.findall(r"[A-Za-z][A-Za-z'-]*[A-Za-z]|[A-Za-z]", text)
    # Lowercase tokens only: a capitalised one is a heading, a name or `Extended Data`.
    occ = collections.Counter(t for t in tokens if t.islower())

    rows = collections.defaultdict(lambda: collections.Counter())
    for form, n in occ.items():
        # Verbs are counted at their past participles only — the methods register's passives. A
        # base or third-person form is as often a noun (`volume`, `vectors`), which only a parse
        # tells apart, so the verb counts are a lower bound.
        lemmas = set()
        if form in wn_participles:
            lemmas |= {l for l, p in wn_forms.get(form, ()) if p == "v"}
        lemmas |= spec_participle.get(form, set())
        for lemma in lemmas:
            if lemma in gain_object:
                rows["gains an object"][(lemma, gain_object[lemma])] += n
            if lemma in new_verbs:
                rows["a verb WordNet lacks"][(lemma, 0)] += n
            if lemma in pp_named:
                rows["names its PP"][(lemma, ",".join(sorted(pp_named[lemma])))] += n
            if lemma in obj_pp_named:
                rows["object + PP"][(lemma, ",".join(sorted(obj_pp_named[lemma])))] += n
        for lemma in {l for l, p in wn_forms.get(form, ()) if p == "a"} | ({form} & set(spec_adjs)):
            rec = spec_adjs.get(lemma)
            s = complements(rec)[3] if rec else set()
            w = adj_preps.get(lemma, set())
            if s - w:
                rows["adjective: SPECIALIST adds a preposition"][(lemma, ",".join(sorted(s - w)))] += n
            if w - s:
                rows["adjective: the importer's preposition is not SPECIALIST's"][(lemma, ",".join(sorted(w - s)))] += n
    print(f"\n== WRN methods + letter + quantity corpus: {len(tokens)} word tokens; lowercase: {sum(occ.values())} tokens, {len(occ)} types ==")
    print("   (verbs at their past participles only)")
    for name in ["gains an object", "a verb WordNet lacks", "names its PP", "object + PP",
                 "adjective: SPECIALIST adds a preposition", "adjective: the importer's preposition is not SPECIALIST's"]:
        r = rows.get(name, {})
        print(f"{name}: {len(r)} lemmas, {sum(r.values())} occurrences")
        if listing or name in ("gains an object", "a verb WordNet lacks"):
            for (lemma, extra), n in sorted(r.items(), key=lambda x: (-x[1], x[0])):
                print(f"    {lemma:<22} {n:>3}  {extra}")


if __name__ == "__main__":
    main()
