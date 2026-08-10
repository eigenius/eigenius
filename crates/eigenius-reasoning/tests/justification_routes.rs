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

//! The two ways a claim gets justified in `demo/prose-to-formulas`, exercised against the gate:
//!
//! - **Prose modus ponens** — `A` and `A → B` both parsed from sentences, nothing Declared;
//! - **a pinned literature rule** — a Declared `A → B` applied to a claim a sentence established.
//!
//! Each has a `Holds` case and a fail-closed case. (A third mechanism — generated shape rules,
//! one Declared rule per parse shape — lived here until D66 replaced it with transparent
//! definitions; `spec_poly` elimination is now exercised end-to-end by the demo's
//! `inference.esl`.)

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::{Exp, InductiveDecl};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::{ClaimSource, ReasoningInstitution, Warrant};
use serde_json::json;

const PRED: &str = "urn:eigenius:demo:onco-typed:RequiresActivity";
const GENE_A: &str = "urn:eigenius:demo:cls:WRN";
const ACT_A: &str = "urn:eigenius:demo:cls:helicase";
const GENE_B: &str = "urn:eigenius:demo:cls:BLM";
const ACT_B: &str = "urn:eigenius:demo:cls:exonuclease";

fn build_chain() -> Arc<Layer> {
    let mut core = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
        .unwrap()
    {
        core.add_resource(r).unwrap();
    }
    let core = Arc::new(core.build(LayerStorage::in_memory()));

    let mut refl = LayerBuilder::new("reflection", Some(core));
    for src in [
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
        include_str!("../../../ontologies/institution/institution-ontology.json"),
    ] {
        for r in eigon_json::parse_document(src).unwrap() {
            refl.add_resource(r).unwrap();
        }
    }
    let refl = Arc::new(refl.build(LayerStorage::in_memory()));

    let mut rsn = LayerBuilder::new("reasoning", Some(refl));
    for r in esl::compile(include_str!("../../../ontologies/reasoning/reasoning.esl")).unwrap() {
        rsn.add_resource(r).unwrap();
    }
    let rsn = Arc::new(rsn.build(LayerStorage::in_memory()));

    // Domain predicate over CLASSES, plus the classes the two sentences mention.
    let dom = r#"
        namespace core = "urn:eigenius:core";
        namespace onco = "urn:eigenius:demo:onco-typed";
        namespace cls  = "urn:eigenius:demo:cls";
        namespace ont  = "urn:eigenius:demo:ont";
        class cls:WRN { }
        class cls:BLM { }
        class cls:helicase { }
        class cls:exonuclease { }
        data onco:RequiresActivity : Set -> Set -> Prop { }
        data onco:HighConcentration : Set -> Prop { }
        data ont:requires : Set -> Set -> Prop { }
    "#;
    let mut d = LayerBuilder::new("domain", Some(rsn));
    for r in esl::compile(dom).expect("domain ESL compiles") {
        d.add_resource(r).unwrap();
    }
    Arc::new(d.build(LayerStorage::in_memory()))
}

/// A stand-in for a parsed sentence: `ont:requires(<gene>, <activity>)`. The real thing is a much
/// larger DCG term, but the shape machinery only cares that the argument classes OCCUR in it.
fn parsed(gene: &str, activity: &str) -> Exp {
    let i = Iri::parse("urn:eigenius:demo:ont:requires").unwrap();
    let decl = Arc::new(InductiveDecl {
        iri: i.clone(),
        name: i.local_name().to_string(),
        params: Vec::new(),
        indices: vec![
            (eigenius_kernel::nbe::term::Patt::Unit, Exp::Sort(1)),
            (eigenius_kernel::nbe::term::Patt::Unit, Exp::Sort(1)),
        ],
        sort: Exp::Sort(0),
        ctors: Vec::new(),
    });
    Exp::InductiveType(
        decl,
        vec![
            Exp::EigonClass(Iri::parse(gene).unwrap()),
            Exp::EigonClass(Iri::parse(activity).unwrap()),
        ],
    )
}

fn claim_resources(claim_iri: &str, prop: &Exp, n: usize) -> Vec<Resource> {
    let iri = |s: &str| Iri::parse(s).unwrap();
    let mut c = Resource::new(iri(claim_iri));
    c.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DERIVED_RESOURCE))]),
    );
    c.set(iri(wk::CANONICAL_PROPOSITION), encode_type(prop).unwrap());
    let mut t = Resource::new(iri(&format!("urn:eigenius:demo:shape:trace_{n}")));
    t.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::PROGRAM_TRACE))]),
    );
    t.set(
        iri(wk::REFLECTION_RESOURCE),
        Value::ResourceRef(iri(claim_iri)),
    );
    t.set(
        iri("urn:eigenius:reflection:source"),
        Value::String("DCG parse (D63)".into()),
    );
    t.set(
        iri("urn:eigenius:reflection:timestamp"),
        Value::String("2026-08-03T00:00:00Z".into()),
    );
    vec![c, t]
}

