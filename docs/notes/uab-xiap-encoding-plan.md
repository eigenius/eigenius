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
5. **Iterate to close the encoding gap.**
6. **Verify, per sentence, that the encoding verbalizes to an acceptable intake of the ORIGINAL
   prose** — not of the CNL rewrite. This is what keeps step 4 honest: a rewrite is a free hand, and
   verbalizing the committed term back and comparing it against the original is what catches drift.
   `verbalize` is `kernel/src/dcg/verbalize.rs`; `demo/prose-to-formulas-v2/narrate.py` is the
   precedent for reading a committed proposition back out.

   **Reuses the reading-adjudication ledger's structure** (`experiments/parsing/reading-adjudications.tsv`):
   `sentence <TAB> term <TAB> verdict <TAB> evidence [<TAB> basis]`, verdict `correct | wrong |
   uncertain`, `uncertain` scoring as unadjudicated. Verdicts accumulate where the parser's do
   rather than in a parallel record.

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
