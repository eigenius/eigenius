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

//! D90 — what an institution sends back is checked against what it declared.
//!
//! `which_ranges_admit_a_proposition.rs` measures the LAYER boundary: which declared
//! ranges let a D85-encoded term through commit-time validation. This measures the
//! INSTITUTION boundary, which is a different place and was unchecked until D90: the
//! output an institution hands the kernel, before the Verdict is built and committed.
//!
//! Each case runs the same stub institution through `dispatch_auto_on_load_for_resource`
//! and differs only in what the institution returns and what its QueryClass declared.
//! The negative controls are the point — a test that only shows the good case passing
//! is not evidence the check runs at all.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::dispatch::dispatch_auto_on_load_for_resource;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::{Institution, InstitutionRuntime, QueryOutcome};
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

const INST: &str = "urn:eigenius:test:contract:inst";
const QC: &str = "urn:eigenius:test:contract:check";
const SUBJECT_CLASS: &str = "urn:eigenius:test:contract:Subject";
const SUBJECT: &str = "urn:eigenius:test:contract:s1";
/// A slot declared `core:resource` with no `class_types` — the one shape
/// `which_ranges_admit_a_proposition` found still admits a term at the layer boundary.
const SMUGGLING_SLOT: &str = "urn:eigenius:test:contract:opaque_finding";
/// The same value on a slot declared in the proposition form.
const DECLARED_SLOT: &str = "urn:eigenius:test:contract:declared_finding";
/// `class_types [eigentt:Term]` + `eigentt:is_a_type` — Rule 21 case 2, a full
/// `check_type`. Strictly stronger than inference, and so an admitted form.
const IS_A_TYPE_SLOT: &str = "urn:eigenius:test:contract:type_finding";
/// `class_types [eigentt:Term]` with NEITHER — Rule 21 case 3, where inference alone
/// runs and the slot states no contract.
const INFERRED_SLOT: &str = "urn:eigenius:test:contract:inferred_finding";
/// An ordinary declared wrapper, ranged on a class. What hides a term one level down.
const WRAPPER_SLOT: &str = "urn:eigenius:test:contract:wrapper";
/// The unranged slot INSIDE that wrapper — declared by nothing the QueryClass lists.
const INNER_SLOT: &str = "urn:eigenius:test:contract:wrapper_inner";

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// The institution under test: returns whatever verdict and properties the case asks
/// for. Stateless, so one struct covers every case.
struct Stub {
    iri: Iri,
    ctor: &'static str,
    /// Properties to set on the gate output.
    output_props: Vec<(String, Value)>,
}

impl Institution for Stub {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }
    fn extract_typed(
        &self,
        _: &Iri,
        _: &Resource,
        _: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        unreachable!("the contract check runs on `query` output")
    }
    fn reify(&self, _: &Iri, _: &Val, _: &ExecutionContext) -> Result<Resource, InstitutionError> {
        unreachable!("the contract check runs on `query` output")
    }
    fn query(
        &self,
        _: &Iri,
        _: &Resource,
        _: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        let mut r = Resource::new_embedded();
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
        );
        r.set(iri(wk::CTOR_NAME), Value::String(self.ctor.to_string()));
        for (p, v) in &self.output_props {
            r.set(iri(p), v.clone());
        }
        Ok(QueryOutcome::from_output(r))
    }
}

/// Which declared form a probe property takes.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    /// `core:resource`, no `class_types` — the shape that admits a term at the layer
    /// boundary.
    Unranged,
    /// `class_types [eigentt:Term]` + `eigentt:expected_type` (Rule 21 case 1).
    TermWithExpectedType,
    /// `class_types [eigentt:Term]` + `eigentt:is_a_type` (Rule 21 case 2).
    TermIsAType,
    /// `class_types [eigentt:Term]` with neither (Rule 21 case 3 — inference only).
    TermInferred,
}

/// A property declaration in one of the four forms.
fn declare_prop_in(id: &str, form: Form) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
    );
    r.set(
        iri(wk::DESCRIPTION),
        Value::String("contract probe property".into()),
    );
    r.set(iri(wk::SHORT_NAME), Value::String("probe".into()));
    if form == Form::Unranged {
        r.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:resource".into()),
        );
        return r;
    }
    r.set(
        iri(wk::DATA_TYPE_PROP),
        Value::String("urn:eigenius:core:inductive".into()),
    );
    r.set(
        iri(wk::CLASS_TYPES),
        Value::Array(vec![Value::String("urn:eigenius:eigentt:Term".into())]),
    );
    match form {
        Form::TermWithExpectedType => r.set(
            iri(wk::EXPECTED_TYPE),
            encode(&Exp::sort(1)).expect("Sort(1) encodes"),
        ),
        Form::TermIsAType => r.set(iri("urn:eigenius:eigentt:is_a_type"), Value::Boolean(true)),
        // Rule 21 case 3: neither, so inference alone runs.
        Form::TermInferred | Form::Unranged => {}
    }
    r
}

