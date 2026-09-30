# Eigenius Controlled English — a style guide for parser-faithful scientific prose

*A controlled natural language (CNL) for writing factual scientific claims that the Eigenius DCG/CCG
parser fully covers, so the encoding captures the **claim** (a kernel-checked `Prop`), not an
approximation. Grounded in the parser's actual capabilities, not aspiration.*

*Originated as a D62 experiment (`2026-06-29`) that rewrote the WRN first page into this style and
measured the coverage change. Moved here `2026-08-19` and trimmed to the guide: the experiment log it
carried is superseded — the corpus now parses 62/62 with `grammar-gap 0` and `missing-lexeme 0`
(`experiments/parsing/baseline.json`), so the June coverage figures describe a parser that no longer
exists. **Corrections applied in the same pass are marked ⚠ below.** This is an authoring guide, so it
is expected to drift as the grammar grows; check a claim against the baseline before relying on it.*

*Revised `2026-09-20` for measured quantities, and `2026-09-27` when D93 (units of measure) and D95
slices 1–5 (quantities in the parser) made them parse. The claims about them were measured over the
full lexicon (`wordnet-umls-aligned-2026-09-27`, cap-only). **What is still marked 🔜 is NOT live**: a
temperature or any other difference needs D95 slice 7. This guide does not state aspiration as
capability.*

*Revised `2026-09-29` for D95 slice 6 — bounds, approximations, ranges, bound symbols and scientific
notation parse — and for the owner's decision that **a plain number is exact**: a bound is written
out (Measured quantities, rules 7–9). The slice-6 claims are checked by `kernel/tests/` over the
bootstrap chain; the full-lexicon run waits for the reseed after D95 slice 8. Revised again the same
day for D95 slice 7: counts, proportions, number words, a determiner before a numeral, and counted
conjuncts sharing a head (DO item 3; Measured quantities, rule 10); and for slice 8: offsets
(`72 h after transduction`), a deadline (`by three weeks`), `every N unit`, the pseudo-partitive, and
a fronted adjunct (DO items 11 and 13; Measured quantities, rule 11).*

## Purpose & posture

The parser is the oracle: a sentence either composes into a kernel-checked typed tree or it does not.
Rather than bend the grammar to arbitrary journal prose (long, compound, statistic-laden), **write the
science in the subset the parser covers**. This matches the encoding objective — we want the
*load-bearing factual claims* as checkable `Prop`s; rhetorical packaging, inline statistics, and
citations are out of the claim by design (D62 S0 routes them out).

Two rules sit above everything else:

- **(R1) One claim per sentence.** Almost every grammar gap below is dissolved by splitting a compound
  journal sentence into several short factual ones.
- **(R2) Faithfulness over parseability — never drop a *qualifier* to make a sentence parse.** A
  simplification may drop *data* (numbers, citations, figure refs — out of the claim by design), but it
  **may not** drop a word that changes the claim's **strength, scope, or modality**: modals
  (`can`/`may`), scalar/comparative adverbs (`preferentially`/`selectively`/`typically`/`highly`),
  scope restrictions (`the four RecQ helicases`, not `the helicases`), or severity/type specificity
  (`double-stranded`). If keeping a qualifier means the sentence does not yet parse, **keep it anyway
  and record the gap** — a faithful un-parsed claim is a tracked to-do; a parsed distorted claim is a
  silent error (the D61 faithfulness gap). See the audit + rule at the end of this note for why.

## DO — constructions the parser covers

1. **Subject–verb–object, one clause.** `WRN is essential in MSI models.` `Depletion of WRN promotes
   apoptosis.` Present tense (`affects`/`affect`) or simple past (`affected`, `was`/`were`).
2. **Predicate nominals & adjectives.** `WRN is a vulnerability.` `WRN is a drug target.` `The
   dependency is selective.` Copula present/past: `is`/`are`/`was`/`were`.
