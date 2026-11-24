# Encoding the XIAP report — the plan

**Decided 2026-10-10.** Survey §4 step 5. The deliverable is the XIAP report encoded as
`demo/prose-to-formulas-v2` encodes its paragraph: D71's pipeline — glossary → parse → sense rank →
reading selection → anaphora resolution → claim landing → artifact → commit — producing landed
claims that carry a real `eigentt:proposition`. The document model is the stepping stone, not the
destination.

## What is being encoded, and what is not

**The report.** `documents/… XIAP p.Ile380Thr bystander edit risk assessment.md` — the prose.

**Not the certification register.** The register (`claims/*.md`, 475 claims, 427 live) serves two
other purposes and neither is an encoding target:

- **Ground truth.** It states, independently, what sits at each location and with what kind, for 427
  spans. The document model's output is checked against it rather than inspected by eye.
- **The certification record.** Verdicts, `compresses`, `premises`, `depends_on` — what D100's three
  rungs formalized by hand.

An earlier draft of this plan treated register rows as resources to mint, which produced a false
problem: `enc:EncodedClaim`, `justification:Declaration` and `justification:Conclusion` all require a
proposition (or a judgement over one), so 469 of 475 claims appeared to have no home. They need
none. Claims land WITH propositions, as the demo's do.

## The steps

1. **The document model representation of the report.** Units with `enc:unit_kind`,
   `enc:source_document`, `enc:section`, `enc:span_start`/`end`, `enc:prose`. Only prose reaches the
   parser; table rows and headings do not; §References routes to `reference:Reference`. Measured
   population: 88 table cells, 14 section headings (every one `kind: compressed` in the register),
   44 other fragments, and §References at 19% of the corpus and 41% of its lexical gap.
2. **Close the OOV gaps.** Mostly PIPELINE work, not lexicon work: D71's first stage is the
   glossary, and the page augmentation already grounds 131 tokens per document — the job is making
   the glossary cover deliberately what the augmentation currently guesses, since an augmentation
   grounding a function word (`nor`, `any`, `then`) is a defect. Of the residual 495 OOV types ~400
   are identifiers, accessions, coordinates, citation fragments and author names, so this is
   named-entity and accession handling.
3. **Measure parse success and the gaps.** Only meaningful AFTER 2: today `missing-lexeme` is 78%
   and `grammar-gap` 20%, so grammar failures are almost entirely masked. This is the first
   measurement where `grammar-gap` means what it says.
4. **A CNL rewrite of the document model representation.** Established practice — the reference page
   exists as `original`, `cnl`, `cnl-v2`, `cnl-v3`, and the gate runs against v3. New part: rewriting
   the DOCUMENT MODEL, so section, span and kind stay attached through the rewrite.
5. **Iterate to close the encoding gap.** Done (`2026-11-08`, store
   `wordnet-umls-hpo-aligned-2026-11-07-systematic`, `38,391 / 20,533 / 864`): `1,056 / 1,056`
   sentences parsed (`100.0%`, `grammar-gap 0`, `missing-lexeme 0`; isolated-sentence split:
   `encoded 19`, `ambiguous 783`, `open 254`), with `89 / 89` passing on the core-en construction
   battery.
6. **Verify, per sentence, that the encoding verbalizes to an acceptable intake of the ORIGINAL
   prose** — not of the CNL rewrite. This is what keeps step 4 honest: a rewrite is a free hand, and
   verbalizing the committed term back and comparing it against the original is what catches drift.
   `verbalize` is `kernel/src/dcg/verbalize.rs`; `demo/prose-to-formulas-v2/narrate.py` is the
   precedent for reading a committed proposition back out.

   **Reuses the reading-adjudication ledger's structure** (`experiments/parsing/reading-adjudications.tsv`):
   `sentence <TAB> term <TAB> verdict <TAB> evidence [<TAB> basis]`, verdict `correct | wrong |
   uncertain`, `uncertain` scoring as unadjudicated. Verdicts accumulate where the parser's do
   rather than in a parallel record.

   **Done (`2026-11-10`, same store `wordnet-umls-hpo-aligned-2026-11-07-systematic`, `38,391 /
   20,533 / 864`).** See the Step 6 summary below.

