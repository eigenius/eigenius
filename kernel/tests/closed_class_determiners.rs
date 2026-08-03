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

//! D63 §8.3 — the COMMITTED closed-class determiner layer, end to end. The
//! determiners (`every`/`some`/`no`/`a`, subject + object) come from the
//! bootstrapped `ontologies/lexicon/closed-class.esl` — chain data, not test
//! fixtures — and compose with the demo domain (`experiments/lexicon`) through
//! the lookup bridge into kernel-checked propositions.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{
    abbreviation_resources, apply, coordinate_np, coordinate_prop, entry_to_item,
    extract_abbreviations, glossary_resources, ground_long_form, is_ctor, pretty_term, type_raise,
    AbbrDef, AbbreviationBinding, Candidate, DocumentPipeline, Identity, InProcessPipeline, Item,
    LexicalIndex, LexicalLookup, NoAbbreviationProposer, Parser, ProposeCtx, Proposer, SenseRanker,
    SentenceEncoding, SentenceOutcome, WordSenses,
};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::Iri;

const DEMO: &str = include_str!("../../experiments/lexicon/lexicon.esl");

/// Bootstrap (which includes the lexicon schema + `closed-class` determiners),
/// then layer the demo domain (Gene/CellLine, `affects`, `primary`, HeLa, …) on
/// top — so the index sees the committed determiners *and* the demo content.
fn index_over_bootstrap() -> (Arc<Layer>, Parser) {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources =
        esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles on bootstrap");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add demo resource");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let index = Parser::build(Arc::clone(&layer));
    (layer, index)
}

/// gap #5 — the LINKING (copular) verb parse mechanism (importer frames 6/7): a `remained` entry with
/// the EXACT category the importer now emits — `(S[dcl,fin]\NP)/(S[dcl,adj]\NP)`, an opaque
/// `(Entity→Prop)→Entity→Prop` relation — consumes a predicative adjective (`primary`) and yields a
/// finite VP, so `HeLa remained primary` parses to `remain_test(λx.primary(x), hela)`. Faithful: the
/// property is the verb's ARGUMENT (kept opaque for veridical `remain` and evidential `seem` alike),
/// not asserted as the copula's vacuous `λP.P` would. Validates the emitted shape without a reseed.
#[test]
fn linking_verb_takes_a_predicative_adjective() {
    const LINKING_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
axiom lexicon:remain_test : (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop
resource lexicon:remained_e : lexicon:LexicalEntry {
    lexicon:form     = "remained";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any))) );
    lexicon:sem      = lexicon:remain_test;
    lexicon:sem_type = type_expr( (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop );
    lexicon:sense    = "remain";
    lexicon:grade    = epistemic:declared;
}
"#;
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix =
        esl::compile_against_layer(LINKING_FIXTURE, &demo_layer).expect("linking fixture compiles");
    let mut b2 = LayerBuilder::new("linking", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add linking");
    }
    let index = Parser::build(Arc::new(b2.build(LayerStorage::in_memory())));

    let closed = index.parse("HeLa remained primary", &Identity);
    assert!(
        !closed.is_empty(),
        "a linking verb + predicative adjective must parse"
    );
    assert!(
        closed
            .iter()
            .any(|p| sem_mentions_axiom(p.sem(), "urn:eigenius:lexicon:remain_test")),
        "the sem is the opaque linking relation over the property + subject; got {:?}",
        closed
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );
}

/// Two senses of a synthetic word `zob`, ranked: rank-0 is a **plural** NP (disagrees with the
/// 3sg verb `affects`), rank-1 is a **singular** NP (agrees). With `sense_cap(1)` only rank-0
/// seeds, so a sentence needs widen-on-failure to reach rank-1. (Both gate like a proper name.)
const ZOB_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
resource lexicon:zob_pl : lexicon:LexicalEntry {
    lexicon:form       = "zob";
    lexicon:cat        = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:pl) );
    lexicon:sem        = lexicon:brca1;
    lexicon:sem_type   = type_expr( lexicon:Gene );
    lexicon:sense      = "zob.0";
    lexicon:sense_rank = 0;
    lexicon:grade      = epistemic:declared;
}
resource lexicon:zob_sg : lexicon:LexicalEntry {
    lexicon:form       = "zob";
    lexicon:cat        = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:sg) );
    lexicon:sem        = lexicon:brca1;
    lexicon:sem_type   = type_expr( lexicon:Gene );
    lexicon:sense      = "zob.1";
    lexicon:sense_rank = 1;
    lexicon:grade      = epistemic:declared;
}
"#;

/// Bootstrap + demo + the two-sense `zob` fixture, with an index carrying `sense_cap`.
fn zob_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix = esl::compile_against_layer(ZOB_FIXTURE, &demo_layer).expect("zob fixture compiles");
    let mut b2 = LayerBuilder::new("zob", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add zob");
    }
    Arc::new(b2.build(LayerStorage::in_memory()))
}

fn index_with_zob(cap: usize) -> Parser {
    Parser::build(zob_layer()).with_sense_cap(cap)
}

/// Like `zob` but with the polarities flipped: rank-0 is the **singular** (agreeing, parsing) sense,
/// rank-1 the **plural** (disagreeing, failing) one. So at `sense_cap(1)` the *static* order keeps the
/// PARSING sense (no widen needed) — which lets a mock reranker that prefers the failing rank-1 sense
/// genuinely diverge from static, forcing the failure that widen-on-failure must then recover.
const ZWORP_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
resource lexicon:zworp_sg : lexicon:LexicalEntry {
    lexicon:form       = "zworp";
    lexicon:cat        = type_expr( lexicon:cat_np(lexicon:CellLine, lexicon:sg) );
    lexicon:sem        = lexicon:hela;
    lexicon:sem_type   = type_expr( lexicon:CellLine );
    lexicon:sense      = "zworp.0";
    lexicon:sense_rank = 0;
    lexicon:grade      = epistemic:declared;
}
resource lexicon:zworp_pl : lexicon:LexicalEntry {
    lexicon:form       = "zworp";
    lexicon:cat        = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:pl) );
    lexicon:sem        = lexicon:brca1;
    lexicon:sem_type   = type_expr( lexicon:Gene );
    lexicon:sense      = "zworp.1";
    lexicon:sense_rank = 1;
    lexicon:grade      = epistemic:declared;
}
"#;

fn zworp_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix =
        esl::compile_against_layer(ZWORP_FIXTURE, &demo_layer).expect("zworp fixture compiles");
    let mut b2 = LayerBuilder::new("zworp", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add zworp");
    }
    Arc::new(b2.build(LayerStorage::in_memory()))
}

#[test]
fn sense_cap_widens_on_failure_for_known_vocabulary() {
    // Adaptive supertagging (GH #97): `sense_cap(1)` seeds only `zob`'s rank-0 (plural) sense, so
    // "zob affects HeLa" fails number agreement (pl subject + 3sg verb). Since every token is
    // known (not OOV), widen-on-failure doubles the cap, admits the rank-1 (singular) sense, and
    // the sentence parses — the cap never loses a parse a known-vocabulary sentence would get.
    let index = index_with_zob(1);
    assert_eq!(
        index.parse("zob affects HeLa", &Identity).len(),
        1,
        "widen-on-failure recovers the cap-dropped rank-1 sense"
    );

    // OOV guard: an unknown word must NOT trigger widening (widening can't supply a missing
    // lexeme) — it fails closed promptly.
    assert!(
        index_with_zob(1)
            .parse("zob affects zzzqnotaword", &Identity)
            .is_empty(),
        "an OOV token fails closed without pointless widening"
    );
}

/// GH#97 / D64 — **widen-on-failure overrides a mis-ranking reranker** ("a bad rank costs a re-parse,
/// never a missed parse" — the proposer-behind-oracle guarantee that makes the untrusted LLM reranker
/// safe). `zworp`'s static order keeps the agreeing **singular** sense at `sense_cap(1)`, so a plain
/// cap parses with no widen. A mock ranker that prefers the **failing plural** sense (`zworp.1`)
/// diverges from static: the cap now seeds only the disagreeing sense, "zworp affects HeLa" cannot
/// parse at the initial cap — and widen-on-failure doubles the cap, admits the singular sense, and
/// recovers the parse. The reranker can reorder but cannot starve a needed sense.
#[test]
fn widen_on_failure_overrides_a_misranking_reranker() {
    // Control: static cap(1) keeps the agreeing singular (rank-0) sense — parses without widening.
    assert_eq!(
        Parser::build(zworp_layer())
            .with_sense_cap(1)
            .parse("zworp affects HeLa", &Identity)
            .len(),
        1,
        "static cap(1) keeps the agreeing sense"
    );
    // Mis-ranking reranker prefers the FAILING plural sense → the cap seeds only it → the parse can
    // only succeed via widen-on-failure admitting the singular sense.
    assert_eq!(
        Parser::build(zworp_layer())
            .with_sense_cap(1)
            .with_sense_ranker(Box::new(PreferSense("zworp.1")))
            .parse("zworp affects HeLa", &Identity)
            .len(),
        1,
        "widen-on-failure recovers the parse despite the reranker preferring the failing sense"
    );
}

/// A deterministic mock [`SenseRanker`] that ranks one target sense **LAST** for every word (others
/// keep seed order) — the adversarial "the reranker buries the needed sense" case (mirror of
/// [`PreferSense`]).
struct BurySense(&'static str);
impl SenseRanker for BurySense {
    fn rank(&self, _sentence: &str, _context: &str, words: &[WordSenses]) -> Vec<Vec<usize>> {
        words
            .iter()
            .map(|w| {
                let mut idx: Vec<usize> = (0..w.candidates.len()).collect();
                idx.sort_by_key(|&i| w.candidates[i].sense == self.0); // target (true) sorts LAST
                idx
            })
            .collect()
    }
}

/// A layer where the form `zib` has **17 senses**: ONE parsing sense (a singular `CellLine`, the only
/// one that agrees with the 3sg verb `affects`) at static rank 0, and 16 non-parsing distractors
/// (plural `Gene`, failing number agreement) at ranks 1–16. A reranker that buries the parsing sense
/// pushes it to position 16 — **beyond** the sense-cap widen ceiling (`SENSE_CAP_WIDEN_MAX = 16`, whose
/// top-16 is positions 0–15) — so cap-widening WITHIN the reranked order can never re-admit it.
fn zib_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let mut fixture = String::from(
        "namespace lexicon   = \"urn:eigenius:lexicon\";\n\
         namespace epistemic = \"urn:eigenius:reflection:epistemic\";\n\
         resource lexicon:zib_needed : lexicon:LexicalEntry {\n\
             lexicon:form       = \"zib\";\n\
             lexicon:cat        = type_expr( lexicon:cat_np(lexicon:CellLine, lexicon:sg) );\n\
             lexicon:sem        = lexicon:hela;\n\
             lexicon:sem_type   = type_expr( lexicon:CellLine );\n\
             lexicon:sense      = \"zib.needed\";\n\
             lexicon:sense_rank = 0;\n\
             lexicon:grade      = epistemic:declared;\n\
         }\n",
    );
    for i in 1..=16 {
        fixture.push_str(&format!(
            "resource lexicon:zib_d{i} : lexicon:LexicalEntry {{\n\
                 lexicon:form       = \"zib\";\n\
                 lexicon:cat        = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:pl) );\n\
                 lexicon:sem        = lexicon:brca1;\n\
                 lexicon:sem_type   = type_expr( lexicon:Gene );\n\
                 lexicon:sense      = \"zib.d{i}\";\n\
                 lexicon:sense_rank = {i};\n\
                 lexicon:grade      = epistemic:declared;\n\
             }}\n"
        ));
    }
    let fix = esl::compile_against_layer(&fixture, &demo_layer).expect("zib fixture compiles");
    let mut b2 = LayerBuilder::new("zib", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add zib");
    }
    Arc::new(b2.build(LayerStorage::in_memory()))
}

/// GH#97 / this session — the **static-rank widen fallback**.
/// [`widen_on_failure_overrides_a_misranking_reranker`] covers the case cap-widening CAN recover (few
/// senses → doubling the cap admits the needed one). This covers the case it CANNOT: a lemma with more
/// senses than the widen ceiling, where the reranker buries the only parsing sense beyond it. Cap
/// escalation within the reranked order stays gapped; a second pass under **static** rank recovers the
/// parse — extending "a bad rank costs a re-parse, never a missed parse" to the whole widen half.
#[test]
fn static_rank_fallback_recovers_a_sense_the_reranker_buried_beyond_widen_max() {
    // Control: static order keeps the parsing (rank-0) sense — parses without any reranker.
    assert_eq!(
        Parser::build(zib_layer())
            .with_sense_cap(2)
            .parse("zib affects HeLa", &Identity)
            .len(),
        1,
        "static cap keeps the agreeing sense at rank 0"
    );
    // Adversarial reranker buries the ONLY parsing sense at position 16 — beyond the cap-widen ceiling
    // (top-16 = positions 0–15). Cap escalation in the reranked order can never re-admit it; the
    // static-rank fallback re-parses under static order and recovers.
    assert_eq!(
        Parser::build(zib_layer())
            .with_sense_cap(2)
            .with_sense_ranker(Box::new(BurySense("zib.needed")))
            .parse("zib affects HeLa", &Identity)
            .len(),
        1,
        "static-rank fallback recovers the parse the reranker buried beyond widen-max"
    );
}

/// Two **both-valid** singular senses of a synthetic word `zarg`: rank-0 → the BRCA1 gene, rank-1
/// → the HeLa cell line. Each makes "zarg affects HeLa" a felicitous `Prop` (both `Entity`-typed
/// subjects), so — unlike `zob` (where rank-0 fails agreement and widen recovers rank-1) — the
/// cap is the *only* thing deciding which survives. That isolates the reranker: with `sense_cap(1)`
/// the static cap keeps rank-0 (BRCA1); a reranker preferring `zarg.1` makes the cap keep HeLa.
const ZARG_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
resource lexicon:zarg_gene : lexicon:LexicalEntry {
    lexicon:form       = "zarg";
    lexicon:cat        = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:sg) );
    lexicon:sem        = lexicon:brca1;
    lexicon:sem_type   = type_expr( lexicon:Gene );
    lexicon:sense      = "zarg.0";
    lexicon:sense_rank = 0;
    lexicon:grade      = epistemic:declared;
}
resource lexicon:zarg_cell : lexicon:LexicalEntry {
    lexicon:form       = "zarg";
    lexicon:cat        = type_expr( lexicon:cat_np(lexicon:CellLine, lexicon:sg) );
    lexicon:sem        = lexicon:hela;
    lexicon:sem_type   = type_expr( lexicon:CellLine );
    lexicon:sense      = "zarg.1";
    lexicon:sense_rank = 1;
    lexicon:grade      = epistemic:declared;
}
"#;

/// Bootstrap + demo + the two-sense `zarg` fixture committed as a layer chain.
fn zarg_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix = esl::compile_against_layer(ZARG_FIXTURE, &demo_layer).expect("zarg fixture compiles");
    let mut b2 = LayerBuilder::new("zarg", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add zarg");
    }
    Arc::new(b2.build(LayerStorage::in_memory()))
}

/// The `zarg` index with `sense_cap` and an optional reranker.
fn index_with_zarg(cap: usize, ranker: Option<Box<dyn SenseRanker + Send + Sync>>) -> Parser {
    let mut index = Parser::build(zarg_layer()).with_sense_cap(cap);
    if let Some(r) = ranker {
        index = index.with_sense_ranker(r);
    }
    index
}

/// A deterministic mock [`SenseRanker`] that ranks one target sense first for every word
/// (others keep seed order) — the CI stand-in for "the context prefers this sense".
struct PreferSense(&'static str);
impl SenseRanker for PreferSense {
    fn rank(&self, _sentence: &str, _context: &str, words: &[WordSenses]) -> Vec<Vec<usize>> {
        words
            .iter()
            .map(|w| {
                let mut idx: Vec<usize> = (0..w.candidates.len()).collect();
                idx.sort_by_key(|&i| w.candidates[i].sense != self.0); // target (false) sorts first
                idx
            })
            .collect()
    }
}

#[test]
fn cell_beam_bounds_a_cell_and_is_a_noop_when_generous() {
    // Lever B (per-cell beam, GH #97). Both senses of `zarg` are valid singular subjects, so
    // "zarg affects HeLa" has TWO full parses unbeamed (affects(HeLa, BRCA1) and affects(HeLa,
    // HeLa)). No `sense_cap` here — the beam is the only lever, acting on the leaf cell.
    // The cell beam is an UNPACKED-path tuning knob (the packed path bounds by cube pruning instead,
    // not the per-cell beam — see `parse_packed`), so pin the unpacked path to exercise it (B9 made
    // packing the default). This is path-specific tuning, not a correctness contract like widen.
    let unbeamed = Parser::build(zarg_layer()).with_packing(false);
    assert_eq!(
        unbeamed.parse("zarg affects HeLa", &Identity).len(),
        2,
        "both senses parse with no beam"
    );

    // A generous beam is a no-op — both readings survive.
    let generous = Parser::build(zarg_layer())
        .with_packing(false)
        .with_cell_beam(16);
    assert_eq!(
        generous.parse("zarg affects HeLa", &Identity).len(),
        2,
        "a generous cell beam keeps both parses (no-op)"
    );

    // A tight beam (2) drops the higher-`Cost` (sr1 → HeLa) sense at the leaf, keeping only the
    // cheaper (sr0 → BRCA1) reading: the beam prunes by Cost and bounds the cell.
    let tight = Parser::build(zarg_layer())
        .with_packing(false)
        .with_cell_beam(2);
    let forest = tight.parse("zarg affects HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "a tight cell beam drops the costlier sense"
    );
    let sem = format!("{:?}", forest[0].sem());
    assert!(
        sem.contains("brca1"),
        "the surviving (cheaper, sr0) reading is the BRCA1 sense: {sem}"
    );
}

// ── D62 Phase 3 — transparent `-ly` adverbs ───────────────────────────
#[test]
fn transparent_ly_adverb_is_recognized_and_does_not_change_the_claim() {
    // The `-ly` derivational rule recognizes an adverb when its adjective base is known to the
    // lexicon (data-driven), seeds transparent modifier items, and the identity sem leaves the
    // claim unchanged. Bases here are demo adjectives: `primarily`←`primary`, `largely`←`large`.
    let (_layer, index) = index_over_bootstrap();

    // Adjective modifier: "primarily" modifies the predicative adjective "primary".
    let base_adj = index.parse("HeLa is primary", &Identity);
    assert_eq!(base_adj.len(), 1, "baseline copular adjective parses once");
    let mod_adj = index.parse("HeLa is primarily primary", &Identity);
    assert!(!mod_adj.is_empty(), "the adverb-modified adjective parses");
    assert_eq!(
        pretty_term(mod_adj[0].sem()),
        pretty_term(base_adj[0].sem()),
        "the `-ly` adverb is transparent — same claim as unmodified"
    );

    // VP modifier (forward): "largely" modifies the VP "affects BRCA1".
    let base_vp = index.parse("HeLa affects BRCA1", &Identity);
    assert_eq!(base_vp.len(), 1, "baseline transitive clause parses once");
    let mod_vp = index.parse("HeLa largely affects BRCA1", &Identity);
    assert!(!mod_vp.is_empty(), "the adverb-modified VP parses");
    assert_eq!(
        pretty_term(mod_vp[0].sem()),
        pretty_term(base_vp[0].sem()),
        "the `-ly` VP adverb is transparent — same claim as unmodified"
    );
}

#[test]
fn ly_adverb_recognition_requires_a_known_adjective_base() {
    // Data-driven gate: an `-ly` token whose base is NOT a known adjective is not an adverb, so the
    // sentence has no parse (the unrecognized token leaves a gap) — confirms recognition isn't a
    // blind `-ly` strip.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is zorply primary", &Identity).is_empty(),
        "an `-ly` token with no adjective base does not seed an adverb"
    );
}

#[test]
fn derived_adjective_reuses_its_base_and_is_transparent() {
    // D63 compound morphology §3, Slice 1: a closed-prefix concatenation (`hyperprimary` ← `primary`)
    // and a right-headed hyphen compound (`double-primary` ← `primary`) each seed the base
    // adjective's own items on the whole-token span, so the derived word parses identically to its
    // base — the affix / left modifier is transparent in v1 (identity sem, like the `-ly` adverbs).
    let (_layer, index) = index_over_bootstrap();

    let base = index.parse("HeLa is primary", &Identity);
    assert_eq!(base.len(), 1, "baseline copular adjective parses once");

    // Concatenated closed prefix `hyper-`.
    let prefixed = index.parse("HeLa is hyperprimary", &Identity);
    assert!(!prefixed.is_empty(), "the prefixed adjective parses");
    assert_eq!(
        pretty_term(prefixed[0].sem()),
        pretty_term(base[0].sem()),
        "`hyperprimary` is transparent — same claim as `primary`"
    );

    // Right-headed hyphen compound.
    let compound = index.parse("HeLa is double-primary", &Identity);
    assert!(!compound.is_empty(), "the hyphen compound adjective parses");
    assert_eq!(
        pretty_term(compound[0].sem()),
        pretty_term(base[0].sem()),
        "`double-primary` is transparent — same claim as `primary`"
    );

    // Attributive (prenominal) use — the real target (`hypermutable cells`): the derived adjective
    // refines the noun through the existing `RefineKind::Attrib` rule.
    assert!(
        !index
            .parse("HeLa is a hyperprimary gene", &Identity)
            .is_empty(),
        "the derived adjective modifies a noun attributively"
    );
}

#[test]
fn derived_adjective_recognition_requires_a_known_base() {
    // Data-driven gate (mirrors the `-ly` gate): a prefix / hyphen compound whose base is NOT a known
    // adjective is not seeded, so the sentence has no parse — recognition is not a blind strip/split.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is hyperzorp", &Identity).is_empty(),
        "a prefix token with no adjective base does not seed a derived adjective"
    );
    assert!(
        index.parse("HeLa is double-zorp", &Identity).is_empty(),
        "a hyphen compound with no adjective head does not seed a derived adjective"
    );
}

/// Synthetic relations for the denominal-suffix rule (D63 compound morphology §3b): a transitive `base`
/// and `resemble` (each `Entity → Entity → Prop`), plus a 1-place `like` **adjective** — so a `-like`
/// token could wrongly take the Slice-1 identity reading if the over-generation fix regressed.
const DENOMINAL_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
axiom lexicon:base_rel : lexicon:Entity -> lexicon:Entity -> Prop
resource lexicon:e_base : lexicon:LexicalEntry {
    lexicon:form     = "base";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:base_rel;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wn:base.v.01";
    lexicon:grade    = epistemic:declared;
}
axiom lexicon:resemble_rel : lexicon:Entity -> lexicon:Entity -> Prop
resource lexicon:e_resemble : lexicon:LexicalEntry {
    lexicon:form     = "resemble";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:resemble_rel;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wn:resemble.v.01";
    lexicon:grade    = epistemic:declared;
}
axiom lexicon:like_adj : lexicon:Entity -> Prop
resource lexicon:e_like : lexicon:LexicalEntry {
    lexicon:form     = "like";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:like_adj;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "wn:like.a.01";
    lexicon:grade    = epistemic:declared;
}
"#;

fn denominal_index() -> Parser {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix = esl::compile_against_layer(DENOMINAL_FIXTURE, &demo_layer)
        .expect("denominal fixture compiles");
    let mut b2 = LayerBuilder::new("denominal", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add denominal relation");
    }
    Parser::build(Arc::new(b2.build(LayerStorage::in_memory())))
}

#[test]
fn denominal_x_based_adjective_predicates_via_the_base_axiom() {
    // D63 compound morphology §3, Slice 2 (`X-based` → `base(x, X)`): `gene-based` (X = the demo
    // `gene` noun) seeds a predicative adjective `S[adj]\NP` with sem `λx. base_rel(x, kind_of(Gene))`
    // over the `base` verb's OWN axiom — not a minted `based_on` (§2a).
    let index = denominal_index();

    // Predicative: `HeLa is gene-based` → base_rel(hela, kind_of(Gene)).
    let pred = index.parse("HeLa is gene-based", &Identity);
    assert!(
        !pred.is_empty(),
        "`gene-based` predicative adjective parses"
    );
    assert!(
        pred.iter().any(|it| {
            let s = pretty_term(it.sem());
            s.contains("base_rel") && s.contains("kind_of")
        }),
        "the sem reuses the `base` axiom over the noun's kind — got {}",
        pretty_term(pred[0].sem())
    );

    // Attributive (the real use — `pcr-based method`): the derived adjective modifies a noun.
    assert!(
        !index
            .parse("HeLa is a gene-based gene", &Identity)
            .is_empty(),
        "`gene-based` modifies a noun attributively"
    );

    // Gate: X must be a known noun — `zorp-based` (zorp unknown) is not seeded.
    assert!(
        index.parse("HeLa is zorp-based", &Identity).is_empty(),
        "`X-based` with an unknown X-noun is not seeded"
    );
}

#[test]
fn denominal_like_routes_to_the_verb_relation_and_does_not_drop_x() {
    // D63 compound morphology §3b: `-like` is an ADJECTIVE-voice denominal suffix, so it routes to the
    // 2-place VERB `resemble` (the 1-place adjective `like` is not a relation) with the subject-voice
    // arg order `resemble(kind_of(X), θ)`. And because `like` is a WordNet adjective, the Slice-1
    // hyphen-head identity rule must NOT fire on `gene-like` (which would drop `X` → `like(hela)`).
    let index = denominal_index();

    let pred = index.parse("HeLa is gene-like", &Identity);
    assert!(!pred.is_empty(), "`gene-like` predicative adjective parses");
    // Adjective-voice order: X (`kind_of(Gene)`) is the FIRST argument of `resemble_rel`.
    assert!(
        pred.iter()
            .any(|it| pretty_term(it.sem()).contains("resemble_rel(kind_of")),
        "`gene-like` → resemble_rel(kind_of(Gene), θ) via the verb relation — got {}",
        pretty_term(pred[0].sem())
    );
    // Over-generation guard: EVERY reading references `kind_of` — X is never dropped (no Slice-1
    // `like_adj(hela)` identity leak).
    assert!(
        pred.iter()
            .all(|it| pretty_term(it.sem()).contains("kind_of")),
        "no reading drops X to the bare `like` adjective identity"
    );
}

