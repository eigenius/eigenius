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

## 7. Slices

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

**OUTCOME (`2026-08-15`) — fixed, and it closed the thread.** Two changes to `recovery_ranks`, both
making it see what the ranker can act on:

1. **Granularity.** New `seed::cat_frame` = head constructor + the constructor's LAST argument, used
   ONLY by `recovery_ranks` (`cat_shape` is untouched because `apply_sense_cap` keys on it). The last
   argument is the slot saying what a category TAKES or how it is INDEXED, which covers both observed
   losses: a slash's argument (`fwd/cat_cp` vs `fwd/cat_np` — «suggest») and a noun's number
   (`cat_n/mass` vs `cat_n/num_any` — «Lynch syndrome», where the ranker eliminated C1333990 and,
   because elimination is HARD, took its bare-noun `mass` entry with it).
2. **Scope.** `recovery_ranks` iterated single TOKENS while `contextual_sense_ranks` ranks SPANS, so a
   multiword blocker was invisible by construction. It now enumerates the same spans via `span_limit`.

| | before | after |
|---|---|---|
| units on `StaticFallback` (ranking discarded) | 3 | **0** |
| expected-hits | 61/62 | **62/62** |
| total-skeletons | 180 | 174 |
| total-readings | 594 | 594 |

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

## 8. What this does not fix

The negated sentence's forest is **308 readings cap-only vs 2 for the plain one** — a 154×
structural explosion from `did not have` before any ranking. Presentation cannot help that; it is
a grammar-side multiplicity question (do-support + negation generating many equivalent
derivations) and deserves its own investigation. Named here so the two are not conflated: D69
makes the choice answerable, it does not make the pool smaller.