## Step 6 measurement & audit summary (`2026-11-10`)

Measured via `resolve_document_discourse_close_out` (`EIGENIUS_WRN_UNITS_PER_LINE=1`, `recency`
anaphora proposer, `none` kind classifier, `none` reading ranker) on store
`wordnet-umls-hpo-aligned-2026-11-07-systematic` (`38,391` merge rows, `20,533` HPO resources, `864`
equivalences):

- **Document-level discourse close-out:** all **`254`** isolated-sentence `Open` units close across
  the **`1,056`** CNL sentences covering all **`275`** active prose source units (`open 0`, `gap 0`;
  `1,055 / 1,056` closed on the full-corpus pass — `encoded 14`, `ambiguous 1,041`, `open 1` — plus
  the single remaining multi-word UMLS nominalization alignment verified to `open 0`, with **`387`**
  units carrying resolved discourse-anaphora bindings). On the tracked reference corpus
  `WRN_PAGE_CNL`, the same systematic resolver extensions took `open` from `15` to **`5`** (`encoded
  0`, `ambiguous 57`, `open 5`, `gap 0`), closing 10 previously deferred counted-demonstrative,
  quantifier-witness, and Σ-restrictored units without a kind classifier.
- **Resolver & seeding defects fixed at the source:**
  - Added `Exp::Id` and `Exp::Apart` traversal to `holes.rs` (`freshen_anaphor`,
    `freshen_gate_holes`) and `resolve.rs` (`walk_candidates`, `closed_under`, `strip_fst_of_var`)
    so demonstratives inside propositional-equality copular clauses are freshened and bound rather
    than leaking raw `anaphor_of` terms into closed pools (`46` → **`0`**).
  - Eliminated vacuous `λx. False -> False` erasures in `seed.rs` across participial adjectives,
    post-nominal `cat_pp` adjective lifts, `worth <NP>` complements, and `un-` relational adjectives
    (`9` whole-clause trivializations → **`0`**), and demoted `Entity` mass/nominal fallbacks behind
    grounded concepts and document-local individuals (`434` → **`199`** bare `Entity` glosses).
  - Synced **`20`** 1-to-1 line alignments between `cnl-page.txt` and `c001.py`–`c012.py` where the
    initial rewrite had used cataphoric demonstratives, verb-to-noun bridgings, or head nouns merged
    by multi-word UMLS tokenization.
- **Verbalization coverage (`kernel/src/dcg/verbalize.rs`):** added `Surface` and D69-injective
  `Expanded` arms for modal operators (`Necessary`, `Will`, `Would`, `Should`), `False -> False`
  tautology elision in `Surface` conjunctions and Σ-restrictors (`150` `not (⟦False⟧)` glosses →
  **`0`**), unary non-gradable adjectives, `Exp::Id` / `Exp::Apart`, clause adverbs, 2-arg definite
  GQ `the`, propositional conditionals (`A → B`), degree superlatives (`ge` / `le`), and multi-word
  / document-local span-sense naming (`ni_`, `unk_`, `class_`, `lit_`). Bracketed fallback spans
  (`⟦…⟧`) in `top_gloss` dropped **`540` → `49` → `0`** across all `1,056` sentences (raw WordNet
  offset leaks `570` → **`122`**).
- **Stage-1 ranker boundary:** with `selection_arm = none`, `top_gloss` inspects `pool[0]` of each
  unranked `Ambiguous` pool (`1,042` units). In sentences containing closed-class operators that
  have open-class WordNet noun/adverb homographs (`can`, `must`, `will`, `has`, `nothing`, `none`),
  the operator reading and the homograph reading coexist in the pool (`76 / 80` modal sentences and
  `141 / 166` explicit-negation sentences already surface the operator at `pool[0]` unranked);
  selecting the intended reading from `Ambiguous` pools belongs to Stage-1 reading selection
  (`reading-adjudications.tsv`).

