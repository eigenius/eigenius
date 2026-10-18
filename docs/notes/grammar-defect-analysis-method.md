# Method — analyzing and fixing grammar defects (WRN-page loop)

The mechanics (provision → seed → align → measure → replay, the traps, the ambiguity
decomposition and attribution instrument) are in `experiments/parsing/README.md`. The
epistemic discipline (witness, don't assert; fail closed; grade every claim) is in
`CLAUDE.md`. This note records the **defect-analysis loop** those two leave implicit —
distilled from the D63 Defect 1/2/3 work (2026-07-23/24).

## 1. Find the defect — bucket, then decompose

`total-skeletons` is the tracked structural lever, so start from the **skeleton
histogram** (units by distinct sense-erased skeleton count), not from reading counts:

```bash
grep -aoE "\[[0-9]+ skeleton" <run>.log | grep -aoE "[0-9]+" | sort -n | uniq -c
```

- **1-skeleton units** are the *pinning* frontier: unambiguous structure, so the only
  question is "is it the RIGHT structure". **Single-skeleton ≠ correct** — 3 of the first
  4 checked were wrong. Verify each by gloss (`EIGENIUS_GLOSS_READINGS=1`) or λ-term
  dissection before pinning.
- **2+-skeleton units** need *adjudication* (which reading is faithful), a per-unit
  correctness judgment, not a sweep.
- **High-skeleton outliers** carry the richest defect signal. Decompose the count into
  independent **axes** (e.g. 16 = 4 clause-structures × 4 compound-bracketings). Axes
  that multiply are separate defects; fix them separately.

## 2. Root-cause it — witness the parse space, don't infer it

Order matters: **evidence before claim**.

- `EIGENIUS_TRACE_FOREST=all` on one sentence shows the hyperedge tree — which rule
  built which cell, and **where a derivation dies** (an empty cell is the blocker). This
  is what found that the relational comparative broke at `than to MSI`, not at the
  adjective.
- `--attribution` ranks levers, but read the two halves differently (README §7a):
  sense sites are felicity-intersected and DO rank; **structure sites are RAW and rank
  nothing**. Never size a structural lever from raw branch counts.
- A defect is often *two coupled causes*, and fixing one alone gaps or does nothing:
  Defect 2a needed the plural `some` determiner **and** the UMLS withhold; Defect 1
  needed the frame **and** elided-`than`. Probe for the second cause before implementing.
- The correct reading may be **absent from the parse space entirely** (Defect 1, and the
  positive-relational gap). Then no amount of ranking or pruning helps — the grammar or
  lexicon must gain the reading first.

### 2a. Match the instrument to the question

Most wasted effort in this loop has been asking a question with the wrong tool, or with no
tool at all. The mapping:

| question | instrument |
| --- | --- |
| *Is this reading correct?* | **verbalizer** (`EIGENIUS_GLOSS_READINGS=1`) — renders meaning; says nothing about derivation |
| *Which rule built this constituent, from which operands, with what provenance?* | **forest trace** (`EIGENIUS_TRACE_FOREST=all`) — the ONLY view with `prov=` and `edges=Combine@k #L + #R` |
| *Where does multiplicity concentrate?* | `--attribution` — but its structural half is raw (README §7a), so not for causation |
| *Did my guard fire?* | a **unit test on the guard**, or the forest trace's `prov` field |
| *Did MY change cause this?* | a **worktree A/B** (§5a) |

Two rules that follow from it:

- **Stop after one wrong guess.** On the essive gap three consecutive hypotheses (the
  article, an un-reduced sem, the determiner) were each plausible, each cost a build-and-
  measure cycle, and each was wrong; the forest trace then answered it in one run. If the
  first hypothesis misses, switch from guessing to tracing.
- **A test that can only confirm is not a test.** "Does pruning MORE readings restore a
  MISSING one?" cannot discriminate — it reproduces the symptom whatever the cause. Before
  running an experiment, say what each outcome would rule out.

The verbalizer is unreasonably effective as a *first* filter: the sortal-name defect was
visible in one line of English — *"the mutation occur in **a nucleotide named a repeat
region**"* — long before the trace confirmed the rule.

## 3. Ground it — check the reference grammars BEFORE designing

`references/openccg/grammars/core-en/` (and `references/openccg/ccgbank`) are the
reference. This step has **reversed a planned fix twice**:

- `than`: CCGbank makes it an optional post-modifier — but an *interpreted* grammar
  cannot set a buried comparison standard from a wrapping modifier, so it must stay a
  complement (`cat_pp_than`) with an elision shift. The literal CCGbank category was wrong
  for us.
- argument/adjunct: core-en `pp.xsl` splits `Prep-Nom` (`PP/NP`, `*NoSem*` case-marker)
  from `Prep-Loc`/`n-postmod` (modifier) **lexically** — we already mirror that — and
  resolves the choice with a **supertagger**. Our sense-reranker ranks *senses*, not
  categories, so nothing in our pipeline plays that role; that is what justified a hard,
  narrowly-scoped prune instead of a soft cost.

- bare NPs: core-en's `bnp` type-changing rule is `n $1 → np $1` — a bare NP is a **plain
  np** — and core-en type-raises only `QuantNP`, because raising is for GENERALIZED
  QUANTIFIERS. Ours reused the existential determiner's raised categories, which fixes the
  result category and so cannot fill a non-final argument slot. **This step was skipped and
  the cost was direct**: an entire design (generalized composition) was proposed, argued
  for, and only abandoned after checking — composition cannot repair a raise whose result
  category is wrong, and the reference had the answer all along.

Conclusion pattern: *mirror the reference's lexical distinctions; where we lack its
disambiguation mechanism, say so explicitly and justify the substitute.*

## 4. Fix the structure — and scope the hard cuts

Per CLAUDE.md: eliminate the bad behaviour, don't guard against it. In practice here:

- Prefer **one general rule over per-word entries** (the `elided_than` unary shift
  replaced `more_deg_bare`/`less_deg_bare`; one `X/cat_pp_than` → `X` shift).
- A **hard prune of a licensed reading** is acceptable only when the eliminated reading is
  *always* invalid for the trigger class, and it must be **coverage-safe by construction**
  — lifted at the final widen rung (mirroring multiword-preference), so it can never
  become a grammar gap. Scope it to where that holds: `suppress_governed_adjunct` covers
  governed **adjectives** (no adjective has a competing adjunct use of its governed
  preposition) and defers **verbs** ("operate on the patient" vs "operate on Monday").
- Distinguish *importer-side* (by TUI / by surface, corpus-wide, needs a reseed) from
  *parse-time* (contextual, no reseed). A decision that depends on syntactic context must
  be parse-time — hard-coding it at import is the wrong shape.

### 4a. Before designing a rule, check the INVENTORY for a missing sibling

A defect that looks like a grammar gap is often a **lexical hole** — an entry present for one
surface form and absent for its sibling. Three now, in this order of discovery, all the same
shape:

| symptom | actual cause |
| --- | --- |
| plural "some X" reified as UMLS `C0205392` | `some` was singular-only |
| "is AN interaction between two events" attaches the PP to the SUBJECT | `a` had three entries (`a_pred`/`a_subj`/`a_obj`), `an` only two — no predicative allomorph |
| "WRN affects each gene." → **0 readings** | `each` had a subject entry and no object entry |

The `an_pred` case cost three wrong diagnoses before the inventory was checked: "PP cannot
attach to a non-subject nominal" (wrong — `pp_mod` fires and builds the refined `cat_n`),
"the copula lacks core-en's `NP` predicate-nominal entry" (wrong — we route predicate
nominals through the predicative article, deliberately), and "`DepApply` Fst-projects away
the refinement" (wrong — that branch only fires when `tvar` occurs in the body, which it
does not for `a_pred`). Each was plausible and each cost a build-and-trace cycle. **A
two-line inventory sweep found it.**

