# WordNet ↔ UMLS concept unification — protocol

**Goal.** When WordNet and UMLS name the *same concept*, make the lexicon denote **one** concept
instead of two. `state` was `wn:n00024720` **and** `umlscui:C1442792` — with verbatim-identical
glosses — so every occurrence doubled the readings.

Design note: [d63-wordnet-umls-concept-unification.md](../../docs/notes/d63-wordnet-umls-concept-unification.md).

---

## Pipeline

```bash
# 1. Candidates — deterministic, no LLM. Every (UMLS concept, WordNet noun synset) pair sharing a
#    surface. 102 292 pairs. UMLS surfaces are LEMMATISED against WordNet (morphy), so the plural
#    atom `Genes` is a candidate for the synset its singular matches — it is a separate entry in the
#    chain (`e_C0017337_0`) and would otherwise never merge.
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

# 5. Resolve the verdicts into the merge set. Deterministic — the three rules are in
#    `crates/eigenius-lexicon-align/src/merge.rs` and are unit-tested there.
cargo run --release --bin lexicon-align -- merges
#    → 102 292 candidates, 81 305 verdicts, 27 196 accepted concept pairs, 311 ties dropped,
#      20 unjudged → 38 389 merges.

# 6. Build the aligned snapshot: emit the layer from merges.json (reading the committed entries
#    out of the base chain) and load it through the kernel. ONE step — the .esl is a build
#    artefact of this run, never a hand-carried input.
scripts/build-alignment-snapshot.sh --base <base> --out <aligned>
#    → 38 389 merges rewrite 51 939 entries (each (cui, surface) can hit both the count entry and
#      its additive `_mass` variant).
```

> **Why 5 exists as a command.** It was a Python one-off, so the load-bearing rule (**a verdict is
> about `(cui, synset)` — the surface is only how the pair was found, so one verdict licenses every
> surface of the concept**) lived nowhere in the repo. Re-deriving it by hand resolved 8 confidence
> ties loosely, and one of them was `cell` — a word on every other line of the WRN page.

> **Why 6 is one step and not two.** It used to be two: emit, then load the emitted `.esl`. On
> 2026-07-12 `merges.json` was rebuilt and only the *load* was re-run. The **stale `.esl` from the
> previous emit loaded cleanly**, the snapshot was named `-v3`, and the measurement reported a v2
> result under a v3 name. Nothing failed — the wrong thing succeeded. The intermediate a human can
> forget to refresh is now gone.

---

## What is committed, and why

| file | committed | why |
|---|---|---|
| **`alignment.jsonl`** | **YES** | 81 305 LLM verdicts, ~$90, **NOT reproducible** (temperature 0 still drifts). Losing it means re-spending the money *and* getting different answers. |
| `merges.json` | yes | the resolved merge set (38 389) — the emitter's input. Derived from `alignment.jsonl` + `candidates.jsonl` by step 5, so it is reproducible; committed because the emitter needs it and regenerating means regenerating candidates first. |
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
confidence; ties DROPPED** (311 of them). With no basis to choose, *prefer to miss*: a missed merge
changes nothing, a wrong one points a word at the wrong concept. `cell` (C1948049) is one of the 311.

**A verdict is about `(cui, synset)`. The surface is only how the pair was found.** One verdict
therefore licenses the merge for **every** surface of that concept. The adjudicator judged
`C0017337` ↔ `n05436752` having been shown `gene`; the chain holds `e_C0017337_0` = **"Genes"** as a
separate entry, and it merges on the same verdict. Keying merges on the judged surface dropped every
plural — 11 700 entries, for free, once fixed.

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
| v2 (+ the un-glossed half) | 26 690 | readings **−0.3%**, `encoded` unchanged |
| **v3 (+ plurals; one-verdict-per-concept)** | **38 389** | readings **−3.6%** (2320→2237), `encoded` unchanged (3) |

**Cross-lexicon de-duplication is done, and it is not the lever.** `grammar-gap 0` held throughout —
v3 rewrites 51 925 entries, 2.8× v2, and still destroyed no correct reading — so it is *correct* and
worth keeping (the extra merges will matter on other text), but it does not reach this corpus.

The **−3.6% is soft, not clean signal.** The v2 (2320) and v3 (2237) totals are separate live-reranker
runs, so the delta conflates the lexicon change with temperature-0 reranker drift; and cap-backfill
absorbs part of every merge (freeing a cap slot lets the parser admit the next sense), which is why
the worst unit's max reading count went *up*, 128 → 184. The direction is right — dedup can only
remove readings — but to isolate the lexicon effect from drift you must replay one `ranks.json`
across both snapshots. `encoded` is flat at 3 either way.

**v3 falsified a specific prediction.** The two units that were pure sense ambiguity over one
skeleton and whose competing senses were exactly the unmerged plurals — `Thus, MSI tumours need
novel therapies` (8 readings) and `Germline mutations in the MMR genes … Lynch syndrome` (3) — were
predicted to collapse to a single reading once the plurals merged. They **dropped** (8→4, 3→2) but
did **not** collapse. The plural merge removed one competing sense each; another remained. Same
lesson a third time: alignment cuts reading *multiplicity*, never down to *one*.