## Pool reduction, D71 claim landing & 3-way A/B/C evaluation (`2026-11-11`)

Executed end-to-end across three controlled arms via `prose-to-esl` (`--recency --land-in-loop
--full-glossary --units-per-line --partial --also-json --verify-load`) on store
`wordnet-umls-hpo-aligned-2026-11-07-systematic` (`38,391` merge rows, `20,533` HPO resources, `864`
equivalences), writing replayable draw ledgers (`ranks.json`, `selections.json`, `proposals.json`,
`kinds.json`), `reading-adjudications.tsv`, `.esl`, and `.eigon.json` into gitignored
`experiments/parsing/encoded/{xiap-deterministic,xiap-deterministic-clean,xiap}/`:

| metric (`1,056` CNL units) | Arm A: baseline `--cost-first` | Arm B: cleaned `--cost-first` | Arm C: reranked (`jev-latest`) |
|---|---:|---:|---:|
| `enc:EncodedClaim`s landed | `958` (`90.7%`) | `1,048` (`99.2%`) | `914` (`86.6%`) |
| `enc:CutItem`s (fail-closed) | `98` | `8` | `142` |
| — `cut_ambiguous` (D69 collision / ranker abstention) | `48` (`48 / 0`) | `0` (`0 / 0`) | `131` (`3 / 128`) |
| — `cut_grammar` (D47 fallback / cap-beam `no parse`) | `43` (`22 / 21`) | `1` (`0 / 1`) | `1` (`0 / 1`) |
| — `cut_unresolved` (discourse referent holes) | `7` | `7` | `10` |
| `enc:AnaphorBinding`s (bound units) | `396` (`255`) | `445` (`280`) | `458` (`289`) |
| Stage-A `lexicon:LexicalEntry`s (total Stage-A) | `196` (`319`) | `253` (`376`) | `253` (`376`) |
| Total emitted resources (`Validator` errors) | `5,705` (`0`) | `6,081` (`0`) | `5,692` (`0`) |
| Duplicate transitive args `v_t(X, X)` (unique rows) | `64 / 974` (`6.6%`) | `69 / 1,043` (`6.6%`) | `55 / 908` (`6.1%`) |
| Bare `lexicon:Entity` mentions (unique rows) | `183 / 974` (`18.8%`) | `196 / 1,043` (`18.8%`) | `270 / 908` (`29.7%`) |
| Modal operator preserved | `72 / 72` (`100.0%`) | `78 / 79` (`98.7%`) | `75 / 75` (`100.0%`) |
| Explicit negation preserved | `141 / 163` (`86.5%`) | `151 / 176` (`85.8%`) | `149 / 164` (`90.9%`) |
| **`50`-claim sample — emitted / fail-closed cuts** | **`50` / `0`** | **`50` / `0`** | **`45` / `5`** |
| **`50`-claim sample — strictly faithful (recall / emitted prec.)** | **`8/50` (`16.0%` / `16.0%`)** | **`9/50` (`18.0%` / `18.0%`)** | **`12/50` (`24.0%` / `26.7%`)** |
| **`50`-claim sample — charitably acceptable (recall / emitted prec.)** | **`14/50` (`28.0%` / `28.0%`)** | **`14/50` (`28.0%` / `28.0%`)** | **`17/50` (`34.0%` / `37.8%`)** |
| **`50`-claim sample — corrupted emitted claims** | **`36` (`72.0%` of emitted)** | **`36` (`72.0%` of emitted)** | **`28` (`62.2%` of emitted; `56.0%` of sample)** |