So, before designing any rule:

- **Group the closed-class entries by surface form and diff the sets.** Every determiner
  should carry a subj/obj pair; an article that has a predicative entry should have it for
  every allomorph. An asymmetry is a defect until proven otherwise.
- **Use a minimal pair that differs ONLY in the suspect word.** "…is *a* relationship
  between two events" vs "…is *an* interaction between two events" settled in one run what
  the traces had not. Likewise "each gene" vs "every gene".
- **A 0-reading probe is stronger evidence than a wrong-reading probe.** `each_obj` was a
  coverage hole invisible to the reference page — no unit exercises it — and only the sweep
  could surface it. Sweep the inventory even when the corpus looks clean.

## 5. Measure honestly — the comparison must be like-for-like

- **cap-only vs reranked are not comparable** (cap-only runs ~2× the readings). Comparing
  a `--no-llm` run against a reranked baseline produces a bogus "REGRESSION".
- The **drift-free number is the `--replay`**, not the live RECORD draw. Live draws drift
  ~5%; the baseline must be the replay.
- Anything the LLM decides that is **not recorded in `ranks.json`** diverges between
  RECORD and REPLAY. The abbreviation proposer is such a case (RECORD 747 vs REPLAY 678),
  so the baseline is the deterministic arm.
- Readings/skeletons **rising** is not automatically over-generation: restoring a correct
  reading costs ambiguity (correctness > ambiguity). Check whether the rise is *added
  correct readings* before treating it as a regression.
