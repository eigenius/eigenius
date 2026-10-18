# D70 — Named entities: proper-name SYNTAX vs named-individual DENOTATION

*Status: **IMPLEMENTED 2026-08-15 — see §0.** §5 D1 and D3 MEASURED 2026-08-15; D2 answered for genes/proteins and unanswerable
for diseases on this corpus. §4 revised: O1 demoted to a stopgap, O2 made concrete by D3.* (The line
that followed, "No code yet", was left over from the pre-implementation draft and contradicted the
sentence before it; removed 2026-08-20. The implemented half is verifiable in the lexicon ontology,
the UMLS importer and the parser.) *Motivated by three independent failures
in the WRN-page measurement (2026-08-13…15) that turn out to be one axis: the UMLS importer decides
"is this a named individual?" with a single flag (`Concept::symbol`), and that flag simultaneously
fixes how a term BEHAVES SYNTACTICALLY (bare, no determiner, no plural) and what it DENOTES (an entity
vs a kind). For a gene locus those coincide. For a disease, a protein species, or any named class they
come apart, and the parser can only have one at a time. Two of the three failures were "fixed" by
mechanisms that paper over this — a mass entry standing in for proper-name syntax, and a recovery pass
substituting a synonymous concept to obtain one — so the cost is already being paid.*

## 0. OUTCOME (`2026-08-15`) — implemented, measured, 62/62

Both bare-standing failures fixed at source. The lexicon offered exactly one route to standing bare —
`lexicon:Num::mass` — and that value bundles "stands bare" with "is an uncountable substance", so
whichever concept happened to carry one won EVERY bare use of a term in ordinary prose, before any
ranking, widening or recovery ran. Two causes, two fixes:

| | cause | fix |
|---|---|---|
| `C4552100` «Lynch Syndrome» | no mass entry — its preferred name ends in a COUNT head, while `C1333990` «…Colorectal Cancer», the SAME disease, ends in a mass one | new `Num::name` — bare, kind-denoting, no mass claim — granted by `NAMED_CONDITION_TUIS` (T047/T191) |
| `C4522088` «Mismatch Repair Deficiency» | T033 Finding blanket-vetoed from head-inheritance, though its head «deficiency» IS uncountable ⇒ bare by NO route | T033 removed from `COUNT_VETO_TUIS`; its one motivating collision (`gENE`, C5849123) is handled per-atom by `drops.json` at 0.99 |

Measured on `wordnet-umls-aligned-2026-08-15-d70b`:

| | before | after |
|---|---|---|
| «MMR deficiency causes cancer.» | Turcot only | **C4522088** and Turcot |
| «Lynch syndrome causes cancer.» | HNPCC only | **C4552100** and HNPCC |
| «…not simply a result of MMR deficiency.» | 4 readings, C4522088 ABSENT | 8 readings, **C4522088 present** |
| expected-hits | 61/62 | **62/62, miss-set empty** |
| units on `StaticFallback` | 1 | **0** |
| total-readings / skeletons | 594 / 180 | 637 (ceiling 700) / **175** |

Everything the downstream chase was compensating for — the ranker overruled, Pass 2 discarding
rankings, `recovery_ranks` resurrecting eliminated senses — was a concept that could not compose in a
bare position. §5's D1/D3 measurements and §1d's determiner minimal pair are what located it; the
route through mechanism-checking rather than outcome-reading is recorded in §4.

ONE RE-PIN: «These lines possess events that are predictive of MMR deficiency.» encoded «MMR
deficiency» as a COMPOUND because the lexicalised concept could not fill that slot. It now resolves to
the concept directly — one skeleton, one token's difference, everything else byte-identical.

THE GATE THAT CAUGHT THE SWAP: the pre-re-pin sweep read `expected-hits 61/62` both before and after
while two units traded places. The miss-set diff — added 2026-08-14 after a scalar ratchet hid exactly
this — named both.

STILL OPEN: D2 for diseases (kind vs entity) is unanswerable on this corpus; O4 (term-type strength,
§1d) is unimplemented and orthogonal; abbreviations still reach bare standing via the glossary's
`mass` inheritance rather than `name` (§D3), which is the same conflation one level down.

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

### 1d. Term-type STRENGTH is computed and then discarded — probably the bigger lever

