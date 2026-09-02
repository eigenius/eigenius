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

//! Rule 16 (an inductive slot names one InductiveType, D32 §3.5) and Rule 17
//! (FormulaTerm App-spine arity check, D32 §5.4 / Phase 19d.0.d).
//!
//! Rule 16 checks the DECLARATION only. An inductive value is a resource (D85 §6.1), so the
//! ordinary rules check the value itself — see [`Validator::check_inductive_value`] for which
//! rule owns which part. Rule 17 is the one thing they do not cover: whether an `App` spine
//! supplies the number of arguments its head operator declares.

use super::super::{iri, ValidationError, ValidationRule, Validator};
use super::eigentt_value::is_constructor_argument;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::program::eigentt_type_mirror::ctor_and_args;

/// FormulaTerm InductiveType IRI (D32 §4). Pinned here so the
/// formula-specific arity rule can short-circuit before doing any
/// chain resolution work.
const FORMULA_TERM_IRI: &str = "urn:eigenius:formulas:FormulaTerm";

/// Operator.operator_arity property IRI. Convenience integer; the
/// rank check uses it as a fast-path before the full
/// operator_signature walk (deferred to a follow-on landing).
const OPERATOR_ARITY_IRI: &str = "urn:eigenius:formulas:operator_arity";

/// `formulas:Operator` class IRI. An `OpRef` head must resolve to an
/// instance of this class (or of a subclass of it) before its
/// `operator_arity` is read.
const OPERATOR_IRI: &str = "urn:eigenius:formulas:Operator";

/// Walk the left spine of an `App(App(App(head, a₃), a₂), a₁)` value and return
/// `(head, [a₁, a₂, a₃])`. Spine args are emitted **right-to-left** as the spine
/// is traversed; the caller may want to reverse if argument order matters
/// semantically. For the arity check, only the count matters so the order is
/// irrelevant.
///
/// A node whose constructor cannot be read stops the walk and becomes the head:
/// Rule 23 reports the malformed value itself, so this rule stays quiet about it.
fn collect_app_spine<'a>(node: &'a Resource, layer: &Layer) -> (&'a Resource, Vec<&'a Value>) {
    let mut spine = Vec::new();
    let mut cursor = node;
    loop {
        let Ok((ctor, args)) = ctor_and_args(cursor, layer) else {
            return (cursor, spine);
        };
        if ctor != "App" || args.len() != 2 {
            return (cursor, spine);
        }
        let Value::Embedded(head) = args[0] else {
            return (cursor, spine);
        };
        spine.push(args[1]);
        cursor = head;
    }
}

