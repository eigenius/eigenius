# DCG parser — status and next steps (2026-07-20)

## Where we are

We're reducing spurious ambiguity in the DCG parser, measured on the first page of the
WRN-helicase paper. Two rules stand above the rest: **coverage is non-negotiable** (every
sentence must parse — grammar-gap 0), and, set this session, **correctness comes before
ambiguity** — a sentence closing on the *right* reading matters more than a low reading count.

## What we did this session

We found and fixed a bug in the sense reranker (the LLM step that chooses which meaning each
word takes). Adjective senses were being shown to the model as a meaningless placeholder
("grammatical function-word reading") instead of their real dictionary definitions — the code
that fetches a definition only looked at the top of the term and missed the definition, which
sits one level deeper for gradable adjectives. With no real definition, and a prompt that says
"omit function words," the model dropped the adjectives and picked medical-qualifier concepts
instead. So "specific" and "stronger" were read as noun-qualifiers, not adjectives, and the
sentences closed on the wrong reading.

The fix teaches the definition lookup to walk into the term and find the gloss. It's
parser-only — no database change, because the definition was already stored. Afterwards the
reranker ranks the adjectives first and the affected sentences close on the correct reading.
Committed as **b757274**, with a variance-checked re-baseline (5 measurement draws; coverage
held in all 5).

One expected side effect: restoring the correct adjective readings *raises* the ambiguity
count, because the broken reranker had been hiding ambiguity by wrongly discarding adjectives.
That's an acceptable trade under "correctness first."

## What we found in the two follow-ups

