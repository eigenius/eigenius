# Hand-off: the `quantities-in-the-parser` branch

*2026-09-29. For the developer taking this branch over.*

## Status `2026-09-29`, second machine — read this before the sections below

This note was written on a different machine from the one it was next picked up on, and several
of its statements are machine-specific or have since been overtaken. What is still true is the
design and the ordering; what follows corrects the rest.

**Overtaken:**

| The note says | Now |
|---|---|
| "the last 15 commits are not pushed; push them before anything else" (step 1) | pushed; `origin/quantities-in-the-parser` holds all 39 including the note itself |
| "the CNL page is not on this machine; obtain it first" (step 2) | present at `references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v3.txt`; the parse-rate run below used it |
| "older snapshots are gone" | the second machine holds them back to `2026-07-11` |
| "disk is the constraint; `cargo test --workspace` fills the volume" | true of the first machine only: 524 GB free here, the full workspace suite runs |
| SPECIALIST "curl from NLM (D97 gives the URL)" | D97 gave only the *terms* page. The URL is now recorded in D97 and scripted as `scripts/provision-specialist.sh`, which verifies the release by content |

**Done since:**

- `e594545` merged `main`'s parse-gate re-baseline (`74da878`). The branch was cut from `c39b8d6`
  and carried figures from 2026-08-17 that two September reseeds had already superseded; measuring
  this branch against those would have credited it with movement earned before it existed.
- `dc15e6e`, `34e4fef` — D97's provisioning: the URL, and a script that checks size, sha256 and
  the three field counts D97 measured, so a *different release* fails loudly rather than shifting
  every D97 number silently.
- **Step 2's measurement is run.** Reseeded and realigned at `34e4fef` into
  `wordnet-umls-2026-09-29-quantities` and `wordnet-umls-aligned-2026-09-29-quantities`
  (`umls_scope: all`). Live reranked run, reranker confirmed engaged, new draw recorded at
  `experiments/parsing/results/2026-09-29-0748-34e4fef-first-page-cnl-v3-reranked/`:

  | | baseline | this branch |
  |---|---|---|
  | units | 62 | 62 |
  | grammar-gap / missing-lexeme | 0 / 0 | **0 / 0** |
  | expected-hits | 62/62 | **62/62**, miss-set unchanged |
  | total-readings | 674 | **626** — improved |
  | total-skeletons | 171 | **175** (ceiling 250) |

  D95 slices 1–5 cost no coverage and shrink the forest by 48 readings while adding 4 skeletons.

**A trap this run found.** `reseed-lexicon-db.sh` names its snapshot from the date alone and
**overwrites a same-day snapshot without warning**. The morning's `main` reseed was replaced in
place by the branch reseed; the aligned store beside it survived, still stamped with the old
base's commit, so the pair silently disagreed. Both are renamed now
(`…-quantities`, `…-main-c39b8d6`) and the survivor's `PROVENANCE` records what happened. The
script should refuse an existing target or suffix it — the same failure the alignment script was
fused to prevent ("nothing failed; the wrong thing succeeded").

**Where step 2 now stops, and it needs the owner.** The live run left **one decision
unadjudicated**, so the selection gate cannot score:

> READING-UNADJUDICATED: «The MSI relationship compared favourably to other strong biomarkers for
> vulnerabilities.» chose a reading with no ledger verdict

The selected reading attaches `for vulnerabilities` to *the relationship*; the correct reading,
which the ranker placed first among the runners-up, attaches it inside the biomarker NP. Both use
identical concept sets — the sentence's 36 candidates reduce to 2 skeletons, so this was a binary
structural choice, and the ranker's rationale argues five sense choices that are the same on both
sides. Filed as **eigenius#264** (rework the ranker: prompt, rendering, and the model — TypeSafe.ai
models to be tested; the client is coupled to one provider today).

Adjudicating that decision is a ground-truth judgement and is the owner's, not the measurer's. With
it recorded as wrong (and 19 correct), the tally is 30 correct / 11 wrong / 0 unadjudicated —
matching `selection-baseline.json`'s committed 30.

**The ordered next steps below therefore resume at:** adjudicate that one decision → replay the
recorded draw and confirm it reproduces the live run exactly → write the new `baseline.json`. Then
step 3 onward as written, with step 5's provisioning already done.