Measured 2026-08-15 against `MRCONSO.RRF` directly. The span «MMR deficiency» seeds two concepts, and
UMLS says plainly which one owns the string:

```
"Mismatch Repair Deficiency"   C4522088   NCI|PT  +  MTH|PN    ← the concept's own preferred term / name
                               C0265325   MSH|CE              ← Turcot syndrome, an ENTRY TERM
"MMR Deficiency"               C4522088   NCI|SY
                               C0265325   MSH|CE
```

`CE` is a cross-reference: MeSH lists "Mismatch Repair Deficiency" under Turcot syndrome as an entry
point, not as its name. `C4522088` holds the string as `PT` and `PN`. **The importer emits both as
equal surface forms**, so at seed time the two are indistinguishable and the collision that has been
chased through the sense ranker, the widen ladder and the recovery pass is, at source, a preferred
term competing with a cross-reference.

The precedence already exists and is already computed. `ConceptBuilder` parses MRRANK into
`(SAB, TTY) → rank` and scores every atom:

```rust
let score = rank * 4 + u32::from(a.ts == "P") * 2 + u32::from(a.ispref == "Y");
```

That score picks the concept's canonical NAME and is then dropped. Nothing reaches the lexical entry,
so `lexicon:LexicalEntry` carries no record of whether a form is a concept's preferred term or
somebody else's cross-reference.

**O4 — carry term-type strength onto the entry.** Emit the atom's strength alongside each entry and
let seeding prefer a concept's own `PT`/`PN` over another concept's `CE` for the same surface. No
grammar change, no bootstrap change, no new category — it is data the importer already reads and
throws away. It does not address §1a or §1b (which are about syntax, not concept choice), but it
targets the wrong-concept class directly, and «MMR deficiency» is the case it was measured on.

Cheaper than O2 and orthogonal to it. Sequence O4 first: it is testable on this page, and if it
resolves the collision then O2 is left facing only the syntax problem it is actually for.

**CORRECTION (same day): O4 does NOT fix §1b or the MMR case.** Minimal pair over the forest, cap-only:

| input | readings | concepts present |
|---|---|---|
| «…a result of **an** MMR deficiency.» | 8 | `C4522088` **and** `C0265325` |
| «…a result of MMR deficiency.» (bare) | 4 | `C0265325` **only** |
| «**An** MMR deficiency causes cancer.» | 4 | `C4522088` **and** `C0265325` |
| «MMR deficiency causes cancer.» (bare) | 2 | `C0265325` **only** |

The determiner is the switch. `C4522088` composes wherever a determiner licenses it and **cannot
appear bare at all** — it has no `mass` entry. So in the bare position there is nothing for term-type
strength to arbitrate: Turcot is the only candidate that CAN compose, and it wins by default rather
than by outranking anything.

O4 remains valid for the DETERMINED case — «of an MMR deficiency» seeds both, and there a concept's own
`PT`/`PN` should outrank another's `CE`. But the corpus sentence is bare, so O4 is not its fix.
Countability is: the bare position requires a mass entry, so every bare use of «MMR deficiency» in this
lexicon necessarily denotes Turcot syndrome, and the same holds for «Lynch syndrome» via C1333990
(§D1). That is O1's target, or O2's.

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

The measurements moved this three times in one day, and the pattern is worth naming: each reversal
came from reading an OUTCOME without checking the MECHANISM that produced it. The countability
hypothesis, O4's target, and D1's over-licensing claim were each corrected by looking at the forest or
the derivation instead of the aggregate. Current position, with the mechanism checked in each case:

**Done (2026-08-15): the number index is out of `cat_frame`.** Unblocked by any of this — concept
substitution to obtain a countability variant is unsound however the countability arrives. Measured
cost, booked in `baseline.json`: `expected-hits` 62 → 61, one unit back on `StaticFallback`. The
62/62 was resting on the unsound mechanism.

**O1 is cheap after all — the objection to it did not survive.** It was demoted on D1's
"mass over-licenses" reading; checking the mechanism showed `much` is an adjective and this grammar
implements no mass quantification, so a `mass` entry buys bare standing and nothing else measurable.
Its real cost is conceptual: it says `mass` where it means `name`, which will bite WHEN mass
quantification is implemented and not before. Against that, it is one list entry, it is additive, and
the determiner minimal pair (§1d) shows it is exactly what the corpus sentence needs — `C4522088`
composes wherever a determiner licenses it and cannot appear bare, so giving it a bare-standing entry
is the fix.

