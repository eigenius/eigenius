# Quantity corpus (D95 slice 5)

Sentences with measured quantities, in the controlled-language register, derived from the WRN
methods (Chan et al., *Nature* 2019) and D95's examples. The methods text is gitignored and not
CC-licensed; these rewrites are not quotations.

- [`corpus.tsv`](corpus.tsv): one sentence per row with the relations every reading must contain and
  the quantities every reading must render, in base units as `verbalize` prints them (`37 °C` is
  `6203/20 K`). A `gap` row names the construction it lacks and who owns it.
- [`content-words.esl`](content-words.esl): the nouns and verbs around the quantities, in the shapes
  the WordNet importer emits. The closed class supplies the rest.

## Checked without a database

```bash
cargo test -p eigenius-kernel --test quantity_corpus
```

`kernel/tests/quantity_corpus.rs` parses every row over the bootstrap chain plus
`content-words.esl`:

- every word is known, so a gap is the grammar's;
- a covered row parses, and every reading contains its relations and renders its values;
- a gap row still does not parse. When one does, its construction has arrived: update the row.

As committed: 29 covered rows, all passing. The two `g` rows have two readings each, gram and
standard gravity (D93). Slices 6 and 7 (D95 implementation plan) added seven, each from a sentence of
the paper: a range (`4–12% gels`, `80–90% confluence`), an approximation (`approximately 100 mm³`), a
bound symbol (`>90%`), and counts — a plain one (`three sgRNAs`), a bounded one (`more than one MMR
gene`) and a count range (`4–7 foci`). The 6 gap rows:

| Gap | Owner |
|---|---|
| a measure phrase modifying a PP, `72 h after transduction` | slice 8 |
| a pseudo-partitive, `10 μg ml⁻¹ of colcemid` | slice 8 |
| `every N unit`, `every 3 days` | slice 8 |
| `every N unit` over a range, `every 2–3 days` | slice 8 |
| a fronted PP adjunct, `After 24 h, …` | pre-existing: `After the dose, …` has no parse either |
| a coordinated NP as an adjunct preposition's object | pre-existing: `treated with the dose and the diet` has no parse either; a verb that governs its `with` takes the group |

## Over the full lexicon

After the reseed, the same sentences run through the parse-rate harness as a page:

```bash
grep -v '^#' experiments/parsing/quantities/corpus.tsv | cut -f1 > /tmp/quantity-page.txt
scripts/measure-parse-rate.sh --page /tmp/quantity-page.txt
```

The WordNet and UMLS senses of the content words replace the fixture's single sense, so the reading
counts there measure sense ambiguity, not the grammar.

Measured `2026-09-27`, cap-only, at `6a3eabf` on `wordnet-umls-aligned-2026-09-27`: of 28 units,
3 encoded, 17 ambiguous (2 to 12 readings, and 200 for the `and with` row), 7 grammar gaps, 1
non-prose (the range).

- The grammar gaps are 4 of the gap rows and 3 covered rows whose verbs the lexicon lacks in the
  passive. WordNet lists `incubate` as intransitive only (both senses, frames 1 and 2), so
  `The cells were incubated.` has no parse; it has no verb `electroporate`. `The cells were kept at
  37 °C for 1 h.` parses (9 readings).
- The coordination gap row parses (2 readings): the imported `treat` governs `with`, and a governed
  argument takes a coordinated group where an adjunct does not.
