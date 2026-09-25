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

*Revised `2026-09-20` for measured quantities. **Those rules are marked 🔜 and are NOT yet live** —
D93 (units of measure) and D95 (quantities in the parser) are specifications, not implementations,
and this guide does not state aspiration as capability. Until they land, the 🔜 rules describe how to
author for the parser that is coming; the ⚠ and unmarked rules describe the one that exists.*

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
3. **Determiners.** `a`/`an`/`the`/`every`/`each`/`all`/`some`/`no`, and the cardinals `two`…`ten`.
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
    `Similarly,` `Notably,` — transparent (they don't change the claim).
12. **Light verbs** that exist in the lexicon, e.g. `gives rise to`.

## DON'T — and how to rewrite it

| Avoid (journal style) | Why | Rewrite recipe |
|---|---|---|
| **Test statistics** (`n = 37`, `P = 4.2 × 10⁻¹³`, `Q = 4.8 × 10⁻²⁴`) | Out of the claim **by design** — a statistic qualifies a claim, it is not one. Routed to a D52 record. | State the **qualitative** claim; the statistic lives elsewhere. `… showed greater dependence …`, not `(n = 37; P = …)`. Unchanged by D93/D95. |
| **Measured quantities** (`37 °C`, `1 h`, `5 ml`, `0.2 mg`, `15%`) | 🔜 Today these are dropped like statistics, so the claim loses them. Under D93/D95 they become part of the claim. | **Today:** state qualitatively, or keep the quantity and record the gap (R2 — a faithful un-parsed claim beats a parsed distorted one). **Once D95 lands:** write the quantity; see "Measured quantities" below. |
| **Ranges and intervals** (`20–30%`, `15–18`, `45–60%`) | Deferred in D95; the en-dash/hyphen distinction that separates a range from a catalogue number is not built. | Keep the range and record the gap. Do **not** collapse it to one endpoint or to a midpoint — that changes the claim, which R2 forbids. |
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

## 🔜 Measured quantities (D93 / D95 — not yet live)

A measured quantity is `⟨numeral⟩ ⟨unit symbol⟩`, and under D95 it becomes one chart item denoting a
value, so it composes as an ordinary noun phrase: a preposition's object (`at 37 °C`, `for 1 h`), a
nominal modifier (`a 24 h incubation`), or a predicate (`The incubation was 1 h.`).

1. **Write the symbol, not the spelled-out unit.** `5 ml`, not `5 millilitres`. The symbol is what
   the unit parser reads; the spelled form is ordinary English words and parses as a different
   thing entirely.
2. **Put a space between the numeral and the symbol.** `37 °C`, `5 ml`, `625 mg`. This is the SI's
   own convention, and it makes the span unambiguous. The exceptions the SI itself makes are `%`
   and the angle symbols (`50%`, `90°`), which close up.
3. **Write compound units with a slash, one solidus only.** `mg/dL`, `ml/min`. Not `mg/ml/h` —
   more than one solidus is ambiguous without brackets, and the SI says so. For anything deeper,
   use negative powers: `mg ml⁻¹ h⁻¹`.
4. **Express a temperature DIFFERENCE in kelvin, never in °C.** `rose by 5 K`, not `rose by 5 °C`.
   The degree Celsius is equal in magnitude to the kelvin, so this is exact and loses nothing — and
   it sidesteps the one genuine ambiguity in the unit system: a bare °C is read as a *point*
   (D93), so `by 5 °C` would be mis-normalised by 273.15. Use °C for a temperature, K for a change
   in temperature.
5. **Disambiguate `g`.** The symbol is both the gram and standard gravity, and only context
   separates them. Write `931 × g` or `931g` for centrifugal force and `931 g` for mass — and
   prefer rephrasing to `931 times gravity` where the sentence allows, since the ranker resolving
   this correctly is not something to rely on in authored text.
6. **One quantity per role.** `incubated at 37 °C for 1 h` is fine — two quantities filling two
   different roles. `between 37 °C and 39 °C` is a range, which is deferred (see the DON'T table).

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

## Worked example (one WRN sentence)

**Original (journal):** *"MSI cancer models required the helicase activity of WRN, but not its
exonuclease activity."*

**Controlled:**
> MSI cancer models required the helicase activity of WRN.
> MSI cancer models did not require the exonuclease activity of WRN.

Two same-shape SVO clauses; the contrast is preserved as an explicit negation; both compose.

## 🔜 Worked example (one WRN methods sentence)

**Original (journal):** *"Experiments were performed in triplicate by adding the appropriate volume
of lentivirus to integrate vectors that encoded the desired sgRNA and the plates were spun at 931g
for 2 h at 30 °C."*

Two claims, a relative clause, a coordination, and three quantities — one of them the ambiguous `g`.

**Controlled:**
> Experiments were performed in triplicate.
> The vectors encoded the desired sgRNA.
> The plates were spun at 931 times gravity for 2 h at 30 °C.

R1 splits the compound sentence; the relative clause becomes its own claim; `931g` is rephrased
because the symbol is ambiguous between the gram and standard gravity (rule 5). The two unambiguous
quantities stay as written, filling two roles of one event — which is the case the methods register
produces constantly and the results register almost never does.

**What is still lost:** *"in triplicate"* is a replication count, which is D52's, not a quantity.
Do not rewrite it as `3 replicates` to make it look like one.

## Success criterion

A passage is "parser-faithful" when every sentence yields a **closed or open** kernel-checked parse
(no GRAMMAR-GAP), and the set of parses captures the passage's factual claims. The experiment measures
the closed/open/gap distribution on the rewritten WRN page against the original.

🔜 **The tracked corpus does not exercise quantities.** The CNL page is results prose and contains
one unit in 2,738 words; the methods material contains 35 in 4,912. So the current gates certify a
register that excludes the very thing D93/D95 add, and a quantity-bearing corpus with a
re-established baseline is part of landing them (D95). A quantity gap and a syntax gap must be
distinguishable in that report — the methods register is where the parser is weakest, and
conflating the two would make the measurement useless.
