# The `invalid` backlog is at zero — state and what comes next (rev 3, 2026-08-03)

The goal this note tracked: eliminate **fundamentally incorrect** parses on the WRN-helicase first
page (CNL-v3, 62 units), measured as the adjudication ledger's `invalid` rows, with `grammar-gap 0`
non-negotiable and no loss of a pinned correct reading.

**That goal is met on this page.** The parser now produces no reading a human has judged
inadmissible. This revision records the end state, the four defects that got us here, what is
provably NOT worth retrying, and where the next work actually is — which is no longer this page.

## Where things stand

Baseline `3d2d052`, snapshot `wordnet-umls-aligned-2026-08-02-consolidated`, tracked replay
`experiments/parsing/ranks/2026-07-29-demonstratives.json`.

| | session start | now |
|---|---|---|
| `invalid` | 15 rows / 7 units | **0** |
| unadjudicated | 0 | **0** |
| grammar-gap | 0 | **0** |
| expected-hits | 60/62 | **60/62** |
| total-skeletons | 179 | **144** |
| total-readings | 988 | 761 |
| encoded | 12 | 14 |
| patch layers on the snapshot | 3 | **0** |

`scripts/eval-parse-rate.sh --baseline` exits 0. The replay keys cleanly against the reseeded
lexicon (62 hits, 0 misses), so the recorded sense draw survived the reseed rather than degrading to
cap-only.

The snapshot is a plain reseed of committed source. Reproduce with BOTH steps — the first makes a
BASE snapshot and every measurement here is against an ALIGNED one:

```
scripts/reseed-lexicon-db.sh --umls-all --snapshot-dir wordnet-umls-2026-08-02-consolidated
scripts/build-alignment-snapshot.sh --base <that> --out wordnet-umls-aligned-2026-08-02-consolidated
```

## The four defects, and what each cost

Every one was a real bug found by reading the chart, not a threshold that got tightened.

| commit | defect | `invalid` | skeletons |
|---|---|---|---|
| `80afab8` | the forward adverb modifier laundered the clause feature | 11 → 6 | 164 → 160 |
| `b90e78b` | VP-adjunct prepositions erased finiteness via `fin_any` | 6 → 3 | 160 → 150 |
| `a369fcd` | governed preposition discarded — stative relational participle | 3 → 3 | 150 → 147 |
| `dfec40a` | **WordNet frames 22 and 23 transposed** against `wninput.5` | 3 → 2 | 147 → 146 |
| `c1adb65` | consolidation reseed — all three layers into source | 2 → 2 | 146 → 146 |
| `3d2d052` | gloss-governed adjectives seeded a competing noun reading | **2 → 0** | 146 → 144 |

### The one that generalises: laundering

Three of the six are the same shape. A rule typed `X → X` on a feature, fed a *subsumed* value,
returns the *base* value — re-opening the hole the feature split was introduced to close.
`pred ⊑ adj` makes SELECTION permissive by design (the copula, negation and every `-ly` adverb must
accept a predicate nominal), so any rule that accepts `adj` and returns a FIXED `adj` launders. The
cure is a BOUND feature variable shared by result and argument. `dcg::category` documents the
mechanism and `the_forward_adverb_modifier_binds_the_clause_feature_it_consumes` pins it.

**One launderer is still open**, and it is the reason a cleanup in this session was reverted — see
"`not_adj` still launders" below.

### The one that was a plain reading error

`convert.rs::classify` mapped `1|2|3|22 => Intransitive` and `4|12|23|27 => PpOblique`, but
`references/WordNet-3.0/doc/man/wninput.5` line 212 reads

```
22  Somebody ----s PP
23  Somebody's (body part) ----s
```

They were transposed, so frame 22's PP was dropped. `give rise` (synset 01752884) got no argument
slot for its `to`-PP, the PP fell to adjunct position, and a free adjunct escaped a finite
`that`-clause onto the matrix subject. **Two unit tests asserted the inverted mapping**, which is
why it survived. Check the vendored spec, not the existing test.

## Three times a "lost pin" was not a loss

This is the single most useful habit the page taught, and it is worth stating as a rule:
**a dropped pin is a question, not a verdict.** Read the unit's readings before treating it as a
regression.

- The VP-adjunct enumeration was blocked for two cycles by a pin it "cost". The pinned reading was
  standing on a **voice-feature violation** — `associated` seeds a 1-place `S[dcl,pss]\NP` and no
  `is` entry selects a saturated `S[pss]\NP` (deliberately, so `*X is affected Y` stays out); the
  `fin_any` VP-adjunct erased the feature, and the erased clause satisfied **all four** saturated
  copula slots at once. Witnessed in `cell[1..9]`.
