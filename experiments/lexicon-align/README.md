# WordNet ↔ UMLS concept unification — protocol

**Goal.** When WordNet and UMLS name the *same concept*, make the lexicon denote **one** concept
instead of two. `state` was `wn:n00024720` **and** `umlscui:C1442792` — with verbatim-identical
glosses — so every occurrence doubled the readings.

Design note: [d63-wordnet-umls-concept-unification.md](../../docs/notes/d63-wordnet-umls-concept-unification.md).

---

## Pipeline

```bash
# 1. Candidates — deterministic, no LLM. Every (UMLS concept, WordNet noun synset) pair sharing a
#    surface. ~79k pairs.
cargo run --release --bin lexicon-align -- candidates \
  --out experiments/lexicon-align/candidates.jsonl

# 2. Validate the judge BEFORE trusting it. It must recover the gold set (near-identical glosses).
cargo run --release --features use-llm --bin lexicon-align -- validate-gold
#    → 99.3% recall (292/294). Below 95% ⇒ STOP.

# 3. Probe its PRECISION — the dangerous direction. A wrong merge DESTROYS the correct reading;
#    a missed merge only leaves things as they are.
cargo run --release --features use-llm --bin lexicon-align -- precision-probe --n 200

# 4. Adjudicate everything. Concurrent, retrying, RESUMABLE (verdicts flush as they land).
cargo run --release --features use-llm --bin lexicon-align -- adjudicate \
  --concurrency 16 --out experiments/lexicon-align/alignment.jsonl
#    → ~30 min, ~$40. Fails CLOSED: a batch that exhausts its retries records NOTHING.

# 5. Emit the alignment LAYER (reads the committed entries from the chain; see below).
cargo run --release --features chain --bin lexicon-align-emit \
  --snapshot <a COPY of the base snapshot>

# 6. Load it onto a copy of the base, producing a new snapshot.
scripts/add-layer-to-snapshot.sh --base <base> --out <aligned> \
  experiments/lexicon-align/alignment.esl
```

---

## What is committed, and why

| file | committed | why |
|---|---|---|
| **`alignment.jsonl`** | **YES** | 77 167 LLM verdicts, ~$80, **NOT reproducible** (temperature 0 still drifts). Losing it means re-spending the money *and* getting different answers. |
| `merges.json` | yes | the adjudicated, conflict-resolved merge set (26 690) — the emitter's input |
| `gold-/probe-verdicts` | yes | the validation record |
| `candidates.jsonl` | no (gitignored) | deterministic, ~1 min to regenerate |
| `alignment.esl` | no (gitignored) | deterministic, 15 s to regenerate |

---

## The rules that keep it safe

**Merge only at confidence ≥ 0.85.** The probe found a real false merge below it: `attachment` —
UMLS *"a file affixed to another file"* (an email attachment) vs WordNet *"a supplementary part or
accessory"*. Different concepts. The model proposed **nothing** below 0.70, so its own uncertainty is
the usable signal.

**One entry, one class.** An entry `(cui, surface)` proposed for two synsets is resolved by **highest
confidence; ties DROPPED** (208 of them). With no basis to choose, *prefer to miss*: a missed merge
changes nothing, a wrong one points a word at the wrong concept.

**Exclude WordNet INSTANCE synsets** (`@i` — `Africa`, `Alabama`). The importer emits them as a
`resource`, not a `class`, and an entry's `cat_n(C, num)` requires `C : Set`. Pointing an entry at an
individual is a type error. **The kernel validator caught this** — 405 such merges produced **721
violations** and the layer was rejected outright. That is the whole reason step 6 loads through the
kernel instead of writing to the store directly.

**Exclude UMLS named individuals** (`cat_np(umlssty:<TUI>, sg)`) — the symmetric case: an instance
cannot denote a class.

**No pre-filter on semantic type.** Requiring the UMLS TUI and the WordNet supersense to agree was
cross-validated and is too lossy to gate on: keeping 93% of known duplicates removes only 23% of the
work, and cutting 61% of the work **discards a quarter of the duplicates** — silently.

---

## The emitter changes two fields and nothing else

Entries are read **from the chain**, never reconstructed — the committed resource is the truth, and
rebuilding it would silently drift (the additive mass variants, `sense_rank`, whatever the importer
adds next).

```
cat  : cat_n(umlscui:C1442792, num_any)  →  cat_n(wn:n00024720, num_any)
sem  : umlscui:C1442792                  →  wn:n00024720
```

Everything else passes through verbatim. **`sense` is deliberately NOT rewritten** — the seed-time
dedup (`dedup_same_concept`) keys on `(cat, sem)`, so the label is irrelevant to it.

> **This is why `ranks.json` could not see the merges.** It records the sense *label*, which the
> alignment never touches, so a merged entry still reported `umls:C1442792`. A "47% → 48%" reading
> taken from it was meaningless. `ranks.json` now also records the resolved `sem`.

**No class is created or modified; no `subclass_of` edge is emitted.** The type lattice is untouched.
(2026-07-11: adding lattice edges — a supersense parent on every WordNet noun, the UMLS TUI ISA tree
— broke the parses and the branch was reverted.)

---

## Result — measured, and negative

| | merges | effect on the WRN page |
|---|---|---|
| v1 (glossed only) | 12 450 | readings **−4.3%**, `encoded` unchanged |
| **v2 (+ the un-glossed half)** | **26 690** | readings **−0.3%**, `encoded` unchanged |

**Cross-lexicon de-duplication is done, and it is not the lever.** `grammar-gap 0` held throughout,
so it is *correct* and worth keeping — the extra merges will matter on other text — but it does not
reach this corpus.

**Why v1 was a wash:** collapsing a duplicate freed a cap slot, and the parser immediately refilled
it with the next sense — often junk. That changed on 2026-07-12, when the reranker gained the ability
to **eliminate** senses (see [../parsing/README.md](../parsing/README.md) §9): a freed slot now stays
free. **Sense elimination, not alignment, is what moved `encoded` (1 → 4).**

**What remains is structural.** The worst unit — `MSI occurs in colon, gastric, endometrial and
ovarian cancers` — is **168 readings across 93 distinct skeletons**, with `sense× ≈ 1.8`. That is
coordination and PP attachment. No lexicon work reaches it.

---

## Known gaps

- **1 pair unadjudicated**: `clostridium perfringens epsilon toxin` — the model declines (a CDC select
  agent), returning no tool call. Fails closed: recorded, not merged, and visible rather than
  silently defaulting to "different".
- **The prompt changed between v1 and v2** (the metadata-artefact rule; the un-glossed handling), and
  the resume reuses v1's verdicts rather than re-judging them. ~$40 saved against a mild
  inconsistency — the new rules target territory v1 never touched, but it is not free.
- **Junk senses are filtered at parse time, not in the lexicon.** `Specialty Type - cancer` (a
  *discipline*, competing with the disease) is still an entry; the reranker now eliminates it in
  context. That is arguably better than a static filter — it is contextual — but the junk is still
  there.