impl Validator {
    /// Rule 16: an inductive slot names one InductiveType (D32 §3.5, D85 §5).
    ///
    /// When a property has `data_type: core:inductive`, its `class_types` must
    /// declare exactly one entry and that entry must resolve to a
    /// `core:InductiveType`. That is the whole of this rule.
    ///
    /// The value itself is not walked here. An inductive value is a resource
    /// whose `is_a` names the constructor's class (D85 §6.1), so the ordinary
    /// rules already own every part of it: Rule 5 gates the wire shape, Rule 6
    /// checks the constructor class against the slot's `class_types` (the
    /// derived class lists the inductive in `parent_classes`), Rule 23 recurses
    /// into the embedded resource, and there Rule 1 checks arity via `requires`
    /// and Rules 5 and 6 check each argument against its declared property. A
    /// second walk here would restate all of it against a shape nothing writes.
    pub(in crate::validation) fn check_inductive_value(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let dt = match self.get_data_type_str(prop_def) {
            Some(dt) => dt,
            None => return vec![],
        };
        if dt != wk::INDUCTIVE {
            return vec![];
        }

        let allowed = match prop_def.get(&iri(wk::CLASS_TYPES)) {
            Some(val) => val.as_iri_array(),
            None => Vec::new(),
        };
        if allowed.len() != 1 {
            return vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeMismatch,
                message: format!(
                    "data_type 'core:inductive' requires exactly one `class_types` entry naming an InductiveType (got {})",
                    allowed.len()
                ),
            }];
        }
        let ind_iri = allowed.into_iter().next().expect("len 1");

        let ind_type = match self.layer.resolve(&ind_iri) {
            Some(r) => r,
            None => {
                return vec![ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::UnresolvedClassReference,
                    message: format!(
                        "inductive type '{ind_iri}' on `class_types` not found in chain"
                    ),
                }];
            }
        };
        if !ind_type.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) {
            return vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeMismatch,
                message: format!(
                    "`class_types` IRI '{ind_iri}' is not an InductiveType — `data_type: core:inductive` requires one"
                ),
            }];
        }

        let _ = (value, ind_type);
        vec![]
    }

    /// Rule 17: FormulaTerm App-spine arity check (D32 §5.4).
    ///
    /// When a property's value is a FormulaTerm whose outer ctor is
    /// `App`, walk the left spine to find the head. If the head is an
    /// `OpRef(iri)`, resolve `iri` to a `formulas:Operator` and confirm
    /// the App spine supplies exactly its declared `operator_arity`
    /// arguments. This catches typos like `App(OpRef("add"), x)` (one
    /// arg short) at commit time rather than at dispatch. Every way the
    /// resolution itself can fail is a diagnostic too — see
    /// [`Self::check_op_ref_head`], which owns the stage-by-stage
    /// contract.
    ///
    /// Type-of-each-arg checking against the operator's full
    /// `operator_signature` (a Pi chain over FormulaTerm) is a
    /// follow-on landing — v1 ships arity-only.
    pub(in crate::validation) fn check_formula_term_arity(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
        owner: &Resource,
    ) -> Vec<ValidationError> {
        // A subterm is checked as part of the term that contains it. `add(x, 2)` is
        // `App(App(OpRef(add), x), 2)`, and its inner `App` is a partial application
        // sitting in the outer one's `head` slot — arity-checking it standalone reports
        // `add` under-applied. Rule 23 recurses into every embedded resource, so without
        // this the outer walk's care to skip intermediate spine heads buys nothing.
        if is_constructor_argument(prop_iri, owner) {
            return vec![];
        }

        // Only fire on properties whose value is a FormulaTerm.
        let dt = match self.get_data_type_str(prop_def) {
            Some(dt) => dt,
            None => return vec![],
        };
        if dt != wk::INDUCTIVE {
            return vec![];
        }
        let allowed = match prop_def.get(&iri(wk::CLASS_TYPES)) {
            Some(val) => val.as_iri_array(),
            None => return vec![],
        };
        if allowed.len() != 1 {
            return vec![];
        }
        if allowed[0].as_str() != FORMULA_TERM_IRI {
            return vec![];
        }

        let Value::Embedded(term) = value else {
            return vec![];
        };

        let mut errors = Vec::new();
        self.walk_formula_term_app_arity(term, prop_iri.as_str().to_string(), res_id, &mut errors);
        errors
    }

    /// Walk a FormulaTerm value tree. When entering an `App` node,
    /// resolve the *whole* left spine to its head + spine args and
    /// arity-check once. Then recurse only into the spine args
    /// (each of which may itself be a sub-tree carrying nested
    /// applications). The intermediate `App` nodes inside the spine
    /// are *not* re-checked — they're partial applications, not
    /// complete operator invocations.
    ///
    /// For non-App nodes, recurse into every arg so nested
    /// applications buried in `Lam(_, ty, body)` etc. still get
    /// checked.
    fn walk_formula_term_app_arity(
        &self,
        node: &Resource,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        let Ok((ctor, args)) = ctor_and_args(node, &self.layer) else {
            return;
        };

        if ctor == "App" {
            let (head, spine_args) = collect_app_spine(node, &self.layer);
            if let Ok(("OpRef", head_args)) = ctor_and_args(head, &self.layer)
                .as_ref()
                .map(|(c, a)| (c.as_str(), a))
            {
                self.check_op_ref_head(head_args, spine_args.len(), &path, res_id, out);
            }
            // Recurse only into spine args (NOT into intermediate
            // App heads — those are partial applications counted by
            // the spine, not separate invocations to arity-check).
            // `collect_app_spine` returns args right-to-left from
            // the deepest App; reverse so paths read left-to-right.
            for (i, arg) in spine_args.iter().rev().enumerate() {
                self.walk_arg_for_app_arity(arg, format!("{path}.args[{i}]"), res_id, out);
            }
            return;
        }

        // Non-App node: recurse into every arg so nested applications
        // inside Lam/Pi bodies, OpRef IRIs (no recursion needed —
        // they're string args), etc. get checked too.
        for (i, arg) in args.iter().enumerate() {
            self.walk_arg_for_app_arity(arg, format!("{path}.args[{i}]"), res_id, out);
        }
    }

    /// Recurse into one constructor argument. A subterm is an embedded value; an
    /// argument list (`core:value_array`) holds subterms element-wise. Everything
    /// else is a leaf this rule has nothing to say about.
    fn walk_arg_for_app_arity(
        &self,
        arg: &Value,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        match arg {
            Value::Embedded(r) => self.walk_formula_term_app_arity(r, path, res_id, out),
            Value::Array(elems) => {
                for (i, e) in elems.iter().enumerate() {
                    self.walk_arg_for_app_arity(e, format!("{path}[{i}]"), res_id, out);
                }
            }
            _ => {}
        }
    }

    /// Rank-check one `App` spine against the operator its `OpRef`
    /// head names, diagnosing every way that resolution can fail.
    ///
    /// Stages, in order: the operand must parse as an IRI, resolve in
    /// the layer chain, and be a `formulas:Operator`
    /// ([`ValidationRule::UnknownOperator`] otherwise); its
    /// `operator_arity` must be a non-negative integer
    /// ([`ValidationRule::OperatorDeclarationMalformed`] otherwise);
    /// and that arity must equal the spine length
    /// ([`ValidationRule::OperatorArityMismatch`] otherwise).
    ///
    /// Two shapes deliberately produce nothing here:
    ///
    /// - **A missing or non-string `OpRef` operand.** The value is a resource
    ///   whose class `requires` the operand property at its declared
    ///   `core:string` (D85 §6.1), so Rule 1 reports the missing argument and
    ///   Rule 5 the wrong type. Re-reporting it would double-diagnose one defect.
    /// - **An operator carrying no `operator_arity` at all.** The
    ///   property is only `recommends` on `formulas:Operator`, which
    ///   `requires` `operator_signature` instead, so an operator
    ///   declaring just the signature is schema-conformant and this
    ///   rule has nothing to read. Which of the two is authoritative —
    ///   and hence whether the absence is an error or a cue to walk
    ///   the signature's Pi binders — is eigenius#163, a maintainer
    ///   decision this rule does not pre-empt.
    fn check_op_ref_head(
        &self,
        head_args: &[&Value],
        spine_len: usize,
        path: &str,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        let unknown = |message: String| ValidationError {
            resource_id: res_id.clone(),
            property: None,
            rule: ValidationRule::UnknownOperator,
            message,
        };

        // Arg shape is Rules 1 and 5's; see the doc comment.
        let Some(Value::String(op_iri_s)) = head_args.first().copied() else {
            return;
        };

        let op_iri = match Iri::parse(op_iri_s) {
            Ok(i) => i,
            Err(e) => {
                out.push(unknown(format!(
                    "{path}: `OpRef` operand `{op_iri_s}` is not a well-formed IRI ({e})"
                )));
                return;
            }
        };

        let Some(op_resource) = self.layer.resolve(&op_iri) else {
            out.push(unknown(format!(
                "{path}: operator `{op_iri_s}` does not resolve in the layer chain"
            )));
            return;
        };

        if !self.is_instance_of_any(&op_resource, &[&iri(OPERATOR_IRI)]) {
            out.push(unknown(format!(
                "{path}: `{op_iri_s}` resolves to a resource that is not a `formulas:Operator`"
            )));
            return;
        }

        // Absent `operator_arity` is schema-conformant; see the doc comment.
        let Some(arity_value) = op_resource.get(&iri(OPERATOR_ARITY_IRI)) else {
            return;
        };

        let Some(arity) = arity_value
            .as_integer()
            .and_then(|a| usize::try_from(a).ok())
        else {
            out.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::OperatorDeclarationMalformed,
                message: format!(
                    "{path}: operator `{op_iri_s}` declares an `operator_arity` that is not a non-negative integer ({arity_value:?}); the App spine cannot be rank-checked"
                ),
            });
            return;
        };

        if arity != spine_len {
            out.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::OperatorArityMismatch,
                message: format!(
                    "{path}: operator `{op_iri_s}` declares arity {arity}; App spine supplies {spine_len} arg(s)"
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{build_core_layer, iri, make_resource};
    use super::super::super::{ValidationRule, Validator};
    use super::FORMULA_TERM_IRI;
    use crate::layer::{Layer, LayerBuilder};
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
    use crate::program::eigentt_type_mirror::CodecNames;
    use std::sync::Arc;

    // ──────────────────────────────────────────────────────────────────
    // Inductive value validation — D32 §3.5
    // ──────────────────────────────────────────────────────────────────

    /// Build a minimal `Nat = zero | succ(Nat)` ontology layer + a
    /// property `nat_value : core:inductive` with `class_types: [Nat]`,
    /// returning the chain layer ready to commit Nat values against.
    fn build_nat_layer() -> Arc<Layer> {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test_nat", Some(core));

        // ctor `zero`: no args.
        let zero_ctor = make_resource(
            "urn:eigenius:test:Nat:zero",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri(wk::INDUCTIVE_CTOR).as_str().to_string(),
                    )]),
                ),
                (wk::CTOR_NAME, Value::String("zero".into())),
                (wk::ARG_TYPES, Value::Array(vec![])),
            ],
        );

        // ctor `succ(pred: Nat)`.
        let succ_arg = make_resource(
            "urn:eigenius:test:Nat:succ:pred",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri(wk::INDUCTIVE_ARG_TYPE).as_str().to_string(),
                    )]),
                ),
                (wk::ARG_NAME, Value::String("pred".into())),
                // `core:type_name` is an `eigentt:Term`, not an IRI string (eigenius#188).
                (
                    wk::TYPE_NAME,
                    crate::testing::term_value(&serde_json::json!({
                        "ctor": "ConstRef", "args": ["urn:eigenius:test:Nat", []],
                    })),
                ),
            ],
        );
        let succ_ctor = make_resource(
            "urn:eigenius:test:Nat:succ",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri(wk::INDUCTIVE_CTOR).as_str().to_string(),
                    )]),
                ),
                (wk::CTOR_NAME, Value::String("succ".into())),
                (
                    wk::ARG_TYPES,
                    Value::Array(vec![Value::Embedded(Box::new(succ_arg))]),
                ),
            ],
        );

        let nat = make_resource(
            "urn:eigenius:test:Nat",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri(wk::INDUCTIVE_TYPE).as_str().to_string(),
                    )]),
                ),
                (wk::SHORT_NAME, Value::String("Nat".into())),
                (
                    wk::CTORS,
                    Value::Array(vec![
                        Value::Embedded(Box::new(zero_ctor)),
                        Value::Embedded(Box::new(succ_ctor)),
                    ]),
                ),
            ],
        );

        // Property `nat_value : core:inductive` typed at Nat.
        let nat_value_prop = make_resource(
            "urn:eigenius:test:nat_value",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("nat_value".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:test:Nat").as_str().to_string(),
                    )]),
                ),
            ],
        );

        builder.add_resource(nat).unwrap();
        builder.add_resource(nat_value_prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// Turn a tagged dict into the value resource it names, against the layer that declares
    /// the inductive. Thin wrapper over [`CodecNames::value_of_tagged`] — the one
    /// implementation — with the test-local inductive as the preference.
    fn value_of(layer: &Layer, inductive: &str, tagged: &serde_json::Value) -> Value {
        CodecNames::from_layer(layer)
            .value_of_tagged(&[inductive], tagged)
            .expect("fixture literal is a value")
    }

    const NAT: &str = "urn:eigenius:test:Nat";
    const LEAN_EXPR: &str = "urn:eigenius:lean:LeanExpr";

    /// `succ(succ(zero))` as a JSON tagged-dict tree.
    fn nat_succ_succ_zero() -> serde_json::Value {
        serde_json::json!({
            "ctor": "succ",
            "args": [{
                "ctor": "succ",
                "args": [{
                    "ctor": "zero",
                    "args": []
                }]
            }]
        })
    }

    #[test]
    fn inductive_value_validates_succ_succ_zero() {
        let nat_layer = build_nat_layer();
        let value = value_of(&nat_layer, NAT, &nat_succ_succ_zero());
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let holder = make_resource(
            "urn:eigenius:test:n2",
            vec![("urn:eigenius:test:nat_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        // The holder itself is a bare resource with no `is_a`, which is its own
        // diagnostic; what this test asserts is that nothing is reported about the Nat
        // value it carries.
        let errors: Vec<_> = Validator::new(Arc::clone(&layer))
            .validate()
            .into_iter()
            .filter(|e| e.message.contains("Nat") || e.message.contains("nat_value"))
            .collect();
        assert!(
            errors.is_empty(),
            "expected succ(succ(zero)) to validate; got {errors:?}"
        );
    }

    /// **An undeclared constructor is rejected.** Rule 16 no longer walks the value,
    /// so this is the ordinary class check doing the work: the value names
    /// `Nat-infinity` in `is_a`, and no such class exists because the inductive
    /// declares no `infinity` constructor for `LayerBuilder::build` to derive one from.
    #[test]
    fn inductive_value_rejects_unknown_ctor() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let mut infinity =
            crate::ontology::resource::Resource::new(iri("urn:eigenius:test:Nat-infinity-value"));
        infinity.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(format!("{NAT}-infinity"))]),
        );
        let bad = make_resource(
            "urn:eigenius:test:bad",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Embedded(Box::new(infinity)),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors.iter().any(|e| e.message.contains("Nat-infinity")),
            "expected the unknown constructor class to be reported; got {errors:?}"
        );
    }

    /// **A missing constructor argument is rejected.** `succ` takes one argument, so
    /// its derived class `requires` `Nat-succ-pred`; omitting it is Rule 1's
    /// `MissingRequiredProperty`, not a separate inductive arity check.
    #[test]
    fn inductive_value_rejects_arity_mismatch() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let mut succ =
            crate::ontology::resource::Resource::new(iri("urn:eigenius:test:succ-of-nothing"));
        succ.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(format!("{NAT}-succ"))]),
        );
        let bad = make_resource(
            "urn:eigenius:test:bad_arity",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Embedded(Box::new(succ)),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors
                .iter()
                .any(|e| e.rule == ValidationRule::MissingRequired
                    && e.message.contains("Nat-succ-pred")),
            "expected the missing constructor argument to be reported; got {errors:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // FormulaTerm App-spine arity check — D32 §5.4 / Phase 19d.0.d
    // ──────────────────────────────────────────────────────────────────

    /// Build a layer chain rooted at the embedded core+formulas
    /// ontologies plus a property `formula_value : core:inductive`
    /// typed at FormulaTerm. Used by the arity-check tests.
    fn build_formula_layer() -> Arc<Layer> {
        // Reuse bootstrap so the formulas: layer (with FormulaTerm +
        // operator catalog) sits in the chain.
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap with formulas layer");
        let formulas = Arc::clone(ctx.head().parent().expect("notebook has parent"));

        let mut builder = LayerBuilder::new("test_formula", Some(formulas));
        let prop = make_resource(
            "urn:eigenius:test:formula_value",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("formula_value".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:formulas:FormulaTerm")
                            .as_str()
                            .to_string(),
                    )]),
                ),
            ],
        );
        builder.add_resource(prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// `App(App(OpRef("formulas:ops:add"), Var("x")), LitFloat(2.0))`
    /// — well-formed binary `add` invocation.
    fn add_x_2() -> serde_json::Value {
        serde_json::json!({
            "ctor": "App",
            "args": [
                {
                    "ctor": "App",
                    "args": [
                        {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                        {"ctor": "Var", "args": ["x"]}
                    ]
                },
                {"ctor": "LitFloat", "args": [2.0]}
            ]
        })
    }

    #[test]
    fn formula_term_well_formed_app_validates() {
        let layer = build_formula_layer();
        let value = value_of(&layer, FORMULA_TERM_IRI, &add_x_2());
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:f1",
            vec![("urn:eigenius:test:formula_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let arity_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorArityMismatch)
            .collect();
        assert!(
            arity_errors.is_empty(),
            "well-formed `add(x, 2)` must not raise OperatorArityMismatch; got {arity_errors:?}"
        );
    }

    #[test]
    fn formula_term_app_rejects_arity_short() {
        // `App(OpRef("add"), x)` — missing the second add argument.
        let underapplied = serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                {"ctor": "Var", "args": ["x"]}
            ]
        });

        let layer = build_formula_layer();
        let value = value_of(&layer, FORMULA_TERM_IRI, &underapplied);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_arity",
            vec![("urn:eigenius:test:formula_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let arity_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorArityMismatch)
            .collect();
        assert_eq!(
            arity_errors.len(),
            1,
            "expected one OperatorArityMismatch for under-applied `add`; got {errors:?}"
        );
        assert!(
            arity_errors[0].message.contains("formulas:ops:add"),
            "error must name the offending operator: {}",
            arity_errors[0].message
        );
        assert!(
            arity_errors[0].message.contains("arity 2"),
            "error must mention the declared arity: {}",
            arity_errors[0].message
        );
    }

    #[test]
    fn formula_term_app_rejects_arity_long() {
        // `App(App(App(OpRef("neg"), x), y), z)` — `neg` is unary;
        // the spine supplies three args.
        let overapplied = serde_json::json!({
            "ctor": "App",
            "args": [
                {
                    "ctor": "App",
                    "args": [
                        {
                            "ctor": "App",
                            "args": [
                                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:neg"]},
                                {"ctor": "Var", "args": ["x"]}
                            ]
                        },
                        {"ctor": "Var", "args": ["y"]}
                    ]
                },
                {"ctor": "Var", "args": ["z"]}
            ]
        });

        let layer = build_formula_layer();
        let value = value_of(&layer, FORMULA_TERM_IRI, &overapplied);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_arity_long",
            vec![("urn:eigenius:test:formula_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let arity_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorArityMismatch)
            .collect();
        assert!(
            !arity_errors.is_empty(),
            "expected an OperatorArityMismatch for over-applied `neg`; got {errors:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Rule 17 operator-resolution diagnostics — eigenius#162
    //
    // Before this landed, every one of the shapes below fell through
    // the `if let` cascade and committed with no diagnostic at all.
    // ──────────────────────────────────────────────────────────────────

    /// Commit `term` under `test:formula_value` on a layer above the
    /// bootstrap formulas layer and return the whole error list.
    fn validate_formula_term(term: serde_json::Value) -> Vec<super::super::super::ValidationError> {
        let layer = build_formula_layer();
        let value = value_of(&layer, FORMULA_TERM_IRI, &term);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:op_ref_holder",
            vec![("urn:eigenius:test:formula_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        Validator::new(layer).validate()
    }

    /// `App(OpRef(<iri>), Var("x"))` — a unary application whose head
    /// names `iri`.
    fn app_of(op_iri: &str) -> serde_json::Value {
        serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "OpRef", "args": [op_iri]},
                {"ctor": "Var", "args": ["x"]}
            ]
        })
    }

    #[test]
    fn op_ref_rejects_unparseable_iri() {
        // Bare `neg` has no scheme, so it is not an IRI at all.
        let errors = validate_formula_term(app_of("neg"));
        let unknown: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UnknownOperator)
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "expected one UnknownOperator for a scheme-less OpRef operand; got {errors:?}"
        );
        assert!(
            unknown[0].message.contains("not a well-formed IRI"),
            "error must say the operand isn't an IRI: {}",
            unknown[0].message
        );
    }

    #[test]
    fn op_ref_rejects_unresolved_iri() {
        // `ops:ad` is one character short of the catalogued `ops:add`
        // — the typo the rule exists to catch.
        let errors = validate_formula_term(app_of("urn:eigenius:formulas:ops:ad"));
        let unknown: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UnknownOperator)
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "expected one UnknownOperator for an unresolvable operator IRI; got {errors:?}"
        );
        assert!(
            unknown[0].message.contains("urn:eigenius:formulas:ops:ad")
                && unknown[0].message.contains("does not resolve"),
            "error must name the unresolved IRI: {}",
            unknown[0].message
        );
    }

    #[test]
    fn op_ref_rejects_non_operator_target() {
        // Resolves fine, but `core:Class` is not a `formulas:Operator`.
        let errors = validate_formula_term(app_of("urn:eigenius:core:Class"));
        let unknown: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UnknownOperator)
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "expected one UnknownOperator for a non-Operator target; got {errors:?}"
        );
        assert!(
            unknown[0].message.contains("not a `formulas:Operator`"),
            "error must say the target isn't an Operator: {}",
            unknown[0].message
        );
    }

    /// Commit a `formulas:Operator` carrying `arity` as its
    /// `operator_arity`, plus a unary application of it, and return the
    /// error list. Lets the arity slot be filled with any `Value` so
    /// malformed declarations can be exercised.
    fn validate_operator_with_arity(arity: Value) -> Vec<super::super::super::ValidationError> {
        let layer = build_formula_layer();
        let value = value_of(
            &layer,
            FORMULA_TERM_IRI,
            &app_of("urn:eigenius:test:ops:bad"),
        );
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let op = make_resource(
            "urn:eigenius:test:ops:bad",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:formulas:Operator").as_str().to_string(),
                    )]),
                ),
                (wk::SHORT_NAME, Value::String("bad".into())),
                ("urn:eigenius:formulas:operator_arity", arity),
            ],
        );
        top.add_resource(op).unwrap();
        let holder = make_resource(
            "urn:eigenius:test:op_ref_holder",
            vec![("urn:eigenius:test:formula_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        Validator::new(layer).validate()
    }

    #[test]
    fn op_ref_rejects_non_integer_arity() {
        let errors = validate_operator_with_arity(Value::String("two".into()));
        let malformed: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorDeclarationMalformed)
            .collect();
        assert_eq!(
            malformed.len(),
            1,
            "expected one OperatorDeclarationMalformed for a string arity; got {errors:?}"
        );
        assert!(
            malformed[0].message.contains("urn:eigenius:test:ops:bad"),
            "error must name the offending operator: {}",
            malformed[0].message
        );
        assert!(
            errors
                .iter()
                .all(|e| e.rule != ValidationRule::OperatorArityMismatch),
            "a malformed arity must not also be reported as a rank mismatch; got {errors:?}"
        );
    }

    #[test]
    fn op_ref_rejects_negative_arity() {
        // `-1` is an integer but not a possible argument count. The old
        // `(arity as usize)` comparison wrapped it to
        // 18446744073709551615 and reported the malformed declaration
        // as a rank mismatch against the invoking resource.
        let errors = validate_operator_with_arity(Value::Integer(-1));
        let malformed: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorDeclarationMalformed)
            .collect();
        assert_eq!(
            malformed.len(),
            1,
            "expected one OperatorDeclarationMalformed for a negative arity; got {errors:?}"
        );
    }

    #[test]
    fn op_ref_accepts_operator_without_arity() {
        // `operator_arity` is only `recommends` on `formulas:Operator`
        // (the class `requires` `operator_signature`), so an operator
        // declaring no arity is schema-conformant and Rule 17 has
        // nothing to check. Pins the deferred half of eigenius#163:
        // if the authoritative property changes, this test changes with
        // it deliberately rather than by accident.
        let layer = build_formula_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let op = make_resource(
            "urn:eigenius:test:ops:no_arity",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:formulas:Operator").as_str().to_string(),
                    )]),
                ),
                (wk::SHORT_NAME, Value::String("no_arity".into())),
            ],
        );
        top.add_resource(op).unwrap();
        let holder = make_resource(
            "urn:eigenius:test:op_ref_holder",
            vec![(
                "urn:eigenius:test:formula_value",
                Value::Json(app_of("urn:eigenius:test:ops:no_arity")),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(layer).validate();
        assert!(
            errors
                .iter()
                .all(|e| e.rule != ValidationRule::UnknownOperator
                    && e.rule != ValidationRule::OperatorDeclarationMalformed
                    && e.rule != ValidationRule::OperatorArityMismatch),
            "an Operator without `operator_arity` must raise no Rule 17 diagnostic; got {errors:?}"
        );
    }

    /// **A non-string `OpRef` operand is diagnosed where the operand is declared.**
    /// Rule 17 stays quiet on it; the derived property `FormulaTerm-OpRef-iri` carries
    /// `data_type: core:string`, so Rule 5 reports it.
    #[test]
    fn op_ref_with_non_string_operand_is_caught_elsewhere() {
        let layer = build_formula_layer();
        let mut op_ref =
            crate::ontology::resource::Resource::new(iri("urn:eigenius:test:bad_op_ref"));
        op_ref.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(format!("{FORMULA_TERM_IRI}-OpRef"))]),
        );
        op_ref.set(
            iri(&format!("{FORMULA_TERM_IRI}-OpRef-iri")),
            Value::Integer(42),
        );
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:op_ref_holder",
            vec![(
                "urn:eigenius:test:formula_value",
                Value::Embedded(Box::new(op_ref)),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(layer).validate();
        assert!(
            errors.iter().any(|e| e.rule == ValidationRule::TypeMismatch
                && e.property
                    .as_ref()
                    .is_some_and(|p| p.as_str().ends_with("OpRef-iri"))),
            "a non-string OpRef operand must be diagnosed at its declared type; got {errors:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Phase 20a.2 — D40 chain-mirrored Lean expressions
    // ──────────────────────────────────────────────────────────────────

    /// Build a layer chain rooted at the embedded bootstrap (which now
    /// carries `lean:LeanExpr` + siblings per D40) plus a property
    /// `proposition_value : core:inductive` typed at `lean:LeanExpr`.
    /// The chain looks like: core → program → reflection → institution
    /// → runtime → formulas → lean-expressions → <this test layer>.
    fn build_lean_expr_layer() -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap with lean-expressions layer");
        // After Phase 20a.4 the chain is:
        //   notebook → lean-institution → lean-expressions → formulas → …
        // We anchor at lean-institution (`ctx.head().parent()`) so the
        // test layer has both the lean:LeanExpr InductiveTypes
        // (resolved through `lean-expressions`) and the
        // institution-side classes (LeanProofTerm etc.) reachable —
        // notebook would also work, but anchoring above it keeps the
        // chain focused.
        let lean_layer = Arc::clone(
            ctx.head()
                .parent()
                .expect("head has lean-institution parent"),
        );

        let mut builder = LayerBuilder::new("test_lean_expr", Some(lean_layer));
        let prop = make_resource(
            "urn:eigenius:test:proposition_value",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("proposition_value".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:lean:LeanExpr").as_str().to_string(),
                    )]),
                ),
            ],
        );
        builder.add_resource(prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// `Lambda { binder_name = Str(Anon, "x"), binder_style = "default",
    ///           binder_type = Const(Str(Anon, "Nat"), Nil),
    ///           body = Var(0) }`
    /// ≈ `λ x : Nat, x` — the smallest non-trivial closed Lean term.
    fn lambda_x_in_nat() -> serde_json::Value {
        let anon = serde_json::json!({"ctor": "Anon"});
        let name_x = serde_json::json!({
            "ctor": "Str",
            "args": [anon.clone(), "x"]
        });
        let name_nat = serde_json::json!({
            "ctor": "Str",
            "args": [anon.clone(), "Nat"]
        });
        let nil = serde_json::json!({"ctor": "Nil"});
        serde_json::json!({
            "ctor": "Lambda",
            "args": [
                name_x,
                "default",
                {
                    "ctor": "Const",
                    "args": [name_nat, nil]
                },
                {"ctor": "Var", "args": [0]}
            ]
        })
    }

    #[test]
    fn lean_expr_lambda_x_in_nat_validates() {
        // Phase 20a.2 acceptance test: a hand-encoded `λ x : Nat, x`
        // value commits cleanly against the chain-mirrored LeanExpr
        // ontology. Each node is a resource stating its constructor's class, so
        // Rule 23 recurses into it and Rules 1, 5 and 6 check its arguments —
        // through LeanName / LeanLevelList / LeanExpr. No validator errors means
        // every cross-reference resolved and the ontology layer is structurally
        // consistent.
        let layer = build_lean_expr_layer();
        let value = value_of(&layer, LEAN_EXPR, &lambda_x_in_nat());
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p1",
            vec![("urn:eigenius:test:proposition_value", value)],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors: Vec<_> = Validator::new(Arc::clone(&layer))
            .validate()
            .into_iter()
            .filter(|e| e.message.contains("Lean"))
            .collect();
        assert!(
            errors.is_empty(),
            "well-formed `λ x : Nat, x` must validate as a lean:LeanExpr; got {errors:?}"
        );
    }

    /// **A constructor the inductive doesn't declare is rejected.** `LeanExpr` has no
    /// `MetaVar`, so `LayerBuilder::build` derives no class for it and the value's
    /// `is_a` names something that does not resolve.
    #[test]
    fn lean_expr_unknown_ctor_rejected() {
        let layer = build_lean_expr_layer();
        let mut meta_var =
            crate::ontology::resource::Resource::new(iri("urn:eigenius:test:meta_var"));
        meta_var.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(
                "urn:eigenius:lean:LeanExpr-MetaVar".into(),
            )]),
        );
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_ctor",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Embedded(Box::new(meta_var)),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("LeanExpr-MetaVar")),
            "unknown ctor `MetaVar` must be reported; got {errors:?}"
        );
    }

    #[test]
    fn lean_expr_resolves_lean_layer_inductives() {
        // Sanity check: after bootstrap, every LeanExpr-related
        // InductiveType is reachable from the head as
        // `is_instance_of(core:InductiveType)`. Catches typos in the
        // ontology JSON or missing entries in the layer chain.
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());
        for ind_iri in &[
            "urn:eigenius:lean:LeanName",
            "urn:eigenius:lean:LeanLevel",
            "urn:eigenius:lean:LeanLevelList",
            "urn:eigenius:lean:LeanExpr",
        ] {
            let parsed = iri(ind_iri);
            let resolved = head.resolve(&parsed).unwrap_or_else(|| {
                panic!("`{ind_iri}` should resolve from the bootstrap chain head")
            });
            assert!(
                resolved.is_instance_of(&iri(wk::INDUCTIVE_TYPE)),
                "`{ind_iri}` should be an InductiveType"
            );
        }
    }

    /// Build a layer with a property `proposition_value` whose
    /// `data_type` is the caller-supplied IRI and whose `class_types`
    /// references `lean:LeanExpr`. Powers the Option A tests that
    /// exercise `core:resource` / `core:resource_array` carrying
    /// inductive values without going through `core:inductive`.
    fn build_lean_expr_property_layer(data_type_iri: &str) -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap with lean-expressions layer");
        let lean_layer = Arc::clone(
            ctx.head()
                .parent()
                .expect("head has lean-institution parent"),
        );

        let mut builder = LayerBuilder::new("test_lean_expr_resource", Some(lean_layer));
        let prop = make_resource(
            "urn:eigenius:test:proposition_value",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("proposition_value".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(data_type_iri))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:lean:LeanExpr").as_str().to_string(),
                    )]),
                ),
            ],
        );
        builder.add_resource(prop).unwrap();
        // Holder class for the option_a tests' propositional carriers.
        // The validator now requires every resource to have at least
        // one is_a class — this declares a minimal placeholder class
        // (no required / recommended properties) so the holder
        // resources can satisfy that without inheriting any
        // class-typing constraints that would interfere with the test.
        let holder_class = make_resource(
            "urn:eigenius:test:PropositionHolder",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::CLASS))])),
                (wk::SHORT_NAME, Value::String("PropositionHolder".into())),
                (
                    wk::DESCRIPTION,
                    Value::String("test placeholder class".into()),
                ),
            ],
        );
        builder.add_resource(holder_class).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// Option A: `data_type: core:resource` with an `InductiveType`
    /// `class_types` accepts a single `Value::Json` carrying the
    /// inductive value, and the validator walks it the same way as
    /// `data_type: core:inductive`.
    #[test]
    fn option_a_resource_with_inductive_class_types_accepts_a_value() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE);
        let value = value_of(&layer, LEAN_EXPR, &lambda_x_in_nat());
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_single",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:test:PropositionHolder")
                            .as_str()
                            .to_string(),
                    )]),
                ),
                ("urn:eigenius:test:proposition_value", value),
            ],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors.is_empty(),
            "`resource` + InductiveType class_types must accept a well-formed Json value; got {errors:?}"
        );
    }

    /// Option A: `data_type: core:resource_array` with an
    /// `InductiveType` `class_types` accepts an `Array` of
    /// `Value::Json` elements; each element is walked against the
    /// declared inductive.
    #[test]
    fn option_a_resource_array_with_inductive_class_types_accepts_a_value_array() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE_ARRAY);
        let value = value_of(&layer, LEAN_EXPR, &lambda_x_in_nat());
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_array",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:test:PropositionHolder")
                            .as_str()
                            .to_string(),
                    )]),
                ),
                (
                    "urn:eigenius:test:proposition_value",
                    Value::Array(vec![value.clone(), value]),
                ),
            ],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors.is_empty(),
            "`resource_array` + InductiveType class_types must accept Array<Json>; got {errors:?}"
        );
    }

    /// **A bad constructor in a `resource_array` element is rejected.** Every element
    /// is a value resource in its own right, so the element naming a class that does
    /// not resolve is reported like any other.
    #[test]
    fn option_a_resource_array_with_bad_ctor_rejects() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE_ARRAY);
        let good = value_of(&layer, LEAN_EXPR, &lambda_x_in_nat());
        let mut bogus =
            crate::ontology::resource::Resource::new(iri("urn:eigenius:test:does_not_exist"));
        bogus.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(format!("{LEAN_EXPR}-DoesNotExist"))]),
        );
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_array_bad",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Array(vec![good, Value::Embedded(Box::new(bogus))]),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("LeanExpr-DoesNotExist")),
            "bad ctor in array must be reported; got {errors:?}"
        );
    }

    /// Regression: a `resource_array` property with a Class
    /// `class_types` still rejects a `Value::Json` element at the
    /// wire-shape check — Option A only loosens the gate when
    /// `class_types` resolves to an `InductiveType`.
    #[test]
    fn option_a_resource_array_with_class_class_types_rejects_json() {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());

        let mut builder = LayerBuilder::new("test_class_array", Some(head));
        // Declare a small Class so class_types resolves.
        let some_class = make_resource(
            "urn:eigenius:test:SomeClass",
            vec![(wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::CLASS))]))],
        );
        let prop = make_resource(
            "urn:eigenius:test:class_array_prop",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("class_array_prop".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(wk::RESOURCE_ARRAY))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:test:SomeClass").as_str().to_string(),
                    )]),
                ),
            ],
        );
        builder.add_resource(some_class).unwrap();
        builder.add_resource(prop).unwrap();
        let lay = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let mut top = LayerBuilder::new("test_top", Some(lay));
        let holder = make_resource(
            "urn:eigenius:test:p_class",
            vec![(
                "urn:eigenius:test:class_array_prop",
                Value::Array(vec![Value::Json(serde_json::json!({"ctor": "Whatever"}))]),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let type_mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::TypeMismatch)
            .collect();
        assert!(
            !type_mismatches.is_empty(),
            "Class class_types must keep rejecting Json elements at the wire-shape gate; got {errors:?}"
        );
    }

    /// **A constructor argument of the wrong type is rejected.** `succ`'s argument is
    /// declared at `Nat`, so its derived property carries `data_type: core:inductive`
    /// and a string in that slot fails the wire-shape gate — the argument is checked
    /// where it is declared, not by a walk over the value.
    #[test]
    fn inductive_value_rejects_nested_arg_type_mismatch() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let mut succ =
            crate::ontology::resource::Resource::new(iri("urn:eigenius:test:succ-of-str"));
        succ.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(format!("{NAT}-succ"))]),
        );
        succ.set(
            iri(&format!("{NAT}-succ-pred")),
            Value::String("not_a_nat".into()),
        );
        let bad = make_resource(
            "urn:eigenius:test:bad_nested",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Embedded(Box::new(succ)),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors.iter().any(|e| e.rule == ValidationRule::TypeMismatch
                && e.property
                    .as_ref()
                    .is_some_and(|p| p.as_str().ends_with("Nat-succ-pred"))),
            "expected the mistyped constructor argument to be reported; got {errors:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // EigenTTType ConstRef resolution (D47 §5 / Phase 4)
    // ──────────────────────────────────────────────────────────────────

    /// Build a chain with the bootstrap layers (core + eigentt-type-fragment),
    /// plus a top layer carrying a property `eigentt_value : core:inductive`
    /// typed at `eigentt:Term`. The top layer is also seeded with a
    /// no-op auxiliary `Property` resource at `urn:eigenius:test:wrong_class`
    /// — used by the wrong-class test as a `ConstRef` target whose primary
    /// class isn't one of the type-former classes.
    fn build_eigentt_test_chain() -> Arc<Layer> {
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut builder = LayerBuilder::new("test_eigentt_top", Some(head));

        // Property `eigentt_value : core:inductive` typed at eigentt:Term.
        let prop = make_resource(
            "urn:eigenius:test:eigentt_value",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("eigentt_value".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String(
                        iri("urn:eigenius:eigentt:Term").as_str().to_string(),
                    )]),
                ),
                (
                    // A TYPE slot, so the decode failure this test is about is
                    // what surfaces rather than a type mismatch.
                    wk::IS_A_TYPE,
                    Value::Boolean(true),
                ),
            ],
        );

        // A Property-class auxiliary resource used as a "wrong-class ConstRef
        // target" in the negative test. Its primary class is Property, not
        // Class/DataType/Inductive/Codata.
        let wrong_class_target = make_resource(
            "urn:eigenius:test:wrong_class",
            vec![
                (wk::IS_A, Value::Array(vec![Value::iri(&iri(wk::PROPERTY))])),
                (wk::SHORT_NAME, Value::String("wrong_class".into())),
                (wk::DATA_TYPE_PROP, Value::iri(&iri(wk::STRING))),
            ],
        );

        builder.add_resource(prop).unwrap();
        builder.add_resource(wrong_class_target).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn eigentt_core_inductive_prop_is_validated_by_rule_21() {
        // A `core:inductive` property ranged at `eigentt:Term` is carved
        // out of `check_inductive_value` (Rule 16) and validated end-to-end by
        // Rule 21 (`check_type_expr_well_typed`, eigentt_value.rs). A bad value
        // (here an unresolved `ConstRef`) must therefore be rejected as
        // `TermMalformed` — proving the carve routes core:inductive eigentt
        // values to the single eigentt owner, not the (removed) bespoke walk.
        //
        // Comprehensive eigentt-value coverage lives with the owners now: the
        // codec's own decode-rejection tests (`eigentt_type_mirror`) and the
        // rule's tests (`validation::rules::eigentt_value`).
        let chain = build_eigentt_test_chain();
        let mut top = LayerBuilder::new("test_carve", Some(chain));
        let holder = make_resource(
            "urn:eigenius:test:carve_bad",
            vec![(
                "urn:eigenius:test:eigentt_value",
                crate::testing::term_value(&serde_json::json!({
                    "ctor": "ConstRef",
                    "args": ["urn:eigenius:nonexistent:Foo", []]
                })),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let malformed: Vec<_> = Validator::new(layer)
            .validate()
            .into_iter()
            .filter(|e| matches!(e.rule, ValidationRule::TermMalformed))
            .collect();
        assert_eq!(
            malformed.len(),
            1,
            "core:inductive eigentt value with an unresolved ConstRef must be rejected by \
             Rule 21 (the carve); got {malformed:?}"
        );
        assert!(malformed[0]
            .message
            .contains("urn:eigenius:nonexistent:Foo"));
    }
}
