# D63 — single-skeleton wrong-reading defects (WRN first page)

Bucketing the 62 first-page units by structural-skeleton count (reranked, drift-free replay of
`2026-07-23-1533`): 21 units have a SINGLE skeleton. 17 were already faithfulness-pinned; the other 4
were checked here (the "easy pins" hypothesis). **Only 1 of the 4 was correct** — single-skeleton does
NOT imply correct, so the remaining 3 are wrong-single-reading defects, recorded below. Method: trace
with the recorded ranks + `EIGENIUS_GLOSS_READINGS=1`.

- ✅ **PINNED** — "We identified three groups of cell lines." A cardinal generalized-quantifier
  (`ΠG#0:Prop. ΠG#1:ΣG#1:§. prep_of(G#1, kind_of(§)). §(G#1.1, speaker) → G#0 → G#0`), verbalizer
  brackets Π-CPS by design; structure correct, cardinality "three" not encoded (same accepted caveat as
  "We analysed two … data sets"). Faithfulness 19 → 20.

## Defect 1 — predicative adjective + PP complement ("dependent on X")

"The lines from rare lineages were **less dependent on WRN**." → gloss *"the line from a rare line be a …
less **WRN protein**, human"*. `were less dependent on WRN` is mis-parsed: "dependent" (predicative
adjective) taking the PP complement "on WRN" is instead read as an ATTRIBUTIVE adjective on a head noun
"WRN" (→ "a … dependent WRN protein"), a predicate nominal. The comparative "less" and the copula "were"
survive, but the adjective's PP-complement frame (`dependent on _`) is not licensed. Likely a real
grammar gap (predicative-adjective subcategorized PP), not a sense/glossary issue — the WRN sense is even
correct (C1337007). 12 sense-variants, 1 (wrong) skeleton.

## Defect 2 — function-word "some" reified + "MSS" glossary miss

"**Some** MSI lines and **some MSS** lines were represented by these screening data sets." → gloss *"the
Disease Screening Data Set represent a **Some (qualifier value)** Microsatellite Instability line and …
a Some (qualifier value) **Marinesco-Sjogren syndrome** line"*. The verb structure is right (passive
`represent(data-set, lines)`, distributed over the coordination), but two argument defects:

- **"some" reification** — the determiner "some" seeds UMLS `C0205392` "Some (qualifier value)", piled
  into `compound_kind(MSI, C0205392)` — an EXTRA compound level, so the skeleton is structurally wrong,
  not just sense-wrong. Same family as the T078/T080 "and"/"For" reifications; the filter did not catch a
  qualifier-value determiner. Lever: extend the function-word / reification skip to determiner-colliding
  UMLS qualifier concepts.
- **"MSS" mis-grounded** — `C0024814` "Marinesco-Sjogren syndrome" (an abbreviation collision) instead of
  "microsatellite stable". "MSS" is not introduced with a parenthetical definition in the CNL, so the
  Schwartz-Hearst abbreviation glossary never binds it. Lever: a document glossary entry for MSS (a
  definitions-section / LLM abbreviation source, or the named-entity/acronym path).

## Defect 3 — coordinated predicate-nominal ("X are A, B and C")

"These groups are MSI lines, microsatellite-stable lines and indeterminate lines." → reading
`And(And(λG#0. the(group, MSI-line, G#0), λG#0. the(group, ms-stable-line, G#0)), λG#0. the(group,
indeterminate-line, G#0))` — a conjunction of three OPEN predicates (`λG#0. …`), which the verbalizer
cannot render (each conjunct bracketed). The predicative complement "A, B and C" builds a coordinated
predicate, but the subject "these groups" does not appear applied at the top — the reading looks like an
open predicate, not a closed proposition (yet the unit is bucketed parsed, not open). Needs structural
verification: is this the intended copula-predication of a coordinated NP complement, or is the subject
dropped? 10 sense-variants, 1 skeleton.

## Systematic analysis (confirmed by frame-probing)

### Defect 1 — root cause + scope (CONFIRMED broad)

Probes (cap-only, all structural readings):

