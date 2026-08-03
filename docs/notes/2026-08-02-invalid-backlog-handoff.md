# Driving the `invalid` backlog to zero — state and next steps (2026-08-02, rev 2)

Handoff note, rewritten end-of-session. The goal is unchanged: eliminate **fundamentally incorrect**
parses on the WRN-helicase first page (CNL-v3, 62 units), measured as the adjudication ledger's
`invalid` rows, with `grammar-gap 0` **non-negotiable** and no loss of a pinned correct reading.

## Where things stand

Baseline `a369fcd`, snapshot `wordnet-umls-aligned-2026-08-02-stative-with`, tracked replay
`experiments/parsing/ranks/2026-07-29-demonstratives.json`.

| | session start | now |
|---|---|---|
| grammar-gap | 0 | **0** |
| expected-hits | 60/62 | **60/62** |
| encoded | 12 | 14 |
| total-readings | 988 | 793 |
| total-skeletons | 179 | **147** |
| `invalid` | 15 rows / 7 units | **3 rows / 2 units** |
| unadjudicated | 0 | **0** |

Four commits, each measured against the one before on the same base and the same drift-free replay:

| commit | change | skeletons | `invalid` |
|---|---|---|---|
| `80afab8` | adverbs BIND the clause feature (kernel) | 164 → 160 | 11 → 6 |
| `6f2a0dd` | baseline adoption for the above | — | — |
| `b90e78b` | VP-adjunct enumeration; the lost pin was an INVALID parse | 160 → 150 | 6 → 3 |
| `a369fcd` | stative `with` participles; the MSI pin RESTORED | 150 → 147 | 3 → 3 |

## The 3 live `invalid` rows

Computed by intersecting `adjudications.tsv` against the produced skeleton set (147 pairs), not read
off the file — most `invalid` rows in the file are now STALE (35 stale rows).

| unit | rows | axis |
|---|---|---|
| We hypothesized that other DNA repair defects would give rise to synthetic-lethal relationships. | 1 | scope island |
| These classifications were highly concordant with PCR-based MSI phenotyping and with predicted MMR deficiency. | 2 | false identity |

**A probe already exists for the first.** `experiments/parsing/probes/frame22-oblique.esl` was written
for exactly this row: WordNet frames 22 and 23 are SWAPPED in `convert.rs::classify` (22 is
`Somebody ----s PP`, 23 is `Somebody's (body part) ----s`), so `give rise` gets no oblique-PP slot and
its `to`-PP can only attach as an adjunct — which then escapes the `that`-clause onto the matrix
subject. The layer simulates the corrected classification for one synset. **It has not been measured
on any recent baseline.** That is the cheapest next thing in the backlog.

**The second is a regression to watch.** All five rows on the `classifications` unit went stale under
the adverb binding (`80afab8`); two are LIVE again under this baseline. Which of the two later layers
reintroduced them is **not measured** — check that before treating them as old news.

## What this baseline owes: a reseed

Both adopted layers are lexicon-level over `wordnet-umls-aligned-2026-08-01-prednom`, so the source
and the snapshot disagree. Nothing is reproducible from a clean checkout until this lands.

1. **VP-adjunct prepositions** — `ontologies/lexicon/closed-class.esl`: the 11 prepositions' `fin_any`
   becomes the six concrete verbal `Fin` values (source: `probes/vpadj-enumerate-verbal.esl`).
2. **Stative relational participles** — `crates/eigenius-wordnet/src/convert.rs`: a verb synset whose
   frames NAME a governed preposition also emits `(S[adj]\NP)/cat_pp_arg(prep_X)` over a 2-place
   axiom. `FrameKind::Essive` (`((S\NP)/cat_pp_arg(prep_as))/NP`) is already the correct template —
   the machinery exists and simply is not applied to the frames that name a preposition. **Ship
   frames 17/31 (`with`, 97 synsets) only** — see the scope limit below.
