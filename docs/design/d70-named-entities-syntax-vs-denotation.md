# D70 — Named entities: proper-name SYNTAX vs named-individual DENOTATION

*Status: design — **§5 D1 and D3 MEASURED 2026-08-15; D2 answered for genes/proteins and unanswerable
for diseases on this corpus. §4 revised: O1 demoted to a stopgap, O2 made concrete by D3**. No code yet. Motivated by three independent failures
in the WRN-page measurement (2026-08-13…15) that turn out to be one axis: the UMLS importer decides
"is this a named individual?" with a single flag (`Concept::symbol`), and that flag simultaneously
fixes how a term BEHAVES SYNTACTICALLY (bare, no determiner, no plural) and what it DENOTES (an entity
vs a kind). For a gene locus those coincide. For a disease, a protein species, or any named class they
come apart, and the parser can only have one at a time. Two of the three failures were "fixed" by
mechanisms that paper over this — a mass entry standing in for proper-name syntax, and a recovery pass
substituting a synonymous concept to obtain one — so the cost is already being paid.*

## 1. Problem

`crates/eigenius-umls/src/convert.rs::render_concept_block`:

```rust
let named_tui: Option<&str> = c.symbol.as_ref().and(c.tuis.first()).map(|t| t.as_str());
```

A concept with a nomenclature symbol (HGNC and friends) is emitted as an **instance** of its primary
semantic type with `cat_np(TUI, sg)` entries. Everything else is emitted as a **class** with
`cat_n(CUI, num_any)` entries, plus an additive `cat_n(CUI, mass)` when [`concept_is_mass`] fires.

That one flag is doing two unrelated jobs.

**Syntax.** `cat_np` is a saturated NP: it stands bare, takes no determiner, has no plural. That is
proper-name behaviour, and it is a fact about the SURFACE — "WRN encodes…", "Lynch syndrome causes…",
"MSI is associated with…" are all bare, whatever those terms denote.

**Denotation.** `cat_np`'s sem is the concept used as an ENTITY. `cat_n`'s leaf denotes a KIND, which
a determiner then quantifies. That is a fact about the ONTOLOGY, and it is independent of how the term
is written.

Because one flag sets both, the lexicon offers exactly two packages — *(proper-name syntax + entity
denotation)* or *(common-noun syntax + kind denotation)* — and nothing else. The three failures below
are each a request for a combination that does not exist.

### 1a. WRN — the pin that cannot be satisfied

Measured 2026-08-14 (`reading-adjudications.tsv`, «Depletion of WRN induced double-stranded DNA
breaks.»). All 48 readings of that unit split on exactly one pair:

```
C1337007  «WRN gene»            cat_np(T028, sg)          → bare individual, ONLY
C0388246  «WRN protein, human»  cat_n(C0388246, num_any)  → kind_of(…), ONLY
```

The skeleton pin wanted the bare individual; the page's other pins use the PROTEIN for depletion and
activity and the GENE for dependency and essentiality. "Bare individual" and "the protein" cannot both
be had, because `C0388246` has no symbol and `C1337007` does. The pin was re-keyed to
`kind_of(C0388246)` on 2026-08-14 with the reasoning recorded — but that closed the ticket by choosing
which half to give up, not by making the combination expressible.

### 1b. Named conditions — proper-name syntax obtained by mis-marking countability

A disease has no nomenclature symbol, so it is a class with `cat_n`. To stand bare it needs the
additive `mass` entry, which [`concept_is_mass`] grants when the preferred name's HEAD is uncountable
or the semantic type is inherently mass. `MASS_DENOTING_TUIS` deliberately excludes diseases, and the
constant's own doc-comment names our sentence as the case head-inheritance is supposed to cover:

> Diseases/neoplasms are deliberately NOT here — they reach mass via head-inheritance … `cause
> cancer`, `arise from Lynch syndrome`; T191/T047 are absent here so their head "cancer" still masses.

Measured 2026-08-15, that theory fails:

```
C4552100  «Lynch Syndrome»                            → num_any only
C1333990  «Hereditary Nonpolyposis Colorectal Cancer» → num_any + mass
```