1. **One sentence explodes to 170 readings** ("Many cancers exhibit an impairment of a DNA
   repair pathway"). This is *benign*: all 170 mean the same thing, differing only in how the
   noun pile "DNA repair pathway" and the "of" phrase bracket, and the intended reading is
   among them. About **half** of its apparent structural count is a **measurement artifact** —
   "cancer" comes from two dictionaries (WordNet and UMLS) that were merged in *meaning* but
   not in their *type label*, so the same reading gets counted twice.

2. **The faithfulness measure is weak.** We gate on the count of single-reading ("encoded")
   sentences, but a sentence can be encoded on the *wrong* reading. The better approach is a
   small, growing set of targeted checks that assert a sentence *contains* the right reading
   (we already have one such check; we'd add more).

Both follow-ups are blocked by the same cross-dictionary labeling artifact: it inflates the
count and would confuse the targeted checks.

## Next steps

1. **Normalize the cross-dictionary artifact** — either a cheap fix in the measurement (ignore
   the dictionary-of-origin label when counting structures) or the deeper fix in the alignment
   (give a merged concept a single type label). The deeper fix also cuts real readings, not
   just the count.
2. **Add correctness "canary" checks**, starting with "specific is an adjective," to guard this
   session's fix from regressing.
3. **Then return to the real structural ambiguity** (the noun pile), now on a clean footing.

## Correction to the step-1 diagnosis (2026-07-20, continued)

Reproduced the x170 sentence deterministically (`trace_one_sentence`, cap-only, no LLM, aligned
snapshot `wordnet-umls-aligned-2026-07-17-chv`): **170 readings / 34 structural skeletons**. The
skeletons pair up — `skel[0..14]` (a slot typed `C§`, a UMLS CUI class) mirror `skel[15..29]`
(typed `n§`, a WordNet class), structurally identical otherwise. So ~half the skeleton count is a
cross-lexicon class-label split, as the note said. **But the mechanism is not what the note claimed.**

The raw IRIs (not sense-erased) show "cancers" (the head, `G#1`) seeds **four** senses: three
WordNet (`n01977832`, `n09752657`, `n14239918`) **plus one UMLS `C1547140`**; "impairment"
similarly seeds three WordNet plus `C0684336`. These C-class senses are **not merged-but-mislabeled
duplicates** — the alignment redefines *both* `cat` and `sem` to WordNet, so a real merge collapses
cleanly. They are **unmerged concepts the adjudicator marked `same:false` on purpose**
(`alignment.jsonl`):

- **`C1547140` "cancer"** — UMLS semantic type **T091 "Biomedical Occupation or Discipline"**,
  sourced from **HL7v2.5** with MTH preferred name **"Specialty Type - cancer"**. A metadata /
  administrative code (the oncology *specialty*), not the disease. **Junk that should not be a
  lexical noun sense at all** — the same class of collision the `drops.json` set exists for, except
  the current drop criterion only catches *case-mangled* atoms (`gENE`→`gene`), and `C1547140`'s
  atom "Cancer" is proper-cased, so it slips through.
- **`C0684336` "impairment"** — semantic type **T033 "Finding"**, "Impaired health", CHV/LNC/AOD
  sourced. A **legitimate distinct clinical sense**; the adjudicator correctly kept it separate. Its
  multiplicity is real sense ambiguity for the reranker to prune, not a lexicon defect.

**Consequences for step 1:**

1. The note's "give the merged concept a single type label" fix rests on a misdiagnosis — there is
   no merged-but-split concept here; there is an **unmerged metadata artifact**. The real structural
   fix is to **drop `C1547140`-class junk** (broaden the drop adjudication beyond case-mangling to
   metadata/administrative CUIs — T091 occupation/discipline + HL7-style specialty codes — that
   collide with a common word already WordNet-covered). Dropping the junk cuts real readings **and**
   deflates the skeleton count honestly (the `C§` skeletons disappear because no C-class cancer sense
   remains to fill the head). Needs a drop pass + reseed.
2. The "cheap measurement fix" (normalize `C§`/`n§` in `erase_senses`) is a **band-aid**: the
   skeleton metric is faithfully reporting real forest content — the junk sense genuinely produces a
   distinct term — so collapsing `C`/`n` would *hide* real junk rather than fix a miscount, and it
   risks conflating WordNet `n`/`v`/`a` POS classes. Rejected under "fix the structure, not the
   measurement."

**Step 2 is not actually blocked.** The correctness canaries assert a sentence *contains* the right
reading; extra junk senses don't confuse a presence check. Exemplars to model new canaries on:
`verify_attributive_comparative_at_scale` and `definite_negation_collapses_referential`
(snapshot-gated, parse-and-assert-a-reading).

## Outcome of the drop fix (measured, 2026-07-20 continued)

Implemented the structural fix: a second drop path in `crates/eigenius-lexicon-align/src/drops.rs`
(metadata-artefact concepts — curated HL7 code-table prefixes + SNOMED `(qualifier value)`/
`(attribute)`/`(qualifier)` tags), regenerated `drops.json` (17 → 275; `C1547140` caught; no
same-surface drop/merge conflict), reseeded, rebuilt the aligned snapshot
(`wordnet-umls-aligned-2026-07-20-metadrops`), measured.

**Coverage holds — grammar-gap 0, missing-lexeme 0** (the non-negotiable gate; 251 of 275 atoms
fired at import, corpus-wide, breaking no parse). The junk is gone: on the x170 sentence
`C1547140` no longer seeds, and the cross-dictionary `C§`/`n§` skeleton split is removed — **34 → 17
skeletons** (cap-only, deterministic), the ~half the note flagged. `C0684336` "Impaired health"
(a real `(finding)`) is correctly kept.

**But the full-page aggregate effect is ~neutral, and this is the real finding.** The drift-free
cap-only A/B (same code, old vs new snapshot) is total-readings **2304 → 2304 (flat)**, total-skeletons
**720 → 709 (−11)**, encoded 4 → 4. The single reranked draw (readings 1328→1266, skeletons 446→433,
encoded 8→9) sits **entirely inside the baseline's own drift bands** — drift, not signal. The cause is
**cap-backfill**: dropping a junk sense frees a `SENSE_CAP=2` slot and the parser refills it with the
next sense, so the per-sentence win does not aggregate. This is the same lesson the WordNet↔UMLS
alignment hit (README §"Result — measured, and negative"): removing a competing sense is *correct and
coverage-safe* but is **not the multiplicity lever** while the cap backfills.

**Verdict:** the drops are worth keeping — they remove genuine lexicon junk (`Specialty Type - cancer`
is not a noun), hold coverage, and close the README's named gap — but the aggregate multiplicity lever
is `SENSE_CAP`/backfill, not lexicon drops. The next real lever is either the cap-backfill discipline
(don't refill a freed slot with a lower-ranked sense) or the noun-pile structure (GH#97), not more
drops.

## Process note

These are not just edits — each needs me to run cycles of:

- **Builds** — Rust compiles, ~30 s to a minute each.
- **Measurements** over the WRN page. With the live reranker these are non-deterministic, so a
  trustworthy read needs several draws (~5 min each) plus a re-baseline.
- **A reseed** for the deeper alignment fix — rebuilding the 2.8 GB WordNet+UMLS snapshot from
  scratch (hours), after which the baseline must be re-measured.

So the measurement-side fixes can land quickly; the alignment fix is a multi-step,
reseed-and-re-baseline job.
