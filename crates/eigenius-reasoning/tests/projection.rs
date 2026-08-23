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

//! D73 §1.2's projections, asked of the flagship chain's own conclusion (eigenius#204).
//!
//! `wrn:concl_wrn_selective` — *WRN is selectively essential in MSI* — is the WRN encoding's
//! headline claim. Its `JustificationTerm`
//! (`experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl:144`):
//!
//! ```text
//! App(App(SpecStr(DeclaredEvidence(discovery_rule), "WRN"),
//!         DerivedEvidence(dd_achilles)),
//!     DerivedEvidence(dd_drive))
//! ```
//!
//! All `App`, no `Sum`: one alternative, three grounds, no fallback anywhere. That shape is what
//! makes the counterfactual answers below sharp — every ground is load-bearing.

use std::sync::Arc;

use eigenius_kernel::nbe::term::{Exp, InductiveDecl};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_reasoning::project::{
    cited_iris, is_fully_verified, leaves_of, support, survives_without, Ground,
};

const RULE: &str = "urn:eigenius:pub:wrn:discovery_rule";
const ACHILLES: &str = "urn:eigenius:pub:wrn:dd_achilles";
const DRIVE: &str = "urn:eigenius:pub:wrn:dd_drive";

fn decl() -> Arc<InductiveDecl> {
    Arc::new(InductiveDecl {
        iri: Iri::parse("urn:eigenius:reasoning:JustificationTerm").unwrap(),
        name: "JustificationTerm".to_string(),
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: Vec::new(),
    })
}
fn ctor(name: &str, args: Vec<Exp>) -> Exp {
    Exp::InductiveCtor(decl(), name.to_string(), args)
}
fn leaf(name: &str, iri: &str) -> Exp {
    ctor(name, vec![Exp::LitString(iri.to_string())])
}

/// The WRN conclusion's term, transcribed from the chain.
fn wrn_conclusion() -> Exp {
    ctor(
        "App",
        vec![
            ctor(
                "App",
                vec![
                    ctor(
                        "SpecStr",
                        vec![
                            leaf("DeclaredEvidence", RULE),
                            Exp::LitString("WRN".to_string()),
                        ],
                    ),
                    leaf("DerivedEvidence", ACHILLES),
                ],
            ),
            leaf("DerivedEvidence", DRIVE),
        ],
    )
}

#[test]
fn the_wrn_conclusion_rests_on_one_declaration_and_two_recomputes() {
    let t = wrn_conclusion();

    // No Sum anywhere: a single chain of dependencies, no fallback.
    assert_eq!(support(&t).expect("projects").len(), 1);

    // "What does this rest on that nobody proved?" — the discovery rule, and only it.
    let declared: Vec<String> = leaves_of(&t, Ground::Declared)
        .expect("projects")
        .into_iter()
        .map(|l| l.iri)
        .collect();
    assert_eq!(declared, vec![RULE.to_string()]);

    // "Which program outputs?" — both differential-dependency recomputes.
    let derived: Vec<String> = leaves_of(&t, Ground::Derived)
        .expect("projects")
        .into_iter()
        .map(|l| l.iri)
        .collect();
    assert_eq!(derived, vec![ACHILLES.to_string(), DRIVE.to_string()]);

    // Nothing on this conclusion is proved, so it is not fully verified — and saying so is the
    // point: the flagship claim's strongest ground is a recompute, not a proof.
    assert!(!is_fully_verified(&t).expect("projects"));

    assert_eq!(cited_iris(&t).expect("projects").len(), 3);
}

#[test]
fn every_ground_of_the_wrn_conclusion_is_load_bearing() {
    // The counterfactual, on real data. With no `Sum` in the term there is exactly one alternative,
    // so withdrawing ANY cited ground collapses it — including the declared rule.
    let t = wrn_conclusion();
    for iri in [RULE, ACHILLES, DRIVE] {
        assert!(
            !survives_without(&t, iri).expect("projects"),
            "{iri} is load-bearing: no alternative avoids it"
        );
    }
    // A ground it never cited costs nothing.
    assert!(survives_without(&t, "urn:eigenius:pub:wrn:not_cited").expect("projects"));
}

#[test]
fn a_second_source_would_make_a_recompute_droppable() {
    // The same conclusion authored with a fallback — `Sum(dd_achilles, dd_drive)` instead of
    // needing both — answers the counterfactual differently. This is the distinction a stored
    // scalar grade erases: both shapes grade the same, and only the polynomial tells them apart.
    let t = ctor(
        "App",
        vec![
            ctor(
                "SpecStr",
                vec![
                    leaf("DeclaredEvidence", RULE),
                    Exp::LitString("WRN".to_string()),
                ],
            ),
            ctor(
                "Sum",
                vec![
                    leaf("DerivedEvidence", ACHILLES),
                    leaf("DerivedEvidence", DRIVE),
                ],
            ),
        ],
    );
    assert_eq!(support(&t).expect("projects").len(), 2, "two alternatives");
    assert!(
        survives_without(&t, ACHILLES).expect("projects"),
        "the DRIVE branch carries it alone"
    );
    assert!(
        !survives_without(&t, RULE).expect("projects"),
        "the rule is still on every branch"
    );
}