## Where the branch stands

- **Base:** `c39b8d6`, the merge of #262 (exact numerics D94, units of measure D93) into `main`.
- **39 commits** since, from `7c700d7` (2026-09-25) to `074aea8` (2026-09-28). The last **15 are not
  pushed** (`6a3eabf` onward); push them before anything else. The working tree is clean.
- **Code:** no Rust or ESL file has changed since `6a3eabf`. At that commit all 55 kernel test
  binaries passed, as did the WordNet and UMLS importer tests, `cargo clippy --workspace
  --all-targets` with `-D warnings`, and `cargo fmt --check`. Everything after `6a3eabf` is design
  documents, experiment corpora and Python measurement scripts.
- **Lexicon store:** reseeded 2026-09-27 at `6a3eabf` into `../db-snapshot/wordnet-umls-2026-09-27`
  and `../db-snapshot/wordnet-umls-aligned-2026-09-27` (both gitignored, outside the repo). Older
  snapshots are gone; they no longer opened against this bootstrap manifest.
- **Parked branch:** `governed-prepositions` (`8a226ef`, pushed), linked from issue #263 (open). Its
  subject is now folded into D97.

## Read first

| Document | What it is |
|---|---|
| [`docs/design/d95-quantities-in-the-parser.md`](../design/d95-quantities-in-the-parser.md) | Quantities in the tokenizer and parser: the design |
| [`docs/design/d95-implementation-plan.md`](../design/d95-implementation-plan.md) | D95's slices, each marked built or owed, with what was measured |
| [`docs/design/d97-specialist-lexicon.md`](../design/d97-specialist-lexicon.md) | The SPECIALIST Lexicon as the lexicon's syntactic authority: proposed |
| [`docs/design/d98-qualifiers-as-operators.md`](../design/d98-qualifiers-as-operators.md) | Qualifiers (`not solely`, `partially`, …) as logical operators: proposed |
| [`experiments/publications/wrn-helicase/docs/06-opaque-predicates-to-propositions.md`](../../experiments/publications/wrn-helicase/docs/06-opaque-predicates-to-propositions.md) | Where the WRN paper states each result the hand-built chain encodes as an opaque predicate |
| [`experiments/publications/wrn-helicase/claims/README.md`](../../experiments/publications/wrn-helicase/claims/README.md) | The WRN claims in controlled English, measured |
| [`experiments/parsing/quantities/README.md`](../../experiments/parsing/quantities/README.md) | The quantity corpus, measured |
| [`docs/method/controlled-english-style-guide.md`](../method/controlled-english-style-guide.md) | The authoring register; its quantity rules are current as of this branch |

## What was done

### D95 — quantities in the parser (slices 1–5 built)

- **Tokenizer** (`kernel/src/dcg/lex.rs`, `preprocess.rs`, `quantity.rs`): a lossless lexer, then a
  preprocessor that owns every token decision — brackets (argument vs gloss), em-dash appositives,
  separators, edge trimming with operator and sign exceptions, commas, token kinds
  (`Word`/`Comma`/`Numeral`/`Quantity`/`NonProse`), and quantity recognition (`37 °C`, `931g`,
  `10 μg ml⁻¹`, `5 mg/kg`, `10%`). Unit spellings are `lexicon:UnitSurface` resources.
- **Grammar:** `cat_mp(unit, reading)` for a measure phrase, with `reading` a value or a difference
  (`units:Quantity(u)` / `units:Difference(u)`); `cat_unit_forall` for consumers of any unit, and the
  `UnitApply` combinator. `931g` seeds both readings of `g` (gram, standard gravity).
- **Consumers** (`closed-class.esl`, `ontology.esl`): `at`, `for`, `in`, `with` and `after` as verb
  adjuncts over a measured value; `of`, `with` and `at` as noun modifiers; a quantity before a noun
  or after the copula (`has_quantity`). `after` joined the closed class, with NP entries.
- **Tokenizer fixes found by the corpus:** an en-dash range (`4–12%`) is one non-prose token; digit
  groups before an attached unit (`1,000g`).
- **Sentence splitting:** no sentence ends inside a parenthesis; a `.` before a letter or digit is
  word-internal (`DepMap.org`); initialisms (`r.p.m.`). WRN texts: 401 → 356 segments, 0 unbalanced.
