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

//! D63 §8.7 Slice 7 — the **scale-up harness**: stand the WordNet import up as a
//! *standing, parseable layer* and run a battery through the real engine.
//!
//! The path is the whole point of the slice: `select_synsets` → `render_document`
//! → compile over the bootstrap head → `LayerBuilder::build` → **hold the
//! `Arc<Layer>`** → `Parser::build` → `parse` with WordNet's Morphy →
//! kernel-gate each parse to a `Prop`. The always-on test runs at **Stage A**
//! (a seeded, hypernymy-closed slice of real WordNet vocabulary) so it is fast
//! and exact; the `#[ignore]` test stands up a large `--limit` slice and records
//! the **forest-size + timing baselines** (done-when #3) that drive the Stage-B
//! sense-ambiguity policy. Run it with:
//!
//!     cargo test -p eigenius-wordnet --test wordnet_scale -- --ignored --nocapture

use std::sync::Arc;
use std::time::Instant;

use eigenius_kernel::dcg::{Identity, Item, Parser};
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::{bootstrap, esl};
use eigenius_wordnet::convert::{render_document, MassNouns};
use eigenius_wordnet::import::{read_sense_ranks, select_synsets, SeedSpec};
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;

/// The WordNet 3.0 dict — `<repo-root>/references/WordNet-3.0/dict`. WordNet is a
/// third-party corpus, NOT vendored (`references/` is gitignored); provision it with
/// `scripts/provision-wordnet.sh`.
const DICT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../references/WordNet-3.0/dict"
);

/// Skip a dict-dependent test cleanly when WordNet isn't provisioned (fresh checkout / CI
/// — `references/` is gitignored). Returns `true` (after logging) when the dict is absent,
/// so the caller early-returns instead of panicking. Checks `data.noun` as the sentinel.
fn dict_missing() -> bool {
    if std::path::Path::new(DICT).join("data.noun").exists() {
        return false;
    }
    eprintln!(
        "SKIP: WordNet dict not found under {DICT} — run scripts/provision-wordnet.sh \
         (references/ is gitignored; this test needs the WordNet 3.0 corpus)"
    );
    true
}

/// Stand up a WordNet layer for `spec`: select (closed under hypernymy) → render →
/// compile over the bootstrap head → build (in-memory). Returns the standing layer
/// plus the wall-clock cost of the build (the index-independent stand-up baseline).
fn stand_up(spec: &SeedSpec) -> (Arc<Layer>, std::time::Duration) {
    let chosen = select_synsets(std::path::Path::new(DICT), spec).expect("read WordNet dict");
    let ranks = read_sense_ranks(std::path::Path::new(DICT), &spec.pos).expect("read index ranks");
    let (doc, rep) = render_document(&chosen, &ranks, &MassNouns::new());
    eprintln!(
        "stand_up: {} synsets → {} noun classes, {} instances, {} verb + {} adj axioms, {} entries",
        chosen.len(),
        rep.noun_classes,
        rep.instances,
        rep.verb_axioms,
        rep.adj_axioms,
        rep.entries,
    );

    let t0 = Instant::now();
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources =
        esl::compile_against_layer(&doc, ctx.head()).expect("wn compiles over bootstrap");
    let mut b = LayerBuilder::new("wn", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add wn resource");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    (layer, t0.elapsed())
}

/// WordNet's Morphy over the full dict — the real surface→lemma bridge.
fn morphy() -> MorphyLemmatizer {
    MorphyLemmatizer::load(std::path::Path::new(DICT)).expect("load Morphy from dict")
}

/// Whether a parse's sem kernel-gates to a `Prop` (the felicity confirmation).
fn gates_to_prop(layer: &Arc<Layer>, sem: &Exp) -> bool {
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(layer));
    matches!(check_infer(&mut ctx, sem), Ok(ty) if readback_val(0, &ty) == Exp::Sort(0))
}

/// How many of a forest's parses gate to a `Prop`.
fn props(layer: &Arc<Layer>, forest: &[Item]) -> usize {
    forest
        .iter()
        .filter(|p| gates_to_prop(layer, p.sem()))
        .count()
}

/// The representative battery — declaratives over real WordNet nouns/verbs +
/// the committed closed-class determiners. Seeds (below) guarantee the content
/// words + their hypernym closure are present.
const BATTERY: &[&str] = &[
    "every dog chases a cat",
    "a dog sees a bird",
    "no cat eats a fish",
    "every animal sees a dog",
    "a bird eats a worm",
];

const BATTERY_SEEDS: &[&str] = &[
    "dog", "cat", "animal", "bird", "fish", "worm", "chase", "see", "eat",
];