- `encoded` is informational, not gated — a unit can be encoded on the WRONG reading.
  The gate is **expected-reading hits** + `grammar-gap 0` (non-negotiable).

### 5a. Attribute a regression before explaining it — worktree A/B

When a measurement worsens, the first question is *did my change cause it*, and the answer
is cheap and non-destructive:

```bash
git worktree add /tmp/eig-base <pre-change-commit>
cd /tmp/eig-base && cargo build --release -p eigenius-wordnet --tests
ln -sfn <repo>/references references          # large data is not in git
EIGENIUS_DB_SNAPSHOT=<same snapshot> EIGENIUS_SENSE_RANKS=<same ranks.json> \
EIGENIUS_WRN_PAGE=<same page> cargo test --release … wrn_first_page_over_full_lexicon
```

**Same ranks, same snapshot, only the code differs.** This settled two faithfulness misses
that had been attributed to the wrong cause: one reproduced on the pre-change tree (so it
was reranker draw variance, not the change), the other did not (so it was). Guessing which
is which from the diff would have been wrong in both directions.

Also: check `git log -L <line>,<line>:experiments/parsing/expected-readings.tsv` for a
broken pin. The note recorded when it was pinned says what was actually VERIFIED — if the
verified property survives and only an unadjudicated detail moved, that is a **stale pin**,
not a lost reading.

### 5b. Removing junk can EXPOSE a gap — that is progress, not regression

