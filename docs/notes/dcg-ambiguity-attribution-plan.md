# D63 — Ambiguity attribution: make the forest report its own multiplicity root-cause

**Status:** IMPLEMENTED 2026-07-20 — `kernel/src/dcg/chart/attribute.rs` (`Forest::attribute` +
`UnitAttribution::render`), hooked in `parse/paths.rs` behind `EIGENIUS_TRACE_ATTRIBUTION`; unit tests
for the derived labels; validated on two WRN units below. **Motivation:** every multiplicity
diagnosis this session was manual forest archaeology — dump readings, `sed`-erase senses, hand-write a
swap-ladder to isolate a trigger, look up each CUI in `MRSTY`. It is slow AND it produced *wrong*
diagnoses (coordinated-modifier, N-N NF, item-1) that only dissolved after several traces, because the
instruments give the **split** (structure vs sense via skeletons/`sense×`) but never the
**attribution** (which word, which rule). The parser already knows both — we throw it away and
reverse-engineer it from pretty-printed λ-terms. This note specs a thin aggregator that emits the
attribution directly.

## 1. The substrate already exists (verified)

The packed shared forest is an AND-OR graph carrying everything attribution needs:

- **`PNode`** (`chart/forest.rs`) — an OR-node: `{ span: (usize,usize), rep: Item, edges: Vec<Edge> }`.
  Multiple `edges` = alternative derivations of the same span (structural ambiguity); the `rep` carries
  `cat` / `sem` / `prov`.
- **`Edge`** — the AND-hyperedges, each NAMING its rule:
  - `Leaf(Item)` — a seeded lexical entry. Several `Leaf` edges on ONE node = the word's competing
    **senses** (same `cat_shape`, packed together; the `sem` is the sense IRI).
  - `Binary { left, right, rule: BinRule }` — token-keyed rules, rule NAMED: `Coordinate(op)`
    (`logic:And`/`Or`), `Relativize`, `ButNot`, `AppositiveSubj/Obj`, `ApposeGroup`, `Reciprocal`.
  - `Unary { child, kind: UnaryKind }` — shifts, NAMED: `Raise`, `BareNp`, `FrontParticipial`,
    `AbsorbComma`, coordination list-completion.
  - `Combine { left, right }` — the general `apply`/compose/nominal-modification step; the rule is NOT
    on the edge, but the resulting node's `rep.prov()` is a **`Combinator`**: `ForwardApp` /
    `BackwardApp` / `ForwardComp` / `Compound` (all nominal-mod) / `TypeRaised` / `KindRaised` / `Modal`.
- **`Forest.cells[i][j]: BTreeMap<Sig, NodeId>`** — the CKY chart; the top-span cell holds the
  finite-clause roots. `kbest` (`chart/packed.rs`) already extracts distinct sems from a node.
- **`erase_senses`** (runs of ≥4 digits → `§`) and `pretty_term` already exist; the sweep already
  reports `total-skeletons` / `sense×`.

**What is missing is one thing: a per-node roll-up that names each multiplicity factor.** No
reading-count recursion over the forest exists today (only `kbest`).

## 2. The algorithm — one forest walk, two factor kinds

For a top-span root, a memoised post-order recursion computes, per node, its reading count AND the
LOCAL ambiguity it introduces:

```
readings(node) = Σ_{e ∈ node.edges} edge_readings(e)          // OR: sum the alternatives
edge_readings(Leaf(_))             = 1
edge_readings(Combine{l,r})        = readings(l) * readings(r)  // AND: product
edge_readings(Unary{c,_})          = readings(c)
edge_readings(Binary{l,r,_})       = readings(l) * readings(r)

local_factor(node) = readings(node) / max_{e} edge_readings(e) // >1 ⇔ this node BRANCHES
```

A node with `local_factor > 1` is an ambiguity site. Classify it by its edges:

- **all edges are `Leaf`** ⇒ **SENSE** ambiguity at span `[i..j]`; factor = #leaf-edges; the labels are
  the distinct sense IRIs (`node.edges` reps' `sem`).
- **edges carry `Binary`/`Unary`/`Combine`** ⇒ **STRUCTURE** ambiguity; factor = #distinct-rule edges;
  the label is the rule/construction of each edge (§3).

Attribution = collect every ambiguity site, weight it by its multiplicative contribution
(`local_factor`, propagated by how many top-readings flow through it), and rank. The reading count is
the raw packed count (pre-felicity/pre-dedup, so it over-approximates the extracted count) — fine for
finding DRIVERS; if the extracted count is wanted, intersect with the `kbest` sem set. `total-readings`
= `readings(root)` after the top-span felicity/dedup already applied by the sweep; the attribution runs
on the same forest so the driver ranking matches what the sweep counts.

## 3. Construction labels — from edge to human name

| edge | label |
|---|---|
| `Binary{rule: Coordinate(op)}` | `coordination(op, span)` — And/Or over the two conjuncts |
| `Binary{rule: Relativize / Appositive* / ApposeGroup / ButNot}` | that rule name + span |
| `Unary{kind}` | the `UnaryKind` name (`bare-np`, `type-raise`, `front-participial`, …) |
| `Combine`, `rep.prov() = ForwardApp/BackwardApp/Comp` | `application` / `composition` — usually not the interesting site |
| `Combine`, `rep.prov() = Modal` | `modal-scope` |
| `Combine`, `rep.prov() = KindRaised` | `kind-shift` |
| `Combine`, `rep.prov() = Compound` | **refine by the rep's restrictor axiom** (below) |

`Combinator::Compound` LUMPS N-N compound / named-compound / PP-noun-mod / attributive-adjective into
one variant. Split it by inspecting the rep's `sem` restrictor `Σx:C. R` App-spine head (the exact
logic already written in `drops.rs`-adjacent `is_adjective_refined` / `is_compound_refined`):
`compound_kind`/`compound` → **compound-bracket**; `measurements:gt`/`lt` (through `Ann`/`Lam`) →
**adjective-attach**; `prep_*` → **PP-attach**; `is_a` → **essive**. This is what turns "8 skeletons"
into "adjective-attach ×2 · compound-bracket ×2 · PP-attach ×2".

## 4. Sense-source classes — from sense IRI to junk-vs-genuine

For each SENSE site, classify every competing sense IRI so the report says WHY it multiplies:

- IRI prefix: `wn:n…` (WordNet) vs `umlscui:C…` (UMLS).
- A **cross-lexicon twin** = a `wn:` and a `umlscui:` sense in the same site denoting the same concept.
  (Rare on the WRN page — mostly already merged; flag it, don't assume it.)
- An **adjective-competing qualifier** = a UMLS sense whose surface is a WordNet ADJECTIVE and whose
  TUI is `Qualitative`/`Temporal`/`Spatial Concept` (the `Rare`/`Infrequent`/`Indeterminate` class the
  noun-collision drop pipeline can't see). TUI is read from the loaded concept class in the chain (the
  UMLS mirror) or `MRSTY`.
- else **genuine polysemy**.

This turns "sense× 30" into "verb ×2 (polysemy) · rare ×3 (1 adj + 2 qualifier-junk) · lineages ×2" —
which is exactly the manual CUI-lookup loop, computed.

## 5. Output — on the sweep, next to the histogram

Per AMBIG unit (behind `EIGENIUS_TRACE_ATTRIBUTION`, and always in `trace_one_sentence`):

```
[unit 45] «MSI is most commonly observed in … cancers»  45 = 5 skel × 9 sense
  STRUCTURE: participle-vs-verb(observed [4..4]) ×2 · adverb-attach(commonly [3..3]) ×2 · compound-bracket(MSI [0..0]) ×?
  SENSE:     observed ×2 polysemy · cancers ×2 (n+C, adjudicated distinct) · colorectal ×2 …
  TOP DRIVER: STRUCTURE participle/adverb (already-genuine) ; SENSE none-junk  → not a clean lever
```

A page-level roll-up (which construction / which sense-class contributes the most readings summed over
all 62 units) tells you where the NEXT lever actually is — the thing I inferred by hand and got wrong
three times.

## 6. Hook points, reuse, limitations

- **New:** `Forest::attribute(&self, tokens, roots: &[NodeId]) -> UnitAttribution` in
  `chart/attribute.rs` — the §2 recursion + §3/§4 labelling. `UnitAttribution { readings, skeletons,
  sites: Vec<Site> }`, `Site { span, kind: Sense|Structure, factor, labels }`.
- **Reuse:** the forest + `prov` + `BinRule`/`UnaryKind` (already there); `erase_senses`,
  `pretty_term`; the restrictor-axiom classifier from the drops work; the TUI lookup from the mirror.
- **Emit:** `wrn_first_page_over_full_lexicon` prints a `Site` summary per AMBIG unit under a flag +
  a page roll-up; `trace_one_sentence` prints it always. No parser/kernel behavior change — read-only
  over the forest the parse already built.
- **Limitations:** (a) the raw packed count over-approximates extracted readings (felicity runs only at
  the top span) — the ranking is still correct; intersect with `kbest` if exact per-site counts are
  needed. (b) `Combine`/`Compound` needs the sem-shape refinement (§3) since the edge doesn't name the
  rule — the one place the label is derived, not read. (c) cross-lexicon-twin detection needs the
  merge/candidate data (adjudicated-distinct vs unmerged) to avoid the overstatement I made — key it on
  the `alignment.jsonl` verdict, not on "wn+umls co-occur".

## 7. What shipped (2026-07-20) and how it validated

`Forest::attribute(&self, tokens, top) -> UnitAttribution` (the §2 memoised `inside_count` recursion +
reachability + per-node Sense/Structure site classification), `UnitAttribution::render` (raw-forest
header + ranked site list), hooked in `parse/paths.rs` next to the `EIGENIUS_TRACE_FOREST` hook (it has
both the forest and the finite-clause `top` roots) behind `EIGENIUS_TRACE_ATTRIBUTION`. Three unit tests
cover the derived labels (`compound_shape_label`, `axiom_class`, `span_text`); the forest-walk itself
rides the WRN `--no-llm` sweep differential.

Deferred from §4 (data-dependent, not wired): TUI-based qualifier classification and
`alignment.jsonl`-verdict twin detection. Sense sites currently print the raw IRIs; that already names
the driver. A page-level roll-up over the sweep (§5) is also not yet wired — only the per-unit block.

Validated against the actual reading dump (`trace_one_sentence`), attribution labels cross-check
exactly:

- *"These indeterminate lines are less dependent on WRN"* — `SENSE [2] «lines» ×4 :
  C0205132|C0700221|C1550648|n08430568` and `SENSE [7] «wrn» ×2 : C0043119|C0388246` are precisely the
  head-noun alternations in readings[0..19]. Confirms the manual finding: this unit is line-polysemy
  sense-driven.
- *"We compared MSI lines, microsatellite-stable lines and indeterminate lines"* — surfaces
  `SENSE [7] «and» ×2 : C1515981|C1550557` (the `RelationshipConjunction-and` reification the baseline
  note flagged as needing a function-word-skip extension) and `SENSE [8] «indeterminate» ×2` (adjective)
  automatically — the drivers I had found by hand, now read off the forest.

Both show the pre-felicity over-count (raw 80/4160 vs extracted 28/128) — labelled as an upper bound, per
limitation (a).

## 8. Page roll-up (§5) — shipped and measured 2026-07-21

`kernel/src/dcg/attribution.rs` (pub `begin`/`snapshot`/`take`; thread-local accumulator keyed by unit
tokens so the cap-widen ladder's retries OVERWRITE rather than double-count — unit-tested). Recorded
from the `paths.rs` hook, armed by the sweep. Exposed as `scripts/measure-parse-rate.sh --attribution`
(also added to the `env -u` strip list, so an ambient value can never enter a run silently) and
documented in `experiments/parsing/README.md` §7a. A partial roll-up is emitted every 10 units, so an
interrupted run still leaves data.

**Reference run** (`--attribution --replay 2026-07-20-1751/ranks.json`, CNL-v3, release, 31.8s):
metrics **identical to baseline — 0 / 10 / 931 / 326** with and without the flag, witnessing that the
instrument is read-only.

SENSE levers — **RETRACTED, see §9.** The first ranking was computed on the RAW forest and is void:
`has` ×7 (excess 10) · `project` 9 · `are` ×3 (9) · `classifications` 8 · `lines` 8 ·
`microsatellite-stable` ×5 (8) · `lineages` 7 · `and` (6) · `were` 5 · `rare` ×3 (4). From it I concluded
that copula/auxiliary reification (`has`/`are`/`were`) was the deepest sense driver. **That conclusion
was an artifact of counting a population the parser had already discarded.**

STRUCTURE levers: `compound` **47/62 units**, excess 282, max ×9 · `Relativize` 52 · `ApposeGroup+
coord(conn_list)` 50 · `BareNp+leaf` 26 · `adjective` 24; generic attachment 465 across 53 units.
**This table is an upper bound, not a lever ranking** (limitation (a)) — §6 measures the extracted
readings as sense-dominated, and the two count different populations. The felicity intersection is the
prerequisite for acting on it.

**Methodology cost, recorded so it is not repeated.** Three runs were wasted (~3h) producing
wrong-but-plausible numbers because the measurement was hand-rolled as `cargo test` instead of the
script: that misses `EIGENIUS_WRN_PAGE` (defaults to the ORIGINAL page, not CNL-v3 — 42-token sentences
instead of 5–11) and misses `--release` (debug ⇒ NbE stack overflow ⇒ **fake grammar gaps**, README §4
trap 1). The README already forbids hand-rolling and names the tell ("tens of minutes ⇒ debug"); the
1h20m/27-unit runtime was that tell, misread as pathological widening. Correct invocation, ~30s:
`scripts/measure-parse-rate.sh --attribution --replay <run>/ranks.json`. Cross-check a run is on the
expected page/profile via per-unit token counts and timings before reading any number off it.

## 9. Felicity intersection (2026-07-21) — and what it refuted

Attribution now runs **after** the top-span felicity filter and `subsume_duplicates`, in `parse_at_cap`,
and takes the surviving readings plus the layer:
`Forest::attribute(tokens, top, readings, layer)`. A sense site reports `surviving/raw`; a sense whose
identifying atoms (tokens with a ≥4-digit run — the same signal `erase_senses` uses) appear in no
surviving reading is marked `[pruned]` and excluded. Sense IRIs resolve through the chain to
`C0018905 "Hemagglutination test" [T059]` (`core:description` + the `core:is_a` → `umlssty:<TUI>`
parents), so the manual `MRCONSO` loop §4 promised to remove is actually removed.

**Verification of the two blockers I had asserted without checking.** §4's "data-dependent, needs
MRSTY / crosses a crate boundary" was **false** — the TUI is a parent class in the chain the parser
already holds. The felicity claim was **half true**: sense sites intersect cheaply (atom containment in
the surviving readings, no `kbest` change), but structure sites genuinely cannot — `kbest` returns bare
`Item`s with no derivation record and truncates per node, so bracketing intersection needs derivation
ids threaded through `kbest`/`cube`/`materialize_unary`. Structure is therefore still RAW and ranks
nothing; the report says so.

**What the intersection refuted.** On "This state has frequent insertion or deletion mutations", the
surface `has` seeds 7 noun/concept senses — hour-angle, `Hemagglutination test`, `Han Chinese`,
`Ha Antibody`, `rich_person`, UMLS `Possess`, UMLS `Have` — and **0 of 7 survive** (the verb site is
2 of 5). The felicity type-check already rejects every one. So the §8 finding that
abbreviation-collisions (N1), UMLS verb reifications (N2) and proper-name collisions (N3) were levers —
and the "noun-only alignment candidate rule is the root cause" conclusion drawn from them — is
**REFUTED**: those senses contribute zero multiplicity. They are real seeding junk, and irrelevant to
the tracked metrics.

**Corrected page levers** (`--attribution --replay`, 62 units, 27.5s, metrics unchanged 0/10/931/326):
`lines` 8 · `project` 7 · `microsatellite-stable` 7 · **`and` 5 units** · `are` 5 · `lineages` 5 ·
`arise` 4 · `rare` 4 · `regions` 4 · `analysed` 4 · `arises` 4 · `classifications` 4 · `deficiency` 4 ·
`screened` 4 · `essential` 3. `has`, `were`, `findings`, `observations`, `wrn` all leave the top 15.

This **restores the baseline's original hypotheses** and removes mine: H(a) genuine polysemy dominates
(`lines`, `project`, `lineages`, `regions`, `classifications`, `deficiency`); H(b) adjective-competing
qualifiers are real and survive (`rare`, 2 units, max ×3); H(c) function-word reification is real and
survives (`and`, 5 units — unlike `has`, its C1515981/C1550557 do reach readings). The baseline was
right; my §8 contribution was measurement error.

**Standing rule:** never rank a lever on raw-forest counts. The raw and intersected rankings disagree at
the top.
