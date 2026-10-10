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

//! Adverbs import at Luo & Shi's `ADV` type (D62 §8.7.5, reopened by eigenius#270).
//!
//! `data.adv` was deferred because there was no type for a predicate modifier. #270 adopted Luo &
//! Shi 2026 and `ontology:adv_*` instantiates `ADV = (e -> t) -> (e -> t)` at a PP's object; a bare
//! adverb is the same thing without the object.
//!
//! What the emitted layer has to satisfy: it compiles over the real bootstrap, the axiom carries the
//! `ADV` type, the entry's sem is a term that KERNEL-GATES at that type, and the modifier conjoins
//! rather than vanishing — the identity sem the parser's derivational rule seeds is what this
//! replaces.

use std::sync::Arc;

use eigenius_kernel::dcg::lexicon::gate_entry;
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::validation::Validator;
use eigenius_wordnet::convert::{render_document, MassNouns, SenseRanks};
use eigenius_wordnet::governance::Governance;
use eigenius_wordnet::import::{select_synsets, SeedSpec};
use eigenius_wordnet::wndb::{parse_data_line, Pos};

/// One adverb synset, rendered and stood up over the bootstrap.
fn adverb_layer() -> (String, Arc<Layer>) {
    let syn = parse_data_line("00064889 02 r 01 deliberately 0 001 \\ 00748310 a 0000 | with intention; in an intentional manner").expect("parse data.adv line");
    let (doc, rep) = render_document(
        &[syn],
        &SenseRanks::new(),
        &MassNouns::new(),
        &Governance::default(),
    );
    assert_eq!(rep.adv_axioms, 1, "one adverb synset → one ADV axiom");
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let resources = esl::compile(&doc, ctx.head())
        .unwrap_or_else(|e| panic!("adverb ESL compiles: {e:?}\n{doc}"));
    let mut b = LayerBuilder::new("wn-adv", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    (doc, Arc::new(b.build(LayerStorage::in_memory())))
}

#[test]
fn an_adverb_synset_compiles_and_validates() {
    let (doc, layer) = adverb_layer();
    let errors: Vec<String> = Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "the adverb layer validates: {errors:#?}\n{doc}"
    );
}

/// The axiom is at `ADV`, and the entry's sem is the CONJOINING term — not the identity the
/// derivational rule seeds. Modifier drop then follows from the entry, which is Luo & Shi's point.
#[test]
fn the_adverb_is_typed_adv_and_conjoins() {
    let (doc, _layer) = adverb_layer();
    assert!(
        doc.contains(
            "axiom wn:deliberately_r_00064889 : (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop"
        ) || doc.contains("(lexicon:Entity -> Prop) -> lexicon:Entity -> Prop"),
        "the axiom carries Luo & Shi's ADV type:\n{doc}"
    );
    assert!(
        doc.contains("logic:And(V(s), wn:"),
        "the entry conjoins the modifier instead of dropping it:\n{doc}"
    );
    assert!(
        !doc.contains("fun (x : lexicon:Entity) => x"),
        "no identity sem survives in an imported adverb:\n{doc}"
    );
}

/// Both manner positions ship, and the forward one BINDS the clause feature.
///
/// `eigenius_kernel::dcg::category::adverb_modifier_cats` builds these same two categories for the
/// parser's derivational fallback, and the two must agree or an imported adverb attaches where a
/// derived one does not. Its forward pre-modifier leaves the clause feature a variable so the
/// adverb hands back whatever it consumed — `adj`, `pred`, `fin`.
///
/// Emitting `cat_s(dcl, fin)` there instead cost six grammar-gaps on the reference page
/// (2026-10-09): `lexicon:is_copula` takes its complement at `cat_s(dcl, adj)`, so «were
/// selectively essential» and «were highly concordant» give the adverb an ADJECTIVAL predicate to
/// modify, and a `fin`-only modifier cannot reach it. The backward post-modifier is verbal only and
/// returns the `fin` it accepts, so it stays finite.
#[test]
fn both_manner_positions_ship_and_the_forward_one_binds_the_feature() {
    let (doc, _) = adverb_layer();
    let fwd = doc
        .lines()
        .find(|l| l.contains("lexicon:cat      = type_expr( lexicon:cat_fin_forall"))
        .expect("a forward pre-modifier binding the clause feature");
    assert!(
        fwd.contains("lexicon:cat_num_forall") && fwd.contains("lexicon:fwd(lexicon:m_all"),
        "forward pre-modifier binds fin AND num: {fwd}"
    );
    assert!(
        !fwd.contains("lexicon:cat_s(lexicon:dcl, lexicon:fin)"),
        "the forward pre-modifier must NOT pin the clause feature to `fin` — that is the \
         2026-10-09 regression: {fwd}"
    );
    let bwd = doc
        .lines()
        .find(|l| {
            l.contains("lexicon:cat      = type_expr( lexicon:cat_num_forall")
                && l.contains("lexicon:bwd(lexicon:m_all")
        })
        .expect("a backward post-modifier");
    assert!(
        bwd.contains("lexicon:cat_s(lexicon:dcl, lexicon:fin)"),
        "the backward post-modifier is verbal only, so it stays finite: {bwd}"
    );
}