3. **Drop the five redundant `_prednom` entries** from `closed-class.esl`
   (`is/are/was/were_copula_prednom`, `not_adj_prednom`) — subsumption makes them duplicates.

Reseed needs `--umls-all`. Re-measure all four numbers afterwards; the layer and the reseed are not
guaranteed to agree (the 2026-07-25 determiner-holes cycle verified they did, and said so explicitly).

### Scope limit on (2), measured

The full rule over all five prepositions (15 `to` | 16 `from` | 17 `with` | 18 `of` | 19 `on`, 378
synsets / 844 entries) **fails coverage**, `grammar-gap 1`. Bisected: frame 15 (`to`, 185 synsets) is
entirely INERT; the breakage is in `from`/`of`/`on`, narrowed to frame 19, then to ONE synset
(`set.v.01115006`, 2 entries). Its `set` entry EVICTS `cat_n(n05674584, mass)` from the `sets` leaf of
«These data sets are project Achilles and project DRIVE.», which then has no parse.

**But that unit's only reading is itself wrong**, so the coverage loss is a wrong reading vanishing
from a unit that has no right one — see the copula section. It is held back because coverage is
coverage, not because the rule is wrong.

---

## The two structural findings that are now the main board

### 1. The per-entry sense cap makes lexical additions zero-sum

`dcg::parse::seed` caps entries per lemma. The cap is keyed per **SENSE** but truncates per **ENTRY**,
and one sense legitimately has several entries (different grammatical categories). This was
characterised on 2026-07-24 with two fixes measured and rejected — and it hit us **three separate
ways in one session**:

- it hid **4 of the 6** enumerated VP-adjunct prepositions per surface (all six share one sense key),
  so the adopted enumeration ships 66 entries of which ~22 are live and is closer to `require
  fin|bse` than to a real enumeration. Witnessed by dumping `leaf[3..3]` at `SENSE_CAP` 2 vs 32;
- it makes `Fix A (c)`'s positive-relational entry inert page-wide (recorded in `seed.rs`, emitted
  4th, never seeds at base cap);
- it **evicts a noun to gap a unit** — the frame-19 case above. Neither the added entry nor the
  evicted one is in the ranker's kept list, so both are unranked and **emission order decides**.

The consequence to internalise: **the lexicon is at capacity for common surfaces, so any correct
addition costs a deletion.** Every "add the missing entry" fix is gated on this.

Do not retry the two rejected fixes blind — read `seed.rs` lines ~204-229 first. Cap-by-sense was
measured twice (encoded 10 → 0, then 11 → 4); exempting the closed class fails
`sense_reranker_overrides_static_cap_order` because `in_lexicon.is_none()` means UNTAGGED, not
closed-class.

### 2. The copula supports predication but not identification or location

Three gaps, each isolated with a minimal pair on the real page (append test sentences to a copy of
the page so the document glossary still mints named individuals — see Instruments):

| sentence | result |
|---|---|
| These data sets are **large**. | ✅ `gt(deg_large(the(C0150098)), std_large)` |
| These data sets are **from WRN**. | ✗ gap — no copula + PP complement |
| These data sets are **the data sets**. | ✗ gap — no equative with a referential complement |
| **These** are large. | ✗ gap — no demonstrative pronoun |
| These data sets are **WRN**. | ✅ parses (bare NP complement works) |

The first row is the control: the subject side is healthy and resolves to **C0150098** (`data set`).
Everything that fails, fails on the predicate side.

**This is why «These data sets are project Achilles and project DRIVE.» is corrupt.** It is an
equative — identity between a definite kind and coordinated named individuals — and the construction
does not exist, so the parser lands on the only analysis that composes: `data` = `datum`
(n05816622) with the VERB `put/place` (v01494310). C0150098 IS built on span [1..2] and appears in
**no reading**. The unit's pin is satisfied by that wrong reading because pins are skeletons and
skeletons are sense-erased (`_provenance_note_2026-07-28-pins-are-sense-blind`).