fn verdict(base: &Arc<Layer>, rs: Vec<Resource>, sentence_iri: &Iri) -> (String, Option<String>) {
    let sentence = rs
        .iter()
        .find(|r| r.id() == Some(sentence_iri))
        .expect("sentence present")
        .clone();
    let mut b = LayerBuilder::new("doc", Some(Arc::clone(base)));
    for r in rs {
        b.add_resource(r).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let ctx = ExecutionContext::new(
        layer,
        "shape-test",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let out = do_validate_justification(&ReasoningInstitution::new(), &sentence, &ctx)
        .expect("gate runs")
        .output;
    (
        out.get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        out.get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
            .and_then(Value::as_str)
            .map(str::to_owned),
    )
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// Prose modus ponens — both premises Derived, nothing Declared
// ═══════════════════════════════════════════════════════════════════════════════════════════════

use eigenius_reasoning::grade::ProseModusPonens;

/// `A` and `A → B` as two separate parsed sentences; conclude `B`.
///
/// This is the shape the grammar's `if` gives: `"S₁ if S₂" ⇒ ⟦S₂⟧ → ⟦S₁⟧`. Both `app` premises are
/// `IsDerivedAs` witnesses minted by the parser's `ProgramTrace`, so the conclusion is warranted
/// without any human declaring an implication.
#[test]
fn modus_ponens_over_two_parsed_sentences() {
    let base = build_chain();
    let a = parsed(GENE_A, ACT_A);
    let b = parsed(GENE_B, ACT_B);
    let conditional = Exp::Arrow(Box::new(a.clone()), Box::new(b.clone()));

    let mut rs = claim_resources("urn:eigenius:demo:mp:claim_rule", &conditional, 10);
    rs.extend(claim_resources(
        "urn:eigenius:demo:mp:claim_premise",
        &a,
        11,
    ));

    let concl = ProseModusPonens {
        rule_claim_iri: "urn:eigenius:demo:mp:claim_rule",
        premise_claim_iri: "urn:eigenius:demo:mp:claim_premise",
        premise: &a,
    }
    .conclude(
        &conditional,
        &ClaimSource {
            stem: "urn:eigenius:demo:mp:concl",
            warrant: Warrant::Declared,
            declared_by: "demo:prose-mp",
            timestamp: "2026-08-03T00:00:00Z",
        },
    )
    .expect("conclusion builds");

    rs.extend(concl.resources.clone());
    let (ctor, diag) = verdict(&base, rs, &concl.sentence_iri);
    assert_eq!(ctor, "Holds", "diagnostic: {diag:?}");
}

/// Fail closed: the premise must be the conditional's antecedent, not merely something like it.
#[test]
fn modus_ponens_refuses_a_premise_that_is_not_the_antecedent() {
    let a = parsed(GENE_A, ACT_A);
    let other = parsed(GENE_B, ACT_B);
    let conditional = Exp::Arrow(Box::new(a), Box::new(parsed(GENE_A, ACT_B)));
    let err = ProseModusPonens {
        rule_claim_iri: "urn:eigenius:demo:mp:claim_rule",
        premise_claim_iri: "urn:eigenius:demo:mp:claim_premise",
        premise: &other,
    }
    .conclude(
        &conditional,
        &ClaimSource {
            stem: "urn:eigenius:demo:mp:concl",
            warrant: Warrant::Declared,
            declared_by: "demo:prose-mp",
            timestamp: "2026-08-03T00:00:00Z",
        },
    )
    .map(|_| ())
    .expect_err("a different proposition must not pass as the antecedent");
    assert!(err.to_string().contains("SAME term"), "{err}");
}

// ═══════════════════════════════════════════════════════════════════════════════════════════════
// A pinned literature rule, applied to a measured claim
// ═══════════════════════════════════════════════════════════════════════════════════════════════

use eigenius_reasoning::grade::ChainRuleApplication;

const LIT_RULE: &str = "urn:eigenius:demo:lit:rule_conc_implies_helicase";
const CONC: &str = "urn:eigenius:demo:onco-typed:HighConcentration";

/// The full arc: a measurement sentence establishes `A`; a rule pinned from the literature says
/// `A → B`; applying it justifies `B` — which is the *activity* sentence's content, now warranted
/// by inference rather than by the document asserting it.
#[test]
fn a_pinned_literature_rule_justifies_a_sentence_by_inference() {
    let base = build_chain();

    // (1) The measurement claim, established by an ordinary parsed sentence. Stand-in for the
    //     domain reading of "MSI cancer models had the high concentration of thymidine."
    let a = json!({ "ctor": "App", "args": [
        { "ctor": "ConstRef", "args": [CONC] },
        { "ctor": "ConstRef", "args": [ACT_B] }
    ]});
    let b = json!({ "ctor": "App", "args": [
        { "ctor": "App", "args": [
            { "ctor": "ConstRef", "args": [PRED] },
            { "ctor": "ConstRef", "args": [GENE_A] }
        ]},
        { "ctor": "ConstRef", "args": [ACT_A] }
    ]});

    let iri = |s: &str| Iri::parse(s).unwrap();
    let mut rs = Vec::new();

    // A prior ReasoningSentence asserting A. Committing it mints IsVerifiedAs(sentence, A) — the
    // lemma-citation path (D54).
    let prior = "urn:eigenius:demo:lit:measured";
    let mut m = Resource::new(iri(prior));
    m.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:reasoning:ReasoningSentence",
        ))]),
    );
    m.set(
        iri("urn:eigenius:reasoning:proposition"),
        Value::Json(a.clone()),
    );
    m.set(
        iri("urn:eigenius:reasoning:justification"),
        Value::Json(
            json!({ "ctor": "DeclaredEvidence", "args": ["urn:eigenius:demo:lit:meas_src"] }),
        ),
    );
    m.set(iri("urn:eigenius:reasoning:certificate"), Value::Json(json!({
        "ctor": "App", "args": [
            { "ctor": "App", "args": [
                { "ctor": "App", "args": [
                    { "ctor": "CtorApp", "args": ["urn:eigenius:reasoning:JustifiedBy", "declared"] },
                    { "ctor": "LitString", "args": ["urn:eigenius:demo:lit:meas_src"] }]},
                a.clone() ]},
            { "ctor": "UnitVal", "args": [] }]})));
    // …with its own Declared source + trace so that certificate stands.
    let mut src = Resource::new(iri("urn:eigenius:demo:lit:meas_src"));
    src.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE))]),
    );
    src.set(iri(wk::CANONICAL_PROPOSITION), Value::Json(a.clone()));
    src.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("demo:measurement".into()),
    );
    let mut src_t = Resource::new(iri("urn:eigenius:demo:lit:meas_src_trace"));
    src_t.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARATION_TRACE))]),
    );
    src_t.set(
        iri(wk::REFLECTION_RESOURCE),
        Value::ResourceRef(iri("urn:eigenius:demo:lit:meas_src")),
    );
    src_t.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("demo:measurement".into()),
    );
    src_t.set(
        iri("urn:eigenius:reflection:timestamp"),
        Value::String("2026-08-03T00:00:00Z".into()),
    );
    rs.extend([src, src_t, m]);

    // (2) The literature rule: A → B, pinned and cited. Authorable because it is in DOMAIN
    //     vocabulary — plain ConstRefs, no Σ-binders.
    let mut rule = Resource::new(iri(LIT_RULE));
    rule.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE))]),
    );
    rule.set(
        iri(wk::CANONICAL_PROPOSITION),
        Value::Json(json!({ "ctor": "Pi", "args": ["", a.clone(), b.clone()] })),
    );
    rule.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("literature:smith-2024-§3".into()),
    );
    rule.set(
        iri("urn:eigenius:reflection:rationale"),
        Value::String("Published: high concentration implies the helicase requirement.".into()),
    );
    let mut rule_t = Resource::new(iri(&format!("{LIT_RULE}_trace")));
    rule_t.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARATION_TRACE))]),
    );
    rule_t.set(
        iri(wk::REFLECTION_RESOURCE),
        Value::ResourceRef(iri(LIT_RULE)),
    );
    rule_t.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("literature:smith-2024-§3".into()),
    );
    rule_t.set(
        iri("urn:eigenius:reflection:timestamp"),
        Value::String("2026-08-03T00:00:00Z".into()),
    );
    rs.extend([rule, rule_t]);

    // (3) Apply the rule to the measured claim.
    let concl = ChainRuleApplication {
        rule_iri: LIT_RULE,
        antecedent_sentence_iri: prior,
        antecedent: &a,
        consequent: &b,
    }
    .conclude(&ClaimSource {
        stem: "urn:eigenius:demo:lit:concl",
        warrant: Warrant::Declared,
        declared_by: "demo:inference",
        timestamp: "2026-08-03T00:00:00Z",
    })
    .expect("conclusion builds");
    rs.extend(concl.resources.clone());

    let (ctor, diag) = verdict(&base, rs, &concl.sentence_iri);
    assert_eq!(ctor, "Holds", "diagnostic: {diag:?}");
}
