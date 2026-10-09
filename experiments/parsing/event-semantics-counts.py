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

"""Re-derive the counts in `docs/notes/event-semantics.md` (eigenius#270).

A PP that the pre-#270 verb-adjunct encoding hangs on the subject, `And(V(…, s), prep_P(s, o))`,
changes meaning under either answer to #270 — the event design this note costs, and the predicate
modifiers adopted on `2026-10-07` (`docs/notes/event-free-modification.md`). This script finds them
in the printed terms, and reads both encodings:

- **Since `2026-10-07`** an adjunct is `adv_P(object, V, subject)` and SAYS SO IN ITS NAME, so it is
  clause-level by definition and needs no anchor test. Its anchor is its LAST argument.
- **Artifacts recorded before that change** carry `prep_P(anchor, object)`, one relation serving both
  the adjunct and the noun-postmodifier role, so they are classified by their anchor as below. The
  ledger and the pins still hold these until they are regenerated on a reseed.

A pre-#270 `prep_P(anchor, object)` application is classified by its anchor and its host:

- NOUN-INTERNAL when the anchor is a bare `G#k` whose nearest binder is a Σ (a noun postmodifier,
  or a relative clause on the Σ variable). These are skipped: events leave them alone.
- Otherwise CLAUSE-LEVEL. Its host is the conjunct beside it in the innermost `And` whose second
  argument it is: a VERB host when that conjunct contains a verb atom `v<offset>_<frame>(`, an
  ADJECTIVE OR COPULA host otherwise.

One row the ANCHOR classifier cannot see: «These libraries define genes that were essential for
proliferation and survival.» hangs «for» on the copular VP inside a relative clause, so its anchor is
Σ-bound like a noun postmodifier. The note counts it by hand («1 inside a relative clause»). This
blind spot is a property of the pre-#270 encoding, not of the script: an `adv_for` is classified by
its name whatever its anchor, so regenerated artifacts do not need the hand count. The
candidates are listed below; «These lines possess events that are predictive of …» is not counted,
because «of» attaches at the `S[adj]` level (the closed class's adjective-complement «of»), which
the term does not show.

The page-sentence counts in the note (15 of 62 sentences end in an «in» or «with» PP run; about 45
end in some PP run) are hand counts listed there, not computed here.

    python3 experiments/parsing/event-semantics-counts.py
"""

import collections
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
LEDGER = os.path.join(ROOT, "experiments/parsing/reading-adjudications.tsv")
PINS = os.path.join(ROOT, "experiments/parsing/expected-readings.tsv")
VERB_SENSES = os.path.join(ROOT, "experiments/lexicon-specialist/verb-senses.tsv")
TRACKED_DRAW = os.path.join(ROOT, "experiments/parsing/selections/2026-10-05-hpo-merge-1.json")
CLOSED_CLASS = os.path.join(ROOT, "ontologies/lexicon/closed-class.esl")

VERB_ATOM = re.compile(r"\bv(\d{8})_(\w+?)\(")
PREP = re.compile(r"\bprep_(\w+)\(")
# The adverbial family (#270, 2026-10-07): `adv_P(object, V, subject)`, clause-level by name.
ADJUNCT = re.compile(r"\badv_(\w+)\(")
PP_ARGUMENT_RELATION = re.compile(r"\bv\d{8}_p(_[a-z]+)?\(")
SUBJECT_GROUND = re.compile(r"of the subject|subject predication")


def read_tsv(path):
    rows = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            rows.append(line.rstrip("\n").split("\t"))
    return rows


def args_at(s, i):
    """Top-level arguments of the application whose `(` is at `s[i]`: (texts, start offsets, end)."""
    depth, cur, start, texts, starts = 0, [], None, [], []
    for j in range(i, len(s)):
        ch = s[j]
        if ch == "(":
            depth += 1
            if depth == 1:
                start = j + 1
                continue
        elif ch == ")":
            depth -= 1
            if depth == 0:
                texts.append("".join(cur).strip())
                starts.append(start + len("".join(cur)) - len("".join(cur).lstrip()))
                return texts, starts, j
        elif ch == "," and depth == 1:
            texts.append("".join(cur).strip())
            starts.append(start + len("".join(cur)) - len("".join(cur).lstrip()))
            cur, start = [], j + 1
            continue
        cur.append(ch)
    raise ValueError(f"unbalanced term at {i}: {s[:80]}")


def host_of(sem, pstart):
    """The first argument of the innermost two-argument `And` whose second argument starts at `pstart`."""
    host = None
    for m in re.finditer(r"\bAnd\(", sem):
        texts, starts, end = args_at(sem, m.end() - 1)
        if len(texts) == 2 and starts[1] == pstart and m.start() < pstart <= end:
            host = texts[0]
    return host


def binder_of(sem, pos, var):
    """The binder (Σ, Π or λ) nearest before `pos` that binds `var`."""
    kind = None
    for m in re.finditer(r"([ΣΠλ])" + re.escape(var) + r"\s*:", sem[:pos]):
        kind = m.group(1)
    return kind


