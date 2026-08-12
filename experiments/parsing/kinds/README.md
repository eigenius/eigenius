# Kind-classifier draws (D68 §4)

Recorded verdicts of the discourse-kind classifier — which `enc:` kind class each unmarked
declarative's claim carries (`kinds.json` format: `[{sentence, gloss, kinds}]`). The
ranks/selections/proposals sibling: record a draw with the close-out harness's
`EIGENIUS_KINDS=<new-file>` arm (live, `--features use-llm`), replay it by pointing
`EIGENIUS_KINDS` at the recorded file; a replay miss lands the claim `enc:Assertion`
(unreferable, fail-closed) and is counted — must be 0 on tracked replays. An empty `kinds`
array is a recorded abstention. Verdicts are model-adjudicated pending human sign-off, like
reading-adjudications.tsv.
