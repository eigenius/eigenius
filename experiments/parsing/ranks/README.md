# Recorded sense rankings — the only irreplaceable measurement artifact

A `ranks.json` is a **recording of an LLM's sense choices**. Unlike the snapshots, the run logs and
`results/` (all regenerable), it cannot be reproduced: `temperature: 0` is not deterministic here — two
live runs of identical code against an identical store differ on ~5% of the capped top-2, which is the
only part that reaches the parser.

`experiments/*/results/` is gitignored, so a recording left there is one `git clean` from gone while
every A/B depends on it. Recordings that become a reference live HERE, tracked.

## `2026-07-26-lemmakeyed.json`

Adopted 2026-07-26 as the replay reference for the lemma-keyed lexicon
(`wordnet-umls-aligned-2026-07-26-lemmakeyed`; UMLS inflected-duplicate prune + `merges-lemma-keyed.json`).
Recorded live at commit `27f6bd0`; replays drift-free (no key misses) against that snapshot.

Use it with:

    EIGENIUS_DB_SNAPSHOT=…/wordnet-umls-aligned-2026-07-26-lemmakeyed \
      scripts/measure-parse-rate.sh --replay experiments/parsing/ranks/2026-07-26-lemmakeyed.json

**It scores 44/45 on the faithfulness gate, and that is expected** — see the header of
`../expected-readings.tsv`. Four live draws on this lexicon scored 44, 44, 43, 44; 45/45 did not come
up once and which pin misses varies. This recording's miss is «Synthetic lethality is an interaction
between two genetic events.», whose pinned structure is reachable in other draws.

**A recording is only valid for the lexicon it was made against.** The rank key includes each word's
candidate sense list, so any lexicon change that touches a sentence's candidates makes the key MISS,
the replay falls back to seed order, and sense elimination silently switches off for that sentence
(observed: readings 1218 -> 1953, encoded 9 -> 8, and a pin failing for that reason alone). The harness
asserts on key misses — do not work around it; re-record instead.
