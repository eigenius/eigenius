# D97 — The SPECIALIST Lexicon as the lexicon's syntactic authority

**Status: proposed** (2026-09-27). Measured against the imported lexicon at the lexicon level; the
parse-level measurement is slice 1. Decisions 1–4 are taken (2026-09-27), and 5 for verbs whose
noun names a concept; the rest of 5 and decisions 6–10 are open.

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
  only `to`; the importer maps all four to `cat_pp_arg(prep_any)`. An adjective's governed
  preposition is read from its gloss, by WordNet's "(usually) followed by" convention and a lemma-in-gloss heuristic that
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

**Provisioning.** One file, no licence gate, no UTS account:

```
scripts/provision-specialist.sh            # fetch if absent, then verify
scripts/provision-specialist.sh --check    # verify what is on disk, fetch nothing
```

which is `curl -o references/specialist/LEXICON
https://data.lhncbc.nlm.nih.gov/public/lsg/lexicon/2026/release/LEX_DOC/LEXICON` plus the checks
below.

56 012 657 bytes, sha256 `259d0283ebe7b027be730538d2c77c10f13f09bb824106306fbf5838d0a629f5`,
534 345 records. The script verifies against the counts this document measures —
`nominalization=` 16 534, `acronym_of` 67 675, `abbreviation_of` 23 989 — which identify the
release more precisely than its name does, and refuses a file that does not match rather than
letting a different lexicon move every number below without anything failing. It downloads to a
temporary file and moves it into place, so a truncated or error-page body never lands looking like
a lexicon.

It is served from `data.lhncbc.nlm.nih.gov/public/`, **not** from the `lhncbc.nlm.nih.gov/LSG/` or
`lsg3.nlm.nih.gov/LexSysGroup/` paths the project pages link to; those redirect to directory
listings that answer 403. The URL is recorded here because it is not derivable from the web pages,
and a hand-off that said "D97 gives the URL" cost an afternoon's probing when D97 gave only the
terms page below.

**SPECIALIST also ships inside the UMLS Full Release** (`umls-<release>-full.zip` →
`<release>aa-otherks.nlm` → `LEX/`), alongside the Semantic Network `provision-umls.sh` already
extracts from there. That route needs a UTS licence and a 30 GB download; the direct fetch above
needs neither. Note the Metathesaurus archives are a different download — a
`-metathesaurus-*.zip` contains `<RELEASE>/META/` and nothing else.

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

### No source is comprehensive, so none replaces another

WordNet lists `incubate` as intransitive only; SPECIALIST has no senses; Wiktionary's uncountable
nouns and SPECIALIST's `uncount` agree on 19,949 lemmas and differ on 25,520. Each misses what
another attests. So a syntactic fact attested by any source is kept, and a source adds to the others
without overriding them: frames join (decision 2), countability is the union (decision 4), and an
adjective's governed prepositions are every attested one (eigenius#263). Where sources disagree, the
parse sees both readings, and the ranker and the felicity gate choose between them, as they do
between senses.

### SPECIALIST adds syntax; WordNet and UMLS keep the senses

A lexical entry needs a `sem`, and a SPECIALIST record has none to give. So SPECIALIST does not
become a third sense lexicon beside WordNet and UMLS. It is read at conversion by both importers, as
the countability list is now, and adds syntactic facts to the entries they already emit:

