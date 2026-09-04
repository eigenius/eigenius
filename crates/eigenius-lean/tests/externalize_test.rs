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

//! D74 — the statement-level check, end to end against a real `lean4export` payload.
//!
//! `notebook_demo_fixture_lands_holds` cannot cover this: its claim
//! (`urn:eigenius:demo:lean:patient_1`) carries only `is_a` and no
//! `reflection:canonical_proposition`, so `claim_proposition` returns `None` and the check is
//! skipped. A green run there says nothing about this path — which is the failure mode this
//! whole line of work keeps finding, so it is stated rather than left to be rediscovered.
//!
//! The toy export declares `PUnit : Sort u`, `PUnit.unit : PUnit.{u}` and `PUnit.rec`. That is
//! enough to drive the fragment's `One` / `Sort` / `Param` arms against terms nanoda parsed
//! itself.

use std::sync::Arc;

use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::level::Level;
use eigenius_kernel::nbe::term::Exp;
use eigenius_lean::checker::{check_proof, ExpectedStatement, Verdict};

const TOY_HOLDS: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");

/// A layer is only consulted to resolve a chain IRI's `short_name`; these propositions name no
/// chain resources, so the bootstrap head serves.
fn head() -> Arc<Layer> {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    Arc::clone(ctx.head())
}

fn check(target: &str, prop: &Exp) -> Verdict {
    let layer = head();
    check_proof(
        TOY_HOLDS,
        target,
        &[],
        Some(&ExpectedStatement {
            proposition: prop,
            layer: &layer,
        }),
    )
    .expect("infrastructure ok")
}

/// The pre-D74 behaviour, kept explicit: with no expected statement the call establishes that a
/// declaration named `target` type-checks, and says nothing about what it proves.
#[test]
fn without_an_expected_statement_the_check_is_name_level_only() {
    let v = check_proof(TOY_HOLDS, "PUnit", &[], None).expect("infrastructure ok");
    assert!(matches!(v, Verdict::Holds), "got {v:?}");
}

/// `PUnit.unit`'s type is `PUnit.{u}`. The fragment's `One` maps to Lean's `PUnit`, so this is
/// the smallest end-to-end statement check there is: externalize, compare under `def_eq`.
#[test]
fn the_claims_proposition_matching_the_target_type_holds() {
    let v = check("PUnit.unit", &Exp::One);
    assert!(
        matches!(v, Verdict::Holds),
        "`One` must externalize to the `PUnit` that `PUnit.unit` inhabits; got {v:?}"
    );
}

/// The point of the whole exercise: a proof that type-checks, bound to a proposition that is not
/// what it proves, must not reach `Holds`. Before D74 this returned `Holds` — check 1 passes,
/// and nothing related the named theorem to the claim (#159).
#[test]
fn a_proposition_the_target_does_not_prove_fails() {
    // `PUnit.unit : PUnit`, not `PUnit : Sort u`.
    let v = check("PUnit.unit", &Exp::Sort(Level::Zero));
    match v {
        Verdict::Fails { diagnostic } => assert!(
            diagnostic.contains("not the claim's proposition"),
            "diagnostic should name the mismatch; got {diagnostic}"
        ),
        other => panic!("a mismatched statement must not Hold; got {other:?}"),
    }
}

/// Refusal is typed and total (D74 §4.2): a proposition outside the fragment fails with the
/// variant named, rather than being approximated into a different theorem.
#[test]
fn a_proposition_outside_the_fragment_is_refused_by_name() {
    // `Lam` and not `LitFloat`: float literals joined the fragment in §4.8.
    let v = check(
        "PUnit.unit",
        &Exp::Lam(
            eigenius_kernel::nbe::term::Patt::Var("x".into()),
            Box::new(Exp::Var("x".into())),
        ),
    );
    match v {
        Verdict::Fails { diagnostic } => {
            assert!(
                diagnostic.contains("Lam"),
                "the refusal must name the variant; got {diagnostic}"
            );
            assert!(
                diagnostic.contains("externalize"),
                "and say it could not be externalized; got {diagnostic}"
            );
        }
        other => panic!("an unrepresentable proposition must not Hold; got {other:?}"),
    }
}