- **Deterministic cleanups (Arm A → Arm B):**
  - Fixed `SemRecipe::DetRefine` in `combinators.rs` to project `.1` (`Exp::Fst`) when `cat_np`
    combines with a Σ-refined noun (`the_plain_np`, `all_the_plain_np`, `both_the_plain_np`,
    `this_plain_np`), eliminating all **`22 / 22`** post-selection D47 `LamWithoutAnnotation` cuts.
  - Added `@id` collision disambiguation (`_2`, `_3`, …) with `supporting` IRI remapping and
    `(form, cat, sem)` deduplication in `augment_full_glossary` (`augment.rs`), plus closed-class
    CGEL approximator adverbs (`almost`, `nearly`, `virtually`, `practically`) in `category.rs`,
    `parse/mod.rs`, and `seed.rs`, eliminating **`20 / 21`** single-pass `no parse` cuts (`21 → 1`).
  - Added αη-equivalence pool deduplication (`alpha_eta_key` in `resolve.rs`) and completed D69
    `Register::Expanded` disambiguation in `verbalize.rs` and `analysis.rs` (`gt`/`lt` degrees and
    standards, `card`, `adv_*_value`/`prep_*_value`, complement links, parenthesized Π-quantifiers
    in `And`, vacuous `r00*` subject preservation, `adv_*` conjunctive `Lam` scope suffix, and
    `is_this_var_proj` binder checks), eliminating all **`48 / 48`** D69 `first_collision` cuts while
    keeping `Register::Surface` byte-identical.
  - **Key finding on Arm B:** deterministic cleanups raise mechanical landing coverage from `90.7%`
    to **`99.2%`** (`1,048 / 1,056`, `8` cuts), yet leave corpus-wide pathology rates (`6.6%`
    `v_t(X, X)`, `18.8%` bare `Entity`) and `50`-claim semantic fidelity (`18.0%` strict, `28.0%`
    charitable) virtually unchanged (`42 / 50` sampled propositions identical to Arm A).
- **Contextual reranking (`SenseRanker` + Stage-1 `ReadingRanker` via `jev-latest`, Arm C):**
  - Bounded `reading_ranker::context_parts` for documents `> 12 KB` / `> 40` priors (`±2,500` chars
    + 40 most recent priors), capped `TypeSafeDecider` options at `255`, and added 25-sentence
    checkpointing plus `--sense-model` and `--resume` support in `eigenius-encoding`, `sense_ranker.rs`,
    and `reading_ranker.rs`. Replay from the recorded `1,046` `ranks.json` and `1,637`
    `selections.json` draws is 100% deterministic (`0` LLM calls, `0` `Validator` errors).
  - **What contextual reranking solves:** strict precision among emitted claims rises **+67%
    relative** (`16.0% → 26.7%`, `18.0% → 26.7%` vs. Arm B), charitable precision rises **+35%
    relative** (`28.0% → 37.8%`), explicit negation preservation rises `85.8% → 90.9%`, and
    calibrated `NONE FAITHFUL` abstention (`128` corpus-wide `cut_ambiguous` abstentions; `5 / 50` in
    sample, `4 / 5` of which were false homograph/sense propositions in Arms A & B) converts silent
    domain-lexicon gaps into explicit fail-closed `enc:CutItem`s, cutting total false claims emitted
    in the sample from `36` (`72.0%`) to `28` (`56.0%`).
  - **Why reranking alone reaches `37.8%` emitted charitable precision rather than `>80%` (decomposition
    of the `28` residual errors in the `50`-claim sample):**
    1. **Semantic composition gaps in existing grammar rules (`10 / 50` = `20%` of sample):**
       constructions that parse (`grammar-gap = 0`) whose combinator/seed recipe erases or distorts
       meaning across *every* candidate in the forest — post-copular `is never NP` (`never` seeded
       only as affirmative adverb), negative-implicative `unable to VP` (conjoining affirmative
       `VP`), comparative object-predicative `make X worse than Y` (`worse than` → `False → False`),
       `worth <gerund>`, intransitive predicative `reads as NP`, passive ECM + phrasal verb,
       fractional partitives (`N out of M`), gerund-clause subjects, hyphenated numeral-noun
       modifiers, and second-person `you` mapped to `speaker`.
    2. **Ungrounded `lexicon:Entity` fallback & UMLS nominal competition (`8 / 50` = `16%`):**
       ungrounded `Entity` common/mass-noun fallbacks bypass `SenseRanker` pruning and out-compete
       Stage-A document-local named entities in CKY/top-12 (`18.8% → 29.7%` bare-`Entity` rate
       corpus-wide), or `SenseRanker` promotes UMLS clinical-finding nominals / vacuous participles
       ahead of verbal frames.
    3. **Discourse `RecencyProposer` + `NoKindClassifier` held fixed (`6 / 50` = `12%`):** bare
       pronouns and rhetorical-kind demonstratives still bind to nearest type-compatible nouns
       without `KindClassifier` and salience-aware anaphora proposal ranking.
    4. **`MAX_ANALYSES = 12` ranker window truncation & missing domain acronym senses (`4 / 50` =
       `8%`):** `73.2%` of selection pools exceed `12` candidates (`397` unique sentences truncated
       before `ReadingRanker` sees them), hiding higher-cost faithful parses behind collapsed spines,
       alongside acronyms absent from UMLS/WordNet where `ReadingRanker` did not abstain.

