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

//! Rule 16 (inductive value type-check, D32 §3.5) and Rule 17 (FormulaTerm
//! App-spine arity check, D32 §5.4 / Phase 19d.0.d). Both walk inductive
//! tagged-dict trees; the latter is a FormulaTerm-specific arity check
//! layered on top.

use std::sync::Arc;

use super::super::{iri, ValidationError, ValidationRule, Validator};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

/// FormulaTerm InductiveType IRI (D32 §4). Pinned here so the
/// formula-specific arity rule can short-circuit before doing any
/// chain resolution work.
const FORMULA_TERM_IRI: &str = "urn:eigenius:formulas:FormulaTerm";

/// Operator.operator_arity property IRI. Convenience integer; the
/// rank check uses it as a fast-path before the full
/// operator_signature walk (deferred to a follow-on landing).
const OPERATOR_ARITY_IRI: &str = "urn:eigenius:formulas:operator_arity";

/// EigenTTType InductiveType IRI (D47 §3). Pinned here so the
/// `ConstRef` resolution check (D47 §5) can short-circuit when
/// the inductive being walked isn't `eigentt:TypeExpr`.
const EIGENTT_TYPE_EXPR_IRI: &str = "urn:eigenius:eigentt:TypeExpr";

/// Walk the left spine of an `App(App(App(head, a₃), a₂), a₁)` tree
/// and return `(head, [a₁, a₂, a₃])`. Spine args are emitted
/// **right-to-left** as the spine is traversed; the caller may want
/// to reverse if argument order matters semantically. For the arity
/// check, only the count matters so the order is irrelevant.
fn collect_app_spine(node: &serde_json::Value) -> (serde_json::Value, Vec<serde_json::Value>) {
    let mut spine = Vec::new();
    let mut cursor = node.clone();
    loop {
        let Some(obj) = cursor.as_object() else {
            return (cursor, spine);
        };
        let ctor = obj.get("ctor").and_then(|v| v.as_str()).unwrap_or("");
        if ctor != "App" {
            return (cursor, spine);
        }
        let args = obj
            .get("args")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if args.len() != 2 {
            return (cursor, spine);
        }
        let mut iter = args.into_iter();
        let head = iter.next().expect("len 2");
        let arg = iter.next().expect("len 2");
        spine.push(arg);
        cursor = head;
    }
}

