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

//! D62 — encoding-pipeline **core-algorithm prototype** (text-only).
//!
//! This is a *prototype*, not the packaged engine: no RPC, no ontology, no institution,
//! no LLM. It exercises the **control-flow heart** of the pipeline against the real DCG
//! parser + a seeded slice of real WordNet, to find the right shape before we build out
//! platform integration:
//!
//!   segment(text)  →  for each unit: parse_scoped  →  classify the outcome:
//!       1 closed parse        → Encoded        (felicity-gated to Prop)
//!       >1 closed parses      → Ambiguous      (stub: pick rank-0; LLM-select later)
//!       empty + unknown token → MissingLexeme  (S5a target: search/inject later)
//!       empty + all known     → GrammarGap     (S5b target: reformulate later)
//!
//! The LLM-proposer stages (segmentation beyond sentence-split, disambiguation,
//! reference resolution, reformulation, lexical recovery) are deliberately **stubbed** —
//! the prototype's job is the control flow + the four-way outcome taxonomy + the gap
//! stream, on real data. Run it with:
//!
//!     cargo test -p eigenius-wordnet --test encoding_prototype -- --nocapture

use std::sync::Arc;

use eigenius_kernel::dcg::{
    is_nonprose, pretty_term, segment_sentences, tokenize, Item, Lemmatizer, LexicalIndex,
};
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::{bootstrap, esl};
use eigenius_wordnet::convert::render_document;
use eigenius_wordnet::import::{read_sense_ranks, select_synsets, SeedSpec};
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;

// ─── harness (mirrors wordnet_scale.rs; duplicated — this is a prototype) ──────────

const DICT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/WordNet-3.0/dict"
);

fn dict_missing() -> bool {
    if std::path::Path::new(DICT).join("data.noun").exists() {
        return false;
    }
    eprintln!("SKIP: WordNet dict not found under {DICT} — run scripts/provision-wordnet.sh");
    true
}

/// Stand up a seeded, hypernymy-closed slice of real WordNet over the bootstrap head.
fn stand_up(spec: &SeedSpec) -> Arc<Layer> {
    let chosen = select_synsets(std::path::Path::new(DICT), spec).expect("read WordNet dict");
    let ranks = read_sense_ranks(std::path::Path::new(DICT), &spec.pos).expect("read index ranks");
    let (doc, _rep) = render_document(&chosen, &ranks);
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources = esl::compile_against_layer(&doc, ctx.head()).expect("wn compiles");
    let mut b = LayerBuilder::new("wn", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add wn resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn morphy() -> MorphyLemmatizer {
    MorphyLemmatizer::load(std::path::Path::new(DICT)).expect("load Morphy")
}

/// Does this sem kernel-gate to a `Prop`? (the felicity confirmation)
fn gates_to_prop(layer: &Arc<Layer>, sem: &Exp) -> bool {
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(layer));
    matches!(check_infer(&mut ctx, sem), Ok(ty) if readback_val(0, &ty) == Exp::Sort(0))
}

// ─── the prototype driver ──────────────────────────────────────────────────────────

/// The classified outcome of attempting to encode one text unit — the four-way taxonomy
/// the pipeline routes on (D62 §4). Open (hole-bearing) parses are out of scope here
/// (reference resolution = D64, deferred); `parse_scoped` returns the closed forest.
#[derive(Debug)]
enum Outcome {
    /// Exactly one felicitous parse → a typed `Prop`.
    Encoded { sem: Exp, is_prop: bool },
    /// Multiple felicitous parses → structural ambiguity. Stub: keep the rank-0 reading
    /// (the parser already ranks by `Cost`); LLM context-select replaces this (S4).
    Ambiguous {
        count: usize,
        top_sem: Exp,
        is_prop: bool,
    },
    /// No parse, and ≥1 token has no lexical entry → lexical recovery target (S5a).
    MissingLexeme { unknown: Vec<String> },
    /// No parse, but every token is known → grammar gap → reformulation target (S5b).
    GrammarGap,
}

struct UnitReport {
    text: String,
    outcome: Outcome,
}

/// The core control flow for one unit: parse → classify.
fn encode_unit(
    text: &str,
    index: &LexicalIndex,
    lem: &dyn Lemmatizer,
    layer: &Arc<Layer>,
) -> Outcome {
    let forest: Vec<Item> = index.parse_scoped(text, lem, None);
    match forest.len() {
        0 => {
            // Diagnose: missing lexeme (route S5a) vs grammar gap (route S5b). Non-prose
            // tokens (stats/figure-refs, S0) are routed out — not counted as missing lexemes.
            let unknown: Vec<String> = tokenize(text)
                .into_iter()
                .filter(|t| !is_nonprose(t) && !index.has_token(t, lem))
                .collect();
            if unknown.is_empty() {
                Outcome::GrammarGap
            } else {
                Outcome::MissingLexeme { unknown }
            }
        }
        1 => {
            let sem = forest[0].sem.clone();
            let is_prop = gates_to_prop(layer, &sem);
            Outcome::Encoded { sem, is_prop }
        }
        n => {
            // forest is ranked by Cost; rank-0 is the stub selection.
            let top_sem = forest[0].sem.clone();
            let is_prop = gates_to_prop(layer, &top_sem);
            Outcome::Ambiguous {
                count: n,
                top_sem,
                is_prop,
            }
        }
    }
}

fn encode_doc(
    doc: &str,
    index: &LexicalIndex,
    lem: &dyn Lemmatizer,
    layer: &Arc<Layer>,
) -> Vec<UnitReport> {
    segment_sentences(doc)
        .into_iter()
        .map(|text| {
            let outcome = encode_unit(&text, index, lem, layer);
            UnitReport { text, outcome }
        })
        .collect()
}

fn print_report(report: &[UnitReport]) {
    eprintln!("\n=== encoding prototype report ===");
    for u in report {
        match &u.outcome {
            Outcome::Encoded { sem, is_prop } => eprintln!(
                "  [ENCODED  prop={is_prop}] {:?}\n      sem: {}",
                u.text,
                pretty_term(sem)
            ),
            Outcome::Ambiguous {
                count,
                top_sem,
                is_prop,
            } => eprintln!(
                "  [AMBIG×{count} prop={is_prop}] {:?}\n      top: {}",
                u.text,
                pretty_term(top_sem)
            ),
            Outcome::MissingLexeme { unknown } => {
                eprintln!("  [MISSING  {unknown:?}] {:?}", u.text)
            }
            Outcome::GrammarGap => eprintln!("  [GRAMMAR-GAP] {:?}", u.text),
        }
    }
    eprintln!("=================================\n");
}

// ─── the test ────────────────────────────────────────────────────────────────────

const SEEDS: &[&str] = &[
    "dog", "cat", "animal", "bird", "fish", "worm", "chase", "see", "eat",
];

/// A cleaned page of real WRN-paper prose (user-provided; OCR noise removed).
const WRN_PAGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/publications/WRN-Helicase-Nature-OCR/first-page-cleaned.txt"
);