Chart evidence for where it dies:
- `cell[4..5]` `project Achilles` → `cat_np(n00795720, sg)` ✅
- `cell[4..8]` coordination completes only as a type-raised **subject** `S/(S\NP)` and as
  `cat_group(n00795720, conn_and, pl)` — **no plain `cat_np`**
- `cell[3..8]` `are project Achilles and project DRIVE` → one node, `cat_group(conn_and, pl)`.
  **No VP at all**, so the subject GQ in `cell[0..2]` has nothing to apply to.

**core-en gives the categories for two of the three, and punts the semantics.**
`v.xsl` `Copula` family: `<entry name="NP">` uses `$tv`, i.e. the copula with an NP complement is
categorially just a transitive verb `(S[dcl]\NP)/NP` — the missing entry. `pp.xsl` `Prep-Loc` is
`pos="Adj"` with a `Predicative` entry, so a locative/source preposition IS a predicative adjective
and the existing adjectival copula consumes it — "X is from Y" needs no new copula entry at all.
`np.xsl` `ProNP` is a plain NP, the shape a bare `these` wants.

But core-en's `NP` entry carries `be(Arg:X, Pred:Y)` over two entities, with an explicit source
comment: `<!-- NB: This doesn't really capture the predicational nature of Y. -->`. It collapses
predicational and identificational. Eigenius already goes further (predicate nominals get real
membership `is_a` via `a_pred`), so adopting that semantics wholesale would be a regression.

**The open design question**, not yet decided: for «These data sets are project Achilles and project
DRIVE.», the subject is ANAPHORIC — it refers to the referent introduced by «We analysed two
independent cancer dependency data sets.», which parses OPEN
(`ΠG#0:Prop. ΠG#1:ΣG#1:C0150098. … → G#0 → G#0`). The sentence's job is to RESOLVE that referent.
Two shapes:

- **assert an identity** — add `eq : Entity → Entity → Prop` plus a copula entry taking `cat_group`.
  Needs group-denoting terms the semantics doesn't otherwise have. Note apposition — the one existing
  construction expressing this relation, «the MMR genes MSH2, MSH6, PMS2 or MLH1» — **distributes**
  the group over the containing predicate rather than forming a group term, so it is not a usable
  template.
- **treat it as resolution** — D64 referent holes already exist and are Π-abstracted, β-reduced on
  resolution. This is the case that mechanism was built for. More invasive; check how resolution is
  currently triggered before committing.

Also worth measuring: whether the demonstrative-as-definite convention (`these` → `the(§)`, pinned
across several units) is itself the obstacle, since it discards the anaphoric link at the lexicon
before the grammar sees it.

---

## Corrections to the record made this session

Do not re-derive these; each cost a cycle.

- **Adverbs are NOT emitted by the WordNet importer.** `Pos::Adv => {}` is deferred and the importer
  emits no `(S[adj]\NP)/(S[adj]\NP)` anywhere. They are built in the KERNEL by
  `dcg::category::adverb_modifier_cats` and seeded in `dcg::parse::seed`. The previous note and
  `baseline.json` both said importer + reseed; it was two `cargo build`s.
- **The `adjective-frames.tsv` row for `associated` is DEAD.** `governed_preposition` is reached only
  from `push_adj`, over the words of ADJECTIVE synsets, and `associated` is not a WordNet adjective
  lemma (`index.adj` 0, unlike `dependent`/`essential`/`concordant`, all 1). Verified: `associated`
  carries no `cat_pp_arg` category at all at `SENSE_CAP=32`.
- **"The 7×7 cross-product makes every feature pair expressible and still loses the pin" is not
  supported.** Those runs were at `SENSE_CAP=2`, so ~2 of 49 entries per preposition were ever live.
  The erasure conclusion happens to be right, but for the `pss`→`pass` reason below, not that one.