- The frame 22/23 swap "cost" the `give rise` pin. That pin was the SAME adjunct analysis as the
  unit's `invalid` row, differing only in where the adjunct landed. Deleting the adjunct killed
  both — the unit went 2 skeletons to 1.
- A `grammar-gap` is not automatically the loss of a correct parse either. The frame-19 case removed
  a reading that was already nonsense (`data` = `datum` + the VERB `put/place`).

## Do not retry these — each is measured

- **A kind-raise refusal gated on `is_adjective_refined`.** Too broad; it also removed "the group is
  an INDETERMINATE line" and re-gapped its unit. `registry.rs` records this plus **five other
  refuted mechanisms** for the same unit (open-vs-closed masking, cost penalty, cell-beam eviction,
  widen/cap, classify budget). Read that comment before designing anything near the kind-raise.
- **A governed-preposition prune without an adjectival-result requirement.** `X/cat_pp_arg(prep_R)`
  for concrete `R` is a relational-WORD test, not an adjective test: it also matches relational
  NOUNS whose nominal reading is correct. Witnessed by surface — `deficiency` (6 entries dropped),
  `dependency` (8), `dependence` (8), `result` (6), `vulnerability` (5), `event` (5),
  `co-occurrence` (4), `activity` (15). Result: **grammar-gap 0 → 9, expected-hits 60 → 49.**
  Requiring `is_adjective_cat` on the RESULT is what makes it correct.
- **Two sense-cap fixes** (cap-by-sense; exempting the closed class) — measured and rejected
  2026-07-24. Read `seed.rs` ~204-229 first.
- **`fin_verbal` as a VP-adjunct result feature** — the root gate admits only `fin`/`fin_any`, gap 2.

## `not_adj` still launders — a correction to rev 2

Rev 2 listed "drop the five redundant `_prednom` entries" as owed. **That is wrong for one of
them.** `not_adj` is `(S[adj]\NP)/(S[adj]\NP)` — an X→X rule — so under `pred ⊑ adj` it accepts a
predicate nominal and hands back `adj`, which `is_adjective_cat` then admits for the attributive
lift, reopening exactly the hole `lexicon:pred` closes. `scope_bearing_covers_the_modal_category_sniff`
catches the deletion.

The four **copula** `_prednom` entries are genuine duplicates (they return `fin`, not their argument
feature), but deleting them is behaviourally unmeasured and frees four cap slots on
`is/are/was/were`, which the per-entry sense cap makes non-neutral in principle. All five were
therefore left in place so the consolidation reseed stayed a pure port.

Two follow-ups, both small:
1. Make `not_adj` BIND its clause feature, the cure the adverbs got at `80afab8`. Then the fifth
   entry becomes genuinely redundant.
2. Measure the four copula deletions as a suppression layer (redefine with a non-matching
   `lexicon:form`); fold into the next reseed if clean.

## What is left on this page

Not much, and none of it is the grammar producing wrong readings.

**2 pin misses of 62**, and they are NOT the same kind of thing:

| unit | status |
|---|---|
| «Synthetic lethality is an interaction between two genetic events.» | correct reading is **producible**, absent on this sense draw (hit in draw 2, missed in 1 and 3) |
| «Depletion of WRN induced double-stranded DNA breaks.» | correct reading is **not produced at all** — no skeleton carries the bare form |

The second is a real absence: it differs from its pin by one token, `prep_of(G#0, §)` vs
`prep_of(G#0, kind_of(§))` — WRN as the class *WRN protein, human* instead of the HGNC-symboled
named individual *WRN gene*. Axis B (sense grounding), one of 5 units page-wide missing it. Do not
re-pin either; that would bake in the wrong analysis.

**84 `available` skeletons.** Read the verdict for what it is: "structurally available and not
something the grammar should refuse — semantically false, or true but dispreferred." It explicitly
includes false readings. These are ranking, not grammar.

**38 stale ledger rows.** Deleting them is a deliberate act; several carry the diagnosis of defect
families still partly open, and 2 of the stale entries are PINS, not ledger rows.

## Where the next work is

The `invalid` ledger was the ranking signal for this whole effort. **At zero it no longer
constrains anything**, so continuing to optimise this page is measuring noise.