def sigma_body(sem, pos, var):
    """The text of the Σ that binds `var` nearest before `pos`, from its binder to the end of its group."""
    binders = [m.start() for m in re.finditer("Σ" + re.escape(var) + r"\s*:", sem[:pos])]
    if not binders:
        return ""
    b, depth = binders[-1], 0
    for o in range(b - 1, -1, -1):
        if sem[o] == ")":
            depth += 1
        elif sem[o] == "(":
            if depth == 0:
                _, _, end = args_at(sem, o)
                return sem[b:end]
            depth -= 1
    return sem[b:]


def clause_level_pps(sem):
    """Each clause-level adjunct in `sem`: (preposition, anchor, host conjunct or None).

    An `adv_P(object, V, subject)` is clause-level by name, its anchor the LAST argument. A
    `prep_P(anchor, object)` is pre-#270 and is classified by its anchor.
    """
    for m in ADJUNCT.finditer(sem):
        texts, _, _ = args_at(sem, m.end() - 1)
        yield m.group(1), (texts[-1] if texts else ""), host_of(sem, m.start())
    for m in PREP.finditer(sem):
        texts, _, _ = args_at(sem, m.end() - 1)
        anchor = texts[0] if texts else ""
        bare = re.fullmatch(r"G#\d+", anchor)
        if bare and binder_of(sem, m.start(), anchor) == "Σ":
            continue
        yield m.group(1), anchor, host_of(sem, m.start())


def governed_placements(path):
    """(offset, preposition) pairs the SPECIALIST judge placed on a verb sense (D97)."""
    placed = set()
    for row in read_tsv(path):
        if len(row) >= 3 and row[2] != "-":
            for off in row[2].split(","):
                placed.add((off.strip().zfill(8), row[1]))
    return placed


