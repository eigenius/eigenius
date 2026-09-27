# D97 — The SPECIALIST Lexicon as the lexicon's syntactic authority

**Status: proposed** (2026-09-27). Measured against the imported lexicon at the lexicon level; the
parse-level measurement is slice 1. Decisions 1–5 are open.

## The gap

The imported lexicon takes its syntax from sources that do not record it for this domain:

- **Transitivity.** The WordNet importer derives a verb's categories from WordNet's sentence frames.
  WordNet lists both senses of `incubate` as intransitive (frames 1 and 2), so `The cells were
  incubated.` has no parse. Two rows of the D95 quantity corpus fail over the full lexicon for this
  reason alone, and a third for the next (`experiments/parsing/quantities/README.md`).
- **Vocabulary.** WordNet has no verb `electroporate`, `transfect`, `lyse` or `phosphorylate`. UMLS
  has their procedures as concepts (`electroporation`, C0206691), which the UMLS importer emits as
  nouns.
- **Governed prepositions.** WordNet's PP frames 4 and 22 name no preposition, and 12 and 27 name
  only `to`; the importer maps all four to `cat_pp_arg(prep_any)`. An adjective's governed preposition is read from
  its gloss, by WordNet's "(usually) followed by" convention and a lemma-in-gloss heuristic that
  eigenius#263 shows reads prose, not governance.
- **Object + PP.** Frames 20 and 21 (`Somebody ----s somebody/something PP`) are classified as plain
  transitives and their PP is dropped (`crates/eigenius-wordnet/src/convert.rs`, `classify`), so
  `treat X with Y` has no `with`-argument. Frame 13 is classified with them, but it is `Somebody
  ----s on something` (`references/WordNet-3.0/doc/man/wninput.5:202`), a PP frame: its `on`-PP is
  read as an object.
- **Countability** comes from Wiktionary's category of uncountable nouns intersected with WordNet
  (`scripts/provision-countability.sh`), a general-English list.

## What SPECIALIST is

The SPECIALIST Lexicon (NLM Lexical Systems Group, 2026 release, `LEX_DOC/LEXICON`, 56 MB) is a
syntactic lexicon of general and biomedical English: 534,345 records, each a base form, a category
and its syntactic facts, with an entry id (`E0034095`). A record has **no senses**: `incubate` is one
verb record, whatever it means.

```
{base=incubate                  {base=electroporate               {base=treat
entry=E0034095                  entry=E0300046                    entry=E0061964
  cat=verb                        cat=verb                          cat=verb
  variants=reg                    variants=reg                      variants=reg
  intran                          tran=np                           intran
  tran=np                         nominalization=electroporation|…  tran=np
  nominalization=incubation|…   }                                   tran=pphr(with,np)
}                                                                   ditran=np,pphr(with,np)
                                                                    ditran=np,pphr(for,np)
                                                                    …
```

| Field | Records | What it says |
|---|---|---|
| `tran=np` / `intran` | 11,765 / 5,892 | the verb takes an object / needs none |
| `tran=pphr(p,np)` | 2,842 | a governed PP |
| `ditran=np,…` | 3,024 | object plus a second complement, usually a governed PP |
| `compl=pphr(p,np)` | 28,547 | an adjective's or noun's governed PP (`dependent on`/`upon`) |
| `tran=fincomp`, `infcomp`, `ingcomp` | 314, 170, 129 | clausal, infinitival and gerund complements |
| `variants=` | all | inflection (`reg`, `regd`, `irreg\|mouse\|mice\|`) and countability (`uncount`, `glreg`) |
| `spelling_variant=` | — | `tumour` for `tumor`, `dependent` for `dependant` |
| `nominalization=` | 16,534 | the verb's noun (`electroporate` ↔ `electroporation`) |
| `acronym_of`, `abbreviation_of` | 67,675, 23,989 | expansions |

**Terms.** The SPECIALIST NLP Tools are "available to all requesters … at no charge". Redistribution
is allowed with the terms included; a distribution must attribute the source "as the SPECIALIST NLP
Tools with the release number and date" and "state any modifications … along with a complete
description" (`lhncbc.nlm.nih.gov/LSG/Projects/lexicon/current/web/termsAndConditions.html`). No UMLS
licence is named. Nothing here redistributes the file; the imported lexicon records the release.

## Measured against the imported lexicon

