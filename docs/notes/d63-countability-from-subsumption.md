# D63 — Countability from the subclass lattice (unified across WordNet + UMLS)

**Status:** Design note (grounded, pre-implementation). Replaces the loose lexical **head-inheritance**
mass-shim ([d63-parse-gap-closure.md §6](d63-parse-gap-closure.md)) with a **subsumption-based**
countability rule over the shared `lexicon:Entity` lattice that both importers already populate. Motivated
by the ambiguity/parse-time blow-up the loose shim causes (the `gENE`→"extension" false positive). Related:
[d62-controlled-language-style-guide.md](d62-controlled-language-style-guide.md), the domain-lexicon-
injection alignment discipline.

**One-line problem.** "Is this noun grammatically *mass* (so a bare singular occurrence shifts to an NP
argument — `MSI contributes to cancers`)?" is currently decided by a **string heuristic** (the last word of
a UMLS concept's preferred name is uncountable) that over-generates. It should be decided by the noun's
**ontological category**, which both WordNet and UMLS already record.

---

## 1. The current shim and why it's imprecise

The importer emits a second `cat_n(C, mass)` entry for mass concepts so a bare singular parses like a bare
plural does. `concept_is_mass` ([`crates/eigenius-umls/src/convert.rs`](../../crates/eigenius-umls/src/convert.rs))
marks a concept mass iff **(a) head-inheritance** — the last word of its preferred name is in the
Wiktionary∩WordNet uncountable list ("microsatellite instability" → head "instability" is mass, so "MSI"
inherits it) — **or (b)** its semantic type is a process/function (`PROCESS_FUNCTION_TUIS`, 11 TUIs).

Branch **(a)** is a lexical string heuristic and over-generates. Canonical failure: `gene` acquires a bogus
`mass` reading because a junk UMLS atom `gENE` = "Gross Extranodal Extension" has the uncountable head
"extension". Every `gene` sentence then carries spurious `mass`-noun readings → the ambiguity + parse-time
inflation of §6/§7. Two planned filters (strictly-uncountable-head; acronym↔domain-word collision) are
*patches on the heuristic*, not a fix of the shape.

## 2. The shared substrate — one `lexicon:Entity` lattice, populated twice

Both importers already place every concept in **one subclass lattice rooted at `lexicon:Entity`** — this is
the unification point, and it exists today:

- **WordNet** ([`convert.rs`](../../crates/eigenius-wordnet/src/convert.rs) §doc): noun synset → `core:Class`;
  `@` hypernyms → `core:subclass_of`; `entity.n.01 ≤ lexicon:Entity`. So a WN noun carries the **full
  hypernym chain** (`methylation ≤ chemical_process ≤ process ≤ … ≤ entity`) — a deep lattice.
- **UMLS** ([`lib.rs`](../../crates/eigenius-umls/src/lib.rs) §doc): concept `C… ⊑` its semantic-type class
  `T… ⊑ lexicon:Entity`. **Flattened** — the TUI ISA hierarchy is not imported (v1), so UMLS is 2 hops:
  `concept ⊑ TUI ⊑ Entity`.

One lattice, two depths. That shapes the anchors below but not the rule.

## 3. The unified rule — count-*veto*, not replacement (corrected `2026-07-09`)

**First attempt (wrong, §8): pure subsumption `concept_is_mass(c) := is_subclass_of(c, mass-anchor)`,
deleting head-inheritance.** It regressed — see §8. Enumerating a *mass* anchor set means every omission is
a coverage loss, and scientific prose uses far more types as bare mass than "substance/process": diseases,
neoplasms, dysfunctions, deficiencies (`cause cancer`, `arise from Lynch syndrome`). Excluding
Neoplastic-Process T191 dropped `cancer`/`Lynch syndrome` mass and gapped the sentences.

**The corrected rule: keep head-inheritance for coverage; use the subclass hierarchy as a precision
*veto*.**

> **`mass(c) := type_mass(c) OR ( head_uncountable(c) AND NOT count_entity(c) )`**, where
> `type_mass` = the inherently-mass TUIs (substance/process/function, §4a), and `count_entity` = the
> discrete-count TUIs (Organism / Anatomical Structure incl. Gene/Cell / Manufactured Object / Finding /
> Lab-result / Model).