| SPECIALIST | The importer's entry |
|---|---|
| `tran=np`, `ditran=np` | an object-taking category, beside WordNet's frames |
| `tran=pphr(p,np)` | `cat_pp_arg(p)`, beside WordNet's `prep_any` |
| `ditran=np,pphr(p,np)` | `((S\NP)/cat_pp_arg(p))/NP`, beside frames 20/21's transitive |
| `compl=pphr(p,np)` on an adjective or noun | the governed preposition (eigenius#263's attested set) |
| `tran=fincomp` | `cat_cp`, as frame 26 is now |
| `uncount` | the additive `cat_n(C, mass)` entry (decision 4) |
| `spelling_variant`, `irreg` inflections | forms |

`infcomp` and `ingcomp` stay deferred, as WordNet's control and raising frames are.

### A verb no other source has is the verb sense of its concept

For a verb SPECIALIST has and WordNet does not — `electroporate` — the entries are **verb senses of
the UMLS concept its nominalization names** (decision 5). SPECIALIST links the verb to its noun by
entry id (`nominalization=electroporation|noun|E0218332`), and the noun, in any of its spellings, is
an atom of the concepts that carry it: `electroporation` of C0206691 (the procedure) and C0678054
(Electroporation Therapy). Each concept gets one verb sense per frame, a predicate
`Entity -> Entity -> Prop` whose sense key is the concept's (`umls:C0206691`). A noun that names two
concepts gives the verb two senses, as a WordNet verb has one per synset, and the ranker chooses.

The UMLS importer emits these, reading SPECIALIST at conversion as it reads the countability list,
with `in_lexicon = lexicon:umls`: the sense is UMLS's, the syntax SPECIALIST's.

Where the noun is no UMLS atom but a WordNet noun, the verb senses are the WordNet synsets', by the
same rule, and the WordNet importer emits them: one per synset of the noun, with the synset's sense
key and `in_lexicon = lexicon:wordnet`.

How the 4,390 verbs only SPECIALIST has resolve, through their nouns and those nouns' spelling
variants, against the concepts the UMLS importer emitted:

| The verb's noun | Verbs | Examples |
|---|---|---|
| names one UMLS concept | 428 | `transfect` (C0040669), `downregulate`, `upregulate`, `alkylate` |
| names several (median 2, max 5) | 92 | `electroporate`, `lyse`, `phosphorylate` (2 each) |
| names no UMLS concept, but is a WordNet noun | 80, with 112 synsets (57 have one, 23 more) | `calcinate` (`calcination`), `cajole` (`cajolery`) |
| names no concept in either source | 2,345 | `absolutise`, `acetoacetylate`, `acetolyse` |
| — the verb has no nominalization | 1,445 | `counterstain`, `acidize`, `afterload` |

Of the WRN texts' seven verbs WordNet lacks, six resolve; `counterstain` has no nominalization.

Verbalization recognises a verb by the WordNet importer's atom name (`v{offset}_{frame}`,
`dcg/verbalize.rs`), so a verb sense of a UMLS concept needs it to recognise a verb by its category
or its entry instead; slice 5 changes that.

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

## Decisions

