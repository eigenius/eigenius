# D63 — PP-attachment control (implementation scoping)

> ## ⛔ SHELVED — do NOT pursue PP-attachment control for the residual gaps (re-assessed `2026-07-09`)
>
> **Empirically refuted as the lever for the corpus's residual gaps.** Diagnosed the 3 residual reranked
> gaps (#3 passive, #4 V-as-Y+compared-to, #7 comparative+PP) with a fragment ladder + wide beam
> (`diagnose_residual_gaps`, `db_backed_encoding.rs`). Finding: **PP-attachment is not the driver.**
> - Every construction fragment PARSES, *including the PPs*: `…as a dependency compared to cells` CLOSED×176,
>   `…in cells` ×85, `…greater dependence on genes than counterparts` ×162, and the coordinated passive
>   `some lines and some lines were represented by data sets` ×6. The grammar attaches PPs (and handles the
>   passive / V-as-Y / comparative) fine — adding a PP raises the reading count but never causes a gap.
> - The **only** thing that tips each full sentence from CLOSED → GAP is swapping generic fillers for the
>   **domain terms** (`MSI cell lines`, `MSS counterparts`, `screening data sets`, `WRN`, `these four
>   lineages`). Those carry big **sense-products** and **multi-noun compound brackets** — that mass blows the
>   chart past the beam (10.8M items, then OOM at wide beam), not the PP structure.
>
> **So the residual is domain-term ambiguity (sense-product × N-N compound bracketing), and the candidate
> lever is an N-N compound-bracketing collapse + beam headroom — NOT PP-attachment.** Lever A here (typed
> pruning of PP slots) resolves a slice that already parses, so it would not move these gaps. Do not
> re-open this note as "the next lever" without new evidence that a PP-attachment *gap* exists.
>
> *(The below scoping is preserved for the record — the three levers and their code touchpoints are still
> accurate as a design, should a genuine PP-attachment problem surface later.)*

**Status:** design / scoping, pre-implementation (⛔ **shelved** — see banner above). This note specifies
*exactly what code each of three levers would touch* to add PP-attachment control to the DCG parser. It is
a scoping/design note, **not** an implementation. Every claim is grounded in the current code (file:line);
anything not verifiable from the code is flagged.

Follow-on to the packed-forest blueprint's deferred item **"PP-attachment control (L, NEW)"**
([d63-packed-forest-parsing-blueprint.md §Deferred, lines 353-355](d63-packed-forest-parsing-blueprint.md)).

## 1. Problem (witnessed)

PP-attachment is **genuine structural ambiguity**: the same surface string yields *different logical
forms* depending on where a PP attaches. It is categorically distinct from the two ambiguities the
existing machinery already handles:

- **Lexical / sense ambiguity** — same category shape, different synset/CUI fills a type-index. Collapsed
  by packing (the sense-product pile → one node; blueprint §2, §10b).
- **Spurious derivational ambiguity** — same reading reached two ways. Eliminated by normal-form
  constraints (Eisner NF via `Combinator` provenance, [parser.rs:312-318](../../kernel/src/dcg/parser.rs);
  the left-branching compound NF, [parser.rs:381](../../kernel/src/dcg/parser.rs) `!is_compound_refined`).

PP-attachment is neither. No grammar rule can delete it — the two readings are both well-formed and both
felicitous. It must be **resolved** (pick one) or **left underspecified** (represent the choice).

**The current behaviour is: produce both, unresolved.** Witnessed by
[`kernel/tests/closed_class_determiners.rs:1908` `pp_attachment_is_ambiguous`](../../kernel/tests/closed_class_determiners.rs):
for `"HeLa affects a cell line in HeLa"` the parser returns **≥2 parses** —

- the **VP-adjunct** reading `And(affects(…), prep_in(s, hela))` (top-level `And`), and
- the **noun-modifier** reading `Σx:CellLine. prep_in(x, hela)` (a Σ-refined object noun, no top-level
  `And`).

Both inhabit `Prop` (`assert_parses_to_prop`, [line 1934](../../kernel/tests/closed_class_determiners.rs)).

**Why it is the real bottleneck on the hard corpus.** Measured on the full WordNet+UMLS lexicon
([blueprint §10c, lines 267-284](d63-packed-forest-parsing-blueprint.md)): the prep-heavy WRN sentence
*"We analysed these data sets for genes that are selectively essential in cancer cells with MSI"* ran
**unpacked 221.6s vs packed 210.6s (~5%)** — both GAP. Packing's 8× win is scoped to *same-`cat_shape`*
sense-product piles; the `for … in … with …` chain spawns **many distinct cat-shapes** (each attachment
site is a differently-shaped constituent), which packing does not collapse (see §6 below). PP-attachment
control — not more packing — is what those sentences need.

## 2. Where the two readings are produced (the pipeline map)

Both readings come entirely from the **lexicon + the general combinators** — there is no PP-specific CKY
rule. The two attachments are seeded as two lexical categories for the same surface preposition, on
purpose:

**VP-adjunct preposition** (`in`/`for`/`with`/`to`/`on`/`from`),
[closed-class.esl:941-988](../../ontologies/lexicon/closed-class.esl):
- category `((S\NP)\(S\NP))/NP` = `fwd(bwd(bwd(S,NP),bwd(S,NP)), NP)`
- sem `λx.λV.λs. And(V(s), prep_in(s, x))` ([closed-class.esl:838-878](../../ontologies/lexicon/closed-class.esl))
- combines by **plain forward then backward application** in
  [`combinable`, parser.rs:319-342](../../kernel/src/dcg/parser.rs) — provenance `ForwardApp`/`BackwardApp`.

**Noun-modifier preposition** (`of` canonical; `in`/`for`/`with`/`on`/`from` *also* get an nmod entry),
[closed-class.esl:1035-1082](../../ontologies/lexicon/closed-class.esl):
- category `cat_pp / NP` = `fwd(cat_pp, NP)`
- sem `λy.λx. prep_in(x, y)` ([closed-class.esl:999-1033](../../ontologies/lexicon/closed-class.esl))
- combines via the **post-nominal refine** `RefineKind::PpMod`: `[cat_n] [cat_pp] → Σx:C. pp(x)`
  ([`combinable`, parser.rs:404-416](../../kernel/src/dcg/parser.rs); built by
  [`build_refine`/`refined_noun`, parser.rs:606-612 + 761-782](../../kernel/src/dcg/parser.rs)).

The lexicon comment states the intent explicitly:
> "a PP after a noun has BOTH attachments as separate parses (PP-attachment ambiguity, carried in the
> forest)" — [closed-class.esl:996-998](../../ontologies/lexicon/closed-class.esl).