/// Every emitted entry must pass the FELICITY GATE — `⟦cat⟧ ≡ sem_type` and the sem inhabits
/// `⟦cat⟧`. This is the gate the importer routes through at commit, so an entry that fails it would
/// be a fail-closed finding rather than a silent drop; asserting it here keeps that off the reseed.
#[test]
fn every_adverb_entry_passes_the_felicity_gate() {
    let (doc, layer) = adverb_layer();
    let mut gated = 0;
    for (iri, r) in layer.iter_resources() {
        if !r.is_instance_of(
            &eigenius_kernel::ontology::Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap(),
        ) {
            continue;
        }
        let cat = gate_entry(&layer, &r)
            .unwrap_or_else(|e| panic!("{iri} must pass the felicity gate: {e}\n{doc}"));
        // ⟦cat⟧ must be `(Entity -> Prop) -> (Entity -> Prop)` — Luo & Shi's ADV. Checked on the
        // TERM, not its printed form: `pretty_term` renders `Arrow(a, b)` as `a → b` with no
        // parentheses on the left, so this type and `Entity -> (Prop -> (Entity -> Prop))` print
        // identically.
        let entity_to_prop = |e: &Exp| {
            matches!(e, Exp::Arrow(a, b)
                if matches!(**a, Exp::EigonClass(ref i) if i.as_str() == "urn:eigenius:lexicon:Entity")
                    && matches!(**b, Exp::Sort(ref l) if l.is_nat(0)))
        };
        match &cat {
            Exp::Arrow(arg, res) if entity_to_prop(arg) && entity_to_prop(res) => {}
            other => panic!(
                "{iri}: ⟦cat⟧ must be ADV = (Entity -> Prop) -> (Entity -> Prop), got {:?}",
                other
            ),
        }
        gated += 1;
    }
    assert_eq!(gated, 2, "one lemma x two manner positions");
}

/// **The SELECTOR offers adverb synsets.**
///
/// Every other test here hands `render_document` a synset it built, so none of them goes through
/// `select_synsets` — and the adverb deferral lived in three places, not one: the converter's match
/// arm, the CLI's default `--pos`, and the selector, which loaded only `data.verb` and `data.adj`.
/// With the first fixed and the other two not, a full reseed reported **`0 adv axioms` over 114038
/// selected synsets** while all four of the tests above passed.
///
/// Reads the real `data.adv`, so it needs the provisioned dict.
#[test]
fn the_selector_offers_adverb_synsets() {
    let dict = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../references/WordNet-3.0/dict"
    ));
    if !dict.join("data.adv").exists() {
        eprintln!(
            "SKIP: WordNet dict not provisioned under {}",
            dict.display()
        );
        return;
    }
    let spec = SeedSpec {
        all: false,
        limit: Some(50),
        seeds: Vec::new(),
        pos: vec![Pos::Adv],
    };
    let chosen = select_synsets(dict, &spec).expect("read data.adv");
    let adverbs = chosen.iter().filter(|s| s.pos == Pos::Adv).count();
    assert_eq!(
        adverbs, 50,
        "select_synsets must offer adverb synsets when the spec asks for them; it loaded only \
         data.verb and data.adj until 2026-10-09"
    );
}

/// The CLI's default `--pos` includes `adv`, so a plain `--all` reseed imports adverbs. `--all`
/// bounds the SELECTION, not the parts of speech, which is why leaving `adv` out of the default was
/// silent rather than an error.
#[test]
fn the_default_pos_list_includes_adv() {
    let src = include_str!("../src/bin/wordnet_import.rs");
    assert!(
        src.contains(r#"default_value = "noun,verb,adj,adv""#),
        "the importer's default --pos must include adv, else the converter's adverb arm never fires"
    );
}

/// **A degree synset gets no manner entries, but keeps its axiom.**
///
/// The reference grammars never assign `less`/`more`/`most` the manner VP modifier `s\np/(s\np)`
/// this importer emits — `references/openccg/test/lexicon.xml` gives `more` JJR/RBR over `n`, `n/n`
/// and predicative `s[adj]\np`, and `grammars/comic` files it under `family="Adjective"`. Imported
/// as a manner adverb, `less` won the selection on «The lines from rare lineages were less dependent
/// on WRN» with a reading that asserts the lines ARE dependent and then modifies the manner —
/// inverting the sentence.
///
/// The axiom still ships so the sense does not dangle; only the per-lemma entries are withheld.
#[test]
fn a_degree_synset_yields_no_manner_entries() {
    let syn = parse_data_line(
        "00099527 02 r 02 less 0 to_a_lesser_extent 0 001 ! 00099341 r 0101 | used to form the comparative",
    )
    .expect("parse the `less` data.adv line");
    let (doc, rep) = render_document(
        &[syn],
        &SenseRanks::new(),
        &MassNouns::new(),
        &Governance::default(),
    );
    assert_eq!(rep.degree_adverb_skipped, 2, "both lemmas withheld:\n{doc}");
    assert_eq!(
        rep.adv_axioms, 1,
        "the axiom still ships, so the sense does not dangle"
    );
    assert!(
        !doc.contains("lexicon:form     = \"less\""),
        "no manner entry on a degree word:\n{doc}"
    );
    assert!(
        doc.contains("axiom wn:r00099527"),
        "the axiom is still declared:\n{doc}"
    );
}

/// A plain manner adverb is unaffected by the degree skip.
#[test]
fn a_manner_synset_still_gets_its_entries() {
    let syn = parse_data_line(
        "00448282 02 r 01 selectively 0 001 \\ 00065184 a 0101 | in a selective manner",
    )
    .expect("parse the `selectively` data.adv line");
    let (doc, rep) = render_document(
        &[syn],
        &SenseRanks::new(),
        &MassNouns::new(),
        &Governance::default(),
    );
    assert_eq!(rep.degree_adverb_skipped, 0);
    assert!(
        doc.contains("lexicon:form     = \"selectively\""),
        "a manner adverb keeps its entries:\n{doc}"
    );
}