/// D74 §6.5 — a `Level::Param` naming something the target does not declare is refused here
/// rather than left to `def_eq`, which would compare one parameter against a different one and
/// fail with nothing to say that universes were the cause.
#[test]
fn a_universe_param_the_target_does_not_declare_is_refused() {
    let v = check("PUnit.unit", &Exp::Sort(Level::Param("nonesuch".into())));
    match v {
        Verdict::Fails { diagnostic } => assert!(
            diagnostic.contains("nonesuch") && diagnostic.contains("universe parameter"),
            "the refusal must name the parameter; got {diagnostic}"
        ),
        other => panic!("an undeclared universe param must not Hold; got {other:?}"),
    }
}

// ─── D74 §4.2 — Σ as `Subtype`, against a real Lean declaration ────────────────────────────
//
// `test_resources/sigma_subtype.json` exports
//     def refined : { w : EigeniusFFI.eigenius.test.Widget // Big w } → PUnit
// whose type is `Subtype Big → PUnit` — the shape the DCG builds for a refined noun
// (`ontology.esl:65`: *"mutator load" → `Σx:Load. compound_kind(x, Mutator)`*), with the
// structure named as #208's mangling spells it.

const SIGMA_SUBTYPE: &[u8] = include_bytes!("../test_resources/sigma_subtype.json");