**O2 is still the principled fix, but its signal needs rechoosing.** D3's TTY rule does not cover the
motivating spans: «MMR Deficiency» is `NCI|SY` and «Lynch syndrome» is `PT`/`PEP`/`PN` — neither is an
abbreviation, so a TTY-driven rule leaves both unable to stand bare. TTY separates `MSI` from
`microsatellite instability` and is a refinement, not the lever. The lever is the SEMANTIC TYPE, the
same signal O1 uses, applied to syntax rather than to a mass claim.

GRAMMAR QUESTION, ANSWERED (`2026-08-15`): the denotation type is fixed by the category CONSTRUCTOR —
`⟦cat_n(T,num)⟧ = Set` (a kind), `⟦cat_np(T,num)⟧ = T` (an entity, `t.clone()`). So `cat_np` cannot
carry a `kind_of` sem; the felicity gate would reject it. But the missing cell does NOT need a new
constructor: `cat_n(T, mass)` plus the bare-mass shift (`kind_raised_nps`) ALREADY yields a bare,
kind-denoting NP. What is welded is the NUMBER value — `lexicon:Num::mass` bundles "stands bare" with
"is a mass noun". The cheap shape of O2 is therefore a fourth `Num` value (`name`) that takes the bare
shift without the mass claim, not a new category: one enum value in the bootstrap, one rule
generalisation, and the importer granting it by semantic type.

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

Two findings, and only the first survives scrutiny.

**Bare standing already routes through the wrong concept in ORDINARY PROSE.** «Lynch syndrome causes
cancer.» is grammatical, unremarkable, and involves no recovery pass, no widening and no fallback — and
it denotes the disease via `C1333990` rather than `C4552100`, because `C1333990` is the only concept
for that span carrying a `mass` entry. The concept substitution seen under `recovery_ranks` (§1c) is
therefore a symptom, not the disease: **the mass entry is the only route to bare standing, so whichever
synonym happens to have one wins every bare use of the term.** This is confirmed independently at the
forest level by the determiner minimal pair above (§1d correction).

**CORRECTED — "mass entries over-license" is NOT supported by this test.** The first reading of it was
that «Much Lynch syndrome» parsing proves the `mass` marking makes false claims the grammar acts on.
Checking the mechanism instead of the outcome:

| input | readings | why |
|---|---|---|
| «Much gene was observed.» | **0** | count noun, no mass entry ⇒ no bare NP to modify |
| «Much syndrome was observed.» | **0** | count noun |
| «Much helicase was observed.» | 2 | T126 Enzyme, inherently mass ⇒ bare NP |
| «Much Lynch syndrome was observed.» | 2 | bare via `C1333990`'s mass entry |

`much` resolves to the WordNet ADJECTIVE `a01610484` (`deg_a01610484` in the sem), not to a mass
determiner — this grammar has **no mass-quantifier rule**. So "much X" parses exactly when X can stand
bare, and the adjective attaches to the resulting bare NP. The test measures BARE STANDING, which is
the intended effect of a mass entry, and says nothing about mass semantics: there is no mass-specific
licensing here to over-fire, because none is implemented.

**Consequence for O1.** The cost that demoted O1 was this over-licensing, and it does not hold in this
grammar. A `mass` entry's only measured effect is bare standing plus whatever adjectival modification
a bare NP admits. O1 is therefore cheaper than §4 claimed, and the honest statement of its cost is
"teaches the grammar `mass` where it means `name`, which will matter WHEN mass quantification is
implemented, and not before".

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

VALIDATED against `MRCONSO` 2026-08-15: `TTY` separates an abbreviation from its expansion exactly —
`C0920269` carries `NCI|AB MSI` and `PDQ|AB MSI` against `NCI|PT`/`MSH|MH`/`MTH|PN` for the full form,
SAME concept. But it does NOT reach eponyms: `C4552100` «Lynch Syndrome» has no `AB` or `ACR` at all,
every form being `PT`/`PEP`/`PM`/`PN`/`SY`. So the TTY rule covers `MSI` and `MMR` and leaves named
conditions unsolved — D3 is answered for abbreviations and OPEN for eponyms.
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