- **The MSI pin was standing on a voice-feature violation.** `associated` seeds a 1-place
  `S[dcl,pss]\NP` (past participle, ACTIVE/perfect); no `is` entry selects a SATURATED `S[pss]\NP` —
  deliberately, so `*X is affected Y` stays out. The `fin_any` VP-adjunct erased the feature and the
  erased clause satisfied **all four** saturated copula slots at once (adj/ger/pass/bse), witnessed
  in `cell[1..9]`.
- **A `grammar-gap` is not automatically the loss of a correct parse.** The frame-19 case removes a
  reading that was already nonsense. Check what the unit's readings ARE before reading a coverage
  metric as a regression.

## Instruments

| instrument | gives | caveat |
|---|---|---|
| `scripts/audit-skeletons.sh` | the validity ledger: correct / available / **invalid**; fail-closed | authoritative skeleton set; needs `EIGENIUS_DB_SNAPSHOT` + `EIGENIUS_SENSE_RANKS` matching the baseline |
| `scripts/probe-mode-layer.sh --base … --layer …` | a patch-layer snapshot + full measure, NO reseed | lexical entries and axioms are patchable; TYPE declarations are not (ManifestDrift) |
| `EIGENIUS_DUMP_READINGS=1` | raw term + gloss on the sweep WITH the document overlay | the only one like-for-like with the ledger; over-counted skeletons 166 vs 164 once — do not use for a census |
| `EIGENIUS_DUMP_CELL=i..j` | packed cell: category + provenance + the Combine that built it | **works during the page sweep**, which is how the ranks-dependent leaf differences were found; samples 20 items per block |
| `EIGENIUS_TRACE_SENTENCE` + `EIGENIUS_TRACE_SKELETONS=1` | fast single-sentence skeletons | **cap-only, no document overlay** — different reading set; leaves can be identical here and differ under the sweep |
| `EIGENIUS_DUMP_SKELETONS=1` | `«unit» [N skeleton(s)]` + indented skeletons | the produced set, for intersecting against the ledger to find LIVE rows |
| `cargo test -p eigenius-wordnet --test gen_stative_layer` | generates the stative layer from `data.verb` | uses the importer's own `past_participles`; `EIGENIUS_STATIVE_FRAMES=17,31` restricts the frame set |

**Testing a sentence that needs the document glossary:** copy the real page, append the test
sentences, point `EIGENIUS_WRN_PAGE` at the copy. `EIGENIUS_TRACE_SENTENCE` cannot do this — named
individuals like `ni_project_achilles` are minted from the document.

**The recorded trap:** skeletons are sense-erased (`§`) and glosses are verbalized English, so
grepping either for a predicate name matches nothing and reads as "0 occurrences" rather than as the
category error it is. This has produced several wrong analyses in this corpus.

## Standing constraints

- `grammar-gap 0` is **non-negotiable**. A lost pin outranks any skeleton reduction — *unless the
  pinned reading is shown invalid*, which is what unblocked the enumeration.
- `--release` is load-bearing (debug overflows the stack in NbE readback → phantom GRAMMAR-GAP).
  Use `scripts/measure-parse-rate.sh`; do not hand-roll `cargo test`.
- `EIGENIUS_WRN_PAGE` / `EIGENIUS_SENSE_RANKS` / `EIGENIUS_DB_SNAPSHOT` must be ABSOLUTE paths.
- Any reseed needs `--umls-all`. The default is a subset (3.29M resources vs 9.19M) and the mismatch
  is SILENT — it once produced a fake catastrophic regression.
- UMLS data is licensed; `/references` is gitignored and UMLS content is never committed.
- `probe-mode-layer.sh` has **no guard** against a kernel-image / base-snapshot bootstrap mismatch. It
  surfaces as `error: kernel exited before becoming healthy` with a hash dump. Cost a debugging cycle
  on 2026-08-01.
- 35 STALE ledger rows. Deleting them is a deliberate act — several carry the diagnosis of defect
  families that are still partly open. 2 of the stale entries are PINS, not ledger rows, and
  `expected-readings.tsv` forbids re-pinning either.