// ── D62 connectives batch — plural demonstratives, prepositions, discourse adverbs ──
#[test]
fn connectives_batch_parses() {
    let (_layer, index) = index_over_bootstrap();

    // Plural demonstratives `these`/`those` (plural noun via PluralS; mirror `all`).
    assert!(
        !index.parse("these genes affect HeLa", &PluralS).is_empty(),
        "`these` + plural noun + plural verb parses"
    );
    assert!(
        !index.parse("those genes affect HeLa", &PluralS).is_empty(),
        "`those` + plural noun + plural verb parses"
    );

    // Prepositions `between`/`within` (VP-adjunct, mirror `in`).
    assert!(
        !index
            .parse("HeLa affects a cell line within HeLa", &Identity)
            .is_empty(),
        "`within` PP-adjunct parses"
    );
    assert!(
        !index
            .parse("HeLa affects a cell line between HeLa", &Identity)
            .is_empty(),
        "`between` PP-adjunct parses"
    );

    // Discourse adverbs `also`/`however`/`yet` — transparent (same claim as unmodified).
    let base = index.parse("HeLa affects BRCA1", &Identity);
    assert_eq!(base.len(), 1, "baseline parses once");
    for sent in [
        "HeLa also affects BRCA1",    // VP-medial modifier
        "however HeLa affects BRCA1", // sentence-initial S/S
        "HeLa affects BRCA1 yet",     // sentence-final S\S
    ] {
        let f = index.parse(sent, &Identity);
        assert!(!f.is_empty(), "`{sent}` parses");
        assert_eq!(
            pretty_term(f[0].sem()),
            pretty_term(base[0].sem()),
            "discourse adverb is transparent (same claim): `{sent}`"
        );
    }
}

#[test]
fn quantified_np_as_preposition_object_parses() {
    // D62 §2 GQ-as-prep-object: a quantified/bare-plural NP scopes into a `cat_pp/NP`
    // preposition's object slot ("a cell line within a gene"), mirroring the verb-object
    // raise (`a_obj`) but parser-side and polymorphic in the functor. Before this, only a
    // bare NAME could be a preposition object (`within HeLa` ✓, `within a gene` ✗).
    let (_layer, index) = index_over_bootstrap();

    // Baseline (name object) — the post-nominal PP refine over a plain `cat_np`.
    assert!(
        !index
            .parse("a cell line within HeLa affects BRCA1", &Identity)
            .is_empty(),
        "name as preposition object parses (baseline)"
    );

    // Singular existential GQ as a preposition object — the gap. Closed parse.
    assert!(
        !index
            .parse("a cell line within a gene affects BRCA1", &Identity)
            .is_empty(),
        "`within a gene` (existential GQ object) parses"
    );

    // Bare-plural as a preposition object — commits to its kind (reshape §7.4) ⇒ a CLOSED parse.
    assert!(
        !index
            .parse("a cell line within genes affects BRCA1", &PluralS)
            .is_empty(),
        "`within genes` (bare-plural kind object) yields a closed parse"
    );
}

#[test]
fn already_covered_constructions_are_derived() {
    // Measurement-first pass (D62 close-out, 2026-06-29): several §2 "gaps" in
    // `docs/notes/d62-grammar-gap-analysis.md` are in fact ALREADY COVERED — proven here on the
    // small lexicon, converting them from Declared to Derived and locking them as regression gates.
    let (_layer, index) = index_over_bootstrap();
    let closes = |s: &str| {
        assert!(
            !index.parse(s, &Identity).is_empty(),
            "expected a closed parse (already-covered construction): {s:?}"
        );
    };

    // #6 — predicate/VP coordination, ALL three shapes (the inventory listed coordinated predicates
    // as a gap; only basic `is a NP` was assumed working).
    closes("HeLa is a gene and a cell line"); //          coordinated NP-predicate
    closes("HeLa affects BRCA1 and affects BRCA1"); //    coordinated VP, same clause feature
    closes("HeLa is primary and affects BRCA1"); //       CROSS-feature VP (adj-pred + verbal) — works
    closes("HeLa affects BRCA1 and is primary");

    // #7 — passive, BOTH the by-agent (long) and agentless (short) forms.
    closes("BRCA1 is affected by HeLa"); //               long passive with by-agent
    closes("BRCA1 is affected"); //                       agentless passive

    // #3 — coordination at OBJECT position (binary + comma list); subject lists are covered by
    // `comma_list_coordination_parses`.
    closes("HeLa affects BRCA1 and BRCA1");
    closes("HeLa affects BRCA1, BRCA1 and BRCA1");

    // Restrictive relative (the covered baseline; the NON-restrictive comma variant is the real gap).
    closes("a gene which affects HeLa is large");
}

#[test]
fn coordination_unpacked_via_list_completion() {
    // Exercise the list-with-operator model on the UNPACKED path directly (with_packing(false)), so
    // plain `and`/`or` binary + n-ary coordination is validated through `coordinate_prop` +
    // `complete_coord` (not just the comma finalizer). D63 §8.4 Phase 3 refactor.
    let (layer, _) = index_over_bootstrap();
    let index = Parser::build(Arc::clone(&layer)).with_packing(false);
    let sem = |s: &str| {
        let ps = index.parse(s, &Identity);
        assert!(!ps.is_empty(), "expected an unpacked parse: {s:?}");
        pretty_term(ps[0].sem())
    };
    // VP / predicate / clause coordination, binary — folds via the completion.
    assert!(
        sem("HeLa affects BRCA1 and affects BRCA1").starts_with("And("),
        "binary VP coordination folds ∧"
    );
    assert!(
        sem("HeLa is a gene and a cell line").starts_with("And("),
        "binary predicate coordination folds ∧"
    );
    assert!(
        sem("HeLa affects BRCA1 or affects BRCA1").starts_with("Or("),
        "binary VP coordination folds ∨"
    );
    // N-ary (no comma) — left-branching fold, and a SINGLE parse (the NF holds through the list model).
    assert!(
        sem("HeLa affects BRCA1 and affects BRCA1 and affects BRCA1").starts_with("And(And("),
        "n-ary VP coordination folds left-branching"
    );
    assert_eq!(
        index
            .parse(
                "HeLa affects BRCA1 and affects BRCA1 and affects BRCA1",
                &Identity
            )
            .len(),
        1,
        "n-ary coordination has a single (left-branching) parse under the list model"
    );
}

#[test]
fn comma_list_inherits_the_final_connective() {
    // D63 §8.4 Phase 6, Step 5b — a list comma is polarity-NEUTRAL: it inherits the list's FINAL
    // explicit connective (`A, B, C or D` = all-`∨`, `A, B, C and D` = all-`∧`), NOT the hardcoded
    // `and`. Verified on BOTH coordination paths: the NP-group path (`coordinate_np` conn_list +
    // rebind) and the prop-ending path (the n-ary fold in `parse_at_cap`). Before Step 5b the NP-`or`
    // list GAPPED (the same-connective guard) and the prop-`or` list MIS-parsed as `Or(And(a,b),c)`.
    let (_layer, index) = index_over_bootstrap();
    let sem = |s: &str| {
        let ps = index.parse(s, &Identity);
        assert!(!ps.is_empty(), "expected a parse: {s:?}");
        pretty_term(ps[0].sem())
    };
    // NP-group path (object position → distribute_object over the group).
    assert!(
        sem("HeLa affects BRCA1, BRCA1 and BRCA1").starts_with("And(And("),
        "comma-AND NP list folds all-∧"
    );
    let np_or = sem("HeLa affects BRCA1, BRCA1 or BRCA1");
    assert!(
        np_or.starts_with("Or(Or(") && !np_or.contains("And"),
        "comma-OR NP list folds all-∨ (was a GAP before Step 5b): {np_or}"
    );
    // Prop-ending path (coordinated predicative NPs after "is").
    let pred_or = sem("HeLa is a gene, a cell line or a gene");
    assert!(
        pred_or.starts_with("Or(Or(") && !pred_or.contains("And"),
        "comma-OR predicate list folds all-∨, NOT Or(And(a,b),c): {pred_or}"
    );
    assert!(
        sem("HeLa is a gene, a cell line and a gene").starts_with("And(And("),
        "comma-AND predicate list folds all-∧"
    );
    // Prop-ending path (coordinated VPs).
    let vp_or = sem("HeLa affects BRCA1, affects BRCA1 or affects BRCA1");
    assert!(
        vp_or.starts_with("Or(Or(") && !vp_or.contains("And"),
        "comma-OR VP list folds all-∨: {vp_or}"
    );
}

#[test]
fn close_apposition_subject_and_object() {
    // D63 §8.4 Phase 6, RC-6 — close nominal apposition: a definite/bare common-noun HEAD + a
    // coreferential name-group ("the genes BRCA1 and MSH2"). `appose_group` passes the group through
    // (gated on the members being of the head's base kind), so it rides the distributive-subject /
    // -object machinery. Both syntactic positions must close.
    let (_layer, index) = index_over_bootstrap();
    let closes = |s: &str| {
        assert!(
            !index.parse(s, &PluralS).is_empty(),
            "expected closed: {s:?}"
        )
    };
    let gaps = |s: &str| {
        assert!(
            index.parse(s, &PluralS).is_empty(),
            "expected NO parse (felicity reject): {s:?}"
        )
    };

    // Baselines that must already close (isolate the apposition from its parts).
    closes("the genes affect HeLa"); //          GQ subject
    closes("BRCA1 and BRCA1 affect HeLa"); //     bare group subject (distribute)
    closes("HeLa affects BRCA1 and BRCA1"); //    bare group object (distribute_object)

    // Apposition — SUBJECT and OBJECT position.
    closes("HeLa affects the genes BRCA1 and BRCA1"); // OBJECT apposition
    closes("the genes BRCA1 and BRCA1 affect HeLa"); //  SUBJECT apposition

    // Felicity reject: a gene-typed group does not appose a cell-typed head.
    gaps("the cell lines BRCA1 and BRCA1 affect HeLa");
}

/// A cross-IMPORTER granularity fixture reproducing the real-lexicon typing the *reflexive* felicity
/// gate missed (d63-parse-gap-closure §4 Step 5). On the full WordNet+UMLS lexicon a NAMED individual
/// carries its BROAD UMLS semantic type (`umlssty:T028` "Gene or Genome"), while the common NOUN
/// carries its NARROWER concept (`umlscui:C0017337` "gene", emitted `: umlssty:T028`). Here `WidgetKind`
/// mirrors the semantic type and `WidgetConcept : WidgetKind` the concept; the name "Wob" is typed at
/// the KIND, the noun "widget" at the CONCEPT. Close apposition must bridge the two via BIDIRECTIONAL
/// subsumption (`WidgetConcept ≤ WidgetKind`), which the one-directional `group ≤ head` gate could not.
const WIDGET_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
namespace demo      = "urn:eigenius:demo";
class demo:WidgetKind : lexicon:Entity { }
class demo:WidgetConcept : demo:WidgetKind { }
resource demo:wob : demo:WidgetKind { }
resource demo:bit : demo:WidgetKind { }
resource lexicon:e_widget : lexicon:LexicalEntry {
    lexicon:form     = "widget";
    lexicon:cat      = type_expr( lexicon:cat_n(demo:WidgetConcept, lexicon:num_any) );
    lexicon:sem      = demo:WidgetConcept;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "demo:widget";
    lexicon:grade    = epistemic:declared;
}
resource lexicon:e_wob : lexicon:LexicalEntry {
    lexicon:form     = "Wob";
    lexicon:cat      = type_expr( lexicon:cat_np(demo:WidgetKind, lexicon:sg) );
    lexicon:sem      = demo:wob;
    lexicon:sem_type = type_expr( demo:WidgetKind );
    lexicon:sense    = "demo:wob";
    lexicon:grade    = epistemic:declared;
}
resource lexicon:e_bit : lexicon:LexicalEntry {
    lexicon:form     = "Bit";
    lexicon:cat      = type_expr( lexicon:cat_np(demo:WidgetKind, lexicon:sg) );
    lexicon:sem      = demo:bit;
    lexicon:sem_type = type_expr( demo:WidgetKind );
    lexicon:sense    = "demo:bit";
    lexicon:grade    = epistemic:declared;
}
"#;

fn widget_index() -> Parser {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix =
        esl::compile_against_layer(WIDGET_FIXTURE, &demo_layer).expect("widget fixture compiles");
    let mut b2 = LayerBuilder::new("widget", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add widget");
    }
    Parser::build(Arc::new(b2.build(LayerStorage::in_memory())))
}

#[test]
fn close_apposition_bridges_concept_and_semantic_type_granularity() {
    // The real-lexicon regression: a CONCEPT-typed head noun ("widget" : WidgetConcept) apposed to
    // SEMANTIC-TYPE-typed names ("Wob"/"Bit" : WidgetKind), with WidgetConcept ≤ WidgetKind. The
    // bidirectional felicity gate accepts (the head is a SUBTYPE of the members' type); a one-directional
    // `members ≤ head` gate would wrongly reject, gapping every "the <noun> <NAME> and <NAME>" over the
    // cross-importer lexicon (d63-parse-gap-closure §4 Step 5).
    let index = widget_index();
    assert!(
        !index
            .parse("the widgets Wob and Bit affect HeLa", &PluralS)
            .is_empty(),
        "a concept-typed head must appose semantic-type-typed names (bidirectional felicity)"
    );
    // Kind clash still rejected: a widget-kind group does not appose a gene head (neither subsumes).
    assert!(
        index
            .parse("the genes Wob and Bit affect HeLa", &PluralS)
            .is_empty(),
        "a widget-kind group must not appose a gene-typed head"
    );
}

/// A VP-adjunct preposition `beside` that is **lexicon-tagged** (`in_lexicon = lexicon:extra_lex`) and
/// **sense-ranked** (`sense_rank = 5`) — the two properties the pied-piping rule was throwing away.
/// Layered on the demo so the rest of the sentence (`the gene … HeLa affects BRCA1 … is large`) parses
/// exactly as in `pied_piping_relative_threads_the_antecedent_into_the_fronted_preposition`.
const PIED_PREP_FIXTURE: &str = r#"
namespace core       = "urn:eigenius:core";
namespace lexicon    = "urn:eigenius:lexicon";
namespace logic      = "urn:eigenius:logic";
namespace ontology   = "urn:eigenius:ontology";
namespace epistemic  = "urn:eigenius:reflection:epistemic";
axiom ontology:prep_beside : lexicon:Entity -> lexicon:Entity -> Prop
resource lexicon:extra_lex : lexicon:Lexicon {
    core:description = "a lexicon the parse scope can exclude — the scope-filter probe.";
}
resource lexicon:prep_beside_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (x : lexicon:Entity) => fun (V : lexicon:Entity -> Prop) => fun (s : lexicon:Entity) =>
            logic:And(V(s), ontology:prep_beside(s, x))
          : lexicon:Entity -> (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop) )
    );
}
resource lexicon:beside_prep : lexicon:LexicalEntry {
    core:description = "preposition 'beside' — TAGGED to lexicon:extra_lex and ranked, to probe the pied-piping rule.";
    lexicon:form        = "beside";
    lexicon:cat         = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin_any), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin_any), lexicon:cat_np(lexicon:Entity, lexicon:num_any))), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem         = lexicon:prep_beside_sem;
    lexicon:sem_type    = type_expr( lexicon:Entity -> (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop) );
    lexicon:sense       = "beside";
    lexicon:sense_rank  = 5;
    lexicon:in_lexicon  = lexicon:extra_lex;
    lexicon:grade       = epistemic:declared;
}
"#;

fn parser_with_pied_prep() -> Parser {
    let (demo, _) = index_over_bootstrap();
    let res = esl::compile_against_layer(PIED_PREP_FIXTURE, &demo).expect("fixture compiles");
    let mut b = LayerBuilder::new("pied-prep", Some(Arc::clone(&demo)));
    for r in res {
        b.add_resource(r).expect("add fixture resource");
    }
    Parser::build(Arc::new(b.build(LayerStorage::in_memory())))
}

const PIED_BESIDE: &str = "the gene beside which HeLa affects BRCA1 is large";

/// **The pied-piping rule bypasses the lexicon SCOPE filter** (`chart_unpacked.rs`, the `entries_for`
/// smuggle). Every seeded entry passes through `scoped(entries, scope)` — D65 §4: *a tagged entry whose
/// lexicon is outside the scope is dropped*. Pied-piping instead calls `entries_for` RAW, straight out
/// of the lexicon, so its preposition is admitted no matter what the caller scoped the parse to.
///
/// Witness: `beside` is tagged `in_lexicon = extra_lex`. Parsed under an EMPTY scope, every tagged entry
/// must be dropped (untagged closed-class / demo entries stay — they are always-available). So the
/// sentence must NOT parse.
///
/// **Pinned to the UNPACKED path**, because on the default path the bug is MASKED by a coincidence: the
/// router's pied-piping detector (`parse_needs_unpacked`) finds the fronted preposition via
/// `lookup_span`, which *is* scope-aware — so an out-of-scope preposition makes the router miss the
/// construct entirely and divert the sentence to the packed path, which has no pied-piping rule. The
/// scope survives by accident, not because the rule respects it. `with_packing(false)` is a supported
/// configuration (it is the differential oracle's baseline), and there the rule runs and the smuggle is
/// directly observable.
#[test]
fn pied_piping_respects_the_lexicon_scope() {
    let index = parser_with_pied_prep().with_packing(false);
    // Unscoped: parses (the preposition is available) — the control.
    assert!(
        !index.parse(PIED_BESIDE, &Identity).is_empty(),
        "control: `beside` pied-piping parses when its lexicon is in scope"
    );
    // Scoped to NOTHING: the tagged `beside` is out of scope, so there is no preposition to pied-pipe.
    let scoped = index.parse_scoped(PIED_BESIDE, &Identity, Some(&[]));
    assert!(
        scoped.is_empty(),
        "pied-piping admitted an OUT-OF-SCOPE preposition — the rule reads the lexicon directly and \
         never applies the scope filter (D65 §4). Got: {:?}",
        scoped.iter().map(|it| pretty_term(it.sem())).collect::<Vec<_>>()
    );
}

/// **The pied-piping rule drops the preposition's `Cost`.** Every other rule sums the costs of all its
/// operands; pied-piping builds its result from `noun.cost() + subj.cost() + vp.cost()` and silently
/// omits the preposition — so its `sense_rank` (and its `lexicon_order`, the PRIMARY rank key) never
/// reach the parse. A pied-piping reading therefore looks cheaper than it is, and ranks above readings
/// it should lose to.
///
/// Witness: `beside` carries `sense_rank = 5`. Every other leaf in the sentence is rank 0, so the parse's
/// cost must be ≥ 5. Today it is 0.
#[test]
fn pied_piping_counts_the_prepositions_cost() {
    let index = parser_with_pied_prep();
    let forest = index.parse(PIED_BESIDE, &Identity);
    assert!(!forest.is_empty(), "control: `beside` pied-piping parses");
    let best = forest.iter().map(|it| it.cost().sense_rank).min().unwrap();
    assert!(
        best >= 5,
        "pied-piping dropped the preposition's cost: `beside` has sense_rank=5 and every other leaf is \
         0, so the parse must cost >= 5, got {best}. The rule sums noun+subj+vp and forgets the prep."
    );
}

#[test]
fn pied_piping_relative_threads_the_antecedent_into_the_fronted_preposition() {
    // D62 §2 #2B: pied-piping `[noun] [prep] which [subject VP]` — the antecedent is the FRONTED
    // preposition's object, threaded into the clause as a VP-adjunct: "the gene in which HeLa affects
    // BRCA1" ⇒ Σg:Gene. And(affects(brca1,hela), prep_in(hela, g)). Reuses the VP-adjunct prep sem.
    let (layer, index) = index_over_bootstrap();
    for (s, prep) in [
        ("the gene in which HeLa affects BRCA1 is large", "prep_in"),
        (
            "the gene within which HeLa affects BRCA1 is large",
            "prep_within",
        ),
    ] {
        let forest = index.parse(s, &Identity);
        assert!(!forest.is_empty(), "pied-piping parses: {s}");
        let sem = pretty_term(forest[0].sem());
        assert!(
            sem.contains(prep) && sem.contains("affects"),
            "pied-piping threads the antecedent into the `{prep}` relation: {sem}"
        );
        // Kernel-gated to Prop.
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
        let ty = check_infer(&mut ctx, forest[0].sem()).expect("type-checks");
        assert_eq!(readback_val(0, &ty), Exp::Sort(0), "inhabits Prop: {s}");
    }
    // Restrictive (non-pied) relative still parses — no regression.
    assert!(
        !index
            .parse("a gene which affects HeLa is large", &Identity)
            .is_empty(),
        "plain restrictive relative unaffected"
    );
}

#[test]
fn fronted_participial_adjunct_is_an_open_parse_with_a_controlled_subject() {
    // D62 §2 #5a: a subject-gapped present-participle VP fronted as a sentence adjunct
    // ("affecting BRCA1, HeLa affects BRCA1" — schematically "hypothesizing that P, we Q") asserts
    // the participial proposition alongside the matrix, with the participle's subject CONTROLLED — a
    // referent hole (D64) ⇒ an OPEN parse. The open sem ABSTRACTS the hole (D64 parametric proposition):
    // `λhole. And(matrix, participle(hole))`.
    let (_layer, index) = index_over_bootstrap();
    let (closed, open) = index.parse_open("affecting BRCA1 , HeLa affects BRCA1", &Identity);
    assert!(
        closed.is_empty(),
        "the controlled subject is unresolved ⇒ not a closed parse"
    );
    assert!(!open.is_empty(), "fronted participial yields an open parse");
    let sem = pretty_term(open[0].item.sem());
    assert!(
        sem.starts_with('λ') && sem.contains("And(") && sem.matches("affects").count() == 2,
        "abstracts the controlled subject over a conjunction of matrix + participial proposition: {sem}"
    );
    assert_eq!(
        open[0].holes.len(),
        1,
        "exactly one hole — the controlled subject: {sem}"
    );

    // Leaf (single-token, intransitive) participle: "arising, HeLa affects BRCA1" — the `ger` VP is
    // the leaf itself, shifted in the seeding loop (not the composed CKY loop).
    let (lc, lo) = index.parse_open("arising , HeLa affects BRCA1", &Identity);
    assert!(
        lc.is_empty() && !lo.is_empty(),
        "leaf intransitive participle yields an open parse"
    );
    assert_eq!(
        lo[0].holes.len(),
        1,
        "leaf participle has one controlled-subject hole"
    );
    assert!(
        pretty_term(lo[0].item.sem()).contains("arises"),
        "leaf participial proposition asserted: {}",
        pretty_term(lo[0].item.sem())
    );

    // The comma is required by the construction but the absorption makes the comma-less variant parse
    // too; the baseline (no adjunct) is unaffected.
    assert_eq!(
        index.parse("HeLa affects BRCA1", &Identity).len(),
        1,
        "baseline declarative unaffected"
    );
}

#[test]
fn non_restrictive_relative_in_object_and_prep_object_position() {
    // D62 §2 #2A (object + prep-object): the appositive relative composes when its antecedent is a
    // verb's direct object (in-situ object raise, mirroring `a_obj`) or a preposition's object (the
    // subject-raise riding the GQ-as-preposition-object rule). Both conjoin the appositive assertion.
    let (_layer, index) = index_over_bootstrap();

    // Verb direct object: "HeLa affects [BRCA1, which affects HeLa]".
    let vo = index.parse("HeLa affects BRCA1 , which affects HeLa", &Identity);
    assert!(!vo.is_empty(), "verb-object appositive parses");
    let vsem = pretty_term(vo[0].sem());
    assert!(
        vsem.starts_with("And(") && vsem.matches("affects").count() == 2,
        "verb-object appositive conjoins matrix + relative assertion: {vsem}"
    );

    // Preposition object: "[a gene within [BRCA1, which affects HeLa]] is large".
    let po = index.parse(
        "a gene within BRCA1 , which affects HeLa , is large",
        &Identity,
    );
    assert!(!po.is_empty(), "prep-object appositive parses");
    assert!(
        pretty_term(po[0].sem()).contains("affects")
            && pretty_term(po[0].sem()).contains("prep_within"),
        "prep-object appositive conjoins the relative assertion into the PP: {}",
        pretty_term(po[0].sem())
    );
}

#[test]
fn transitional_adverbs_and_fronted_comma_parse() {
    // D62 §2 #5b: sentence-initial TRANSITIONAL adverbs (`thus`/`therefore`/…) attach at the clause
    // level and are transparent (same claim as unmodified); a comma after the fronted adverb is
    // absorbed; and a DEGREE-modified adverb (`more largely`) forms a transparent sentence adverb.
    let (_layer, index) = index_over_bootstrap();
    let base = index.parse("HeLa affects BRCA1", &Identity);
    assert_eq!(base.len(), 1, "baseline parses once");
    let base_sem = pretty_term(base[0].sem());
    for s in [
        "thus HeLa affects BRCA1",      // sentence-initial transitional, no comma
        "thus , HeLa affects BRCA1",    // + fronted-comma absorption
        "therefore HeLa affects BRCA1", // another transitional
        "more largely , HeLa affects BRCA1", // degree-modified adverb + comma
    ] {
        let f = index.parse(s, &Identity);
        assert!(!f.is_empty(), "`{s}` parses");
        assert_eq!(
            pretty_term(f[0].sem()),
            base_sem,
            "transitional/degree adverb is transparent (same claim): `{s}`"
        );
    }
}

#[test]
fn non_restrictive_relative_is_a_separate_assertion() {
    // D62 §2 #2A: a NON-restrictive (comma-set-off) relative on a referring NP is a SEPARATE
    // assertion — the antecedent type-raised to a conjoining quantifier `λP. And(P(r), body(r))`,
    // NOT a Σ-restriction (core-en `RelPro-Appos`). `BRCA1, which affects HeLa, is primary` ⇒
    // `And(is_primary(brca1), affects(…, brca1))` — both conjuncts about the same referent.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("BRCA1 , which affects HeLa , is primary", &Identity);
    assert!(!forest.is_empty(), "non-restrictive relative parses");
    let sem = pretty_term(forest[0].sem());
    assert!(
        sem.contains("And") && sem.contains("is_primary") && sem.contains("affects"),
        "non-restrictive relative conjoins the matrix and the relative assertion: {sem}"
    );
    // Each parse still inhabits Prop (kernel-gated).
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "inhabits Prop");

    // Regression: the RESTRICTIVE relative (no comma) still Σ-refines the noun (not conjoined).
    assert!(
        !index
            .parse("a gene which affects HeLa is large", &Identity)
            .is_empty(),
        "restrictive relative unaffected"
    );
}

