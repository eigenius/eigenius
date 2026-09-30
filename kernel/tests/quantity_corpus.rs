// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! D95 slice 5 — the quantity corpus (`experiments/parsing/quantities/`), parsed over the bootstrap
//! chain and the corpus's content words, so grammar coverage of quantities is checked without a
//! database. A covered row must parse, and every reading must contain its relations and render its
//! values; a gap row must still not parse, so a construction that arrives updates its row.

use std::sync::Arc;

use eigenius_kernel::dcg::verbalize::{unit_sense_names, verbalize, Vb};
use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const CORPUS: &str = include_str!("../../experiments/parsing/quantities/corpus.tsv");
const CONTENT_WORDS: &str = include_str!("../../experiments/parsing/quantities/content-words.esl");

struct Row<'a> {
    sentence: &'a str,
    /// The ontology relations every reading contains; empty for a gap.
    relations: Vec<&'a str>,
    /// The quantities every reading renders; for a gap, the missing construction.
    values: Vec<&'a str>,
}

impl Row<'_> {
    fn is_gap(&self) -> bool {
        self.relations.is_empty()
    }
}

fn rows() -> Vec<Row<'static>> {
    CORPUS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(cols.len(), 4, "four columns: {l:?}");
            let relations = match cols[1] {
                "gap" => Vec::new(),
                r => r.split(' ').collect(),
            };
            Row {
                sentence: cols[0],
                relations,
                values: cols[2].split(';').collect(),
            }
        })
        .collect()
}

fn layer() -> Arc<Layer> {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let resources = esl::compile(CONTENT_WORDS, ctx.head()).expect("content words compile");
    let mut b = LayerBuilder::new("quantity-corpus", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add content word");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Every word of every row is known, so a gap is the grammar's and not the fixture's.
#[test]
fn every_word_is_known() {
    let parser = Parser::build(layer());
    for row in rows() {
        let unknown = parser.unknown_words(row.sentence, &Identity);
        assert!(unknown.is_empty(), "{}: {unknown:?}", row.sentence);
    }
}

#[test]
fn every_covered_row_meets_its_consumer() {
    let layer = layer();
    let parser = Parser::build(Arc::clone(&layer));
    let mut failures = Vec::new();
    for row in rows().iter().filter(|r| !r.is_gap()) {
        let names = unit_sense_names(row.sentence, &parser, &Identity, &layer);
        let vb = Vb::surface(&names, &layer);
        let readings = parser.parse(row.sentence, &Identity);
        if readings.is_empty() {
            failures.push(format!("{}: no parse", row.sentence));
            continue;
        }
        for it in &readings {
            let sem = format!("{:?}", it.sem());
            let text = verbalize(it.sem(), &vb);
            let missing: Vec<&str> = row
                .relations
                .iter()
                .filter(|r| !sem.contains(&format!("urn:eigenius:ontology:{r}\"")))
                .chain(row.values.iter().filter(|v| !text.contains(*v)))
                .copied()
                .collect();
            if !missing.is_empty() {
                failures.push(format!(
                    "{}: a reading lacks {missing:?}\n    {text}\n    {}",
                    row.sentence,
                    pretty_term(it.sem())
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_gap_is_still_a_gap() {
    let parser = Parser::build(layer());
    for row in rows().iter().filter(|r| r.is_gap()) {
        let readings = parser.parse(row.sentence, &Identity);
        assert!(
            readings.is_empty(),
            "{} parses ({} readings), so `{}` has arrived: update its row in corpus.tsv",
            row.sentence,
            readings.len(),
            row.values[0]
        );
    }
}
