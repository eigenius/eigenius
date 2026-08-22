# prose-to-formulas **v2** — the composed pipeline

> **v1 is RETIRED (`2026-08-17`) — this is the maintained demo.** v1 selected by skeleton pin, which
> is sense-erased and cannot break a tie between readings differing only in sense, so it broke on any
> lexicon change that added a sense to its paragraph — twice. This one selects with the reading
> ranker and survives inventory changes. v1's files were deleted; only its README remains as the
> record. The in-process D67 §3.5 acceptance test (`crates/eigenius-encoding/tests/acceptance.rs`)
> now reads THIS demo's artifacts.

Same argument as [v1](../prose-to-formulas/) (retired, README only), produced end to end by the pipeline
[D71](../../docs/design/d71-document-formalization-service.md) §3 contracts: **glossary → parse →
sense rank → reading selection → anaphora resolution → claim landing → artifact → commit**. v1
stops after selection-by-human-pin; this runs the whole thing.

```bash
docker compose build kernel                       # once, after any kernel change
EIGENIUS_DB_SNAPSHOT=/path/to/wordnet-umls-aligned-2026-08-12-d67 \
  demo/prose-to-formulas-v2/run.sh                # add --reparse to re-derive the claims
```

## What it shows that v1 does not

**No pins.** v1's `pins.tsv` declared which reading was correct and the run failed closed if the
pin matched anything but one reading. Here the **reading ranker** chooses, in document context,
and its choice is committed as an `enc:DecisionPoint` carrying the ranker's own rationale. It
landed the same term the human pin did — `claim_1`'s proposition is byte-identical across the two
demos — so the pin became a check that was passed rather than the mechanism.

**Anaphora, resolved against claims landed in the same run.** The third sentence refers back:

> These findings show that WRN is a vulnerability of MSI cancer models.

«These findings» is not a definite description; it is a **restrictor-typed hole** (D64). It
resolves against `enc:EncodedClaim`s the lander committed for the *earlier sentences of this same
document*, and the accepted binding is on the chain as an `enc:AnaphorBinding` naming the
antecedent by IRI, with the proposer's confidence and reasoning. The resulting claim's formula
**contains the antecedent claim as a term**:

```
v00664788_c(is_a(WRN, (Σx0:n14543931. of(x0, (Σx1:C1516211. MSI-kind(x1))))), claim_2)
```

**The discourse-kind axis, visibly doing work.** Each landed claim carries its kind as a second
`is_a` class (D68) — here `claim_1` is an `enc:Observation` and `claim_2` an `enc:Finding`. The
anaphor's restrictor is checked against that axis by the **kernel**, not the model: the
Observation is not eligible for «these findings», so the proposer had exactly one candidate to
rank. Its recorded rationale says so in as many words. The model proposes; the kernel vetoes.

## Determinism

Four recorded draws, one per LLM stage — `ranks*.json` (sense ranking), `selections*.json`
(reading selection), `proposals*.json` (anaphora), `kinds*.json` (discourse kind). `--reparse`
replays all four: no LLM, no network, no key, and both artifacts regenerate **byte-identically**.
A key MISS in any stage fails closed rather than silently degrading — a missed rank falls back to
cap-only, a missed selection abstains, a missed kind lands `enc:Assertion` (unreferable).

Each kind verdict records the classifier's one-sentence reasoning, including for abstentions,
because these verdicts are model judgments pending human sign-off.

## The tamper

`paragraph-edited.txt` negates the measurement. `inference.esl` — unchanged, the same file that
committed on the intact branch — cites `claim_1` directly for its antecedent. The witness key
hashes the *proposition*; the edited sentence parses to a different term, so there is no witness
under that key and the load is **rejected at the gate**. The asserted route (sentence 2 states the
conclusion) survives; the derived one does not.

**One token, one line.** The two variants are parsed and selected **independently** — no shared
draw, no pin — and they still land the same term apart from the negation: the edited `claim_1` is
the intact proposition with a trailing `-> logic:False`, byte-identical otherwise.

That did not hold when this demo was first built. The negated sentence's 120-reading pool
rendered to **4 distinct strings**, so the ranker could not see the difference between
«exonuclease activity» as the single GO concept C1148824 and as an `activity ⊗ exonuclease`
compound, and it picked a compound reading. The expanded rendering register
([D69](../../docs/notes/d69-reading-presentation.md)) shows concept identities and definitions,
and with it the ranker picks the reading a human had pinned — on both variants, independently.

## Files

| file | generated? | what it is |
|---|---|---|
| `paragraph.txt` / `paragraph-edited.txt` | — | three sentences; the edit negates the first |
| `ranks*.json`, `selections*.json`, `proposals*.json`, `kinds*.json` | recorded once each | the four LLM stages' draws, replayed |
| `onco-typed.esl` | — | domain predicates DEFINED over the parser's lexicon (copied from v1) |
| `literature-rules.esl` | — | the pinned `∀m. A → B` — the only DeclarationTrace on the branch |
| `claims-intact.esl` / `claims-edited.esl` | `prose-to-esl` | the artifact: units, claims (with kinds), DeclarationTraces, DecisionPoints, AnaphorBindings, and one run-level ProgramTrace on the `enc:ReasoningStructure` |
| `inference.esl` | hand-authored | the recorded derivation citing `claim_1` |

`--chain-load` puts `encoding.esl` + `claim-kind-alignment.esl` on the parse's own chain: the
restrictor veto needs the kind classes and their lexicon alignment, and those are not in the
lexicon snapshot.

## Status

v2 is intended to **replace** v1 once reviewed. v1 remains for now as the pinned-selection
reference; retiring it means deleting `demo/prose-to-formulas/` and repointing the docs that cite
it.