Supporting machinery: `is_vp_adjunct_prep` distinguishes the two categories
([lookup.rs:2894-2897](../../kernel/src/dcg/lookup.rs)); the CKY loop calls `apply` at
[lookup.rs:1905 (unpacked)](../../kernel/src/dcg/lookup.rs) and on node representatives at
[lookup.rs:1458 (packed)](../../kernel/src/dcg/lookup.rs); the felicity gate is the only filter, run at
the full span ([lookup.rs:2395-2474](../../kernel/src/dcg/lookup.rs)).

### 2a. Load-bearing finding: an *accidental* attachment cost bias already exists

The noun-modifier refine returns `Combinator::Compound`
([`refined_noun`, parser.rs:780](../../kernel/src/dcg/parser.rs)), and `apply` adds
`COMPOUND_STEP_PENALTY` (=8) to `Cost::sense_rank` for every `Compound` step
([parser.rs:216-220](../../kernel/src/dcg/parser.rs)). The VP-adjunct reading is plain application
(no `Compound` step) and gets **no** penalty. **⇒ the noun-modifier ("low"/local, object-noun) attachment
already costs +8 more than the VP-adjunct ("high") attachment.** This is an existing, undocumented,
implicit *high-attachment preference* — it only bites under a beam / cube budget (an unbeamed `parse`
returns both), but on the operational-beam corpus sentences it already influences *which* attachment
survives. Any Lever-B design must account for it rather than assume a neutral baseline (see §4).

