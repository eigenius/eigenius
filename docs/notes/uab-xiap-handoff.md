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
branch  uab-step5-document-model
base    the cleaned root commit "Eigenius" (2026-10-18)
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

The LLM-component removal (folded into the cleaned base) took out the orchestration/TypeScript
components — `CompleteText`, `CompleteJson`, the `ai` SDK pin — **not** the Rust reranker. `use-llm`
is still declared in `cli`, `eigenius-wordnet`, `eigenius-encoding` and `lexicon-align`, and nothing
under `crates/eigenius-wordnet`, `kernel/src/dcg` or `scripts/measure-parse-rate.sh` was touched —
so every measurement recorded in the notes remains reproducible, and the cap-only `--no-llm` arm is
unaffected.

## Standing the environment back up

**The snapshot is required for every measurement.** Name:
`wordnet-umls-hpo-aligned-2026-10-13-fa`, 3.8 GB, in `db-snapshot/` beside the repo.

Either copy it (fastest, one `rsync`), or rebuild it. Rebuilding is step 1 (`reseed-lexicon-db.sh`,
**~30 minutes** — not the ~3 h older notes assumed) followed by the four-step chain, whose recipe is
preserved in `reseed-logs/chain.sh`: `build-alignment-snapshot.sh` (expect **38,391** merges) →
`obograph-import` on `references/hpo/hp.json` → `add-layer-to-snapshot.sh` (expect **20,533** HPO
resources) → `build-hpo-alignment-snapshot.sh` (expect **864** equivalences). All four scripts are
tracked. Those three counts are the provenance check: if they differ, the snapshot is not the one
the recorded numbers were taken on.

Match the baseline on **every** axis, UMLS scope included (`--umls-all`) — the reseed default is a
subset and the mismatch is silent.

Measurement command, the arms every number in the notes was taken with:

```bash
scripts/measure-parse-rate.sh --page <corpus> --no-llm --units-per-line \
  --snapshot <db-snapshot>/wordnet-umls-hpo-aligned-2026-10-13-fa
```

The full 1,056-sentence CNL corpus takes roughly an hour; the 325-unit page about 15 minutes. Run
them in the background.

## Where the six-step order stands

The owner's order for survey §4 step 5. Steps 1–4 are done.

1. ~~Document model of the XIAP report~~ — done.
2. ~~Close the OOV gaps~~ — done; `missing-lexeme` is **0** on both the page and all 427 claim spans.
3. ~~Measure parse success and gaps~~ — done, and re-run per-kind over the 427 gold spans as the
   survey actually specifies.
4. ~~Apply a CNL rewrite~~ — done. 325 units → **1,056 sentences**, and the result is **2×**:
   source units fully covered go 19/298 (6.4%) → **37/275 (13.5%)**.
5. **Iterate to close the encoding gap** — NEXT, and read the warning below before planning it.
6. **Verify each encoding via `verbalize`** against the original prose. `cnl-map.tsv` maps every
   source unit to its output line range; step 6 needs it, which is another reason not to lose it.

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