3. **Determiners.** `a`/`an`/`the`/`every`/`each`/`all`/`some`/`no`, and numbers — `one` … `ten` in
   words, any number in digits (`three genes`, `3 genes`, `1,000 cells`), which read alike.
   - **A determiner before a number**: `the four other RecQ helicases` states the four; `these two
     genetic events` points back at exactly two earlier referents. Today the only set a demonstrative
     resolves to is a run of findings of one kind (D68), so `these four lineages` after four named
     kinds stays open — write the NP out (`the four lineages`) where no such run precedes it.
   - **Counted conjuncts may share their head**: `five MSS and five MSI cell lines` is five of each,
     and `15% of colon, 22% of gastric and 12% of ovarian cancers` a proportion of each. Each conjunct
     is a number or a proportion with the modifiers of its own kind; the head is written once.
   - **A plain count is exact** — `5 MSI cell lines` states five (decided `2026-09-29`; every plain
     count in the WRN paper is exact), as `has_count(…, 5)`. Write a bound out: `at least 1,000
     cells`, `more than one MMR gene`, `5 or more cell lines`, `fewer than 5 cells`, and a count range
     with an en-dash, `4–7 foci` (rule 7 under Measured quantities).
   - A bare number is a count, not a measurement: `incubated at 37` has no parse — write the unit.
   - ⚠ **Bare plurals and bare mass nouns now CLOSE, not open.** `Cancers exhibit defects.` composes as
     a kind predication (`kind_of`), not as a deferred quantifier. Prefer the bare form for
     kind-level claims — it is the shorter and the closed one.
   - ⚠ **Demonstratives are anaphoric, not determiners.** `this`/`that`/`these`/`those` open a
     RESTRICTOR-TYPED referent hole (`lexicon:anaphor_of`), so `These findings show …` resolves only
     against an antecedent that is itself a finding, earlier in the same document. A demonstrative in
     the first sentence, or one whose restrictor matches nothing prior, leaves the sentence OPEN.
     Write the full NP when there is no antecedent to point at.
4. **Coordination.** `and`/`or`; comma lists `X, Y and Z`; sentence-level `S but S`. Contrastive
   `requires A but not B` **when A and B are the same kind of thing** (e.g. two activities).
5. **Adjectives & compounds.**
   - *Genuine* stacked attributive adjectives — each modifies the head **independently** (`a human
     colorectal tumour`); and noun–noun compounds (`cancer models`, `cell line`, `MSI cancer models`).
   - **A lexicalized compound modifier is ONE term, not a stack — HYPHENATE it** when the lexicon does
     not already carry it. Write **`microsatellite-stable lines`**, not `microsatellite stable …`.
     Hyphenation makes the parser read it as a single compound adjective (via the D63 hyphen
     morphology, like `double-stranded`) instead of a stack of independent adjectives it is not.
     ⚠ `synthetic lethal` is no longer an example: it is now a curated multiword entry for C4280020
     (`experiments/lexicon-align/atom-overrides.json`), so both spellings work. That is the better fix
     when a term recurs — hyphenation is the authoring workaround for terms the lexicon lacks.
     Rationale: "Hyphenate lexicalized compound modifiers" below.
6. **Prepositional phrases.** `of`/`in`/`for`/`with`/`on`/`from`/`within`/`between`, as noun
   post-modifiers (`a biomarker of dependency`) and verb adjuncts (`essential in MSI models`). The
   object may be a determined NP (`within a gene`, `for tumours`).
7. **Relative clauses.** Restrictive `the gene that affects X` / `which affects X`; non-restrictive
   `WRN, which encodes a helicase, is essential.`