## 3. Lever A — type-based pruning via the felicity oracle

**Idea (Eigenius's distinctive lever).** An ill-typed attachment dies at the felicity gate: if a PP's
object type does not fit the slot it attaches to, the kernel `check` rejects that reading and only the
well-typed attachment survives. This is *resolution for free* — no cost model, no heuristic, just types.

**Where the gate is.** `reduced_felicitous` ([lookup.rs:2395-2402](../../kernel/src/dcg/lookup.rs)) and
`classify_felicitous` ([lookup.rs:2414-2474](../../kernel/src/dcg/lookup.rs)) denote the item's category,
evaluate its sem to normal form, and `check` it against the denoted type. A reading that fails `check` is
dropped (`ok()?`). This gate already runs on every full-span candidate; **no checker change is needed** to
make it prune attachments — the pruning is a *consequence* of the types on the slots.

**Why it is currently WEAK.** The gate can only reject an attachment if some slot is *narrow enough to
exclude the PP object's type*. Today every relevant slot is generic at the entity root:

- Verb argument slots: `FrameKind::arrow` / `FrameKind::cat` emit **`lexicon:Entity`** for every argument
  ([convert.rs:164-176 (arrow), 183-199 (cat)](../../crates/eigenius-wordnet/src/convert.rs); the const at
  [convert.rs:61](../../crates/eigenius-wordnet/src/convert.rs); module doc "argument types are generic at
  the noun root", [convert.rs:19-21](../../crates/eigenius-wordnet/src/convert.rs); the comment "every slot
  generic at the noun root", [convert.rs:164](../../crates/eigenius-wordnet/src/convert.rs)).
- Preposition slots: both the VP-adjunct and nmod prep sems/cats type every slot at `lexicon:Entity`
  ([closed-class.esl:838-988](../../ontologies/lexicon/closed-class.esl)).

`type_subsumes(Entity, X)` holds for the *entire* noun lattice (`entity.n.01` is rooted at `lexicon:Entity`,
[convert.rs:55-61](../../crates/eigenius-wordnet/src/convert.rs); the subsumption test is
[category.rs:261-266](../../kernel/src/dcg/category.rs), via `unify_type`
[category.rs:229-241](../../kernel/src/dcg/category.rs)). **⇒ every attachment is well-typed, so the gate
prunes nothing.** Both readings pass `check` (that is exactly what `pp_attachment_is_ambiguous` asserts).

**What makes it strong — stage-2 selectional typing (a DATA change, not a grammar change).** Replace the
`ENTITY_TOP` slot types with real domain types on the argument slots (e.g. `depends_on :
(S\NP_CellLine)/NP_Gene`, the pattern the demo lexicon already uses at
[lexicon.esl:152](../../experiments/lexicon/lexicon.esl) per [blueprint §4, lines 106-108](d63-packed-forest-parsing-blueprint.md)).
Then a mis-attachment whose object type violates the slot fails `check` at the gate and is pruned; only
the felicitous attachment survives. **Edit locus:** the importer emitters —
`FrameKind::arrow`/`FrameKind::cat` ([convert.rs:167-199](../../crates/eigenius-wordnet/src/convert.rs)) and
the `classify` frame map ([convert.rs:211-219](../../crates/eigenius-wordnet/src/convert.rs)) — would emit
concrete selectional slot types derived from the frame + a selectional-preference source; the prep entries
in [closed-class.esl:838-988](../../ontologies/lexicon/closed-class.esl) would carry typed object slots.
The kernel parser is untouched.

**Critical coupling — Lever A TRADES OFF against packing.** The packed router gates on the grammar being
*index-independent*: `cat_has_selectional_slot` ([category.rs:283-293](../../kernel/src/dcg/category.rs),
via `slot_is_concrete_nonentity` [category.rs:297+](../../kernel/src/dcg/category.rs)) returns `true` for any
functor argument slot that is a concrete class other than `Entity`. `parse_needs_unpacked` case (2)
([lookup.rs:1058-1071](../../kernel/src/dcg/lookup.rs)) then routes *any sentence seeding such a functor*
to the **unpacked** path (`routes_packed` = false, [lookup.rs:1080-1084](../../kernel/src/dcg/lookup.rs)).
So the moment stage-2 selectional slots exist, the sentences that carry them lose node-level packing. This
is *by design* (packing merely defers selectional pruning to felicity — blueprint §4 Option A — which is
only sound while slots are `Entity`-generic), but it means **Lever A and the packing win are mutually
exclusive on the same slot**. The pruning is still correct on the unpacked path; the question is cost.

**This is the same data/typing axis** as the countability lexicon (`MassNouns`,
[convert.rs:39-47](../../crates/eigenius-wordnet/src/convert.rs)) and the named-individual track — all are
"put the right type on the slot," not grammar edits. The WRN measurements repeatedly pointed at this axis
(GH#93 selectional restrictions; [d63-cnl-parse-levers-plan.md](d63-cnl-parse-levers-plan.md)).

**Fixes / leaves.**
- *Fixes:* eliminates ill-typed attachments entirely (resolution, not ranking) — the strongest possible
  outcome where selection is genuinely discriminating.
- *Leaves:* attachments that are *both* well-typed (e.g. `in cancer cells` could felicitously modify either
  a `Process` VP or a `Gene` noun if both accept a `CellLine` locative) remain ambiguous — types
  under-determine them. It also requires a selectional-preference data source that does not exist yet, and
  it disables packing on the typed sentences (§6).

## 4. Lever B — a preference / cost model over attachments

**Idea.** Rank the competing attachments by a cost and let the beam / cube extractor keep the preferred
one (e.g. a low-attachment / recency bias, the way `COMPOUND_STEP_PENALTY` biases against deep noun-piles).

**The cost infrastructure already exists** (and is the *only* new-plumbing-free part of this lever):
- `Cost { lexicon_order, sense_rank }`, additive & monotone
  ([parser.rs:83-115](../../kernel/src/dcg/parser.rs)); leaf cost from the entry's `sense_rank`
  ([lexicon.rs:138-142](../../kernel/src/dcg/lexicon.rs), [parser.rs:101-106](../../kernel/src/dcg/parser.rs)).
- `COMPOUND_STEP_PENALTY` = 8, summed per `Combinator::Compound` step in `apply`
  ([parser.rs:71 + 216-220](../../kernel/src/dcg/parser.rs)) — the exact precedent for an attachment penalty.
- Cube ordering by combined child cost (`CubeCandidate` `Ord`, [packed.rs:160-182](../../kernel/src/dcg/packed.rs);
  the cube loop [lookup.rs:1244-1290](../../kernel/src/dcg/lookup.rs)); the per-cell beam sorts by `Cost`
  (`beam_cell`, [lookup.rs:2921+](../../kernel/src/dcg/lookup.rs); applied at
  [lookup.rs:1849, 2244](../../kernel/src/dcg/lookup.rs)).

**But there is NO PP-attachment preference today** — only the *accidental* one from §2a
(`PpMod → Compound → +8`), which happens to penalise the noun-modifier (low) attachment.

**Exact edit sites** to add a *deliberate* attachment preference:
1. **Tag the attachment step.** The two attachments are already distinguishable at the combination site:
   the VP-adjunct is plain application over an `is_vp_adjunct_prep` category
   ([lookup.rs:2894-2897](../../kernel/src/dcg/lookup.rs); recognised in `combinable` at
   [parser.rs:319-342](../../kernel/src/dcg/parser.rs)); the noun-modifier is `RefineKind::PpMod`
   ([parser.rs:404-416](../../kernel/src/dcg/parser.rs)). To bias attachment, either (a) add a distinct
   `Combinator` variant (e.g. `PpAttach`) or a distinct `RefineKind` cost tier so `apply`
   ([parser.rs:216-221](../../kernel/src/dcg/parser.rs)) can add an attachment-specific penalty, or (b)
   give `PpMod` its own penalty separate from `COMPOUND_STEP_PENALTY` (today they share it, which is why
   the §2a bias is accidental and un-tunable).
2. **Inject the bias in `apply`.** The single choke point where a step's cost is finalised is
   [parser.rs:215-221](../../kernel/src/dcg/parser.rs) (`cost = left+right (+ penalty)`). A recency /
   low-attachment bias (prefer attaching to the *nearest* head) would be a penalty proportional to the
   attachment *distance* — but note: `apply` sees only the two items, **not their spans**, so a
   distance-based bias needs span information threaded to the combination site (spans live in the packed
   `PNode` [packed.rs:110-111](../../kernel/src/dcg/packed.rs) and the unpacked chart indices, not in
   `Item`). A *flat* attachment-type bias (VP-adjunct vs nmod) needs no span and is a one-line penalty; a
   *distance* bias needs span plumbing. Flag: the flat bias is the cheap, self-contained edit; the
   distance bias is a small structural change.
3. **Composition with sense-rank.** The penalty lands in `Cost::sense_rank` (the secondary key;
   `lexicon_order` is primary, [parser.rs:83-90](../../kernel/src/dcg/parser.rs)). It therefore composes
   *additively* with the existing sense-rank cost and is dominated by lexicon order — matching how
   `COMPOUND_STEP_PENALTY` is scoped ("large enough to dominate per-leaf sense-rank noise at a few steps,"
   [parser.rs:67-71](../../kernel/src/dcg/parser.rs)). No ordering change is needed; the derived `Ord` on
   `Cost` already sorts lexicographically ([parser.rs:76-83](../../kernel/src/dcg/parser.rs)).

**Fixes / leaves.**
- *Fixes:* gives a *tunable, deterministic* attachment ordering that the beam/cube keeps — turns a GAP
  (both survive, beam evicts the wrong one) into a preferred parse cheaply, and *first* fixes the §2a
  accidental bias by making it explicit.
- *Leaves:* it *ranks*, it does not *resolve* — the dispreferred attachment is still generated (cost is
  paid) and, unbeamed, still returned. A hand-tuned scalar bias is linguistically crude (the literature's
  lexical-association models — Hindle & Rooth 1993, *Structural Ambiguity and Lexical Relations*, CL 19(1)
  — condition on the verb/noun/prep triple; **citation to verify before adding to the bib**). It does not
  reduce the node count that dominates the hard sentences (§6) — it only picks a winner among the nodes
  already built.

## 5. Lever C — underspecification (represent, don't resolve)

**Idea.** Emit the *unresolved* attachment rather than guessing — a packed-UDRS-style disjunction node
(Dörre 1997, *Efficient Construction of Underspecified Semantics under Massive Ambiguity*,
[cmp-lg/9706028](https://arxiv.org/abs/cmp-lg/9706028), already grounded in
[blueprint §3, lines 44-47](d63-packed-forest-parsing-blueprint.md)). For a KG-encoding goal this may be
the *right* answer: the encoder records "the PP attaches here-or-there" and defers the choice to a later,
better-informed stage, instead of committing to a possibly-wrong logical form.

**What the machinery offers today.**
- The **packed forest already represents both attachments losslessly**: at the full span both readings are
  `cat_s(dcl, fin)` and (per the differential oracle,
  [`packed_forest_equals_unpacked_on_core_grammar`, closed_class_determiners.rs:1047+](../../kernel/tests/closed_class_determiners.rs))
  survive extraction. The top-span `PNode` ([packed.rs:108-117](../../kernel/src/dcg/packed.rs)) holds both
  as separate `Item`s in its k-best; the forest *is* an AND-OR structure. **So Lever C's representation
  substrate exists** — the missing piece is *emitting the packed node instead of the enumerated readings*.
- The **hole mechanism** (`OpenParse` / `HoleInfo` / `HoleKind`,
  [lookup.rs:2780-2811](../../kernel/src/dcg/lookup.rs)) is the current "represent, don't resolve" channel,
  but it is the *wrong shape* for attachment: a `HoleKind` (`EntityRef` / `Quantification`) is a *free
  variable to be filled with a value* ([hole_specs built at lookup.rs:1136-1152](../../kernel/src/dcg/lookup.rs);
  resolved by `resolve_open`, [lookup.rs:2484](../../kernel/src/dcg/lookup.rs)). An attachment ambiguity is
  a *choice among discrete structural alternatives*, not a value slot — so a `HoleKind::Attachment` bolted
  onto the same `substitute-a-value` resolver would be a category error.

**Two scoping options (flagged; a design decision, not decided here):**
- **C1 — emit the packed OR-node.** Add an output path that, for the top span, returns the packed `PNode`
  (or a distilled disjunction of the competing sems) rather than the flattened k-best. New output type
  alongside `Item` / `OpenParse`; `parse_packed` ([lookup.rs:1093+](../../kernel/src/dcg/lookup.rs)) would
  stop at the node instead of running the felicity pop-filter per reading. **This reuses the existing
  forest** and is the closest to Dörre. Downstream (the Deno/TypeScript orchestration encoding layer)
  consumes a disjunction node = "one of these logical forms" and records the underspecification in the KG.
- **C2 — a packed-UDRS sem node.** Introduce an underspecified sem constructor (an EigenTT term meaning
  "attach `pp` at one of {site₁, site₂}") built once at the combination site, materialised lazily. Heavier
  (new sem former + kernel typing rule for it), but produces a *single* felicitous term the gate can check,
  rather than a forest node the gate cannot type as a whole.

**Output-format change** (either option): the parse result becomes "closed | open | **underspecified**".
Today `parse_open` returns `(Vec<Item>, Vec<OpenParse>)` ([lookup.rs:988+](../../kernel/src/dcg/lookup.rs));
C1/C2 add a third arm the encoding layer must handle. This is a public-surface change and should ride a
deliberate API-shape decision (the CLAUDE.md "get the shape right" bar).

**Fixes / leaves.**
- *Fixes:* never guesses wrong; records the ambiguity as data (right for a KG that can carry
  underspecification); avoids paying to enumerate + rank readings.
- *Leaves:* pushes the resolution downstream — some consumer must still eventually choose (or the KG must
  natively support disjunctive attachment). Requires the biggest output-contract change of the three.

## 6. Interaction with the packed forest (verified)

**Attachment ambiguity = distinct cat-shapes ⇒ NOT collapsed by packing.** The packing signature is
`Sig = (cat_shape, Combinator)` ([packed.rs:36, `node_sig` at 39-41](../../kernel/src/dcg/packed.rs)), and
nodes are keyed per cell by `(span, Sig)` (`Forest::cells`, [packed.rs:123](../../kernel/src/dcg/packed.rs);
`get_or_create`, [packed.rs:136-148](../../kernel/src/dcg/packed.rs)). Two items merge into one node **iff**
they share span *and* `cat_shape` *and* provenance. The competing attachments produce constituents of
*different shape over different sub-spans* — a `cat_pp`-refined `cat_n` (object noun) vs a VP `S\NP` vs an
`(S\NP)\(S\NP)` adjunct — so they occupy **distinct `(span, Sig)` keys and are never merged**. Packing
collapses only the *sense-product within one signature*, not attachment alternatives. This confirms the
[blueprint §10c](d63-packed-forest-parsing-blueprint.md) framing against the actual node signatures.

Nuance worth stating: at the *full* span the two readings *do* share `Sig` (both `cat_s(dcl,fin)`) and pack
into one top node holding two sems — and both are extracted (the differential oracle). So packing
**preserves** the ambiguity (does not lose it) but also does **not resolve** it, and the intermediate
distinct-shape nodes are what make the corpus sentences expensive. Packing is therefore orthogonal to all
three levers: A removes readings before extraction (and disables packing on typed slots, §3); B reorders
what the cube keeps; C emits the packed node instead of extracting.

## 7. Recommended sequencing + highest-ROI lever

**Sequencing.**
1. **Lever B, minimal first — de-accidentalise the existing bias (§2a).** Split `PpMod`'s penalty from
   `COMPOUND_STEP_PENALTY` so the current implicit high-attachment preference becomes an explicit, tunable
   knob. Tiny, self-contained ([parser.rs:216-221 + 606-612 + 780](../../kernel/src/dcg/parser.rs)),
   fully within the kernel, immediately makes the beam behaviour on the corpus *legible*. Low risk, low
   cost, and a prerequisite for honestly evaluating the others.
2. **Lever C (C1) — emit the packed OR-node** for the KG-encoding path, since the forest already
   represents both readings; this is the correct long-term answer for an encoder that can carry
   underspecification, and it is *additive* (a new output arm, not a rewrite).
3. **Lever A — stage-2 selectional typing** as the deep fix, sequenced with the countability / named-
   individual data work it shares an axis with, accepting the packing trade-off (§3) on typed sentences.

**Single highest-ROI lever: Lever A (type-based pruning via stage-2 selectional typing).**

*Rationale, grounded in the measurement.* The WRN measurements repeatedly localise the residual to a
**data/typing axis, not a grammar axis** ([blueprint §4 lines 97-108](d63-packed-forest-parsing-blueprint.md);
[d63-cnl-parse-levers-plan.md](d63-cnl-parse-levers-plan.md); the memory notes on selectional typing and
countability). Lever A is the *only* lever that **resolves** rather than ranks or defers: an ill-typed
attachment ceases to be generated at the felicity gate that already runs
([lookup.rs:2414-2474](../../kernel/src/dcg/lookup.rs)) — no new scoring, no new output contract, **zero
kernel-parser change**; the work is entirely in the importer emitters
([convert.rs:167-219](../../crates/eigenius-wordnet/src/convert.rs)) and lexicon slot types. It is also the
lever that uses Eigenius's *distinctive* asset — a real dependent-type checker as the felicity oracle —
which no purely-statistical parser has. Its two honest costs are (a) it needs a selectional-preference data
source that does not exist yet, and (b) it disables node-level packing on the typed sentences (§3, the
`cat_has_selectional_slot` router gate). Both are acceptable: (a) is the same data investment the roadmap
already commits to, and (b) trades a same-shape-pile optimisation (which by §6/§10c *doesn't help these
sentences anyway*) for the actual bottleneck fix. Lever B is the cheapest *stopgap* and should land first
for legibility; Lever C is the right *representation* for the encoder; but Lever A is where the durable
win is, because it attacks the ambiguity at the level (types) the measurements keep pointing to.

## 8. Grounding hygiene

- Dörre 1997 (underspecification packing) — already grounded, [cmp-lg/9706028](https://arxiv.org/abs/cmp-lg/9706028)
  ([blueprint §3](d63-packed-forest-parsing-blueprint.md)).
- Hindle & Rooth 1993, *Structural Ambiguity and Lexical Relations*, CL 19(1) — cited in §4 as the
  canonical lexical-association PP-attachment model; **DOI/anthology id to verify** before adding to
  [docs/references/eigenius_related_work.bib](../references/eigenius_related_work.bib). Frazier's Minimal
  Attachment and Kimball's Right Association (the psycholinguistic high/low-attachment principles behind a
  §4 bias) are likewise **to verify**, not asserted here.