/// A property declaration, optionally in the proposition form
/// (`class_types [eigentt:Term]` + `eigentt:expected_type`).
fn declare_prop(id: &str, as_term: bool) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
    );
    r.set(
        iri(wk::DESCRIPTION),
        Value::String("contract probe property".into()),
    );
    r.set(iri(wk::SHORT_NAME), Value::String("probe".into()));
    if as_term {
        r.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:inductive".into()),
        );
        r.set(
            iri(wk::CLASS_TYPES),
            Value::Array(vec![Value::String("urn:eigenius:eigentt:Term".into())]),
        );
        r.set(
            iri(wk::EXPECTED_TYPE),
            encode(&Exp::sort(1)).expect("Sort(1) encodes"),
        );
    } else {
        r.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:resource".into()),
        );
    }
    r
}

fn encode(exp: &Exp) -> Option<Value> {
    eigenius_kernel::program::eigentt_type_mirror::encode_type(
        exp,
        eigenius_kernel::testing::codec_names(),
    )
    .ok()
}

/// Run one dispatch and return the contract errors it produced.
fn run_case(
    ctor: &'static str,
    output_props: Vec<(String, Value)>,
    result_properties: Vec<&str>,
    permitted_verdicts: Vec<&str>,
) -> Vec<String> {
    run_case_returning(
        ctor,
        output_props,
        result_properties,
        permitted_verdicts,
        wk::VERDICT,
    )
}

fn run_case_returning(
    ctor: &'static str,
    output_props: Vec<(String, Value)>,
    result_properties: Vec<&str>,
    permitted_verdicts: Vec<&str>,
    result_class: &str,
) -> Vec<String> {
    let boot = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let storage = boot.storage().clone();
    let mut b = LayerBuilder::new("contract-probe", Some(Arc::clone(boot.head())));

    b.add_resource(declare_prop_in(SMUGGLING_SLOT, Form::Unranged))
        .unwrap();
    b.add_resource(declare_prop_in(DECLARED_SLOT, Form::TermWithExpectedType))
        .unwrap();
    b.add_resource(declare_prop_in(IS_A_TYPE_SLOT, Form::TermIsAType))
        .unwrap();
    b.add_resource(declare_prop_in(INFERRED_SLOT, Form::TermInferred))
        .unwrap();
    b.add_resource(declare_prop_in(INNER_SLOT, Form::Unranged))
        .unwrap();
    // The wrapper is an ordinary declared slot ranged on a class — exactly what the
    // closed check admits, and what used to hide a term one level down.
    let mut wrapper_class = Resource::new(iri("urn:eigenius:test:contract:Wrapper"));
    wrapper_class.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    wrapper_class.set(
        iri(wk::DESCRIPTION),
        Value::String("probe wrapper class".into()),
    );
    wrapper_class.set(iri(wk::SHORT_NAME), Value::String("Wrapper".into()));
    b.add_resource(wrapper_class).unwrap();
    let mut wrapper = declare_prop_in(WRAPPER_SLOT, Form::Unranged);
    wrapper.set(
        iri(wk::CLASS_TYPES),
        Value::Array(vec![Value::String(
            "urn:eigenius:test:contract:Wrapper".into(),
        )]),
    );
    b.add_resource(wrapper).unwrap();

    // The institution and the gated class the QueryClass names. Both were missing while
    // the fixture never ran a Validator over itself, and the dispatch does not resolve
    // either — it keys on the class IRI and looks the institution up in the runtime — so
    // the tests passed against a chain that could not commit.
    let mut subject_class = Resource::new(iri(SUBJECT_CLASS));
    subject_class.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    subject_class.set(
        iri(wk::DESCRIPTION),
        Value::String("the class the probe QueryClass gates on".into()),
    );
    subject_class.set(iri(wk::SHORT_NAME), Value::String("Subject".into()));
    b.add_resource(subject_class).unwrap();

    let mut institution = Resource::new(iri(INST));
    institution.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            "urn:eigenius:institution:Institution".to_string(),
        )]),
    );
    institution.set(
        iri("urn:eigenius:institution:institution_iri"),
        Value::String(INST.to_string()),
    );
    institution.set(
        iri("urn:eigenius:institution:institution_name"),
        Value::String("ContractProbe".into()),
    );
    b.add_resource(institution).unwrap();

    let mut qc = Resource::new(iri(QC));
    qc.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
    );
    qc.set(iri(wk::QUERY_CLASS), Value::String(SUBJECT_CLASS.into()));
    qc.set(iri(wk::RESULT_CLASS), Value::String(result_class.into()));
    qc.set(
        iri(wk::DISPATCH_ROLE),
        Value::Array(vec![Value::String(wk::DISPATCH_AUTO_ON_LOAD.into())]),
    );
    qc.set(
        iri(wk::QUERY_HANDLER),
        Value::String("urn:eigenius:test:contract:proc".into()),
    );
    qc.set(
        iri("urn:eigenius:institution:institution_ref"),
        Value::String(INST.into()),
    );
    if !result_properties.is_empty() {
        qc.set(
            iri("urn:eigenius:institution:result_properties"),
            Value::Array(
                result_properties
                    .iter()
                    .map(|p| Value::String((*p).to_string()))
                    .collect(),
            ),
        );
    }
    if !permitted_verdicts.is_empty() {
        qc.set(
            iri("urn:eigenius:institution:permitted_verdicts"),
            Value::Array(
                permitted_verdicts
                    .iter()
                    .map(|p| Value::String((*p).to_string()))
                    .collect(),
            ),
        );
    }
    b.add_resource(qc).unwrap();

    let layer = Arc::new(b.build(storage.clone()));

    // The fixture must itself be committable, or a refusal proves nothing about the
    // boundary — it could be proving the declarations are malformed.
    let declaration_errors =
        eigenius_kernel::validation::Validator::new(Arc::clone(&layer)).validate();
    assert!(
        declaration_errors.is_empty(),
        "the probe declarations do not validate:\n{}",
        declaration_errors
            .iter()
            .map(|e| format!("  [{:?}] {}", e.rule, e.message))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let (index, errors) = InstitutionIndex::from_layer(&layer);
    assert!(errors.is_empty(), "index errors: {errors:?}");

    let mut runtime = InstitutionRuntime::new();
    runtime
        .register(Box::new(Stub {
            iri: iri(INST),
            ctor,
            output_props,
        }))
        .unwrap();

    let ctx = ExecutionContext::new(layer, "contract-probe", ExecutionMode::ReadOnly, storage);
    let mut subject = Resource::new(iri(SUBJECT));
    subject.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(SUBJECT_CLASS.into())]),
    );

    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    outcome
        .errors
        .into_iter()
        .map(|e| e.message)
        .filter(|m| m.contains("result contract"))
        .collect()
}