/// Exploratory (not an assertion): feed real, cleaned WRN-paper prose through the
/// prototype and report the outcome distribution. Seeds the WordNet slice from the
/// page's OWN words, so a MissingLexeme is *genuine* out-of-vocabulary (gene names,
/// acronyms, multiword domain terms), not an artefact of a toy slice. Heavy (builds a
/// slice from the page vocab) — run manually:
///
///     cargo test -p eigenius-wordnet --test encoding_prototype \
///         prototype_over_wrn_first_page -- --ignored --nocapture
#[test]
#[ignore = "exploratory: feeds real WRN-paper prose; run with --ignored --nocapture"]
fn prototype_over_wrn_first_page() {
    if dict_missing() {
        return;
    }
    let page = match std::fs::read_to_string(WRN_PAGE) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIP: {WRN_PAGE} not found");
            return;
        }
    };

    // Seed the slice from the page's own alphabetic tokens, so a MissingLexeme is
    // genuine out-of-vocabulary (not an artefact of a toy slice).
    let seeds: std::collections::BTreeSet<String> = tokenize(&page)
        .into_iter()
        .filter(|t| t.chars().all(|c| c.is_ascii_alphabetic()) && t.len() > 2)
        .collect();
    let seed_refs: Vec<&str> = seeds.iter().map(String::as_str).collect();
    eprintln!(
        "seeding WordNet slice from {} distinct page tokens",
        seed_refs.len()
    );

    let layer = stand_up(&SeedSpec::seeded(seed_refs));
    let index = LexicalIndex::build(Arc::clone(&layer));
    let lem = morphy();

    let report = encode_doc(&page, &index, &lem, &layer);

    let (mut enc, mut amb, mut miss, mut gap) = (0, 0, 0, 0);
    let mut oov: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for u in &report {
        match &u.outcome {
            Outcome::Encoded { .. } => enc += 1,
            Outcome::Ambiguous { .. } => amb += 1,
            Outcome::MissingLexeme { unknown } => {
                miss += 1;
                oov.extend(unknown.iter().cloned());
            }
            Outcome::GrammarGap => gap += 1,
        }
    }
    eprintln!(
        "\n=== WRN first page: {} units → encoded {enc}, ambiguous {amb}, \
         missing-lexeme {miss}, grammar-gap {gap} ===",
        report.len()
    );
    eprintln!("distinct out-of-vocabulary tokens ({}): {oov:?}", oov.len());
    eprintln!("\n--- first 8 units (90-char preview) ---");
    for u in report.iter().take(8) {
        let tag = match &u.outcome {
            Outcome::Encoded { .. } => "ENCODED",
            Outcome::Ambiguous { .. } => "AMBIG",
            Outcome::MissingLexeme { .. } => "MISSING",
            Outcome::GrammarGap => "GRAMMAR-GAP",
        };
        let t: String = u.text.chars().take(90).collect();
        eprintln!("  [{tag}] {t}…");
    }
}

