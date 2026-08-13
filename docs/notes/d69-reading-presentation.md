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

## 8. What this does not fix

The negated sentence's forest is **308 readings cap-only vs 2 for the plain one** — a 154×
structural explosion from `did not have` before any ranking. Presentation cannot help that; it is
a grammar-side multiplicity question (do-support + negation generating many equivalent
derivations) and deserves its own investigation. Named here so the two are not conflated: D69
makes the choice answerable, it does not make the pool smaller.