- **Numerals:** `1` seeds the singular cardinal (`one`), 2 and up the plural.
- **Corpus:** `experiments/parsing/quantities/` (28 sentences with a content-word fixture), checked
  without a database by `kernel/tests/quantity_corpus.rs`; also measured over the full lexicon.
- **The style guide's quantity rules** were rewritten from measurements: a quantity is not a noun
  phrase; write `931 RCF`, not `931 × g`.
- D93, D95 and D96 were corrected where the code contradicted them. `g` has two unit senses. The
  WordNet importer's counts were fixed.

### D97 — the SPECIALIST Lexicon (designed and measured; nothing built)

- The 2026 LEXICON file is at `references/specialist/LEXICON` (gitignored; free to use and
  redistribute with attribution — terms in D97).
- `experiments/lexicon-specialist/measure-specialist.py` compares it with the reseed's WordNet
  entries. Headline: 399 WordNet verbs gain an object (`incubate`), 4,390 verbs WordNet lacks
  (`electroporate`), 744 PP verbs get a named preposition, 1,301 object + PP frames.
- **Decided (with the owner):**
  - 1: the senses the derivational pointers pick, and an LLM judge for the rest (999 verbs, 1,804
    items).
  - 2: SPECIALIST frames join WordNet's.
  - 3: every preposition SPECIALIST names gets a `lexicon:Prep` constructor and an argument entry
    (52 new).
  - 4: countability is the union of Wiktionary and SPECIALIST.
  - 5, in part: a verb only SPECIALIST has is a verb sense of the concept its nominalization names —
    UMLS first, else WordNet.
- Issue #263's governed-preposition problem is folded in.

### The WRN study: from opaque predicates to propositions

- The PMC author manuscript is fetched and pinned (`experiments/publications/wrn-helicase/data/`,
  `pmc_jats`/`pmc_text` in `sources.tsv`) and rendered to citable text by
  `extract/jats_to_text.py`. Unlike the OCR texts, it has all 14 figure legends.
- `docs/06-opaque-predicates-to-propositions.md` maps each of the 27 predicates the chain's
  conclusions use to the sentence that states it and the text that defines its terms.
- `claims/corpus.tsv`: 62 controlled-English claims, following the style guide, measured cap-only
  over the full lexicon. 25 have a reviewed reading pinned; 14 drop a qualifier; 13 miss a name; the
  rest are recorded with their cause.

### D98 — qualifiers as logical operators (designed; nothing built)

Every adverb is transparent today, so `p53 activity is not solely responsible for WRN dependence`
parses as `…is not responsible…`. D98 states each qualifier's exact form from the paper's own
sentences (`not solely P` = `P(a) ∧ ∃y. P(y) ∧ ¬(y = a)`), and found that ESL cannot write
equality: the kernel has `Id`, but the term encoder never produces it.

## Next steps, in order

1. **Push** the 15 unpushed commits.
2. **Finish D95 slice 5**, which needs the owner:
   - re-record the sense ranks and selections against the new snapshot (live LLM calls, so budget
     and an `ANTHROPIC_API_KEY`); sentence splitting changed, so recorded draws keyed by sentence
     text will miss;
   - the parse-rate run on the CNL page and a new `experiments/parsing/baseline.json`. The page
     (`references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v3.txt`) is not on this
     machine; obtain it first.
3. **Get the open decisions from the owner.**
   - D98: 1 (how a presupposition survives negation), 2 (a verb's degree), 3 (the contrast class of
     `selectively`), 4 (where `significantly` and `independently` live).
   - D97: 5 (the 3,790 SPECIALIST-only verbs with no concept), 6 (whether a governed preposition
     reaches the meaning), 7 (decision 1 for adjectives and nouns), 8 (what a "no sense fits" verdict
     leads to), 9 (SPECIALIST's facts for UMLS entries), 10 (the closed-class list for the new
     prepositions).
4. **D98 slice 1: equality in ESL** — a surface form that encodes to the kernel's `Id`. Needs no
   decision, and blocks every exclusive. Then slice 2 (exclusives), once decision 1 is taken.
