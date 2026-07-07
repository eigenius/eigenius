#!/usr/bin/env bash

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

# Provision the COUNTABILITY LEXICON for the DCG engine (D62 bare-mass arguments).
#
# Why: a bare *plural* common noun is a felicitous NP argument in the grammar ("genes affect
# cells"), but a bare *singular* one is not — UNLESS the noun is uncountable/mass ("lethality
# matters", "mutation occurs"). Countability is NOT morphologically derivable (`mutation` and
# `function` share a suffix yet differ), so the WordNet importer reads an external list of
# uncountable noun lemmas and emits an additive `cat_n(C, mass)` entry for each (see
# crates/eigenius-wordnet/src/convert.rs, `--countability`).
#
# Source: the English Wiktionary `Category:English uncountable nouns` (CC-BY-SA), intersected
# with the WordNet 3.0 noun lemmas (so the 288k-member category's proper-noun/symbol noise is
# stripped to the ~32k lemmas the importer can actually mark). The intersection is a list of
# facts (lemmas); the derived file is gitignored (under /references) and rebuilt on demand.
#
# Usage:
#   scripts/provision-countability.sh                 # fetch + intersect → the default OUT
#
# Env overrides:
#   DICT   WordNet dict dir (default: references/WordNet-3.0/dict — run provision-wordnet.sh first)
#   OUT    output list path (default: references/wiktionary/uncountable-nouns.txt, gitignored)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DICT="${DICT:-references/WordNet-3.0/dict}"
OUT="${OUT:-references/wiktionary/uncountable-nouns.txt}"

if [[ ! -f "$DICT/index.noun" ]]; then
  echo "error: WordNet noun index not found at $DICT/index.noun" >&2
  echo "  run scripts/provision-wordnet.sh first (WordNet is provisioned, not vendored)." >&2
  exit 2
fi

mkdir -p "$(dirname "$OUT")"

echo "provision-countability: fetching Wiktionary 'English uncountable nouns' ∩ WordNet → $OUT" >&2

python3 - "$DICT/index.noun" "$OUT" << 'PY'
import sys, json, time, urllib.request, urllib.parse

index_noun, out_path = sys.argv[1], sys.argv[2]
API = "https://en.wiktionary.org/w/api.php"
UA = "eigenius-countability/1.0 (research; https://github.com/eigenius/eigenius)"

def api(params):
    params = dict(params); params["format"] = "json"
    url = API + "?" + urllib.parse.urlencode(params)
    for attempt in range(5):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=30) as r:
                return json.load(r)
        except Exception:
            time.sleep(1.5 * (attempt + 1))
    raise SystemExit(f"error: Wiktionary API failed: {url}")

# 1. WordNet noun lemmas, normalized (lowercase, '_' → space).
wn = set()
with open(index_noun) as f:
    for line in f:
        if line.startswith(" "):
            continue  # license header
        parts = line.split()
        if len(parts) >= 4:
            wn.add(parts[0].lower().replace("_", " "))
print(f"  WordNet noun lemmas: {len(wn)}", file=sys.stderr)

# 2. Stream the uncountable category, keeping only members that are WordNet lemmas.
inter, cont, pages, seen = set(), None, 0, 0
while True:
    params = {"action": "query", "list": "categorymembers",
              "cmtitle": "Category:English uncountable nouns",
              "cmlimit": "500", "cmnamespace": "0"}
    if cont:
        params["cmcontinue"] = cont
    d = api(params)
    for m in d["query"]["categorymembers"]:
        seen += 1
        t = m["title"].lower()
        if t in wn:
            inter.add(t)
    pages += 1
    if pages % 50 == 0:
        print(f"  {pages} pages, {seen} members seen, {len(inter)} matched", file=sys.stderr)
    cont = d.get("continue", {}).get("cmcontinue")
    if not cont:
        break

with open(out_path, "w") as o:
    o.write("# Uncountable (mass) noun lemmas: Wiktionary 'Category:English uncountable nouns'\n")
    o.write("# (CC-BY-SA) intersected with WordNet 3.0 noun lemmas. One lemma per line.\n")
    o.write("# Rebuild: scripts/provision-countability.sh\n")
    for w in sorted(inter):
        o.write(w + "\n")
print(f"DONE: {len(inter)} uncountable lemmas (of {seen} category members) → {out_path}", file=sys.stderr)
PY

echo "provision-countability: wrote $OUT ($(grep -vc '^#' "$OUT") lemmas)" >&2
