# UAB XIAP parse work — handoff to another machine

Written `2026-10-18` for whoever resumes this on a different machine, in a session with no memory of
the one that produced it.

**For WHAT the work is and where it stands, read [work-stack.md](work-stack.md) entry 7 and
[uab-xiap-grammar-gaps.md](uab-xiap-grammar-gaps.md).** They are tracked, so they travel with the
branch and are current. This note covers only what those cannot: **the artifacts that do NOT travel,
and how to stand the environment back up.**

## Read this first: most of the work product is not in git

The standing rule is **no UAB data into git**, including derived encodings. That rule is correct and
stays. Its consequence for a machine move is that cloning the branch gives you the *code* and the
*findings* but none of the *corpus*, and some of what is missing is hand-authored and cannot be
regenerated.

| artifact | where | travels? | if lost |
|---|---|---|---|
| kernel/grammar changes, ESL edits, probes, scripts | the branch | **yes** | — |
| the method and findings docs | the branch | **yes** | — |
| **the CNL rewrite** `experiments/parsing/derived/step5/c0*.py` | local, gitignored | **NO** | **hand-authored; unrecoverable** |
| the document model `.../step5/docmodel-page.txt`, `document-model.json`, `xiap-document-model.esl` | local, gitignored | **NO** | re-derivable from the source, with effort |
| the claim-span corpus `.../step5/claim-spans*.{txt,tsv}` | local, gitignored | **NO** | re-derivable (script below) |
| D71 claim-landing artifacts `experiments/parsing/encoded/xiap/` (`.esl`, `.eigon.json`, `selections.json`, `proposals.json`, `kinds.json`, `reading-adjudications.tsv`) | local, gitignored | **NO** | regenerable (~35 min via `prose-to-esl` on the snapshot) |
| UAB source material `experiments/uab/` | local, gitignored | **NO** | re-obtain from the UAB Round 1 delivery |
| the DB snapshot (3.8 GB) | `db-snapshot/`, outside the repo | **NO** | rebuildable, ~30 min + chain |
| measurement logs (14 MB) | `reseed-logs/`, outside the repo | **NO** | only history; re-runs regenerate |

**The one irreplaceable item is `c001.py` … `c012.py`.** Those twelve files are the CNL rewrite of all
325 source units — roughly a day of authoring judgement, each sentence written to satisfy R1/R2/R3
and the DON'T filter. Nothing regenerates them. **Copy them out of band before you wipe this machine**
(scp/rsync to the target, or any private channel; not git, not a public host).

Copy the whole directory and you get everything derived in one move:

```bash
rsync -av <this-machine>:~/src/eigenius/eigenius/experiments/parsing/derived/step5/ \
          ~/src/eigenius/eigenius/experiments/parsing/derived/step5/
rsync -av <this-machine>:~/src/eigenius/eigenius/experiments/uab/ \
          ~/src/eigenius/eigenius/experiments/uab/
```

## Branch state

```
branch  uab-xiap-claim-landing
base    main (with uab-step6-encoding-verification merged)
```

**The repository was REPLACED with a cleaned version on `2026-10-19`.** The old history — 345
commits, and every branch other than `main` and this one — is gone from the remote. This branch was
rebuilt on the new root and carries all of the work; `main` is the cleaned base.

Local and `origin/uab-step5-document-model` are identical as of `2026-10-20`, so **no force-push is
needed** and nothing here diverges from the remote.

**The cleaning strips references that no longer resolve** — commit SHAs and GitHub issue numbers —
because a fresh root makes them dangling. Two consequences when writing notes in this repo:

- **Do not cite a commit SHA or a `#NNN` issue number**; name the change instead ("the short-passive
  participial lift", "the LLM-component removal"). Anything already written that cites one was
  rewritten by the cleaning, and this note was among them.
- A measurement's provenance is the STORE NAME plus its three reproduce counts, never a commit SHA.

**The pre-clean history survives in exactly one place**, a verified self-contained bundle:

```
/home/hm/src/eigenius/history-backup/eigenius-pre-clean-2026-10-20.bundle    (32 MB, 20 refs)
git clone eigenius-pre-clean-2026-10-20.bundle old-history     # to read it
```

It is outside the repo and therefore does not travel — copy it with the rest if the old history
matters to you. The local branch `pre-clean-history-2026-10-20` and tag `pre-clean-2026-10-20` point
at the same thing while this working copy lives.

**SUPERSEDED `2026-11-08`: the live store is `wordnet-umls-hpo-aligned-2026-11-07-systematic`**
(reseeded via `scripts/reseed-chain.sh`, all three provenance counts reproduced and gated — `38,391`
merge rows, `20,533` HPO resources, `864` equivalences). Earlier stores (`2026-10-26-basic`,
`2026-10-13-fa`, and other historical snapshots) were superseded as bootstrap layers and WordNet verb
frame / closed-class declarative entries evolved. Read the rest of this section as the record of why
`-fa` died, not as instructions.

The LLM-component removal edited four bootstrap ontologies (`core-ontology.json`,
`program-ontology.json`, `program-traces.json`, `prov.esl`), and the repository cleaning moved them
again. A store is stamped with the bootstrap manifest and refuses to open when it drifts, so
`wordnet-umls-hpo-aligned-2026-10-13-fa` failed with `BootstrapError::ManifestDrift` on exactly those
four layers — every other layer, ours included, still matched. There is no migration: the kernel
notes that a compatibility arm for this "was written and removed", and that "the reseed that answers
it rewrites the chain from source".

An earlier revision of this note claimed the recorded measurements stayed reproducible on the new
base. **That was wrong.** It checked that the LLM removal left the harness CODE alone, which it did,
and failed to check the bootstrap ONTOLOGIES, which is what a snapshot is pinned to. The numbers in
the notes remain correct as measurements — they were taken on a store that matched the code at the
time — but reproducing or extending any of them needs a rebuilt store first.

The LLM removal itself still does not disturb the parse path: `use-llm` is declared in `cli`,
`eigenius-wordnet`, `eigenius-encoding` and `lexicon-align`, and nothing under
`crates/eigenius-wordnet`, `kernel/src/dcg` or `scripts/measure-parse-rate.sh` was touched, so the
cap-only `--no-llm` arm is unaffected once a current store exists.

## Standing the environment back up

**The snapshot is required for every measurement.** Name:
`wordnet-umls-hpo-aligned-2026-10-26-basic`, 3.7 GB, in `db-snapshot/` beside the repo, with the raw
base `wordnet-umls-2026-10-26-basic` (3.5 GB) beside it.

Either copy it (fastest, one `rsync`), or rebuild it. Rebuilding is step 1 (`reseed-lexicon-db.sh`,
**~30 minutes** — not the ~3 h older notes assumed) followed by the four-step chain, whose recipe is
**`scripts/reseed-chain.sh <date-tag>`**, which runs all four steps and CHECKS the three
provenance counts rather than printing them — 38,391 merge rows, 20,533 HPO resources, 864
equivalences — exiting non-zero on a mismatch, because a differing count means an input changed and
measurements are not comparable. (It was `reseed-logs/chain.sh`, which lives OUTSIDE the repo and
so is absent on a fresh checkout; the recipe was reconstructed from prose twice in one week before
being tracked on 2026-10-28.)

Match the baseline on **every** axis, UMLS scope included (`--umls-all`) — the reseed default is a
subset and the mismatch is silent.

Measurement command, the arms every number in the notes was taken with:

```bash
scripts/measure-parse-rate.sh --page <corpus> --no-llm --units-per-line \
  --snapshot <db-snapshot>/wordnet-umls-hpo-aligned-2026-10-26-basic
```

The full 1,056-sentence CNL corpus takes roughly an hour; the 325-unit page about 15 minutes. Run
them in the background.

## Where the six-step order stands

The owner's order for survey §4 step 5. All six steps are done (`2026-11-10`).

1. ~~Document model of the XIAP report~~ — done.
2. ~~Close the OOV gaps~~ — done; `missing-lexeme` is **0** on both the page and all 427 claim spans.
3. ~~Measure parse success and gaps~~ — done, and re-run per-kind over the 427 gold spans as the
   survey actually specifies.
4. ~~Apply a CNL rewrite~~ — done. 325 units → **1,056 sentences** (`275` active prose units).
5. ~~Iterate to close the encoding gap~~ — done (`2026-11-08`, store
   `wordnet-umls-hpo-aligned-2026-11-07-systematic`, `38,391 / 20,533 / 864`). **1,056 / 1,056
   sentences parsed (100.0%, `grammar-gap 0`, `missing-lexeme 0`)**, isolated-sentence split
   `encoded 19`, `ambiguous 783`, `open 254`, and **89 / 89** on the core-en construction battery.
6. ~~Verify each encoding via `verbalize` against the original prose~~ — done (`2026-11-10`, same
   store). Document-level discourse resolution (`resolve_document_discourse_close_out` with
   `EIGENIUS_WRN_UNITS_PER_LINE=1` and `EIGENIUS_VERBALIZE_OUT=<tsv>`) closes all **254** isolated
   `Open` units across all **1,056** sentences (`open 0`, `gap 0`; `387` units with resolved
   discourse bindings, `20` 1-to-1 line alignments synced back into `c001.py`–`c012.py`), and
   `verbalize` (`Register::Surface`) renders all `1,056` top readings with **0** bracketed fallback
   spans (`540 → 0`), **0** `not (⟦False⟧)` artifacts (`150 → 0`), and **0** unfreshened
   `anaphor_of` terms (`46 → 0`). End-to-end deterministic (`--no-llm`) D71 claim landing via
   `prose-to-esl` (`CostFirstReadingRanker` + `RecencyProposer` + `DerivedClaimLander`) emits **`5,705`**
   validated resources (`958 / 1,056` `enc:EncodedClaim`s, `396` `enc:AnaphorBinding`s, `98`
   `enc:CutItem`s, `0` kernel `Validator` errors) into gitignored `experiments/parsing/encoded/xiap/`,
   and a `50`-claim stratified semantic audit establishes that contextual `SenseRanker` + Stage-1
   `ReadingRanker` reranking is required for faithful proposition selection (`16%` strict / `28%`
   charitable fidelity under cap-only + minimum-cost selection). See
   [uab-xiap-encoding-plan.md](uab-xiap-encoding-plan.md) for the full Step 6 and D71 landing summary.

## What step 5 should know before it starts

**There is no large lever.** Fixing the two confirmed blockers *completely* — the decimal numeral and
the `Å` token — caps at **47 of 275 units fully covered (17.1%)**, up from 37. **228 of 275 units are
blocked by a diffuse tail.** Plan for a long list of small fixes, not a breakthrough.

Confirmed by A/B, one variable changed and nothing else:

- **A bare decimal numeral does not parse.** DECIMAL 0/15 vs INTEGER 7/15. A D95 quantity gap. It
  falls hardest on the methods and predictor-score material, where nearly every claim carries a
  number.
- **`Å` is unseedable.** All 7 remaining `non-prose` sentences are Ångström contacts. A D93 gap.

Refuted by the same method, so do not spend time on it: **mid-sentence transitional adverbs**. The
shape scored +15.3pp over the base rate across 97 sentences and looked like a work order; moving the
adverb to the licensed sentence-initial position gave MID 1/14 against FRONT 1/14 — identical.

## Traps that cost time here

- **A pattern's count in the failing set is not evidence.** Divide by the base failure rate first,
  then check what fraction of failures carry *none* of the candidates. On the raw page every
  mechanically-countable DON'T row sat at the 86% base rate and 46% of gap units carried none of
  them. Even a real lift is only a candidate until a one-variable A/B confirms it.
- **An A/B corpus feeds the recurrence-based named-entity miner.** The glossary augmentation is built
  per *document* and mints a named entity only on recurrence; an A/B holds every sentence twice, once
  per arm, so its content words recur by construction. A 6-token sentence was `GRAMMAR-GAP` in the
  1,056-unit run and `AMBIG` in a 28-unit A/B for exactly that reason. Both arms share the distortion,
  so the **contrast** is valid — the **absolute rate** is not, and neither is a premise carried in
  from the document's context. Check the `augmentation:` and `named entities:` log lines before
  comparing any two runs.
- **A parse is not a correct parse.** Three probes counted as covered were a compound-noun pile and
  an adjective-degree reading. Print the sem and compare it against a control that must mean the same
  thing; a reading count alone passes all three.
- **A glossary source must mint entries under the surface the parser asks for.** Re-deriving the
  tokenizer drifts — that defect appeared four separate times.

## Loose end

The reading-count disagreement between runs is explained (the A/B augmentation trap above), but it
has a consequence nobody has sized: because augmentation is document-scoped, **the gap count on a
large corpus may be slightly pessimistic**, since a sentence can fail in the full document and parse
in a smaller one. Nothing in the recorded numbers depends on it — page and CNL were each measured as
whole documents, which is the intended reading — but a per-sentence claim lifted out of either run
carries that caveat.