**The same disease.** HNPCC is the former name for Lynch syndrome. Head-inheritance reads the head of
whichever synonym UMLS chose as preferred, so countability tracks the NAME, not the condition. The
identical split holds for `C4522088` «Mismatch Repair Deficiency» (no mass) against `C0265325` «Turcot
syndrome (disorder)» (mass).

The deeper point: a mass entry is the wrong instrument here even when it fires. "Lynch syndrome"
stands bare because it is a NAME, not because it is a substance; marking it mass also predicts "*much
Lynch syndrome" and licenses it wherever a mass noun is licensed. The grammar is being told a falsehood
that happens to have the right consequence in one position.

### 1c. The cost is already being paid — a rescue that substitutes concepts

D69 §7k made `recovery_ranks` key on `cat_frame` (head constructor + the constructor's last argument)
so a word that lost a syntactic frame could be rescued without discarding the whole sentence's
ranking. Including the NUMBER index in that key means a lost `cat_n/mass` frame also triggers the
rescue's else-branch, which restores senses the ranker deliberately eliminated. Consequences measured
on the same page:

| span | ranker kept | rescue restored | verdict |
|---|---|---|---|
| «lynch syndrome» | C4552100 | C1333990 (has `mass`) | same disease, older name — **benign** |
| «mmr deficiency» | C4522088 | C0265325 (has `mass`) | «Mismatch Repair Deficiency» → **Turcot syndrome**, wrong |

One mechanism, one harmless outcome and one wrong one — and the harmless one is why it was recorded as
a success on 2026-08-15 without the substitution being noticed. **A parse should not have to swap in a
different concept to obtain a countability variant.** That the swap sometimes lands on a synonym is
luck, not soundness.

## 2. What is actually needed

Four combinations exist in the domain; the importer offers two.

| | entity denotation | kind denotation |
|---|---|---|
| **proper-name syntax** (bare, no det, no plural) | `WRN` the locus ✅ today | `Lynch syndrome`, `MSI`, `WRN protein` ❌ |
| **common-noun syntax** (needs a determiner) | — (rare; a named thing referred to descriptively) | `a helicase`, `two syndromes` ✅ today |

The empty cell is the one the corpus keeps asking for: **a bare-standing, kind-referring NP** — a
proper name OF a class. English has this productively (`Cancer is a disease`, `Lynch syndrome causes
colorectal cancer`, `MSI is associated with responses`), and biomedical prose is saturated with it.

Today that cell is reached only by mis-marking the concept as mass, and only when its preferred name
happens to end in an uncountable head.

## 3. Options

**O1 — Countability by semantic type.** Add T047 (Disease or Syndrome) and T191 (Neoplastic Process)
to `MASS_DENOTING_TUIS`, so bare-standing capability follows what a concept IS rather than what it is
called. Additive, so count uses survive. *Small, inside the existing design's intent, testable today.*
Does not address 1a, and still says "mass" where it means "name".

**O2 — Decouple the two axes.** Separate the syntactic decision (proper-name vs common-noun entries)
from the denotational one (entity vs kind), and let the importer set them independently:
nomenclature-symbol concepts keep entity denotation; concepts whose type is a named class (diseases,
protein species, cell lines) get proper-name SYNTAX with `kind_of` denotation. Requires a category for
the empty cell — either a new `cat_np_kind`, or `cat_np` with a `kind_of` sem, whichever the grammar
takes more cleanly. *Addresses all three failures. Bootstrap + importer change ⇒ reseed.*

**O3 — Per-concept curation.** Extend `atom-overrides.json` to force entries per CUI. *Rejected as the
primary fix: it is the "add a guard rather than eliminate the bad behaviour" pattern, and the set of
affected concepts is every named condition in UMLS.*

## 4. Recommendation — REVISED after the §5 measurements

The measurements moved this. The first draft recommended **O1 now, O2 later**; O1 is now the weaker
option and should probably be skipped.

**Done (2026-08-15): the number index is out of `cat_frame`.** Unblocked by any of this — concept
substitution to obtain a countability variant is unsound however the countability arrives. Measured
cost, booked in `baseline.json`: `expected-hits` 62 → 61, one unit back on `StaticFallback`. The
62/62 was resting on the unsound mechanism.

**O1 is now a trade, not a fix.** D1 shows a `mass` entry both grants bare standing AND licenses
`much X`, which is false for every named condition. Adding T047/T191 to `MASS_DENOTING_TUIS` would let
`C4552100` stand bare — fixing the concept selection in «Lynch syndrome causes cancer.» — while
extending `much X` licensing across every disease in UMLS. It swaps a wrong concept for a wrong
licensing. Take it only as a deliberate stopgap, with the cost stated.

**O2 is the fix, and D3 made it concrete.** The decoupling is not one new flag but two lookups at two
levels: syntax per FORM from `MRCONSO.TTY`, denotation per CONCEPT from the semantic type. Both
sources exist in data the importer already reads, and [`push_entries`] is already per-form, so the
seam is in the right place. It remains a bootstrap change ⇒ reseed, and it still needs the grammar
question settled: does `cat_np` accept a `kind_of` sem, or is a distinct category cleaner?

**Not yet answerable: whether diseases are kinds or entities (D2).** The gene/protein half is settled
by the gold set. The disease half is not, and this corpus cannot settle it — every candidate
instantiation phrase («patients with cancer», «tumours with MSI») is a RELATION, which is expressible
either way. Kind denotation stands as the unfalsified default, consistent with the protein evidence.
O2 does not depend on resolving it: the syntax half is independently measured, and it is the half both
§1a and §1b are blocked on.

## 5. What the experiment series already answers

These are not preferences. Two are largely settled by the adjudicated gold set, and the third is a
measurement nobody has run. Stated with the evidence, and with the confound where there is one.

### D1 — Are bare-standing capability and mass semantics the same thing? **ANSWERED: no. Measured 2026-08-15.**

Minimal pairs parsed against `wordnet-umls-aligned-2026-08-14-overrides-all` (cap-only, deterministic):

| input | readings | concept used |
|---|---|---|
| «Lynch syndrome causes cancer.» | 2 | **C1333990** — "Hereditary Nonpolyposis Colorectal Cancer causes Cancer" |
| «Much Lynch syndrome was observed.» | 2 | **C1333990** — "a much Hereditary Nonpolyposis Colorectal Cancer is observed" |
| «Much MMR deficiency was observed.» | 2 | **C0265325** — "a much Turcot syndrome (disorder) is observed" |
| «Much cancer was observed.» | 4 | n01977832 / C-cancer (acceptable English; the control) |

Two findings, and the first is the one that matters.

**Bare standing already routes through the wrong concept in ORDINARY PROSE.** «Lynch syndrome causes
cancer.» is grammatical, unremarkable, and involves no recovery pass, no widening and no fallback — and
it denotes the disease via `C1333990` rather than `C4552100`, because `C1333990` is the only concept
for that span carrying a `mass` entry. The concept-substitution seen under `recovery_ranks` (§1c) is
therefore a symptom, not the disease: **the mass entry is the only route to bare standing, so whichever
synonym happens to have one wins every bare use of the term.**

**Mass entries over-license.** «Much Lynch syndrome» and «much MMR deficiency» both parse. Those are
ungrammatical — a named condition cannot be quantified with `much` — so the `mass` marking is not
merely an imprecise way of saying "can stand bare"; it makes claims that are false and the grammar
acts on them.

Together these answer D1 **no**, and they price O1: adding T047/T191 to `MASS_DENOTING_TUIS` would let
`C4552100` stand bare (fixing the concept selection above) while extending `much X` licensing to every
disease in UMLS. That is trading one wrong for another, which moves O1 from "small fix inside the
existing design" to "stopgap with a known cost" — and strengthens O2, where bare standing comes from
proper-name syntax and carries no mass claim at all.

### D2 — Should proper-name syntax imply entity denotation? **Answered for genes/proteins; NOT for diseases.**

Occurrences across the 67 `correct` rows of `reading-adjudications.tsv` (2026-08-15):

| concept | entity positions | kind positions |
|---|---|---|
| `C1337007` WRN gene (HGNC symbol ⇒ `cat_np` today) | **10** (7 bare argument, 3 `compound`) | 0 |
| `C0388246` WRN protein (no symbol ⇒ `cat_n` today) | 0 | **11** (9 `kind_of`, 2 `compound_kind`) |
| all disease/condition concepts | **0** | **51** (24 `kind_of`, 27 `compound_kind`) |

For the WRN pair this is real evidence: BOTH denotations were in the forest, adjudication ranged over
both, and it chose entity for the gene and kind for the protein every single time, across 21
occurrences. So the answer for D2 is **no** — the protein needs bare-ish syntax with KIND denotation,
which is the missing cell, and no amount of syntax should drag entity denotation along with it.

**CONFOUND, stated because it undercuts the tidy row.** The gold is adjudicated over readings the
parser produced, so for diseases "0 entity positions" is not evidence — the entity reading was never
in the forest to be rejected. The disease row is consistent with kind denotation, not proof of it. The
WRN row does not have this problem, which is exactly why it carries the argument.

What WOULD settle the disease case: a sentence requiring INSTANTIATION — predicating a condition of a
particular, «this patient has Lynch syndrome», «two cases of Turcot syndrome». If instantiation is
needed anywhere in the corpus, diseases are classes and the question is closed. The WRN page may
simply not contain such a sentence, in which case this page cannot settle it and a broader corpus
must.

### D3 — Which concepts get proper-name syntax? **MEASURED 2026-08-15: no concept-level field predicts it — because it is not a property of the concept.**

Every term that occurs bare in the source prose, and what actually licenses it:

| term (all bare in the prose) | route to bare standing |
|---|---|
| `WRN`, `MSH2`, `MMR`, `PARP-1` | `cat_np(T028, sg)` — nomenclature symbol |
| `MSI`, `microsatellite instability`, `Lynch syndrome` | `cat_n(C, mass)` only |
| `project Achilles`, `project DRIVE` | no lexicon entry at all — injected by the page augmentation as named-entity individuals |

**Three routes, and the `symbol` flag covers only genes.** Everything else bare arrives through
countability (§1b's accident) or through document-level augmentation. So the rule cannot come from
`Concept::symbol`, and there is no other concept-level field that separates the rows — `Lynch
syndrome` and `microsatellite instability` differ in nothing at the concept level from terms that
require a determiner.

The reason the search fails is that **the question was asked at the wrong level.** Compare two forms of
ONE concept, `C0920269`: `MSI` stands bare, `microsatellite instability` is a common noun phrase. Same
concept, same TUI, same everything the importer looks at — different syntactic behaviour, because an
ABBREVIATION is name-like and its expansion is not. Proper-name syntax is a property of the SURFACE
FORM, not of the concept it denotes.

That is decidable from UMLS: `MRCONSO.TTY` distinguishes abbreviations (`AB`), preferred names, eponyms
and synonyms per ATOM. And the importer is already per-form — [`push_entries`] iterates a concept's
forms and emits one entry each — so the natural home for the syntactic decision already exists and is
in the right place.

**This sharpens O2 into two independent lookups rather than one flag:**

| decision | level | source |
|---|---|---|
| proper-name vs common-noun SYNTAX | per FORM | `MRCONSO.TTY` (abbreviation / eponym / symbol) |
| entity vs kind DENOTATION | per CONCEPT | semantic type, as today for `symbol` |

Under that split, `MSI` gets proper-name syntax with kind denotation; `microsatellite instability`
gets common-noun syntax with the same denotation; `WRN` gets proper-name syntax with entity
denotation; and `Lynch syndrome` gets proper-name syntax with kind denotation — the missing cell in
§2, reached without any mass claim.

## 6. Verification

O1: one `measure-parse-rate.sh` sweep. Gate — `expected-hits` must not drop and the miss-set must not
gain a unit; `probe_blocking_word` on «Germline mutations … cause Lynch syndrome.» must show the unit
resolving without `C1333990` being restored.

O2: acceptance is that «Depletion of WRN induced double-stranded DNA breaks.» can express *bare
individual + the protein* — the combination §1a shows to be unreachable — and that the 2026-08-14
re-pin can be revisited on its merits rather than forced by what the lexicon offers.

## 7. Related

- D63 §8.7 / `d63-countability-from-subsumption.md` §4a–§5 — head-inheritance and the count veto.
- D69 §7k — `cat_frame`, the recovery rescue, and the concept-substitution finding.
- D65 — lexicon identity and per-parse scoping; a new category lands in the same declared vocabulary.
- `experiments/parsing/reading-adjudications.tsv` — the «Depletion of WRN» rows and their re-pin
  reasoning; `baseline.json` `_provenance_note_2026-08-14-atom-overrides`.