- "WRN was essential." → `gt(essential(WRN), std)` — predicative gradable adjective works ALONE.
- "The gene was dependent on WRN." → `And(gt(dependent(gene), std), prep_on(gene, WRN))`.
- "WRN was essential for proliferation." → `And(gt(essential(WRN), std), prep_for(WRN, proliferation))`.
- "MSI is associated with responses." → `And(gt(assoc(MSI), std), prep_with(MSI, responses))`.

The PP is ALWAYS attached as a SEPARATE `And` conjunct `prep_X(subj, obj)` — "subj is Adj AND subj is
X-related-to obj" — never as the adjective's complement. **Root cause:** a WordNet adjective's sem is a
one-place gradable property `gt(deg_a(x), std_a)` with NO relatum slot, so "dependent"/"essential" cannot
consume "on WRN"/"for proliferation" as an argument; the copula's predicative-complement path conjoins
the adjective and the PP over the shared subject. The intended relational reading `dependent_on(gene,
WRN)` does not exist in the parse space. Recurs across: dependent-on / essential-for / associated-with /
dispensable-in / concordant-with — several of the ambiguity-tail units.

**Fix options:** (a) a rule that reinterprets `predicative-adjective + PP` as a two-place relation —
requires the adjective to expose a relatum, which the one-place WordNet sem does not, so this needs a
relational adjective encoding (deep); (b) accept the `And(gt(adj(subj)), prep_X(subj, obj))` conjunction
as the canonical CNL encoding, SUPPRESS the competing attributive reading (the "dependent WRN protein"
that beat it in unit 4), and let the And-reading pin. Difficulty: MEDIUM–HARD (a semantic-modeling
decision, not a local fix).

### Defect 2 — two independent sense/grounding levers (CONFIRMED)

- **2a "some" reification.** "Some cancers are common." → BOTH skeletons carry `compound_kind(cancer, §)`
  where `§` = `C0205392` "Some (qualifier value)": "some" reifies as a NOUN compounded onto the head, and
  the existential/determiner reading is ABSENT (no GQ/exists structure in any reading). Same family as the
  T078/T080 `and`/`For`/`each` reifications, but a qualifier-value colliding with a DETERMINER. **Fix:**
  importer-side skip of determiner-colliding qualifier concepts (extend the function-word filter) and/or a
  winning determiner entry for "some" (needs a reseed). Difficulty: MEDIUM.
- **2b "MSS" mis-grounded.** "MSS lines are common." is structurally fine (`subclass_of(MSS-line,
  common)`) but "MSS" grounds to `C0024814` "Marinesco-Sjogren syndrome" (an abbreviation collision), not
  "microsatellite stable" — MSS has no parenthetical definition in the CNL so the Schwartz-Hearst
  glossary never binds it. **Fix:** a document-glossary entry for MSS (definitions-section / LLM
  abbreviation source / acronym path). Difficulty: EASY–MEDIUM.

### Defect 3 — coordinated predicative complement (code read done: NOT a clear bug)

Code read (copula + predicative-nominal path). The copula (`is/are/was/were`) is
`(S[dcl,fin]\NP) / (S[dcl,adj]\NP)`, sem `λP.P` — it lifts an *adjectival predicate*. Predicate nominals
become that predicate two ways: `a_pred` → `λs. is_a(s, T)` (instance subject), `kind_nominal` →
`λK. subclass_of(K, T)` (kind subject). The **single** case uses this cleanly:

- "These groups are MSI lines." → `subclass_of(these-groups-kind, MSI-lines-kind)` — closed, correct.

The **coordinated** case does NOT: "These groups are MSI lines and MSS lines." →
`And(λG#0. the(group, kind_of(MSI-line), G#0), λG#0. the(group, kind_of(MSS-line), G#0))` — same under
"The" (so NOT D64 anaphora). Findings:

1. **Closed, not open.** The unit is bucketed *ambiguous* (closed), and the reading passed the felicity
   gate whose `⟦cat_s⟧ = Prop` type-check holds — so it IS a closed `Prop`, not an open-predicate bug.