def main():
    ledger = read_tsv(LEDGER)
    pins = read_tsv(PINS)
    placed = governed_placements(VERB_SENSES)
    out = sys.stdout

    # ── Ledger: rows whose PP the change moves ─────────────────────────────────────────────────
    verb_rows = [r for r in ledger if VERB_ATOM.search(r[1])]
    cells = collections.Counter()
    subject_grounds = 0
    pp_rows = set()
    for i, r in enumerate(ledger):
        hosts = [("verb" if host and VERB_ATOM.search(host) else "adjective or copula")
                 for _, _, host in clause_level_pps(r[1])]
        if not hosts:
            continue
        pp_rows.add(i)
        host = "verb" if "verb" in hosts else "adjective or copula"
        cells[(host, "verb row" if VERB_ATOM.search(r[1]) else "verbless row", r[2])] += 1
        if host == "verb" and r[2] == "wrong" and SUBJECT_GROUND.search(r[3] if len(r) > 3 else ""):
            subject_grounds += 1

    verbless = len(ledger) - len(verb_rows)
    on_verb = sum(n for k, n in cells.items() if k[0] == "verb")
    on_adj = sum(n for k, n in cells.items() if k[0] == "adjective or copula")
    adj_verbless = sum(n for k, n in cells.items() if k[0] != "verb" and k[1] == "verbless row")
    print(f"ledger rows: {len(ledger)} ({len(verb_rows)} carry a verb atom, {verbless} do not)", file=out)
    print(f"rows with a clause-level PP: {len(pp_rows)}", file=out)
    print(f"  on a verb: {on_verb}  "
          f"(correct {sum(n for k, n in cells.items() if k[0] == 'verb' and k[2] == 'correct')}, "
          f"wrong {sum(n for k, n in cells.items() if k[0] == 'verb' and k[2] == 'wrong')}; "
          f"{subject_grounds} wrong rows grounded on subject predication)", file=out)
    print(f"  on an adjective or copula: {on_adj}  "
          f"(correct {sum(n for k, n in cells.items() if k[0] != 'verb' and k[2] == 'correct')}, "
          f"wrong {sum(n for k, n in cells.items() if k[0] != 'verb' and k[2] == 'wrong')}; "
          f"{adj_verbless} in verbless rows)", file=out)
    mechanical = len(verb_rows) - sum(1 for i in pp_rows if VERB_ATOM.search(ledger[i][1]))
    print(f"verb rows without such a PP: {mechanical}; less the relative-clause row counted by hand "
          f"(below): {mechanical - 1}; less U1's 3 `correct` noun-attachment rows, which open "
          f"question 5 turns `wrong`: {mechanical - 4} carried over mechanically", file=out)
    print(f"verbless rows without such a PP (unchanged): {verbless - adj_verbless}", file=out)

    print("\nrelative-clause candidates (Σ-anchored PP beside an adjective on the same variable, "
          "in a sentence with «that is/are/was/were»):", file=out)
    for r in ledger:
        if not re.search(r"\bthat (is|are|was|were)\b", r[0]):
            continue
        for m in PREP.finditer(r[1]):
            texts, _, _ = args_at(r[1], m.end() - 1)
            anchor = texts[0] if texts else ""
            if re.fullmatch(r"G#\d+", anchor) and binder_of(r[1], m.start(), anchor) == "Σ" \
                    and re.search(r"gt\(deg_a\d+\(" + re.escape(anchor) + r"\)",
                                  sigma_body(r[1], m.start(), anchor)):
                print(f"  {r[2]:7} prep_{m.group(1)} | {r[0]}", file=out)
                break

    # ── Governed prepositions hung on their own verb as a free adjunct (open question 4) ──────
    governed = collections.Counter()
    governed_sentences = set()
    for r in ledger:
        for prep, _, host in clause_level_pps(r[1]):
            if host and any((off, prep) in placed for off, _ in VERB_ATOM.findall(host)):
                basis = r[4] if len(r) > 4 else ""
                governed[(r[2], "structure only" if basis.strip() == "structure" else basis)] += 1
                governed_sentences.add(r[0])
                break
    print(f"\nfree adjuncts of a governed preposition: {sum(governed.values())} rows, "
          f"{len(governed_sentences)} sentences", file=out)
    for (verdict, basis), n in sorted(governed.items()):
        print(f"  {verdict}, basis {basis!r}: {n}", file=out)

    # ── The tracked draw's choices on those sentences ────────────────────────────────────────────
    ledger_verdict = {(r[0], r[1]): r[2] for r in ledger}
    draw = json.load(open(TRACKED_DRAW, encoding="utf-8"))
    decided = [e for e in draw if not e.get("abstained")]
    correct = sum(1 for e in decided
                  if ledger_verdict.get((e["sentence"], e["candidates"][e["chosen"]]["sem"])) == "correct")
    adjunct_picks = 0
    for e in decided:
        if e["sentence"] not in governed_sentences:
            continue
        sem = e["candidates"][e["chosen"]]["sem"]
        if any(host and any((off, prep) in placed for off, _ in VERB_ATOM.findall(host))
               for prep, _, host in clause_level_pps(sem)):
            adjunct_picks += 1
    print(f"tracked draw ({os.path.basename(TRACKED_DRAW)}): {correct}/{len(decided)} correct; "
          f"{adjunct_picks} of its picks hang a governed preposition as a free adjunct", file=out)

    # ── Pins ─────────────────────────────────────────────────────────────────────────────────────
    by_sentence = collections.defaultdict(list)
    for r in ledger:
        by_sentence[r[0]].append(r)
    verb_pins = [p for p in pins if any(VERB_ATOM.search(r[1]) for r in by_sentence.get(p[0], []))]
    pp_pins = [p for p in pins if any(True for _ in clause_level_pps(p[1]))]
    named, generic = [], []
    for p in pins:
        rels = {m.group(0) for r in by_sentence.get(p[0], []) if r[2] == "correct"
                for m in PP_ARGUMENT_RELATION.finditer(r[1])}
        if any("_p_" in rel for rel in rels):
            named.append(p[0])
        elif rels:
            generic.append(p[0])
    print(f"\npins: {len(pins)}; pinned sentences whose ledger rows carry a verb atom: {len(verb_pins)}; "
          f"without ledger rows: {sum(1 for p in pins if p[0] not in by_sentence)}", file=out)
    print(f"pins with a clause-level PP: {len(pp_pins)} (the note adds the 2 whose PP sits on a "
          f"relative clause's predicate, for 15 adjunct pins)", file=out)
    print(f"pinned sentences with a correct reading on a PP-argument relation: {len(named) + len(generic)} "
          f"({len(named)} named `_p_<prep>`, {len(generic)} on the any-preposition frame `_p`)", file=out)

    # ── The lexicon side of slice 2 ─────────────────────────────────────────────────────────────
    closed = open(CLOSED_CLASS, encoding="utf-8").read()
    entries = re.findall(r"resource (lexicon:\S+) : lexicon:LexicalEntry \{(.*?)\n\}", closed, re.S)
    cat_s_entries = sum(1 for _, body in entries if re.search(r"lexicon:cat\s*=.*lexicon:cat_s\(", body))
    vp_adjunct = re.compile(
        r"lexicon:fwd\(lexicon:m_\w+, lexicon:bwd\(lexicon:m_\w+, lexicon:bwd\(lexicon:m_\w+, "
        r"lexicon:cat_s\(lexicon:dcl, lexicon:fin\), lexicon:cat_np\([^)]*\)\), lexicon:bwd\(lexicon:m_\w+, "
        r"lexicon:cat_s\(lexicon:dcl, lexicon:fin\), lexicon:cat_np\([^)]*\)\)\), lexicon:cat_np\(")
    vp_adjunct_forms = sorted({re.search(r'lexicon:form\s*=\s*"([^"]*)"', body).group(1)
                               for _, body in entries if vp_adjunct.search(body)})
    print(f"\nclosed class: {closed.count('lexicon:cat_s(')} `cat_s` occurrences in {cat_s_entries} entries", file=out)
    print(f"prepositions with a finite VP-adjunct entry over an NP: {len(vp_adjunct_forms)} "
          f"({', '.join(vp_adjunct_forms)})", file=out)


if __name__ == "__main__":
    main()