1. **Lemma or sense — decided: the senses the evidence picks, and an LLM judge where it picks
   none.** SPECIALIST speaks for a lemma (`incubate` takes an object); WordNet's frames are per
   sense, and every added frame is an entry per sense and form. So the importer must choose the
   senses a frame goes on. Counted from the WordNet dict files:

   | Frame SPECIALIST adds | Verbs | One sense | Several senses |
   |---|---|---|---|
   | an object | 409 | 301 | 108 |
   | a named PP preposition | 743 | 124 | 619 |
   | an object and a PP | 1,305 | 321 | 984 |

   **The evidence.** WordNet's derivational pointers (`+`) link a verb sense to noun senses, and
   SPECIALIST names the verb's noun (`incubate` → `incubation`). A sense whose pointer reaches that
   noun is evidence the frame is its. Over the several-sense verbs:

   | | object | named PP | object + PP |
   |---|---|---|---|
   | the pointers pick some senses, not all | 12 | 205 | 360 |
   | every sense points to the noun | 13 | 71 | 127 |
   | no sense points to it | 1 | 23 | 34 |
   | the verb has no nominalization | 82 | 320 | 463 |

   **Decided:**
   - One sense: the frame is its.
   - Where the pointers pick some senses and not all: those senses.
   - The open cases — every sense points, none does, or there is no nominalization — go to an LLM
     judge: 999 verbs, 1,804 items (a verb and a frame: 96 objects, 757 named PPs, 951 object +
     PP), 10,473 sense judgements, 5.8 senses per item. The pointers decide 1,050 items.

   **The judge** follows the WordNet–UMLS concept alignment's protocol
   (`experiments/lexicon-align/README.md`), whose 81,305 verdicts cost about $90:
   - It sees the lemma, the frame with an example built from it (`X incubated Y`, `treat X with
     Y`), and each sense's gloss, examples and WordNet frames; it answers per sense whether the
     frame fits, with a confidence, and may answer that no sense fits.
   - **Validated before it is trusted**, on a gold set WordNet supplies: verbs whose senses differ
     in WordNet's own frames (some take an object, some do not). With those frames hidden, the judge
     must recover which senses take the object. A recall or precision below the threshold the
     alignment used stops the run.
   - A verdict is accepted at the confidence threshold the validation sets; the verdicts are
     committed, since they are not reproducible, and the run is resumable and fails closed.
   - **No frame is dropped.** Where the judge accepts no sense, every sense gets the frame, as the
     union would. A "no sense fits" verdict is also recorded: it marks a sense WordNet lacks, as
     SPECIALIST's transitive `mediate` (`WRN mediates repair`) is not WordNet's "act between
     parties".

   *Rejected:* every sense for every frame, which gives wrong-sense readings where the evidence
   says otherwise (`mediate`'s "occupy an intermediate position X"); and the evidenced senses only,
   which drops the frame wherever a several-sense verb has no evidence: 923 cases over the three
   kinds, 865 with no nominalization and 58 where no sense points to it.
2. **Union or authority — decided: union.** Where WordNet has a frame SPECIALIST does not (a
   PP-oblique `prep_any` beside SPECIALIST's named preposition), SPECIALIST's frame joins it. Both
   entries carry the sense's axiom, so they yield the same sem wherever both apply; slice 1 measures
   the readings that remain.
3. **The preposition inventory — decided: every preposition SPECIALIST names.** `lexicon:Prep`
   names 13. Over verbs, adjectives and nouns,
   SPECIALIST records 30,950 governed prepositions (a record × a preposition), 14,174 of them on a
   WordNet lemma; 4,963 (2,717 on WordNet) name one of 53 prepositions outside the enum:

   | Outside the enum | Cases | On WordNet | Closed-class entry |
   |---|---|---|---|
   | `by` | 4,070 | 2,077 | `by_agent`, `by_nmod` |
   | `between`, `after`, `among`, `without`, `within` | 252 | 134 | yes |
   | `over`, `onto`, `through`, `per`, `around`, `off`, `towards`, `under`, … (32) | 552 | 441 | none |
   | multiword: `out of`, `due to`, `according to`, `as to`, `in terms of`, … (15) | 89 | 65 | none |

   - `by` is 82% of them, and 3,921 of its 4,070 are noun complements — the agent of a
     nominalization (`abolition by`, `activation by`), the relation `by_agent` reads on a passive.
     Only 26 verbs take a `by` argument (`abide by`) and 40 an object and a `by` PP (`multiply X by
     Y`).
   - A preposition with no closed-class entry parses in no role today, so naming it in the enum
     alone changes no parse.
   - In the WRN texts and the quantity corpus, a word is directly followed by a preposition
     SPECIALIST says it governs 253 times; 246 are inside the enum. The other 7: `by` 4 (`activation
     by`, `analysed by`, `caused by`, `study by`), `between` 2 (`interaction between`,
     `relationship between`), `than` 1 (`more than`, which the comparative reads).

   **Decided.** `lexicon:Prep` gains a constructor for every preposition a SPECIALIST frame names,
   and the closed class an argument entry for each, shaped as `at_arg` is (`cat_pp_arg(prep_p) /
   NP`, sem `argmarker_sem`): 52 of each, 37 single prepositions and 15 multiword forms (`out of`,
   `due to`), which multiword seeding reads. Every frame is then emitted with the preposition it
   attests. Two exceptions:
   - **`by` on a noun is not emitted as an argument.** Its 3,921 cases are the agent of a
     nominalization (`activation by X`), which `by_nmod` already reads as `prep_by(x, X)`; an
     argument entry would add a second reading of the same relation. The 83 adjectives (`abolishable
     by`, which no passive reads) and the 66 verb frames (`abide by`, `multiply X by Y`) are emitted,
     as `prep_by`.
   - **`than`** stays the comparative's, read through `cat_pp_than`.

   *Rejected:* mapping a preposition with no closed-class entry to `prep_any`. `argue over X` would
   still not parse, since `over X` cannot become an argument PP, and the wildcard slot would accept
   `argue on X` or `argue at X` as arguments — readings no source attests, bought by discarding the
   preposition SPECIALIST records.

   An argument entry does not put a word on the importers' closed-class list
   (`PREPOSITIONS_AND_CONJUNCTIONS`): `one` has closed-class entries and keeps WordNet's senses.
   Whether `over`, `through` or `off` lose their content senses (`an over` in cricket, `a through
   train`) is decided per word, as it was for `after`. The kernel's single preposition list
   (`GOVERNED_PREPOSITIONS`, on the `governed-prepositions` branch) is extended with the enum, and its
   test keeps the two equal.
4. **Countability — decided: both.** A lemma either source flags uncountable gets the additive
   `cat_n(C, mass)` entry. Today 27,207 WordNet lemmas carry 43,474 mass entries; SPECIALIST adds
   12,863 lemmas and about 14,665 entries (one per existing count entry). The UMLS importer takes the
   same list, through head inheritance, so its mass entries move too; not yet measured.
5. **What a verb only SPECIALIST has denotes — decided where its noun names a concept: a verb sense
   of that concept.** A UMLS concept where the noun is a UMLS atom (520 verbs); otherwise a WordNet
   synset where it is a WordNet noun (80 verbs, 112 senses). See "A verb no other source has is the
   verb sense of its concept". **Open for the 3,790 with no concept to attach to**: no entry, or a
   sense-less predicate that records SPECIALIST's syntax alone.
6. **Whether a governed preposition reaches the meaning.** `lexicon:Prep` is syntactic, erased by
   ⟦·⟧: a sense's PP-oblique reading is one relation, `v{offset}_p`, and on the
   `governed-prepositions` branch an adjective's relational readings share one `deg_{loc}_rel`
   whatever the preposition (`crates/eigenius-wordnet/src/convert.rs`). While WordNet named no
   preposition that was one reading. With SPECIALIST naming several, a word gets one entry per
   preposition and the same meaning from each: `changed from A` and `changed into A` are one claim,
   and so are `treat X with Y` (the instrument) and `treat X for Y` (the purpose).
   - Reach, over WordNet lemmas: 485 verbs whose PP argument names two or more prepositions
     (`account for`/`to`, `abound in`/`with`), 435 whose object + PP frames do (`acquit of`/`on`,
     `adapt for`/`to`), 130 adjectives, 2,404 nouns.
   - *Proposed:* one relation per sense and preposition (`v{offset}_p_from`, `v{offset}_p_into`), so
     the preposition is part of the predicate. It changes the axiom and entry shape of every
     governed-PP reading, eigenius#263's adjectives with them. Keeping one relation conflates the
     claims, which R2 of the style guide forbids.
7. **Decision 1 for adjectives and nouns.** Decision 1 chooses the senses a verb frame goes on.
   SPECIALIST's governed prepositions on adjectives and nouns are per lemma as well: 590 WordNet
   adjectives, 354 with several senses; 6,425 WordNet nouns, 3,329 with several senses. Does the
   same rule apply — derivational pointers where they discriminate, the judge otherwise? For nouns
   that multiplies the judge's work several times over, so it is a budget question as well.
8. **What a "no sense fits" verdict leads to.** The judge will name frames whose sense WordNet
   lacks (`mediate` as in `WRN mediates repair`). Decision 1 keeps such a frame on every sense. Open:
   whether it also yields a new sense — the verb sense of the concept its noun names, by decision
   5's rule (`mediate` → `mediation`) — or stays a recorded gap.
9. **SPECIALIST's facts for UMLS entries.** The proposal applies SPECIALIST to the UMLS importer for
   countability and for verbs defined by their nouns (decision 5). Open: whether it also gives UMLS
   concepts their governed prepositions (`dependence on`), spelling variants (`tumour`/`tumor`) and
   irregular plurals, as it does WordNet's lemmas.
10. **The closed-class list for the new prepositions.** Decision 3 gives 47 prepositions argument
   entries without putting them on the importers' closed-class list; whether `over`, `through` or
   `off` lose their content senses is decided per word, with no rule yet. *Proposed:* off the list
   unless a measurement shows their content senses harm parses, as `As` = arsenic showed for `as`.

## Slices

1. **Provision and measure.** `scripts/provision-specialist.sh` fetches `LEX_DOC/LEXICON` for a pinned
   release with a checksum into `references/specialist/` (gitignored), as
   `provision-countability.sh` does; the reseed's `PROVENANCE` records it. A reader in a small crate
   parses the records. The sense judge (decision 1) runs, validated first, and its verdicts are
   committed. The WordNet importer gains SPECIALIST's frames on the senses decision 1 picks, behind a
   flag, and a reseed with it on is measured against the `2026-09-27` snapshot on the CNL page and
   the quantity corpus: readings, gaps, pins — the cost of decisions 1–4.
2. **Verbs.** Objects, named PP prepositions, clausal complements. The 52 `lexicon:Prep`
   constructors and closed-class argument entries (decision 3), and the kernel's single preposition
   list. Frame 13 moves to the PP-oblique kind, as `on`.
3. **Adjectives and nouns**: eigenius#263's attested set.
4. **Object + PP**: the `((S\NP)/cat_pp_arg(p))/NP` category, its passive (`were treated with
   etoposide`), and the importer's frames 20/21.
5. **Verb senses of concepts**: the UMLS importer emits them through SPECIALIST's nominalizations,
   and the WordNet importer where the noun is only a WordNet noun (decision 5); verbalization
   recognises a verb by its category or entry rather than its atom name.
6. **Countability**: the union of Wiktionary and SPECIALIST (decision 4).

Slices 2–6 change the imported lexicon; they share one reseed and one re-adjudication of the pins
that move.

## Out of scope

SPECIALIST's `acronym_of` and `abbreviation_of` (91,664 records), which bear on D63's abbreviation
grounding; its adjective features (`stative`, attributive and predicative `position`); its trademarks.