impl Validator {
    /// Resolve `class_types` to an `InductiveType` resource when the
    /// property declares exactly one entry pointing to one. Returns
    /// `None` for the Class case (the original `class_types`
    /// semantics) and for mixed/empty lists. Powers the Option A
    /// unification across `core:resource`, `core:resource_array`,
    /// and (implicitly, via the singleton constraint) `core:inductive`.
    pub(in crate::validation) fn class_types_inductive_target(
        &self,
        prop_def: &Resource,
    ) -> Option<Arc<Resource>> {
        let class_iris = prop_def.get(&iri(wk::CLASS_TYPES))?.as_iri_array();
        if class_iris.len() != 1 {
            return None;
        }
        let target = self.layer.resolve(&class_iris[0])?;
        if target.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) {
            Some(target)
        } else {
            None
        }
    }

    /// Rule 16: Inductive value type-checking (D32 §3.5).
    ///
    /// When a property has `data_type: core:inductive`, its `class_types`
    /// must declare exactly one `core:InductiveType`, and the value must
    /// be a tagged-dict tree (`{ "ctor": ..., "args": [...] }`) whose
    /// every node corresponds to a ctor declared on the inductive and
    /// whose every arg matches the ctor's declared `arg_types[i].type_name`.
    /// Errors carry structured paths so users see
    /// `term.args[0].args[1]: ctor 'foo' not declared on FormulaTerm`.
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

        // eigentt:TypeExpr values are validated end to end by Rule 21
        // (`check_type_expr_well_typed`, eigentt_value.rs): decode + NbE
        // type-check. Skip the generic inductive walk here so the two don't
        // produce duplicate diagnostics — Rule 21 is the single eigentt owner.
        if ind_iri.as_str() == EIGENTT_TYPE_EXPR_IRI {
            return vec![];
        }

        let mut errors = Vec::new();
        self.walk_inductive_value(
            value,
            &ind_type,
            prop_iri.as_str().to_string(),
            res_id,
            &mut errors,
        );
        errors
    }

    /// Recursively type-check an inductive value tree against an
    /// `InductiveType` resource. `path` accumulates a structured trace
    /// (`term.args[0].args[1]`) for diagnostic clarity.
    pub(in crate::validation) fn walk_inductive_value(
        &self,
        value: &Value,
        inductive_type: &Resource,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        let json = match value {
            Value::Json(j) => j,
            other => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: expected JSON tagged-dict for inductive value, got {other:?}"
                    ),
                });
                return;
            }
        };

        let obj = match json.as_object() {
            Some(o) => o,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!("{path}: inductive value must be a JSON object"),
                });
                return;
            }
        };

        let ctor_name = match obj.get("ctor").and_then(serde_json::Value::as_str) {
            Some(s) => s,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!("{path}: inductive value missing string `ctor` field"),
                });
                return;
            }
        };

        let args_array = obj
            .get("args")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        // Find the ctor declaration on the inductive type.
        let ctors_value = match inductive_type.get(&iri(wk::CTORS)) {
            Some(v) => v,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: InductiveType `{}` has no `ctors` declared",
                        inductive_type.id().map(|i| i.as_str()).unwrap_or("?"),
                    ),
                });
                return;
            }
        };
        let ctor_arr = match ctors_value {
            Value::Array(a) => a,
            _ => return, // Earlier rules will have flagged this
        };

        let matching_ctor = ctor_arr.iter().find_map(|c| match c {
            Value::Embedded(r) => {
                let name = r
                    .get(&iri(wk::CTOR_NAME))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name == ctor_name {
                    Some(r.as_ref())
                } else {
                    None
                }
            }
            _ => None,
        });

        let ctor = match matching_ctor {
            Some(c) => c,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: ctor `{ctor_name}` not declared on InductiveType `{}`",
                        inductive_type.id().map(|i| i.as_str()).unwrap_or("?"),
                    ),
                });
                return;
            }
        };

        let arg_types: Vec<Resource> = match ctor.get(&iri(wk::ARG_TYPES)) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Value::Embedded(r) => Some(r.as_ref().clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        if args_array.len() != arg_types.len() {
            out.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::InductiveValueMismatch,
                message: format!(
                    "{path}: ctor `{ctor_name}` expects {} arg(s), got {}",
                    arg_types.len(),
                    args_array.len(),
                ),
            });
            return;
        }

        for (i, (arg_value, arg_type_decl)) in args_array.iter().zip(arg_types.iter()).enumerate() {
            let type_name = arg_type_decl
                .get(&iri(wk::TYPE_NAME))
                .and_then(Value::as_str)
                .unwrap_or("");
            let child_path = format!("{path}.args[{i}]");
            self.check_inductive_arg(arg_value, type_name, child_path, res_id, out);
        }
    }

    /// Validate one argument value against the `type_name` declared on
    /// its `InductiveArgType`. Dispatches on whether `type_name` resolves
    /// to a primitive `DataType`, a `Class`, an `InductiveType`, or a
    /// bare type-parameter name (deferred — parameter-aware checking
    /// lands when parametric inductives have their first chain
    /// consumer; v1 callers use only monomorphic inductives).
    fn check_inductive_arg(
        &self,
        arg_value: &serde_json::Value,
        type_name: &str,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        // Try to parse as IRI and resolve. If it doesn't parse or
        // doesn't resolve, treat as an unbound parameter name and
        // skip (v1 deferral).
        let type_iri = match Iri::parse(type_name) {
            Ok(i) => i,
            Err(_) => return, // Bare parameter name; deferred per v1.
        };

        // Primitive type IRIs are well-known; check inline.
        let ok = match type_name {
            wk::STRING => arg_value.is_string(),
            wk::INTEGER => arg_value.is_i64(),
            wk::FLOAT => arg_value.is_number(),
            wk::BOOLEAN => arg_value.is_boolean(),
            _ => {
                // Resolve to a chain Resource.
                let referent = match self.layer.resolve(&type_iri) {
                    Some(r) => r,
                    None => return, // Treat as unbound parameter; deferred.
                };
                // InductiveType: recurse.
                if referent.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) {
                    self.walk_inductive_value(
                        &Value::Json(arg_value.clone()),
                        &referent,
                        path,
                        res_id,
                        out,
                    );
                    return;
                }
                // Class: arg is an embedded resource ref or IRI string —
                // structural shape only; deeper class-type checking
                // would duplicate `check_class_types` and is deferred.
                // For v1, accept any string (IRI ref) or object (embedded).
                if referent.is_instance_of(&iri(wk::CLASS)) {
                    arg_value.is_string() || arg_value.is_object()
                } else {
                    // Unknown referent kind — skip silently.
                    true
                }
            }
        };

        if !ok {
            out.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::InductiveValueMismatch,
                message: format!("{path}: value does not match declared `type_name` `{type_name}`"),
            });
        }
    }

    /// Rule 17: FormulaTerm App-spine arity check (D32 §5.4).
    ///
    /// When a property's value is a FormulaTerm whose outer ctor is
    /// `App`, walk the left spine to find the head. If the head is an
    /// `OpRef(iri)` whose target resolves to an `Operator` resource
    /// with a declared `operator_arity`, confirm the App spine
    /// supplies exactly that many arguments. This catches typos like
    /// `App(OpRef("add"), x)` (one arg short) at commit time rather
    /// than at dispatch.
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
    ) -> Vec<ValidationError> {
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

        let json = match value {
            Value::Json(j) => j,
            _ => return vec![],
        };

        let mut errors = Vec::new();
        self.walk_formula_term_app_arity(json, prop_iri.as_str().to_string(), res_id, &mut errors);
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
        node: &serde_json::Value,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        let obj = match node.as_object() {
            Some(o) => o,
            None => return,
        };
        let ctor = obj
            .get("ctor")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if ctor == "App" {
            let (head, spine_args) = collect_app_spine(node);
            if let Some(head_obj) = head.as_object() {
                let head_ctor = head_obj
                    .get("ctor")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if head_ctor == "OpRef" {
                    let op_iri_s = head_obj
                        .get("args")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(serde_json::Value::as_str);
                    if let Some(op_iri_s) = op_iri_s {
                        if let Ok(op_iri) = Iri::parse(op_iri_s) {
                            if let Some(op_resource) = self.layer.resolve(&op_iri) {
                                if let Some(arity_value) = op_resource.get(&iri(OPERATOR_ARITY_IRI))
                                {
                                    if let Some(arity) = arity_value.as_integer() {
                                        if (arity as usize) != spine_args.len() {
                                            out.push(ValidationError {
                                                resource_id: res_id.clone(),
                                                property: None,
                                                rule: ValidationRule::OperatorArityMismatch,
                                                message: format!(
                                                    "{path}: operator `{op_iri_s}` declares arity {arity}; App spine supplies {} arg(s)",
                                                    spine_args.len(),
                                                ),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Recurse only into spine args (NOT into intermediate
            // App heads — those are partial applications counted by
            // the spine, not separate invocations to arity-check).
            // `collect_app_spine` returns args right-to-left from
            // the deepest App; reverse so paths read left-to-right.
            let spine_left_to_right: Vec<&serde_json::Value> = spine_args.iter().rev().collect();
            for (i, arg) in spine_left_to_right.iter().enumerate() {
                let child_path = format!("{path}.args[{i}]");
                self.walk_formula_term_app_arity(arg, child_path, res_id, out);
            }
            return;
        }

        // Non-App node: recurse into every arg so nested applications
        // inside Lam/Pi bodies, OpRef IRIs (no recursion needed —
        // they're string args), etc. get checked too.
        let args = obj
            .get("args")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (i, arg) in args.iter().enumerate() {
            let child_path = format!("{path}.args[{i}]");
            self.walk_formula_term_app_arity(arg, child_path, res_id, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{build_core_layer, iri, make_resource};
    use super::super::super::{ValidationRule, Validator};
    use crate::layer::{Layer, LayerBuilder};
    use crate::ontology::resource::Value;
    use crate::ontology::well_known as wk;
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
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_CTOR))]),
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
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_ARG_TYPE))]),
                ),
                (wk::ARG_NAME, Value::String("pred".into())),
                (wk::TYPE_NAME, Value::String("urn:eigenius:test:Nat".into())),
            ],
        );
        let succ_ctor = make_resource(
            "urn:eigenius:test:Nat:succ",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_CTOR))]),
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
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_TYPE))]),
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
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("nat_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Nat"))]),
                ),
            ],
        );

        builder.add_resource(nat).unwrap();
        builder.add_resource(nat_value_prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

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
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let holder = make_resource(
            "urn:eigenius:test:n2",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(nat_succ_succ_zero()),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            inductive_errors.is_empty(),
            "expected no InductiveValueMismatch on succ(succ(zero)); got {errors:?}"
        );
    }

    #[test]
    fn inductive_value_rejects_unknown_ctor() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let bad = make_resource(
            "urn:eigenius:test:bad",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(serde_json::json!({
                    "ctor": "infinity",
                    "args": []
                })),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert_eq!(
            mismatches.len(),
            1,
            "expected exactly one InductiveValueMismatch for unknown ctor; got {errors:?}"
        );
        assert!(
            mismatches[0].message.contains("infinity"),
            "error must mention the offending ctor name: {}",
            mismatches[0].message
        );
    }

    #[test]
    fn inductive_value_rejects_arity_mismatch() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        // succ takes one arg; supply zero.
        let bad = make_resource(
            "urn:eigenius:test:bad_arity",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(serde_json::json!({
                    "ctor": "succ",
                    "args": []
                })),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert_eq!(
            mismatches.len(),
            1,
            "expected one InductiveValueMismatch for arity mismatch; got {errors:?}"
        );
        assert!(
            mismatches[0].message.contains("expects 1 arg"),
            "error must describe the arity mismatch: {}",
            mismatches[0].message
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
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("formula_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:formulas:FormulaTerm",
                    ))]),
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
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:f1",
            vec![("urn:eigenius:test:formula_value", Value::Json(add_x_2()))],
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
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_arity",
            vec![("urn:eigenius:test:formula_value", Value::Json(underapplied))],
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
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_arity_long",
            vec![("urn:eigenius:test:formula_value", Value::Json(overapplied))],
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
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("proposition_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:lean:LeanExpr"))]),
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
        // ontology — the chain-side type-check walks the tagged-dict
        // shape, dispatches on each ctor name, and recurses into
        // child arguments through LeanName / LeanLevelList / LeanExpr.
        // No validator errors means the inductive-value walker
        // successfully resolved every cross-reference and the
        // ontology layer is structurally consistent.
        let layer = build_lean_expr_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p1",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Json(lambda_x_in_nat()),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            inductive_errors.is_empty(),
            "well-formed `λ x : Nat, x` must validate as a lean:LeanExpr; got {inductive_errors:?}"
        );
    }

    #[test]
    fn lean_expr_unknown_ctor_rejected() {
        // A value with a ctor name the LeanExpr inductive doesn't
        // declare must surface as `InductiveValueMismatch` — exercises
        // the per-ctor lookup in `walk_inductive_value`.
        let bogus = serde_json::json!({
            "ctor": "MetaVar",
            "args": [42]
        });
        let layer = build_lean_expr_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_ctor",
            vec![("urn:eigenius:test:proposition_value", Value::Json(bogus))],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            !inductive_errors.is_empty(),
            "unknown ctor `MetaVar` must trigger an InductiveValueMismatch; got {errors:?}"
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
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("proposition_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(data_type_iri))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:lean:LeanExpr"))]),
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
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
                ),
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
    fn option_a_resource_with_inductive_class_types_accepts_json() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_single",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:test:PropositionHolder",
                    ))]),
                ),
                (
                    "urn:eigenius:test:proposition_value",
                    Value::Json(lambda_x_in_nat()),
                ),
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
    fn option_a_resource_array_with_inductive_class_types_accepts_json_array() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE_ARRAY);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_array",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:test:PropositionHolder",
                    ))]),
                ),
                (
                    "urn:eigenius:test:proposition_value",
                    Value::Array(vec![
                        Value::Json(lambda_x_in_nat()),
                        Value::Json(lambda_x_in_nat()),
                    ]),
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

    /// Option A: malformed inductive value in a `resource_array`
    /// element surfaces as `InductiveValueMismatch` with a structured
    /// path indicating the bad index — the dispatch in
    /// `check_class_types` walks each Json element.
    #[test]
    fn option_a_resource_array_with_bad_ctor_rejects() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE_ARRAY);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let bogus = serde_json::json!({"ctor": "DoesNotExist", "args": []});
        let holder = make_resource(
            "urn:eigenius:test:p_array_bad",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Array(vec![Value::Json(lambda_x_in_nat()), Value::Json(bogus)]),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            !inductive_errors.is_empty(),
            "bad ctor in array must surface as InductiveValueMismatch; got {errors:?}"
        );
        let saw_index_path = inductive_errors.iter().any(|e| e.message.contains("[1]"));
        assert!(
            saw_index_path,
            "error message should reference the failing array index `[1]`; got {inductive_errors:?}"
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
            vec![(
                wk::IS_A,
                Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
            )],
        );
        let prop = make_resource(
            "urn:eigenius:test:class_array_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("class_array_prop".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::ResourceRef(iri(wk::RESOURCE_ARRAY)),
                ),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:SomeClass"))]),
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

    #[test]
    fn inductive_value_rejects_nested_arg_type_mismatch() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        // succ's arg should be a Nat; supply a JSON string.
        let bad = make_resource(
            "urn:eigenius:test:bad_nested",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(serde_json::json!({
                    "ctor": "succ",
                    "args": ["not_a_nat"]
                })),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            !mismatches.is_empty(),
            "expected an InductiveValueMismatch for nested arg type mismatch; got {errors:?}"
        );
        // Path should mention args[0].
        let path_match = mismatches.iter().any(|e| e.message.contains("args[0]"));
        assert!(
            path_match,
            "error must include structured path `args[0]`: {mismatches:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // EigenTTType ConstRef resolution (D47 §5 / Phase 4)
    // ──────────────────────────────────────────────────────────────────

    /// Build a chain with the bootstrap layers (core + eigentt-type-fragment),
    /// plus a top layer carrying a property `eigentt_value : core:inductive`
    /// typed at `eigentt:TypeExpr`. The top layer is also seeded with a
    /// no-op auxiliary `Property` resource at `urn:eigenius:test:wrong_class`
    /// — used by the wrong-class test as a `ConstRef` target whose primary
    /// class isn't one of the type-former classes.
    fn build_eigentt_test_chain() -> Arc<Layer> {
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut builder = LayerBuilder::new("test_eigentt_top", Some(head));

        // Property `eigentt_value : core:inductive` typed at eigentt:TypeExpr.
        let prop = make_resource(
            "urn:eigenius:test:eigentt_value",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("eigentt_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:eigentt:TypeExpr",
                    ))]),
                ),
            ],
        );

        // A Property-class auxiliary resource used as a "wrong-class ConstRef
        // target" in the negative test. Its primary class is Property, not
        // Class/DataType/Inductive/Codata.
        let wrong_class_target = make_resource(
            "urn:eigenius:test:wrong_class",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("wrong_class".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::STRING))),
            ],
        );

        builder.add_resource(prop).unwrap();
        builder.add_resource(wrong_class_target).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn eigentt_core_inductive_prop_is_validated_by_rule_21() {
        // A `core:inductive` property ranged at `eigentt:TypeExpr` is carved
        // out of `check_inductive_value` (Rule 16) and validated end-to-end by
        // Rule 21 (`check_type_expr_well_typed`, eigentt_value.rs). A bad value
        // (here an unresolved `ConstRef`) must therefore be rejected as
        // `TypeExprMalformed` — proving the carve routes core:inductive eigentt
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
                Value::Json(serde_json::json!({
                    "ctor": "ConstRef",
                    "args": ["urn:eigenius:nonexistent:Foo"]
                })),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let malformed: Vec<_> = Validator::new(layer)
            .validate()
            .into_iter()
            .filter(|e| matches!(e.rule, ValidationRule::TypeExprMalformed))
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