/// A D85-encoded proposition: an embedded resource whose `is_a` names its constructor
/// class. Structurally indistinguishable from any other embedded value, which is why the
/// carrying property's declaration is the whole of what decides whether it is checked.
fn a_proposition() -> Value {
    encode(&Exp::sort(0)).expect("Sort(0) encodes")
}

#[test]
fn a_term_on_an_undeclared_slot_is_refused() {
    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(SMUGGLING_SLOT.to_string(), a_proposition())],
        vec![SMUGGLING_SLOT],
        vec![],
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("declares neither"),
        "expected the term-slot violation, got: {}",
        errs[0]
    );
}

#[test]
fn the_same_term_on_a_slot_declared_as_a_proposition_is_admitted() {
    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(DECLARED_SLOT.to_string(), a_proposition())],
        vec![DECLARED_SLOT],
        vec![],
    );
    assert!(errs.is_empty(), "expected no violation, got: {errs:?}");
}

#[test]
fn a_property_the_query_class_did_not_declare_is_refused() {
    // Same well-declared slot as the admitted case — what changes is that the
    // QueryClass no longer lists it, so it is outside the contract entirely.
    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(DECLARED_SLOT.to_string(), a_proposition())],
        vec![],
        vec![],
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("does not declare"),
        "expected the undeclared-property violation, got: {}",
        errs[0]
    );
}

#[test]
fn a_verdict_outside_the_permitted_set_is_refused() {
    let errs = run_case(
        wk::VERDICT_FAILS,
        vec![],
        vec![],
        vec![
            "urn:eigenius:institution:Verdict-Holds",
            "urn:eigenius:institution:Verdict-Undecidable",
        ],
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("outside the declared permitted set"),
        "expected the permitted-verdict violation, got: {}",
        errs[0]
    );
}