`grammar-gap 0` can be **false comfort**: a unit may be "parsing" only through a junk
reading. Delete the junk and it gaps — which looks like a regression and is actually the
first honest measurement of that unit. It has happened twice (unit 4 via a UMLS reification,
the essive unit via WordNet's `as`=arsenic noun). The response is to close the gap properly,
not to restore the junk; land the junk-removal and the gap-closure **together** so the gate
is never green on a lie.

### 5b′. A metric can improve because a BUDGET truncated the evidence

Before blaming a rule for a lost reading, check whether the reading was ever *classified*.
The parse spends two bounded budgets — `kbest`'s per-node `k` (`DEFAULT_FOREST_CAP`, 256) and
`CLASSIFY_BUDGET` (256 felicity evals) — and both truncate a **cost-sorted** list. A correct
reading that costs more than the flat alternatives is dropped, the unit still parses, and no
coverage metric can see it.

Witnessed 2026-07-25. `bnp` gave a bare kind a plain `cat_np`; every kind-argument reading
then had a second derivation with an identical sem. "We also identified MSI cell lines from
rare lineages." went to **376 candidates carrying 44 distinct sems** — 88% duplicate mass —
and the duplicates evicted its nested-PP readings: 4 structural skeletons → 2, silently. Five
of six units then looked *encoded* (one reading) when they were merely truncated to one. Part
of the celebrated `total-skeletons 169 → 150` was this, not junk removal.

The instrument is a **candidate census**: `EIGENIUS_PARSE_DEBUG=1` now prints
`candidates=N (raw=R distinct-sem=D)`. `R == 256` means a budget bound the result; `D ≪ R`
means the budget was spent on duplicates. The fix was to spend both budgets on *distinct*
`(cat, sem)` pairs — deduplication is cost-free correctness, since `subsume_duplicates`
collapses the same readings after the eval anyway.

Two durable lessons:

- **A cap that silently drops output is not a guard, it is a defect.** Both truncations now
  log what they drop (§8's "no silent caps"). Nine units on the CNL-v3 page still bind
  `CLASSIFY_BUDGET` — visible now instead of invisible.
- **Suspect the measurement when a metric improves and the glosses do not.** The skeleton
  count fell while five units' glosses stayed wrong. That combination is the signature.

### 5c. Draw variance is real — a pin can turn on a coin flip

`temperature:0` is documented as NOT deterministic (~5% of the capped top-2 moves between
runs). A unit whose correct reading needs one specific sense in a 2-slot cap is therefore
fragile: `lead` has 14 verb senses, FOUR of which carry the frame-04 PP-oblique reading, and
whether "does not lead to cell death" keeps its pinned reading depends on the draw. Before
treating such a miss as a regression, take another draw — the repo's own methodology is
multi-draw with reported bands. Record the fragility either way.

## 6. Pin it — and expect the gate itself to be brittle

Pin the verified skeleton in `experiments/parsing/expected-readings.tsv` (sentence TAB
skeleton TAB note), bump `expected_reading_hits`/`_curated`, and confirm by replay.

The gate has twice broken on a **measurement** artifact rather than a parse change, both
times caught fail-closed:

- lexicon prefixes surviving sense-erasure (`n§` ≠ `C§`) counted one bracketing as two
  skeletons — 26% of the tracked lever;
- D64 hole binders are position-keyed (`$anaphor$i_j`), so a grammar change that moved the
  freshening site broke a pin on a reading that was never lost → `normalize_holes`
  α-normalizes them.

Rule: a skeleton must be a **structural fingerprint**. If a derivation-specific detail
(sense id, binder name, freshening span) leaks into it, that is a gate bug — fix the
eraser, then re-pin.

## 7. Record it

Every accepted change updates `experiments/parsing/baseline.json`: the `expected` block
(drift-free replay numbers), a `_provenance_note_*`, the `snapshot_lineage` entry when a
reseed was needed, and a `history[0]` entry stating root cause, fix, witness, and the
**known bounds/residuals**. That file is the durable record — this loop's memory.

## 8. Record what was REJECTED, in the code

A measured negative is a result and belongs where the next person will look — the code that
would be changed. `kernel/src/dcg/parse/seed.rs` carries the sense-cap characterisation plus
BOTH rejected fixes with their numbers, which is what stopped a third attempt from being
mounted blind. Cap-by-sense was in fact retried once (on the theory that new Eisner guards
had removed the duplication that sank it) and rejected again — the retry was justified, and
recording it was what made the justification checkable.

Corollary: **do not revert in the middle of an investigation.** A partially-working tree with
one failing test is an artifact; reverting destroys the state the next step needs and forces
re-derivation. Revert only when the change is positively established as wrong AND its finding
is already recorded — findings first, revert second, never the reverse.

## 9. Iterate lexicon changes as a LAYER, not a reseed

A reseed is ~40 minutes. To try a lexicon/ESL change, add it as a layer on an existing
snapshot — the base is treated as immutable, so a known-good snapshot always survives:

```bash
scripts/add-layer-to-snapshot.sh --base <clean-snapshot> --out <new-snapshot> fix.esl
```

Reseed only to BAKE IN what the layer proved, or for an importer-emission change
(`convert.rs`), where the content hash must match.