The hierarchy *gates* the heuristic's false positives instead of *reproducing* its coverage. This kills the
`gENE` collision — "Gross Extranodal Extension" (C5849123) is T033 Finding ∈ `count_entity` → its head
"extension" mass is vetoed — while `cancer`/`Lynch syndrome` (T191, not a count-entity, head "cancer")
keep their head-inheritance mass. It reuses the same `is_subclass_of` lattice the felicity oracle walks
([`category.rs:298`](../../kernel/src/dcg/category.rs#L298)), and it is **lower-risk**: the veto only
*removes* mass from types that are unambiguously discrete, never bets on a hand-enumerated *mass* list.

## 4. The anchors, per source

Only the **anchor set** differs by source (same rule). Each source already carries a coarse ontological
category tag that is the natural anchor granularity:

### 4a. UMLS — mass-denoting **semantic-type classes** (TUIs)

Because UMLS is flat, subsumption is one hop = "is the concept's TUI mass-denoting." Extend the current
process/function set (11) to the full mass-denoting block. The semantic-network branch is recoverable from
`MRSTY.RRF`'s **STN** (semantic tree number) field — no `SRSTRE` needed. Confident mass-denoting groups:

- **Processes / functions / activities** — the current set: `T038`–`T046` (the Function block),
  `T067`/`T070` (Phenomenon or Process). *(already live)*
- **Substances** — chemicals, body substances, amino-acid/nucleic-acid sequences (`DNA`, `RNA`): the
  Physical-Object→Substance branch (e.g. `T103`, `T104`, `T109`, `T114`, `T116`, `T120`, `T121`, `T123`,
  `T126`, `T127`, `T131`, `T167`, `T197` — **to confirm against SRDEF**, §Grounding).
- **Phenomena, attributes, quantitative concepts** — `T067`/`T070`/`T080`/`T081`/`T082` etc. (**confirm**).

Clearly **count**: Physical Object → Anatomical Structure / Fully Formed Anatomical Structure, Organism
(Gene or Genome `T028`, Cell `T025`, Organism, Body Part), Manufactured Object, Group. **Either** (need §5):
Disease or Syndrome, Neoplastic Process, Finding — a *disease* counts (`a cancer`) but is often mass in flux.

### 4b. WordNet — mass-denoting **supersenses** (`lex_filenum`)

WN's 26 noun **lexicographer files** are a coarse, enumerable category system (one per synset), the natural
anchor — parallel to UMLS TUIs. The `lex_filenum` is in the source format
([`wndb.rs:22`](../../crates/eigenius-wordnet/src/wndb.rs#L22)) but **not currently emitted** (the reader
skips it) — recording it is part of this work. Classification of the 26:

| verdict | supersenses |
|---|---|
| **mass** (anchor) | `noun.substance` (DNA, water), `noun.process` (methylation), `noun.state` (**instability → MSI**), `noun.phenomenon`, `noun.attribute`, `noun.feeling` |
| **count** (default) | `noun.animal`, `noun.person`, `noun.plant`, `noun.artifact`, `noun.object`, `noun.group`, `noun.location`, `noun.body` |
| **either** (→ §5) | `noun.act`, `noun.event`, `noun.cognition`, `noun.communication`, `noun.possession`, `noun.quantity`, `noun.relation`, `noun.shape`, `noun.time`, `noun.food`, `noun.motive`, `noun.Tops` |

This directly recovers the corpus's mass cases from the ontology: MSI/instability = `noun.state` → mass;
DNA = `noun.substance` → mass; methylation = `noun.process` → mass — with **zero** head-string dependence.
*(Alternative if we don't record `lex_filenum`: mark a handful of hypernym roots — `substance.n.01`,
`process.n.06`, `state.n.02`, `phenomenon.n.01`, `attribute.n.02` — and subsume through the hypernym chain.
The supersense is simpler and enumerable; the hypernym-root path reuses the existing lattice with no new
field. Decision D3 below.)*

## 5. The load-bearing caveat — grammatical ≠ ontological → **prior + override**

**Grammatical countability is not a pure function of ontological category.** `furniture`, `information`,
`advice`, `evidence` are grammatically *mass* under discrete/artifact/cognition branches; conversely some
process/event nouns count (`a reaction`, `three mutations`, `an event`). A subsumption-only rule *will*
misfire on these — which is exactly why the current WN path uses a curated per-lemma list.

So the design is **prior + override**, not subsumption-only:

1. **Subsumption is the default/prior** — `is_subclass_of(c, mass-denoting anchor)` → mass. Precise where
   ontology predicts grammar (nearly all UMLS technical concepts; most WN process/substance/state nouns).
2. **A curated per-lemma list overrides** the divergences (`furniture` mass despite `artifact`; `reaction`
   count despite `process`). Keep the Wiktionary∩WordNet list — but **demote it from primary signal to an
   override table** (and add a small count-override for count nouns under a mass supersense).

This split maps onto the two sources: **UMLS is dominated by (1)** (technical concepts align tightly to
their semantic types — the big precision win, and it retires head-inheritance), while **WordNet leans more
on (2)** (everyday nouns carry more grammatical idiosyncrasy). The `either` supersenses/TUIs are exactly the
rows where (2) decides; for them, subsumption abstains and the default is **count** unless the override lists
the lemma mass.

## 6. The clean endpoint (optional) — a shared upper-ontology category set

§4 has two anchor sets (TUIs, supersenses). The tidy endpoint aligns both to a small shared upper ontology —
`noun.process ⊑ upper:Process`, `T044 ⊑ upper:Process`, `noun.substance ⊑ upper:Substance`, … — and marks
countability **once** on `upper:{Process,Substance,Phenomenon,State,Attribute}` (mass) vs
`upper:{Object,Agent,Artifact,…}` (count). Then the rule truly unifies: one anchor set, both sources inherit
through it. Bounded mapping (26 supersenses + ~127 TUIs → ~a dozen upper categories); the same alignment the
domain-lexicon-injection discipline already calls for. **Recommended as a follow-on**, not a blocker — the
per-source anchors in §4 are usable immediately.

## 7. Implementation plan

1. **UMLS (biggest win first).** Replace `concept_is_mass`'s head-inheritance branch with TUI-subsumption
   against a `MASS_DENOTING_TUIS` set (extend the current 11 with the §4a substance/phenomenon/attribute
   groups). Keep branch (b) — it *is* this design, just widened. Measure the AMBIG/parse-time delta on the
   corpus (does the `gene`-family false-mass disappear? does S1/S2 ambiguity drop further?).
2. **WordNet.** Emit `lex_filenum` from the reader; add a `supersense → {mass,count,either}` map; countable
   default for `either`. Keep the Wiktionary list as the **override**, not the base.
3. **Shared `Countability` determination** consumed by both importers; the mass-denoting anchor set is the
   single reviewed artifact.
4. *(Follow-on)* the §6 upper-ontology alignment.

## 8. Verification

### 8a. First attempt — pure TUI-subsumption — REGRESSED (fail-closed, `2026-07-09`)

`concept_is_mass` was first rewritten to pure TUI-subsumption over a `MASS_DENOTING_TUIS` set (Substance
`A1.4.*` + Phenomenon/Process/Function/Dysfunction `B2.*`, **excluding** the "count-able" clinical entities
Disease T047 / Neoplasm T191 / Mental-Dysfunction T048 / Model T050), head-inheritance deleted. At the
importer level it looked clean: over the 2.5M-concept Metathesaurus it removed **89,200** count-entity
false positives (head-uncountable Findings/Diseases/Neoplasms — "…**test**", "…**cancer**", "…**dysfunction**")
including the `gENE` case, and covered the corpus's substance/process subjects (T049/T043/T044/T045/T114).

**But a reseed + parse-level measure caught a regression.** Deterministic (cap-only) over the affected
sentences: **old snapshot (head-inheritance) — GAP 0; new snapshot (pure-TUI) — GAP 2.** Both "Lynch
syndrome" sentences (`MSI can arise from Lynch syndrome.`, `…cause Lynch syndrome.`) parse on the old
snapshot and **gap** on the new one. Root cause: Lynch syndrome (C1333990) is **T191 Neoplastic Process**
— which I excluded — and its preferred name is "Hereditary Nonpolyposis Colorectal **Cancer**" (head
"cancer" ∈ the uncountable list). Head-inheritance had massed it via that head; excluding T191 dropped the
mass; `cause Lynch syndrome` (a bare object) then gapped. **Lesson: scientific prose uses diseases /
neoplasms / dysfunctions as bare mass, so a *mass* allow-list will always under-cover — and head-inheritance's
"imprecise" broad coverage was actually right for the corpus.**

### 8b. Corrected — count-veto — IMPLEMENTED (`2026-07-09`)

`concept_is_mass` (in [`crates/eigenius-umls/src/convert.rs`](../../crates/eigenius-umls/src/convert.rs)) is
now the §3 count-veto: **`type_mass OR (head_uncountable AND NOT count_entity)`**. `MASS_DENOTING_TUIS`
(inherently-mass substance/process/function — the type-mass branch, unchanged) + a new `COUNT_VETO_TUIS`
(discrete-count: Organism A1.1.\* / Anatomical A1.2.\* incl. Gene T028, Cell T025 / Manufactured A1.3.\* /
Finding T033 / Lab-result T034 / Sign T184 / Model T050), both grounded from `MRSTY` STN branches.

- **Precision, kept:** `gENE` = C5849123 is T033 Finding ∈ `count_entity` → head "extension" mass vetoed →
  no `gENE` mass form → collision gone. The count-entity false positives (Findings/Results/Models/discrete
  objects with uncountable heads) are removed by the veto.
- **Coverage, restored:** `cancer`/`Lynch syndrome` (T191, *not* a count-entity, head "cancer") keep their
  head-inheritance mass; `MSI`/`methylation`/`DNA` keep type-mass. The 8a regression is fixed.
- **Unit tests (green, 14):** `disease_neoplasm_stays_mass_via_head_inheritance` (**the regression guard** —
  T191 + head "cancer" → mass), `count_veto_kills_head_inheritance_false_positive` (gENE/T033 + "extension"
  → vetoed, no mass), `mass_concept_is_mass_by_semantic_type_not_head` (MSI T049), `count_entity_…` (Werner),
  `process_function_…` (T044).

**Parse-level measure — DONE (Derived, `2026-07-09`, deterministic cap-only, full v3 page over BOTH
snapshots, diffed).** Reseeded the count-veto → `db-snapshot/wordnet-umls-all-2026-07-09` (chain
diagnostics: `gENE` C5849123 mass **0**, Lynch syndrome C1333990 mass **16**, methylation C0025723 mass
**2**). Full-page cap-only:

| snapshot | encoded | ambiguous | GAP |
|---|---|---|---|
| head-inheritance (`07-08`) | 1 | 53 | **8** |
| count-veto (`07-09`) | 1 | 54 | **7** |

- **Regressions: NONE** — no unit gaps under count-veto that didn't already gap under head-inheritance
  (deterministic diff over all 62 units).
- **Fixed: 1** — *"These lines possess events that are predictive of MMR deficiency."* gaps under
  head-inheritance, parses under count-veto (removing spurious mass on discrete-count concepts un-crowded
  the cap-only beam). So the count-veto is a small **net gain** on the corpus (GAP 8→7), not merely neutral,
  on top of the lexicon-wide precision.

**Reranked (`--features use-llm`) tally — the authoritative config.** head-inheritance: 62 units →
**ENCODED 2 / AMBIG 55 / GAP 5**; count-veto: **ENCODED 1 / AMBIG 58 / GAP 3**. Gap diff: **zero
regressions, two fixed** — #8 (*predictive of MMR deficiency*) and #9 (*not simply a result of MMR
deficiency*) both gap under head-inheritance and parse under count-veto. So under the real config the
count-veto is **GAP 5 → 3** (the search-limited MMR-deficiency sentences reach their parse once spurious
discrete-count mass is removed). The ENCODED 2→1 dip is **reranker non-determinism** (cap-only is ENCODED 1
for *both* snapshots; ENCODED sits on the encoded↔ambiguous line and swings with LLM sampling — not a
gapped unit). **Net across both sweeps: the count-veto removes 1–2 corpus gaps, regresses none, and adds the
lexicon-wide precision.**

Verified clean across the `#102` (CheckError) merge: `cargo check --workspace` passes, the snapshot resumes
(no ManifestDrift), and the deterministic regression-fix holds (GAP 0 on the affected sentences). 14 unit
tests green. **The WordNet step (§4b) is deliberately NOT built** — analysis showed WordNet supersenses
don't predict sense-level countability (the `iron`/`furniture` problem), so its per-lemma list stays as-is.

## 9. Decision points

- **D1 — the mass/count/either calls.** The §4a TUI groups and §4b supersense table need a grounded review
  pass (§Grounding). The `either` set is where the override table does the work — enumerate the corpus's
  `either`-category nouns and check each.
- **D2 — the override table's role.** Confirm the Wiktionary∩WordNet list becomes the *override* (mass nouns
  under a count/either category) and add a *count-override* (count nouns under a mass category, e.g.
  `reaction`). Is one bidirectional table cleanest, or two?
- **D3 — WN anchor granularity.** Supersense (`lex_filenum`, emit a new field, enumerable) vs hypernym roots
  (reuse the existing lattice, no new field). Recommend supersense for simplicity; confirm the importer can
  emit it.
- **D4 — `either`'s default.** Count-unless-overridden (proposed) vs a per-category default. Count-default is
  safe (a spurious *count*-only noun just grammar-gaps as a bare singular — recoverable — whereas a spurious
  *mass* entry inflates ambiguity, the current failure).

## Grounding note

The confident classifications above (WN's 26 supersenses; the process/function TUIs already live) are used
as anchors. The **full mass-denoting-TUI enumeration is a grounding pass over the real UMLS Semantic Network**
(`SRDEF` definitions + `MRSTY` STN tree positions in `references/umls/`), **not** to be guessed — the
specific substance/phenomenon TUIs in §4a are marked *to confirm*. Never assign a TUI's countability without
reading its SRDEF definition.