#[test]
fn prototype_classifies_a_text_document_into_the_four_outcomes() {
    if dict_missing() {
        return;
    }
    let layer = stand_up(&SeedSpec::seeded(SEEDS.iter().copied()));
    let index = LexicalIndex::build(Arc::clone(&layer));
    let lem = morphy();

    // A small "document": one parseable claim, one with an unknown word, one with all
    // words known but no valid sentence structure.
    let doc = "A dog sees a bird. A dog sees a quokka. Dog dog dog.";
    let report = encode_doc(doc, &index, &lem, &layer);
    print_report(&report);

    assert_eq!(report.len(), 3, "segmenter split into three units");

    // Unit 0: parses (one or more felicitous readings), and the chosen reading is a Prop.
    match &report[0].outcome {
        Outcome::Encoded { is_prop, .. } | Outcome::Ambiguous { is_prop, .. } => {
            assert!(*is_prop, "the chosen reading gates to Prop")
        }
        other => panic!("unit 0 should parse; got {other:?}"),
    }

    // Unit 1: 'quokka' is not in the seeded slice → missing-lexeme (S5a target).
    match &report[1].outcome {
        Outcome::MissingLexeme { unknown } => {
            assert!(
                unknown.iter().any(|t| t == "quokka"),
                "missing-lexeme should flag 'quokka'; got {unknown:?}"
            )
        }
        other => panic!("unit 1 should be missing-lexeme; got {other:?}"),
    }

    // Unit 2: all tokens known ('dog') but no valid parse → grammar gap (S5b target).
    match &report[2].outcome {
        Outcome::GrammarGap => {}
        other => panic!("unit 2 should be a grammar gap; got {other:?}"),
    }
}

// ─── P1 — S0 verification on real WRN prose (uses the DCG engine's S0) ───────────────
//
// S0 is now in the engine: `dcg::segment_sentences` (segmentation) + `dcg::is_nonprose`
// (routing) + the em-dash/slash/bracket splitting folded into `dcg::tokenize`. This test
// confirms, on the cleaned WRN first page, that the engine S0 fixes the naive over-split
// (4 paragraphs → 47 units) and routes stats/figure-refs out while keeping gene symbols.

#[test]
#[ignore = "exploratory P1: run with --ignored --nocapture"]
fn p1_s0_cleans_wrn_page() {
    let page = match std::fs::read_to_string(WRN_PAGE) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIP: {WRN_PAGE} not found");
            return;
        }
    };

    let naive_units = page
        .split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count();
    let units = segment_sentences(&page);

    let (mut lex, mut non) = (0usize, 0usize);
    let mut routed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut lexset: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for u in &units {
        for t in tokenize(u) {
            if is_nonprose(&t) {
                non += 1;
                routed.insert(t);
            } else {
                lex += 1;
                lexset.insert(t);
            }
        }
    }

    eprintln!("\n=== P1 S0 (engine) on WRN first page ===");
    eprintln!(
        "segmentation: naive {naive_units} units  →  S0 {} units",
        units.len()
    );
    eprintln!("tokens: {lex} lexical, {non} routed-out (non-prose)");
    eprintln!(
        "routed-out (distinct, sample): {:?}",
        routed.iter().take(25).collect::<Vec<_>>()
    );
    for gene in ["mlh1", "msh2", "brca1", "kras", "braf", "wrn"] {
        eprintln!(
            "  gene {gene:>6}: lexical={} routed={}",
            lexset.contains(gene),
            routed.contains(gene)
        );
    }

    assert!(
        units.len() < naive_units,
        "engine S0 over-splits far less than naive"
    );
    assert!(
        routed.iter().any(|t| t.starts_with("10")),
        "stat values routed out"
    );
    assert!(
        !routed.contains("mlh1") && !routed.contains("msh2"),
        "genes kept as lexical"
    );
    // em-dash splitting in dcg::tokenize: no fused tokens survive.
    assert!(
        !lexset.iter().any(|t| t.contains('—')),
        "em-dash-fused tokens split"
    );
}