5. **D97 slice 1:** a provisioning script for SPECIALIST (pinned release, checksum), a reader, the
   sense judge (validated first on WordNet's own frames, verdicts committed), the importer behind a
   flag, and a reseed measured against the 2026-09-27 snapshot.
6. **The WRN claims corpus:**
   - gate its pins in the parse-rate harness, which reads only
     `experiments/parsing/expected-readings.tsv` today;
   - re-run it with the reranker once step 2 is done.
7. **D95 slices 6 and 7** (arguments and standards; degree semantics). Slice 7's degrees overlap
   D98 slice 3 (`fully`, `partially`): design them together.

## Known defects, found and not yet fixed

| Defect | Recorded in | Goes with |
|---|---|---|
| Qualifying adverbs are transparent; four WRN claims change meaning | D98 | D98 |
| ESL cannot write equality | D98 | D98 slice 1 |
| A name introduced in its own sentence is not grounded (`sgWRN-EIJ`, `shWRN1`, `γH2AX`); the style guide tells authors to do exactly that | claims README | not assigned |
| WordNet has `incubate` as intransitive only; no verb `electroporate` | D97; quantity README | D97 |
| The WordNet importer reads frame 13 (`----s on something`) as transitive | D97 | D97 slice 2 |
| In a cap-only run, a lower-ranked right sense never enters if a wrong reading parses (the sense cap widens only on no parse): `increased` | claims README | the reranked run |
| A hyphenated prenominal measure (`an 8-day viability assay`) is a missing lexeme | docs/06; claims README | not in any D95 slice |
| `all` has no collective reading; relative `where` and `whose`; `expose X as Y`; `greater colocalization … than` | claims README | grammar; the last one D97 decision 4 |
| A governed preposition does not reach the meaning (`contributes to`) | D97 decision 6 | D97 |
| `Project DRIVE` reads as `a project drive` where `Project Achilles` reads as a name | claims README | not assigned |
| The expected-readings pin for `Synthetic lethality is an interaction between two genetic events.` misses in cap-only runs | claims README | re-check after step 2 |

## Working on this machine

- **Disk is the constraint.** `cargo test --workspace` fills the volume and fails as a link error.
  Run `cargo test -p <crate>`, one crate at a time. A kernel-only change can still break other crates
  (a new enum variant broke `eigenius-lean`), so also run `cargo build --workspace`.
- **Any bootstrap ontology edit** moves the manifest: update `EXPECTED` in
  `kernel/tests/bootstrap_manifest_pinned.rs` in the same commit, then reseed.
- **Reseeding:** `scripts/reseed-lexicon-db.sh --umls-all`, then `scripts/build-alignment-snapshot.sh`.
  Run with `MALLOC_ARENA_MAX=1`, rust-analyzer stopped and `docker compose down` first. Provision
  countability first (`scripts/provision-countability.sh`). The kernel peaks near 12.9 GiB. On
  2026-09-27 the 28 UMLS chunks took about 2 minutes each; a long gap in the log is usually the
  machine sleeping, not a stall.
- **Read exit codes, not piped output.** Redirect to a log and `echo "EXIT: $?"`; a `| tail` reports
  the last command's status. The reseed once reported 0 with a failed build this way.
- **Measuring parses:** `scripts/measure-parse-rate.sh --no-llm --page <abs path>` runs cap-only
  against the newest snapshot. Its exit code 2 means "differs from the committed CNL-page baseline",
  which every other page does. `EIGENIUS_GLOSS_READINGS=1 EIGENIUS_GLOSS_MAX=200` prints each
  sentence's distinct skeletons with a gloss, which is how the corpus pins were reviewed.
- **Gitignored inputs:**

  | Input | Location | How to get it |
  |---|---|---|
  | WordNet 3.0 | `references/WordNet-3.0/` | `scripts/provision-wordnet.sh` |
  | UMLS 2026AA (licensed) | `references/umls/2026AA/META/` | your own UTS licence, `scripts/provision-umls.sh extract` |
  | Wiktionary uncountables | `references/wiktionary/` | `scripts/provision-countability.sh` |
  | SPECIALIST 2026 | `references/specialist/LEXICON` | `curl` from NLM (D97 gives the URL); D97 slice 1 scripts it |
  | WRN paper, PMC JATS and text | `experiments/publications/wrn-helicase/data/slices/` | `data/fetch.sh` |
  | WRN paper, Nature OCR | `references/publications/WRN-Helicase-Nature-OCR/` | methods and letter present; the CNL page missing |