8. **Passive.** `WRN was depleted.` `Apoptosis was promoted by depletion.`
9. **Negation.** `WRN does not affect MSS models.` `The activity is not essential.`
10. **Clausal complements (report verbs).** `These findings show that WRN is a vulnerability.`
11. **Transitional adverbs** (sentence-initial): `Thus,` `Therefore,` `Hence,` `Moreover,`
    `Similarly,` `Notably,` — transparent (they don't change the claim). **One adjunct may open the
    sentence**, with or without a comma, and reads as it does after the verb: `After 24 h, the medium
    was replaced.` = `The medium was replaced after 24 h.` A second opening adjunct (`Then, 7 days
    after transduction, cells were collected.`) has no parse: keep one, move the other after the
    verb.
12. **Light verbs** that exist in the lexicon, e.g. `gives rise to`.
13. **Measured quantities** after a preposition, before a noun, or after the copula: `kept at
    37 °C for 1 h`, `10 μM etoposide`, `The incubation was 1 h.` — and bounded or ranged in the same
    places: `at less than 37 °C`, `at 37 °C or higher`, `>90% infection efficiency`, `for 2–3 h`; before
    `after`, `before`, `post` and `later` (`purified 72 h after transduction`, `fixed 2 days later`),
    after `by` and `every` (`recovered by three weeks`, `changed every 3 days`), and before `of` and a
    noun (`300 μl of CellTiter-Glo`). See "Measured quantities" below.

## DON'T — and how to rewrite it

| Avoid (journal style) | Why | Rewrite recipe |
|---|---|---|
| **Test statistics** (`n = 37`, `P = 4.2 × 10⁻¹³`, `Q = 4.8 × 10⁻²⁴`) | Out of the claim **by design** — a statistic qualifies a claim, it is not one. Routed to a D52 record. | State the **qualitative** claim; the statistic lives elsewhere. `… showed greater dependence …`, not `(n = 37; P = …)`. Unchanged by D93/D95. |
| **A quantity as a verb's object** (`contained 4 μg`, `reached ~100 mm³`) | A quantity is not a noun phrase. It composes where a word takes one (DO §13); an imported verb takes a noun phrase (D95 slice 8, decision 7). | Name the quantity: `reached a volume of about 100 mm³`, `contained 4 μg of puromycin`. |
| **A preposition with a clause as its object** (`96 h after adding doxycycline`) | A preposition takes a noun phrase; a gerund clause has no parse, with or without the quantity. | Nominalize: `96 h after the addition of doxycycline`. |
| **`the` before a mass noun** (`The viability was assayed.`) | `the` has entries for a singular and a plural count noun only. | Write the bare mass noun: `Cell viability was assayed.` |
| **A range with the unit twice, or in words** (`37 °C–39 °C`, `between 37 °C and 39 °C`) | The range grammar reads a digit pair with one unit or `%` after it (D95 slice 6). An en-dash pair with no unit is a count range (`4–7 foci`, slice 7); a hyphen pair with no unit is not a range, since it may be a catalogue number (`926-68021`). | Write the pair with the unit once, after it: `37–39 °C`, `20–30%`. Do **not** collapse a range to one endpoint or to a midpoint — that changes the claim, which R2 forbids. |
| **A plain number meant as a bound** (`5 cell lines` for "five or more", `37 °C` for "37 °C or above") | A plain number is exact (Measured quantities, rule 7). | Write the bound: `at least 5 cell lines`, `37 °C or higher`. |
| **Parenthetical asides / inline abbreviations** (`(MSI)`, `(PARP-1)`, `(Fig. 1a)`) | Asides are dropped; the parenthetical can't be a claim. | Introduce an abbreviation in its **own** sentence, or just use one form consistently. Drop figure/citation refs. |
| **Telegraphic caption annotations** (`Scale bar, 50 μm`; `pH 7.5`; `(1,200 V, 20 ms, 2 pulses)`) | Not sentences — a label and its value, with no verb. Figure legends and instrument settings are written in an elliptical register the sentence grammar does not cover. | Expand to the sentence it abbreviates: `The scale bar is 50 μm.` A parameter list becomes one sentence per parameter. This is **register**, not content — nothing is added or dropped, so R2 is satisfied. |
| **Em-dash appositives** (`—an interaction…—`) | Not covered; the dash content is dropped. | Split into separate sentences: `Synthetic lethality is an interaction between two genetic events. …` |
| **Long multi-clause sentences** (relative + subordinate + parenthetical stacked) | Each clause must compose; one gap kills the whole, and long units hit the beam. | **One claim per sentence.** |
| **`because` / `although` subordinate clauses** | Not in the lexicon (OOV); subordinators unbuilt. | Split + use a transitional: `…. Therefore ….` Drop concessive `although` or restate as two facts. |
| **Cross-type `but not`** (`required the helicase activity … but not its exonuclease activity` — different kinds) | The two objects must be the same category. | Split: `MSI models required the helicase activity of WRN. MSI models did not require the exonuclease activity of WRN.` |
| **Deeply-embedded / determined-subject pied-piping** (`the way in which the co-occurrence leads…`) | Only simple/name-subject pied-piping is covered. | Rephrase as a separate clause: `The co-occurrence leads to cell death.` |
| **Novel / OOV or en-dash hyphenations** (`CRISPR–Cas9-mediated`; an en-dash `–`, not a hyphen) | An unknown head/base is OOV; the en-dash isn't the hyphen token. | Rephrase or drop the modifier. **But a hyphenated compound whose head is a known adjective now PARSES** (D63 morphology: `double-stranded`, `pcr-based`, `large-scale`, `synthetic-lethal`) — **prefer** hyphenation for lexicalized compound modifiers (DO §5), don't avoid it. |
| **Possessive ellipsis / heavy gapping**, fronted reduced clauses with complex complements | Limited; gapping beyond same-type `but not` isn't covered. | Use an explicit subject and a full verb in each clause. |
| **`and/or`** | Not a token; collapsing it to `and` overstates (requires *both*). | Write **`or`** — `logic:Or` is **inclusive** (true if either or both), which is exactly what `and/or` means. (Faithfulness rule, not just style — `and/or → and` is a meaning change; `and/or → or` is meaning-preserving.) |

## Measured quantities (D93 / D95)

A measured quantity is `⟨numeral⟩ ⟨unit symbol⟩`. It is read as one token and converted to base units
(D93): `37 °C` is 310.15 K, `1 h` is 3600 s. It is **not a noun phrase**, so it composes only where
something takes it:

- **after a preposition**: `at`, `for`, `in`, `with` and `after` as verb adjuncts (`kept at
  37 °C for 1 h`, `resuspended in 50 μl`, `harvested after 72 h`), and `of`, `with` and `at` after a
  noun (`a dose of 5 mg/kg`, `a volume of 2,000 mm³`);
- **before a noun**: `10 μM etoposide`, `a 24 h incubation`, `10% FBS`;
- **after the copula**: `The incubation was 1 h.`, `The temperature was 37 °C.`;
- **before a temporal preposition**, as its offset: `purified 72 h after transduction`, `6 h before
  collection`, `4 days post transduction`, `fixed 2 days later` (D95 slice 8);
- **after `by` and `every`**: `recovered by three weeks`, `changed every 3 days`, `every 2–3 days`;
- **before `of` and a noun**, an amount of the noun's stuff (a volume, a mass, an amount of substance,
  a concentration, a duration): `300 μl of CellTiter-Glo`, `0.2 μg/ml of doxycycline`, `24 h of
  puromycin selection`. A percentage before `of` is a proportion (`15% of colon cancers`).