/// D62/GH#97 Lever 3 — **VP-adjunct preposition takes a quantified / bare / compound object.**
/// The S4 gap (witnessed on the full lexicon as `… for cancer therapeutics`) localized on the small
/// lexicon to the VP-adjunct prep (`to`/`for` = `(S\NP)\(S\NP)/NP`): its object slot only accepted a
/// bare NAME, because the GQ-as-preposition-object raise was restricted to the noun-modifier `cat_pp`
/// functor. Extending the raise to the VP-adjunct functor closes it — for single, compound, and
/// bare-plural objects alike (the noun-modifier `within` already worked, kept here as the control).
#[test]
fn vp_adjunct_preposition_takes_quantified_and_compound_objects() {
    let (_layer, index) = index_over_bootstrap();
    let closes = |s: &str| {
        assert!(
            !index.parse(s, &PluralS).is_empty(),
            "expected a closed parse: {s:?}"
        );
    };

    // VP-adjunct prep (`to`) — the fixed cases: determined single, determined compound, bare compound.
    // Bare-plural (incl. compound) objects now COMMIT to their kind (reshape §7.4) ⇒ closed.
    closes("HeLa affects BRCA1 to a gene"); //                    GQ (determined single) object
    closes("HeLa affects BRCA1 to a gene cell line"); //          compound (determined) object
    closes("HeLa affects BRCA1 to gene genes"); //                compound (bare-plural KIND) object
                                                // Noun-modifier prep (`within`) — control, already supported (still must hold).
    closes("a cell line within a gene cell line affects BRCA1");
    closes("a cell line within gene genes affects BRCA1"); //     compound bare-plural KIND, noun-mod prep
}

/// D62/GH#97 — a **modal clause composes with a VP-adjunct PP whose object is quantified** (the
/// Lever-3 raise interacting with the modal's base VP). Confirms S4's `can … for <obj>` shape is
/// grammar-complete on the clean lexicon: the modal `can` takes the base VP `affect BRCA1 to a gene`
/// (the `to`-PP attaches to the base VP, then the modal wraps it). (S4's full-lexicon gap is beam/sense
/// scale — uniform with S1/S3/S5 — not a grammar gap; see d63-cnl-parse-levers-plan.)
#[test]
fn modal_clause_takes_a_vp_adjunct_pp() {
    let (_layer, index) = index_over_bootstrap();
    let closes = |s: &str| {
        assert!(
            !index.parse(s, &Identity).is_empty(),
            "expected a closed parse: {s:?}"
        );
    };
    closes("HeLa can affect BRCA1"); //               modal + base + name object
    closes("HeLa can affect a gene"); //              modal + base + GQ object
    closes("HeLa can affect BRCA1 to a gene"); //     modal + VP-adjunct prep GQ object (Lever 3)
    closes("HeLa can affect a gene to BRCA1"); //     modal + GQ object + VP-adjunct prep name object
}

/// D62/GH#97 — **mood-polymorphic VP-adjunct preposition.** The VP-adjunct prep cats were
/// `fin`-locked (`cat_s(dcl, fin)`), so a PP could attach only to a FINITE VP — never to a BASE VP
/// under do-support / a modal, which is what `does not lead to cell death` / `can … for …` need.
/// Making the prep `fin_any` lets the PP attach INSIDE the base VP, with the correct scope (the PP
/// falls under `Possible`/negation), and the reading is a real verb+prep term (not a noun-pile).
#[test]
fn vp_adjunct_pp_attaches_inside_a_base_vp() {
    let (_layer, index) = index_over_bootstrap();
    let sem_of = |s: &str| -> Vec<String> {
        index
            .parse(s, &Identity)
            .iter()
            .map(|it| match eval(it.sem(), &Rho::Nil) {
                Ok(v) => pretty_term(&readback_val(0, &v)),
                Err(_) => pretty_term(it.sem()),
            })
            .collect()
    };
    // Modal: the PP scopes UNDER `Possible` (attached to the base VP the modal consumes).
    let modal = sem_of("HeLa can affect BRCA1 to HeLa");
    assert!(
        modal
            .iter()
            .any(|s| s.contains("Possible(And(") && s.contains("prep_to")),
        "the `to`-PP attaches inside the modal's base VP (under Possible); got: {modal:?}"
    );
    // Do-support negation: the PP scopes UNDER the negation (`(affect ∧ to) → False`).
    let neg = sem_of("HeLa does not affect BRCA1 to HeLa");
    assert!(
        neg.iter()
            .any(|s| s.contains("prep_to") && s.contains("False") && s.contains("And(")),
        "the `to`-PP attaches inside the negated base VP; got: {neg:?}"
    );
}

#[test]
fn to_preposition_parses() {
    // D62 CNL: `to` added as a preposition (VP-adjunct + noun-modifier), mirroring `in`/`for` —
    // pervasive (`leads to`, `respond to`, `compared to`, `essential to`).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        !index
            .parse("HeLa affects BRCA1 to HeLa", &Identity)
            .is_empty(),
        "`to` VP-adjunct PP parses"
    );
    assert!(
        !index
            .parse("HeLa affects a gene to HeLa", &Identity)
            .is_empty(),
        "`to` PP with a determined object parses (GQ-as-prep-object)"
    );
}

#[test]
fn mass_noun_is_a_bare_argument() {
    // D63 kind-predication reshape (Phase A): a mass noun (`cat_n(_, mass)`) is a bare argument
    // (subject + object) that denotes its KIND realized as an entity — `kind_of(C) : Entity` — so the
    // parse is CLOSED (`affects(kind_of(Instability), hela)`), not an open deferred quantifier. `*a
    // instability` still fails (mass meets neither sg nor pl).
    let (_layer, index) = index_over_bootstrap();
    let subj = index.parse("instability affects HeLa", &Identity);
    assert_eq!(
        subj.len(),
        1,
        "bare mass subject yields a single closed parse"
    );
    assert!(
        pretty_term(subj[0].sem()).contains("kind_of(Instability)"),
        "the mass subject is the kind nominalized to an entity, got {}",
        pretty_term(subj[0].sem())
    );
    let obj = index.parse("HeLa affects instability", &Identity);
    assert_eq!(
        obj.len(),
        1,
        "bare mass object yields a single closed parse"
    );
    assert!(
        pretty_term(obj[0].sem()).contains("kind_of(Instability)"),
        "the mass object is likewise the kind nominalized"
    );
    assert!(
        index
            .parse("a instability affects HeLa", &Identity)
            .is_empty(),
        "`a` + mass noun correctly fails (mass is not singular-countable)"
    );
}

#[test]
fn plural_definite_the_parses() {
    // D62 CNL fix 1: `the` + a PLURAL noun (`the genes affect HeLa`) now parses — the singular `the`
    // failed determiner/noun number agreement on a plural noun. Subject + object positions.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        !index.parse("the genes affect HeLa", &PluralS).is_empty(),
        "`the` + plural noun (subject) parses"
    );
    assert!(
        !index.parse("HeLa affects the genes", &PluralS).is_empty(),
        "`the` + plural noun (object) parses"
    );
    // Singular `the` unaffected.
    assert!(
        !index.parse("the gene affects HeLa", &Identity).is_empty(),
        "`the` + singular noun still parses"
    );
}

#[test]
fn but_not_contrastive_object_ellipsis() {
    // D62 §2 #8: `[verb] O₁ but not O₂` — the shared verb applies affirmatively to O₁ and negatively
    // to the elided O₂: `V(O₁) ∧ ¬V(O₂)` (intuitionistic `¬P = P → logic:False`). Two paths: a
    // contrastive `conn_but_not` GROUP for bare-name objects (not Prop-ending), and the general
    // contrastive conjunction for determined-NP / GQ objects (the WRN shape).
    let (_layer, index) = index_over_bootstrap();

    // Bare-name objects (group path).
    let f = index.parse("HeLa affects BRCA1 but not HeLa", &Identity);
    assert!(!f.is_empty(), "bare-name `but not` parses");
    let sem = pretty_term(f[0].sem());
    assert!(
        sem.starts_with("And(") && sem.contains("False"),
        "affirms O₁, negates O₂ (`→ False`): {sem}"
    );

    // Determined-NP objects, same base type, with a POSSESSIVE O₂ (the `… but not its …` WRN shape):
    // the possessor is a referent hole ⇒ an OPEN parse.
    let (closed, open) = index.parse_open("HeLa affects the gene but not its gene", &Identity);
    assert!(
        closed.is_empty() && !open.is_empty(),
        "possessive O₂ yields an open parse (the WRN `but not its …` shape)"
    );
    let osem = pretty_term(open[0].item.sem());
    assert!(
        osem.contains("And(") && osem.contains("False") && osem.contains("poss_of"),
        "the contrastive conjunction negates the possessive conjunct: {osem}"
    );
    assert_eq!(open[0].holes.len(), 1, "one hole — the possessor of O₂");
}

#[test]
fn but_not_cross_type_objects_is_a_known_gap() {
    // D62 §2 #8 limitation: two determined objects of DIFFERENT base types
    // (`a gene but not a cell line`) don't coordinate — the object-raise (`a_obj`) bakes the noun
    // type into the GQ category, so the two GQs aren't the same category. Widening the shared verb
    // slot to the common supertype is a follow-on. (The WRN case has the SAME base type — `… activity`
    // — so it is covered; see `but_not_contrastive_object_ellipsis`.)
    let (_layer, index) = index_over_bootstrap();
    let (closed, open) = index.parse_open("HeLa affects a gene but not a cell line", &Identity);
    assert!(
        closed.is_empty() && open.is_empty(),
        "cross-type `but not` objects not yet coordinated (update when slot-widening lands)"
    );
}

#[test]
fn cardinal_numerals_are_plural_determiners() {
    // D62 §2 #4: word-form cardinals (`two`..`ten`) parse as plural determiners in subject and object
    // position. First-cut semantics is existential with the count DROPPED (`two genes` ≈ `∃ genes`);
    // the exact cardinality is a faithfulness follow-on.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        !index.parse("two genes affect HeLa", &PluralS).is_empty(),
        "numeral subject parses"
    );
    assert!(
        !index.parse("HeLa affects two genes", &PluralS).is_empty(),
        "numeral object parses"
    );
    // Numerals mirror the existing plural determiners (`these`/`those`) exactly, including their
    // (pre-existing) agreement behaviour; no numeral-specific agreement claim is made here.
    assert!(
        !index.parse("four genes affect HeLa", &PluralS).is_empty(),
        "another cardinal (`four`) parses"
    );
}

#[test]
fn light_verb_give_rise_to_is_a_multiword_transitive() {
    // D62 §2 #7: a light-verb MWE (`give rise to`) seeds as a 3-token span and composes as a
    // transitive verb `(S\NP)/NP` over an opaque causation axiom. Present (3sg/pl) + past forms.
    let (_layer, index) = index_over_bootstrap();
    let f = index.parse("HeLa gives rise to BRCA1", &Identity);
    assert!(!f.is_empty(), "3sg light verb parses");
    assert!(
        pretty_term(f[0].sem()).contains("give_rise_to"),
        "maps to the causation axiom: {}",
        pretty_term(f[0].sem())
    );
    assert!(
        !index.parse("HeLa gave rise to BRCA1", &Identity).is_empty(),
        "past light verb parses"
    );
    // Bare-plural subject commits to its kind (reshape §7.4) ⇒ a closed parse.
    assert!(
        !index.parse("genes give rise to HeLa", &PluralS).is_empty(),
        "bare-plural kind subject of the light verb yields a closed parse"
    );
}

#[test]
fn post_nominal_alone_is_an_exclusive_focus_refinement() {
    // D61: `alone` (bare post-nominal `cat_pp`) refines the head noun via the PpMod rule —
    // `[cat_n(gene)] [cat_pp]` → `Σx:gene. sole(x)` — under ANY determiner. The exclusive
    // `ontology:sole` operator is the "= only" reading. Validated on the demo lexicon (no reseed).
    let (_layer, index) = index_over_bootstrap();
    let f = index.parse("each gene alone affects HeLa", &Identity);
    assert!(!f.is_empty(), "`each gene alone affects HeLa` parses");
    let sem = pretty_term(f[0].sem());
    assert!(
        sem.contains("sole"),
        "the subject carries the exclusive `sole` operator: {sem}"
    );
    // Baseline without `alone` still parses (no regression to the plain determiner path).
    assert!(
        !index.parse("each gene affects HeLa", &Identity).is_empty(),
        "`each gene affects HeLa` still parses"
    );

    // The FULL structure of sentence 3 — `each [N] alone does not [V]` — over the demo lexicon:
    // `alone` refines the subject noun AND declarative do-support + VP-negation compose, giving
    // `∀x:(Σy:gene. sole(y)). ¬affect(x, hela)`. This is exactly the shape "each event alone does not
    // lead to cell death" takes once the full lexicon is reseeded (WordNet event/lead in place of
    // demo gene/affect). Witnesses the universal quantifier, the `sole` refinement, and the negation
    // (`→ logic:False`) all in one reading.
    let neg = index.parse("each gene alone does not affect HeLa", &Identity);
    assert!(
        !neg.is_empty(),
        "`each gene alone does not affect HeLa` parses (alone + do-support + negation)"
    );
    let nsem = pretty_term(neg[0].sem());
    assert!(
        nsem.contains("sole") && nsem.contains("False"),
        "faithful reading: universal over the sole-refined noun, negated: {nsem}"
    );
}

#[test]
fn comma_list_coordination_parses() {
    // D62 S0 slice 2: a comma is a conjunctive list separator, so a multi-item subject list builds
    // the (left-branching) member group the distributive subject rule consumes.
    let (_layer, index) = index_over_bootstrap();
    // Comma-only 2-member list (no final `and`).
    assert!(
        !index.parse("HeLa, BRCA1 affect HeLa", &Identity).is_empty(),
        "comma joins a two-member subject group"
    );
    // 3-member `A, B and C` list (commas + final `and`, left-branching).
    assert!(
        !index
            .parse("HeLa, BRCA1 and HeLa affect HeLa", &Identity)
            .is_empty(),
        "comma + `and` builds a three-member group"
    );
    // Baseline binary `and` (no comma) still parses — no regression.
    assert!(
        !index
            .parse("HeLa and BRCA1 affect HeLa", &Identity)
            .is_empty(),
        "binary `and` coordination unaffected"
    );
}

