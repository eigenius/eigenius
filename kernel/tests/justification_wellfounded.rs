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

//! P6's exit gates: the transitive cycle is rejected, and the shared-class case is not.

use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::esl;
use eigenius_kernel::justification::wellfounded::{self, WellFoundedError};
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::validation::{ValidationRule, Validator};

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

/// Stack one ESL source on the bootstrap head.
fn layer_with(src: &str, name: &str) -> Arc<Layer> {
    let ctx = bootstrap().expect("bootstrap seeds");
    let head = Arc::clone(ctx.head());
    let resources = esl::compile_against_layer(src, &head)
        .unwrap_or_else(|errs| panic!("{name} failed to compile: {errs:?}"));
    let mut b = LayerBuilder::new(name, Some(head));
    for r in resources {
        b.add_resource(r).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Stack two sources as two layers, so the cycle is genuinely cross-layer.
fn two_layers(first: &str, second: &str) -> Arc<Layer> {
    let lower = layer_with(first, "probe-lower");
    let resources = esl::compile_against_layer(second, &lower)
        .unwrap_or_else(|errs| panic!("probe-upper failed to compile: {errs:?}"));
    let mut b = LayerBuilder::new("probe-upper", Some(lower));
    for r in resources {
        b.add_resource(r).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

const HEADER: &str = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace prov          = "urn:eigenius:prov";
namespace agent         = "urn:eigenius:prov:agent";
namespace probe         = "urn:eigenius:probe";

data probe:P : Prop { }
data probe:Q : Prop { }
"#;

/// **The paper's retroactive-upgrade cycle, across two layers.**
///
/// A later declaration can upgrade a bare observation into an application, which puts a
/// backward edge into the derivation graph. A single-step structural check cannot see a
/// cycle formed by two such upgrades in different layers, which is why the condition is
/// evaluated over the transitive closure.
///
/// Layer 1 commits `concl_p`, whose only ground is `concl_q`. Layer 2 commits `concl_q`,
/// whose only ground is `concl_p`. Neither is well-founded: every alternative in each
/// one's support passes back through it.
#[test]
fn a_two_layer_cycle_is_rejected() {
    let lower = format!(
        r#"{HEADER}
resource probe:concl_p : justification:Conclusion {{
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               verified("urn:eigenius:probe:concl_q", probe:Q),
               justification:Certificate(Verified("urn:eigenius:probe:concl_q"), probe:P) )
    );
}}
"#
    );
    let upper = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace probe         = "urn:eigenius:probe";

resource probe:concl_q : justification:Conclusion {
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               verified("urn:eigenius:probe:concl_p", probe:P),
               justification:Certificate(Verified("urn:eigenius:probe:concl_p"), probe:Q) )
    );
}
"#;
    let layer = two_layers(&lower, upper);

    for target in ["urn:eigenius:probe:concl_p", "urn:eigenius:probe:concl_q"] {
        let err = wellfounded::check(&layer, &iri(target))
            .expect_err("a conclusion whose only ground cycles back to it must be rejected");
        let WellFoundedError::Circular { path, .. } = &err else {
            panic!("expected a Circular error, got {err:?}");
        };
        assert!(
            path.len() >= 2,
            "the diagnostic must name a witnessing cycle, got {path:?}"
        );
        assert!(
            err.to_string().contains("not well-founded"),
            "diagnostic should say what is wrong: {err}"
        );
    }
}

/// **The false-rejection this check must not make.**
///
/// A conclusion's PROPOSITION and its PREMISE may reference the same class, and that is
/// ordinary — the whole point of a bridge is to talk about the same vocabulary on both
/// sides. `core:mentions` records proposition edges and justification edges from the same
/// subject with no predicate to tell them apart, so an edge-set cycle walk reports this
/// as a cycle. Reading the decoded TERM does not: a class is not a premise.
#[test]
fn a_shared_class_between_proposition_and_premise_is_not_a_cycle() {
    let src = format!(
        r#"{HEADER}
// The premise is Declared, so the condition is vacuous on it — and `probe:P`
// appears in BOTH the premise's proposition and the conclusion's.
resource probe:premise : justification:Claim {{
    prov:was_attributed_to = agent:eigenius_core_team;
    reflection:canonical_proposition = type_expr( probe:P );
}}

resource probe:concl_shared : justification:Conclusion {{
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               declared("urn:eigenius:probe:premise", probe:P),
               justification:Certificate(Declared("urn:eigenius:probe:premise"), probe:P) )
    );
}}
"#
    )
    .replace(
        "namespace probe         = \"urn:eigenius:probe\";",
        "namespace probe         = \"urn:eigenius:probe\";\nnamespace reflection    = \"urn:eigenius:reflection\";",
    );
    let layer = layer_with(&src, "probe-shared");

    wellfounded::check(&layer, &iri("urn:eigenius:probe:concl_shared"))
        .expect("a conclusion sharing a class with its premise is not circular");
}