2. **Different path.** The coordinated complement does not reuse `kind_nominal` (`subclass_of`); it routes
   through a **referential `the`-distribution** — `the(subject-class, restrictor, x)` = "x is the [group]
   that is an MSI-line", coordinated over the members. Likely the `coordinate_np` bare-kind path building
   a `cat_group` from "MSI lines and MSS lines", then a distributive predication, rather than coordinating
   the `subclass_of` predicates.
3. **Possibly a legitimate reading.** "These groups ARE [the] MSI lines, ms-stable lines and indeterminate
   lines" is an identity/enumeration; the referential-`the` reading may be *closer* to that than the
   generic `subclass_of`. The main symptom is that the verbalizer can't render it (each conjunct
   bracketed) and the `λG#0` form is non-canonical.

**Verdict:** Defect 3 is the LEAST clearly-broken of the three — a closed, plausibly-intended
referential reading that routes through a different construction and defeats the verbalizer. It is a
modeling/canonicalization question (should coordinated predicative nominals reuse the `subclass_of`
path? is the referential reading the intended one?), NOT an obvious crash/gap. Lower priority than
Defects 1–2. Difficulty: MEDIUM, but unclear payoff.

## Defect 1 — deep dive: existing machinery, forest trace, Fix A vs Fix B

### Shared root cause

Every preposition sem is a uniform VP-adjunct `λx. λV. λs. And(V(s), prep_X(s, x))` — the `And` is baked
into the preposition. The grammar does NOT distinguish a COMPLEMENT PP (subcategorized: "dependent ON")
from an ADJUNCT PP (circumstantial: "essential IN 2020"), so "dependent on WRN" →
`And(gt(dependent(gene), std), prep_on(gene, WRN))` — the PP is a separate conjunct, not the adjective's
relatum.

### The importer already has relational machinery — but it is unreachable for the corpus