#[test]
fn sense_reranker_overrides_static_cap_order() {
    // Static cap(1), no reranker: keeps rank-0 (BRCA1) → the parse's sem mentions BRCA1.
    let forest = index_with_zarg(1, None).parse("zarg affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "one parse survives the cap");
    let sem = format!("{:?}", forest[0].sem());
    assert!(
        sem.contains("brca1"),
        "static cap keeps the rank-0 (BRCA1) sense: {sem}"
    );

    // Same cap(1), but a reranker preferring `zarg.1`: the cap now keeps HeLa, not BRCA1 — the
    // contextual rank overrode the static `sense_rank`, and the kernel still gated the result.
    let forest = index_with_zarg(1, Some(Box::new(PreferSense("zarg.1"))))
        .parse("zarg affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "still one parse survives the cap");
    let sem = format!("{:?}", forest[0].sem());
    assert!(
        !sem.contains("brca1"),
        "the reranker dropped the rank-0 BRCA1 sense from the cap: {sem}"
    );
    assert!(
        sem.contains("hela"),
        "the reranker kept the contextual HeLa sense: {sem}"
    );
}

fn assert_parses_to_prop(sentence: &str) {
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse(sentence, &Identity);
    assert!(
        !forest.is_empty(),
        "'{sentence}' must yield at least one felicitous S:Prop parse from the committed determiners"
    );
    for p in &forest {
        assert!(
            is_ctor(p.cat(), "cat_s").is_some(),
            "'{sentence}': each parse is an S"
        );
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
        let ty = check_infer(&mut ctx, p.sem())
            .unwrap_or_else(|e| panic!("'{sentence}' must type-check: {e}"));
        assert_eq!(
            readback_val(0, &ty),
            Exp::Sort(0),
            "'{sentence}' must inhabit Prop"
        );
    }
}

#[test]
fn committed_subject_determiners_parse() {
    // `every` / `no` (subject) from the committed closed-class layer.
    assert_parses_to_prop("every cell line affects HeLa"); // ∀c:CellLine. affects(HeLa, c)
    assert_parses_to_prop("no cell line affects HeLa"); //    ∀c:CellLine. ¬affects(HeLa, c)
}

/// D62/D63 — **N-N compound stacking** (`MSI cancer models` shape) and **bare-plural over a composed
/// compound**. 3-noun stacking already left-branches (`[[A B] C]`); the gap was that the bare-plural
/// NP shift ran only at lexical leaves, so a *composed* plural compound could never be a bare argument
/// NP. The shift now also runs on composed `cat_n(_, pl)` cells.
#[test]
fn compound_stacking_and_bare_plural_compound() {
    let (_layer, index) = index_over_bootstrap();
    // 3-noun stacking under a determiner: [[gene gene] cell line].
    assert!(
        !index
            .parse("a gene gene cell line affects HeLa", &Identity)
            .is_empty(),
        "3-noun compound stacking must parse"
    );
    // Bare-plural over a composed compound: [gene genes] (modifier + plural head), bare subject → a
    // CLOSED kind-predication `affect(hela, kind_of(Σx:Gene. compound_kind(x,Gene)))`. The whole refined
    // type is nominalized (reshape §7.4) — the compound-noun case the DetRefine Fst-projection broke.
    let closed = index.parse("gene genes affect HeLa", &PluralS);
    assert!(
        closed
            .iter()
            .any(|p| pretty_term(p.sem()).contains("kind_of(")
                && pretty_term(p.sem()).contains("compound_kind(")),
        "a bare-plural composed compound is a closed kind-predication over the refined type"
    );
    // Compound bare-plural noun as a KIND SUBJECT of the copula (this session — the `bnp`-unary-rule
    // re-alignment with core-en): the COMPOSED compound `gene genes` also gets the `cat_kind`
    // copula-subject edge, so `are_kind` yields `subclass_of` over the COMPOUND kind. The leaf shift in
    // `lookup_span` never reached a chart-formed compound, so before this the compound-kind subject
    // GAPed — the corpus `Nucleotide repeat regions are microsatellites` gap.
    let kind_subj = index.parse("gene genes are cell lines", &PluralS);
    assert!(
        kind_subj.iter().any(|p| {
            sem_mentions_axiom(p.sem(), "urn:eigenius:ontology:subclass_of")
                && sem_mentions_axiom(p.sem(), "urn:eigenius:ontology:compound_kind")
        }),
        "a compound bare-plural noun is a kind subject: `gene genes are cell lines` → subclass_of over \
         the compound kind; got {:?}",
        kind_subj
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );
}

/// D63 Option A (blueprint §11 3b) — the **packing router is a safe no-op stub** so far. With
/// `with_packing(true)`, a selectional sentence (`depends on` — `NP_Gene`/`NP_CellLine` slots) is
/// routed to the UNPACKED path by the guard, and a generic sentence to the packed *stub* (which also
/// delegates to unpacked). Both must still parse — proving the router + `parse_needs_unpacked` guard
/// don't break parsing before the real packed forest (3c/3d) lands.
#[test]
fn packing_flag_router_is_a_safe_noop() {
    let (layer, _) = index_over_bootstrap();
    let index = Parser::build(Arc::clone(&layer)).with_packing(true);
    // Selectional (`depends on` wants a CellLine subject + Gene object; HeLa:CellLine, BRCA1:Gene —
    // the demo's own worked sentence; routes unpacked via the guard):
    assert!(
        !index.parse("HeLa depends on BRCA1", &Identity).is_empty(),
        "selectional sentence must parse (routed unpacked)"
    );
    // Generic (routes to the packed stub → unpacked):
    assert!(
        !index.parse("HeLa affects BRCA1", &Identity).is_empty(),
        "generic sentence must parse (packed stub → unpacked)"
    );
}

/// D63 Option A (blueprint §11 3f.2) — the **guard fail-closed** check, asserting the router's
/// *decision* directly (via `routes_packed`), since packed ≡ unpacked makes the taken path otherwise
/// invisible. Packable sentences route packed; selectional / token-keyed-construct sentences route
/// unpacked; flag off never packs.
#[test]
fn packing_router_decision_is_correct() {
    let (layer, _) = index_over_bootstrap();
    let on = Parser::build(Arc::clone(&layer)).with_packing(true);
    let off = Parser::build(Arc::clone(&layer)).with_packing(false);

    // Index-independent, construct-free ⇒ PACKED.
    assert!(on.routes_packed("HeLa affects BRCA1", &Identity));
    assert!(on.routes_packed("every cell line affects HeLa", &Identity));
    assert!(on.routes_packed("HeLa is a large gene", &Identity));
    // Restrictive `that`-relative is packed now (§11 3g.3).
    assert!(on.routes_packed("a gene that affects HeLa is large", &Identity));
    // Coordination (`and`/`or`) is packed now (§11 3g.3 — the `Coordinate` edge).
    assert!(on.routes_packed("HeLa and BRCA1 affect HeLa", &Identity));
    // The restrictive which-relative and the wh-determiner `which` are packed now (§11 3g.3):
    // `which` is no longer a blanket guard — only its pied-piping use routes unpacked.
    assert!(on.routes_packed("a gene which affects HeLa is large", &Identity));
    assert!(on.routes_packed("which cell line affects HeLa", &Identity));

    // Comma constructs are packed (§11 3g.3): list coordination builds the deferred `cat_coord` /
    // `cat_group` via `Coordinate` edges + the `CoordComplete` unary shift — all binary/unary, so the
    // packed hyperedge model expresses the list-with-operator model directly (D63 §8.4 Phase 3, the
    // coordination refactor). The appositive (`Appositive*`) and fronted-modifier comma (`AbsorbComma`)
    // are packed too.
    assert!(on.routes_packed("HeLa, BRCA1 affect HeLa", &Identity)); // list coordination
    assert!(on.routes_packed("BRCA1 , which affects HeLa , is primary", &Identity)); // appositive
    assert!(on.routes_packed("thus , HeLa affects BRCA1", &Identity)); // fronted-comma absorption

    // Selectional (`depends on`: Gene/CellLine slots) is PACKED now — per-cell packing keys the
    // concrete-slot items finer (`node_sig` → `cat_key`) so they never wrongly share a node, instead
    // of forcing the whole sentence unpacked (D63 §11 3d). Soundness is witnessed by the differential
    // oracle `packed_forest_equals_unpacked_on_core_grammar`, which now covers selectional sentences.
    assert!(on.routes_packed("HeLa depends on BRCA1", &Identity));
    // Pied-piping (`[prep] which`) is the one construct the packed forest builds no edge for (ternary,
    // non-piling) ⇒ routed UNPACKED (structural detection). A completeness carve-out, not soundness.
    assert!(!on.routes_packed("the gene in which HeLa affects BRCA1 is large", &Identity));

    // Flag off ⇒ never packs, even for a packable sentence.
    assert!(!off.routes_packed("HeLa affects BRCA1", &Identity));
}

/// The shared **driver-parity** assertion behind the differential oracle (reorganization plan
/// Phase 0, `docs/notes/dcg-module-reorganization-plan.md`). For each case, parses on BOTH chart
/// paths and compares the CLOSED forest as a sorted multiset of sems plus the OPEN count.
///
/// `exercises_rule` marks a case added to WITNESS a specific rule. Such a case is **fail-closed**:
/// if it parses to nothing on either path it "agrees" trivially, so it no longer witnesses anything
/// — and a corpus quietly degrading to vacuous agreement is the failure mode this oracle exists to
/// prevent. A case that legitimately has no parse (a negative case) is marked `false`.
fn assert_paths_agree(off: &Parser, on: &Parser, cases: &[(&str, bool)]) {
    for &(s, exercises_rule) in cases {
        let (co, oo) = off.parse_open(s, &Identity);
        let (cn, on2) = on.parse_open(s, &Identity);
        let mut so: Vec<String> = co.iter().map(|it| pretty_term(it.sem())).collect();
        let mut sn: Vec<String> = cn.iter().map(|it| pretty_term(it.sem())).collect();
        so.sort();
        sn.sort();
        assert_eq!(so, sn, "packed≠unpacked CLOSED forest for {s:?}");
        assert_eq!(oo.len(), on2.len(), "packed≠unpacked OPEN count for {s:?}");
        if exercises_rule {
            assert!(
                !co.is_empty() || !oo.is_empty(),
                "{s:?} is a rule-exercising oracle case but parses to NOTHING on either path — \
                 it agrees vacuously and no longer witnesses the rule it was added for"
            );
        }
    }
}

/// D63 Option A (blueprint §11 3f.1) — the **differential oracle**: on index-independent,
/// construct-free sentences (where the router actually engages packing), the packed path must produce
/// the SAME felicitous forests as the unpacked path. Proves the packed forest + cube extractor are
/// equivalent to the flat CKY (and that deferring selectional pruning to felicity is sound: felicity
/// ⊇ unify). Compares the closed forest as a sorted multiset of normalized sems, plus the open count.
#[test]
fn packed_forest_equals_unpacked_on_core_grammar() {
    let (layer, _) = index_over_bootstrap();
    let off = Parser::build(Arc::clone(&layer)).with_packing(false);
    let on = Parser::build(Arc::clone(&layer)).with_packing(true);
    assert_paths_agree(
        &off,
        &on,
        &[
            ("HeLa affects BRCA1", true),
            ("every cell line affects HeLa", true),
            ("no gene affects HeLa", true),
            ("HeLa is a gene", true),
            ("HeLa is large", true),
            ("HeLa is a large gene", true),
            ("a large primary gene affects HeLa", true),
            ("HeLa affects a gene", true),
            // Restrictive `that`-relative (§11 3g.3 — packed via the Relativize edge):
            ("a gene that affects HeLa is large", true),
            ("every gene that affects HeLa is large", true),
            // Coordination (§11 3g.3 — the Coordinate edge + group distribution):
            ("HeLa affects BRCA1 and HeLa affects BRCA1", true), // same-category Prop conjunction
            ("HeLa and BRCA1 affect HeLa", true), // NP-group subject, distributed over the verb
            ("HeLa affects BRCA1 or HeLa affects BRCA1", true), // disjunction
            // Reciprocal (§11 3g.3 — the Reciprocal edge over a coordinated group):
            ("HeLa and BRCA1 affect each other", true), // ordered distinct pairs related by the verb
            // An or-group gets NO reciprocal reading — a NEGATIVE case (both paths agree on nothing).
            ("HeLa or BRCA1 affect each other", false),
            // Contrastive `but not` (§11 3g.3 — the ButNot edge) + plain `but` (the subordinator leaf):
            ("HeLa affects BRCA1 but not HeLa", true), // bare-name contrastive object ellipsis
            ("HeLa affects BRCA1 but HeLa affects BRCA1", true), // plain `but` subordinator (leaf)
            // Restrictive which-relative + wh-determiner `which` (§11 3g.3 — Relativize edge / leaves):
            ("every cell line which affects HeLa is primary", true), // which-relative (Relativize)
            ("which cell line affects HeLa", true), // subject wh-determiner (cat_q, lexical leaf)
            // Comma constructs (§11 3g.3 — Coordinate / Appositive* / AbsorbComma edges):
            ("HeLa, BRCA1 affect HeLa", true), // comma list coordination (2-member group)
            ("HeLa affects BRCA1 , which affects HeLa", true), // verb-object appositive
            ("BRCA1 , which affects HeLa , is primary", true), // subject appositive (comma absorbed)
            ("thus , HeLa affects BRCA1", true), // fronted transitional + comma absorption
            ("more largely , HeLa affects BRCA1", true), // degree-modified fronted adverb + comma
            // CONCRETE SELECTIONAL SLOTS (D63 §11 3d — per-cell packing). These route PACKED now (the
            // whole-sentence selectional carve-out is gone); the concrete-slot items key finer via
            // `cat_key` so they never wrongly share a node. This is the soundness witness for the
            // refinement — the packed path must still equal the unpacked path with the residue present.
            ("HeLa depends on BRCA1", true), // selectional verb `depends on`, both args concrete
            ("every cell line depends on BRCA1", true), // selectional verb + determiner subject
            ("no cell line depends on BRCA1", true), // selectional verb + negative determiner
            ("a cell line that depends on BRCA1 is primary", true), // selectional verb in a relative
            ("HeLa depends on BRCA1 and HeLa depends on BRCA1", true), // selectional + coordination
            // Close nominal apposition (D63 §8.4 Phase 6, RC-6 — the packed `ApposeGroup` edge).
            // Singular head + name-GROUP so it works under the `Identity` lemmatizer (the plural-head
            // form is in `close_apposition_subject_and_object`, which uses `PluralS`).
            ("the gene BRCA1 and BRCA1 affect HeLa", true),
            ("HeLa affects the gene BRCA1 and BRCA1", true),
        ],
    );
}

/// Driver parity over a corpus built to STRESS the two rule-wiring sites where the packed and
/// unpacked paths are written separately (reorganization plan Phase 0): the coordination rule (which
/// the unpacked path fires as two independent `if let`s — `coordinate_prop` AND `coordinate_np` — but
/// the packed path fires as `coordinate_prop().or_else(coordinate_np)`, taking only the first) and the
/// `but not` rule (whose packed DECISION reads a sem, `is_coordination(r.sem())`, on a node
/// REPRESENTATIVE). n-ary lists, comma lists, mixed connectives, group-vs-GQ coordination, object-GQ
/// generalization, and coordination inside a relative are all exercised.
///
/// Sentences that do NOT parse are excluded rather than carried as `false` flags: a non-parsing case
/// agrees vacuously and would be fake coverage. The ones that were tried and dropped are recorded in
/// `coordination_gaps_are_not_driver_divergences` below, because "it doesn't parse" is a finding about
/// the GRAMMAR, not licence to widen the oracle with cases that assert nothing.
#[test]
fn packed_forest_equals_unpacked_on_coordination_and_butnot_stress() {
    let (layer, _) = index_over_bootstrap();
    let off = Parser::build(Arc::clone(&layer)).with_packing(false);
    let on = Parser::build(Arc::clone(&layer)).with_packing(true);
    assert_paths_agree(
        &off,
        &on,
        &[
            // --- coordination: the prop-list vs NP-group split (the `.or_else` divergence site) ---
            ("HeLa affects BRCA1 and MSH2", true), // object NP-group (coordinate_np)
            ("HeLa and BRCA1 affect BRCA1 and MSH2", true), // NP-group subject AND object
            ("HeLa and BRCA1 and MSH2 affect HeLa", true), // n-ary left-branching group
            ("HeLa , BRCA1 and MSH2 affect HeLa", true), // comma list finalized by `and`
            ("HeLa affects a gene or a cell line", true), // object-GQ generalization (common_cat)
            ("HeLa is large and primary", true),   // predicative-adjective coordination
            (
                "HeLa affects BRCA1 and HeLa affects BRCA1 and HeLa affects BRCA1",
                true,
            ), // 3 props
            (
                "HeLa affects BRCA1 , HeLa affects BRCA1 and HeLa affects BRCA1",
                true,
            ), // comma props
            ("a gene that affects BRCA1 and MSH2 is large", true), // coordination inside a relative
            // --- `but not`: the sem-reading decision (`is_coordination`) on a representative ---
            ("HeLa affects BRCA1 but not MSH2", true), // bare-name contrastive object
            ("HeLa is large but not primary", true),   // predicative-adjective contrastive
            ("HeLa affects BRCA1 but not HeLa affects BRCA1", true), // clausal contrastive
        ],
    );
}

/// Phase 0 finding (reorganization plan): sentences tried as `but not` / coordination stress cases
/// that parse to NOTHING on **both** paths. They are recorded here rather than silently dropped —
/// each is a GRAMMAR gap, not a driver divergence, and two of them matter to the refactor:
///
/// - `but not` over a COORDINATED operand does not parse. Those were the only sentences that would
///   have put a coordination sem and a non-coordination sem in one packed node — i.e. the only ones
///   that could witness whether `apply_bin_rule`'s `ButNot` sem-read on a REPRESENTATIVE
///   (`lookup.rs`, `is_coordination(r.sem())`) can diverge from the per-pair decision. **The hazard
///   is therefore latent and unwitnessable at the sentence level in this fixture** — Phase 2 must fix
///   it structurally (an explicit `sem_blind: false` rule), not wait for a failing parse to prove it.
/// - A coordinated QUANTIFIED subject (`a gene and a cell line affect HeLa`) does not parse: a raised
///   GQ is `S/(S\NP)`, not a `cat_np`, so `coordinate_np` cannot fire, and `coordinate_prop` refuses
///   to generalize subject-GQs (agreement would stop biting). Out of scope for the reorganization;
///   filed here as an observed gap.
///
/// This test asserts they still fail on BOTH paths — so if a future change makes one parse, it must
/// be re-examined and promoted into the stress corpus above rather than silently changing behaviour.
#[test]
fn coordination_gaps_are_not_driver_divergences() {
    let (layer, _) = index_over_bootstrap();
    let off = Parser::build(Arc::clone(&layer)).with_packing(false);
    let on = Parser::build(Arc::clone(&layer)).with_packing(true);
    for s in [
        "a gene and a cell line affect HeLa", // coordinated GQ subject
        "every gene and every cell line affect HeLa", // coordinated universal subject
        "HeLa affects BRCA1 and MSH2 but not HeLa", // `but not` after a coordinated object
        "HeLa affects BRCA1 but not BRCA1 and MSH2", // `but not` OVER a coordination
        "a gene that affects HeLa and a cell line that affects HeLa are large", // coordinated relatives
    ] {
        let (co, oo) = off.parse_open(s, &Identity);
        let (cn, on2) = on.parse_open(s, &Identity);
        assert!(
            co.is_empty() && oo.is_empty() && cn.is_empty() && on2.is_empty(),
            "{s:?} now PARSES (unpacked {}/{}, packed {}/{}) — a recorded grammar gap has closed. \
             Re-examine it and move it into the stress corpus; do not just delete this line.",
            co.len(),
            oo.len(),
            cn.len(),
            on2.len(),
        );
    }
}

/// Reorganization plan, **open decision #1** (`docs/notes/dcg-module-reorganization-plan.md`): are
/// [`coordinate_prop`] and [`coordinate_np`] DISJOINT — i.e. can one `(left, right, op)` triple ever
/// make both return `Some`?
///
/// It matters because the two chart paths wire coordination differently. The unpacked CKY fires them
/// as two independent `if let`s and pushes BOTH results; the packed path's `apply_bin_rule` uses
/// `coordinate_prop(…).or_else(|| coordinate_np(…))` and keeps only the FIRST. If a triple can fire
/// both, the packed path silently drops a reading and the paths are not equivalent — a bug, not a
/// refactoring detail. If they are disjoint, `.or_else` is safe and Phase 2 may unify them freely.
///
/// The structural argument is that they are disjoint on the LEFT category: `coordinate_np` requires a
/// `cat_np`/`cat_group` left, while `coordinate_prop` requires a `cat_coord` left or a **prop-ending**
/// one — and `⟦cat_np(T)⟧ = T` (a class) and `⟦cat_group(C)⟧ = List C` are never prop-ending. That is
/// an argument, not a witness, so this test EXERCISES it: build a category pool from the real
/// bootstrap + demo lexicon, close it under composition (application, type-raising, and both
/// coordination builders — so raised GQs, `cat_group`s and `cat_coord`s are all in the pool), and
/// assert no ordered pair under any connective fires both.
#[test]
fn coordinate_prop_and_coordinate_np_are_disjoint() {
    let (layer, _) = index_over_bootstrap();
    let entry_class = Iri::parse("urn:eigenius:lexicon:LexicalEntry").expect("iri");
    // Connectives a coordination rule accepts: `and`, `or`, and the neutral comma list.
    const OPS: [&str; 3] = [
        "urn:eigenius:logic:And",
        "urn:eigenius:logic:Or",
        "urn:eigenius:lexicon:conn_list",
    ];

    // Seed: every lexical entry's item (determiners, nouns, names, verbs, adjectives, prepositions).
    let mut pool: Vec<Item> = Vec::new();
    for (_iri, r) in layer.iter_all_resources() {
        if !r.is_instance_of(&entry_class) {
            continue;
        }
        if let Ok(it) = entry_to_item(&layer, r.as_ref()) {
            pool.push(it);
        }
    }
    assert!(
        pool.len() > 20,
        "expected the bootstrap + demo lexicon to seed a real pool, got {}",
        pool.len()
    );

    // Dedupe by CATEGORY (one representative item per distinct category — the builders decide on
    // categories, and the pool would otherwise be dominated by same-category senses).
    let dedup_by_cat = |items: Vec<Item>| -> Vec<Item> {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for it in items {
            if seen.insert(pretty_term(it.cat())) {
                out.push(it);
            }
        }
        out
    };
    pool = dedup_by_cat(pool);

    // Close under composition so the pool contains the categories coordination actually sees at
    // runtime: applications (det+noun → GQ, verb+NP → VP/S), type-raised NPs, and — crucially — the
    // OUTPUTS of both coordination builders (`cat_group` / `cat_coord`), which are the left operands
    // of the n-ary (extend) cases.
    for _round in 0..2 {
        let mut grown = pool.clone();
        for l in &pool {
            if let Some((cat, sem)) = type_raise(l.cat(), l.sem(), &layer) {
                grown.push(Item::new(cat, sem));
            }
            for r in &pool {
                if let Some(it) = apply(l, r, &layer, eigenius_kernel::dcg::RightContext::Other) {
                    grown.push(it);
                }
                for op in OPS {
                    if let Some((cat, sem)) =
                        coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &layer)
                    {
                        grown.push(Item::new(cat, sem));
                    }
                    if let Some((cat, sem)) =
                        coordinate_prop(op, l.cat(), l.sem(), r.cat(), r.sem(), &layer)
                    {
                        grown.push(Item::new(cat, sem));
                    }
                }
            }
        }
        pool = dedup_by_cat(grown);
    }

    // The pool must actually contain the categories this test exists to discriminate, or it proves
    // nothing (fail-closed: a pool that never built a group/coord/raised-GQ would "pass" vacuously).
    let has = |ctor: &str| pool.iter().any(|it| is_ctor(it.cat(), ctor).is_some());
    assert!(has("cat_np"), "pool has no cat_np");
    assert!(
        has("cat_group"),
        "pool has no cat_group (coordinate_np output)"
    );
    assert!(
        has("cat_coord"),
        "pool has no cat_coord (coordinate_prop output)"
    );
    assert!(has("cat_s"), "pool has no cat_s (a prop-ending category)");

    // The witness: no (left, right, op) triple fires BOTH builders.
    let mut both = Vec::new();
    for l in &pool {
        for r in &pool {
            for op in OPS {
                let p = coordinate_prop(op, l.cat(), l.sem(), r.cat(), r.sem(), &layer);
                let n = coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &layer);
                if p.is_some() && n.is_some() {
                    both.push(format!(
                        "op={op} left={} right={}",
                        pretty_term(l.cat()),
                        pretty_term(r.cat())
                    ));
                }
            }
        }
    }
    assert!(
        both.is_empty(),
        "coordinate_prop and coordinate_np BOTH fire on {} triple(s) over a {}-category pool — the \
         packed path's `.or_else` therefore DROPS a reading the unpacked path keeps (a real \
         divergence, not a refactoring detail):\n  {}",
        both.len(),
        pool.len(),
        both.join("\n  ")
    );
    eprintln!(
        "disjointness witnessed over {} distinct categories ({} ordered pairs × {} connectives)",
        pool.len(),
        pool.len() * pool.len(),
        OPS.len()
    );
}

/// D63 §5.3 — **close naming apposition**, i.e. the CLASSIFIER + DESIGNATOR construction: a sortal
/// common noun + a proper name or identifier (`gene BRCA1`, `project Achilles`, `chromosome 7`) → the
/// definite INDIVIDUAL of the classifier's type bearing that designator, `the(Σx:Sortal. named(x,
/// designator)).1`, at `cat_np(Sortal, …)` — the classifier supplies the TYPE, the designator supplies
/// the IDENTITY. The designator's own class need NOT be the sortal (coining), so this is distinct from
/// `appose_group`'s kind-checked group apposition — the singleton, un-type-checked case.
///
/// Was `kind_of(Σ…)` at `cat_np(Entity, …)` until 2026-07-25 — a kind coerced to an entity, with the
/// classifier's class discarded. See `build_name` for why that shape cost 204 skeletons on one unit.
#[test]
fn sortal_plus_proper_name_is_a_named_individual() {
    let (_layer, index) = index_over_bootstrap();
    // `gene BRCA1` heads a finite clause as a named-individual subject.
    let closed = index.parse("gene BRCA1 affects HeLa", &Identity);
    assert!(
        !closed.is_empty(),
        "`gene BRCA1 affects HeLa` — a sortal + proper-name subject should parse"
    );
    assert!(
        closed.iter().any(|it| {
            let s = pretty_term(it.sem());
            s.contains("named(") && s.contains("the(") && !s.contains("kind_of(")
        }),
        "expected a definite-individual `the(Σ… named(…)).1` reading (NOT the old kind coercion), got: {:?}",
        closed
            .iter()
            .map(|it| pretty_term(it.sem()))
            .collect::<Vec<_>>()
    );
    // Also felicitous as an object.
    assert!(
        !index.parse("HeLa affects gene BRCA1", &Identity).is_empty(),
        "`HeLa affects gene BRCA1` — named individual in object position"
    );
}

/// D63 §8.5 — **stacked attributive adjectives** (`synthetic lethal vulnerability`). Refining an
/// already-refined noun conjoins over the **same base** (`Σx:Base. P(x) ∧ adj(x)`) rather than nesting
/// (`Σy:Σ. adj(y)`, which applied the adjective to the Σ *pair* — ill-typed, so stacked adjectives
/// didn't parse). A flat Σ keeps every adjective over the base entity.
#[test]
fn stacked_attributive_adjectives_parse() {
    // 1 attributive adjective (baseline).
    assert_parses_to_prop("HeLa is a large gene");
    // 2 stacked attributive adjectives — predicate-nominal and determiner-subject positions.
    assert_parses_to_prop("HeLa is a large primary gene");
    assert_parses_to_prop("a large primary gene affects HeLa");
}

/// D63 §8.10 — **simple past tense**: a finite past-tense verb heads a declarative clause
/// (`HeLa affected BRCA1`), and the **past copula** `was`/`were` carries a predicate nominal /
/// adjective (`HeLa was a gene`, `HeLa was large`). Past-tense finite verbs have no number agreement
/// (`num_any` subject); the copula keeps it (`was` sg / `were` pl). The WRN page is past-tense
/// narrative, so without these almost nothing on it parses. (Present-tense forms still parse.)
#[test]
fn past_tense_clauses_parse() {
    let (_layer, index) = index_over_bootstrap();
    // Finite simple-past lexical verb (demo `e_affect_fin_past`; importer `e_v…_fpast`).
    assert_parses_to_prop("HeLa affected BRCA1");
    // Past copula + predicate nominal / adjective.
    assert_parses_to_prop("HeLa was a gene");
    assert_parses_to_prop("HeLa was large");
    // `were` (plural past copula) over a bare-plural subject + predicate adjective. A bare plural
    // commits to its kind (reshape §7.4) ⇒ a CLOSED parse.
    assert!(
        !index.parse("genes were large", &PluralS).is_empty(),
        "`genes were large` — were (pl past copula) + bare-plural kind subject + predicate adjective"
    );
    // Sanity: present tense still parses.
    assert_parses_to_prop("HeLa affects BRCA1");
}

/// D63 kind-predication reshape §7.4 — bare-plural NP arguments COMMIT to the kind (Carlson 1977: a
/// bare plural denotes its kind, exactly like a bare mass noun), so the clause is a **closed** parse:
/// "genes affect HeLa" → `affect(hela, kind_of(Gene))`, no deferred `Quantification` hole. Agreement
/// still bites (a bare plural is `pl`), and a bare *singular* count noun does not shift.
#[test]
fn bare_plural_np_is_a_closed_kind_argument() {
    let (_layer, index) = index_over_bootstrap();

    // Bare-plural subject / object: each parses CLOSED, the plural nominalized to its kind.
    for s in ["genes affect HeLa", "HeLa affects genes"] {
        let closed = index.parse(s, &PluralS);
        assert_eq!(
            closed.len(),
            1,
            "'{s}': a bare plural is a single closed kind-predication"
        );
        assert!(
            pretty_term(closed[0].sem()).contains("kind_of(Gene)"),
            "'{s}': the bare plural is its kind realized as an entity, got {}",
            pretty_term(closed[0].sem())
        );
    }

    // `genes affect genes` closes with the kind in BOTH subject and object.
    let both = index.parse("genes affect genes", &PluralS);
    assert!(!both.is_empty(), "`genes affect genes` parses closed");
    assert_eq!(
        pretty_term(both[0].sem()).matches("kind_of(Gene)").count(),
        2,
        "both argument positions nominalize the kind, got {}",
        pretty_term(both[0].sem())
    );

    // (Agreement — a bare plural is `pl`, so a 3sg verb rejects it as subject — holds in the real
    // grammar via the `pl` NP num reused from `these_subj`; it is verified by
    // `subject_verb_agreement_bites`. It cannot be checked here because the test's `PluralS`
    // lemmatizer strips `-s` from `affects` → `affect`, so `affects` also seeds the plural verb.)

    // A bare SINGULAR count noun does NOT shift (core-en: pl-or-mass only).
    let (c2, o2) = index.parse_open("gene affects HeLa", &PluralS);
    assert!(
        c2.is_empty() && o2.is_empty(),
        "bare singular count noun is not an argument NP"
    );
}

/// D63 §8.5 — a **predicate nominal over an adjective-refined noun** (`HeLa is a large gene`). The
/// predicate-nominal determiner `a_pred` (`λT.λs. is_a(s,T)`) does NOT bind a predicate over the noun
/// type `T` (its body is `S[adj]\NP(Entity)`), so the GQ Fst-projection refined-noun case must NOT
/// fire for it — doing so produced an ill-formed term that applied the **subject** as a function
/// (`NotAFunction` panic in readback). Regression guard: with the Fst case gated on `tvar ∈ body`,
/// `a_pred(Σ) = λs. is_a(s, Σ)` parses cleanly, for both a proper-noun (resource) and a determiner
/// subject; and the GQ-over-refined-noun path (`every large gene …`) still works.
#[test]
fn predicate_nominal_over_refined_noun_parses() {
    let (_layer, index) = index_over_bootstrap();
    // Proper-noun (cat_np resource) subject — the shape that panicked (HeLa ≈ the WRN gene).
    assert!(
        !index.parse("HeLa is a large gene", &Identity).is_empty(),
        "resource subject + adjective-refined predicate nominal must parse (no readback panic)"
    );
    // Determiner subject — same refined predicate nominal.
    assert!(
        !index
            .parse("a cell line is a large gene", &Identity)
            .is_empty(),
        "determiner subject + adjective-refined predicate nominal must parse"
    );
    // The GQ-over-refined-noun path (Fst projection) is unaffected — `tvar` ∈ the GQ body.
    assert!(
        !index
            .parse("every large gene affects HeLa", &Identity)
            .is_empty(),
        "GQ over a refined noun still parses (Fst projection path intact)"
    );
}

// (Removed `probe_kernel_gates_a_higher_order_quantifier_typed_hole` with the kind-predication reshape
// Phase B: it was a de-risk probe for the D62 bare-plural **deferred-quantifier** hole
// (`Π(A:Set).(A→Prop)→Prop` in head position), and that hole is retired — bare plural/mass now commit to
// `kind_of(t)`. The kernel's higher-order-neutral gating it proved is untested-but-unused now; re-add a
// focused probe if a future construction (e.g. a `ProofObligation` hole) needs a head-position neutral.)

// ── D63 §8.10 Slice 6-agr — subject-verb number agreement ─────────────
#[test]
fn subject_verb_agreement_bites() {
    // A singular subject takes the 3sg verb `affects`; the plural-finite `affect`
    // (subject `pl`) is rejected — proper noun (HeLa = sg) and singular determiner alike.
    let (_layer, index) = index_over_bootstrap();
    assert_eq!(
        index.parse("HeLa affects BRCA1", &Identity).len(),
        1,
        "sg subject + 3sg verb parses (single)"
    );
    assert!(
        index.parse("HeLa affect BRCA1", &Identity).is_empty(),
        "sg subject + plural-finite verb has no parse"
    );
    assert!(
        !index
            .parse("every cell line affects HeLa", &Identity)
            .is_empty(),
        "every (sg) + 3sg verb parses"
    );
    assert!(
        index
            .parse("every cell line affect HeLa", &Identity)
            .is_empty(),
        "every (sg) + plural-finite verb has no parse (agreement bites)"
    );
    // A coordinated (plural) group takes the plural-finite verb, not the 3sg:
    // "HeLa and BRCA1 affect HeLa" ✓ (churned tests) / "… affects …" ✗ (distributive
    // num-check, D63 §8.10).
    assert!(
        index
            .parse("HeLa and BRCA1 affects HeLa", &Identity)
            .is_empty(),
        "plural group + 3sg verb has no parse (distributive agreement bites)"
    );
}

#[test]
fn auxiliary_agreement_bites() {
    // Finite auxiliaries agree with the subject: a singular subject (HeLa) takes the
    // sg aux (is/has), and the plural aux (are/have) is rejected.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        !index.parse("HeLa is affecting BRCA1", &Identity).is_empty(),
        "is (sg) + sg subject parses"
    );
    assert!(
        index
            .parse("HeLa are affecting BRCA1", &Identity)
            .is_empty(),
        "are (pl) + sg subject has no parse (aux agreement bites)"
    );
    assert!(
        !index.parse("HeLa has affected BRCA1", &Identity).is_empty(),
        "has (sg) + sg subject parses"
    );
    assert!(
        index
            .parse("HeLa have affected BRCA1", &Identity)
            .is_empty(),
        "have (pl) + sg subject has no parse"
    );
}

// ── D63 §8.11 Slice 6-cl — clausal complements ────────────────────────
#[test]
fn clausal_complement_parses_intensionally() {
    // "HeLa shows that BRCA1 affects HeLa" → shows(affects(hela, brca1), hela) : Prop.
    // The complement is NOT asserted — the sem is headed by the opaque report axiom
    // `shows`, not by `affects` (intensional context).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa shows that BRCA1 affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one clausal-complement parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("clausal sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a report clause denotes Prop"
    );
    match forest[0].sem() {
        Exp::App(f, _) => match &**f {
            Exp::App(g, _) => assert!(
                matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:lexicon:shows"),
                "clausal head is the opaque report axiom `shows`, got {g:?}"
            ),
            other => panic!("expected shows(P, subj), got {other:?}"),
        },
        other => panic!("expected a report application, got {other:?}"),
    }
}

#[test]
fn embedded_cp_is_not_a_clause_root_and_relativizer_still_works() {
    // `cat_cp` is not a clause root: a bare "that BRCA1 affects HeLa" does not parse as a
    // standalone sentence (it's an embedded complement awaiting a clause-taking verb).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("that BRCA1 affects HeLa", &Identity).is_empty(),
        "a leading complementizer-'that' clause is not a standalone sentence"
    );
    // No regression: the relativizer `that` (6-rel) still composes (distinct context:
    // noun + gapped body, not verb + full clause).
    assert!(
        !index
            .parse("every cell line that affects HeLa is primary", &Identity)
            .is_empty(),
        "the relativizer 'that' still composes after adding the complementizer"
    );
}