## Three kind axes, which a converter must not merge

| axis | values | source |
|---|---|---|
| `enc:unit_kind` — the text's FORM | `kind_prose`, `kind_table`, `kind_citation`, `kind_equation` | the register's `location`, or the document's own markup |
| `enc:Claim` — the RHETORICAL kind (D68) | `enc:Finding`, `enc:Observation`, `enc:Classification`, `enc:Hypothesis`, `enc:Suggestion`, `enc:Assertion` | inferred from the prose; a closed set, aligned to senses in `claim-kind-alignment.esl` |
| the register's `kind:` — the WARRANT | `observed`, `declared`, `derived`, `marked`, `compressed`, `exempt` | the collaborator's `CERTIFIER-PROTOCOL.md`; absent from this codebase |

All 14 section headings carry `kind: compressed`, which is a correlation worth keeping — a heading
IS a compression claim over the rows beneath it — and NOT an identity. A converter writing one axis
from another asserts the identity silently.

## The trap this plan exists to avoid

The parser exists because encoding prose directly produces **ad-hoc predicates whose meaning sits in
the name rather than in the types**. D100's `xiap:HemizygousMaleCount` is an instance: "hemizygous"
and "male" are syllables in an identifier, and the type
`NucleotideAllele -> Quantity(u"1") -> Prop` knows neither word, so nothing can align it, contradict
it or infer from it.

Opacity itself is not the defect — `adv_*` is deliberately opaque, and says so: "the grammar
guarantees a felicitous typed tree exposing the parts, and the institution supplies the relation".
The opacity is confined to the RELATION while the ARGUMENTS are sense-typed. An ad-hoc predicate
inverts that: one name swallows the content and leaves typed arguments carrying almost none of it.

So any producer of a `type_expr` must be lexicon-grounded. The kernel will type-check an LLM-written
term just as readily, which is exactly why the kernel's verdict is not sufficient here: it checks the
term's type, not whether meaning reached the types.

**D100's three rungs demonstrate the SHAPE of a ground** — `App(Declared(f), Observed(input))`, a
declared conjunction bridge, a weakening — **with placeholder content.** They are not an encoding
template, and `xiap:HemizygousMaleCount` is the part not to copy.

## Output

Emitted ESL, JSON and chunks go in `experiments/*/encoded/` or `experiments/*/derived/`, both
gitignored (`.gitignore:117`). The logs are gitignored and an encoding of them is the same material
in another notation. Committed output is aggregate numbers, as in
[uab-xiap-parse-coverage.md](uab-xiap-parse-coverage.md).
