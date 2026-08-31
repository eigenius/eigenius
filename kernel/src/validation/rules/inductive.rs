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

/// `formulas:Operator` class IRI. An `OpRef` head must resolve to an
/// instance of this class (or of a subclass of it) before its
/// `operator_arity` is read.
const OPERATOR_IRI: &str = "urn:eigenius:formulas:Operator";

/// EigenTTType InductiveType IRI (D47 §3). Pinned here so the
/// `ConstRef` resolution check (D47 §5) can short-circuit when
/// the inductive being walked isn't `eigentt:Term`.
const EIGENTT_TYPE_EXPR_IRI: &str = "urn:eigenius:eigentt:Term";

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

        // eigentt:Term values are validated end to end by Rule 21
        // (`check_type_expr_well_typed`, eigentt_value.rs): decode + NbE
        // type-check. Skip the generic inductive walk here so the two don't
        // produce duplicate diagnostics — Rule 21 is the single eigentt owner.
        //
        // `eigentt:Judgement` was exempted alongside it by P5 and is NOT any more (D83
        // §4.3). The exemption was a workaround for a shape disagreement, not a division
        // of labour: a judgement was stored as `CtorApp(eigentt:Judgement, holds, …)`, an
        // `eigentt:Term` value in a slot declaring `eigentt:Judgement`, so this walk read
        // `App` as the constructor and reported "ctor `App` not declared" for every
        // judgement on every chain. A judgement is now written as `holds(…)` — D32 §3.7's
        // tagged dict against the inductive the slot declares — so this walk reads it
        // correctly and Rule 21 keeps only the job the generic walk cannot do: decoding
        // the two terms and NbE-checking one against the other.
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
            // eigenius#188 / N4: `type_name` is an `eigentt:Term` value, so the type this
            // dispatches on is the value's HEAD.
            let child_path = format!("{path}.args[{i}]");
            let type_name = match crate::program::ground::arg_type_head(arg_type_decl) {
                Ok(n) => n,
                Err(e) => {
                    // An argument whose declared type cannot be read is not an argument this rule
                    // can pass. It read `.unwrap_or_default()` here, which turned an unreadable
                    // `type_name` into the empty string and then into a "typed by the parameter
                    // ``" report naming a parameter that does not exist.
                    out.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: None,
                        rule: ValidationRule::InductiveValueMismatch,
                        message: format!("{child_path}: {e}"),
                    });
                    continue;
                }
            };
            // D83 §3.4 — a `cardinality: list` slot holds a JSON array of the element
            // encoding, not one element. Checking the array itself against `type_name`
            // would report every list as a type mismatch.
            if arg_type_decl
                .get(&iri(wk::CARDINALITY))
                .and_then(Value::as_str)
                == Some(wk::CARDINALITY_LIST)
            {
                let Some(elems) = arg_value.as_array() else {
                    out.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: None,
                        rule: ValidationRule::InductiveValueMismatch,
                        message: format!(
                            "{child_path}: `cardinality: list` slot expects a JSON array, got {arg_value}"
                        ),
                    });
                    continue;
                };
                for (k, elem) in elems.iter().enumerate() {
                    self.check_inductive_arg(
                        elem,
                        &type_name,
                        format!("{child_path}[{k}]"),
                        res_id,
                        out,
                    );
                }
                continue;
            }
            self.check_inductive_arg(arg_value, &type_name, child_path, res_id, out);
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
        // A bare name is a type-PARAMETER reference. Parameter-aware checking — instantiating the
        // parameter from the value's own type arguments — is genuinely not built, so this cannot
        // check the value. It says so instead of returning `Ok`.
        //
        // Until eigenius#188 this arm was `Err(_) => return` with the comment "deferred per v1;
        // v1 callers use only monomorphic inductives". That premise was half true: the
        // DECLARATIONS are parametric — `core:Option.some(A)`, `logic:And.conj(P, Q)`,
        // `logic:Or.inl/inr` — and `closed-class.esl` gives English "but" the semantics
        // `λs₂. λs₁. logic:And(s₁, s₂)`, so any parsed sentence containing "but" produces an
        // `And` value whose arguments are typed `P` and `Q`. Silent admission was one prose
        // encoding from mattering.
        let type_iri = match Iri::parse(type_name) {
            Ok(i) => i,
            Err(_) => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: argument is typed by the parameter `{type_name}`, which this \
                         rule cannot check — parameter-aware validation is not built. The value \
                         is NOT validated (eigenius#188 / N4)."
                    ),
                });
                return;
            }
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
                    self.check_op_ref_head(head_obj, spine_args.len(), &path, res_id, out);
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
    /// - **A missing or non-string `OpRef` operand.** Rule 16
    ///   (`walk_inductive_value`) already walks every `OpRef` node in
    ///   the same value against the ctor's declared `arg_types`
    ///   (`iri: core:string`) and raises `InductiveValueMismatch` for
    ///   both the wrong arg count and the wrong arg type. Re-reporting
    ///   it would double-diagnose one defect.
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
        head_obj: &serde_json::Map<String, serde_json::Value>,
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

        // Arg shape is Rule 16's; see the doc comment.
        let Some(op_iri_s) = head_obj
            .get("args")
            .and_then(serde_json::Value::as_array)
            .and_then(|a| a.first())
            .and_then(serde_json::Value::as_str)
        else {
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
                // `core:type_name` is an `eigentt:Term`, not an IRI string (eigenius#188).
                (
                    wk::TYPE_NAME,
                    Value::Json(serde_json::json!({
                        "ctor": "ConstRef", "args": ["urn:eigenius:test:Nat"],
                    })),
                ),
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

    /// A `Nat = zero | succ(Nat) | sum([Nat])` layer — the third constructor takes a
    /// `cardinality: list` slot (D83 §3.4).
    fn build_nat_with_list_ctor_layer() -> Arc<Layer> {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test_nat_list", Some(core));

        let zero_ctor = make_resource(
            "urn:eigenius:test:NatL:zero",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_CTOR))]),
                ),
                (wk::CTOR_NAME, Value::String("zero".into())),
                (wk::ARG_TYPES, Value::Array(vec![])),
            ],
        );
        // ctor `sum(terms: [NatL])` — a list slot.
        let sum_arg = make_resource(
            "urn:eigenius:test:NatL:sum:terms",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_ARG_TYPE))]),
                ),
                (wk::ARG_NAME, Value::String("terms".into())),
                (
                    wk::TYPE_NAME,
                    Value::Json(serde_json::json!({
                        "ctor": "ConstRef", "args": ["urn:eigenius:test:NatL"],
                    })),
                ),
                (
                    wk::CARDINALITY,
                    Value::String(wk::CARDINALITY_LIST.to_string()),
                ),
            ],
        );
        let sum_ctor = make_resource(
            "urn:eigenius:test:NatL:sum",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_CTOR))]),
                ),
                (wk::CTOR_NAME, Value::String("sum".into())),
                (
                    wk::ARG_TYPES,
                    Value::Array(vec![Value::Embedded(Box::new(sum_arg))]),
                ),
            ],
        );
        let nat = make_resource(
            "urn:eigenius:test:NatL",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_TYPE))]),
                ),
                (wk::SHORT_NAME, Value::String("NatL".into())),
                (
                    wk::CTORS,
                    Value::Array(vec![
                        Value::Embedded(Box::new(zero_ctor)),
                        Value::Embedded(Box::new(sum_ctor)),
                    ]),
                ),
            ],
        );
        let prop = make_resource(
            "urn:eigenius:test:natl_value",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("natl_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:NatL"))]),
                ),
            ],
        );
        builder.add_resource(nat).unwrap();
        builder.add_resource(prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    fn natl_errors(value: serde_json::Value) -> Vec<String> {
        let base = build_nat_with_list_ctor_layer();
        let mut top = LayerBuilder::new("test_top", Some(base));
        top.add_resource(make_resource(
            "urn:eigenius:test:nl",
            vec![("urn:eigenius:test:natl_value", Value::Json(value))],
        ))
        .unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        Validator::new(Arc::clone(&layer))
            .validate()
            .into_iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .map(|e| e.message)
            .collect()
    }

    /// D83 §3.4 — a list slot takes a JSON array, and every element is checked.
    #[test]
    fn a_list_slot_accepts_an_array_and_checks_each_element() {
        let errors = natl_errors(serde_json::json!({
            "ctor": "sum",
            "args": [[{"ctor": "zero", "args": []}, {"ctor": "zero", "args": []}]],
        }));
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
    }

    /// An empty list is a well-formed list — this is `CtorApp`'s nullary case, which
    /// every nullary constructor application on the chain now carries.
    #[test]
    fn a_list_slot_accepts_the_empty_array() {
        let errors = natl_errors(serde_json::json!({"ctor": "sum", "args": [[]]}));
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
    }

    /// The element type is still enforced inside the list. Without the per-element walk
    /// a list slot would be the one place an arbitrary value could be smuggled in.
    #[test]
    fn a_list_slot_rejects_a_bad_element() {
        let errors = natl_errors(serde_json::json!({
            "ctor": "sum",
            "args": [[{"ctor": "zero", "args": []}, {"ctor": "nope", "args": []}]],
        }));
        assert!(
            errors.iter().any(|m| m.contains("`nope` not declared")),
            "expected the bad element to be named, got {errors:?}"
        );
    }

    /// A single value where a list is declared is rejected rather than silently read as a
    /// one-element list — the encoding says array, so a non-array is malformed.
    #[test]
    fn a_list_slot_rejects_a_bare_element() {
        let errors = natl_errors(serde_json::json!({
            "ctor": "sum",
            "args": [{"ctor": "zero", "args": []}],
        }));
        assert!(
            errors.iter().any(|m| m.contains("expects a JSON array")),
            "expected an array-shape error, got {errors:?}"
        );
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
    // Rule 17 operator-resolution diagnostics — eigenius#162
    //
    // Before this landed, every one of the shapes below fell through
    // the `if let` cascade and committed with no diagnostic at all.
    // ──────────────────────────────────────────────────────────────────

    /// Commit `term` under `test:formula_value` on a layer above the
    /// bootstrap formulas layer and return the whole error list.
    fn validate_formula_term(term: serde_json::Value) -> Vec<super::super::super::ValidationError> {
        let layer = build_formula_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:op_ref_holder",
            vec![("urn:eigenius:test:formula_value", Value::Json(term))],
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
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let op = make_resource(
            "urn:eigenius:test:ops:bad",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:formulas:Operator",
                    ))]),
                ),
                (wk::SHORT_NAME, Value::String("bad".into())),
                ("urn:eigenius:formulas:operator_arity", arity),
            ],
        );
        top.add_resource(op).unwrap();
        let holder = make_resource(
            "urn:eigenius:test:op_ref_holder",
            vec![(
                "urn:eigenius:test:formula_value",
                Value::Json(app_of("urn:eigenius:test:ops:bad")),
            )],
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
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:formulas:Operator",
                    ))]),
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

    #[test]
    fn op_ref_with_non_string_operand_is_caught_by_rule_16() {
        // Rule 17 stays quiet on a malformed `OpRef` operand because
        // Rule 16 owns the ctor arg shape. Verify the defect is
        // diagnosed there rather than falling through both rules.
        let errors = validate_formula_term(serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "OpRef", "args": [42]},
                {"ctor": "Var", "args": ["x"]}
            ]
        }));
        assert!(
            errors
                .iter()
                .any(|e| e.rule == ValidationRule::InductiveValueMismatch),
            "a non-string OpRef operand must be diagnosed by Rule 16; got {errors:?}"
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
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("eigentt_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:eigentt:Term"))]),
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