// ── D62 §2d — clausal subordinators ───────────────────────────────────
#[test]
fn conditional_if_builds_native_implication() {
    // "S₁ if S₂" ⇒ ⟦S₂⟧ → ⟦S₁⟧ — the conditional is the type theory's NATIVE implication
    // (an arrow between Props), NOT an opaque binary operator. So the parsed sem is headed
    // by Exp::Arrow and type-checks to Prop (felicity gate admits native implication).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 if BRCA1 affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one conditional parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("conditional sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a conditional denotes Prop"
    );
    // Native implication is a FUNCTION TYPE (`Exp::Pi` — NbE normalizes the non-dependent
    // arrow to a Pi), NOT an opaque operator application. This is the whole point: a
    // proof of the antecedent yields the consequent by ordinary function application.
    assert!(
        matches!(forest[0].sem(), Exp::Pi(_, _, _)),
        "`if` builds native implication (a Pi/arrow, not an opaque App), got {:?}",
        forest[0].sem()
    );
}

#[test]
fn contrastive_but_maps_to_conjunction() {
    // "S₁ but S₂" ⇒ And(⟦S₁⟧, ⟦S₂⟧). Verified adequate against every `but` in the WRN source
    // (all "X but (not) Y" — truth-conditionally plain conjunction; the contrast is rhetorical,
    // carried by explicit negation, and not part of the typed claim). So a but-clause denotes
    // the SAME proposition as the `and`-coordination of its clauses: headed by `logic:And`,
    // type-checks to Prop, two conjuncts in source order.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 but BRCA1 affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one but parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("but sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a but-clause denotes Prop"
    );
    let conjuncts = and_conjuncts(forest[0].sem())
        .unwrap_or_else(|| panic!("`but` maps to logic:And, got {:?}", forest[0].sem()));
    assert_eq!(
        conjuncts.len(),
        2,
        "but joins exactly two clauses as a conjunction"
    );
}

// ── D62 §2e / D64 Phase A — referential pronouns as open-parse holes ───
#[test]
fn referential_pronoun_yields_an_open_parse_with_a_hole() {
    // The open-parse carrier: a referential pronoun seeds a referent HOLE (a fresh free var),
    // so "it affects HeLa" has NO closed parse — it is an OPEN parse carrying one Entity hole,
    // type-checked (the hole bound to Entity) but awaiting the D64 resolver. A fully-referring
    // sentence stays closed with an empty open forest (no regression to the closed grammar).
    let (_layer, index) = index_over_bootstrap();

    let (closed, open) = index.parse_open("it affects HeLa", &Identity);
    assert!(
        closed.is_empty(),
        "a pronoun-subject sentence has no CLOSED parse (got {})",
        closed.len()
    );
    assert_eq!(
        open.len(),
        1,
        "exactly one open parse for 'it affects HeLa'"
    );
    assert_eq!(
        open[0].holes.len(),
        1,
        "carries exactly one referent hole, got {:?}",
        open[0].holes
    );
    assert!(
        is_ctor(open[0].item.cat(), "cat_s").is_some(),
        "the open parse is a sentence (S)"
    );

    // No regression: a fully-referring sentence is closed, with no holes.
    let (closed2, open2) = index.parse_open("HeLa affects BRCA1", &Identity);
    assert_eq!(closed2.len(), 1, "fully-referring sentence parses closed");
    assert!(
        open2.is_empty(),
        "no open parses when there are no pronouns"
    );
}

#[test]
fn two_pronoun_occurrences_are_two_distinct_holes() {
    // Per-occurrence identity (the point of a hole vs. a shared constant): "it affects it"
    // carries TWO distinct referent holes.
    let (_layer, index) = index_over_bootstrap();
    let (closed, open) = index.parse_open("it affects it", &Identity);
    assert!(
        closed.is_empty(),
        "no closed parse (both arguments are holes)"
    );
    assert!(
        !open.is_empty(),
        "at least one open parse for 'it affects it'"
    );
    let holes = &open[0].holes;
    assert_eq!(holes.len(), 2, "two distinct referent holes");
    assert_ne!(
        holes[0].var, holes[1].var,
        "the two holes are distinct (per-occurrence identity)"
    );
}

#[test]
fn deictic_we_is_a_closed_referring_np() {
    // `we` is deictic, not anaphoric: it denotes the author(s) via the fixed `lexicon:speaker`
    // constant, so "we affect HeLa" is a CLOSED parse (no hole) — unlike `it`/`they`.
    let (_layer, index) = index_over_bootstrap();
    let (closed, open) = index.parse_open("we affect HeLa", &Identity);
    assert_eq!(closed.len(), 1, "deictic `we` yields one closed parse");
    assert!(
        open.is_empty(),
        "`we` introduces no referent hole (deictic, not anaphoric)"
    );
}

#[test]
fn possessive_determiner_yields_an_open_parse_with_a_possessor_hole() {
    // `its`/`their` are possessive DETERMINERS carrying an anaphoric possessor hole nested in
    // the determiner λ. "its gene affects HeLa" ⇒ ∃x:Gene. poss_of(x, ?ref) ∧ affects(hela, x)
    // — an OPEN parse with one possessor hole, type-checked (the hole bound to Entity).
    let (_layer, index) = index_over_bootstrap();
    let (closed, open) = index.parse_open("its gene affects HeLa", &Identity);
    assert!(
        closed.is_empty(),
        "a possessive-headed subject has no closed parse"
    );
    assert!(
        !open.is_empty(),
        "at least one open parse for 'its gene affects HeLa'"
    );
    assert_eq!(
        open[0].holes.len(),
        1,
        "exactly one possessor hole, got {:?}",
        open[0].holes
    );
    assert!(
        is_ctor(open[0].item.cat(), "cat_s").is_some(),
        "the open parse is a sentence (S)"
    );

    // The possessive works in OBJECT position too: "HeLa affects its gene" ⇒ an open parse
    // with one possessor hole (its_obj, the object determiner shape).
    let (closed_o, open_o) = index.parse_open("HeLa affects its gene", &Identity);
    assert!(closed_o.is_empty(), "object possessive has no closed parse");
    assert_eq!(
        open_o.len(),
        1,
        "one open parse for 'HeLa affects its gene'"
    );
    assert_eq!(
        open_o[0].holes.len(),
        1,
        "one possessor hole (object position)"
    );

    // `their` (plural) works too — with a plural-aware lemmatizer (so the plural noun `genes`
    // reduces to the lexicon form `gene` and is marked pl; the `Identity` lemmatizer above does
    // not, which is why these use `PluralS`). "their genes affect HeLa" ⇒ an open parse with one
    // possessor hole; the plural determiner `all` likewise composes ("all genes affect HeLa").
    let (closed_t, open_t) = index.parse_open("their genes affect HeLa", &PluralS);
    assert!(
        closed_t.is_empty(),
        "their-possessive subject has no closed parse"
    );
    assert_eq!(
        open_t.len(),
        1,
        "one open parse for 'their genes affect HeLa'"
    );
    assert_eq!(open_t[0].holes.len(), 1, "one possessor hole for `their`");
    let (closed_all, open_all) = index.parse_open("all genes affect HeLa", &PluralS);
    assert_eq!(
        closed_all.len(),
        1,
        "plural determiner `all` composes (closed)"
    );
    assert!(open_all.is_empty(), "`all` introduces no hole");
}

#[test]
fn resolve_open_substitutes_an_antecedent_and_re_gates() {
    // The trusted resolve primitive (D64 §4): substitute a hole with a proposed chain antecedent
    // and re-gate. "it affects HeLa" + (?ref := BRCA1) ⇒ a CLOSED parse affects(hela, brca1).
    let (layer, index) = index_over_bootstrap();
    let (_c, open) = index.parse_open("it affects HeLa", &Identity);
    assert_eq!(open.len(), 1, "one open parse");
    let hole = open[0].holes[0].var.clone();

    // antecedent = the committed chain entity BRCA1 (a Gene <: Entity).
    let brca1 = layer
        .resolve(&Iri::parse("urn:eigenius:lexicon:brca1").unwrap())
        .expect("brca1 resolves");
    let ante = Exp::EigonResource(Box::new((*brca1).clone()));
    let resolved = index
        .resolve_open(&open[0], &[(hole, ante)])
        .expect("resolves to a closed parse");

    // The resolved parse is closed and type-checks to Prop (re-gated by the kernel).
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, resolved.sem()).expect("resolved sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the resolved parse denotes Prop"
    );

    // Fail-closed: an UNRESOLVED hole (no binding) leaves a free var ⇒ no closed parse.
    assert!(
        index.resolve_open(&open[0], &[]).is_none(),
        "an unresolved hole fails closed (no binding ⇒ no admissible parse)"
    );
}

/// A deterministic mock [`Proposer`]: picks the in-scope candidate whose surface form equals the
/// target (a ranked single-element list), else proposes nothing (unresolvable). Stands in for the
/// LLM proposer to exercise the resolve loop without an LLM.
struct PickBySurface(&'static str);
impl Proposer for PickBySurface {
    fn propose(&self, ctx: &ProposeCtx) -> Vec<eigenius_kernel::ontology::Iri> {
        ctx.candidates
            .iter()
            .filter(|c| c.surface == self.0)
            .map(|c| c.iri.clone())
            .collect()
    }
}

#[test]
fn resolve_loop_with_mock_proposer_resolves_and_fails_closed() {
    // The resolve loop (D64 §4) with a deterministic mock proposer: candidates → propose →
    // resolve_open (kernel re-gate) → closed parse, or fail-closed when unresolvable.
    let (layer, index) = index_over_bootstrap();
    let (_c, open) = index.parse_open("it affects HeLa", &Identity);
    assert_eq!(open.len(), 1, "one open parse");
    let candidates = vec![
        Candidate {
            iri: Iri::parse("urn:eigenius:lexicon:brca1").unwrap(),
            surface: "BRCA1".into(),
        },
        Candidate {
            iri: Iri::parse("urn:eigenius:lexicon:hela").unwrap(),
            surface: "HeLa".into(),
        },
    ];

    // The mock proposes BRCA1 → the loop resolves it through the kernel to a closed Prop.
    let resolved = index
        .resolve_with(
            &open[0],
            "it affects HeLa",
            &candidates,
            &PickBySurface("BRCA1"),
        )
        .expect("mock-proposed antecedent resolves through the kernel re-gate");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, resolved.sem()).expect("resolved sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the resolved parse denotes Prop"
    );

    // A proposer that suggests nothing ⇒ fail closed (no committed parse).
    assert!(
        index
            .resolve_with(
                &open[0],
                "it affects HeLa",
                &candidates,
                &PickBySurface("NONE")
            )
            .is_none(),
        "an unresolvable hole fails closed"
    );
}

#[test]
fn resolve_document_threads_discourse_across_sentences() {
    // Stage C end-to-end (D64 §4, the discourse resolve loop): a 2-sentence document. Sentence 1
    // (closed) introduces HeLa + BRCA1; sentence 2 "it affects HeLa" carries a referent hole.
    // `resolve_document` harvests sentence 1's entities into the candidate set and hands them to the
    // proposer — WITHOUT the caller supplying candidates by hand (that discourse threading is the new
    // piece) — the mock binds `it` and the kernel re-gates it to a closed Prop.
    let (_layer, index) = index_over_bootstrap();
    let doc = ["HeLa affects BRCA1", "it affects HeLa"];

    let resolved = index.resolve_document(&doc, &Identity, &PickBySurface("brca1"));
    assert_eq!(resolved.len(), 2);
    assert!(
        matches!(
            resolved[0],
            SentenceOutcome::Encoded(_) | SentenceOutcome::Ambiguous(_)
        ),
        "sentence 1 parses closed"
    );
    let SentenceOutcome::Encoded(s2) = &resolved[1] else {
        panic!("sentence 2's pronoun should resolve to a single closed prop");
    };
    assert!(
        pretty_term(s2.sem()).contains("brca1"),
        "`it` bound to a prior-sentence entity (brca1): {}",
        pretty_term(s2.sem())
    );

    // Fail-closed: a proposer that finds no antecedent ⇒ the sentence stays Open, not a wrong closed parse.
    let none = index.resolve_document(&doc, &Identity, &PickBySurface("nonexistent"));
    assert!(
        matches!(none[1], SentenceOutcome::Open(_)),
        "an unresolvable pronoun stays Open (fail-closed)"
    );
}

#[test]
fn document_only_augmentation_harvests_bindings_and_flags_oov() {
    // D63 lexicon-augmentation Phase 1 (`DocumentOnly`): the document's own abbreviation definition is
    // harvested as a grounded `LexicalBinding` (method `DefinitionExtracted`, provenance kept), and an
    // unknown token is a fail-closed `Gap` — never silently dropped.
    use eigenius_kernel::dcg::{augment_document_only, ResolutionMethod};
    let (base, _index) = index_over_bootstrap();
    let doc = "The instability (INS) was assayed. INS affects HeLa. zzqxword affects HeLa.";
    let aug = augment_document_only(&base, doc, &NoAbbreviationProposer, &Identity);

    // Harvest: INS → a grounded binding wrapping a proposed `lexicon:LexicalEntry`, with provenance.
    let ins = aug
        .added
        .iter()
        .find(|b| b.provenance.surface == "INS")
        .expect("INS harvested as a binding");
    assert_eq!(ins.provenance.long_form.as_deref(), Some("instability"));
    assert_eq!(ins.provenance.method, ResolutionMethod::DefinitionExtracted);
    assert!(
        ins.provenance.grounded_to.is_some(),
        "INS grounded to the mass concept"
    );
    let entry_class = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    assert!(
        ins.proposed.is_instance_of(&entry_class),
        "the proposed resource is a lexicon:LexicalEntry"
    );

    // Fail closed: the unknown token is a `Gap`; the commit set carries at least the INS entry.
    assert!(
        aug.missing_oov
            .iter()
            .any(|g| g.surface.to_lowercase() == "zzqxword"),
        "the OOV token `zzqxword` is a Gap"
    );
    assert!(
        !aug.resources().is_empty(),
        "the augmentation commits at least the INS entry"
    );
}

/// A concept + a **multiword** form entry aliasing it — the snapshot shape in miniature (C0084304's atoms
/// are multiword entries; the exact index can't token-match `recq`, but the bootstrap's `form_text_index`
/// BM25 index can). The form `core:TextIndex` comes from the lexicon ontology (one per property), not here.
const RECQ_FORM_INDEX_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
class lexicon:RecqHelicases : lexicon:Entity {
    description = "The RecQ helicase family (test concept, mirrors UMLS C0084304).";
}
resource lexicon:e_recq_family : lexicon:LexicalEntry {
    lexicon:form     = "recq family of dna helicases";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:RecqHelicases, lexicon:num_any) );
    lexicon:sem      = lexicon:RecqHelicases;
    lexicon:sem_type = type_expr( Set );
    lexicon:grade    = epistemic:declared;
}
"#;

#[test]
fn lexicon_backed_augmentation_grounds_oov_via_the_form_text_index() {
    // D63 lexicon-augmentation Phase 2 (`LexiconBacked`, §6a): a form `core:TextIndex` token-matches the OOV
    // surface `recq` to a seeded multiword atom → grounds it to that concept, in-process. The exact
    // `ValueIndex` misses it (`recq` ≠ `recq family of dna helicases`); the BM25 `TextIndex` closes it.
    use eigenius_kernel::dcg::{augment_lexicon_backed, NominalCategoryProposer, ResolutionMethod};
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    // One shared storage across the chain so the bootstrap's `form_text_index` (discovered via the
    // per-storage triple index) is visible to the recq layer — as in production's single backend.
    let storage = ctx.head().storage().clone();
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(storage.clone()));
    let fix = esl::compile_against_layer(RECQ_FORM_INDEX_FIXTURE, &demo_layer)
        .expect("recq fixture compiles");
    let mut b2 = LayerBuilder::new("recq", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add recq fixture");
    }
    let base = Arc::new(b2.build(storage));

    // Sanity: bare `recq` is OOV under the exact index (has_token=false), exactly as on the snapshot.
    let index = Parser::build(Arc::clone(&base));
    assert!(
        !index.has_token("recq", &Identity),
        "bare `recq` is OOV under the exact form index"
    );

    // LexiconBacked grounds it via the form TextIndex → a RetrievalGrounded binding aliasing the concept.
    let aug = augment_lexicon_backed(
        &base,
        "recq affects HeLa.",
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &Identity,
    );
    let recq = aug
        .added
        .iter()
        .find(|b| b.provenance.surface.to_lowercase() == "recq")
        .expect("`recq` grounded via retrieval");
    assert_eq!(recq.provenance.method, ResolutionMethod::RetrievalGrounded);
    assert_eq!(
        recq.provenance.grounded_to.as_ref().map(|i| i.as_str()),
        Some("urn:eigenius:lexicon:RecqHelicases"),
        "grounded to the family concept"
    );
    assert!(
        !aug.missing_oov
            .iter()
            .any(|g| g.surface.to_lowercase() == "recq"),
        "`recq` moved out of missing_oov"
    );
}

#[test]
fn probe_recq_form_index_active_and_populated() {
    use eigenius_kernel::layer::resolve_active_text_indexes;
    use eigenius_kernel::query::text::analyzer::registry::analyzer_for;
    use eigenius_kernel::query::text::search::run_text_search;
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    // Build the whole chain on ONE storage (the bootstrap's) — index discovery scans the
    // per-storage triple index, so the bootstrap's form_text_index is only visible to a child
    // layer built on the same storage. This mirrors production, where a chain lives on a single
    // backend; a per-layer `LayerStorage::in_memory()` would hide the inherited index.
    let storage = ctx.head().storage().clone();
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(storage.clone()));
    let fix = esl::compile_against_layer(RECQ_FORM_INDEX_FIXTURE, &demo_layer)
        .expect("recq fixture compiles");
    let mut b2 = LayerBuilder::new("recq", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add recq fixture");
    }
    let base = Arc::new(b2.build(storage));
    let active = resolve_active_text_indexes(&base);
    eprintln!("ACTIVE text indexes over base: {}", active.len());
    for a in &active {
        eprintln!(
            "  idx={} target={} analyzer={}",
            a.iri.as_str(),
            a.target_property.as_str(),
            a.analyzer
        );
    }
    let form_prop =
        eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:lexicon:form").unwrap();
    let idx = active
        .iter()
        .find(|a| a.target_property == form_prop)
        .expect("a form text index is active");
    let analyzer = analyzer_for(&idx.analyzer).expect("analyzer");
    let hits = run_text_search(
        &base,
        base.storage().text_index.as_ref(),
        &idx.iri,
        analyzer.as_ref(),
        "recq",
    )
    .expect("search ok");
    eprintln!("HITS for 'recq': {}", hits.len());
    for h in &hits {
        eprintln!("  subj={} score={}", h.subject.as_str(), h.score);
    }
    assert!(!hits.is_empty(), "recq should hit e_recq_family");
}

/// A nominal concept, a verb **axiom** (with a committed sibling entry), and a `core:description` on each
/// mentioning `supercoils`. The SAME OOV surface must ground to *different* concepts by the proposed POS
/// (§6a, the (B) step): a **nominal** OOV → the class `demo:Gyrase` (the form path skips the axiom, the
/// gloss path keeps the class); a **verb** OOV → the axiom `demo:v_supercoil`, minted with the sibling's
/// verb cat (not `cat_n`). `supercoils` stems to the sibling form `supercoil`, so the form text index
/// reaches the axiom — which the POS filter, not ranking, admits or rejects. The sibling's cat is the
/// transitive base `(S[dcl,bse]\NP)/NP` (verbatim from the WordNet converter) so the minter has a real
/// verb cat to clone.
const DESCRIPTION_GROUNDING_FIXTURE: &str = r#"
namespace demo      = "urn:eigenius:demo";
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
class demo:Gyrase : lexicon:Entity {
    description = "a bacterial topoisomerase enzyme that supercoils chromosomal dna in living cells";
}
axiom demo:v_supercoil : lexicon:Entity -> lexicon:Entity -> Prop desc: "supercoils dna"
resource demo:e_supercoil : lexicon:LexicalEntry {
    lexicon:form     = "supercoil";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:bse), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = demo:v_supercoil;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:grade    = epistemic:declared;
}
"#;

/// Build `bootstrap → demo → fixture` on one shared storage (so the core `description_text_index` is
/// discovered over `base`), returning the fixture head.
fn description_grounding_base() -> Arc<eigenius_kernel::layer::Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let storage = ctx.head().storage().clone();
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(storage.clone()));
    let fix = esl::compile_against_layer(DESCRIPTION_GROUNDING_FIXTURE, &demo_layer)
        .expect("description fixture compiles");
    let mut b2 = LayerBuilder::new("fixture", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add description fixture");
    }
    Arc::new(b2.build(storage))
}

#[test]
fn lexicon_backed_augmentation_grounds_nominal_oov_to_class_not_axiom() {
    use eigenius_kernel::dcg::{augment_lexicon_backed, NominalCategoryProposer, ResolutionMethod};
    let base = description_grounding_base();

    // `supercoils` is OOV under the exact index (Identity; the form is `supercoil`).
    let index = Parser::build(Arc::clone(&base));
    assert!(
        !index.has_token("supercoils", &Identity),
        "supercoils is OOV under the exact form index"
    );

    // Nominal POS: the form path reaches the axiom (via the stemmed sibling) but the POS filter drops it;
    // the gloss path keeps the class → grounds to demo:Gyrase, minted a nominal alias.
    let aug = augment_lexicon_backed(
        &base,
        "supercoils affects HeLa.",
        &NoAbbreviationProposer,
        &NominalCategoryProposer,
        &Identity,
    );
    let g = aug
        .added
        .iter()
        .find(|b| b.provenance.surface.to_lowercase() == "supercoils")
        .expect("supercoils grounded (nominal)");
    assert_eq!(g.provenance.method, ResolutionMethod::RetrievalGrounded);
    assert_eq!(
        g.provenance.grounded_to.as_ref().map(|i| i.as_str()),
        Some("urn:eigenius:demo:Gyrase"),
        "nominal POS grounds to the class, NOT the verb axiom demo:v_supercoil"
    );
}

#[test]
fn lexicon_backed_augmentation_grounds_verb_oov_to_axiom_with_verb_cat() {
    use eigenius_kernel::dcg::{augment_lexicon_backed, CategoryProposer, ExpectedCat};
    use eigenius_kernel::ontology::iri::Iri;
    // The (untrusted) POS proposer names every OOV a verb — the (B) source, deterministic here.
    struct AlwaysVerb;
    impl CategoryProposer for AlwaysVerb {
        fn propose_category(&self, _surface: &str, _context: &str) -> Option<ExpectedCat> {
            Some(ExpectedCat::Verb)
        }
    }
    let base = description_grounding_base();

    // SAME OOV surface as the nominal test — but Verb POS grounds it to the AXIOM (predicate denotation),
    // and the minter clones the sibling's verb cat (not cat_n).
    let aug = augment_lexicon_backed(
        &base,
        "supercoils affects HeLa.",
        &NoAbbreviationProposer,
        &AlwaysVerb,
        &Identity,
    );
    let g = aug
        .added
        .iter()
        .find(|b| b.provenance.surface.to_lowercase() == "supercoils")
        .expect("supercoils grounded (verb)");
    assert_eq!(
        g.provenance.grounded_to.as_ref().map(|i| i.as_str()),
        Some("urn:eigenius:demo:v_supercoil"),
        "verb POS grounds to the axiom, NOT the class demo:Gyrase"
    );
    // The minted alias carries the sibling's verb cat + names the axiom — not a nominal `cat_n`.
    let cat_prop = Iri::parse("urn:eigenius:lexicon:cat").unwrap();
    let sem_prop = Iri::parse("urn:eigenius:lexicon:sem").unwrap();
    let sib = base
        .resolve(&Iri::parse("urn:eigenius:demo:e_supercoil").unwrap())
        .unwrap();
    assert_eq!(
        g.proposed.get(&cat_prop),
        sib.get(&cat_prop),
        "minted verb entry reuses the sibling's verb cat"
    );
    assert_eq!(
        g.proposed.get(&sem_prop),
        Some(&eigenius_kernel::ontology::resource::Value::ResourceRef(
            Iri::parse("urn:eigenius:demo:v_supercoil").unwrap()
        )),
        "minted verb entry's sem IS the axiom"
    );
}

/// Live-LLM smoke test for the (B) POS source: the `AnthropicCategoryProposer` tags a word by its role
/// in the sentence — a verb as `Verb`, a noun as `Nominal`. Non-deterministic (a live model), so it is
/// `#[ignore]`d and asserts only the clear cases. Run:
///   ANTHROPIC_API_KEY=… cargo test -p eigenius-kernel --features use-llm --test closed_class_determiners \
///       anthropic_category_proposer_tags_pos -- --ignored --nocapture
#[cfg(feature = "use-llm")]
#[test]
#[ignore = "live LLM POS proposer; --features use-llm --ignored --nocapture"]
fn anthropic_category_proposer_tags_pos_in_context() {
    use eigenius_kernel::dcg::{AnthropicCategoryProposer, CategoryProposer, ExpectedCat};
    let Some(p) = AnthropicCategoryProposer::from_env() else {
        eprintln!("SKIP: ANTHROPIC_API_KEY unset");
        return;
    };
    let sentence = "The kinase phosphorylates the substrate protein.";
    let verb = p.propose_category("phosphorylates", sentence);
    let noun = p.propose_category("kinase", sentence);
    eprintln!("phosphorylates → {verb:?}  |  kinase → {noun:?}");
    assert_eq!(
        verb,
        Some(ExpectedCat::Verb),
        "a verb in context tags as Verb"
    );
    assert_eq!(
        noun,
        Some(ExpectedCat::Nominal),
        "a noun in context tags as Nominal"
    );
}