1. **A second page.** Highest value by a distance. One page is exhausted; the next defects live in
   text this grammar has not seen. Everything below is better attacked with a second page's evidence
   than with this one's.
2. **The per-entry sense cap** — `dcg::parse::seed` caps per lemma, keyed per SENSE but truncating
   per ENTRY. It bit **three ways in one session**: hid 4 of 6 enumerated prepositions per surface;
   keeps `Fix A (c)` inert page-wide; and *evicted a noun to gap a unit*, where neither the added nor
   the evicted entry was ranked so **emission order decided**. The consequence to internalise: the
   lexicon is at capacity for common surfaces, so **any correct addition costs a deletion**. Every
   "just add the missing entry" fix is gated on this.
3. **The copula gap family** — no equative with a referential complement, no PP complement, no
   demonstrative pronoun. Each isolated with a minimal pair; `core-en` supplies categories for two of
   the three (`Copula/NP` = `$tv`; `Prep-Loc` is `pos="Adj"`, so "X is from Y" needs no new copula
   entry) but concedes its semantics collapses predication and identification. This is why «These
   data sets are project Achilles and project DRIVE.» has no correct reading available at all.

## Instruments

| instrument | gives | caveat |
|---|---|---|
| `scripts/eval-parse-rate.sh <run.log> --baseline` | the gate; exit 0/2 | scores an existing run log, not a snapshot |
| `scripts/measure-parse-rate.sh --snapshot … --replay …` | a full measurement | builds RELEASE; do not hand-roll `cargo test` |
| `scripts/audit-skeletons.sh` | the validity ledger, fail-closed | needs `EIGENIUS_DB_SNAPSHOT` + `EIGENIUS_SENSE_RANKS` matching the baseline |
| `scripts/probe-mode-layer.sh --base … --layer …` | patch-layer snapshot + measure, NO reseed | lexical entries and axioms are patchable; TYPE declarations are not (ManifestDrift) |
| `EIGENIUS_DUMP_CELL=i..j` | packed cell: category + provenance + the Combine that built it | **works during the page sweep**; labels tokens, so grep `tok="…"` rather than guessing the index |
| `EIGENIUS_DUMP_SKELETONS=1` | the produced set, for intersecting against the ledger | this is how you find LIVE rows; most `invalid` rows in the file are stale |
| `EIGENIUS_GLOSS_READINGS=1` | verbalized English per skeleton | needs `-- --ignored`; the test is `#[ignore]`d |
| `cargo run -p eigenius-wordnet --bin wordnet-import` | the importer's ESL, standalone | **diff this against a probe layer before spending a reseed** — it caught nothing this time, which is the point |

**Testing a sentence that needs the document glossary:** copy the real page, append the test
sentences, point `EIGENIUS_WRN_PAGE` at the copy. `EIGENIUS_TRACE_SENTENCE` cannot do this — named
individuals are minted from the document, and it is cap-only besides.

**The recorded trap:** skeletons are sense-erased (`§`) and glosses are verbalized English, so
grepping either for a predicate name matches nothing and reads as "0 occurrences" rather than as the
category error it is. This has produced several wrong analyses in this corpus.

## Standing constraints

- `grammar-gap 0` is **non-negotiable**. A lost pin outranks any skeleton reduction — *unless the
  pinned reading is shown invalid*, which happened three times.
- `--release` is load-bearing (debug overflows the stack in NbE readback → phantom GRAMMAR-GAP).
- `EIGENIUS_WRN_PAGE` / `EIGENIUS_SENSE_RANKS` / `EIGENIUS_DB_SNAPSHOT` must be ABSOLUTE paths.
- Any reseed needs `--umls-all`. The default is a subset (3.29M resources vs 9.19M) and the mismatch
  is SILENT — it once produced a fake catastrophic regression. It also needs the ALIGNMENT step
  after it; `build-alignment-snapshot.sh`'s header records a run where skipping the regeneration
  "loaded cleanly" and reported a v2 result under a v3 name.
- Editing `ontologies/lexicon/closed-class.esl` changes the bootstrap hash — the harness fail-closes
  on ManifestDrift rather than silently measuring the old lexicon, so such edits cannot be measured
  before the reseed that bakes them in.
- The reseed logs `building kernel docker image from HEAD (<rev>)` even on a dirty tree. The image
  is built from the WORKING TREE (`context: .`, and `closed-class.esl` is `include_str!`-embedded),
  so the content is right but the recorded provenance is misleading.
- UMLS data is licensed; `/references` is gitignored and UMLS content is never committed.