`experiments/lexicon-specialist/measure-specialist.py` joins SPECIALIST to the entries the WordNet
importer emitted in the `2026-09-27` reseed (`wordnet-chain/*.esl`, at `6a3eabf`), by lemma and
category, and to the Wiktionary list.

**Verbs.** 12,129 SPECIALIST verb records; 11,500 WordNet verb lemmas; 7,382 joined.

| Change | Lemmas | Entries |
|---|---|---|
| gains an object: WordNet has no object-taking sense, SPECIALIST has `tran=np` | 399 | +3,936 (one per existing sense × form) |
| SPECIALIST has it with an object or a PP; WordNet has no such verb | 4,390 | — |
| PP-oblique in WordNet (`prep_any`); SPECIALIST names the preposition | 744 | — |
| object + PP (`ditran=np,pphr`) | 1,301 | — |

These frames name 57 prepositions. `lexicon:Prep` has 13 (plus `prep_any`); the verbs whose frames
name one outside it: `over` 68, `by` 52, `through` 41, `onto` 39, `around` 29, `off` 27, `out of`
25, `between` 17, `after` 15, `among` 14, `under` 9, `across` 9.

**Adjectives.** 21,475 WordNet adjective lemmas. The importer gives 890 a governed preposition;
SPECIALIST has `compl=pphr` for 598. They agree on a preposition for 174 lemmas; SPECIALIST adds one
for 473; the importer has one SPECIALIST does not for 739. That last set mixes the gloss heuristic's
misreadings (eigenius#263) with SPECIALIST's gaps; the count does not separate them.

**Countability**, over WordNet noun lemmas: Wiktionary 32,283 uncountable, SPECIALIST 33,135
`uncount`, 19,949 in both, 13,186 SPECIALIST only, 12,334 Wiktionary only.

**The WRN methods, letter and quantity corpus** (5,823 lowercase word tokens). Verbs are counted at
their past participles only, the methods register's passives: a base or third-person form is as
often a noun (`volume`, `vectors`), which only a parse separates. The counts are a lower bound.

- **Gains an object**: `incubate` 3 occurrences (+12 entries: 2 senses × 6 forms), `mutate` 3 (+6),
  `mediate` 1 (+18).
- **A verb WordNet lacks**: `electroporate` 2, `counterstain`, `downregulate`, `lyse`,
  `phosphorylate`, `transfect`, `upregulate` 1 each.
- **Names its PP**: 30 lemmas, 86 occurrences — `compared with`, `treated with`, `associated with`,
  `derived from`.
- **Object + PP**: 104 lemmas, 286 occurrences — `treat` (`for`, `to`, `with`), `supplement`
  (`with`, `by`), `fix with`, `wash with`, `infect with`, `insert into`, `transfer into`.
- **Adjectives**, matched by form, so a token counts whatever its use: SPECIALIST adds a preposition
  for 35 lemmas (125 occurrences), among them `dependent upon`, `predictive of`, `indicative of`,
  `homologous to`, `comparable to`; the importer has one SPECIALIST does not for 44 (143), among
  them `first in`, `high in`, `same in`, `following in`.

What these do to the **parse** — readings per sentence, gaps closed, pins moved — is not measured;
that is slice 1.

## The proposal

### SPECIALIST decides syntax; WordNet and UMLS keep the senses

A lexical entry needs a `sem`, and a SPECIALIST record has none to give. So SPECIALIST does not
become a third sense lexicon beside WordNet and UMLS. It is read at conversion by both importers, as
the countability list is now, and decides the syntactic facts of the entries they already emit:

| SPECIALIST | The importer's entry |
|---|---|
| `tran=np`, `ditran=np` | an object-taking category, where WordNet's frames give none |
| `tran=pphr(p,np)` | `cat_pp_arg(p)` in place of `prep_any` |
| `ditran=np,pphr(p,np)` | `((S\NP)/cat_pp_arg(p))/NP`, where frames 20/21 drop the PP |
| `compl=pphr(p,np)` on an adjective or noun | the governed preposition (eigenius#263's attested set) |
| `tran=fincomp` | `cat_cp`, as frame 26 is now |
| `uncount` | the additive `cat_n(C, mass)` entry (decision 4) |
| `spelling_variant`, `irreg` inflections | forms |

`infcomp` and `ingcomp` stay deferred, as WordNet's control and raising frames are.

### The lemmas no other source has: a small importer

For a verb (or adjective) SPECIALIST has in a category neither WordNet nor UMLS has it in —
`electroporate` — a new importer mints one sense-less predicate per lemma and frame
(`specialist:electroporate_t : Entity -> Entity -> Prop`), with entries `in_lexicon =
lexicon:specialist`, a `lexicon:Lexicon` resource carrying the release, and a place in the
`LexiconProfile`. Where the verb's `nominalization` names a noun that is a UMLS atom
(`electroporation` → C0206691), the predicate is linked to that concept; the link is recorded, not
used as the `sem` (decision 5).

### The join is deterministic; the concept alignment is unchanged

- **SPECIALIST ↔ WordNet** by lemma and category, through the base, the spelling variants and the
  inflections. **SPECIALIST ↔ UMLS** by atom string, for nouns (countability) and for the
  nominalization link. No LLM: the key is the word, not the concept.
- **WordNet ↔ UMLS concept alignment** (`experiments/lexicon-align/`, the LLM-adjudicated merges) is
  unchanged in kind. Its emitter rewrites entries a merge touches; it must carry the SPECIALIST-
  derived categories through, which a test pins.
- It all happens at conversion, so it rides a reseed; the alignment snapshot is built as now.

### eigenius#263 folds in

\#263 proposes SPECIALIST as the attested source for adjectives' governed prepositions, with an
additive interim until it is provisioned. With SPECIALIST in hand the interim is not needed:
attested = WordNet's "followed by" convention ∪ SPECIALIST's `compl=pphr` ∪ the curated frames, the
gloss heuristic only where none speaks, never `as`. The `governed-prepositions` branch's single
kernel list (`GOVERNED_PREPOSITIONS`) is where decision 3 lands.

## Decisions (open)

1. **Lemma or sense.** SPECIALIST speaks for a lemma; WordNet's frames are per sense. Giving
   SPECIALIST's frames to every sense of the lemma is determinate and adds the entries counted above
   (+3,936 for objects alone); choosing senses needs evidence SPECIALIST does not have. *Proposed:*
   the union, accepted or refused on slice 1's parse measurement.
2. **Union or authority.** Where WordNet has a frame SPECIALIST does not (a PP-oblique `prep_any`
   beside SPECIALIST's named preposition), does SPECIALIST's frame replace it or join it? Replacing
   removes readings the grammar has today.
3. **The preposition inventory.** `lexicon:Prep` names 13; SPECIALIST's frames name 57. Extend the
   enum to the prepositions the closed class has entries for (`by`, `between`, `after`, `among`,
   `onto`, `through`, …) and map the rest to `prep_any`, or extend it to all 57. `by` needs care: the
   closed class's `by_agent` already reads the passive agent.
4. **Countability.** Wiktionary and SPECIALIST agree on 19,949 lemmas and differ on 25,520. Union,
   SPECIALIST alone, or Wiktionary alone. Mass entries are additive, so the union costs readings, not
   coverage.
5. **The sense-less predicate.** What `electroporate` denotes: a predicate of its own, linked to the
   UMLS procedure through the nominalization, or an existential over the procedure concept. The first
   is what the grammar consumes today.

## Slices

1. **Provision and measure.** `scripts/provision-specialist.sh` fetches `LEX_DOC/LEXICON` for a pinned
   release with a checksum into `references/specialist/` (gitignored), as
   `provision-countability.sh` does; the reseed's `PROVENANCE` records it. A reader in a small crate
   parses the records. The WordNet importer gains the union behind a flag, and a reseed with it on
   is measured against the `2026-09-27` snapshot on the CNL page and the quantity corpus: readings,
   gaps, pins. Decisions 1, 2 and 4 are taken on that.
2. **Verbs.** Objects, named PP prepositions, clausal complements; decision 3 and the kernel's
   single preposition list. Frame 13 moves to the PP-oblique kind, as `on`.
3. **Adjectives and nouns**: eigenius#263's attested set.
4. **Object + PP**: the `((S\NP)/cat_pp_arg(p))/NP` category, its passive (`were treated with
   etoposide`), and the importer's frames 20/21.
5. **The SPECIALIST importer** for lemmas no other source has, and the nominalization link.
6. **Countability**, per decision 4.

Slices 2–6 change the imported lexicon; they share one reseed and one re-adjudication of the pins
that move.

## Out of scope

SPECIALIST's `acronym_of` and `abbreviation_of` (91,664 records), which bear on D63's abbreviation
grounding; its adjective features (`stative`, attributive and predicative `position`); its trademarks.