#[test]
fn in_process_pipeline_encodes_a_document_end_to_end() {
    // The WHOLE pipeline in one `encode()` call (the `DocumentPipeline` contract): Stage A (glossary —
    // `instability (INS)` → INS grounded to the mass concept) → Stage B (bare `INS` closes via the kind
    // shift) → Stage C (`it` resolves against the threaded discourse). Deterministic: the
    // `NoAbbreviationProposer` for extraction and a mock `PickBySurface` for anaphora stand in for the
    // LLM steps (Phase 2 swaps those impls, same contract).
    let (base, _index) = index_over_bootstrap();
    let doc = "The instability (INS) was assayed. INS affects HeLa. it affects HeLa.";
    let pipeline = InProcessPipeline::new(
        Arc::clone(&base),
        &Identity,
        &NoAbbreviationProposer,
        &PickBySurface("hela"),
    );
    let enc = pipeline.encode(doc);

    // Stage A — the abbreviation was harvested as a grounded binding in the document augmentation.
    assert!(
        enc.augmentation
            .added
            .iter()
            .any(|b| b.provenance.surface == "INS"
                && b.provenance.long_form.as_deref() == Some("instability")),
        "Stage A harvested `INS ← instability` as a grounded binding"
    );
    assert_eq!(enc.sentences.len(), 3, "three body sentences");

    // Stage B — bare `INS` (glossary mass alias) parses closed; the Encoded reading is the grounded kind.
    let ins = find_sentence(&enc.sentences, "INS affects");
    match &ins.outcome {
        SentenceOutcome::Encoded(item) => assert!(
            pretty_term(item.sem()).contains("kind_of(Instability)"),
            "bare INS is the grounded kind: {}",
            pretty_term(item.sem())
        ),
        SentenceOutcome::Ambiguous(_) => {} // multiple closed readings — still parsed, acceptable
        SentenceOutcome::Open(_) | SentenceOutcome::Gap => {
            panic!("`INS affects HeLa` should parse closed (Encoded/Ambiguous), got Open/Gap")
        }
    }

    // Stage C — `it` resolves against the discourse (HeLa introduced by the prior sentence) and encodes.
    let it = find_sentence(&enc.sentences, "it affects");
    assert!(
        matches!(it.outcome, SentenceOutcome::Encoded(_)),
        "`it affects HeLa` resolves its pronoun against the discourse and encodes"
    );
}

#[test]
fn non_pp_verb_rejects_a_pp_complement() {
    // Over-generation guard for the PP-frame fix (docs/notes/d63-parse-gap-closure.md Step 2). A
    // genuinely transitive verb `(S\NP)/NP` (WordNet frame 8, "----s something") takes a *bare NP* and
    // MUST NOT absorb a `prep + NP`: `*HeLa affects to BRCA1` is ungrammatical and correctly gaps.
    // The Step-2 fix gives *PP-oblique-frame* verbs a `(S\NP)/cat_pp` category so "contributes to
    // cancers" parses — it must be FRAME-SPECIFIC and must NOT blanket-license "V prep NP", or it would
    // wrongly admit `*affects to BRCA1`. (`convert.rs::classify` already distinguishes frame 8 from the
    // PP-oblique frames 12/13/20/21/27, so the fix can be precise.) This test must stay green after it.
    let (_layer, index) = index_over_bootstrap();
    // Clean transitive: a bare NP object closes.
    assert_eq!(index.parse("HeLa affects BRCA1", &Identity).len(), 1);
    // A non-PP verb + `prep + NP` where a bare object is expected → correctly no parse.
    assert!(index.parse("HeLa affects to BRCA1", &Identity).is_empty());
    // A PP still adjoins once the verb has its bare object (both attachments → ≥1 parse).
    assert!(!index
        .parse("HeLa affects BRCA1 in HeLa", &Identity)
        .is_empty());
}

/// A verb that subcategorizes for a PP — `contributes : (S\NP)/cat_pp_arg(prep_any)` — reusing the demo
/// `affects` binary axiom as its relation. The Step-2 fix in miniature (the importer emits this shape for
/// PP-oblique WordNet frames). `prep_any` (the preposition-agnostic verb frame) accepts any marker.
const CONTRIB_FIXTURE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
resource lexicon:e_contributes : lexicon:LexicalEntry {
    lexicon:form     = "contributes";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_pp_arg(lexicon:prep_any)) );
    lexicon:sem      = lexicon:affects;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wn:contribute.v.01";
    lexicon:grade    = epistemic:declared;
}
"#;

fn contrib_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in demo {
        b.add_resource(r).expect("add demo");
    }
    let demo_layer = Arc::new(b.build(LayerStorage::in_memory()));
    let fix =
        esl::compile_against_layer(CONTRIB_FIXTURE, &demo_layer).expect("contrib fixture compiles");
    let mut b2 = LayerBuilder::new("contrib", Some(Arc::clone(&demo_layer)));
    for r in fix {
        b2.add_resource(r).expect("add contrib");
    }
    Arc::new(b2.build(LayerStorage::in_memory()))
}

#[test]
fn argument_pp_verb_parses_verb_prep_object() {
    // The Step-2 fix in miniature (docs/notes/d63-parse-gap-closure.md): a verb subcategorizing for a PP
    // (`contributes : (S\NP)/cat_pp_arg`) composes with "to <object>" (the argument-marker
    // `to : cat_pp_arg/NP`), while a plain transitive verb still rejects a stray "to X".
    let index = Parser::build(contrib_layer());

    // Argument-PP verb + "to <individual>" → parses; sem reuses the `affects` axiom.
    let closed = index.parse("HeLa contributes to BRCA1", &Identity);
    assert!(
        !closed.is_empty(),
        "`HeLa contributes to BRCA1` should parse (argument-PP verb)"
    );
    assert!(
        pretty_term(closed[0].sem()).contains("affects"),
        "sem should apply the verb's binary relation: {}",
        pretty_term(closed[0].sem())
    );

    // Felicity guard: the argument-marker does NOT leak to a plain transitive verb.
    assert!(index.parse("HeLa affects to BRCA1", &Identity).is_empty());

    // Kind object (bare plural → kind_of), the acceptance shape "MSI contributes to cancers": the
    // argument-marker feeds a *raised* GQ through the extended GqPrepObj rule. Needs `PluralS` to
    // lemmatize "genes" → "gene".
    let kind = index.parse("HeLa contributes to genes", &PluralS);
    assert!(
        !kind.is_empty(),
        "`HeLa contributes to genes` (bare-plural kind object) should parse"
    );
    assert!(
        pretty_term(kind[0].sem()).contains("kind_of"),
        "the object is the kind: {}",
        pretty_term(kind[0].sem())
    );

    // A MASS/kind subject composes too (not just an individual) — the shape of "MSI contributes to
    // cancers" once MSI is grounded to a mass concept by the Stage-A glossary.
    assert!(
        !index
            .parse("instability contributes to genes", &PluralS)
            .is_empty(),
        "a bare mass subject + argument-PP verb should parse"
    );
}

fn find_sentence<'a>(sentences: &'a [SentenceEncoding], needle: &str) -> &'a SentenceEncoding {
    sentences
        .iter()
        .find(|s| s.text.contains(needle))
        .unwrap_or_else(|| panic!("no sentence matching {needle:?}"))
}

#[cfg(feature = "use-llm")]
#[test]
fn live_anthropic_proposer_resolves_a_referent_through_the_kernel() {
    // The live-LLM resolver path (D64 §4), behind the `use-llm` feature: a real Anthropic model
    // proposes an antecedent for the referent hole, and the kernel re-gates it to a closed Prop.
    // Skips cleanly if no key is set; runs live when ANTHROPIC_API_KEY is present.
    use eigenius_kernel::dcg::resolver_llm::AnthropicProposer;
    let Some(proposer) = AnthropicProposer::from_env() else {
        eprintln!("SKIP live_anthropic_proposer: ANTHROPIC_API_KEY unset");
        return;
    };
    let (layer, index) = index_over_bootstrap();
    let (_closed, open) = index.parse_open("it affects HeLa", &Identity);
    assert_eq!(open.len(), 1, "one open parse with one referent hole");
    let candidates = vec![
        Candidate {
            iri: Iri::parse("urn:eigenius:lexicon:brca1").unwrap(),
            surface: "BRCA1".into(),
        },
        Candidate {
            iri: Iri::parse("urn:eigenius:lexicon:hela").unwrap(),
            surface: "HeLa".into(),
        },
    ];
    let resolved = index
        .resolve_with(&open[0], "it affects HeLa", &candidates, &proposer)
        .expect("live LLM proposes an antecedent the kernel re-gates to a closed Prop");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, resolved.sem()).expect("resolved sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the live-resolved parse denotes Prop"
    );
}

/// A `-s`-stripping lemmatizer (`genes` → `gene`, marking the surface plural), enough to
/// exercise plural common nouns in this crate's tests without the full WordNet Morphy. The
/// `Identity` lemmatizer used elsewhere does not reduce plurals, so a plural surface never
/// matches its singular lexicon form.
struct PluralS;
impl eigenius_kernel::dcg::Lemmatizer for PluralS {
    fn lemmas(&self, surface: &str, _pos: eigenius_kernel::dcg::Pos) -> Vec<String> {
        let s = surface.trim().to_lowercase();
        match s.strip_suffix('s') {
            Some(b) if !b.is_empty() => vec![s.clone(), b.to_string()],
            _ => vec![s],
        }
    }
}

// ── D63 §8.12 Slice 6-cmp — comparatives (degree semantics) ───────────
/// Whether `sem` is headed by the opaque float ordering `measurements:gt`.
fn is_gt_headed(sem: &Exp) -> bool {
    match sem {
        Exp::App(f, _) => matches!(&**f, Exp::App(g, _)
            if matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:measurements:gt")),
        _ => false,
    }
}

#[test]
fn comparative_compares_degrees() {
    // "HeLa is larger than BRCA1" → gt(deg_large(hela), deg_large(brca1)) : Prop — the
    // comparative compares the two entities' degrees via the opaque float ordering.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is larger than BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one comparative parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("comparative sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "comparative denotes Prop"
    );
    assert!(
        is_gt_headed(forest[0].sem()),
        "comparative is headed by measurements:gt, got {:?}",
        forest[0].sem()
    );
}

#[test]
fn positive_gradable_adjective_is_measure_based() {
    // "HeLa is large" → gt(deg_large(hela), std_large) : Prop — the positive unified with
    // the comparative under one measure (combo 1).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is large", &Identity);
    assert_eq!(forest.len(), 1, "exactly one positive parse");
    assert!(
        is_gt_headed(forest[0].sem()),
        "the measure-based positive is headed by measurements:gt, got {:?}",
        forest[0].sem()
    );
    assert_parses_to_prop("HeLa is large");
}

#[test]
fn comparative_requires_than() {
    // `cat_pp_than` forces the `than` marker: a bare NP standard `*HeLa is larger BRCA1`
    // has no parse.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is larger BRCA1", &Identity).is_empty(),
        "the comparative requires the `than` marker (no bare-NP standard)"
    );
}