It cannot be a subject or a verb's object: `The cells contained 4 μg.` has no parse; `The medium
contained 2 μg ml⁻¹ puromycin.` does.

1. **Write the symbol, not the spelled-out unit.** `5 ml`, not `5 millilitres`. The symbol is what
   the unit parser reads; the spelled form is ordinary English words and parses as a different
   thing entirely.
2. **Put a space between the numeral and the symbol.** `37 °C`, `5 ml`, `625 mg`. This is the SI's
   own convention, and it makes the span unambiguous. The exceptions the SI itself makes are `%`
   and the angle symbols (`50%`, `90°`), which close up. A closed-up form (`931g`) is read too, but
   a word with the same spelling competes with it (`5A`, a culture medium, is also 5 A).
3. **Write compound units with a slash, `per`, or negative powers.** `mg/dL`, `mg per kg`,
   `μg ml⁻¹`. One solidus only: `mg/ml/h` is ambiguous without brackets, and the SI says so; write
   `mg ml⁻¹ h⁻¹`. A factor after a space needs its exponent: `μg ml⁻¹`, not `μg ml`.
4. 🔜 **A difference has no consumer yet.** What takes a quantity decides whether it is a value or a
   difference (D95): `at 37 °C` is the temperature 310.15 K, and a difference in °C is read without
   the 273.15 offset, so `5 °C` and `5 K` state the same change. But nothing takes a difference until
   D95 slice 7: `The temperature rose by 5 K.` has no parse. Keep a change as written and record the
   gap.
5. **Write `RCF` for centrifugal force.** `931g` and `931 g` are each read two ways, as the gram and
   as standard gravity, and the reading choice is left open (D93). `931 RCF` is standard gravity
   only. Do not write `931 × g` (`×` is an operator, so the sentence is non-prose) or `931 times
   gravity` (no parse: `at` has no noun-phrase adjunct entry).
6. **One quantity per role.** `kept at 37 °C for 1 h` is fine — two quantities filling two
   different roles. A range is one quantity: write it as rule 8 says, not `between 37 °C and 39 °C`.
7. **A plain value is exact; write a bound out.** `37 °C`, `2 h` and `5 cell lines` state exactly
   that value. Where a bound is meant, say so:

   | Meaning | Write | Also read |
   |---|---|---|
   | at least N | `at least N`, `N or more`, `N or higher` | `≥ N` |
   | at most N | `at most N`, `up to N`, `N or less`, `N or lower` | `≤ N` |
   | more than N | `more than N` | `> N` |
   | less than N | `less than N` | `< N` |
   | about N | `approximately N`, `about N`, `around N`, `roughly N` | `~N`, `≈ N` |

   A bound goes wherever the value goes: `incubated at 37 °C or higher`, `a dose of at least
   5 mg/kg`, `The temperature was less than 37 °C.` A symbol between a noun phrase and a value is a
   comparison without the copula (`the temperature < 37 °C`), but in a sentence of prose write the
   copula and the words. The same forms bound a count (`at least 1,000 cells`), where `fewer than N`
   and `N or fewer` are also read. 🔜 A bound on a difference (`rose by more than 5 °C`) has no
   parse.
   **A proportion** is `N% of`, `half of`, or a bound on either, before a plural noun phrase or a
   mass: `15% of colon cancers`, `more than half of the samples`, `45–60% of the cancers`, `Half of
   the cell pellet was saved.` `none of`, `all of`, `each of`, `most of` and `some of` are read too.
   Write `the` or no determiner before the noun: `such` has no entry.
8. **A range is the pair with the unit once, after it**: `2–3 days`, `80–90% confluence`,
   `30–37 °C`. Both ends are read in that unit and the range means `from … to …`, both ends included.
   Prefer the en-dash; a hyphen with a unit after it (`45-60%`) reads the same. Without a unit a pair
   is not a range — `926-68021` is a catalogue number — and `37 °C–39 °C` or `between 37 °C and
   39 °C` has no parse.
9. **Scientific notation is one number**: `2 × 10⁻¹⁶`, `1.5 × 10³ cells`, `10³`. Write `×` (or `x`)
   and a superscript or caret exponent; a unit after it applies to the whole number
   (`2 × 10⁻³ mg/kg`).
10. **A number word takes a unit as digits do**: `nine days` is `9 days`, for `one` … `ten`. Before a
    noun, join the number to the unit's NAME with a hyphen: `an eight-day viability assay`, `a
    10-minute incubation`. A unit SYMBOL takes no hyphen (`a 2 h incubation`, not `a 2-h incubation`,
    which is read as a word): the SI writes it so, and a hyphen before letters that spell a symbol
    names a compound (`5-mC`, `3-MA`).
11. **An offset goes before its preposition, and `post` is written apart**: `4 days post
    transduction`, not `4 days post-transduction` (one hyphenated token, read as a word). `after`,
    `before`, `post` and `later` take the offset; a bound or approximation on it reads as on any value
    (`about 6 h before collection`).

**What this does not change.** A statistic is still not a quantity. `n = 37` counts samples and
`P = 4.2 × 10⁻¹³` qualifies an inference; neither is a measured value of a physical quantity, and
both stay out of the claim.

## Hyphenate lexicalized compound modifiers (`synthetic-lethal`, not `synthetic lethal`)

A **lexicalized compound modifier** is a domain term of art whose parts do *not* combine compositionally
in general English — `synthetic lethal` is not "synthetic ∧ lethal" (a target that is artificial and
deadly); it is the attributive form of *synthetic lethality* (C4280020), the genetic concept where two
perturbations are each tolerated alone but lethal in combination. Left unhyphenated, such a term
**masquerades as a stack of independent adjectives**: `synthetic` and `lethal` each carry adjective *and*
noun senses, so the parser enumerates the Cartesian product of adjective/compound bracketings — a spurious
structural blow-up (D63 `d63-nominal-modification-normal-form.md` §1: S5 alone gave 12 skeletons), and the
"all-adjective" reading it settles on is the **wrong claim**.

**Rule.** Hyphenate a compound modifier when its parts would otherwise each be read as a separate
adjective (`microsatellite-stable`) *and the lexicon does not carry the term* — if it recurs across
documents, add the multiword entry instead and neither spelling will fork. The D63 hyphen morphology reads it as one
compound adjective (head must be a known adjective — `lethal`, `stable` — exactly as `double-stranded`
works). This is *more* faithful (R2), not just faster: the claim is about one property, not a conjunction
of two. **Noun–noun compound modifiers** (`immune checkpoint blockade`, `DNA repair pathway`, `cell cycle
arrest`) are already handled by the compound rule and need not be hyphenated — the masquerade only arises
when a part has an adjective reading. A compound that is a lexicon **unit** already (noun `synthetic
lethality` → C4280020, `cell death`, `dna repair`) is fine as written; hyphenation is for the *modifier*
surface the lexicon doesn't carry.

## Vocabulary note (orthogonal to style)

Style ≠ vocabulary. Domain terms the lexicon doesn't know (`cas9`, `recq`, novel hyphenations) are
**OOV** regardless of style; the measurement reports OOV separately. Where a known synonym exists,
prefer it; otherwise keep the domain term and accept the OOV (a vocabulary-import question, not a
style one). Gene/entity symbols (`WRN`, `MSH2`) resolve as named individuals where the UMLS/HGNC
import provides them. ⚠ Named *conditions* (`Lynch syndrome`, `MMR deficiency`) also stand bare now —
D70 gave them `lexicon:Num::name`, which grants bare standing without claiming they are mass nouns —
so write them as they appear in prose, without a determiner.

⚠ **A verb the lexicon has only as intransitive has no passive.** WordNet lists both senses of
`incubate` as intransitive, so `The cells were incubated.` has no parse, with or without `at 37 °C`;
`electroporate` is not a WordNet verb at all. Until the lexicon records these verbs' objects, write a
verb it has as transitive: `The cells were kept at 37 °C for 1 h.` parses.

## Worked example (one WRN sentence)

**Original (journal):** *"MSI cancer models required the helicase activity of WRN, but not its
exonuclease activity."*

**Controlled:**
> MSI cancer models required the helicase activity of WRN.
> MSI cancer models did not require the exonuclease activity of WRN.

Two same-shape SVO clauses; the contrast is preserved as an explicit negation; both compose.

## Worked example (one WRN methods sentence)

**Original (journal):** *"Experiments were performed in triplicate by adding the appropriate volume
of lentivirus to integrate vectors that encoded the desired sgRNA and the plates were spun at 931g
for 2 h at 30 °C."*

Two claims, a relative clause, a coordination, and three quantities — one of them the ambiguous `g`.

**Controlled:**
> Experiments were performed in triplicate.
> The vectors encoded the desired sgRNA.
> The plates were spun at 931 RCF for 2 h at 30 °C.

R1 splits the compound sentence; the relative clause becomes its own claim; `931g` becomes `931 RCF`
because the symbol is ambiguous between the gram and standard gravity (rule 5). The two unambiguous
quantities stay as written, filling two roles of one event — which is the case the methods register
produces constantly and the results register almost never does. All three sentences parse; the
third has 6 readings, against 12 with `931g`.

**What is still lost:** *"in triplicate"* is a replication count, which is D52's, not a quantity.
Do not rewrite it as `3 replicates` to make it look like one.

## Success criterion

A passage is "parser-faithful" when every sentence yields a **closed or open** kernel-checked parse
(no GRAMMAR-GAP), and the set of parses captures the passage's factual claims. The experiment measures
the closed/open/gap distribution on the rewritten WRN page against the original.

**Quantities have their own corpus.** The CNL page is results prose and contains one unit in 2,738
words; the methods material contains 35 in 4,912. `experiments/parsing/quantities/` holds 36
sentences derived from the WRN paper. Each names the relations its readings must contain, or, for
a gap, the construction it lacks and the D95 slice that owns it; `kernel/tests/quantity_corpus.rs`
checks them without a database, and its README records the full-lexicon run. A quantity gap stays
distinguishable from a syntax gap.
