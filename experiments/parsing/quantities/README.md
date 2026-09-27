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

As committed: 22 covered rows, all passing, each with one reading. The two `g` rows are the
exception, with two readings each, gram and standard gravity (D93). The 6 gap rows:

| Gap | Owner |
|---|---|
| a measure phrase modifying a PP, `72 h after transduction` | not in D95's slices |
| a pseudo-partitive, `10 μg ml⁻¹ of colcemid` | not in D95's slices |
| `every N unit` | out of v1 (D95 Scope) |
| a range, `4–12% gels` (one non-prose token) | out of v1 (D95 Scope) |
| a fronted PP adjunct, `After 24 h, …` | pre-existing: `After the dose, …` has no parse either |
| NP coordination as a preposition's object | pre-existing: `with the etoposide and the hydroxyurea` has no parse either |

## Over the full lexicon

After the reseed, the same sentences run through the parse-rate harness as a page:

```bash
grep -v '^#' experiments/parsing/quantities/corpus.tsv | cut -f1 > /tmp/quantity-page.txt
scripts/measure-parse-rate.sh --page /tmp/quantity-page.txt
```

The WordNet and UMLS senses of the content words replace the fixture's single sense, so the reading
counts there measure sense ambiguity, not the grammar.