#[test]
fn phrasal_comparative_compares_measure_degrees() {
    // D63 §8.12 phrasal comparative (d63-comparative-phrasal.md): "X <v> greater dependence on Y than Z"
    // → gt(μ_dep(Y)(x), μ_dep(Y)(z)). The comparative-quantifier determiner `greater` selects the measure
    // noun `dependence` (cat_measure), absorbs the light verb, and threads the pending `than` as the
    // `/cat_pp_than` on the resulting VP.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse(
        "HeLa affects greater dependence on BRCA1 than MSH2",
        &Identity,
    );
    assert!(!forest.is_empty(), "phrasal comparative parses");
    // EXACT denotation, not merely gt-headed: subject x=hela, than-object y=msh2, ground=brca1, in
    // the right order — a swapped `gt(μ(msh2), μ(hela))` MUST fail this.
    let expected = "gt(deg_dependent(brca1, hela), deg_dependent(brca1, msh2))";
    assert!(
        forest.iter().any(|p| pretty_term(p.sem()) == expected),
        "expected exact `{expected}` among parses, got: {:?}",
        forest
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("phrasal comparative sem type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "denotes Prop");

    // `less` (degree-LESS over cat_measure) parses over the same machinery. `fewer` is now count-only
    // (over cat_n, added in A2), so `*fewer dependence` is ungrammatical (asserted in the A2 test).
    assert!(
        !index
            .parse("HeLa affects less dependence on BRCA1 than MSH2", &Identity)
            .is_empty(),
        "less phrasal comparative parses"
    );
    // Restriction: `greater` selects `cat_measure`, so a non-measure noun gets no comparative parse.
    assert!(
        index
            .parse("HeLa affects greater gene on BRCA1 than MSH2", &Identity)
            .is_empty(),
        "`greater` rejects a non-measure noun (`gene` is cat_n, not cat_measure)"
    );
}

#[test]
fn cardinality_comparative_over_a_count_noun() {
    // #9 (d63-comparative-phrasal.md §5.1): `fewer`/`more` are CARDINALITY operators over any count
    // noun `cat_n` — the noun is consumed by the cat_forall+cat_n rule and counted by the opaque
    // `card : Set → Entity → float`. `*fewer dependence` (a cat_measure, not cat_n) has no parse;
    // `less dependence` (degree over cat_measure) is the scalar counterpart.
    let (layer, index) = index_over_bootstrap();

    // fewer (x has FEWER than y): gt(card(T,y), card(T,x)), x=subject=hela, y=than-obj=msh2.
    let fewer = index.parse("HeLa affects fewer genes than MSH2", &PluralS);
    assert!(
        fewer
            .iter()
            .any(|p| pretty_term(p.sem()) == "gt(card(Gene, msh2), card(Gene, hela))"),
        "fewer over a count noun compares cardinalities: {:?}",
        fewer
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, fewer[0].sem()).expect("cardinality sem type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "denotes Prop");

    // more (x has MORE than y): gt(card(T,x), card(T,y)).
    let more = index.parse("HeLa affects more genes than MSH2", &PluralS);
    assert!(
        more.iter()
            .any(|p| pretty_term(p.sem()) == "gt(card(Gene, hela), card(Gene, msh2))"),
        "more over a count noun: {:?}",
        more.iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );

    // Agreement: `*fewer dependence` — dependence is cat_measure, not cat_n → no parse.
    assert!(
        index
            .parse("HeLa affects fewer dependence on BRCA1 than MSH2", &PluralS)
            .is_empty(),
        "*fewer dependence has no parse (fewer selects cat_n; dependence is cat_measure)"
    );

    // Compound count noun composes (KindCompound) before the count — the #9 shape (`deletion
    // mutations`); here the demo's `gene cell lines` N-N compound.
    assert!(
        !index
            .parse("HeLa affects fewer gene cell lines than MSH2", &PluralS)
            .is_empty(),
        "fewer over a COMPOUND count noun parses (KindCompound feeds the cardinality)"
    );
}

#[test]
fn adjectival_comparative_and_nominal_share_one_scale() {
    // #8 (d63-comparative-phrasal.md §5.2/§5.5b): analytic `more`/`less` over a gradable ADJECTIVE's
    // `deg_A`, predicative frame `((S[adj]\NP)/cat_pp_than)/cat_measure`. A4: the adjective `dependent`
    // and the noun `dependence` share ONE scale `deg_dependent`, so `more dependent on WRN` (adjective)
    // and `greater dependence on WRN` (noun) denote IDENTICALLY.
    let (layer, index) = index_over_bootstrap();
    let both = "gt(deg_dependent(brca1, hela), deg_dependent(brca1, msh2))";

    // adjectival: "HeLa is more dependent on BRCA1 than MSH2".
    let more_adj = index.parse("HeLa is more dependent on BRCA1 than MSH2", &Identity);
    assert!(
        more_adj.iter().any(|p| pretty_term(p.sem()) == both),
        "more (adjectival) → gt(deg(x), deg(y)): {:?}",
        more_adj
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, more_adj[0].sem()).expect("adjectival comparative type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "denotes Prop");

    // nominal: "HeLa affects greater dependence on BRCA1 than MSH2" → the SAME term (A4 unification).
    let greater_noun = index.parse(
        "HeLa affects greater dependence on BRCA1 than MSH2",
        &Identity,
    );
    assert!(
        greater_noun.iter().any(|p| pretty_term(p.sem()) == both),
        "adjectival and nominal comparatives share one deg: {:?}",
        greater_noun
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );

    // less (adjectival, LESS): gt(deg(y), deg(x)).
    let less_adj = index.parse("HeLa is less dependent on BRCA1 than MSH2", &Identity);
    assert!(
        less_adj.iter().any(|p| pretty_term(p.sem())
            == "gt(deg_dependent(brca1, msh2), deg_dependent(brca1, hela))"),
        "less (adjectival): {:?}",
        less_adj
            .iter()
            .map(|p| pretty_term(p.sem()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn governed_preposition_gates_the_oblique_pp() {
    // D63 §5.3 C3-precision (d63-comparative-phrasal.md): the relational adjective `dependent` governs
    // `on` — its demo category is `cat_measure / cat_pp_arg(prep_on)` — so the WRONG preposition is
    // rejected at the feature-meet. The two sentences differ ONLY in the preposition, isolating the gate:
    // `on` (prep_on marker) meets the prep_on slot; `to` (prep_to marker) does not (prep_to ≠ prep_on,
    // neither is the prep_any wildcard). A plain transitive/PP-agnostic verb would take prep_any and
    // accept either — the precision is that a *gloss-governed* head pins its preposition.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        !index
            .parse("HeLa is more dependent on BRCA1 than MSH2", &Identity)
            .is_empty(),
        "`more dependent ON BRCA1` composes — the `on` marker's prep_on meets the governed slot"
    );
    assert!(
        index
            .parse("HeLa is more dependent to BRCA1 than MSH2", &Identity)
            .is_empty(),
        "`*more dependent TO BRCA1` must NOT parse — `dependent` governs `on`, so cat_pp_arg(prep_to) \
         fails the feature-meet against cat_pp_arg(prep_on)"
    );
}

// ── D63 §8.13 Slice 6-mod — compound nouns + PP adjuncts ──────────────
/// Whether `e`'s App-spine head is the named opaque axiom.
fn head_is_axiom(e: &Exp, iri: &str) -> bool {
    let mut cur = e;
    while let Exp::App(f, _) = cur {
        cur = f;
    }
    matches!(cur, Exp::EigonAxiom(i) if i.as_str() == iri)
}

#[test]
fn compound_noun_refines_the_head() {
    // "a BRCA1 cell line affects HeLa": [NP BRCA1] + [N cell line] → the refined noun
    // Σx:CellLine. compound(x, brca1); `a` quantifies it (Fst), the verb applies. That it
    // parses to Prop at all *is* the witness the compound rule fired (without it, `a` has
    // no noun and there is no parse).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a BRCA1 cell line affects HeLa", &Identity);
    assert!(
        !forest.is_empty(),
        "the compound-noun sentence must parse (the compound rule must fire)"
    );
    assert_parses_to_prop("a BRCA1 cell line affects HeLa");
}

#[test]
fn pp_adjunct_adds_an_opaque_conjunct() {
    // "HeLa affects BRCA1 in HeLa" → And(affects(brca1, hela), prep_in(hela, hela)) : Prop.
    // The PP modifies the VP, conjoining the opaque locative.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 in HeLa", &Identity);
    assert!(!forest.is_empty(), "the PP-adjunct sentence must parse");
    let has_prep = forest.iter().any(|p| {
        matches!(p.sem(), Exp::InductiveType(decl, args)
            if decl.name == "And" && args.len() == 2
                && head_is_axiom(&args[1], "urn:eigenius:ontology:prep_in"))
    });
    assert!(
        has_prep,
        "a parse is And(VP-predication, prep_in(s, x)); got {:?}",
        forest[0].sem()
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let p = forest
        .iter()
        .find(|p| matches!(p.sem(), Exp::InductiveType(decl, _) if decl.name == "And"))
        .unwrap();
    let ty = check_infer(&mut ctx, p.sem()).expect("PP-adjunct sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "PP-adjunct clause denotes Prop"
    );
}

/// Whether the opaque axiom `iri` occurs ANYWHERE in `e` (descends binder types too —
/// a 6-mod restrictor lives in the Σ embedded inside the determiner's ∃/∀ encoding).
fn sem_mentions_axiom(e: &Exp, iri: &str) -> bool {
    let any = |xs: &[Exp]| xs.iter().any(|x| sem_mentions_axiom(x, iri));
    match e {
        Exp::EigonAxiom(i) => i.as_str() == iri,
        Exp::App(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) | Exp::Ann(a, b) => {
            sem_mentions_axiom(a, iri) || sem_mentions_axiom(b, iri)
        }
        Exp::Pi(_, a, b) | Exp::Sig(_, a, b) => {
            sem_mentions_axiom(a, iri) || sem_mentions_axiom(b, iri)
        }
        Exp::Lam(_, a) | Exp::Fst(a) | Exp::Snd(a) | Exp::Con(_, a) => sem_mentions_axiom(a, iri),
        Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => any(args),
        _ => false,
    }
}

#[test]
fn n_n_kind_compound_refines_with_compound_kind() {
    // "a gene cell line affects HeLa": [N gene] + [N cell line] → Σx:CellLine.
    // compound_kind(x, Gene) (the modifier is the kind `Gene`, a `Set`); `a` quantifies it.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a gene cell line affects HeLa", &Identity);
    assert!(
        !forest.is_empty(),
        "the N-N kind-compound sentence must parse (the N-N rule must fire)"
    );
    assert!(
        forest
            .iter()
            .any(|p| sem_mentions_axiom(p.sem(), "urn:eigenius:ontology:compound_kind")),
        "a parse refines the head with ontology:compound_kind; got {:?}",
        forest[0].sem()
    );
    assert_parses_to_prop("a gene cell line affects HeLa");
}

#[test]
fn pp_noun_modifier_refines_the_head() {
    // "a cell line of BRCA1 affects HeLa": [N cell line] + [PP of BRCA1] → Σx:CellLine.
    // prep_of(x, brca1) (post-nominal refine); `a` quantifies it. `of` is a pure
    // noun-modifier (no VP-adjunct entry), so this is the post-nominal path.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a cell line of BRCA1 affects HeLa", &Identity);
    assert!(
        !forest.is_empty(),
        "the PP-noun-modifier sentence must parse (the post-nominal rule must fire)"
    );
    assert!(
        forest
            .iter()
            .any(|p| sem_mentions_axiom(p.sem(), "urn:eigenius:ontology:prep_of")),
        "a parse refines the head with ontology:prep_of; got {:?}",
        forest[0].sem()
    );
    assert_parses_to_prop("a cell line of BRCA1 affects HeLa");
}

#[test]
fn pp_attachment_is_ambiguous() {
    // "HeLa affects a cell line in HeLa": `in` is BOTH a VP-adjunct and a noun-modifier, so
    // the PP attaches two ways — to the VP (And(affects(…), prep_in(s, hela))) and to the
    // object noun (Σx:CellLine. prep_in(x, hela)). Both felicitous → ≥2 parses, the
    // attachment ambiguity carried in the forest (D63 §8.13).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects a cell line in HeLa", &Identity);
    assert!(
        forest.len() >= 2,
        "PP attachment is ambiguous (VP-adjunct vs noun-modifier) → ≥2 parses, got {}",
        forest.len()
    );
    // One parse conjoins at the VP (And-headed); another refines the noun (no top-level And).
    assert!(
        forest
            .iter()
            .any(|p| matches!(p.sem(), Exp::InductiveType(d, _) if d.name == "And")),
        "one attachment conjoins the locative at the VP (And-headed)"
    );
    assert!(
        forest
            .iter()
            .any(|p| !matches!(p.sem(), Exp::InductiveType(d, _) if d.name == "And")),
        "the other attachment refines the object noun (not top-level And)"
    );
    assert_parses_to_prop("HeLa affects a cell line in HeLa");
}

#[test]
fn compound_chain_is_left_branching() {
    // "a BRCA1 gene cell line affects HeLa": a 3-element compound chain. The left-branching
    // normal form (D63 §8.13) collapses [[BRCA1 gene] cell line] vs [BRCA1 [gene cell line]]
    // to the single left-branching tree — the head of a compound may not itself be a
    // compound result — so there is exactly ONE parse (no spurious bracketing ambiguity).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a BRCA1 gene cell line affects HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "the compound chain has a single left-branching parse (NF), got {}",
        forest.len()
    );
    assert_parses_to_prop("a BRCA1 gene cell line affects HeLa");
}

#[test]
fn committed_object_determiner_parses() {
    // `a` (object, type-raised) from the committed closed-class layer.
    assert_parses_to_prop("HeLa affects a cell line"); // ∃c:CellLine. affects(c, HeLa)
}

#[test]
fn committed_determiners_compose_both_positions() {
    // Subject `every` + object `a`, both committed.
    assert_parses_to_prop("every cell line affects a cell line");
}

// ── D63 §8.4 Phase 3 — generalized conjunction (connectives) ─────────
// `and`/`or` are parser-level reserved words; coordination pointwise-lifts the
// connective (logic:And/Or) over same-category, Prop-ending conjuncts.

#[test]
fn sentence_coordination_parses() {
    // S and S: "HeLa affects BRCA1 and BRCA1 affects HeLa"
    //   → logic:And(affects(BRCA1, HeLa), affects(HeLa, BRCA1)) : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 and BRCA1 affects HeLa");
}

#[test]
fn vp_coordination_parses() {
    // VP and VP (pointwise lift at S\NP): "HeLa affects BRCA1 and affects HeLa"
    //   → λs. And(affects(BRCA1, s), affects(HeLa, s)) applied to HeLa : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 and affects HeLa");
}

#[test]
fn disjunction_parses() {
    // `or` → logic:Or, same generalized-conjunction machinery.
    assert_parses_to_prop("HeLa affects BRCA1 or BRCA1 affects HeLa");
}

#[test]
fn coordinators_are_known_to_the_missing_lexeme_signal() {
    // `and`/`or` are consumed by the parser's coordination rule, NOT lexical entries
    // (coordination is polymorphic over `Cat`; the felicity gate can't type a
    // category-polymorphic entry). The missing-lexeme signal `has_token` (D62 §7.6a)
    // must still report them as KNOWN — else the encoding pipeline routes a
    // structurally-handled connective to lexical recovery. `but` is deliberately NOT a
    // coordinator (it is a distinct sentential connective); it is now a lexicalized
    // subordinator (D62 §2d), so the signal sees it via the ordinary lexical path.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.has_token("and", &Identity),
        "`and` is a known connective"
    );
    assert!(
        index.has_token("or", &Identity),
        "`or` is a known connective"
    );
    assert!(
        index.has_token("but", &Identity),
        "`but` is known via its lexical entry (2d subordinator), not the coordination rule"
    );
}

// ── D63 §8.4 Phase 6 — NP coordination as `List`-groups (distributive) ─
// A coordinated NP is a member-retaining group (`cat_group(C, pl)` over `List C`);
// the distributive reading maps a one-place predicate over the members and
// ∧-folds — "X and Y affects Z" → affects(Z,X) ∧ affects(Z,Y).

/// The operands of a left-branching connective chain (`op(op(a, b), c)` / `op(a,
/// b)`) for `conn` ∈ {"And", "Or"}, flattened left-to-right; `None` if `sem` is not
/// headed by that connective.
fn conn_chain(sem: &Exp, conn: &str) -> Option<Vec<Exp>> {
    match sem {
        Exp::InductiveType(decl, args) if decl.name == conn && args.len() == 2 => {
            let mut left = conn_chain(&args[0], conn).unwrap_or_else(|| vec![args[0].clone()]);
            left.push(args[1].clone());
            Some(left)
        }
        _ => None,
    }
}

fn and_conjuncts(sem: &Exp) -> Option<Vec<Exp>> {
    conn_chain(sem, "And")
}

#[test]
fn distributive_np_coordination_parses() {
    // "HeLa and BRCA1 affect HeLa": the coordinated subject is a group
    // [hela, brca1] : List Entity (CellLine ⊔ Gene = Entity); the predicate
    // `affects HeLa` = λs. affects(hela, s) distributes over the members →
    // affects(hela, hela) ∧ affects(hela, brca1) : Prop.
    assert_parses_to_prop("HeLa and BRCA1 affect HeLa");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 affect HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one distributive parse");
    let conjuncts = and_conjuncts(forest[0].sem())
        .expect("distributive sem is a logic:And of the per-member predications");
    assert_eq!(
        conjuncts.len(),
        2,
        "two members ⇒ two conjuncts; got {}",
        conjuncts.len()
    );
}

#[test]
fn disjunctive_np_coordination_distributes_with_or() {
    // "HeLa or BRCA1 affect HeLa": an `or`-group distributes with ∨ →
    // affects(hela, hela) ∨ affects(hela, brca1) : Prop.
    assert_parses_to_prop("HeLa or BRCA1 affect HeLa");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa or BRCA1 affect HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "exactly one disjunctive-distributive parse"
    );
    let disjuncts = conn_chain(forest[0].sem(), "Or")
        .expect("disjunctive sem is a logic:Or of the per-member predications");
    assert_eq!(disjuncts.len(), 2, "two members ⇒ two disjuncts");
}

#[test]
fn distributive_object_coordination_parses() {
    // Object-position distribution: "HeLa affects BRCA1 and HeLa" — the object is a
    // group [brca1, hela]; the verb distributes over it →
    // affects(brca1, hela) ∧ affects(hela, hela) : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 and HeLa");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 and HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one distributive-object parse");
    assert_eq!(
        and_conjuncts(forest[0].sem()).map(|c| c.len()),
        Some(2),
        "two object members ⇒ two conjuncts"
    );
}

#[test]
#[ignore = "probe: subject-GQ + coordinated(distributed) object (s20 residual); --ignored --nocapture"]
fn probe_subject_gq_distributed_object() {
    let (_layer, index) = index_over_bootstrap();
    let probe = |s: &str, lem: &dyn eigenius_kernel::dcg::Lemmatizer| {
        let (c, o) = index.parse_open(s, lem);
        let tag = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  {tag:<10} {s:?}");
    };
    eprintln!("\n=== subject-GQ + coordinated object: simple vs bare-plural/comparative coordinands (demo) ===");
    probe("a gene affects a gene or a cell line", &Identity); //      subj-GQ + det⊕det (CLOSES)
    probe("a gene affects genes or a cell line", &PluralS); //        subj-GQ + BARE-PLURAL ⊕ GQ
    probe("genes affect genes or a cell line", &PluralS); //          plural subj + same (baseline)
    probe("a gene affects a gene or a larger cell line", &Identity); // subj-GQ + GQ ⊕ COMPARATIVE
    probe("a gene affects genes or a larger cell line", &PluralS); // subj-GQ + bare-plural ⊕ comparative (s20 shape)
}

#[test]
#[ignore = "probe: RC-8 clausal-complement sentence shapes (demo); --ignored --nocapture"]
fn probe_rc8_demo() {
    let (_layer, index) = index_over_bootstrap();
    let probe = |s: &str| {
        let (c, o) = index.parse_open(s, &Identity);
        let tag = if !c.is_empty() {
            format!("CLOSED×{}", c.len())
        } else if !o.is_empty() {
            format!("open×{}", o.len())
        } else {
            "GAP".to_string()
        };
        eprintln!("  {tag:<10} {s:?}");
    };
    // RC-8 s1: `We hypothesized that MSI and MMR deficiency may create vulnerabilities` — clausal
    // complement + coordinated subject + embedded modal. Demo proxy with `shows` (clause verb).
    eprintln!("\n=== RC-8 s1 shape (clausal + coord subject + modal) — demo ===");
    probe("HeLa shows that BRCA1 affects HeLa"); //             clausal baseline
    probe("HeLa shows that BRCA1 may affect HeLa"); //          clausal + embedded modal
    probe("HeLa shows that BRCA1 and HeLa affect HeLa"); //     clausal + coordinated subject
    probe("HeLa shows that BRCA1 and HeLa may affect HeLa"); // clausal + coord subject + modal (s1 shape)
                                                             // RC-8 s2 core: `… is not simply a result of MMR deficiency` — copula + predicate NOMINAL (`a result
                                                             // of X`), vs the predicate ADJECTIVES the copula is known to take. Isolate in the demo.
    eprintln!("=== RC-8 s2 core: copula + predicate NOMINAL (± neg) — demo ===");
    probe("HeLa is primary"); //         copula + predicate ADJECTIVE (baseline, known-good)
    probe("HeLa is a cell line"); //     copula + predicate NOMINAL
    probe("HeLa is a gene"); //          copula + predicate NOMINAL (different type)
    probe("HeLa is not a cell line"); // + negation
    probe("HeLa is a cell line of BRCA1"); //      predicate nominal + of-PP complement (`a result OF X`)
    probe("HeLa is not a cell line of BRCA1"); //  + negation
    probe("HeLa shows that HeLa is a cell line of BRCA1"); // full s2 shape (clausal + copula-nominal-of-PP)
}

#[test]
fn s20_shape_parses_open_with_modal_coordination_and_comparative() {
    // The s20 sentence (`WRN dependency may require specific lineages or a stronger mutation phenotype`)
    // composes three fixes: the modal (`may` → Possible), heterogeneous object-GQ coordination over
    // DIFFERENT types (type-preserving `Or`), and the attributive comparative (anaphoric standard hole).
    // Demo proxy `HeLa may affect a gene or a larger cell line` must parse OPEN — not gap, not closed
    // (the comparative standard is unresolved) — as `Possible(Or(∃:Gene …, ∃:(Σ CellLine. gt(deg,
    // anaphor)) …))` with exactly one comparison-standard hole the D64 resolver fills.
    let (_layer, index) = index_over_bootstrap();
    let (closed, open) =
        index.parse_open("HeLa may affect a gene or a larger cell line", &Identity);
    assert!(
        closed.is_empty(),
        "the comparative standard is unresolved → the s20 shape must be OPEN, not closed"
    );
    let it = open
        .iter()
        .find(|o| o.holes.len() == 1)
        .expect("an open parse with exactly one comparison-standard hole");
    let sem = pretty_term(it.item.sem());
    assert!(
        sem.contains("Possible(")
            && sem.contains("Or(")
            && sem.contains(":Gene.")
            && sem.contains(":CellLine.")
            && sem.contains("gt(deg_large")
            && sem.contains("$anaphor$"),
        "s20 shape = modal + type-preserving disjunction (Gene ∨ CellLine) + comparative-standard hole: {sem}"
    );
}

#[test]
fn heterogeneous_object_gq_coordination_generalizes_type() {
    // D63 §8.4 — coordinating type-raised OBJECT quantifiers over DIFFERENT noun types (s20's object
    // `specific lineages or a stronger mutation phenotype` = Lineage ⊕ Phenotype). The object-GQ
    // categories differ only in the exposed object slot (`cat_np(Gene)` vs `cat_np(CellLine)`); the
    // coordination widens that slot to their `common_super` (Entity) so a general verb still fills it,
    // while the per-disjunct SEMANTICS keep the distinct bound types: `∃g:Gene.V(g) ∨ ∃c:CellLine.V(c)`.
    let (_layer, index) = index_over_bootstrap();

    // det ⊕ det, different types → closes with a TYPE-PRESERVING disjunction (not collapsed to Entity).
    let forest = index.parse("HeLa affects a gene or a cell line", &Identity);
    assert!(
        !forest.is_empty(),
        "different-type object-GQ coordination must close"
    );
    let sem = pretty_term(forest[0].sem());
    assert!(
        sem.starts_with("Or(") && sem.contains(":Gene.") && sem.contains(":CellLine."),
        "the disjunction preserves BOTH bound types (Gene ∨ CellLine, not widened to Entity): {sem}"
    );

    // plural ⊕ det, different types — the s20 object shape — also closes.
    assert!(
        !index
            .parse("HeLa affects genes or a cell line", &PluralS)
            .is_empty(),
        "plural ⊕ determined, different types (the s20 object shape) closes"
    );

    // Guard: the object-GQ gate must NOT leak into SUBJECT coordination — a coordinated plural subject
    // still needs the plural-finite verb, so `*HeLa and BRCA1 affects HeLa` (3sg) stays out (agreement
    // bites; forward-headed subject-GQs are excluded from the generalization).
    assert!(
        index
            .parse("HeLa and BRCA1 affects HeLa", &Identity)
            .is_empty(),
        "the object-GQ generalization does not bypass subject-verb agreement"
    );
}

#[test]
fn attributive_comparative_opens_with_a_standard_hole() {
    // D63 §8.5 / d63-comparative-phrasal §8: an attributive comparative (`a larger cell line`,
    // `a stronger mutation phenotype` — s20) has NO explicit `than`-standard; the standard is
    // anaphoric (a discourse comparison class), unlike the positive's absolute norm `std_large`. So
    // it parses OPEN — a bare `S[adj]\NP` reading `λx. gt(deg(x), deg(anaphor))` (e_larger_attrib) +
    // the attributive refine rule → `Σx. gt(deg(x), deg($anaphor$))`, one comparison-standard hole the
    // D64 resolver fills. It must NOT gap, and must NOT spuriously close (a closed parse would mean the
    // relative standard was silently invented).
    let (_layer, index) = index_over_bootstrap();

    // Positive attributive stays CLOSED — absolute `std_large`, no hole.
    assert!(
        !index
            .parse("HeLa affects a large cell line", &Identity)
            .is_empty(),
        "positive `a large cell line` closes (absolute standard)"
    );

    // Comparative attributive: OPEN, not gap, not closed; exactly one comparison-standard hole.
    let (closed, open) = index.parse_open("HeLa affects a larger cell line", &Identity);
    assert!(
        closed.is_empty(),
        "`a larger cell line` must NOT close — the comparative standard is unresolved"
    );
    let one_hole = open.iter().find(|o| o.holes.len() == 1);
    assert!(
        one_hole.is_some(),
        "`a larger cell line` must parse OPEN with one comparison-standard hole, got {} open",
        open.len()
    );
    let sem = pretty_term(one_hole.unwrap().item.sem());
    assert!(
        sem.contains("gt(") && sem.contains("deg_large"),
        "the open sem compares the degree against an anaphoric standard: {sem}"
    );

    // Works in subject position too.
    let (c2, o2) = index.parse_open("a larger cell line affects HeLa", &Identity);
    assert!(
        c2.is_empty() && o2.iter().any(|o| o.holes.len() == 1),
        "subject-position attributive comparative also opens with one hole"
    );
}

#[test]
fn distributive_object_coordination_with_or_parses() {
    // Object distribution with `or`: "HeLa affects BRCA1 or HeLa" →
    // affects(brca1, hela) ∨ affects(hela, hela) : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 or HeLa");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 or HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one disjunctive-object parse");
    assert_eq!(
        conn_chain(forest[0].sem(), "Or").map(|c| c.len()),
        Some(2),
        "two object members ⇒ two disjuncts"
    );
}

/// The length of a `List` cons-chain sem (`cons(_, h, t)` / `nil`); `None` if not
/// a well-formed list.
fn cons_len(sem: &Exp) -> Option<usize> {
    match sem {
        Exp::InductiveCtor(_, n, args) if n == "nil" && args.is_empty() => Some(0),
        Exp::InductiveCtor(_, n, args) if n == "cons" && args.len() == 2 => {
            Some(1 + cons_len(&args[1])?)
        }
        _ => None,
    }
}

#[test]
fn collective_np_coordination_parses() {
    // "HeLa and BRCA1 form a complex": the collective verb is typed over the GROUP
    // (`S\Group(Entity)`, ⟦·⟧ = List Entity → Prop), so the coordinated subject —
    // the retained group [hela, brca1] : List Entity — is its argument directly →
    // forms_complex([hela, brca1]) : Prop. No mereological sum entity invented.
    assert_parses_to_prop("HeLa and BRCA1 form a complex");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 form a complex", &Identity);
    assert_eq!(forest.len(), 1, "exactly one collective parse");
    match forest[0].sem() {
        Exp::App(_head, arg) => assert_eq!(
            cons_len(arg),
            Some(2),
            "the collective verb consumes the retained 2-member group list"
        ),
        other => panic!("collective sem must be V applied to the group list, got {other:?}"),
    }
}

#[test]
fn collective_rejects_an_or_group() {
    // Collective is `and`-only: "HeLa or BRCA1 form a complex" has no parse — the
    // collective verb's `conn_and` group slot won't accept an `or`-group, and
    // `cat_group` doesn't distribute (its slot isn't `cat_np`).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index
            .parse("HeLa or BRCA1 form a complex", &Identity)
            .is_empty(),
        "an or-group must not get a collective reading"
    );
}

#[test]
fn reciprocal_np_coordination_parses() {
    // "HeLa and BRCA1 affect each other": the verb is related over every ordered
    // distinct pair of the subject group's members → affects(brca1, hela) ∧
    // affects(hela, brca1) : Prop. ("each other" is a reserved reciprocal anaphor.)
    assert_parses_to_prop("HeLa and BRCA1 affect each other");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 affect each other", &Identity);
    assert_eq!(forest.len(), 1, "exactly one reciprocal parse");
    // 2 members ⇒ 2 ordered distinct pairs ⇒ 2 conjuncts.
    assert_eq!(
        and_conjuncts(forest[0].sem()).map(|c| c.len()),
        Some(2),
        "two members ⇒ two ordered-pair conjuncts"
    );
}

#[test]
fn reciprocal_three_members_has_six_ordered_pairs() {
    // n members ⇒ n·(n−1) ordered distinct pairs: 3 members → 6 conjuncts.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 and HeLa affect each other", &Identity);
    assert_eq!(forest.len(), 1, "exactly one reciprocal parse");
    assert_eq!(
        and_conjuncts(forest[0].sem()).map(|c| c.len()),
        Some(6),
        "three members ⇒ 3·2 = 6 ordered-pair conjuncts"
    );
}

#[test]
fn reciprocal_rejects_an_or_group() {
    // Reciprocity is conjunctive — an `or`-group gets no reciprocal reading.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index
            .parse("HeLa or BRCA1 affect each other", &Identity)
            .is_empty(),
        "an or-group must not get a reciprocal reading"
    );
}

// ── D63 §8.5 Slice 5b — subject wh-questions ──────────────────────────
// A wh-question denotes its answer-property ⟦Q(T)⟧ = T → Prop. A SUBJECT wh has
// its gap adjacent to the VP, so it composes by plain application (no extraction).

/// The queried type of a `cat_q(T)` result — `T` as an `EigonClass` IRI string.
fn cat_q_type(cat: &Exp) -> Option<String> {
    match is_ctor(cat, "cat_q")?.first()? {
        Exp::EigonClass(iri) => Some(iri.as_str().to_string()),
        _ => None,
    }
}

#[test]
fn subject_wh_what_parses_to_an_entity_answer_property() {
    // "what affects HeLa": the gap is the subject → λx:Entity. affects(hela, x) :
    // Entity → Prop. The result category is Q(Entity); the felicity filter confirms
    // the sem inhabits ⟦Q(Entity)⟧ = Entity → Prop (else the forest would be empty).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("what affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one subject-wh parse");
    assert_eq!(
        cat_q_type(forest[0].cat()).as_deref(),
        Some("urn:eigenius:lexicon:Entity"),
        "'what' queries the Entity top"
    );
    assert!(
        matches!(forest[0].sem(), Exp::Lam(_, _)),
        "the answer-property is a λ (T → Prop), got {:?}",
        forest[0].sem()
    );
}

#[test]
fn subject_wh_which_narrows_the_answer_type_to_the_noun() {
    // "which cell line affects HeLa": the restrictor narrows the answer to CellLine
    // → λx:CellLine. affects(hela, x) : CellLine → Prop. The Entity-typed verb fills
    // the `S\NP_CellLine` slot by the contravariant functor subsumption (§8.2 item 4),
    // and the η-expanded `which` sem binds `x:CellLine` so the answer type narrows
    // via the covariant application coercion (CellLine ≤ Entity).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("which cell line affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one restricted subject-wh parse");
    assert_eq!(
        cat_q_type(forest[0].cat()).as_deref(),
        Some("urn:eigenius:lexicon:CellLine"),
        "'which cell line' narrows the queried type to CellLine"
    );
    assert!(
        matches!(forest[0].sem(), Exp::Lam(_, _)),
        "answer-property is a λ"
    );
}

#[test]
fn subject_wh_which_requires_a_noun_restrictor() {
    // `which` is a determiner-shaped wh — it needs a common-noun restrictor on its
    // right; there is no bare-`which` subject reading.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("which affects HeLa", &Identity).is_empty(),
        "'which' needs a common-noun restrictor"
    );
}

// ── D63 §8.5 Slice 5a — polar (yes/no) questions ──────────────────────
// Auxiliary inversion: `aux + subject + base-VP → S[q]`. ⟦S[q]⟧ = Prop (the queried
// proposition), `mood`-tagged `q`. Application-only; the aux selects a base VP.

/// The mood of a `cat_s` result (`dcl` / `q`).
fn sentence_mood(cat: &Exp) -> Option<String> {
    match is_ctor(cat, "cat_s")?.first()? {
        Exp::InductiveCtor(_, n, _) => Some(n.clone()),
        _ => None,
    }
}

#[test]
fn polar_question_parses_to_a_queried_prop() {
    // "does HeLa affect BRCA1?": aux inversion → the queried proposition
    // affect(brca1, hela) : Prop, tagged `mood = q` (asked, not asserted).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("does HeLa affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one polar parse");
    assert_eq!(
        sentence_mood(forest[0].cat()).as_deref(),
        Some("q"),
        "a polar question is tagged mood = q"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("polar sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a polar question denotes Prop"
    );
}

#[test]
fn bare_base_clause_is_not_a_finite_root() {
    // "*HeLa affect BRCA1" — the base-form verb yields a base clause `S[_,bse]`,
    // which is not a standalone finite sentence (the finiteness root gate). Only
    // "HeLa affects BRCA1" (finite) or "does HeLa affect BRCA1" (aux) are roots.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa affect BRCA1", &Identity).is_empty(),
        "a bare base clause must not parse as a finite root"
    );
    // the finite form still parses:
    assert!(
        !index.parse("HeLa affects BRCA1", &Identity).is_empty(),
        "the finite declarative still parses"
    );
}

#[test]
fn auxiliary_requires_a_base_form_complement() {
    // "*does HeLa affects BRCA1" — the aux selects a base VP (`S[dcl,bse]\NP`); the
    // finite "affects" fails the Fin-meet, so there is no parse.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("does HeLa affects BRCA1", &Identity).is_empty(),
        "the auxiliary must reject a finite (non-base) complement"
    );
}

// ── D63 §8.5 Slice 5c — object wh-extraction (forward composition B + Eisner) ──
// The object gap is non-adjacent: forward composition builds `S[q]/NP` ("does HeLa
// ∘ affect"), the wh-word consumes it → the answer-property `T → Prop`.

#[test]
fn object_wh_what_extracts_via_composition() {
    // "what does HeLa affect?": `does HeLa` (S[q]/(S[bse]\NP)) >B `affect`
    // ((S[bse]\NP)/NP) → S[q]/NP (λz. affect(z, hela)); `what` consumes it →
    // λx:Entity. affect(x, hela) : Entity → Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("what does HeLa affect", &Identity);
    assert_eq!(forest.len(), 1, "exactly one object-wh parse");
    assert_eq!(
        cat_q_type(forest[0].cat()).as_deref(),
        Some("urn:eigenius:lexicon:Entity"),
        "object 'what' queries the Entity top"
    );
    assert!(
        matches!(forest[0].sem(), Exp::Lam(_, _)),
        "the answer-property is a λ"
    );
}

#[test]
fn object_wh_which_narrows_to_the_noun() {
    // "which cell line does HeLa affect?": the restrictor narrows the answer to
    // CellLine → λx:CellLine. affect(x, hela) : CellLine → Prop. The composed
    // `S[q]/NP_Entity` fills the `S[q]/NP_CellLine` slot by contravariant subsumption.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("which cell line does HeLa affect", &Identity);
    assert_eq!(forest.len(), 1, "exactly one restricted object-wh parse");
    assert_eq!(
        cat_q_type(forest[0].cat()).as_deref(),
        Some("urn:eigenius:lexicon:CellLine"),
        "'which cell line' narrows the queried type to CellLine"
    );
}

// ── D63 §8.5 Slice 3a — copula + predicative adjective ────────────────
// `is`/`are` supply finiteness to a BASE adjective predicate ("HeLa is primary").

#[test]
fn copula_with_predicative_adjective_parses() {
    // "HeLa is primary": the copula lifts the base adjective `primary`
    // (S[dcl,bse]\NP) to a finite VP → is_primary(hela) : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one copula parse");
    assert_eq!(
        sentence_mood(forest[0].cat()).as_deref(),
        Some("dcl"),
        "a copular predication is a declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("copula sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the predication denotes Prop"
    );
}

#[test]
fn bare_adjective_needs_the_copula() {
    // "*HeLa primary" — `primary` is a BASE predicate (S[dcl,bse]\NP); without the
    // copula the clause is non-finite, so it is not a standalone root.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa primary", &Identity).is_empty(),
        "a bare predicative adjective is not a finite root without the copula"
    );
}

// ── D63 §8.5 Slice 3b — attributive adjectives (Σ-refinement, engine-level) ──
// "primary cell line" refines the noun to Σx:CellLine. is_primary(x); a determiner
// quantifies over the Σ-type with Fst-projection (correct restrictor for ∀ and ∃).

#[test]
fn attributive_adjective_existential_parses() {
    // "a primary cell line affects HeLa" → ∃z:(Σx:CellLine. is_primary(x)).
    // affects(Fst z, hela) ≡ ∃x:CellLine. is_primary(x) ∧ affects(x, hela) : Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a primary cell line affects HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "exactly one attributive (existential) parse"
    );
    assert_parses_to_prop("a primary cell line affects HeLa");
}

#[test]
fn attributive_adjective_universal_parses() {
    // "every primary cell line affects HeLa" → ∀z:(Σx:CellLine. is_primary(x)).
    // affects(Fst z, hela) ≡ ∀x:CellLine. is_primary(x) → affects(x, hela) : Prop.
    // (The Σ-type yields the implication restrictor for ∀ uniformly — no kernel
    // coercion; the Fst is engine-inserted.)
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every primary cell line affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one attributive (universal) parse");
    assert_parses_to_prop("every primary cell line affects HeLa");
}

// ── D63 §8.5 Slice 3c — predicate nominals (opaque is_a) ──────────────
// "HeLa is a cell line" → ontology:is_a(hela, CellLine) : Prop — an opaque
// membership claim (the ontology's own relation, grounded downstream by ChainWitness).

#[test]
fn predicate_nominal_parses_to_is_a() {
    // The predicative `a` forms `λs. is_a(s, CellLine)` (an adjectival predicate);
    // the copula lifts it; the subject applies → is_a(hela, CellLine) : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is a cell line", &Identity);
    assert_eq!(forest.len(), 1, "exactly one predicate-nominal parse");
    assert_eq!(
        sentence_mood(forest[0].cat()).as_deref(),
        Some("dcl"),
        "a predicate nominal is a declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("predicate-nominal sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a predicate nominal denotes Prop"
    );
    // Structure: is_a(hela, CellLine) = App(App(ontology:is_a, hela), CellLine).
    match forest[0].sem() {
        Exp::App(f, _) => match &**f {
            Exp::App(g, _) => assert!(
                matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:ontology:is_a"),
                "predicate-nominal head is ontology:is_a, got {g:?}"
            ),
            other => panic!("expected is_a application, got {other:?}"),
        },
        other => panic!("expected is_a(s, C), got {other:?}"),
    }
}

#[test]
fn do_support_rejects_an_adjective() {
    // The `adj` category fix (Slice 3b step 1): do-support selects base VERBS, not
    // adjectives, so "*does HeLa primary" has no parse (the aux's `bse` slot rejects
    // the `adj` predicate). With the earlier `adj = bse` conflation this wrongly parsed.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("does HeLa primary", &Identity).is_empty(),
        "do-support must reject an adjectival complement"
    );
}

#[test]
fn copula_rejects_a_verbal_complement() {
    // "*HeLa is affects HeLa" — the copula selects a BASE predicate; the finite verb
    // "affects" fails the Fin-meet, so this over-generation is blocked.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is affects HeLa", &Identity).is_empty(),
        "the copula must reject a finite verbal complement"
    );
}

// ── D63 §8.6 Slice 6-neg — negation (¬P := P → logic:False) ───────────

/// Whether `sem` is a negation `… → logic:False` (an arrow/Π whose codomain is
/// `logic:False`).
fn is_negation(sem: &Exp) -> bool {
    let cod = match sem {
        Exp::Arrow(_, c) => c,
        Exp::Pi(_, _, c) => c,
        _ => return false,
    };
    matches!(&**cod, Exp::InductiveType(decl, _) if decl.name == "False")
}

#[test]
fn verbal_negation_parses() {
    // "HeLa does not affect BRCA1": declarative do-support + `not` over the base VP
    // → affect(brca1, hela) → logic:False : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa does not affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one verbal-negation parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("negation sem type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "negation denotes Prop");
    assert!(
        is_negation(forest[0].sem()),
        "sem is ¬(…) = … → logic:False, got {:?}",
        forest[0].sem()
    );
}

