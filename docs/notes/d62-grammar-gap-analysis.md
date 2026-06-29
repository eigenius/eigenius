# D62 — grammar-gap analysis: why the 12 fully-known WRN units don't parse

*Analysis note. After the full-UMLS + closed-class/adverb batch, the WRN first page is **grammar-limited,
not vocabulary-limited** (`d62-encoding-prototype-findings.md`): 12 of 26 units are fully known yet
yield no parse. This note diagnoses the blocking constructions and maps each to a remediation in the
reference grammar (core-en OpenCCG, `references/openccg/grammars/core-en/`). Grading: the
construction inventory is **Declared** (linguistic inspection of the 12 units, not yet chart-instrumented);
the open/grammar split and the core-en families cited are **Derived** (measured / read from the grammar).*

## 1. First: they are *true* grammar gaps, not hidden open parses (Derived)

The harness classified via the **closed** forest only, so a unit yielding only an **open** parse (a
referent hole from `we`/`its`/pronouns, D64) would be misfiled as a grammar gap. Fixed the harness to
split `Open` from `GrammarGap` (parse via `parse_open`) and re-ran: **open = 0.** All 12 produce **no
full-span `S` at all** — closed *or* open. So the pronoun holes never even arise: the clause fails to
assemble before reference resolution is reachable. This is purely missing **grammar coverage**.

(A caveat on precision: the blocking constructions below are inferred by linguistic inspection, not by
instrumenting the chart to see the exact stall point. Confirmation = chart-max-span instrumentation, or
the ratchet — fix a construction, re-measure.)

## 2. The recurring uncovered constructions (Declared)

Each long unit needs *all* its constructions to compose; one gap kills the full-span parse. So the
leverage is in the **recurring** constructions. Ranked by frequency across the 12, with D63 status and
the core-en remediation:

| # | Construction | Units | D63 status | core-en remediation |
|---|---|---|---|---|
| 1 | **Apposition** — parenthetical `(MSI)`/`(PARP-1)`, em-dash appositive, comma-appositive naming (`the MMR genes MSH2, MSH6, …`; `data sets, project Achilles and project DRIVE`) | 1,2,3,6,9,11 | **none** | appositive comma (`punct.xsl:128`) + `RelPro-Appos` family (`misc.xsl:48`, `rel.appos`) + appositive PP (`pp.xsl:50`) |
| 2 | **Non-restrictive & pied-piping relatives** — `…, which results from…`, `…, which is caused by…`, `in which`, `through which` | 1,3,6,11 | restrictive `that`/`which` only (Slice 6-rel); no comma / no pied-piping | `RelPro-Appos` + `which` Wh entry (`dict.xsl:256`) + pied-piping via the relativizer (`misc.xsl:31`) |
| 3 | **Multi-item comma lists** — `X, Y, Z and W` (`colorectal, endometrial, gastric and ovarian cancers`) | 6,9,10,11 | binary left-branching `and`/`or` only | list-completion **typechanging** rules: `np-list-c` / `s-list` / `pred-adj-list` (`conj.xsl:157–263`) |
| 4 | **Numerals / measure determiners** — `14 cell lines`, `two data sets`, `0.56-fold fewer` | 1,2,9,11,12 | **none** (no numeral determiners) | numerals as determiners (`det.xsl`); measure phrases |
| 5 | **Fronted participial / adverbial adjuncts** — `Hypothesizing that …, we …`; `demonstrating that …`; `More commonly,`; `Thus,` | 3,7,8,9,10 | sentence adverbs `S/S`/`S\S` (just added) but not participial adjuncts or `thus`/`more commonly` | `s.from-1.fronted` (`cats.xsl:881`) + adverb Initial family (`adv.xsl`) + reduced relatives (`unary-rules.xsl:22`) for `-ing` adjuncts |
| 6 | **Predicate nominals + coordinated predicates** — `WRN is a vulnerability and … drug target`; `were distinct and contained …` | 5,12 | copula + predicative **adjective** only | copula `be` takes the predicate as an argument (`v.xsl:484` `copula.pred`; X and P both args) + `pred-adj-list` |
| 7 | **Long passives + light verbs** — `is caused by [agent]`; `give rise to`; `are needed` | 1,3,6,8 | short passive (existential agent) only | passive lexical forms (`v.xsl`); light-verb `give rise to` as an MWE |
| 8 | **`but not X` contrastive ellipsis** — `…, but not its exonuclease activity` | 4 | `but`→`And`; no negated-NP ellipsis | the `but` family (`conj.xsl`) with the elided predicate |
| 9 | **Deep PP-stacked / complex subject NPs** — `The success of … inhibitors in cancers with deficiencies in …` | 2,8,11 | PP noun-modifier exists; depth/subject-NP composition is the strain | (covered shape; stress-test once 1–5 land) |

Out of band: **U10 is also an S0 segmentation defect** — the sentence splitter over-merged two
sentences at `Fig. 1d, e.` (abbreviation period), producing a giant unit. A tokenizer/segmenter fix,
not grammar.

## 3. Prioritization (Declared)

Two axes: **recurrence** (unblocks many) and **nearest-unit** (unblocks a specific short unit now).

**Highest leverage — necessary across most units (do first):**
- **Apposition (#1)**, especially the parenthetical `(MSI)`/`(PARP-1)` gloss — it appears in the
  majority of units; nothing parses around it. core-en's appositive-comma + appositive-relative is the
  blueprint.
- **Multi-item comma lists (#3)** — core-en's list-completion typechanging rules are a clean, bounded
  add over our existing binary coordination.
- **Numerals (#4)** — small, self-contained determiner additions; recur across 5 units.

**Nearest wins — unblock a specific short unit (high morale / validates the ratchet):**
- **Predicate nominals (#6)** → unit 5 (`These findings show that WRN is a … vulnerability …`) is
  close once `is a NP` + coordinated-NP predicate work.
- **`but not X` (#8)** → unit 4 (the shortest) is close once contrastive ellipsis + the `of`-PP land
  (it would then be an **open** parse via `its`, not a grammar gap).
- **Fronted discourse adverbs (#5, partial)** → `thus` / `more commonly` (extend the lexicalized
  discourse-adverb set + degree `more`); helps units 7, 8.

**Then:** non-restrictive/pied-piping relatives (#2), fronted participials (#5), long passives (#7),
and finally stress-test deep NP stacking (#9). And the S0 abbreviation-merge for unit 10.

## 4. Note on method / next step

This is the *map*, not the *fix*. The honest next step before building is to **chart-instrument** one
or two units (report the maximal-span constituents the chart did build) to confirm the inferred stall
points — then take the highest-leverage construction (apposition) first and re-measure, letting the
ratchet validate each construction empirically rather than trusting the inventory wholesale.