#[test]
fn the_same_verdict_is_admitted_where_no_permitted_set_is_declared() {
    // The pre-D90 behaviour is preserved exactly: a QueryClass that declares no
    // restriction returns any of the three.
    let errs = run_case(wk::VERDICT_FAILS, vec![], vec![], vec![]);
    assert!(errs.is_empty(), "expected no violation, got: {errs:?}");
}

#[test]
fn a_term_on_an_undeclared_slot_of_a_derivation_is_refused() {
    // The gate output is not the only way out. A derivation carries the same risk, and
    // the term-slot check covers it for that reason.
    let boot = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let storage = boot.storage().clone();
    let mut b = LayerBuilder::new("contract-probe", Some(Arc::clone(boot.head())));
    b.add_resource(declare_prop(SMUGGLING_SLOT, false)).unwrap();

    let mut qc = Resource::new(iri(QC));
    qc.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
    );
    qc.set(iri(wk::QUERY_CLASS), Value::String(SUBJECT_CLASS.into()));
    qc.set(iri(wk::RESULT_CLASS), Value::String(wk::VERDICT.into()));
    qc.set(
        iri(wk::DISPATCH_ROLE),
        Value::Array(vec![Value::String(wk::DISPATCH_AUTO_ON_LOAD.into())]),
    );
    qc.set(
        iri(wk::QUERY_HANDLER),
        Value::String("urn:eigenius:test:contract:proc".into()),
    );
    qc.set(
        iri("urn:eigenius:institution:institution_ref"),
        Value::String(INST.into()),
    );
    b.add_resource(qc).unwrap();

    struct DerivationStub {
        iri: Iri,
    }
    impl Institution for DerivationStub {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!()
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!()
        }
        fn query(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<QueryOutcome, InstitutionError> {
            let mut gate = Resource::new_embedded();
            gate.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
            );
            gate.set(
                iri(wk::CTOR_NAME),
                Value::String(wk::VERDICT_HOLDS.to_string()),
            );
            let mut derivation = Resource::new(iri("urn:eigenius:test:contract:s1:result"));
            derivation.set(iri(SMUGGLING_SLOT), a_proposition());
            Ok(QueryOutcome {
                output: gate,
                derivations: vec![derivation],
                partial_invocation: None,
            })
        }
    }

    let layer = Arc::new(b.build(storage.clone()));
    let (index, errors) = InstitutionIndex::from_layer(&layer);
    assert!(errors.is_empty(), "index errors: {errors:?}");
    let mut runtime = InstitutionRuntime::new();
    runtime
        .register(Box::new(DerivationStub { iri: iri(INST) }))
        .unwrap();
    let ctx = ExecutionContext::new(layer, "contract-probe", ExecutionMode::ReadOnly, storage);
    let mut subject = Resource::new(iri(SUBJECT));
    subject.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(SUBJECT_CLASS.into())]),
    );

    let outcome = dispatch_auto_on_load_for_resource(&subject, &index, &runtime, &ctx);
    let msgs: Vec<String> = outcome
        .errors
        .into_iter()
        .map(|e| e.message)
        .filter(|m| m.contains("result contract"))
        .collect();
    assert_eq!(msgs.len(), 1, "expected exactly one violation: {msgs:?}");
    assert!(
        msgs[0].contains("declares neither"),
        "expected the term-slot violation, got: {}",
        msgs[0]
    );
}

/// **The door a review found open.** `institution:from_subject` is declared
/// `core:resource` with no `class_types` — the one shape that admits a term at the layer
/// boundary — and the exemption list treated it as kernel-stamped ON THE GATE OUTPUT,
/// where the kernel does not stamp it: `protected_verdict_properties` does not contain
/// it, and the merge copies it through verbatim. So a term rode out on it with zero
/// violations and zero validation errors, which is eigenius#226 through a second door.
///
/// The exemption now delegates to the merge's own drop set, so it cannot drift wider
/// than what the kernel actually stamps.
#[test]
fn a_term_on_a_property_the_kernel_does_not_actually_stamp_is_refused() {
    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(
            "urn:eigenius:institution:from_subject".to_string(),
            a_proposition(),
        )],
        vec![],
        vec![],
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("does not declare"),
        "expected the undeclared-property violation, got: {}",
        errs[0]
    );
}

/// **The second door.** A term one level down, inside a declared wrapper that the
/// QueryClass DOES list and whose range is satisfied. The closed check sees only the
/// outer key; the term check used to stop at the first embedded value. Both now descend.
#[test]
fn a_term_nested_inside_a_declared_wrapper_is_refused() {
    let mut inner = Resource::new_embedded();
    inner.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            "urn:eigenius:test:contract:Wrapper".into(),
        )]),
    );
    inner.set(iri(INNER_SLOT), a_proposition());

    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(WRAPPER_SLOT.to_string(), Value::Embedded(Box::new(inner)))],
        vec![WRAPPER_SLOT],
        vec![],
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("declares neither") && errs[0].contains("wrapper_inner"),
        "expected the term-slot violation naming the INNER slot, got: {}",
        errs[0]
    );
}