#[test]
fn stage_a_battery_parses_to_props_over_real_wordnet() {
    // Stand up a seeded, hypernymy-closed slice of REAL WordNet and parse the
    // battery through the engine to kernel-checked Props. Sense ambiguity makes the
    // forest > 1 (measured below); the felicity gate keeps the well-typed ones.
    if dict_missing() {
        return;
    }
    let (layer, build) = stand_up(&SeedSpec::seeded(BATTERY_SEEDS.iter().copied()));
    let t0 = Instant::now();
    let index = Parser::build(Arc::clone(&layer));
    let index_build = t0.elapsed();
    let lemma = morphy();
    eprintln!("  layer build {build:?}, index build {index_build:?}");

    eprintln!(
        "  {:<28} {:>7} {:>6} {:>10}",
        "sentence", "forest", "props", "parse"
    );
    for &s in BATTERY {
        let t = Instant::now();
        let forest = index.parse(s, &lemma);
        let dt = t.elapsed();
        let n_props = props(&layer, &forest);
        eprintln!("  {s:<28} {:>7} {n_props:>6} {dt:>10.2?}", forest.len());
        assert!(
            n_props >= 1,
            "'{s}' must yield at least one felicitous Prop over real WordNet (forest={})",
            forest.len()
        );
        // RANK witness (D63 §8.7 Stage B): the forest is returned lowest-cost
        // (most-frequent-sense) first — non-decreasing in cost.
        assert!(
            forest.windows(2).all(|w| w[0].cost() <= w[1].cost()),
            "'{s}': forest must be ranked by ascending sense-frequency cost"
        );
        // CAP witness: never more than the default cap (the 1.8k blow-up is bounded).
        assert!(
            forest.len() <= eigenius_kernel::dcg::DEFAULT_FOREST_CAP,
            "'{s}': forest must be capped at DEFAULT_FOREST_CAP"
        );
    }
}

/// Readback-normalized sem string, for counting DISTINCT meanings in a forest.
fn sem_key(sem: &Exp) -> String {
    format!(
        "{:?}",
        readback_val(0, &eval(sem, &Rho::Nil).expect("eval sem"))
    )
}

#[test]
fn no_spurious_duplication_from_feature_vars() {
    // D63 §8.10 — the object determiner now PRESERVES the verb's finiteness +
    // subject-number (feature variables, not `*_any` laundering). So every parse in
    // the forest is a DISTINCT sense-tuple: total == distinct, with no byte-identical
    // copies. (Before the fix, Morphy's `eats → {eat, eats}` let the base + plural verb
    // forms also pass the singular subject determiner, tripling the forest.)
    if dict_missing() {
        return;
    }
    let (_layer, index) = {
        let (l, _) = stand_up(&SeedSpec::seeded(BATTERY_SEEDS.iter().copied()));
        let i = Parser::build(Arc::clone(&l));
        (l, i)
    };
    let lemma = morphy();
    // Stays under the cap so the WHOLE forest is observable (no truncation hiding dups).
    let forest = index.parse("no cat eats a fish", &lemma);
    let mut keys: Vec<String> = forest.iter().map(|p| sem_key(p.sem())).collect();
    let total = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(
        total,
        keys.len(),
        "every parse must be a distinct meaning — got {total} parses, {} distinct \
         (spurious duplication regressed)",
        keys.len()
    );
}

#[test]
fn singular_subject_rejects_bare_and_plural_verb() {
    // The agreement bite the feature-variable fix restores: a SINGULAR subject with the
    // bare/plural verb form has NO parse — even though Morphy reaches those forms from
    // "eats". Before the fix the object determiner laundered finiteness/number to `_any`,
    // so "every cat eat a fish" wrongly parsed (112×).
    if dict_missing() {
        return;
    }
    let (_layer, index) = {
        let (l, _) = stand_up(&SeedSpec::seeded(BATTERY_SEEDS.iter().copied()));
        let i = Parser::build(Arc::clone(&l));
        (l, i)
    };
    let lemma = morphy();
    assert!(
        !index.parse("every cat eats a fish", &lemma).is_empty(),
        "the 3sg verb with a singular subject must parse"
    );
    assert!(
        index.parse("every cat eat a fish", &Identity).is_empty(),
        "the bare/plural verb form with a singular subject must NOT parse (agreement bites)"
    );
}

#[test]
#[ignore = "heavy: stands up a large WordNet slice; run with --ignored --nocapture for baselines"]
fn stage_b_baselines_over_a_large_slice() {
    // Done-when #3: the witnessed baselines that justify the Stage-B sense-ambiguity
    // policy. Stand up the first ~12k synsets/POS (closed) and record index-build
    // time + per-sentence parse-time + forest-size distribution over the battery.
    let (layer, build) = stand_up(&SeedSpec::limit(12_000));
    let t0 = Instant::now();
    let index = Parser::build(Arc::clone(&layer));
    let index_build = t0.elapsed();
    let lemma = morphy();
    eprintln!("BASELINE: layer build {build:?}, index build {index_build:?}");

    eprintln!(
        "{:<28} {:>8} {:>6} {:>12}",
        "sentence", "forest", "props", "parse"
    );
    let mut sizes: Vec<usize> = Vec::new();
    for &s in BATTERY {
        let t = Instant::now();
        let forest = index.parse(s, &lemma);
        let dt = t.elapsed();
        sizes.push(forest.len());
        eprintln!(
            "{s:<28} {:>8} {:>6} {dt:>12.2?}",
            forest.len(),
            props(&layer, &forest)
        );
    }
    sizes.sort_unstable();
    eprintln!(
        "forest sizes: min {} median {} max {}",
        sizes.first().copied().unwrap_or(0),
        sizes[sizes.len() / 2],
        sizes.last().copied().unwrap_or(0),
    );
}
