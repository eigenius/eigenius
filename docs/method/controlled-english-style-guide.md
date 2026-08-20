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
| **Inline numbers / statistics** (`n = 37`, `P = 4.2 × 10⁻¹³`, `51 cell lines`, `0.56-fold`, `15%`) | The parser routes non-prose out; numbers are **dropped**, so a numeric claim is lost. | State the **qualitative** claim; put the statistic elsewhere (a separate D52 record). `… showed greater dependence …` not `(n=37; P=…)`. |
| **Parenthetical asides / inline abbreviations** (`(MSI)`, `(PARP-1)`, `(Fig. 1a)`) | Asides are dropped; the parenthetical can't be a claim. | Introduce an abbreviation in its **own** sentence, or just use one form consistently. Drop figure/citation refs. |
| **Em-dash appositives** (`—an interaction…—`) | Not covered; the dash content is dropped. | Split into separate sentences: `Synthetic lethality is an interaction between two genetic events. …` |
| **Long multi-clause sentences** (relative + subordinate + parenthetical stacked) | Each clause must compose; one gap kills the whole, and long units hit the beam. | **One claim per sentence.** |
| **`because` / `although` subordinate clauses** | Not in the lexicon (OOV); subordinators unbuilt. | Split + use a transitional: `…. Therefore ….` Drop concessive `although` or restate as two facts. |
| **Cross-type `but not`** (`required the helicase activity … but not its exonuclease activity` — different kinds) | The two objects must be the same category. | Split: `MSI models required the helicase activity of WRN. MSI models did not require the exonuclease activity of WRN.` |
| **Deeply-embedded / determined-subject pied-piping** (`the way in which the co-occurrence leads…`) | Only simple/name-subject pied-piping is covered. | Rephrase as a separate clause: `The co-occurrence leads to cell death.` |
| **Novel / OOV or en-dash hyphenations** (`CRISPR–Cas9-mediated`; an en-dash `–`, not a hyphen) | An unknown head/base is OOV; the en-dash isn't the hyphen token. | Rephrase or drop the modifier. **But a hyphenated compound whose head is a known adjective now PARSES** (D63 morphology: `double-stranded`, `pcr-based`, `large-scale`, `synthetic-lethal`) — **prefer** hyphenation for lexicalized compound modifiers (DO §5), don't avoid it. |
| **Possessive ellipsis / heavy gapping**, fronted reduced clauses with complex complements | Limited; gapping beyond same-type `but not` isn't covered. | Use an explicit subject and a full verb in each clause. |
| **`and/or`** | Not a token; collapsing it to `and` overstates (requires *both*). | Write **`or`** — `logic:Or` is **inclusive** (true if either or both), which is exactly what `and/or` means. (Faithfulness rule, not just style — `and/or → and` is a meaning change; `and/or → or` is meaning-preserving.) |

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

## Success criterion

A passage is "parser-faithful" when every sentence yields a **closed or open** kernel-checked parse
(no GRAMMAR-GAP), and the set of parses captures the passage's factual claims. The experiment measures
the closed/open/gap distribution on the rewritten WRN page against the original.
