# Kind-classifier draws (D68 §4)

Recorded verdicts of the discourse-kind classifier — which `enc:` kind class each unmarked
declarative's claim carries (`kinds.json` format: `[{sentence, gloss, kinds}]`). The
ranks/selections/proposals sibling: record a draw with the close-out harness's
`EIGENIUS_KINDS=<new-file>` arm (live, `--features use-llm`), replay it by pointing
`EIGENIUS_KINDS` at the recorded file; a replay miss lands the claim `enc:Assertion`
(unreferable, fail-closed) and is counted — must be 0 on tracked replays. An empty `kinds`
array is a recorded abstention. Each verdict also carries the classifier's **`rationale`** — one
sentence saying why, including for abstentions, since an unmarked claim is unreferable and the
reason it stayed unmarked is the reviewable part. Draws recorded before the field exists replay
unchanged (`rationale` is optional; the replay key is sentence+gloss). Verdicts are model-adjudicated pending human sign-off, like
reading-adjudications.tsv.

Every path in the harness environment must be ABSOLUTE (the test binary's CWD is the crate
dir); the page defaults to the CNL rewrite, which is what the tracked numbers are measured on:

```bash
EIGENIUS_DB_SNAPSHOT=/abs/db-snapshot/wordnet-umls-aligned-2026-08-12-d67 \
EIGENIUS_SENSE_RANKS=/abs/experiments/parsing/ranks/2026-07-29-demonstratives.json \
EIGENIUS_KINDS=/abs/experiments/parsing/kinds/2026-08-12-reference.json \
  cargo test --release -p eigenius-wordnet --test db_backed_encoding \
  resolve_document_discourse_close_out -- --ignored --nocapture
```

Expected on `2026-08-12-reference.json` (NO reading ranker — the resolver floor): encoded 12 /
ambiguous 40 / open 10 / gap 0 over 62 units; kind tally 2 Finding, 4 Observation, 3
Classification, 1 Suggestion, 2 Assertion; ranks replay 62/0, kind replay 12/0.

Adding `EIGENIUS_SELECTIONS=/abs/experiments/parsing/selections/2026-08-12-d67-discourse.json`
runs the **composed** configuration — the reading ranker inside the discourse loop, which is the
pipeline as designed. Expected: encoded 50 / ambiguous 1 / open 11 / gap 0; 49 claims landing 14
Finding, 13 Observation, 5 Classification, 3 Hypothesis, 2 Suggestion, 12 Assertion; replays
62/0 ranks, 39/0 selections, 47/0 kinds. Pair it with
`2026-08-12-d67-discourse.json` — a kinds draw recorded against the no-ranker run does NOT cover
this one (it asks 47 questions where the floor asks 12, and every miss lands Assertion).
