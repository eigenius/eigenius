# D69 — Reading presentation: make the ranker's question answerable

**Status: plan, awaiting review — no code yet.** Trigger: the reading ranker chose a compound
reading over a single-concept one in the v2 demo, and the captured prompt shows it could not have
done otherwise.

## 1. The measurement

Captured live (`EIGENIUS_DUMP_SELECT_PROMPT=1`) for «MSI cancer models did not have the
exonuclease activity of WRN.», every other stage replaying:

| what the pool contains | what the prompt shows |
|---|---|
| 120 candidates | 120 numbered lines, 13,476 characters |
| **120 distinct sems** | **4 distinct strings** |
| 8 distinct skeletons | 8 `Structure N:` headers, each showing ONE gloss; structures 1/3/5/7 show the same string as each other, 2/4/6/8 likewise |

The four strings differ on exactly two axes: `Cancer Model` (the C1516211 concept) vs
`cancer model` (a compound), and `WRN protein, human` vs `WRN gene`. The ranker's recorded
rationale reasons about those two axes and nothing else — correctly, since nothing else was there.

**The collision that matters.** «exonuclease activity» renders identically for

```
Σ x0 : C1148824. prep_of(x0, kind_of(C0388246))                          → "the exonuclease activity of WRN"
Σ x0 : n00407535. compound_kind(x0, n14606137) ∧ prep_of(x0, kind_of(…)) → "the exonuclease activity of WRN"
```

and `verbalize` is not at fault in any local sense: `noun_phrase` renders head + compound
modifiers as "modifier head" ([verbalize.rs](../../kernel/src/dcg/verbalize.rs) `noun_phrase`,
`name_atom`), and C1148824's own label IS "exonuclease activity". The two readings **mean**
differently and **say** the same. That is precisely why the compound parse exists as a competitor.

So the conclusion is not "improve the English". **Strict verbalization is approximately a left
inverse of parsing: it reconstructs the input sentence.** Every reading of one sentence therefore
converges on (nearly) that sentence — by construction, the renderer collapses exactly the
ambiguities the parse resolved. Asking a surface-reconstructing renderer to distinguish readings
of one surface is asking it to fail.