**Why v1 was a wash:** collapsing a duplicate freed a cap slot, and the parser immediately refilled
it with the next sense — often junk. That changed on 2026-07-12, when the reranker gained the ability
to **eliminate** senses (see [../parsing/README.md](../parsing/README.md) §9): a freed slot now stays
free. **Sense elimination, not alignment, is what moved `encoded` (1 → 4).**

**What remains is structural, or junk — never an unmerged duplicate.** The worst unit — `MSI occurs
in colon, gastric, endometrial and ovarian cancers` — is **168 readings across 93 distinct
skeletons**, `sense× ≈ 1.8`: coordination and PP attachment. And the residual *sense* competition,
where there is any, is a UMLS **metadata artefact** the adjudicator correctly declined to merge — not
a duplicate it missed. The v3 dive on `novel therapies are needed for tumours with MSI` (4 readings,
2 skeletons) shows exactly one sense pair left: `C0686904` **"Patient need for (contextual
qualifier)"** — a data-entry qualifier seeded as a content noun — against WordNet `n00023773`
*motivation/need*. Judged `same=false` (0.88), so alignment leaves it; the reranker must kill it in
context. Alignment removes true duplicates; what it leaves is structure and junk entries, and neither
is its job.

---

## Known gaps

- **1 pair unadjudicated**: `clostridium perfringens epsilon toxin` — the model declines (a CDC select
  agent), returning no tool call. Fails closed: recorded, not merged, and visible rather than
  silently defaulting to "different".
- **The prompt changed between v1 and v2** (the metadata-artefact rule; the un-glossed handling), and
  the resume reuses v1's verdicts rather than re-judging them. ~$40 saved against a mild
  inconsistency — the new rules target territory v1 never touched, but it is not free.
- **Junk senses are now retracted in the lexicon, not only at parse time (2026-07-20).** The drop
  set's **second path** (`crate::drops`, "metadata-artefact CONCEPTS") removes them at import:
  `Specialty Type - cancer` (`C1547140`, an HL7 oncology-specialty code competing with the disease)
  and `Specific (qualifier value)` (`C0205369`, an adjective reified as a code — the sense the
  reranker-gloss fix had to demote per sentence) no longer seed a `LexicalEntry`. The concept is
  identified from its UMLS preferred name — a curated set of HL7 code-table prefixes (`Specialty Type
  - `, `Specimen Source Codes - `, …) and the SNOMED modifier tags `(qualifier value)` / `(attribute)`
  / `(qualifier)` — under the same gate the case path uses (collision ⇒ WordNet covers the surface; a
  confident `same=false` verdict; never a merged surface). It caught **258** junk `(cui, form)` atoms
  across **235** concepts over the whole lexicon (drops.json 17 → 275). This is distinct from
  alignment (which only merges genuine duplicates) and reranking (which only hides them per-sentence).
  - **Residual gap:** the criterion is preferred-name-pattern-based, not source-based. A cleaner
    signal is the atom's SAB (HL7V2.5 / HL7V3.0 / administrative vocabularies), but `Candidate` does
    not carry SAB; adding it would let the importer refuse an administrative-source atom on a
    common-word surface without curating prefixes. Entity-tagged SNOMED concepts a real noun carries
    (`(finding)`, `(procedure)`, `(substance)`) are deliberately left in — they are genuine senses.

## `merges-lemma-keyed.json` — a DERIVED artifact (2026-07-26)

`merges.json` stays the recorded adjudication: 38 389 LLM verdicts, never hand-edited. This file is
mechanically derived from it and regenerable; it is **not** a second source of truth.

**Filter:** drop a merge whose `surface` is a regular English plural (`-ies→-y`, `-s` excluding
`-ss`/`-us`/`-is`) of ANOTHER surface merged for the SAME `cui`. 38 389 → **30 776** (7 613 dropped).

**Why.** The lexicon is lemma-keyed and WordNet honours that by construction, but UMLS `MRCONSO.STR`
holds surface strings, so plurals ship as forms. On 2026-07-12 `merges.json` was deliberately grown
26 690 → 38 389 *to add those plural surfaces*, and 24% of the merges are now inflected forms. Once the
importer prunes inflected duplicates (`convert::is_inflection_of_sibling`) those merges match no entry
in the base chain and become no-ops — silent dead weight that also makes the run's "entries redefined"
count stop meaning what it used to.

**Scoped WITHIN one cui, deliberately.** The sibling is the evidence that the surface really is an
inflection. Without it the crude rule is unreliable: the no-sibling group is dominated by non-plurals
(`aids`→`aid`, `acoustics`→`acoustic`, `nervus abducens`, `acanthosis nigricans`), so those merges are
left ALONE rather than corrected. Correcting them would rewrite `aids` to `aid`.

**Regenerate** with the filter above over `merges.json`; do not edit either file by hand.