/// The carve-out, stated as a test. A Declared premise has no support to inspect, so the
/// condition is vacuous on it — required, not convenient: Artemov's constant
/// specifications permit self-referential axioms `c : A(c)`, and that self-referentiality
/// is strictly necessary for realizing certain S4 theorems in LP.
#[test]
fn a_declared_premise_is_vacuously_well_founded() {
    let layer = layer_with(HEADER, "probe-empty");
    wellfounded::check(&layer, &iri("urn:eigenius:probe:not_on_chain"))
        .expect("anything with no support to inspect is vacuously well-founded");
}

/// **The case that decides where this check lives.**
///
/// `Sum(a, b)` is carried by either branch alone. A cycle through `a` while `b` is
/// acyclic therefore leaves the conclusion WELL-FOUNDED — the `b` alternative still
/// carries it.
///
/// `core:mentions` records both branches' edges undifferentiated, so a cycle walk over
/// the edge set rejects this commit. That is a false rejection, and by the
/// wrong-direction-safe reasoning this refactor uses throughout it is the losing
/// direction: an incorrect reject destroys data, where an incorrect admit is caught by
/// the next check. No predicate filter recovers the distinction, because it is not in
/// the edges at all — which is why the support ALGEBRA moved into the kernel and this
/// condition is evaluated over decoded terms.
#[test]
fn a_cycle_in_one_sum_branch_does_not_reject_when_the_other_carries_it() {
    let lower = format!(
        r#"{HEADER}
resource probe:solid : justification:Claim {{
    prov:was_attributed_to = agent:eigenius_core_team;
    reflection:canonical_proposition = type_expr( probe:P );
}}

// Grounded on `Sum(Verified(concl_cyclic), Declared(solid))`: the left branch cycles
// back through this conclusion, the right branch is a Declared premise with no support
// to inspect. One good alternative is enough.
resource probe:concl_sum : justification:Conclusion {{
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               sum_r( probe:P,
                      Verified("urn:eigenius:probe:concl_cyclic"),
                      Declared("urn:eigenius:probe:solid"),
                      verified("urn:eigenius:probe:concl_cyclic", probe:P),
                      declared("urn:eigenius:probe:solid", probe:P) ),
               justification:Certificate(
                   justification:Sum(
                       Verified("urn:eigenius:probe:concl_cyclic"),
                       Declared("urn:eigenius:probe:solid")),
                   probe:P) )
    );
}}
"#
    )
    .replace(
        "namespace probe         = \"urn:eigenius:probe\";",
        "namespace probe         = \"urn:eigenius:probe\";\nnamespace reflection    = \"urn:eigenius:reflection\";",
    );
    // The upper layer closes the loop: `concl_cyclic` rests only on `concl_sum`.
    let upper = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace probe         = "urn:eigenius:probe";

resource probe:concl_cyclic : justification:Conclusion {
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               verified("urn:eigenius:probe:concl_sum", probe:P),
               justification:Certificate(Verified("urn:eigenius:probe:concl_sum"), probe:P) )
    );
}
"#;
    let layer = two_layers(&lower, upper);

    // The Sum-grounded conclusion survives: its right branch never touches the cycle.
    wellfounded::check(&layer, &iri("urn:eigenius:probe:concl_sum"))
        .expect("a cycle in one Sum branch must not reject a conclusion the other branch carries");

    // And so does the conclusion resting on it. `concl_cyclic` cites `concl_sum`, which
    // IS well-founded — via the branch that avoids the loop — so the chain terminates and
    // there is no cycle to find. An edge-set walk sees `concl_sum → concl_cyclic →
    // concl_sum` and rejects both; reading the support sees that one of the two edges
    // leaving `concl_sum` is an ALTERNATIVE, not a requirement, and rejects neither.
    wellfounded::check(&layer, &iri("urn:eigenius:probe:concl_cyclic"))
        .expect("the loop is broken at concl_sum, so nothing downstream of it is circular either");
}

/// The check is wired into commit, not merely available as a library call.
///
/// Rule 23 runs from `validate_resource`, so a layer carrying the cycle fails structural
/// validation — which is what "rejected at commit" means, and what the live loader runs.
#[test]
fn the_cycle_is_rejected_by_the_validator_not_just_the_library() {
    let lower = format!(
        r#"{HEADER}
resource probe:concl_p : justification:Conclusion {{
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               verified("urn:eigenius:probe:concl_q", probe:Q),
               justification:Certificate(Verified("urn:eigenius:probe:concl_q"), probe:P) )
    );
}}
"#
    );
    let upper = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace probe         = "urn:eigenius:probe";

resource probe:concl_q : justification:Conclusion {
    justification:subject_iri = "urn:eigenius:probe:subject";
    justification:judgement = type_expr(
        holds( eigentt:logic_kernel,
               verified("urn:eigenius:probe:concl_p", probe:P),
               justification:Certificate(Verified("urn:eigenius:probe:concl_p"), probe:Q) )
    );
}
"#;
    let layer = two_layers(&lower, upper);
    let errors = Validator::new(layer).validate();

    let cyclic: Vec<_> = errors
        .iter()
        .filter(|e| e.rule == ValidationRule::NotWellFounded)
        .collect();
    assert!(
        !cyclic.is_empty(),
        "the validator must reject the cycle; got {} error(s), none NotWellFounded: {:#?}",
        errors.len(),
        errors.iter().take(5).collect::<Vec<_>>()
    );
}