The evidence says so twice over. 120 readings → 4 strings, and the 4 differ only where two
concept LABELS happen to differ in wording or case: «Cancer Model» (C1516211) vs «cancer model»
(the compound's head), «WRN protein, human» (C0388246) vs «WRN gene» (C1337007). The
discrimination we currently get is an ACCIDENT of label strings, not a designed property. Where
labels coincide — C1148824's label is literally "exonuclease activity" — discrimination is zero.

The fix is therefore a SECOND RENDERING MODE, not a better paraphrase: an **expanded
verbalization** whose job is to expose the semantic commitments a reading makes, where strict
verbalization's job is to read like the source.

Scope check, so this is not overstated: on the corpus page, 0 of the 6 wrong selections were
gloss-indistinguishable from a correct candidate (57% of presented candidates carry a distinct
rendering). Collisions are not what causes the page's selection errors. They are total on this
sentence, and they decide what the demo commits.

## 2. Two defects, separately fixable

**D69-A — the rendering is not injective on the candidate set.** Distinct sems may render to one
string. The ranker is then asked to choose between indices it cannot tell apart, and answers
anyway, with a confident rationale about the axes it *can* see.

**D69-B — indistinguishable candidates are presented as separate choices.** Even granting a lossy
renderer, offering 45 identical lines is a defect in the presentation itself: it invites an
arbitrary index, wastes ~13 KB of prompt, and makes the resulting `DecisionPoint` rationale
unfalsifiable (it explains a choice that was not made on the stated grounds).

They are different fixes: A is about what a rendering must carry, B is about what may be offered.

## 3. The invariant

> **Every candidate presented to a ranker renders to a string distinct from every other candidate
> in the same pool. A pool that cannot satisfy this is a diagnostic, not a silent choice.**

This is the presentation-side analogue of the fail-closed discipline the recorded stages already
have (a replay miss abstains and is counted; it does not quietly degrade).

## 4. D69-A — the expanded verbalizer

**Two modes over one traversal.** `verbalize` gains a mode parameter; the tree walk, the naming
helpers and the axiom dispatch stay shared (the 2026-07-25 "gate's renderer and selector's
renderer must be ONE function" argument survives — it is one function with two output registers,
not two functions that can drift).

| mode | audience | job |
|---|---|---|
| `Surface` (today's) | humans, the gate's narration, `narrate.py`, claim descriptions | read like the source sentence |
| `Expanded` (new) | rankers, proposers, any model asked to CHOOSE | expose what the reading commits to |

What `Expanded` must expose, derived from what the pool actually varies:

- **Concept identity at every content position** — label AND IRI (`«exonuclease activity»
  [C1148824]`), because labels collide and IRIs do not.
- **Concept meaning where the chain has one.** C1148824 carries a `core:description` ("Catalysis
  of the hydrolysis of ester linkages within nucleic acids…"). A model choosing between a named
  biological process and a generic noun modified by another noun should see that definition; it
  is the single most decisive fact available and today it is thrown away.
- **The compound relation as UNSPECIFIED.** `compound_kind(x, exonuclease)` asserts that *some*
  relation holds between the activity and exonuclease — that is the whole semantic difference
  from the named concept, and "exonuclease activity" hides it. Render it as what it is.
- **Attachment and scope** — which Σ a modifier lands in, what the negation scopes over. Two
  readings differing only in bracketing must differ in the rendering.

Sketch of the contested pair (final surface is a slice-2 decision, see §4a):

```
[A] the unique x with
      x : «exonuclease activity» [C1148824]  — "catalysis of the hydrolysis of ester linkages
                                                within nucleic acids, removing residues from
                                                the 3' or 5' end"
      of(x, kind «WRN protein, human» [C0388246])
[B] the unique x with
      x : «activity» [n00407535]             — "any specific behavior"
      compound-with(x, «exonuclease» [n14606137])   ← relation UNSPECIFIED by the parse
      of(x, kind «WRN protein, human» [C0388246])
```

**Why this no longer depends on the pool.** An earlier draft of this plan added concept IDs only
where two candidates would otherwise look the same — which made the fix depend on which other
readings happened to be present. With `Expanded`, every reading shows its concepts and structure
always, so each is distinct on its own: A reads differently from B whether or not B is there.
Grouping the pool and printing shared parts once (§5) is then only about keeping the prompt
short.

**Fail closed regardless.** If two candidates in one pool still render identically under
`Expanded`, the renderer cannot express a distinction the parser makes: error naming both sems,
never present the pool. The assert is cheap and it is what stops this defect recurring silently.

## 4a. Which format to use is a measurement, not a preference

An expanded rendering may well read worse to a model than fluent English — more tokens, less
familiar shape. We do not have to argue about it. Run the ranker over the 62 sentences that have
a human-verified reading and count how often it picks that reading: that is
`selection_accuracy`, and it answers the question directly. The gold file records each reading as
a TERM, not as English (`reading-adjudications.tsv` keys on the sem — §6), so changing how we
render text cannot invalidate it. Build one version, measure it; if someone wants a different
format, measure that one too and compare the numbers.

## 5. D69-B — presentation shape

With §4, the 120 lines become 120 distinct lines. That satisfies the invariant and is still a bad
question. The pool factorizes — 8 structures × ~15 sense assignments — and the two axes are
different kinds of judgment:

- **Structure**: which bracketing/attachment. Visible in the skeleton, adjudicable, and the thing
  `reading-adjudications.tsv` already partly tracks.
- **Sense assignment**: which concept each surface denotes, GIVEN the structure.

Proposal: present them as **two levels** — each structure rendered ONCE in `Expanded` form with
its shared slots filled, then the sense choices as a small table listing only the positions that
vary, each option carrying label + IRI + definition. The model picks a structure and a sense
assignment. This is where the economy comes from: not from hiding semantics, but from not
repeating the invariant parts 120 times. A two-call ranker (structure, then senses) is the
natural realization; a single call with a two-level prompt is the cheaper first step.

Also in scope for B: **cap what is presented** with an explicit, logged truncation if a pool is
still large after grouping (no silent caps — the D62 rule).

## 6. Blast radius (what this invalidates)

Rendering feeds three recorded keys. Verified in code:

| artifact | keyed on | invalidated? |
|---|---|---|
| `selections*.json` | sentence + doc sha + prior glosses + **each candidate's skeleton, gloss, sem** (`selection_key`) | **YES** — every draw misses |
| `kinds*.json` | sentence + **chosen reading's gloss** (`kind_key`) | **YES** |
| `proposals*.json` | hole + candidate surfaces (claim glosses) | **YES** for claim antecedents |
| `ranks*.json` | sentence + word senses | no — pre-rendering |
| `reading-adjudications.tsv` | sentence + **sem** (verbatim term) | **no** — the gold survives |
| `expected-readings.tsv` / skeleton ledger | skeletons | **no** — rendering does not touch the forest |
| `baseline.json` parse metrics | forest | **no** — must hold EXACTLY; that is the regression check |

The gold sets surviving is what makes this affordable: re-drawing is mechanical, re-adjudicating
would not have been.

## 7. Slices (ALL DONE — 2 and 5 on 2026-08-13, 3 and 4 on 2026-08-17)

1. **This note** — review gate.
2. **D69-A**: the `Expanded` mode + the injectivity assert, with unit tests on the
   concept-vs-compound pair from §1 (they must render differently) and on a bracketing pair.
   Gate: parse metrics byte-identical (rendering is post-parse); `Surface` output unchanged
   byte-for-byte, so the gate narration and claim descriptions do not move; the assert fires on a
   constructed colliding pool.
3. **Re-draw**: re-record selections + kinds + proposals on the d67 snapshot (page and demo),
   re-baseline `selection-baseline.json` WITH provenance, re-measure the composed close-out
   (currently 50/1/11/0). Report selection accuracy before/after — the honest question this whole
   note exists to answer is *how much of the ranker's error was blindness*.
4. **D69-B**: two-level presentation + logged truncation. Re-draw again (same mechanics), measure
   again. Keep 3 and 4 separate so the attribution is clean.
5. **Demo v2 regeneration** + the caveat paragraph in its README rewritten (it currently blames
   the sense draws; §1 of this note is the real story).

## 7a. Slice 2 — implemented `2026-08-13`

`Register::{Surface, Expanded}` on `Vb` (one traversal, two output registers). `Expanded`:
`«label» [id]` at every content position, `+ compound-with X (relation unspecified)` for a
compound modifier, `+ relation Y` for each restrictor instead of word order. `Surface` untouched.
The pool's concepts travel on `DocumentContext.concepts` as `ConceptNote { id, label, definition }`
— the chain's own `core:description`, printed ONCE as a legend beside the candidates rather than
repeated per line. The injectivity check (`first_collision`) runs in `resolve_document` BEFORE any
ranker sees the pool, so it guards the pin and replay arms too; on a collision it abstains and
prints both sems.

**Gates, all met:** parse metrics EXACT (readings 226, skeletons 144, hits 60/62, gap 0, encoded
11 — rendering is post-parse, as claimed); `Surface` byte-identical (unit test); 173 workspace
suites; clippy `-D warnings` on plain and on wordnet+encoding+reasoning `use-llm`. No collision
fired on the page — after `Expanded` the corpus page's pools are injective.

**What the ranker now sees** (same sentence, same forest):

```
every «process» [n00029677] + compound-with «DNA Repair» [C0012899] (relation unspecified)
  is a «target» [n05981230] + degree-greater is «attractive»
```

## 7b. Slice 3 — the measurement, and why it is not yet conclusive

Live draw over the replayed d67 forest (`experiments/parsing/selections/2026-08-13-d69-expanded.json`):

| | Surface (2026-08-12) | Expanded (2026-08-13) |
|---|---|---|
| structure-correct | 23/31 | **23/31 — unchanged** |
| invalid-selected | 0 | 0 |
| reading-correct | 21/31 | **14 correct, 7 wrong, 10 UNADJUDICATED** |

The register changed **11 of 31 choices** — 6 keeping the structure and moving senses, 5 moving
structure; 20 identical. That is the expected signature: the register exposes senses, so sense
choices move, while structural accuracy holds.

The gated number cannot be computed yet, and the harness says so rather than guessing: 10 chosen
readings are not in `reading-adjudications.tsv`, and an unadjudicated decision is not scoreable.
Bounding it without touching the gold set: **best case 24/31, worst case 14/31, against a 21/31
baseline** — the interval straddles the baseline, so the change is INCONCLUSIVE on the gated
metric today. 8 of the 10 unadjudicated choices match the human-pinned STRUCTURE.

Completing it needs those 10 adjudicated. Note the hazard plainly: the same class of judge that
made the selections should not certify them, so these want human sign-off (or at least an
independent pass) rather than a model marking its own work — the numbers above are reported
unadjudicated for exactly that reason.

## 7c. Slices 3 and 5 — completed `2026-08-13`

**A second lossy site, found by the guard rather than by inspection.** On the composed run the
injectivity check ABSTAINED on «WRN dependency may require specific lineages or a stronger
mutation phenotype.»: two candidates rendered identically because `Expanded` dropped a
comparative's STANDARD — `gt(deg(x), std_a…)` ("stronger than the norm") and
`gt(deg(x), deg(t))` ("stronger than t", an elided «than» recovered from the discourse) both came
out as "degree-greater strong". Rendering the standard fixed it, and the effect is measurable:

| composed configuration (discourse loop + ranker) | encoded | ambiguous | open | gap |
|---|---|---|---|---|
| Surface register | 50 | 1 | 11 | 0 |
| Expanded, comparative standard dropped | 50 | 1 | 11 | 0 |
| **Expanded, standard rendered** | **51** | **0** | 11 | 0 |

The abstention WAS the residual ambiguous unit; making the pool injective let the ranker decide
it. Replay-verified: ranks 62/0, selections 39/0, kinds 48/0. Draws:
`selections/2026-08-13-d69-discourse.json`, `kinds/2026-08-13-d69-discourse.json`.

**The chooser's register must not leak into the discourse.** `SelectionOutcome.chosen_gloss` was
the candidate's string, which is now `Expanded` — and that gloss is threaded into later
sentences' context, the kind classifier's prompt, and the proposer's priors. Those consumers are
READING the sentence, not choosing between renderings of it. `chosen_gloss` is now recomputed in
`Surface`, which both keeps three prompts readable and shrinks the blast radius §6 predicted:
only the SELECTION draws invalidate on a presentation change, not kinds and proposals.

**Coupled stages must be drawn in ONE pass.** Recording selections first and kinds second produced
a pair that could not reproduce itself (36/2 selection misses on replay): kinds decide claim
KINDS, kinds gate which claims are eligible anaphora antecedents, antecedents change the candidate
pool, and the pool is in the selection key. Recorded together, both replay 0-miss. Worth stating
as harness discipline, not a one-off.

**Demo v2 (slice 5) regenerated, and its caveat is retired.** The two paragraph variants are
parsed and selected INDEPENDENTLY — no shared draw, no pin — and now land the same term apart
from the negation: the edited `claim_1` is the intact proposition with a trailing
`-> logic:False`, byte-identical otherwise. Before D69 the ranker picked a compound reading for
the negated sentence because it could not see the concept. `run.sh --reparse` passes end to end
(intact COMMITTED, edited REJECTED) with all four stages replaying.

## 7d. Cross-model probe on the crab sentence (`2026-08-13`)

The exact prompt (captured and verified in `experiments/parsing/results/prompt-sentence6-2026-08-13.txt`)
was put to three different models. «WRN was dispensable in models of microsatellite-stable
cancers.», 20 readings = 2 structures (WRN gene / WRN protein) × 2 cancer senses (crab genus /
astrological sign) × 5 "stable" adjectives. **No faithful reading exists**: the disease sense and
the model-system sense are absent from this pool.

| | chose | WRN | cancers | "stable" | abstained |
|---|---|---|---|---|---|
| model 1 (the recorded draw) | [12] | gene | crab | a02274089 "firm and dependable; *the economy is stable*" | no |
| model 2 | [10] | gene | crab | a02290998 "resistant to change of position; *a stable ladder*" | no |
| model 3 | [10] | gene | crab | (not stated) | no |

**Unanimous where evidence exists, arbitrary where it does not.** All three take the GENE over the
protein — the substantive call — and two cite the prior sentence's «WRN gene» for consistency, so
the discourse threading is working. All three take the crab, because nothing better is offered.
They split only on which non-genomic "stable" to attach; none of the five is *microsatellite*
stability.

**The intra-structure choice looks POSITIONAL, not semantic.** [10] is the first reading of the
gene structure and two of three picked it; model 3's `runners_up` is literally `11,12,…,19,0,…,9`
— sequential order within the preferred structure, then the other structure. That is what a
ranker emits when it is discriminating structures and not senses. **This is the empirical case for
§5 (D69-B):** ten near-identical lines per structure invite a positional pick, and presenting the
sense assignment as an explicit small table is the fix. Slice 4 now has a measurement behind it,
not just prompt economy.

**Three failures to abstain.** The prompt offers abstention but discourages it — "*prefer choosing
when one reading is clearly best*". Facing a pool where every option is wrong, all three chose and
justified. A two-model result would be an anecdote; three is a defect in the instruction. Fix:
make "no candidate faithfully expresses the sentence" a first-class outcome, and say that a
reading whose concepts contradict their own definitions is not a candidate.

**All three rationales are DISEASE rationales — none of them thinks it is picking a crab.**
Model 1: "n01977832, the biological disease concept, not the astrological sign". Model 2: "avoids
the astrological sense of Cancer". Model 3: "the WRN gene's essentiality … in cancer cell line
models". Read together they show what actually happened, and it is not evidence-overriding: the
two options are LABELLED IDENTICALLY in the candidate lines — «Cancer» [n01977832] and «Cancer»
[n09752657] — one is transparently astrology, so the models eliminated it and took the survivor to
be the disease sense. Model 1's "biological" is even literally true of Cancridae; only "disease"
is false. Under a forced choice, that is sound reasoning about an unsound pool.

Which relocates the defect a second time. It is not the ranker's judgment, and it is not (here)
the legend's absence. It is that **the pool offers two senses sharing one label, no faithful
option, and an instruction that discourages saying so.** The disease sense n14239918 is labelled
«cancer» lowercase and is not in this pool at all. Presentation fixes that follow: an explicit
"none of these readings is faithful" affordance (not a reluctant `abstain` flag), and a label
collision — two candidates whose labels match but whose ids differ — treated as a signal to
surface the definitions inline rather than only in the legend.

## 7e. The crab was a FROZEN FAILURE in the sense-rank draw (`2026-08-13`)

Raised in review: "crab and the zodiac sign shouldn't have made it into the parse to begin with."
Correct, and the upstream stage is where the defect is.

**The measurement.** In `ranks/2026-07-29-demonstratives.json`, «WRN was dispensable in models of
microsatellite-stable cancers.» carries `order = 0..N-1` for EVERY word — all 20 senses of
«models», all 7 of «cancers», nothing eliminated, nothing even reordered. Across the file's 422
word-entries, 80% are proper subsets (real elimination) and exactly **one sentence of 62** is
identity throughout: this one. And identity-for-everything is precisely what the code returned on
failure:

```rust
let Some(reply) = self.ask(&prompt) else { return identity(); };
if reply.rankings.len() != words.len() { return identity(); }
```

So a failed call was recorded as a ranking. On replay the key is FOUND, so it counts as a hit,
`assert_replay_faithful` passes at 62/0, and the run reports clean. The crab and the astrological
sign reached the reading ranker because for this one sentence the sense ranker never ran, and the
fallback keeps everything. The prompt was never the problem — it shows each candidate's WordNet
definition, and re-asked, the live ranker keeps **2 of 7** senses for «cancers»: `n14239918` "any
malignant growth or tumor" and its UMLS twin C0006826, dropping the crab, the zodiac sign and the
rest; and **5 of 20** for «models», dropping `n00898804` "the act of representing something". That
sentence's forest halves, 20 readings → 10.

**Three fixes, all landed.**

1. **`SenseRanker::rank` returns `Option`.** A transport failure, a malformed reply, or a replay
   miss is a NON-ANSWER, structurally distinct from "I ranked and kept everything". The caller
   falls back to seed order exactly as before; the difference is that the RECORDER now writes
   nothing, so a re-run asks again instead of inheriting the failure. Locked by
   `a_ranker_that_does_not_answer_records_nothing`.
2. **Identity answers are flagged at record time.** An answer that eliminates nothing anywhere is
   either a ranker declining to work or a failure that slipped through some other impl's
   fallback; the recorder says so on stderr while the run can still be repeated.
3. **Re-asked live** — the result above, obtained through the real CLI path without touching any
   committed draw.

**Consequence for the open adjudications.** Item 6's crab is not a ranker error and not a lexicon
gap: the disease sense exists, is chosen elsewhere on the same page, and was excluded from this
sentence's pool by a recorded failure. The reading is still `wrong`, but the finding is upstream.

**Not yet done — a re-baseline event.** The committed draw still contains the poisoned entry, and
`baseline.json`'s parse metrics (226 readings, 144 skeletons) are calibrated ON it. Re-recording
the page fixes the entry and moves those totals (that unit alone drops ~10 readings), so it needs
`baseline.json` AND `selection-baseline.json` re-derived together with provenance. Proposed, not
performed.

## 7f. Why a correct sense ranking produced a projection screen (`2026-08-13`)

«We analysed data from large-scale silencing screens.» The parse uses `n04152829` "a white or
silvered surface where pictures can be projected". Traced end to end:

1. **The ranker eliminated every NOMINAL sense of «silencing»** — it kept 2 of 7, both verb senses
   (`silence.v.00461493`, `silence.v.00463007`). The nominal senses (`C0858952`, `n13925550`, …)
   are absent from its answer. That is a judgment error: the sentence needs a noun modifier.
2. **For «screens» it ranked three verb senses above the three screening nouns**, and the cap
   compounds that: the cap counts ENTRIES while the ranking keys SENSES, and the rank-0 verb sense
   owns FOUR entries (one per grammatical category), so a cap of 2 is consumed by two categorial
   variants of one verb. The known gap of 2026-07-24, here changing which sense lands rather than
   only how much ambiguity there is.
3. **The noun phrase cannot be built** — neither word offers a noun — so the sentence yields
   nothing at the base cap.
4. **Pass 1 of the widen ladder cannot recover it.** The elimination cut is applied at EVERY rung
   (`eff = min(cap, ranked)`), so a sense the ranker omitted can never seed while ranks are in
   force. No cap value produces a noun for «silencing». This is the July change that stopped widen
   from flooding other words with rejects; the cost, unmeasured until now, is that a WRONG
   elimination has no in-Pass-1 recovery at all.
5. **Pass 2 discards the ranking wholesale** (`ranks = None`, no cut, static frequency) and parses.
   Static frequency for «screen» is the projection surface and the CRT display; for «silencing»,
   "the state of being silent". Those two axes give exactly the 4 readings observed.

**Two experiments confirm the chain.** Re-ranking only «screens» so the screening noun is rank 0
changes nothing — the pool is still the same 4 Pass-2 readings, because «silencing» is still
verbs-only. Re-ranking BOTH words' nominal senses to rank 0 yields
`kind_of(Σx0 : C0220908. compound_kind(x0, C0858952) ∧ large-scale(x0))` — the correct reading,
and the pool drops to 2. A third check: the candidate pool is byte-identical with the ranking
applied and with no ranking at all, which is the signature of Pass 2.

**The design tension, stated plainly.** Pass 2 exists so a wrong elimination costs a slower parse
rather than a lost one. It does prevent the gap. But it converts a wrong elimination on ONE word
into "no ranking at all for the whole sentence", so the recovery silently substitutes the most
frequent senses everywhere. A targeted relaxation — restore the eliminated senses of the word that
has no admissible category, keep the ranking on every other word — would have kept this sentence's
other five words correctly ranked and admitted the screening noun.

## 7j. Method: name the best reading, then move the processing to it (`2026-08-13`)

Stated by the maintainer while working the adjudication backlog, and it is the rule the whole
day's work retrospectively followed:

> The objective is to identify the most appropriate reading of each sentence. Once that has been
> established, we can use that as goal post to adjust the processing.

The consequence is that "is the chosen reading faithful?" is the wrong terminal question. Once the
best reading is named, every gap becomes mechanical and measurable:

| the best reading is… | the defect is | fixed on 2026-08-13 by |
|---|---|---|
| not in the forest at all | coverage — lexicon, cap, tokenisation | re-recording the frozen sense-rank draw (crab); the hyphen-joined multiword (C4321493); the sense-counted cap (C0220908, and WRN as an individual) |
| in the forest, not chosen | selection — presentation, ranker, prompt | the `Expanded` register + concept legend; the injectivity guard; the comparative standard |

Four fixes in a day, each triggered by someone naming the reading that should have won. None of
them would have been found by asking whether the chosen reading was acceptable — each chosen
reading WAS acceptable-looking, which is exactly why the defects survived so long.

The tension this creates with the twins policy is real and deliberate: `correct` marks a faithful
reading, but faithful-but-weaker is not the goal post. The ledger header now asks that the
evidence name the BEST reading even when several are faithful, so the signal is not lost.

## 7k. Why three units fall to Pass 2 — `cat_shape` cannot see a lost ARGUMENT category (`2026-08-15`)

Measured with two new instruments: `WidenTrace`/`WidenPass` (which pass and rung produced a forest,
printed per unit as a `WIDEN` line) and `probe_blocking_word` (leave-one-out over the recorded sense
draw — blank ONE word's ranking, keep every other word's, and see which one rescues the sentence).

**Three of 62 units resolve only on `StaticFallback`, with the ranking discarded for every word:**

| unit | attempts | Pass 1b | Pass 2 succeeded at | blocking word |
|---|---|---|---|---|
| These observations suggest … MMR deficiency | 9 | ran, promoted 19 | **cap 2** | «suggest» |
| Germline mutations … cause Lynch syndrome | 10 | ran, promoted 8 | cap 4 | «Lynch syndrome» |
| MSI cell lines from these four lineages … | 12 | ran, promoted 11 | cap 16 | «greater» |

The MMR row is the diagnostic one: it fails under the ranking at caps 2, 4, 8 and 16 — every rung,
twice, once in Pass 1 and once in Pass 1b — then parses on the FIRST Pass-2 attempt at the BASE cap of
2. Widening was never the issue. The ranking itself blocks composition, and exactly one word is
responsible in each of the three.

**The mechanism, for «suggest».** The draw kept `v00930806_t` (rank 0) and `v00930368_i` (rank 1) and
ELIMINATED `v00927430_c`. The suffixes are the frame: `_t` transitive, `_i` intransitive, `_c` clausal
complement. «These observations suggest THAT …» needs the clausal frame, and it was the one thrown
away, so no cap admits it and no widening recovers it.

**Why Pass 1b could not fix it.** `recovery_ranks` promotes the highest-ranked sense of every category
SHAPE the cut removed — and `seed::cat_shape` walks to the head of the application spine and returns
only that constructor's name. A transitive verb `fwd(m, bwd(m, S, NP), NP)` and a clausal verb
`fwd(m, bwd(m, S, NP), S)` therefore BOTH hash to `"fwd"`. With `_t` (fwd) and `_i` (bwd) surviving,
`full.difference(&survived)` is empty for «suggest»: nothing looks lost, so nothing is promoted. The
detector is blind to a lost ARGUMENT category because it only looks at the top-level slash direction.

This is the class §7g was NOT built for. Pass 1b's trigger is "the word can no longer fill any slot it
could have filled", measured at head-constructor granularity. Here the word still fills a slot — just
not the slot the sentence needs.

**Consequence, and why it matters beyond these three sentences.** Pass 2 rescues the sentence by
discarding EVERY word's ranking, so one blocked word costs the whole sentence its sense selection.
That is the confirmed cause of both anomalies in the MMR unit: the ranker kept `C4522088` and dropped
Turcot, and put the WRN GENE individual at rank 0 — and neither survived, so static frequency chose
«Turcot syndrome» and the protein kind. The unit's pin (which wants the bare-individual `compound`)
misses for the same reason. **Readings from a `StaticFallback` unit are not evidence about the
ranker**, and before `WidenTrace` there was no way to tell them apart from readings it did choose.

**OUTCOME (`2026-08-15`) — half of it; D70 closed the thread.** Two changes to `recovery_ranks`, both
making it see what the ranker can act on:

1. **Granularity.** New `seed::cat_frame` = head constructor + the constructor's LAST argument, used
   ONLY by `recovery_ranks` (`cat_shape` is untouched because `apply_sense_cap` keys on it). The last
   argument is the slot saying what a category TAKES or how it is INDEXED, which covers both observed
   losses: a slash's argument (`fwd/cat_cp` vs `fwd/cat_np` — «suggest») and a noun's number
   (`cat_n/mass` vs `cat_n/num_any` — «Lynch syndrome», where the ranker eliminated C1333990 and,
   because elimination is HARD, took its bare-noun `mass` entry with it).
2. **Scope.** `recovery_ranks` iterated single TOKENS while `contextual_sense_ranks` ranks SPANS, so a
   multiword blocker was invisible by construction. It now enumerates the same spans via `span_limit`.

**CORRECTED `2026-08-15` (same day) — this table over-credited the change.** The figures below were
measured with BOTH halves of `cat_frame` in place, and the second half (the NUMBER index) was reverted
hours later as unsound: it obtained a missing `cat_n/mass` frame by restoring a ranker-ELIMINATED
sense, i.e. by substituting a different CONCEPT (D70 §1c — «mmr deficiency» C4522088 → C0265325 Turcot
syndrome). What this note's change is actually responsible for is the SLASH-ARGUMENT half.

| | before | after `cat_frame` (slash-argument only, as shipped) | after D70 |
|---|---|---|---|
| units on `StaticFallback` | 3 | **1** | **0** |
| expected-hits | 61/62 | 61/62 | **62/62** |
| total-skeletons | 180 | 180 | 175 |
| total-readings | 594 | 594 | 637 |

The slash-argument key is what fixed «suggest» (lost `fwd/cat_cp`) and «greater»; it is well-founded
because promoting another sense OF THE SAME WORD changes which reading is reachable, not which entity
is denoted. The remaining fallback — «Germline mutations … cause Lynch syndrome.» — and the MMR unit's
pin both needed D70, where the real cause turned out to be a concept that could not compose in a bare
position at all.

«These observations suggest … MMR deficiency» RECOVERED its pin — the pin wants WRN as the bare
individual, the ranker had put the WRN GENE at rank 0 all along, and Pass 2 was discarding it. The
page now has NO unit whose readings were chosen by static frequency, and 62/62 is its first full
faithfulness sweep. `expected_reading_misses` is empty.

Neither change was reached by reasoning alone. `probe_blocking_word` named one word per sentence
(«suggest», «greater», «Lynch syndrome»), and the second gap only surfaced because fixing the first
left «Lynch syndrome» still falling back.

FOLLOW-UP: the MMR unit's reading-level ledger rows describe readings produced under the old fallback
(Turcot syndrome, the WRN protein-kind). They need re-adjudicating against a fresh composed draw.

**The fix this pointed at** (now made): give the recovery detector a finer
shape that includes the ARGUMENT's head constructor, so `fwd(…, NP)` ≠ `fwd(…, S)`. Then the lost
clausal frame is visible, Pass 1b promotes `v00927430_c`, and every other word keeps its ranking —
the narrow rescue, exactly as designed. CAUTION: `cat_shape` is also used by `apply_sense_cap`, where
finer shapes would change which categories the cap keeps, so the safer change is a SEPARATE finer
function used only by `recovery_ranks`, measured on its own. The leave-one-out probe is the
instrument for checking it: all three units should flip from `StaticFallback` to a ranking-respecting
pass, and no unit that currently parses may regress.

## 7m. Slice 4 — D69-B measured and REJECTED (`2026-08-17`)

Implemented as specified in §5: each structure's invariant frame rendered ONCE, the positions whose
sense varies enumerated as slots `{A}`, `{B}`, … with their options, and each reading given as its
slot assignment plus its index. Plus §5's other requirement, explicit LOGGED truncation
(`MAX_STRUCTURES_SHOWN`, structures dropped whole so a shown structure is never half-shown, the drop
both `eprintln!`-ed and stated in the prompt — the D62 no-silent-caps rule).

It produces exactly the intended shape:

```
Structure 1:
  possibly, «scientist» [n10560637] {A} «Synthetic Lethality» [C4280020] and «scientist» … {B} … {C}
    A = «exploit» [v01162754] | «exploit» [v01164273]
    B = «therapeutic» [n04074482] | «Therapeutic procedure» [C0087111]
    C = «cancer» [n14239918] | «Malignant Neoplasms» [C0006826]
      [0] A=[v01162754] B=[n04074482] C=[n14239918]
      …
```

**And it is WORSE.** Same forest, same ranks replay (2026-08-17-d70b, 62/0), both draws fully
adjudicated:

| | reading-correct | reading-wrong | structure-correct |
|---|---|---|---|
| slice 3 — flat `Expanded` listing | **30/40** | 10 | **33/40** |
| slice 4 — two-level slots | **24/40** | 16 | 29/40 |

Six decisions lost, and the structure diagnostic fell too — which is the telling part, since presenting
structures once and named was supposed to make the structural choice EASIER. What the two-level draw
picked instead, on units the flat draw got right: narrow modal scope on «WRN dependency may require…»
(and the protein where the page uses the gene), a THREE-way conjunction split on «PARP-1 inhibitors…»
that detaches «with deficiencies» from «cancers», `v01753788` 'bring into existence' for «create»,
n14561618 'a symptom of' for «impairment», C0600688 «Toxic effect» for the property «toxicity».

Reading the pattern honestly: the model must recombine `A=[…] B=[…] C=[…]` back into a proposition
before it can judge one, and it appears to judge the recombination less reliably than a rendered
sentence — the economy was bought from the reader's comprehension, not from redundancy. §4a said the
surface is settled by A/B on selection accuracy and not by taste; the A/B has answered.

**Disposition.** The flat listing is the DEFAULT again. The two-level renderer is kept behind
`EIGENIUS_SELECT_TWO_LEVEL=1` so the A/B is repeatable and because the idea may be right in the
realisation §5 actually preferred — a two-CALL ranker (structure, then senses), of which this
single-call prompt was the cheap proxy. The truncation half is kept ON unconditionally: it is
independently correct and unrelated to the presentation question.

**Variance caveat.** A re-run of the restored default drew 28/40 correct with 2 novel readings, against
slice 3's 30/40 — same code, same forest. Draw-to-draw variance is real at this scale, so a 2-decision
difference is not a signal; the 6-decision gap to D69-B is.

## 7n. The two-call ranker — eigenius#264 strand 1, DEFAULT (`2026-09-30`)

The realisation §5 preferred, built on the owner's design (2026-09-30): a **structure call**, then a
**sense call**. eigenius#264's witness was the ranker reasoning about senses when the candidates
differed only in attachment — «The MSI relationship compared favourably to other strong biomarkers
for vulnerabilities.», 36 readings, 2 skeletons, identical concept sets.

- **The structure call** shows each structure once, in the **Structural register**
  (`verbalize::Register::Structural`): Expanded's explicit relations and grouping, with every content
  position named by the sentence's own words (`unit_surface_names`, over the spans seeding looks up,
  shortest first, derived adjectives on their own token), so the readings of one structure render
  alike. Groups are by skeleton first, so a sense the rendering cannot hide never splits a structure.
  Under the structures it lists **how they differ**, from each reading's links
  (`verbalize::structure_links`): per phrase, where it attaches, what it modifies, which verb it is an
  argument of — «for vulnerabilities»: attaches to «relationship» in structures 1, 2; attaches to
  «biomarkers» in structures 3, 4. The rationale must decide those lines.
- **The sense call** is the flat listing, restricted to the chosen structure's readings, numbered from
  0. A pool with one structure skips the first call; a structure with one reading the second.
- `EIGENIUS_DUMP_STRUCTURES=1` prints every unit's structure question on a replay, without a model.

**The A/B** (owner's protocol: three live draws per arm, snapshot
`wordnet-umls-aligned-2026-09-30-governed-preps`, rankings `ranks/2026-09-29-d95-slice8.json`, every
chosen reading adjudicated, each draw re-scored by replay against the final ledger):

| arm | draw | reading-correct | structure-correct |
|---|---|---|---|
| flat | `2026-09-30-governed-preps` | 23/41 | 27 |
| flat | `2026-09-30-ranker-baseline-2` | 21/41 | 27 |
| flat | `2026-09-30-ranker-baseline-3` | 24/41 | 28 |
| two-call | `2026-09-30-ranker-twocall-v2-1` | 29/41 | 34 |
| two-call | `2026-09-30-ranker-twocall-v2-2` | 28/41 | 35 |
| two-call | `2026-09-30-ranker-twocall-v2-3` | 30/41 | 36 |

Means 22.7 against 29.0 correct, 27.3 against 35.0 structure; the ranges do not overlap, and §7m's
draw-to-draw variance is 2. **Disposition: two calls are the default**; `EIGENIUS_SELECT_FLAT`
(`measure-parse-rate.sh --flat-ranker`) keeps the flat listing so the A/B can be repeated.

What the structure call surfaced, and the owner ruled the same day: on «Project Achilles screened
cell lines with a CRISPR library.» and its DRIVE sibling the library is the instrument of the screening,
not a property of the cell lines; on «Depletion of WRN promoted apoptosis and cell cycle arrest in MSI
models.» the locative scopes over both effects. All three pins moved to the verb-adjunct encoding,
as the «with sequencing» pin already was; the July pins had verified other properties of these units.

Two earlier versions are not arms. A pilot rendered atoms by concept labels and leaked senses through
unrendered fragments (`⟦a02734544(G#0)⟧`), splitting one structure into two; and three draws of the
first two-call build abstained four times, silently: the sense call listed a structure's readings
under their indices in the whole list, and a reply by position fell outside the structure. The
listing is numbered from 0 now, and a malformed reply is logged before the ranker abstains.

## 7o. The model — eigenius#264 strand 2, `jev-latest` DEFAULT (`2026-09-30`)

The owner's design (2026-09-30): the two calls go through a provider-neutral decision interface
(`dcg::decision`: a `Choice` of context, question, notes and keyed options; a `Decider` returns the
chosen key, runners-up, probabilities where the provider gives them, and a rationale where it gives
one). Two deciders: Anthropic (the forced `emit` tool, or the JSON-schema output mode on the Claude 5
models, which reject a forced tool choice) and TypeSafe System One (`render_typesafe`: the context as
`state`, one `choice` question whose criteria are the options; no rationale, so the record holds the
probabilities). Every arm sees the same context as Claude.

**The A/B** (three live draws per model, same snapshot and rankings as §7n, 28 new ledger rows,
each draw re-scored by replay; harness run time per draw, the forest replayed):

| model | reading-correct | structure-correct | decisions identical in all 3 draws | time |
|---|---|---|---|---|
| `claude-sonnet-4-6` | 26, 25, 26 | 30, 29, 30 | 38/41 | ~400 s |
| `claude-sonnet-5-5` | 29, 24, 28 | 33, 30, 33 | 20/41 | ~270 s |
| `jev-latest` | 26, 26, 26 | 30, 30, 30 | 37/41 | ~60 s |

The Claude 5 models reject `temperature`, which accounts for sonnet-5-5's spread. sonnet-4-6 scored
29, 28, 30 (structure 34, 35, 36) on §7n's dedicated prompt over the same forest and ledger: the
neutral rendering (`decision::render_prompt`) costs it about 3 readings. Two differences from the
§7n prompt: the final instruction no longer names what to decide (where each phrase attaches, and
why), and structures are labelled `[1]` rather than `Structure 1:`. jev reads `render_typesafe`,
not `render_prompt`.

**Disposition (owner, 2026-09-30): `jev-latest` is the reading ranker's default**
(`model_config::DEFAULT_READING_MODEL`), at sonnet-4-6's accuracy in a sixth of the time, and its
prompt and presentation are the next thing measured. The sense ranker and the other proposers stay on
`DEFAULT_MODEL`. A formalization request names the two separately (`FormalizationOptions.model`,
`.reading_model`), and each seam's draws record their own model. The flat listing asks Anthropic
models only and keeps `claude-sonnet-4-6`.

## 7p. The grammatical analyses — derivations and a grammar-book structure call (`2026-09-30`)

**What the structural register was deciding.** An offline screen of jev's requests
(`EIGENIUS_DUMP_DECISIONS`, the TypeSafe API called directly; ten prompt variants: inline
attachments, one question per phrase, paraphrases, yes/no per structure, the sentence alone as
state, per-word sense questions, inline definitions) left jev within its run-to-run noise of ±1.5.
The structural register rendered the verb-adjunct encoding as the subject's ("… and we with
«sequencing»"), which reads as nonsense, so jev attached every PP to the noun — right on most of
this page's pins. Rendered as the verb's, jev attached to the verb almost everywhere (structure
~31 → 28.75 of 41) and got 2 of 9 attachment units; one sonnet-4-6 draw got 7 of 9 (structure 33).

**The owner's design.** Show the alternatives as a grammar book does — the sentence bracketed where
the analyses group its words differently, and each contested phrase's grammatical function — and
ask «Which grammatical analysis of `the_sentence` matches what it means in `document`?», with no
parser in the question. Asking whether an analysis is *correct* scored lower with jev (15.5 and
17.0 of 27 structure calls, against 18.5 for *matches what it means*). The parser records how each
chart entry was built and returns that trace with the reading:

- `dcg::derivation` — a field on `Item`, stamped by the drivers (seeding, packed k-best's cube,
  `materialize_unary`, the unpacked CKY); each leaf records the sense atoms its tokens contribute.
  The sweep checks every reading of an ambiguous unit carries a well-formed derivation over the
  whole unit (667 of 667).
- `dcg::analysis` — an analysis brackets the constituents it builds and some other does not, and
  states the functions of the links it does not share: subject, object, prepositional object,
  object complement, noun modifier, adjective, predicate adjective, numeral, postmodifier, adverbial
  of a verb or of a predicated adjective, second predicate, adjective complement. A predication
  line separates a class generalisation from a statement about a kind. Phrases are named by the
  words of the leaf that introduced them in that reading. Analyses alike in both are one option.

```text
[1] We [ascertained MSI status] with sequencing.
      «with sequencing» is an adverbial of «ascertained»: it says how, where, when or why
[2] We ascertained [MSI [status with sequencing]].
      «with sequencing» postmodifies «status»: it says which or what kind of status
```

Screened (structure calls only, the 27 units whose pinned structure is offered, eight jev runs):
15.4 for the replaced form, 18.5 for a word-search prototype, 19.9 for the built form.

**The A/B** (three live draws per model, same snapshot and rankings, 12 new ledger rows, each draw
re-scored by replay):

| model | reading-correct | structure-correct | abstained |
|---|---|---|---|
| `jev-latest` | 28, 28, 27 | 31, 32, 30 | 3, 2, 3 |
| `claude-sonnet-4-6` | 24, 24, 24 | 32, 32, 32 | 0 |

The replaced presentation: jev 26, 26, 26 (structure 30, 30, 30); sonnet-4-6 26, 25, 26 (30, 29,
30). jev's abstentions are `none` in the sense call. Under a correct structure sonnet chose a wrong
sense on 8 units and jev on 5, the 5 shared; three of sonnet's are «analysed» v00644583, ruled out
by the maintainer on 2026-08-13.

**Disposition:** `jev-latest` stays the default; selection re-baselined at 28 on its first draw
(`2026-09-30-analyses-jev-latest-1`, 15d9bd7). The Structural register, the span-based surface
names and the structure contrasts are removed.

## 7q. One question per word in the sense call (`2026-10-01`)

**The owner's design:** an independent choice for each word whose sense differs among the chosen
analysis's readings, asked together. The decision interface carries several questions over one
context (`decision::Choice { context, questions }`): one TypeSafe request with a `choice` per id,
one Anthropic prompt whose reply schema has an answer per id. Each reading's senses come from its
derivation's leaves (`ReadingCandidate::senses_at`). The sense call asks «Which sense of «w» matches
what `the_sentence` means in `document`?» per word, each sense shown with its label and definition,
and takes the reading the answers support most (`Decided::weight`: the probability, or a falling
weight down a ranking). Readings that differ in no word's sense are put as whole readings.

The whole-reading question listed every combination of senses — 144 readings in one unit, near
TypeSafe's 255-option limit for one choice; per word, the same unit is a few short questions.

| sense call | reading-correct | structure-correct | abstained |
|---|---|---|---|
| whole readings (§7p draws) | 28, 28, 27 | 31, 32, 30 | 3, 2, 3 |
| one question per word | 28, 29, 28 | 31, 32, 32 | 0, 0, 0 |

Three live jev-latest draws, 5 new ledger rows, each re-scored by replay. The word question has no
`none`, so every unit is decided. One word question decided a structure: «MLH1» as the protein or the
gene modifying «promoter» gives the readings different skeletons in one analysis. Selection
re-baselined at 28 on the first draw (`2026-10-01-word-senses-jev-latest-1`, 13aa718).

**The structure call's errors, analysed** (six draws of §7p, 81 structure decisions per model;
per unit with the options shown, the models' answers and the evidence:
`d69-structure-call-errors.md`):
the pin was offered in all 27 calls; jev chose another structure in 25, sonnet-4-6 in 24, over 10
units. By cause: presentation 9 and 12, a pin or ledger row open to question 15 and 12, model error
under a clear presentation 0 and 0, one undetermined. 31 of the 49 are a multiword concept against
its decomposition. Proposed, for the maintainer:
- a concept taking a whole span («double-stranded DNA breaks» C1511667, «immune checkpoint blockade»
  C5392067) shows no bracket, because every analysis builds a constituent over that span — show it as
  one term, with a function line saying so;
- one policy for a lexicalised concept against its compositional twin (pins with alternates, or the
  structure diagnostic accepting `departs`), and a review of row 59 and the «MSI results from
  deficient DNA mismatch repair.» pin, whose stated ground («drops the compound») does not hold: its
  C1155661 covers «DNA mismatch repair»;
- «Many cancers exhibit an impairment of a DNA repair pathway.»: only readings taking «a DNA» as
  C0000702 «DNA, A-Form» match the pin, which the ledger's BEST row rejects — re-pin, and gate «a DNA»
  → C0000702 in the lexicon;
- the predication lines (kind against generalisation) in the sentence's words; a numeral line for
  `the_count`; a degree term rendered apart from a plain adjective;
- «for cancer therapeutics» (the open attachment) and the subject-oriented adverbial encoding;
- `◇A ∨ ◇B` against `◇(A ∨ B)` treated as one option, as the «WRN dependency …» pin note says.

## 7r. A multi-word concept as one marked term (`2026-10-01`)

The first recommendation of `d69-structure-call-errors.md` (P1). A leaf over several tokens naming one
concept is a term; analyses are compared by span and kind, so a term and a phrase composed over the
same words differ, and both are shown — the term as `⟨…⟩` with ««…» is one term, a single named
concept», the phrase in `[…]`:

```text
[1] Depletion of WRN induced ⟨double-stranded DNA breaks⟩.
      «double-stranded DNA breaks» is one term, a single named concept
[4] Depletion of WRN induced [double-stranded ⟨DNA breaks⟩].
      «DNA breaks» is one term, a single named concept
      «double-stranded» is an adjective describing «DNA breaks»
```

Screened (structure calls only, 27 units, eight jev runs): 19.4 before, 20.9 with terms, 20.0 with
terms and a note explaining the notation. The note swung near-tied units both ways (−8 on «Some
cancers do not respond …», +8 on «WRN dependency may require …») and is not sent.

| structure call | reading-correct | structure-correct |
|---|---|---|
| before (§7q draws) | 28, 29, 28 | 31, 32, 32 |
| terms marked | 28, 27, 28 | 32, 32, 33 |

Over the three draws: «Depletion of WRN promoted …» takes the pinned concept (structure +3);
«The use of immune checkpoint blockade …» +2 correct; «Some cancers do not respond …» takes the
pinned structure twice, with the «respond» sense the 2026-08-13 ruling rules out; «Defects in DNA
mismatch repair …» −3, where the term C1155661 is now visible and chosen against a compositional pin
— recommendation 2, for the maintainer. Selection baseline unchanged at 28, tracked on
`2026-10-01-terms-jev-latest-1` (0b26c2b).

## 7s. The sense ranker's arms — none beats S-guide2 (`2026-10-05`/`06`, closed)

The trigger: «We analysed data from large-scale silencing screens.» has no correct candidate. The sense
ranker keeps three verb senses of «screens» ('examine methodically', 'test or examine for the presence
of disease', 'examine in order to test suitability') and eliminates every noun, C0220908 «Screening
procedure» included — the §7f failure, still present. The parser then restores a noun by static
frequency (the CRT display, the projection screen). Each arm ran live on
`wordnet-umls-hpo-aligned-2026-10-05-merge` at 958525d and is scored by the SENSE RANKS line, against
the reading ledger, floor 0.02:

| sense ranker | words | right sense kept | right sense first | eliminated | readings | pins |
|---|---|---|---|---|---|---|
| S-guide2, `claude-sonnet-4-6` (the do-sense recording, replayed) | 216 | 214 | 191 | 2 | 679 | 62 |
| each option labelled with its part of speech (one draw) | 216 | 212 | 193 | 4 | 723 | 62 |
| `claude-sonnet-5-5`, thinking (three draws) | 216 | 214, 212, 214 | 179, 178, 181 | 2, 4, 2 | 442, 450, 511 | 61 |
| `claude-opus-5-5`, thinking (two draws) | 198, 195 | 196, 194 | 177, 175 | 2, 1 | 551, 470 | 61 |

- The part-of-speech labels left «screens» on the same three verbs and took «silencing» to its two
  verbs as well: the model matches on meaning and overrides the label. No sense in the lexicon is a
  genetic screen, and no gene-silencing concept is reachable from «silencing» (C0858952 is CHV's
  'silence', a Mental or Behavioral Dysfunction).
- The thinking models put the right sense first 10–16 words less often, lose the pin of «The use of
  immune checkpoint blockade can be limited by toxicity.», and keep the «silencing screens» failure
  (`sonnet-5-5` drops both noun senses of «silencing» in every draw).
- `claude-opus-5-5` refused 5 and 7 of its sense calls with `stop_details.category: "bio"`; those
  sentences fell back to seed order, so it ranked 18–21 fewer words. It cannot rank this page as asked.
- A third arm — `claude-sonnet-4-6` writing its reasoning before its choice — was stopped before its
  first draw finished. The reply schema puts `choice` before `rationale` because `serde_json` sorts
  keys (no `preserve_order`), so today's rationale is written after the choice; the arm renamed the
  field `analysis` to sort first.

None is adopted; S-guide2 on `claude-sonnet-4-6` stays. What remains is not a ranking question: the
lexicon has no genetic-screen or gene-silencing sense, and a wrong elimination is restored by
frequency rather than by the ranker's order.

## 8. What this does not fix

The negated sentence's forest is **308 readings cap-only vs 2 for the plain one** — a 154×
structural explosion from `did not have` before any ranking. Presentation cannot help that; it is
a grammar-side multiplicity question (do-support + negation generating many equivalent
derivations) and deserves its own investigation. Named here so the two are not conflated: D69
makes the choice answerable, it does not make the pool smaller.