/// The walk stops at a term rather than descending into it. A term's constructor
/// arguments are nested embedded resources under generated property IRIs; treating them
/// as slots would refuse every legitimate term, which is why Rule 21 exempts them too.
#[test]
fn the_walk_does_not_descend_into_a_terms_own_arguments() {
    let errs = run_case(
        wk::VERDICT_HOLDS,
        // Sort(1) is not a leaf: it carries a `Term-Sort-level` argument, itself an
        // embedded resource. If the walk descended, that argument would be checked as a
        // slot and refused.
        vec![(DECLARED_SLOT.to_string(), encode(&Exp::sort(1)).unwrap())],
        vec![DECLARED_SLOT],
        vec![],
    );
    assert!(errs.is_empty(), "expected no violation, got: {errs:?}");
}

/// Rule 21 case 2 — `class_types [eigentt:Term]` with `eigentt:is_a_type` — is a full
/// `check_type`, strictly stronger than inference. Refusing it would have refused
/// `eigentt:axiom_statement`, `eigentt:definition_type`, `core:ctor_type` and
/// `lexicon:sem_type`, every slot whose inhabited sorts vary within the slot.
#[test]
fn a_term_slot_declared_is_a_type_is_admitted() {
    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(IS_A_TYPE_SLOT.to_string(), a_proposition())],
        vec![IS_A_TYPE_SLOT],
        vec![],
    );
    assert!(errs.is_empty(), "expected no violation, got: {errs:?}");
}

/// Its control: Rule 21 case 3, where the slot declares the range and NEITHER form, so
/// inference alone runs. An institution's epistemic output should not rest on inference
/// agreeing with intent, so this one is refused.
#[test]
fn a_term_slot_that_only_gets_inferred_is_refused() {
    let errs = run_case(
        wk::VERDICT_HOLDS,
        vec![(INFERRED_SLOT.to_string(), a_proposition())],
        vec![INFERRED_SLOT],
        vec![],
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("inferred rather than checked"),
        "expected the no-checked-form violation, got: {}",
        errs[0]
    );
}

/// D14 §4.4 has always said an AutoOnLoad QueryClass returns a Verdict. Nothing checked,
/// while `build_verdict_resource` stamps `is_a: [Verdict]` regardless — so the closed
/// check would have run against a class the committed resource is not an instance of,
/// and an institution could widen its own contract by naming a wider one.
#[test]
fn an_auto_on_load_query_class_must_return_a_verdict() {
    let errs = run_case_returning(
        wk::VERDICT_HOLDS,
        vec![],
        vec![],
        vec![],
        "urn:eigenius:prov:ProgramTrace",
    );
    assert_eq!(errs.len(), 1, "expected exactly one violation: {errs:?}");
    assert!(
        errs[0].contains("returns a Verdict"),
        "expected the result-class violation, got: {}",
        errs[0]
    );
}

/// A `result_class` that does not resolve is fail-closed: the check refuses rather than
/// admitting unchecked.
///
/// Called directly rather than through a dispatch, because the state is UNREACHABLE that
/// way and a fixture built to reach it does not validate — Rule 22 §b refuses a
/// resource-typed property whose value does not resolve, so such a QueryClass cannot
/// commit. (Not Rule 14: that one returns early unless the resource is a `core:Class` or
/// a `core:Property`, and a QueryClass instance is neither.) This pins the branch as
/// defence in depth and says why it is not reachable.
#[test]
fn an_unresolvable_result_class_refuses_the_dispatch() {
    use eigenius_kernel::institution::result_contract::check_output;

    let boot = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let head = Arc::clone(boot.head());

    let mut output = Resource::new_embedded();
    output.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
    );
    output.set(
        iri(wk::CTOR_NAME),
        Value::String(wk::VERDICT_HOLDS.to_string()),
    );

    let violations = check_output(
        &output,
        &[],
        &iri("urn:eigenius:test:contract:NoSuchClass"),
        &[],
        &[],
        wk::VERDICT_HOLDS,
        &head,
    );
    assert_eq!(
        violations.len(),
        1,
        "expected one violation: {violations:?}"
    );
    assert!(
        violations[0].to_string().contains("does not resolve"),
        "expected the unresolved-result-class violation, got: {}",
        violations[0]
    );
}