#[test]
fn copular_negation_parses() {
    // "HeLa is not primary": copula + `not` over the adjectival predicate →
    // is_primary(hela) → logic:False : Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is not primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one copular-negation parse");
    assert!(
        is_negation(forest[0].sem()),
        "sem is ¬is_primary(hela), got {:?}",
        forest[0].sem()
    );
}

// ── D63 §8.9 Slice 6-aux — progressive + perfect auxiliaries ──────────
// Aspect auxiliaries are finiteness-lifters (λP.P): progressive `be` over a present-
// participle (`ger`) VP, perfect `have` over a past-participle (`pss`) VP. Tense/aspect
// erased, so the proposition equals the plain declarative's.

#[test]
fn progressive_auxiliary_parses() {
    // "HeLa is affecting BRCA1": is_prog lifts the `ger` VP "affecting BRCA1"
    // (S[ger]\NP) to a finite VP → affects(brca1, hela) : Prop (aspect erased).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is affecting BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one progressive parse");
    assert_eq!(
        sentence_mood(forest[0].cat()).as_deref(),
        Some("dcl"),
        "a progressive clause is a finite declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("progressive sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "progressive denotes Prop"
    );
}

#[test]
fn perfect_auxiliary_parses() {
    // "HeLa has affected BRCA1": has_perf lifts the `pss` VP "affected BRCA1"
    // (S[pss]\NP) to a finite VP → affects(brca1, hela) : Prop (tense erased).
    assert_parses_to_prop("HeLa has affected BRCA1");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa has affected BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one perfect parse");
}

#[test]
fn short_passive_parses_with_existential_agent() {
    // "BRCA1 is affected": the passive `be` takes the unsaturated past-participle TV
    // and closes the agent → ∃a:Entity. affects(brca1, a) : Prop (BRCA1 is the patient).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("BRCA1 is affected", &Identity);
    assert_eq!(forest.len(), 1, "exactly one short-passive parse");
    assert_eq!(
        sentence_mood(forest[0].cat()).as_deref(),
        Some("dcl"),
        "a passive clause is a finite declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("passive sem type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "passive denotes Prop");
    // The agent is existentially closed — impredicative ∃ is `∀C:Prop. (…→C) → C`,
    // i.e. the sem is a Π/→ whose ultimate codomain is the bound `C` (a Var), not a
    // bare predicate application. (Distinguishes it from the active "affects".)
    assert!(
        matches!(forest[0].sem(), Exp::Pi(_, _, _) | Exp::Arrow(_, _)),
        "short passive closes the agent with an (impredicative) ∃, got {:?}",
        forest[0].sem()
    );
}

// ── D63 §8.9 Slice 6-aux — modal auxiliaries (opaque ◇/□) ─────────────
// Modals wrap the proposition with the opaque logic-layer operators: can/could/may/
// might → Possible, must → Necessary. Do-support-shaped aux over a BASE VP.

/// The head modal operator of `op(P)` = App(EigonAxiom(logic:Possible|Necessary), _).
fn modal_head(sem: &Exp) -> Option<String> {
    match sem {
        Exp::App(f, _) => match &**f {
            Exp::EigonAxiom(iri) => Some(iri.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn modal_can_wraps_the_proposition_in_possible() {
    // "HeLa can affect BRCA1" → Possible(affects(brca1, hela)) : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa can affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one modal parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("modal sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a modal claim denotes Prop"
    );
    assert_eq!(
        modal_head(forest[0].sem()).as_deref(),
        Some("urn:eigenius:logic:Possible"),
        "`can` wraps the proposition in the opaque ◇ (logic:Possible)"
    );
}

#[test]
fn modal_must_wraps_the_proposition_in_necessary() {
    // "HeLa must affect BRCA1" → Necessary(affects(brca1, hela)) : Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa must affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one modal parse");
    assert_eq!(
        modal_head(forest[0].sem()).as_deref(),
        Some("urn:eigenius:logic:Necessary"),
        "`must` wraps the proposition in the opaque □ (logic:Necessary)"
    );
}

#[test]
fn future_conditional_deontic_modals_each_wrap_their_own_opaque_operator() {
    // will / would / should are NOT ◇/□ — each carries its own opaque operator
    // (logic:Will / Would / Should), interpreted on the reasoning side (justification
    // logic). Distinct heads, so the future/conditional/deontic flavor is preserved.
    let (layer, index) = index_over_bootstrap();
    for (modal, op) in [
        ("will", "urn:eigenius:logic:Will"),
        ("would", "urn:eigenius:logic:Would"),
        ("should", "urn:eigenius:logic:Should"),
    ] {
        let sentence = format!("HeLa {modal} affect BRCA1");
        let forest = index.parse(&sentence, &Identity);
        assert_eq!(forest.len(), 1, "exactly one modal parse for `{modal}`");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
        let ty = check_infer(&mut ctx, forest[0].sem())
            .unwrap_or_else(|e| panic!("`{modal}` sem type-checks: {e}"));
        assert_eq!(readback_val(0, &ty), Exp::Sort(0), "`{modal}` denotes Prop");
        assert_eq!(
            modal_head(forest[0].sem()).as_deref(),
            Some(op),
            "`{modal}` wraps the proposition in its own opaque operator {op}"
        );
    }
    // Like the alethic modals, they select a BASE VP: a finite complement is rejected.
    assert!(
        index.parse("HeLa will affects BRCA1", &Identity).is_empty(),
        "`will` selects the base form (rejects a finite complement)"
    );
}

#[test]
fn modal_selects_a_base_vp() {
    // The modal aux selects a BASE VP (`S[bse]\NP`), like do-support — so the finite
    // "affects" is rejected: "*HeLa can affects BRCA1" has no parse (Fin-meet failure).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa can affects BRCA1", &Identity).is_empty(),
        "a modal must reject a finite complement (it selects the base form)"
    );
}

#[test]
fn agentive_long_passive_parses_with_the_by_agent() {
    // "BRCA1 is affected by HeLa": `by HeLa` supplies the agent and tags the patient-VP
    // with the `pass` voice feature; passive `be` lifts it → affects(brca1, hela) : Prop
    // — the SAME proposition as active "HeLa affects BRCA1" (agent supplied, not closed).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("BRCA1 is affected by HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one agentive-passive parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("agentive passive type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "agentive passive denotes Prop"
    );
    // Unlike the short passive (an impredicative ∃ = Π/→), the agent is supplied, so the
    // sem is a direct predication `affects(brca1, hela)` = App(App(affects, _), _).
    match forest[0].sem() {
        Exp::App(f, _) => assert!(
            matches!(&**f, Exp::App(g, _)
                if matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:lexicon:affects")),
            "agentive passive is affects(patient, agent), got head {f:?}"
        ),
        other => panic!("expected a direct predication affects(_, _), got {other:?}"),
    }
}

#[test]
fn passive_be_rejects_a_saturated_participle() {
    // The over-generation guard: passive `be` takes the TV *before* its object slot is
    // filled, so once the object is supplied ("affected BRCA1" : S[pss]\NP) it no longer
    // matches — `*HeLa is affected BRCA1` has no parse (no spurious active reading).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is affected BRCA1", &Identity).is_empty(),
        "passive be must not consume a saturated participle (no `*X is affected Y`)"
    );
}

#[test]
fn aspect_auxiliaries_select_the_right_participle() {
    // The `ger`/`pss` complement slots are exact: the progressive `be` rejects a base
    // verb ("*HeLa is affect BRCA1") and the perfect `have` rejects a gerund
    // ("*HeLa has affecting BRCA1") — finiteness/form mismatch, no parse (fail-closed).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is affect BRCA1", &Identity).is_empty(),
        "progressive be must reject a base verb (needs a present participle)"
    );
    assert!(
        index
            .parse("HeLa has affecting BRCA1", &Identity)
            .is_empty(),
        "perfect have must reject a gerund (needs a past participle)"
    );
}

#[test]
fn eisner_keeps_polar_single_despite_available_composition() {
    // With forward composition B now globally available, "does HeLa affect BRCA1"
    // could be derived a *second* way (`does HeLa ∘ affect` → S[q]/NP, then apply
    // BRCA1). Eisner normal form blocks that — a `>B` output may not be the functor
    // of `>` — so the application derivation is the *only* parse. This is the
    // regression witness that composition didn't reintroduce spurious ambiguity.
    let (_layer, index) = index_over_bootstrap();
    assert_eq!(
        index.parse("does HeLa affect BRCA1", &Identity).len(),
        1,
        "Eisner NF keeps the polar question a single parse despite B being available"
    );
}

// ── D63 §8.9 Slice 6-T + 6-rel — type-raising + restrictive relatives ──
// `that` is a reserved relativizer; `[noun] that [body]` Σ-refines the noun. A
// SUBJECT relative body is a VP `S\NP` (application only); an OBJECT relative body
// is `S/NP`, built by bounded type-raising `T` (NP → S/(S\NP)) + forward `B`. The
// refined noun then rides the 3b determiner+`Fst` machinery into a full sentence.

#[test]
fn subject_relative_clause_parses() {
    // "every cell line that affects HeLa is primary": the subject relative body
    // "affects HeLa" is a VP `S\NP` (sem λx. affects(hela, x)); the relativizer refines
    // → Σx:CellLine. affects(hela, x); `every`+`Fst` quantifies; `is primary` predicates
    // → ∀z:(Σx:CellLine. affects(hela, x)). is_primary(Fst z) : Prop.
    assert_parses_to_prop("every cell line that affects HeLa is primary");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every cell line that affects HeLa is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one subject-relative parse");
}

#[test]
fn which_relative_clause_parses() {
    // D62 grammar-gap batch: `which` is a relativizer too (restrictive; the non-restrictive comma
    // reading collapses here — the comma is S0-stripped, the contrast is semantic/deferred).
    assert_parses_to_prop("every cell line which affects HeLa is primary");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every cell line which affects HeLa is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one which-relative parse");
    // A sentence-initial wh-`which` must NOT be captured by the relativizer (no noun precedes).
    assert!(
        !index
            .parse("which cell line affects HeLa", &Identity)
            .is_empty(),
        "sentence-initial wh-which still parses (unaffected by the relativizer extension)"
    );
}

#[test]
fn object_relative_clause_parses() {
    // "every cell line that HeLa affects is primary": the object relative body "HeLa
    // affects" has an object gap — built by type-raising HeLa (NP → S/(S\NP)) then
    // forward-composing with `affects` → `S/NP` (sem λx. affects(x, hela)); the
    // relativizer refines → Σx:CellLine. affects(x, hela) → a kernel-checked Prop.
    assert_parses_to_prop("every cell line that HeLa affects is primary");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every cell line that HeLa affects is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one object-relative parse");
}

#[test]
fn relative_clause_refines_the_noun_to_a_sigma_over_its_base_type() {
    // The refined noun's restrictor is a Σ over the noun's CONCRETE base type CellLine
    // (the 3b move — so `body(x)` type-checks without kernel bounded quantification),
    // existentially this time: "a cell line that affects HeLa is primary".
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a cell line that affects HeLa is primary", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "exactly one existential subject-relative parse"
    );
    assert_parses_to_prop("a cell line that affects HeLa is primary");
}

#[test]
fn type_raising_keeps_plain_declaratives_single() {
    // Regression gate (the Eisner `TypeRaised`-can't-apply clause): with `T` now
    // globally available, "HeLa affects BRCA1" could be re-derived as T(HeLa) applied
    // to the VP "affects BRCA1". ENF blocks a raised functor from forward-applying, so
    // the canonical backward-application derivation is the ONLY parse.
    let (_layer, index) = index_over_bootstrap();
    assert_eq!(
        index.parse("HeLa affects BRCA1", &Identity).len(),
        1,
        "type-raising must not reintroduce a spurious declarative parse"
    );
    // And the polar question stays single too (T + the 5c composition both available).
    assert_eq!(
        index.parse("does HeLa affect BRCA1", &Identity).len(),
        1,
        "type-raising must not perturb the polar question's single parse"
    );
}

#[test]
fn nary_distributive_group_is_left_branching_single_parse() {
    // n-ary NP coordination builds a single left-branching group (the Phase-4
    // normal form, here enforced by `coordinate_np` requiring a plain-NP right
    // conjunct): "HeLa and BRCA1 and HeLa affect HeLa" → one parse, three
    // conjuncts.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 and HeLa affect HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "n-ary distributive group must have a single parse; got {}",
        forest.len()
    );
    assert_eq!(
        and_conjuncts(forest[0].sem()).map(|c| c.len()),
        Some(3),
        "three members ⇒ three conjuncts"
    );
}

#[test]
fn nary_coordination_has_a_single_left_branching_parse() {
    // Spurious-ambiguity control (D63 §8.4 Phase 4): without a normal form,
    // `A and B and C` yields two logically-equivalent parses (left- vs right-
    // branching `And`). The left-branching normal form keeps exactly one.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse(
        "HeLa affects BRCA1 and BRCA1 affects HeLa and HeLa affects BRCA1",
        &Identity,
    );
    assert_eq!(
        forest.len(),
        1,
        "n-ary coordination must have a single (left-branching) parse; got {}",
        forest.len()
    );
}

// ── D65 §2.2 / slice 2 — the lazy `Parser` over the form `ValueIndex` ──
//
// On a SHARED-storage chain (bootstrap + a child layer on the same storage), the
// `lexicon:form_index` `core:ValueIndex` is active, so `Parser::build` takes
// the lazy path: O(1) build, forms resolved on demand and memoised. This proves
// (a) the lazy path activates, (b) it is behaviourally lazy (no forms cached until
// a parse touches them), and (c) it yields the SAME parse forest as the eager scan.

/// Bootstrap + the demo domain on the SAME storage as the bootstrap chain, so the
/// declared `lexicon:form` `ValueIndex` is discoverable/active ⇒ lazy `Parser`.
/// The bootstrap+demo chain on SHARED storage, and the lexicon over it — for tests that assert on the
/// index itself (its laziness / coverage) rather than on a parse.
fn shared_lexicon() -> (Arc<Layer>, Arc<LexicalIndex>) {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources =
        esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles on bootstrap");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add demo resource");
    }
    let layer = Arc::new(b.build(ctx.head().storage().clone()));
    let ix = Arc::new(LexicalIndex::build(Arc::clone(&layer)));
    (layer, ix)
}

/// Same, on ISOLATED storage (so no `ValueIndex` is active and the index takes the eager path).
fn eager_lexicon() -> (Arc<Layer>, Arc<LexicalIndex>) {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources =
        esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles on bootstrap");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add demo resource");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let ix = Arc::new(LexicalIndex::build(Arc::clone(&layer)));
    (layer, ix)
}

/// Multiset of reduced-sem keys for a forest — order-independent equivalence.
fn sem_keys(forest: &[eigenius_kernel::dcg::Item]) -> Vec<String> {
    let mut keys: Vec<String> = forest
        .iter()
        .map(|p| {
            format!(
                "{:?} :: {:?}",
                p.cat(),
                readback_val(0, &eval(p.sem(), &Rho::Nil).expect("eval sem"))
            )
        })
        .collect();
    keys.sort();
    keys
}

#[test]
fn lazy_index_is_lazy_and_matches_eager() {
    let sentence = "every cell line affects HeLa";

    // Lazy (shared storage): nothing is cached until a parse touches a form. The INDEX is the thing
    // under test here, so we hold it directly and put a `Parser` over it (`Parser::over`) to drive the
    // parse — the lexicon and the parser are separate objects now, and this test is the one that cares.
    let (shared_layer, lazy_ix) = shared_lexicon();
    assert_eq!(
        lazy_ix.len(),
        0,
        "the lazy index caches no forms before any parse (it probes the ValueIndex on demand)"
    );
    let lazy = Parser::over(Arc::clone(&lazy_ix) as Arc<dyn LexicalLookup>, shared_layer);
    let lazy_forest = lazy.parse(sentence, &Identity);
    assert!(
        !lazy_ix.is_empty(),
        "after a parse the lazy index has memoised the forms its sentence touched"
    );
    assert!(
        !lazy_forest.is_empty(),
        "the lazy path must yield at least one felicitous parse"
    );

    // Eager (isolated storage): the same content scanned up front.
    let (eager_layer, eager_ix) = eager_lexicon();
    assert!(
        eager_ix.len() >= 6,
        "the eager index materialises every committed form up front"
    );
    let eager = Parser::over(eager_ix as Arc<dyn LexicalLookup>, eager_layer);
    let eager_forest = eager.parse(sentence, &Identity);

    // Behaviour-equivalence: identical parse forests (as reduced-sem multisets).
    assert_eq!(
        sem_keys(&lazy_forest),
        sem_keys(&eager_forest),
        "the lazy and eager paths must produce the same forest"
    );
}

// ── D63 Phase 1 — abbreviation-injection lever (document-preprocessing) ─────────
fn layer_on(parent: &Arc<Layer>, name: &str, src: &str) -> Arc<Layer> {
    let resources = esl::compile_against_layer(src, parent).expect("fixture compiles");
    let mut b = LayerBuilder::new(name, Some(Arc::clone(parent)));
    for r in resources {
        b.add_resource(r).expect("add fixture resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Inject one abbreviation binding onto the demo via the programmatic alias emitter
/// (`dcg::glossary::abbreviation_resources`, the actual Stage-2 code path — resources built directly,
/// no ESL round-trip) and return an index over the resulting chained doc layer.
fn demo_with_alias(demo: &Arc<Layer>, long: &str, concept: &str) -> Parser {
    let binding = AbbreviationBinding {
        abbr: "wsi",
        long_form: long,
        concept_iri: concept,
        doc_ns: "urn:eigenius:doc",
    };
    let res = abbreviation_resources(demo, &binding).expect("emit alias entry");
    let mut b = LayerBuilder::new("alias", Some(Arc::clone(demo)));
    for r in res {
        b.add_resource(r).expect("add alias resource");
    }
    Parser::build(Arc::new(b.build(LayerStorage::in_memory())))
}

/// D63 Phase 1 (the #1 CNL-v2 lever) × the kind-predication reshape (Phase A): an abbreviation grounded
/// to a **mass phenomenon** class — the `MSI = "microsatellite instability"` case, here `wsi →
/// Instability`, head noun `instability` mass — is emitted as `cat_n(concept, mass)`. A bare `wsi`, OOV
/// before, now recovers as a **closed** kind-predication: the bare-mass shift nominalizes the kind to an
/// entity (`kind_of(Instability)`), so "wsi affects HeLa" → `affects(hela, kind_of(Instability)) : Prop`
/// — a complete proposition, not the earlier open deferred quantifier. No parser change per document —
/// just the chained doc-scoped alias layer over the reshaped grammar.
#[test]
fn abbreviation_injection_recovers_bare_argument() {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = layer_on(ctx.head(), "demo", DEMO);
    let base = Parser::build(Arc::clone(&demo));
    let injected = demo_with_alias(&demo, "instability", "urn:eigenius:lexicon:Instability");

    // Base: `wsi` is OOV — no parse, closed or open.
    assert!(base.parse("wsi affects HeLa", &Identity).is_empty());
    assert!(
        base.parse_open("wsi affects HeLa", &Identity).1.is_empty(),
        "the abbreviation is unknown before the alias is injected"
    );

    // Injected: the bare mass subject recovers as a CLOSED kind-predication (reshape Phase A) — the
    // kind nominalized to an entity, not an open deferred quantifier.
    let closed = injected.parse("wsi affects HeLa", &Identity);
    assert_eq!(
        closed.len(),
        1,
        "the bare-mass abbreviation recovers a single closed parse"
    );
    assert!(
        pretty_term(closed[0].sem()).contains("kind_of(Instability)"),
        "the recovered subject is the grounded kind nominalized to an entity, got {}",
        pretty_term(closed[0].sem())
    );
}

/// D63 Phase 1 — the abbreviation/alias emitter keys on the grounded concept's ONTOLOGICAL KIND, so
/// the NP-vs-N denotational fork (witnessed earlier as a real fork) is resolved *at emission*, not
/// papered over by minting a named individual for every abbreviation (the wedge). The three kinds a
/// biomedical abbreviation grounds to, each getting the reading its kind licenses:
///
///   * **mass phenomenon** (MSI = "microsatellite instability") → `cat_n(C, mass)`, sem = the class:
///     a bare subject is a CLOSED kind-predication `kind_of(C)` (reshape Phase A), a prenominal modifier
///     is `compound_kind(x, C)` — the property as a classifier ("MSI cell lines").
///   * **count common noun** (CL = "cell line") → `cat_n(C, num_any)`: NOT bare-licensed (needs a
///     determiner, "the CL") — the wedge wrongly made it a bare named individual.
///   * **named individual** (WRN, an HGNC gene) → `cat_np(sty, sg)`, sem = the SAME instance: a bare
///     subject is a closed entity reference, a prenominal modifier is `compound(x, instance)`.
#[test]
fn abbreviation_emission_keys_on_ontological_kind() {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let demo = layer_on(ctx.head(), "demo", DEMO);

    // Mass phenomenon: bare subject → CLOSED kind-predication (reshape Phase A); modifier → compound_kind.
    let mass = demo_with_alias(&demo, "instability", "urn:eigenius:lexicon:Instability");
    let mass_subj = mass.parse("wsi affects HeLa", &Identity);
    assert_eq!(
        mass_subj.len(),
        1,
        "the bare mass subject is a single closed kind-predication"
    );
    assert!(
        pretty_term(mass_subj[0].sem()).contains("kind_of(Instability)"),
        "the mass subject nominalizes the kind to an entity (not a deferred quantifier)"
    );
    // As a prenominal classifier the mass noun reads ONLY as the kind classifier `compound_kind(x, C)`.
    // Because the bare-mass shift is type-raised (`S/(S\NP)`, not a `cat_np`), it cannot feed the
    // named-entity compound rule, so there is NO spurious `compound(x, kind_of(C))` duplicate (reshape
    // §7.5). This asserts the single reading — a regression guard against the duplicate returning.
    let mass_mod = mass.parse("a wsi cell line affects HeLa", &Identity);
    assert_eq!(mass_mod.len(), 1, "the mass modifier has a single reading");
    assert!(
        pretty_term(mass_mod[0].sem()).contains("compound_kind(")
            && !pretty_term(mass_mod[0].sem()).contains("kind_of("),
        "the phenomenon classifies the head noun by its kind (compound_kind), with no kind_of-entity compound"
    );

    // Count common noun: needs a determiner — no bare reading (the wedge over-licensed this).
    let count = demo_with_alias(&demo, "cell line", "urn:eigenius:lexicon:CellLine");
    assert!(
        count.parse("wsi affects HeLa", &Identity).is_empty()
            && count.parse_open("wsi affects HeLa", &Identity).1.is_empty(),
        "a count abbreviation is not a bare subject (it needs a determiner)"
    );

    // Named individual: bare subject → CLOSED reference reusing the SAME instance; modifier → compound.
    let indiv = demo_with_alias(&demo, "brca one", "urn:eigenius:lexicon:brca1");
    let subj = indiv.parse("wsi affects HeLa", &Identity);
    assert_eq!(
        subj.len(),
        1,
        "a named individual is a closed bare reference"
    );
    assert!(
        pretty_term(subj[0].sem()).contains("brca1"),
        "the alias reuses the SAME instance — no fresh individual is minted"
    );
    let indiv_mod = indiv.parse("a wsi cell line affects HeLa", &Identity);
    assert_eq!(indiv_mod.len(), 1);
    let mod_sem = pretty_term(indiv_mod[0].sem());
    assert!(
        mod_sem.contains("compound(") && !mod_sem.contains("compound_kind("),
        "an individual modifier is compound(x, instance), not a kind classifier"
    );
}

/// D63 Phase 1 — the full deterministic Stage-A pipeline end to end on the demo, the MSI scenario:
/// **extract** a `Long Form (ABBR)` definition (Schwartz-Hearst), **ground** the long form to an
/// existing concept class (retrieve-first), **emit** the alias entry via `glossary_resources`, inject
/// it as a chained layer, and confirm the bare abbreviation — OOV before — now **parses**. The demo's
/// `instability` is a mass phenomenon (like MSI's head), so the recovered bare argument is a CLOSED
/// kind-predication (reshape Phase A), exactly as `MSI contributes to cancers` should encode.
#[test]
fn abbreviation_pipeline_end_to_end() {
    let (demo_layer, _) = index_over_bootstrap();

    // Stage A · extract: `the instability (INS) …` → `INS ← instability`.
    let defs = extract_abbreviations("the instability (INS) was assayed for a gene");
    assert_eq!(
        defs,
        vec![AbbrDef {
            short_form: "INS".to_string(),
            long_form: "instability".to_string(),
            context: "the instability".to_string(),
        }],
        "Schwartz-Hearst extracts the parenthetical definition"
    );

    // Stage A · ground: `instability` → the Instability concept class (retrieve-first).
    let concept = ground_long_form(&demo_layer, &defs[0].long_form)
        .expect("the long form grounds to an existing concept");
    assert_eq!(concept.as_str(), "urn:eigenius:lexicon:Instability");

    // Stage A · emit + inject: the alias entry `cat_n(Instability, mass)`, sem = the class (mass,
    // because the long form's head noun `instability` is uncountable) — via `glossary_resources`.
    let res = glossary_resources(&demo_layer, &defs);
    let mut b = LayerBuilder::new("doc-glossary", Some(Arc::clone(&demo_layer)));
    for r in res {
        b.add_resource(r).expect("add glossary resource");
    }
    let doc_layer = Arc::new(b.build(LayerStorage::in_memory()));

    // Stage B · parse: bare `INS` was OOV before; now the alias recovers it as a CLOSED kind-predication
    // (the bare-mass shift nominalizes the kind — reshape Phase A).
    let base = Parser::build(Arc::clone(&demo_layer));
    let injected = Parser::build(doc_layer);
    assert!(
        base.parse("INS affects HeLa", &Identity).is_empty()
            && base.parse_open("INS affects HeLa", &Identity).1.is_empty(),
        "the abbreviation is unknown before the glossary is injected"
    );
    let closed = injected.parse("INS affects HeLa", &Identity);
    assert_eq!(
        closed.len(),
        1,
        "extract → ground → emit → inject recovers a closed kind-predication"
    );
    assert!(
        pretty_term(closed[0].sem()).contains("kind_of(Instability)"),
        "the recovered parse denotes the grounded kind, nominalized"
    );
}