/// A layer declaring the class the fixture's structure mirrors, so `EigonClass` resolves to
/// `EigeniusFFI.eigenius.test.Widget` through D30's naming authority.
fn widget_layer() -> Arc<Layer> {
    use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::{Resource, Value};
    use eigenius_kernel::ontology::well_known as wk;

    let parent = head();
    let mut b = LayerBuilder::new("widget", Some(Arc::clone(&parent)));
    let mut r = Resource::new(Iri::parse("urn:eigenius:test:Widget").unwrap());
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    r.set(
        Iri::parse(wk::SHORT_NAME).unwrap(),
        Value::String("Widget".to_string()),
    );
    b.add_resource(r).expect("add Widget");

    // The refinement predicate, as an ESL `axiom` would appear.
    let mut a = Resource::new(Iri::parse("urn:eigenius:test:Big").unwrap());
    a.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    a.set(
        Iri::parse(wk::SHORT_NAME).unwrap(),
        Value::String("Big".to_string()),
    );
    b.add_resource(a).expect("add Big");
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// `Σ w : Widget. Big(w)` in domain position, against `Subtype Big → PUnit`.
///
/// This is the case that decides §4.2: the parser's Σ is a refinement over a class, and
/// `Subtype` is what it denotes.
#[test]
fn a_sigma_over_a_class_matches_a_lean_subtype() {
    let layer = widget_layer();
    let widget = Exp::EigonClass(
        eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:test:Widget").unwrap(),
    );
    let big = Exp::EigonAxiom(
        eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:test:Big").unwrap(),
    );
    // Σ w : Widget. Big(w)  →  PUnit
    let sigma = Exp::Sig(
        eigenius_kernel::nbe::term::Patt::Var("w".into()),
        Box::new(widget),
        Box::new(Exp::App(Box::new(big), Box::new(Exp::Var("w".into())))),
    );
    let prop = Exp::Arrow(Box::new(sigma), Box::new(Exp::One));

    // `Big` is an `axiom` in the fixture, as a chain relation is in ESL
    // (`axiom ontology:compound_kind : lexicon:Entity -> Set -> Prop`), so it must be permitted.
    let v = check_proof(
        SIGMA_SUBTYPE,
        "refined",
        &[
            // `Float`'s model depends on the standard three (`Float.add depends on axioms:
            // [propext, Classical.choice, Quot.sound]`), so a fixture using floats needs them —
            // the same set `DEFAULT_LEAN_AXIOMS` carries.
            "propext".to_string(),
            "Classical.choice".to_string(),
            "Quot.sound".to_string(),
            "EigeniusFFI.eigenius.test.Big".to_string(),
            "EigeniusFFI.eigenius.test.Measured".to_string(),
        ],
        Some(&ExpectedStatement {
            proposition: &prop,
            layer: &layer,
        }),
    )
    .expect("infrastructure ok");
    assert!(
        matches!(v, Verdict::Holds),
        "a Σ over a class must externalize to the `Subtype` the declaration uses; got {v:?}"
    );
}

/// `Σ w : Widget. Big(w) → Widget` where the codomain is reached by projection.
///
/// The fixture's `projected : { w : Widget // Big w } → Widget` has that type. Nothing here
/// exercises `Fst` in the STATEMENT — a projection appears in a term, not a type — so this pins
/// the type side, and `fst_recovers_its_implicits_by_inference` below pins the reconstruction.
#[test]
fn a_projection_declarations_type_still_matches() {
    let layer = widget_layer();
    let iri = |s: &str| eigenius_kernel::ontology::iri::Iri::parse(s).unwrap();
    let widget = Exp::EigonClass(iri("urn:eigenius:test:Widget"));
    let big = Exp::EigonAxiom(iri("urn:eigenius:test:Big"));
    let sigma = Exp::Sig(
        eigenius_kernel::nbe::term::Patt::Var("w".into()),
        Box::new(widget.clone()),
        Box::new(Exp::App(Box::new(big), Box::new(Exp::Var("w".into())))),
    );
    let prop = Exp::Arrow(Box::new(sigma), Box::new(widget));

    let v = check_proof(
        SIGMA_SUBTYPE,
        "projected",
        &[
            // `Float`'s model depends on the standard three (`Float.add depends on axioms:
            // [propext, Classical.choice, Quot.sound]`), so a fixture using floats needs them —
            // the same set `DEFAULT_LEAN_AXIOMS` carries.
            "propext".to_string(),
            "Classical.choice".to_string(),
            "Quot.sound".to_string(),
            "EigeniusFFI.eigenius.test.Big".to_string(),
            "EigeniusFFI.eigenius.test.Measured".to_string(),
        ],
        Some(&ExpectedStatement {
            proposition: &prop,
            layer: &layer,
        }),
    )
    .expect("infrastructure ok");
    assert!(matches!(v, Verdict::Holds), "got {v:?}");
}

/// `Fst` UNDER A BINDER now translates — the case the fork was made for.
///
/// The implicits of `Subtype.val : {α} → {p} → Subtype p → α` are recovered by inferring the
/// scrutinee's type. That needs two things nanoda would not give from outside: `infer` reachable
/// (via the public `is_proof`, which returns `(is_prop, infer(e))`) and a way to BUILD while a
/// checker is alive — `TypeChecker::new` asserts `dbj_level_counter == 0`, so no checker can be
/// created once a binder is open, and the `ctx` field was `pub(crate)`. `eigenius/nanoda_lib`
/// adds the accessor; externalization then builds locally nameless, as nanoda does internally,
/// so the scrutinee is closed and `infer` accepts it.
///
/// The proposition really contains a `Fst`: `∀ s : (Σ w : Widget. Big w), Big (Fst s)`, checked
/// against `theorem projects_in_the_type : ∀ s : Subtype Big, Big s.val`.
#[test]
fn fst_under_a_binder_recovers_its_implicits_by_inference() {
    let layer = widget_layer();
    let iri = |s: &str| eigenius_kernel::ontology::iri::Iri::parse(s).unwrap();
    let widget = Exp::EigonClass(iri("urn:eigenius:test:Widget"));
    let big = || Exp::EigonAxiom(iri("urn:eigenius:test:Big"));
    let sigma = Exp::Sig(
        eigenius_kernel::nbe::term::Patt::Var("w".into()),
        Box::new(widget),
        Box::new(Exp::App(Box::new(big()), Box::new(Exp::Var("w".into())))),
    );
    let prop = Exp::Pi(
        eigenius_kernel::nbe::term::Patt::Var("s".into()),
        Box::new(sigma),
        Box::new(Exp::App(
            Box::new(big()),
            Box::new(Exp::Fst(Box::new(Exp::Var("s".into())))),
        )),
    );
    let v = check_proof(
        SIGMA_SUBTYPE,
        "projects_in_the_type",
        &[
            // `Float`'s model depends on the standard three (`Float.add depends on axioms:
            // [propext, Classical.choice, Quot.sound]`), so a fixture using floats needs them —
            // the same set `DEFAULT_LEAN_AXIOMS` carries.
            "propext".to_string(),
            "Classical.choice".to_string(),
            "Quot.sound".to_string(),
            "EigeniusFFI.eigenius.test.Big".to_string(),
            "EigeniusFFI.eigenius.test.Measured".to_string(),
        ],
        Some(&ExpectedStatement {
            proposition: &prop,
            layer: &layer,
        }),
    )
    .expect("infrastructure ok");
    assert!(
        matches!(v, Verdict::Holds),
        "`Fst` under a binder must rebuild `Subtype.val` with inferred implicits; got {v:?}"
    );
}

// ─── D74 §4.8 — float literals and the Float type ──────────────────────────────────────────
//
// The motivating case for the whole fragment: a measurement claim asserts the value a
// computation produced. `0.1` is `@OfScientific.ofScientific Float instOfScientificFloat 1 true 1`
// — a typeclass application over NAT literals — so the externalizer builds it rather than
// emitting a node, and exactness rests on the shortest round-trip decimal.

fn measured_layer() -> Arc<Layer> {
    use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::{Resource, Value};
    use eigenius_kernel::ontology::well_known as wk;

    let parent = head();
    let mut b = LayerBuilder::new("measured", Some(Arc::clone(&parent)));
    let mut r = Resource::new(Iri::parse("urn:eigenius:test:Measured").unwrap());
    r.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    r.set(
        Iri::parse(wk::SHORT_NAME).unwrap(),
        Value::String("Measured".to_string()),
    );
    b.add_resource(r).expect("add Measured");
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn measured(arg: Exp) -> Exp {
    let iri = eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:test:Measured").unwrap();
    let p = || {
        Exp::App(
            Box::new(Exp::EigonAxiom(iri.clone())),
            Box::new(arg.clone()),
        )
    };
    Exp::Arrow(Box::new(p()), Box::new(p()))
}

fn check_measured(target: &str, prop: &Exp) -> Verdict {
    let layer = measured_layer();
    check_proof(
        SIGMA_SUBTYPE,
        target,
        &[
            // `Float`'s model depends on the standard three (`Float.add depends on axioms:
            // [propext, Classical.choice, Quot.sound]`), so a fixture using floats needs them —
            // the same set `DEFAULT_LEAN_AXIOMS` carries.
            "propext".to_string(),
            "Classical.choice".to_string(),
            "Quot.sound".to_string(),
            "EigeniusFFI.eigenius.test.Big".to_string(),
            "EigeniusFFI.eigenius.test.Measured".to_string(),
        ],
        Some(&ExpectedStatement {
            proposition: prop,
            layer: &layer,
        }),
    )
    .expect("infrastructure ok")
}

/// A positive float literal reproduces the exact `f64` Lean elaborated.
#[test]
fn a_float_literal_matches_what_lean_elaborated() {
    let v = check_measured("measured_refl", &measured(Exp::LitFloat(0.1)));
    assert!(matches!(v, Verdict::Holds), "got {v:?}");
}

/// Negative literals wrap in `Neg.neg`, which is what Lean itself emits for `-2.5`.
#[test]
fn a_negative_float_literal_matches() {
    let v = check_measured("measured_neg_refl", &measured(Exp::LitFloat(-2.5)));
    assert!(matches!(v, Verdict::Holds), "got {v:?}");
}

/// A DIFFERENT float must not match — the check is on the value, not on the shape.
#[test]
fn a_different_float_literal_fails() {
    let v = check_measured("measured_refl", &measured(Exp::LitFloat(0.2)));
    match v {
        Verdict::Fails { diagnostic } => assert!(
            diagnostic.contains("not the claim's proposition"),
            "got {diagnostic}"
        ),
        other => panic!("0.2 must not match a proof about 0.1; got {other:?}"),
    }
}

/// The Float TYPE needs no encoding — it is an ordinary `Const`, like `String` and `Int`. This is
/// the shape most measurement claims take: the quantity is bound, not written out.
#[test]
fn a_proposition_quantifying_over_float_matches() {
    let prop = Exp::Pi(
        eigenius_kernel::nbe::term::Patt::Var("x".into()),
        Box::new(Exp::EigonPrimitive(
            eigenius_kernel::nbe::term::PrimitiveType::Float,
        )),
        Box::new(measured(Exp::Var("x".into()))),
    );
    let v = check_measured("quantifies_over_float", &prop);
    assert!(matches!(v, Verdict::Holds), "got {v:?}");
}

/// NaN and ±∞ have no decimal form, so they are refused rather than approximated.
#[test]
fn a_non_finite_float_is_refused() {
    let v = check_measured("measured_refl", &measured(Exp::LitFloat(f64::NAN)));
    match v {
        Verdict::Fails { diagnostic } => assert!(
            diagnostic.contains("not finite"),
            "the refusal must say why; got {diagnostic}"
        ),
        other => panic!("NaN must be refused; got {other:?}"),
    }
}