`crates/eigenius-wordnet/src/convert.rs` (push_adj): gradable-vs-pertainym (WordNet `\`), a gloss-derived
`governed_preposition`, a 2-place `deg_{loc}_rel`, and a `cat_measure/cat_pp_arg(prep)` entry. Forest
trace (`EIGENIUS_TRACE_FOREST=all` on "The cell was more addicted to WRN than to MSI") confirms it
COMPOSES: `[4..6] addicted to wrn → cat_measure`, `[3..6] more addicted to wrn → (S[adj]\NP)/cat_pp_than`.
It BREAKS at `[7..9] than to MSI` (empty): `than` seeds `fwd(cat_pp_than, cat_np)` — it takes a bare NP,
but "to MSI" is a PP. So the relational reading is reachable ONLY for "more/less X **than [NP]**", which
the corpus never uses (the page has elided-than "less dependent on WRN" and positive "essential for X"),
and "dependent" has no frame anyway (gloss says "contingent on" / "addicted to a drug", not "dependent
on").

### Fix A — relational adjective sem (bind the relatum) — three pieces, all trace-confirmed

- (a) **Frame acquisition** — LLM proposer feeding `governed_preposition` (importer, reseed), replacing
  the low-recall gloss heuristic; the LLM also tags gradable-vs-relational → gradable-with-prep gets
  `gt(deg_A(subj,obj), std)`, non-gradable-pertainym-with-prep gets a NEW `adj_rel(A, subj, obj)` path
  (pertainyms get only `is_X` today, no PP argument).
- (b) **Elided-than** — a `(S[adj]\NP)/cat_measure` reading for `more`/`less` (anaphoric/absolute
  standard) so "less dependent on WRN" completes WITHOUT an explicit "than NP" (mirrors the synthetic
  comparative `cmp_attrib_sem`). **Highest-leverage single step** — reaches the actual unit-4 form.
- (c) **Positive relational** — adjective + PP-ground → `S[adj]\NP` directly, so positive "essential for
  X" / "associated with X" get a relational reading.
- Result: "dependent on WRN" → `gt(deg_dependent(gene, WRN), std)`, adjunct `And` gated away — correctness
  and a multiplicity win.
- **Risk:** multi-part grammar change (b, c) → regression surface (hold grammar-gap 0 + differential
  oracle); gating precision (a same-prep adjunct "dependent on Tuesday" could be misread as the ground);
  untrusted frames (fail-closed validation + record/replay); reseed cost; the pertainym `adj_rel` path is
  new surface.
- **Future:** the subcat-frame source GENERALIZES to verbs ("depend on", "represented by") — a reusable
  lexical-enrichment track alongside glossary/NER; later "than [PP]" for explicit relational comparatives,
  superlatives ("most dependent on"), adjective→source-verb frame linking.

### Fix B — accept the And-adjunct, suppress the worse competitor

- Keep `And(gt(adj(s), std), prep_X(s, obj))` as the canonical CNL encoding (already produced); suppress
  the competing wrong reading (the predicate-nominal "is a dependent WRN protein" that beat the `And` in
  unit 4). No importer change, no reseed, no LLM.
- Result: `And(dependent(gene), prep_on(gene, WRN))` pins as canonical; ambiguity reduced.
- **Risk:** semantic imprecision (generic `prep_on`, not tied to the adjective — a permanent
  approximation that cannot answer "what does the gene depend on?"); over-suppression (the
  predicate-nominal reading is correct for genuine "X is a Y"); papers over the complement-vs-adjunct
  distinction (band-aid per CLAUDE.md).
- **Future:** COMPATIBLE with a later Fix A (the `And` becomes the fallback for frameless adjectives; A
  adds the gated relational reading). A lightweight middle ground: tag `prep_on` as the adjective's
  argument without a full relational sem.

### Reference check — CCGbank on `than` (core-en has nothing)

core-en does NOT cover comparatives/`than` at all (adjectives are `n/n` + predicative + a measure
`np/np`; no PP complement, no degree machinery) — not a reference here. **CCGbank** (gold-standard CCG
over real WSJ text, `references/openccg/ccgbank/data/*.auto`) categorizes `than` as an OPTIONAL
POST-MODIFIER, not a complement of `more`/`less`:

```text
((S[adj]\NP)\(S[adj]\NP))/NP      than   — post-modifies an adjectival predicate, over an NP
((S[adj]\NP)\(S[adj]\NP))/S[inv]  than   — … over an inverted clause
(NP\NP)/NP   (NP\NP)/PP   (NP\NP)/S[pss] than  — nominal comparatives (incl. over a PP)
((S\NP)\(S\NP))/NP   PP/NP   conj         than  — verbal / other
```

Implications for Fix A's comparative half:

- `than [Y]` is an `X\X` post-modifier attaching to an ALREADY-COMPLETE `S[adj]\NP`. Our
  `more`/`less` = `((S[adj]\NP)/cat_pp_than)/cat_measure` makes `than` an OBLIGATORY forward complement —
  the non-standard choice, and exactly why "less dependent on WRN" (no `than`) gaps (the `/cat_pp_than`
  slot never fills). Standard CCG makes "more/less X" complete on its own → **elided-than is free** (= my
  piece (b), now reference-confirmed).
- CCGbank `than` also takes a **PP** (`(NP\NP)/PP`), so "…than on MSS" is standard — our NP-only `than`
  is the deviation.

So pieces (b) + the than-PP gap collapse into ONE reference-grounded change: refactor `than` to the
CCGbank optional post-modifier `((S[adj]\NP)\(S[adj]\NP))/{NP,PP,S}` and drop the `/cat_pp_than`
obligation from `more`/`less`. Aligns us with the gold standard and is the correct structural shape.

### (b′) prototype — VALIDATED end-to-end (reseed `wordnet-umls-aligned-2026-07-23-relcmp`)

Committed `9d2b4a3` (additive `more_deg_bare`/`less_deg_bare` = `(S[adj]\NP)/cat_measure`, anaphoric
standard). Reseeded (closed-class is `include_str!`-embedded → full drop-and-reseed + alignment) and
tested:

- **Mechanism works.** "The cell was less addicted to WRN." (no `than`; "addicted" HAS the gloss frame
  "to") → the relational reading `gt(deg_addicted_rel(WRN, x), deg_addicted_rel(WRN, anaphor))`. Before
  (b′) this was a hard GAP.
- **The reading is OPEN.** The anaphoric standard (`lexicon:anaphor` = the elided comparison target) is a
  referent hole, so `index.parse` (closed-only) returns 0 while the forest forms a complete
  `cat_s(dcl,fin)`; the open-aware carrier classifies it OPEN (holes=1). This is the honest analysis
  ("less … than [contextual standard]") and is CONSISTENT with `cmp_attrib_sem` (synthetic comparatives
  are open the same way). Decision point: keep OPEN (honest, awaits D64 resolution) vs an absolute closed
  standard (needs a per-measure `std` the general operator lacks).
- **Coverage-safe.** grammar-gap 0 on the aligned snapshot. (The base-snapshot gap on "…a stronger
  mutation phenotype" was an UNALIGNMENT artifact — that sentence has no `more`/`less`, and an ADDITIVE
  lexical change cannot turn a parse into a gap.)
- **No faithfulness regression.** The lone cap-only miss ("Each event alone does not lead to cell death")
  is a pre-existing cap-only artifact (missed in the prior named-entity cap-only run too), not from (b′);
  it hits under the reranker.
- **Effect on the page.** "The lines from rare lineages were less dependent on WRN" moved from a
  wrong CLOSED reading ("a … WRN protein", the Defect-1 gloss) to an OPEN comparative — the comparative
  structure is now correct; "on WRN" is still an ADJUNCT (relational complement awaits piece (a) frame
  acquisition, since "dependent"'s gloss yields no governed prep).

Net: (b′) is the right, reference-grounded fix and it works. Unit 4's on-WRN becomes the relational
complement only once piece (a) gives "dependent" the "on" frame; the comparative half is done.

### (a) frame acquisition — VALIDATED end-to-end (reseed `…relcmp-a`)

`governed_preposition` (gloss heuristic) is low-recall — "dependent"'s glosses ("addicted to a drug" /
"contingent on something else") have no "dependent on", so it got no frame. Fix: a committed
`crates/eigenius-wordnet/adjective-frames.tsv` (the high-confidence LLM-proposer output; four curated
from the page — dependent/on, essential/for, associated/with, concordant/with), `include_str!`-embedded
(crate-local so the Docker build context carries it) and consulted as a FALLBACK after the gloss. Adj
entries 464611→464666 (+55: the frame adjectives now emit their relational `cat_pp_arg` readings).

Witnessed (aligned relcmp-a, grammar-gap 0): unit 4 "The lines from rare lineages were less dependent on
WRN" is OPEN and its skeleton set now CONTAINS the correct relational reading

```text
λ$anaphor. gt(§(kind_of(§), $anaphor), §(kind_of(§), the(Σ. prep_from(_, kind_of(Σ. gt(§,§)))).1))
   ≈ gt(deg_dependent_rel(WRN, anaphor), deg_dependent_rel(WRN, the-lines-from-rare-lineages))
```

— WRN bound as the RELATUM (2-place `deg_rel`), the comparative complete (b′ elided-than), the standard
an abstracted parameter (the Π-representation), the whole thing a pinnable open skeleton
(open-skeletonizable). The full chain works: **(a)** frame → **(b′)** elided-than → **Π-abstraction** →
**open-skeletonizable**. The adjunct `And` readings still coexist (the frame ADDS the relational reading;
suppressing the adjunct is the than-post-modifier / Fix-B lever). Pinning unit 4 + re-baselining to
relcmp-a (a reranked re-record) is the remaining bookkeeping.

### Relationship / sequencing

NOT mutually exclusive. Fix B is the productive baseline; Fix A the precise refinement gated on frames,
`And` as fallback. Natural sequence **B → A**: B makes the `And` canonical + pinnable cheaply now; A then
activates the existing relational machinery incrementally, piece (b) first.

## Takeaway

Single-skeleton is a WEAK correctness signal — 3 of 4 checked were wrong. The faithfulness corpus grows
only on VERIFIED readings (now 20/20); these 3 units stay UNPINNED until fixed. Priority read:
**Defect 1** is the broadest (correctness + a multiplicity lever across the tail) but — per the forest
trace — a multi-part grammar+lexicon project (Fix A) or a productive approximation (Fix B), sequence
B → A; **Defect 2** is two clean sense/glossary levers; **Defect 3** needs a code read first (done: not a
clear bug).
