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

//! Ground type resolution: bridge between Eigon ontology and EigenTT types.
//!
//! Resolves class IRIs from the layer chain into EigenTT Sigma types.
//! Required properties map to direct Sigma components.
//! Recommended properties map to Option (Sum(some T | none 1)) components.
//! Constraints (allows_only, class_types) map to Sum types.

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt, PrimitiveType};
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Resolve a class IRI to a EigenTT type.
///
/// The resulting type is a nested Sigma:
/// - Required properties: Σ name : T. ...
/// - Recommended properties: Σ name : Option(T). ...
pub fn resolve_class_type(class_iri: &Iri, layer: &Layer) -> Result<Val, String> {
    // Check for primitive types first
    match class_iri.as_str() {
        wk::STRING => return Ok(Val::EigonPrimitive(PrimitiveType::String)),
        wk::INTEGER => return Ok(Val::EigonPrimitive(PrimitiveType::Integer)),
        wk::FLOAT => return Ok(Val::EigonPrimitive(PrimitiveType::Float)),
        wk::BOOLEAN => return Ok(Val::EigonPrimitive(PrimitiveType::Boolean)),
        wk::JSON => return Ok(Val::EigonPrimitive(PrimitiveType::Json)),
        _ => {}
    }

    let resource_arc = layer
        .resolve(class_iri)
        .ok_or_else(|| format!("class '{}' not found in layer chain", class_iri))?;
    let resource: &crate::ontology::resource::Resource = &resource_arc;

    // Inductive types resolve to Val::InductiveType with the full
    // Arc<InductiveDecl> built from the resource's params + ctors
    // embedded shape (Phase 11b step 9, D19 §10). The value returned
    // is the unapplied type former — `Val::InductiveType { decl,
    // params: vec![] }`. Parameter application is the job of Step 10
    // (constructor application resolution).
    if is_inductive_type(resource) {
        return resolve_inductive_type(class_iri, resource, layer);
    }

    // D78 Phase C — the class's constraint is a RECORD over its `requires`.
    //
    // Three deletions from the Σ-chain this replaces:
    //
    // - **`recommends` no longer contributes a field.** It was Option-wrapped
    //   into the chain, which says the record *has* a field holding `some x` or
    //   `none`; a resource that omits the property has no field at all, and only
    //   absence is what `recommends` describes (D78 §1.1). `Option` survives
    //   where it belongs — the Julia and Lean mirror generators emit closed
    //   target-language structs, which do need a nullable slot.
    // - **No `Val::One` short-circuit for an empty class.** All 749 shipped
    //   classes with no `requires` resolved to the *same* `Val::One` and were
    //   definitionally equal to one another; an empty record is per-class
    //   (D78 §1.2, §5.1).
    // - **No right-nested Σ.** `build_sigma_chain` bound `local_name()`, so two
    //   properties sharing a local name across namespaces were one field to a
    //   projection. A record is keyed by full IRI (D78 §9).
    let (required, _recommended) = collect_properties(class_iri, layer)?;

    let mut fields: Vec<(Iri, Patt, Exp)> = Vec::new();
    for prop_iri in &required {
        let prop_type = resolve_property_type(prop_iri, layer)?;
        // The field's type is closed — a class field's type is a function of the
        // *property*, never of an earlier field (D78 §4.1) — so reading it back
        // into an `Exp` loses nothing, and the telescope carries no dependency
        // until conditional requirements land (D78 §1.3).
        let ty = crate::nbe::readback::readback_val(0, &prop_type);
        fields.push((
            prop_iri.clone(),
            Patt::Var(prop_iri.local_name().to_string()),
            ty,
        ));
    }

    let record_exp = Exp::record(fields).map_err(|e| e.to_string())?;
    crate::nbe::eval::eval(&record_exp, &crate::nbe::env::Rho::Nil)
        .map_err(|e| format!("could not evaluate class record for '{class_iri}': {e:?}"))
}

/// D78 §4 — the field set a constraint demands.
///
/// **`requires` only.** `recommends` contributes nothing at the type level
/// (D78 §1.1): it names properties that may be absent, and *if present, well
/// typed* is what Rule 3 already checks for any property regardless of class.
/// A recommended property is not a field of the constraint.
///
/// Transitive: `collect_properties` walks `subclass_of`, so a subclass's field
/// set includes its ancestors'.
pub fn constraint_fields(class_iri: &Iri, layer: &Layer) -> Result<BTreeSet<Iri>, String> {
    Ok(collect_properties(class_iri, layer)?.0)
}

/// D78 §4 — does `sub` entail `sup`? That is: does every record satisfying
/// `sub` satisfy `sup`?
///
/// `fields(sup) ⊆ fields(sub)`, and **nothing else**. The rule as first drafted
/// carried a per-field variance clause, `type_sub(ℓ) <: type_sup(ℓ)`; it is
/// **vacuous** and deliberately absent (D78 §4.1). A field's type is a function
/// of the *property* — `resolve_property_type` takes only a property IRI, and
/// `collect_properties_inner` collects IRIs without types — so there is no
/// per-`(class, property)` type for two constraints to disagree about. Adding
/// the clause back would be adding a check that cannot fail.
pub fn entails(sub: &Iri, sup: &Iri, layer: &Layer) -> Result<bool, String> {
    let needed = constraint_fields(sup, layer)?;
    let have = constraint_fields(sub, layer)?;
    Ok(needed.is_subset(&have))
}

/// D78 §3 — does the **conjunction** of `constraints` entail `sup`?
///
/// This is the judgment that earns its place. Over `subclass_of` declarations
/// entailment is automatic — `collect_properties` walks the relation, so a
/// declared subclass includes its parent's fields by construction, which is why
/// D78 ships no validation rule for it. The real use is `Refine` subtyping,
/// where the two constraint sets come from `is_a` lists and are **not
/// necessarily related by `subclass_of`**: whether the union of one set's fields
/// covers another's has no structural guarantee behind it.
///
/// A constraint is a field set, so `fields(⋀S) = ⋃_{C∈S} fields(C)` and §4's
/// rule applies to that union unchanged.
pub fn conjunction_entails(
    constraints: &BTreeSet<Iri>,
    sup: &Iri,
    layer: &Layer,
) -> Result<bool, String> {
    let needed = constraint_fields(sup, layer)?;
    let mut have: BTreeSet<Iri> = BTreeSet::new();
    for c in constraints {
        have.extend(constraint_fields(c, layer)?);
    }
    Ok(needed.is_subset(&have))
}

/// Collect required and recommended properties for a class (including inherited).
fn collect_properties(
    class_iri: &Iri,
    layer: &Layer,
) -> Result<(BTreeSet<Iri>, BTreeSet<Iri>), String> {
    let mut required = BTreeSet::new();
    let mut recommended = BTreeSet::new();
    let mut visited = BTreeSet::new();
    collect_properties_inner(
        class_iri,
        layer,
        &mut required,
        &mut recommended,
        &mut visited,
    )?;
    Ok((required, recommended))
}

fn collect_properties_inner(
    class_iri: &Iri,
    layer: &Layer,
    required: &mut BTreeSet<Iri>,
    recommended: &mut BTreeSet<Iri>,
    visited: &mut BTreeSet<Iri>,
) -> Result<(), String> {
    if !visited.insert(class_iri.clone()) {
        return Ok(());
    }
    let resource = match layer.resolve(class_iri) {
        Some(r) => r,
        None => return Ok(()),
    };

    // Collect requires
    let requires_iri = Iri::parse(wk::REQUIRES).unwrap();
    if let Some(requires_val) = resource.get(&requires_iri) {
        for prop_iri in requires_val.as_iri_array() {
            required.insert(prop_iri);
        }
    }

    // Collect recommends
    let recommends_iri = Iri::parse(wk::RECOMMENDS).unwrap();
    if let Some(recommends_val) = resource.get(&recommends_iri) {
        for prop_iri in recommends_val.as_iri_array() {
            recommended.insert(prop_iri);
        }
    }

    // Walk parent classes
    let subclass_iri = Iri::parse(wk::PARENT_CLASSES).unwrap();
    if let Some(parents_val) = resource.get(&subclass_iri) {
        for parent_iri in parents_val.as_iri_array() {
            collect_properties_inner(&parent_iri, layer, required, recommended, visited)?;
        }
    }

    Ok(())
}

/// Resolve a property's data_type to a EigenTT Val.
///
/// Handles all data types including resource references (with class_types
/// and allows_only), arrays, and primitive types.
pub fn resolve_property_type(prop_iri: &Iri, layer: &Layer) -> Result<Val, String> {
    let resource_arc = layer
        .resolve(prop_iri)
        .ok_or_else(|| format!("property '{}' not found", prop_iri))?;
    let resource: &crate::ontology::resource::Resource = &resource_arc;

    let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
    // `data_type` is a `data_type: resource` property — canonical
    // shape is `ResourceRef`, but `as_iri` also accepts the
    // pre-canonical `String` shape from intermediate resources.
    let data_type_str = match resource.get(&dt_iri).and_then(|v| v.as_iri()) {
        Some(i) => i.as_str().to_string(),
        None => return Ok(Val::sort(1)), // Unknown data type
    };

    match data_type_str.as_str() {
        wk::STRING => Ok(Val::EigonPrimitive(PrimitiveType::String)),
        wk::INTEGER => Ok(Val::EigonPrimitive(PrimitiveType::Integer)),
        wk::FLOAT => Ok(Val::EigonPrimitive(PrimitiveType::Float)),
        wk::BOOLEAN => Ok(Val::EigonPrimitive(PrimitiveType::Boolean)),
        wk::JSON => Ok(Val::EigonPrimitive(PrimitiveType::Json)),

        wk::RESOURCE => {
            // Check for allows_only first (enum type)
            let ao_iri = Iri::parse(wk::ALLOWS_ONLY).unwrap();
            if let Some(ao_val) = resource.get(&ao_iri) {
                let allowed_iris = ao_val.as_iri_array();
                if !allowed_iris.is_empty() {
                    return Ok(make_enum_type(&allowed_iris));
                }
            }

            // Check for class_types (union or single class)
            let ct_iri = Iri::parse(wk::CLASS_TYPES).unwrap();
            if let Some(ct_val) = resource.get(&ct_iri) {
                let class_iris = ct_val.as_iri_array();
                if class_iris.len() == 1 {
                    return Ok(Val::EigonClass(class_iris[0].clone()));
                }
                if class_iris.len() > 1 {
                    return Ok(make_union_type(&class_iris));
                }
            }

            Ok(Val::sort(1)) // Untyped resource reference
        }

        wk::RESOURCE_ARRAY => {
            // Array of resources — wrap element type in a list type
            let inner = resolve_array_element_type(resource, layer)?;
            Ok(make_list_type(inner))
        }

        wk::VALUE_ARRAY => {
            // Array of values — wrap element type in a list type.
            // `element_type` is `data_type: resource`, post-canonical
            // shape is `ResourceRef`.
            let et_iri = Iri::parse(wk::ELEMENT_TYPE).unwrap();
            let elem_type = if let Some(et_iri_val) = resource.get(&et_iri).and_then(|v| v.as_iri())
            {
                match et_iri_val.as_str() {
                    wk::STRING => Val::EigonPrimitive(PrimitiveType::String),
                    wk::INTEGER => Val::EigonPrimitive(PrimitiveType::Integer),
                    wk::FLOAT => Val::EigonPrimitive(PrimitiveType::Float),
                    wk::BOOLEAN => Val::EigonPrimitive(PrimitiveType::Boolean),
                    _ => Val::sort(1),
                }
            } else {
                Val::sort(1)
            };
            Ok(make_list_type(elem_type))
        }

        _ => Ok(Val::sort(1)), // Unknown data type
    }
}

/// Resolve the element type for a resource_array property.
fn resolve_array_element_type(
    resource: &crate::ontology::resource::Resource,
    _layer: &Layer,
) -> Result<Val, String> {
    let ct_iri = Iri::parse(wk::CLASS_TYPES).unwrap();
    if let Some(ct_val) = resource.get(&ct_iri) {
        let class_iris = ct_val.as_iri_array();
        if let Some(first) = class_iris.first() {
            return Ok(Val::EigonClass(first.clone()));
        }
    }
    Ok(Val::sort(1))
}

/// Make a list type wrapping an element type.
///
/// Wraps the canonical `List(A)` inductive declaration from
/// [`crate::nbe::term::list_decl`] (Phase 11b step 6, D19 §9).
fn make_list_type(elem: Val) -> Val {
    Val::InductiveType {
        decl: crate::nbe::term::list_decl(),
        params: vec![elem],
        indices: Vec::new(),
    }
}

/// Make an enum type from allows_only IRIs: Sum(iri1 1 | iri2 1 | ...)
fn make_enum_type(iris: &[Iri]) -> Val {
    let summands: Vec<(String, Exp)> = iris
        .iter()
        .map(|iri| (iri.local_name().to_string(), Exp::One))
        .collect();
    Val::Data(summands, Rho::Nil)
}

/// Make a union type from multiple class_types: Sum(class1 T1 | class2 T2 | ...)
fn make_union_type(iris: &[Iri]) -> Val {
    let summands: Vec<(String, Exp)> = iris
        .iter()
        .map(|iri| (iri.local_name().to_string(), Exp::EigonClass(iri.clone())))
        .collect();
    Val::Data(summands, Rho::Nil)
}

/// Check whether a resource represents an inductive type declaration
/// (Phase 11b step 9).
pub(crate) fn is_inductive_type(resource: &crate::ontology::resource::Resource) -> bool {
    resource
        .is_a()
        .iter()
        .any(|c| c.as_str() == wk::INDUCTIVE_TYPE)
}

/// Resolve an `InductiveType` resource into `Val::InductiveType`.
///
/// Builds an `Arc<InductiveDecl>` from the resource's embedded params
/// and ctors, reconstructing each constructor's full Π-telescope
/// type (`Π param₁ … Π param_n. Π arg₁ … Π arg_m. Self(params)`) from
/// the compact AST shape that the ESL compiler emitted.
///
/// Returns the unapplied type former — `Val::InductiveType { decl,
/// params: vec![] }`. For parameterised inductives, Phase 11b step 10+
/// will add the pathway that applies parameters at use sites.
pub(crate) fn resolve_inductive_type(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    layer: &Layer,
) -> Result<Val, String> {
    let short_name = match resource.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(format!("inductive type '{class_iri}' missing 'short_name'")),
    };

    let params_telescope = decode_params(class_iri, resource, layer)?;
    let indices_telescope = decode_indices(class_iri, resource, layer)?;
    let sort = decode_result_sort(class_iri, resource)?;

    // Build the self-reference stub used inside constructor types.
    // Empty `ctors` is fine — name-based lookup is all the kernel
    // needs for inner self-refs (see Phase 11b step 2 notes).
    //
    // Stub-Arc preservation (eigenius#72 Layer 2 / D48): the stub
    // carries the real `indices` telescope so that ctor-internal
    // self-references like `Vec(A, n)` decode against the same shape
    // the kernel's check pass expects. `params` stays empty in the
    // stub since references inside ctor bodies thread params lexically.
    let self_ref = Arc::new(InductiveDecl {
        iri: class_iri.clone(),
        name: short_name.clone(),
        params: Vec::new(),
        indices: indices_telescope.clone(),
        sort: sort.clone(),
        ctors: Vec::new(),
    });

    let ctors = decode_ctors(class_iri, resource, &self_ref, &params_telescope, layer)?;

    let decl = Arc::new(InductiveDecl {
        iri: class_iri.clone(),
        name: short_name,
        params: params_telescope,
        indices: indices_telescope,
        sort,
        ctors,
    });
    Ok(Val::InductiveType {
        decl,
        params: Vec::new(),
        indices: Vec::new(),
    })
}

/// Decode the optional `core:indices` array on an inductive-type
/// resource (eigenius#72 Layer 2). Same shape as `core:type_params`.
/// Returns an empty vector when absent — matching the pre-Layer-2
/// non-indexed default.
fn decode_indices(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    layer: &Layer,
) -> Result<Vec<(Patt, Exp)>, String> {
    let indices_iri = Iri::parse(wk::INDICES).unwrap();
    let arr = match resource.get(&indices_iri) {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(format!(
                "inductive type '{class_iri}' has non-array `indices`"
            ));
        }
        None => return Ok(Vec::new()),
    };
    let mut indices = Vec::new();
    for entry in arr {
        let pr = match entry {
            Value::Embedded(r) => r.as_ref(),
            _ => {
                return Err(format!(
                    "inductive type '{class_iri}' `indices` must be embedded InductiveParam resources"
                ));
            }
        };
        let name = match pr.get(&Iri::parse(wk::PARAM_NAME).unwrap()) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "inductive type '{class_iri}' index missing `param_name`"
                ));
            }
        };
        // An index kind is a `eigentt:TypeExpr`, decoded exactly like a parameter kind — the two
        // telescopes are the same shape and the ESL compiler emits them through the same code.
        //
        // This read used to be `Some(Value::String(s)) => s, _ => "urn:eigenius:core:Set"`, and the
        // default was not a forward-compat courtesy: `urn:eigenius:core:Set` is not a declared
        // resource, so it decoded to `EigonClass(core:Set)` — a class type nothing can inhabit.
        // Every index kind that failed to be a string got it silently, and nothing noticed because
        // nothing type-checked the telescope. `check_type`'s `Exp::Inductive` arm now does
        // (`check_inductive_decl_telescopes`), and the first thing it reported was
        // `reasoning:JustifiedBy.declared` failing `EigonPrimitive(String) ≠ EigonClass(core:Set)`.
        let Some(kind_value) = pr.get(&Iri::parse(wk::PARAM_KIND).unwrap()) else {
            return Err(format!(
                "inductive type '{class_iri}' index '{name}' missing `param_kind`"
            ));
        };
        let kind_exp = decode_param_kind(kind_value, class_iri, layer)?;
        // Anonymous-index encoding: the ESL parser uses "_" as the
        // sentinel name. Honour the encoding by emitting `Patt::Unit`.
        let patt = if name == "_" {
            Patt::Unit
        } else {
            Patt::Var(name)
        };
        indices.push((patt, kind_exp));
    }
    Ok(indices)
}

/// Decode the optional `core:result_sort` string on an inductive-type
/// resource (eigenius#72 Layer 2). Recognised forms: `"Prop"`,
/// `"Set"`, `"Type:N"`. Absent or unrecognised → `Sort(1)` (the
/// pre-Layer-2 default).
fn decode_result_sort(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
) -> Result<Exp, String> {
    // eigenius#188: `result_sort` is a `core:Level` value, not a string. The old grammar —
    // `"Prop"` / `"Set"` / `"Type:N"`, parsed by hand here — could not express a level VARIABLE,
    // so `data X : Sort u` was inexpressible and nothing validated the string's shape. Decoding
    // through the same codec as every other level means one representation and one validator.
    let sort_iri = Iri::parse(wk::RESULT_SORT).unwrap();
    match resource.get(&sort_iri) {
        Some(Value::Json(j)) => crate::program::eigentt_type_mirror::decode_level_json(j)
            .map(Exp::Sort)
            .map_err(|e| format!("inductive type '{class_iri}' has malformed `result_sort`: {e}")),
        Some(other) => Err(format!(
            "inductive type '{class_iri}' has a `result_sort` that is not a core:Level value: \
             {other:?}"
        )),
        // Absent defaults to `Set`, as it always has.
        None => Ok(Exp::sort(1)),
    }
}

fn decode_params(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    layer: &Layer,
) -> Result<Vec<(Patt, Exp)>, String> {
    let type_params_iri = Iri::parse(wk::TYPE_PARAMS).unwrap();
    let arr = match resource.get(&type_params_iri) {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(format!(
                "inductive type '{class_iri}' has non-array `type_params`"
            ))
        }
        None => return Ok(Vec::new()),
    };
    let mut params = Vec::new();
    for entry in arr {
        let pr = match entry {
            Value::Embedded(r) => r.as_ref(),
            _ => {
                return Err(format!(
                    "inductive type '{class_iri}' `type_params` must be embedded InductiveParam resources"
                ))
            }
        };
        let name = match pr.get(&Iri::parse(wk::PARAM_NAME).unwrap()) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "inductive type '{class_iri}' param missing `param_name`"
                ))
            }
        };
        // Absent `param_kind` still defaults to `Set`, as it always has. What is gone is the
        // silent default for a kind that was PRESENT and unrecognised (eigenius#188 / N4).
        let kind_exp = match pr.get(&Iri::parse(wk::PARAM_KIND).unwrap()) {
            Some(v) => decode_param_kind(v, class_iri, layer)?,
            None => Exp::sort(1),
        };
        params.push((Patt::Var(name), kind_exp));
    }
    Ok(params)
}

/// Whether `arg_iri` names a declared inductive in the chain, and if so a
/// name-only stub `InductiveDecl` for it.
///
/// **The one rule for a fully-qualified IRI appearing in TYPE position.**
/// `InductiveDecl` equality is by IRI, so a stub carrying just the IRI and
/// short name is enough for the type checker's name-based dispatch; we
/// deliberately do NOT recurse into `resolve_inductive_type` for the target,
/// which would loop on mutually-referential declarations.
///
/// Two decoders reach for this — `decode_arg_type` (constructor argument
/// types) and `decode_param_kind` (parameter AND index telescopes, which are
/// one path since eigenius#188). Three of them used to disagree:
/// only the first consulted the chain, so an inductive named as a
/// constructor argument decoded to `Exp::InductiveType` while the *same*
/// inductive named as an index kind decoded to `Exp::EigonClass`. That
/// disagreement is eigenius#199 — it made `reasoning:JustifiedBy`'s index
/// #0 (`JustificationTerm`) an `EigonClass` that no inhabitant could check
/// against, so the one relation carrying the platform's guarantee was the
/// one whose type the surface language could not express.
fn inductive_stub_for(arg_iri: &Iri, layer: &Layer) -> Option<Arc<InductiveDecl>> {
    let resource_arc = layer.resolve(arg_iri)?;
    let resource: &crate::ontology::resource::Resource = &resource_arc;
    if !is_inductive_type(resource) {
        return None;
    }
    let name = match resource.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
        Some(Value::String(s)) => s.clone(),
        _ => arg_iri.local_name().to_string(),
    };
    Some(Arc::new(InductiveDecl {
        iri: arg_iri.clone(),
        name,
        params: Vec::new(),
        indices: Vec::new(),
        sort: Exp::sort(1),
        ctors: Vec::new(),
    }))
}

/// Decode an `InductiveParam`'s kind from its `core:param_kind` value (eigenius#188 / N4).
///
/// This was `decode_param_kind_str`, a six-way dispatch over a STRING — `Size`, `Prop`, `Set`,
/// `Type:N`, the primitive IRIs, an inductive IRI — **with a silent fallthrough to `Sort(1)`**.
/// That default typed a class-typed parameter `Set`, which accepts anything: the parameter looked
/// checked and was not. The string could not carry a level variable either, so `data Vec (A : Sort u)`
/// was inexpressible.
///
/// The kind is a type expression and now says so. Every case the string encoded is a `TypeExpr`
/// constructor, and `decode_type` already dispatches all of them — including the `ConstRef`
/// resolution that distinguishes a primitive from an inductive from a class.
fn decode_param_kind(value: &Value, class_iri: &Iri, layer: &Layer) -> Result<Exp, String> {
    crate::program::eigentt_type_mirror::decode_type(value, layer)
        .map_err(|e| format!("inductive type '{class_iri}' has a malformed `core:param_kind`: {e}"))
}

fn decode_ctors(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    self_ref: &Arc<InductiveDecl>,
    params: &[(Patt, Exp)],
    layer: &Layer,
) -> Result<Vec<InductiveCtorDecl>, String> {
    let ctors_iri = Iri::parse(wk::CTORS).unwrap();
    let arr = match resource.get(&ctors_iri) {
        Some(Value::Array(a)) => a,
        _ => {
            return Err(format!(
                "inductive type '{class_iri}' missing or non-array `ctors`"
            ))
        }
    };
    let mut out = Vec::new();
    for entry in arr {
        let cr = match entry {
            Value::Embedded(r) => r.as_ref(),
            _ => {
                return Err(format!(
                    "inductive type '{class_iri}' ctors must be embedded InductiveCtor resources"
                ))
            }
        };
        let name = match cr.get(&Iri::parse(wk::CTOR_NAME).unwrap()) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "inductive type '{class_iri}' ctor missing `ctor_name`"
                ))
            }
        };
        // eigenius#72 Layer 2 — if the ctor carries a `core:ctor_type`
        // payload (D47-encoded full Π-telescope), decode it directly
        // and skip the legacy positional path. The decoded Exp already
        // includes the params + indices + conclusion shape; the kernel
        // type checker takes it from there.
        let ctor_typ_iri = Iri::parse(wk::CTOR_TYPE).unwrap();
        let ctor_typ = if let Some(ct) = cr.get(&ctor_typ_iri) {
            // Self-reference threading: the codec needs to know it's
            // decoding a ctor for the in-construction `class_iri` so
            // that ConstRef / CtorApp targets matching `class_iri`
            // short-circuit to the stub `self_ref` instead of
            // recursively re-entering `resolve_inductive_type`. Without
            // this, any ctor body that mentions its own decl
            // (e.g. `cons : ... -> Vec(A, n)`) loops unboundedly.
            crate::program::eigentt_type_mirror::decode_type_with_self_ref(
                ct,
                layer,
                Some((class_iri, self_ref)),
            )
            .map_err(|e| {
                format!("inductive type '{class_iri}.{name}' has malformed `ctor_type`: {e:?}")
            })?
        } else {
            let arg_types_arr = match cr.get(&Iri::parse(wk::ARG_TYPES).unwrap()) {
                Some(Value::Array(a)) => a.as_slice(),
                None => &[],
                Some(_) => {
                    return Err(format!(
                        "inductive type '{class_iri}.{name}' has non-array `arg_types`"
                    ));
                }
            };
            build_ctor_type(class_iri, self_ref, params, arg_types_arr, layer)?
        };
        out.push(InductiveCtorDecl {
            name,
            typ: ctor_typ,
        });
    }
    Ok(out)
}

/// Assemble a constructor's full type expression:
/// `Π params. [Π|SizedPi] args. Self(params)`.
///
/// Each ctor arg is either a positional anonymous Pi, a named Pi
/// binder (for size-polymorphic args without a bound), or a
/// `SizedPi` (for named `Size` binders with an upper bound — the
/// sized-termination entry point from the ESL surface).
fn build_ctor_type(
    class_iri: &Iri,
    self_ref: &Arc<InductiveDecl>,
    params: &[(Patt, Exp)],
    arg_types: &[Value],
    layer: &Layer,
) -> Result<Exp, String> {
    // Result type: Self(param₁, param₂, ...).
    let param_vars: Vec<Exp> = params
        .iter()
        .map(|(p, _)| match p {
            Patt::Var(n) => Exp::Var(n.clone()),
            _ => Exp::Unit,
        })
        .collect();
    let mut result = Exp::InductiveType(self_ref.clone(), param_vars);

    // Decode all args upfront — preserves their shape so the wrapping
    // pass below can dispatch on positional / Pi-binder / SizedPi.
    let decoded: Vec<DecodedArg> = arg_types
        .iter()
        .map(|a| decode_ctor_arg(class_iri, self_ref, a, layer))
        .collect::<Result<Vec<_>, String>>()?;

    // Wrap in reverse so the first arg is outermost.
    for arg in decoded.into_iter().rev() {
        result = match arg {
            DecodedArg::Positional(typ) => Exp::Pi(Patt::Unit, Box::new(typ), Box::new(result)),
            DecodedArg::PiBinder { name, kind } => {
                Exp::Pi(Patt::Var(name), Box::new(kind), Box::new(result))
            }
        };
    }

    // Wrap each parameter binder in reverse.
    for (patt, kind) in params.iter().rev() {
        result = Exp::Pi(patt.clone(), Box::new(kind.clone()), Box::new(result));
    }

    Ok(result)
}

/// One of three shapes a ctor arg can take after decoding.
enum DecodedArg {
    /// Anonymous arg — the bare positional form.
    Positional(Exp),
    /// Named Pi binder (e.g. a size-polymorphic ctor without a bound).
    PiBinder { name: String, kind: Exp },
}

/// Decode a constructor-arg resource into a `DecodedArg`.
///
/// Binder-shaped resources carry `binder_name`; everything else is
/// positional. A binder whose kind is `Size` and that
/// additionally carries `binder_bound` emits `SizedBinder`;
/// otherwise it emits `PiBinder` (used for size-polymorphic args
/// without a bound).
fn decode_ctor_arg(
    class_iri: &Iri,
    self_ref: &Arc<InductiveDecl>,
    value: &Value,
    layer: &Layer,
) -> Result<DecodedArg, String> {
    let r = match value {
        Value::Embedded(r) => r.as_ref(),
        _ => return Err("InductiveArgType must be embedded".to_string()),
    };
    let binder_name = r
        .get(&Iri::parse(wk::BINDER_NAME).unwrap())
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if let Some(name) = binder_name {
        // Kind is stored in `type_name`; decode in the same way as
        // a normal arg type so `Size`/`Inf`/param-refs all work.
        let kind_exp = decode_arg_type(class_iri, self_ref, value, layer)?;
        Ok(DecodedArg::PiBinder {
            name,
            kind: kind_exp,
        })
    } else {
        Ok(DecodedArg::Positional(decode_arg_type(
            class_iri, self_ref, value, layer,
        )?))
    }
}

/// The head reference an `InductiveArgType`'s `core:type_name` names, as the bare dispatch key the
/// decoders key on: a parameter name, an IRI, or `"Size"` for the size sort.
///
/// eigenius#188 / N4 retyped `core:type_name` from a string to an `eigentt:TypeExpr`. The HEAD is
/// read out rather than the value decoded whole, because `core:type_args` is a SEPARATE property —
/// an applied type is `type_name` + `type_args`, not one `App` spine — and because a
/// self-reference must resolve to the in-construction declaration's stub rather than being looked
/// up in a layer that does not yet contain it.
///
/// Both consumers call this: [`decode_arg_type`] for constructor arguments and
/// `decode_codata_observation_type` for a parameterised reference in an observation type. The
/// second read `type_name` as a `Value::String` until this function existed, so it reported
/// "missing `type_name`" for every codata observation once the property stopped being a string.
pub fn arg_type_head(r: &crate::ontology::resource::Resource) -> Result<String, String> {
    let value = r
        .get(&Iri::parse(wk::TYPE_NAME).unwrap())
        .ok_or_else(|| "InductiveArgType missing `type_name`".to_string())?;
    let head = match value {
        Value::Json(j) => j,
        other => {
            return Err(format!(
                "InductiveArgType `type_name` must be an eigentt:TypeExpr value, got {other:?}"
            ))
        }
    };
    let ctor = head
        .get("ctor")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "InductiveArgType `type_name` has no ctor".to_string())?;
    let arg0 = || -> Option<&str> { head.get("args")?.as_array()?.first()?.as_str() };
    match ctor {
        "Var" => Ok(arg0()
            .ok_or_else(|| "`Var` type_name takes a name".to_string())?
            .to_string()),
        "SizeSort" => Ok("Size".to_string()),
        "ConstRef" => Ok(arg0()
            .ok_or_else(|| "`ConstRef` type_name takes an IRI".to_string())?
            .to_string()),
        other => Err(format!(
            "InductiveArgType `type_name` head `{other}` is not a type reference — expected \
             `Var`, `ConstRef` or `SizeSort`"
        )),
    }
}

/// Decode one `InductiveArgType` resource to its `Exp`.
///
/// Cases driven by the encoded `type_name`:
/// - Bare string (no namespace separator): a parameter reference,
///   emitted as `Exp::Var`.
/// - IRI equal to the enclosing inductive's IRI: a self-reference,
///   emitted as `Exp::InductiveType(self_ref, type_args...)`.
/// - IRI of another inductive type in the layer chain: emitted as
///   `Exp::InductiveType(stub_decl, type_args...)` where the stub
///   carries the matching short name. This makes cross-inductive
///   constructor arguments type-check correctly without resolving
///   the full target decl (which would risk infinite recursion for
///   mutually-referential inductives).
/// - Primitive IRI: emitted as `Exp::EigonPrimitive`.
/// - Any other class IRI: emitted as `Exp::EigonClass(iri)` to let
///   the type checker resolve it via the layer chain.
fn decode_arg_type(
    class_iri: &Iri,
    self_ref: &Arc<InductiveDecl>,
    value: &Value,
    layer: &Layer,
) -> Result<Exp, String> {
    let r = match value {
        Value::Embedded(r) => r.as_ref(),
        _ => return Err("InductiveArgType must be embedded".to_string()),
    };
    let owned_name = arg_type_head(r)?;
    let type_name: &str = &owned_name;
    let type_args_arr = match r.get(&Iri::parse(wk::TYPE_ARGS).unwrap()) {
        Some(Value::Array(a)) => a.as_slice(),
        None => &[],
        Some(_) => return Err("InductiveArgType `type_args` must be an array".to_string()),
    };

    // Heuristic distinguisher: bare parameter names carry no namespace
    // separator, every IRI produced by the ESL compiler contains `:`.
    // The compile step preserves this invariant, so the check is
    // exact rather than fuzzy.
    //
    // `Inf` and `Size` were reserved bare literals for the size sort until eigenius#218; every
    // bare name is now a parameter reference and nothing is special-cased.
    if !type_name.contains(':') {
        if !type_args_arr.is_empty() {
            return Err(format!(
                "bare parameter reference `{type_name}` cannot take type arguments"
            ));
        }
        return Ok(Exp::Var(type_name.to_string()));
    }

    let arg_iri =
        Iri::parse(type_name).map_err(|e| format!("invalid type_name IRI '{type_name}': {e}"))?;

    // Self-reference: the arg type is the inductive being built.
    if arg_iri == *class_iri {
        let sub_args: Result<Vec<Exp>, String> = type_args_arr
            .iter()
            .map(|a| decode_arg_type(class_iri, self_ref, a, layer))
            .collect();
        return Ok(Exp::InductiveType(self_ref.clone(), sub_args?));
    }

    // Primitive type IRIs get folded to the corresponding Exp form.
    match arg_iri.as_str() {
        wk::STRING => return Ok(Exp::EigonPrimitive(PrimitiveType::String)),
        wk::INTEGER => return Ok(Exp::EigonPrimitive(PrimitiveType::Integer)),
        wk::FLOAT => return Ok(Exp::EigonPrimitive(PrimitiveType::Float)),
        wk::BOOLEAN => return Ok(Exp::EigonPrimitive(PrimitiveType::Boolean)),
        wk::JSON => return Ok(Exp::EigonPrimitive(PrimitiveType::Json)),
        _ => {}
    }

    // Cross-inductive reference: the arg type is some other declared
    // inductive in the layer. Emit an `Exp::InductiveType` with a
    // name-only stub Arc so the type checker matches by name. We
    // deliberately do NOT recurse into `resolve_inductive_type` for
    // the target — the stub is enough for name-based dispatch and
    // avoids infinite recursion on mutually-referential decls (out of
    // scope but worth guarding against).
    if let Some(stub) = inductive_stub_for(&arg_iri, layer) {
        let sub_args: Result<Vec<Exp>, String> = type_args_arr
            .iter()
            .map(|a| decode_arg_type(class_iri, self_ref, a, layer))
            .collect();
        return Ok(Exp::InductiveType(stub, sub_args?));
    }

    // Any other class IRI: emit an EigonClass marker. The type
    // checker resolves this against the layer chain at use time.
    if !type_args_arr.is_empty() {
        return Err(format!(
            "parameterised references to non-inductive class `{type_name}` are not supported"
        ));
    }
    Ok(Exp::EigonClass(arg_iri))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use std::sync::Arc;

    pub(super) fn build_test_layer() -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }
        let core = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        Arc::new(domain_builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn resolve_primitive_string() {
        let layer = build_test_layer();
        let iri = Iri::parse(wk::STRING).unwrap();
        let typ = resolve_class_type(&iri, &layer).unwrap();
        assert!(matches!(typ, Val::EigonPrimitive(PrimitiveType::String)));
    }

    #[test]
    fn resolve_dog_class() {
        let layer = build_test_layer();
        let iri = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let typ = resolve_class_type(&iri, &layer).unwrap();
        // D78 Phase C — a record now, and keyed by full IRI. Dog requires two
        // properties: `name`, inherited from Animal, and its own `breed`.
        match typ {
            Val::Record(fields, _) => {
                let keys: Vec<&str> = fields.iter().map(|(i, _, _)| i.as_str()).collect();
                assert_eq!(
                    keys,
                    ["urn:eigenius:example:breed", "urn:eigenius:example:name"],
                    "canonical order is IRI order for independent fields"
                );
            }
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn resolve_nonexistent() {
        let layer = build_test_layer();
        let iri = Iri::parse("urn:eigenius:nonexistent:Foo").unwrap();
        assert!(resolve_class_type(&iri, &layer).is_err());
    }

    #[test]
    fn resolve_class_collects_recommends() {
        // Verify that collect_properties picks up recommends
        let layer = build_test_layer();
        let iri = Iri::parse(wk::CLASS).unwrap();
        let (required, recommended) = collect_properties(&iri, &layer).unwrap();
        // Class requires: is_a, description, short_name
        assert!(required.len() >= 3);
        // Class recommends: subclass_of, requires, recommends, etc.
        assert!(!recommended.is_empty());
    }

    #[test]
    fn resolve_property_with_allows_only() {
        // data_type property has allows_only constraint
        let layer = build_test_layer();
        let iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
        let typ = resolve_property_type(&iri, &layer).unwrap();
        // Should be a Sum type (enum)
        assert!(
            matches!(typ, Val::Data(ref summands, _) if !summands.is_empty()),
            "data_type should resolve to an enum type, got {:?}",
            typ
        );
    }

    // `option_type_has_two_constructors` was removed with `make_option_type` in
    // D78 Phase C. It asserted that a `recommends` property became an
    // `Option`-typed field of the class type; under clause 8 a recommended
    // property is not a field of the constraint at all (D78 §1.1), so there is
    // no Option to have two constructors. `Option` survives in the Julia and
    // Lean mirror generators, which emit closed structs and do need a nullable
    // slot — it is not the class type's business.

    #[test]
    fn readback_class_with_recommends_roundtrips() -> Result<(), Box<dyn std::error::Error>> {
        // The path that caused the __data_0 crash: resolve a class → readback →
        // re-evaluate, which is what parse_program does.
        //
        // D78 Phase C changed what this produces. `core:Class` recommends
        // several properties it does not require, and those used to become
        // `Option`-typed fields — the source of the crash. They are no longer
        // fields at all (§1.1), so the regression this guards can no longer
        // arise from `recommends`; the round-trip itself is still worth pinning.
        let layer = build_test_layer();
        let iri = Iri::parse(wk::CLASS).unwrap();
        let typ = resolve_class_type(&iri, &layer)?;

        let exp = crate::nbe::readback::readback_val(0, &typ);
        let val = crate::nbe::eval::eval(&exp, &Rho::Nil)?;

        let (before, after) = match (&typ, &val) {
            (Val::Record(a, _), Val::Record(b, _)) => (a.len(), b.len()),
            other => panic!("expected records, got {other:?}"),
        };
        assert_eq!(before, after, "the round-trip must preserve the field set");

        // And no recommended-only property is among them.
        let (required, recommended) = collect_properties(&iri, &layer)?;
        if let Val::Record(fields, _) = &val {
            for (f, _, _) in fields {
                assert!(required.contains(f), "unexpected field {f}");
                assert!(
                    !(recommended.contains(f) && !required.contains(f)),
                    "a recommended-only property must not be a field: {f}"
                );
            }
        }
        Ok(())
    }

    // --- Inductive type resolution (Phase 11b step 9) ---

    /// Build a test layer from core-ontology.json + an ESL source
    /// compiled in-line. Used to verify the round-trip
    /// ESL → JSON resources → layer → `resolve_inductive_type`.
    fn build_layer_with_esl(esl_source: &str) -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(crate::layer::LayerStorage::in_memory()));

        let user_resources = crate::esl::compile(esl_source).expect("ESL compile failed");
        let mut user_builder = LayerBuilder::new("user", Some(core));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        Arc::new(user_builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn resolve_nat_inductive_from_esl() {
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:Nat {
                zero,
                succ(ex:Nat),
            }
            "#,
        );
        let nat_iri = Iri::parse("urn:eigenius:example:Nat").unwrap();
        let val = resolve_class_type(&nat_iri, &layer).expect("resolve Nat");

        match val {
            Val::InductiveType {
                decl,
                params,
                indices: _,
            } => {
                assert!(params.is_empty());
                assert_eq!(decl.name, "Nat");
                assert!(decl.params.is_empty());
                assert_eq!(decl.ctors.len(), 2);
                assert_eq!(decl.ctors[0].name, "zero");
                assert_eq!(decl.ctors[1].name, "succ");

                // zero's type: InductiveType(Nat, [])
                match &decl.ctors[0].typ {
                    Exp::InductiveType(d, args) => {
                        assert_eq!(d.name, "Nat");
                        assert!(args.is_empty());
                    }
                    other => panic!("expected InductiveType for zero, got {other:?}"),
                }

                // succ's type: Π _:Nat. Nat
                match &decl.ctors[1].typ {
                    Exp::Pi(Patt::Unit, dom, body) => {
                        assert!(
                            matches!(dom.as_ref(), Exp::InductiveType(d, a) if d.name == "Nat" && a.is_empty())
                        );
                        assert!(
                            matches!(body.as_ref(), Exp::InductiveType(d, a) if d.name == "Nat" && a.is_empty())
                        );
                    }
                    other => panic!("expected Pi for succ, got {other:?}"),
                }
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn resolve_asserts_inductive_from_core_ontology() {
        // D39 §4.1 — `Asserts(iri) : Prop`. Authored directly in
        // ontologies/core/core-ontology.json: a uniform-parameter
        // 0-ctor inductive in Sort(0) whose single parameter `iri`
        // is typed at `core:string` (the kernel-side rep; the
        // iri-format constraint is a property-level concern, not a
        // type-theory concern). This test confirms the decoder picks
        // up the new declaration end-to-end:
        // - core ontology loads cleanly with the new resource,
        // - the new `decode_param_kind_str` arm maps `core:string` to
        //   `Exp::EigonPrimitive(PrimitiveType::String)`,
        // - `decode_result_sort` parses "Prop" → `Sort(0)`,
        // - zero ctors decode to an empty `decl.ctors` (the
        //   `large_elim_admitted` Case A path makes this admissible
        //   per D46 §7).
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let layer = Arc::new(core_builder.build(crate::layer::LayerStorage::in_memory()));

        let asserts_iri = Iri::parse("urn:eigenius:core:Asserts").unwrap();
        let val = resolve_class_type(&asserts_iri, &layer).expect("resolve Asserts");

        match val {
            Val::InductiveType {
                decl,
                params,
                indices,
            } => {
                assert!(
                    params.is_empty(),
                    "type former is unapplied; no params bound"
                );
                assert!(indices.is_empty(), "Asserts uses parameter, not index");
                assert_eq!(decl.name, "Asserts");
                assert_eq!(
                    decl.params.len(),
                    1,
                    "one parameter named iri (uniform across the type former)"
                );
                let (patt, kind) = &decl.params[0];
                assert!(
                    matches!(patt, Patt::Var(name) if name == "iri"),
                    "param name must be `iri`; got {:?}",
                    patt
                );
                assert!(
                    matches!(kind, Exp::EigonPrimitive(PrimitiveType::String)),
                    "param kind `core:string` must decode to EigonPrimitive(String); got {:?}",
                    kind
                );
                assert!(
                    matches!(&decl.sort, Exp::Sort(l) if l.is_nat(0)),
                    "result_sort `Prop` must decode to Sort(0); got {:?}",
                    decl.sort
                );
                assert!(
                    decl.ctors.is_empty(),
                    "Asserts has zero constructors — D39 §4.1; got {} ctors",
                    decl.ctors.len()
                );
            }
            other => panic!("expected Val::InductiveType for Asserts, got {other:?}"),
        }
    }

    #[test]
    fn resolve_bool_inductive_from_esl() {
        let layer = build_layer_with_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:Bool {
                tt,
                ff,
            }
            "#,
        );
        let bool_iri = Iri::parse("urn:eigenius:example:Bool").unwrap();
        let val = resolve_class_type(&bool_iri, &layer).expect("resolve Bool");
        match val {
            Val::InductiveType {
                decl,
                params,
                indices: _,
            } => {
                assert!(params.is_empty());
                assert_eq!(decl.name, "Bool");
                assert_eq!(decl.ctors.len(), 2);
                assert_eq!(decl.ctors[0].name, "tt");
                assert_eq!(decl.ctors[1].name, "ff");
                // Both ctor types are bare InductiveType — no Pi wrapping
                assert!(matches!(decl.ctors[0].typ, Exp::InductiveType(_, _)));
                assert!(matches!(decl.ctors[1].typ, Exp::InductiveType(_, _)));
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn universe_polymorphic_parameter_kind_survives_the_round_trip() {
        // The loose end eigenius#188 left, and the reason N4 retyped `core:param_kind`. Slice 5b
        // made `data X : Sort u` work by retyping `core:result_sort` to a `core:Level`; a
        // PARAMETER's kind stayed a string, whose grammar was `"Prop" | "Set" | "Type:N"` and could
        // not spell a level variable, so `data Vec (A : Sort u)` was rejected by the compiler.
        //
        // The assertion is on the decoded kernel term, not on the emitted JSON: the parameter's
        // kind must come back as `Sort(Param("u"))`. A `Sort(_)` match would pass against the old
        // silent `Sort(1)` default, so it pins the level itself.
        let layer = build_layer_with_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            universe u;

            data ex:Box(A : Sort u) {
                wrap(A),
            }
            "#,
        );
        let box_iri = Iri::parse("urn:eigenius:example:Box").unwrap();
        let val = resolve_class_type(&box_iri, &layer).expect("resolve Box");
        let Val::InductiveType { decl, .. } = val else {
            panic!("expected Val::InductiveType");
        };
        assert_eq!(decl.params.len(), 1);
        assert!(matches!(&decl.params[0].0, Patt::Var(n) if n == "A"));
        match &decl.params[0].1 {
            Exp::Sort(l) => assert_eq!(l, &crate::nbe::level::Level::Param("u".to_string())),
            other => panic!("expected a polymorphic Sort as the param kind, got {other:?}"),
        }
        // `wrap`'s type is `Π A : Sort u. Π _ : A. Box(A)` — the parameter binder carries the same
        // polymorphic sort, so the level reaches the constructor telescope too.
        match &decl.ctors[0].typ {
            Exp::Pi(Patt::Var(pn), dom, _) => {
                assert_eq!(pn, "A");
                assert!(
                    matches!(dom.as_ref(), Exp::Sort(l) if l == &crate::nbe::level::Level::Param("u".to_string()))
                );
            }
            other => panic!("expected Pi for wrap, got {other:?}"),
        }
    }

    #[test]
    fn resolve_list_parametric_inductive_from_esl() {
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:List(A : Set) {
                nil,
                cons(A, ex:List(A)),
            }
            "#,
        );
        let list_iri = Iri::parse("urn:eigenius:example:List").unwrap();
        let val = resolve_class_type(&list_iri, &layer).expect("resolve List");
        match val {
            Val::InductiveType {
                decl,
                params,
                indices: _,
            } => {
                assert!(params.is_empty());
                assert_eq!(decl.name, "List");
                assert_eq!(decl.params.len(), 1);
                assert!(matches!(&decl.params[0].0, Patt::Var(n) if n == "A"));

                // nil's type: Π A:Set. List(A)
                match &decl.ctors[0].typ {
                    Exp::Pi(Patt::Var(pn), dom, body) => {
                        assert_eq!(pn, "A");
                        assert!(matches!(&dom.as_ref(), Exp::Sort(l) if l.is_nat(1)));
                        match body.as_ref() {
                            Exp::InductiveType(d, args) => {
                                assert_eq!(d.name, "List");
                                assert_eq!(args.len(), 1);
                                assert!(matches!(&args[0], Exp::Var(n) if n == "A"));
                            }
                            other => panic!("expected InductiveType in nil body, got {other:?}"),
                        }
                    }
                    other => panic!("expected Pi for nil, got {other:?}"),
                }

                // cons's type: Π A:Set. Π _:A. Π _:List(A). List(A) — depth 3
                let mut depth = 0;
                let mut cursor = &decl.ctors[1].typ;
                while let Exp::Pi(_, _, body) = cursor {
                    depth += 1;
                    cursor = body;
                }
                assert_eq!(depth, 3, "cons should be a 3-binder Π-chain");
                assert!(matches!(cursor, Exp::InductiveType(d, _) if d.name == "List"));
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    // --- Sized types through ESL surface (Phase 11b step 15h) ---

    // --- Self-referential parameterised codata (Phase 11b step 15j, D19 §8) ---

    #[test]
    fn resolve_inductive_with_sort_literal_indices_roundtrips() {
        // D39 §5 / D49 ChainWitness: indices can be Sort literals
        // (Prop / Set / Type N) in addition to bare-name or class
        // references. Full ESL → JSON resources → layer →
        // resolve_class_type round-trip. The ctor body references the
        // inductive itself (`ex:SortIdx(p)`), which exercises the
        // codec's self-reference short-circuit — without it,
        // `decode_type` recurses into `resolve_inductive_type` for the
        // same IRI and overflows the stack.
        let layer = build_layer_with_esl(
            r#"
            namespace ex = "urn:eigenius:example";

            data ex:SortIdx : Prop -> Set {
                mk : forall (p : Prop) => ex:SortIdx(p),
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SortIdx").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SortIdx");
        match val {
            Val::InductiveType { decl, .. } => {
                assert!(
                    decl.params.is_empty(),
                    "expected zero params, got {:?}",
                    decl.params
                );
                assert_eq!(
                    decl.indices.len(),
                    1,
                    "expected one index, got {:?}",
                    decl.indices
                );
                match &decl.indices[0].1 {
                    Exp::Sort(l) if l.is_nat(0) => {}
                    other => panic!("index 0: expected Sort(0) for Prop, got {other:?}"),
                }
                match &decl.sort {
                    Exp::Sort(l) if l.is_nat(1) => {}
                    other => panic!("expected result Sort(1) for Set, got {other:?}"),
                }
                // The ctor body must decode against the stub Arc, not
                // re-trigger resolve_inductive_type for the same IRI.
                assert_eq!(decl.ctors.len(), 1);
                assert_eq!(decl.ctors[0].name, "mk");
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    fn probe_iri() -> Iri {
        Iri::parse("urn:eigenius:t:Probe").unwrap()
    }

    /// Build a `core:param_kind` value the way the compiler now does (eigenius#188 / N4).
    fn kind_val(j: serde_json::Value) -> Value {
        Value::Json(j)
    }

    fn const_ref(iri: &str) -> Value {
        kind_val(serde_json::json!({"ctor": "ConstRef", "args": [iri]}))
    }

    fn sort_kind(n: usize) -> Value {
        let mut lvl = serde_json::json!({"ctor": "Zero", "args": []});
        for _ in 0..n {
            lvl = serde_json::json!({"ctor": "Succ", "args": [lvl]});
        }
        kind_val(serde_json::json!({"ctor": "Sort", "args": [lvl]}))
    }

    #[test]
    fn decode_param_kind_str_maps_sort_literals() {
        // D39 §5 / D49 ChainWitness predicates need the kernel decoder
        // to recognise the Sort-literal kind strings the ESL compiler
        // emits for intermediate index positions ("Prop" / "Set" /
        // "Type:N"). Without this mapping, JustifiedBy and similar
        // sort-indexed predicates can't round-trip through the codec.
        let layer = build_test_layer();
        assert!(
            matches!(&decode_param_kind(&sort_kind(0), &probe_iri(), &layer).unwrap(), Exp::Sort(l) if l.is_nat(0))
        );
        assert!(
            matches!(&decode_param_kind(&sort_kind(1), &probe_iri(), &layer).unwrap(), Exp::Sort(l) if l.is_nat(1))
        );
        assert!(
            matches!(&decode_param_kind(&sort_kind(1), &probe_iri(), &layer).unwrap(), Exp::Sort(l) if l.is_nat(1))
        );
        assert!(
            matches!(&decode_param_kind(&sort_kind(3), &probe_iri(), &layer).unwrap(), Exp::Sort(l) if l.is_nat(3))
        );
        assert!(
            matches!(&decode_param_kind(&sort_kind(8), &probe_iri(), &layer).unwrap(), Exp::Sort(l) if l.is_nat(8))
        );
    }

    #[test]
    fn index_and_param_kinds_naming_an_inductive_decode_to_inductive_type() {
        // eigenius#199. `decode_arg_type` has always consulted the chain
        // and produced `Exp::InductiveType` for an inductive named in a
        // constructor argument. The index and parameter telescopes did
        // not: an index kind fell through to `EigonClass`, a param kind
        // all the way to `Sort(1)`. Since a value of that inductive
        // infers to `InductiveType`, the index form could never be
        // satisfied — `reasoning:JustifiedBy`'s type was unwritable.
        //
        // eigenius#188 / N4: index and parameter kinds are now decoded by ONE function, so the
        // two can no longer disagree by construction. The assertions below kept their pairing
        // anyway — they pin the #199 fact, and a future split would fail them.
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut builder = LayerBuilder::new("core", None);
        for r in eigon_json::parse_document(core_json).unwrap() {
            builder.add_resource(r).unwrap();
        }
        let core = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let src = r#"
            namespace core = "urn:eigenius:core";
            namespace t    = "urn:eigenius:t";
            data t:Term { Leaf(core:string), }
            class t:PlainClass { }
        "#;
        let mut b = LayerBuilder::new("t", Some(core));
        for r in crate::esl::compile(src).expect("test ESL compiles") {
            b.add_resource(r).unwrap();
        }
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        match decode_param_kind(&const_ref("urn:eigenius:t:Term"), &probe_iri(), &layer).unwrap() {
            Exp::InductiveType(decl, args) => {
                assert_eq!(decl.iri.as_str(), "urn:eigenius:t:Term");
                assert!(args.is_empty());
            }
            other => panic!("index kind naming an inductive decoded to {other:?}"),
        }
        match decode_param_kind(&const_ref("urn:eigenius:t:Term"), &probe_iri(), &layer).unwrap() {
            Exp::InductiveType(decl, _) => assert_eq!(decl.iri.as_str(), "urn:eigenius:t:Term"),
            other => panic!("param kind naming an inductive decoded to {other:?}"),
        }
        assert!(matches!(
            decode_param_kind(
                &const_ref("urn:eigenius:t:PlainClass"),
                &probe_iri(),
                &layer
            )
            .unwrap(),
            Exp::EigonClass(_)
        ));
        // eigenius#188 / N4 — a class-typed PARAMETER decodes to `EigonClass`, matching the index
        // path. This assertion read `Exp::Sort(l) if l.is_nat(1)` with the comment "A
        // non-inductive class keeps its previous decoding on both paths" — pinning the silent
        // fallthrough that typed such a parameter `Set`, which accepts anything. The two paths
        // agree now; they did not before.
        assert!(matches!(
            decode_param_kind(
                &const_ref("urn:eigenius:t:PlainClass"),
                &probe_iri(),
                &layer
            )
            .unwrap(),
            Exp::EigonClass(_)
        ));
    }
}

#[cfg(test)]
mod record_agrees_with_sigma_chain {
    //! D78 §7 Phase A gate: a class's record and its Σ-chain must carry the same
    //! `(field, type)` pairs.
    //!
    //! Not `eq_nf` equality — a record and a Σ-chain are different types and will
    //! never compare equal. The assertion is that the two carry the same content,
    //! which is what makes Phase C a substitution rather than a rewrite.

    use super::tests::build_test_layer;
    use super::*;
    use crate::nbe::term::{Exp, Patt};

    /// Flatten a class Σ-chain into `(binder name, field type)` pairs.
    ///
    /// Note what this can extract and what it cannot: `build_sigma_chain` binds
    /// `prop_iri.local_name()` (`:305`), so the **IRI is not recoverable** from
    /// the chain — the local-name collision D78 §9 records. And its own comment
    /// at `:299-301` states the rest type does not depend on the current binder,
    /// so the chain is a flat product wearing Σ clothing.
    fn sigma_fields(v: &Val) -> Vec<(String, Val)> {
        let mut out = Vec::new();
        let mut cur = v.clone();
        loop {
            match cur {
                Val::Sig(t, g) => {
                    let name = match &g.patt {
                        Patt::Var(n) => n.clone(),
                        other => panic!("unexpected Σ binder {other:?}"),
                    };
                    out.push((name, *t));
                    match crate::nbe::eval::eval(&g.body, &g.env) {
                        Ok(rest) => cur = rest,
                        Err(e) => panic!("could not walk the chain: {e:?}"),
                    }
                }
                Val::One => break,
                other => panic!("chain ended in {other:?}, expected Val::One"),
            }
        }
        out
    }

    #[test]
    #[ignore = "D78 Phase C landed: resolve_class_type returns a record, so there is no Σ-chain \
                left to compare against. Kept as the record of the Phase A gate that licensed the \
                switch; `a_class_resolves_to_a_record_over_its_requires` is its successor."]
    fn a_class_record_carries_the_same_fields_as_its_sigma_chain() {
        let layer = build_test_layer();
        let dog = Iri::parse("urn:eigenius:example:Dog").unwrap();

        let chain = resolve_class_type(&dog, &layer).unwrap();
        let from_chain: Vec<String> = sigma_fields(&chain).into_iter().map(|(n, _)| n).collect();

        // Build the record the same collection would produce.
        let (required, _recommended) = collect_properties(&dog, &layer).unwrap();
        let fields: Vec<(Iri, Patt, Exp)> = required
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    Patt::Var(p.local_name().to_string()),
                    Exp::sort(1),
                )
            })
            .collect();
        let record = Exp::record(fields).unwrap();
        let from_record: Vec<String> = match &record {
            Exp::Record(fs) => fs
                .iter()
                .map(|(i, _, _)| i.local_name().to_string())
                .collect(),
            other => panic!("expected a record, got {other:?}"),
        };

        let mut a = from_chain.clone();
        let mut b = from_record.clone();
        a.sort();
        b.sort();
        assert_eq!(
            a, b,
            "record and Σ-chain must agree on the field set: chain={from_chain:?} record={from_record:?}"
        );
        assert!(
            !a.is_empty(),
            "Dog must have required fields, or this proves nothing"
        );
    }

    #[test]
    fn a_class_with_no_requires_is_now_an_empty_record_not_val_one() {
        // **Flipped by D78 Phase C, as written to.** 749 of 894 shipped classes
        // have no `requires` (§1.2). They used to resolve to the *same*
        // `Val::One` and were definitionally equal to one another; an empty
        // record is per-class.
        let layer = build_test_layer();
        let any = Iri::parse("urn:eigenius:core:Resource").unwrap();
        let resolved = resolve_class_type(&any, &layer).unwrap();
        match resolved {
            Val::Record(fields, _) => assert!(
                fields.is_empty(),
                "core:Resource requires nothing, so its record is empty; got {fields:?}"
            ),
            other => panic!("expected an empty record, not {other:?}"),
        }
    }

    #[test]
    fn a_class_resolves_to_a_record_over_its_requires() {
        // Successor to the Phase A alongside gate: the field set that gate
        // compared against the Σ-chain is now what `resolve_class_type` returns.
        let layer = build_test_layer();
        let dog = Iri::parse("urn:eigenius:example:Dog").unwrap();
        let resolved = resolve_class_type(&dog, &layer).unwrap();
        let (required, _) = collect_properties(&dog, &layer).unwrap();
        match resolved {
            Val::Record(fields, _) => {
                let keys: std::collections::BTreeSet<Iri> =
                    fields.iter().map(|(i, _, _)| i.clone()).collect();
                assert_eq!(keys, required, "the record is exactly the required set");
                assert!(!keys.is_empty(), "Dog must require something");
            }
            other => panic!("expected a record, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod entailment {
    //! D78 §4 / §4.1 — `C ⊨ D` is field-set inclusion, and its real use is the
    //! **non-`subclass_of`** case.

    use super::tests::build_test_layer;
    use super::*;
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;

    fn i(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// A class requiring exactly the listed properties, with no `subclass_of`.
    /// Unrelated by declaration is the point: entailment must decide on fields.
    fn cls(id: &str, requires: &[&str]) -> Resource {
        let mut r = Resource::new(i(id));
        r.set(
            i(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(i(wk::CLASS))]),
        );
        r.set(
            i(wk::SHORT_NAME),
            Value::String(id.rsplit(':').next().unwrap().into()),
        );
        r.set(i(wk::DESCRIPTION), Value::String("test class".into()));
        r.set(
            i(wk::REQUIRES),
            Value::Array(requires.iter().map(|p| Value::ResourceRef(i(p))).collect()),
        );
        r
    }

    /// Layer with four classes over the animals properties, none related by
    /// `subclass_of`.
    fn layer() -> Arc<Layer> {
        const NAME: &str = "urn:eigenius:example:name";
        const BREED: &str = "urn:eigenius:example:breed";
        let mut b = crate::layer::LayerBuilder::new("entailment", Some(build_test_layer()));
        b.add_resource(cls("urn:t:Both", &[NAME, BREED])).unwrap();
        b.add_resource(cls("urn:t:JustName", &[NAME])).unwrap();
        b.add_resource(cls("urn:t:JustBreed", &[BREED])).unwrap();
        b.add_resource(cls("urn:t:Nothing", &[])).unwrap();
        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn a_constraint_entails_itself() {
        let l = layer();
        assert!(entails(&i("urn:t:Both"), &i("urn:t:Both"), &l).unwrap());
    }

    #[test]
    fn more_fields_entails_fewer_without_any_subclass_declaration() {
        // The actual use (§4.1): `Both` and `JustName` are unrelated by
        // `subclass_of`, so nothing structural guarantees this — it is decided
        // on fields.
        let l = layer();
        assert!(entails(&i("urn:t:Both"), &i("urn:t:JustName"), &l).unwrap());
        assert!(
            !entails(&i("urn:t:JustName"), &i("urn:t:Both"), &l).unwrap(),
            "fewer fields must not entail more"
        );
    }

    #[test]
    fn disjoint_constraints_do_not_entail_each_other() {
        let l = layer();
        assert!(!entails(&i("urn:t:JustName"), &i("urn:t:JustBreed"), &l).unwrap());
        assert!(!entails(&i("urn:t:JustBreed"), &i("urn:t:JustName"), &l).unwrap());
    }

    #[test]
    fn everything_entails_the_empty_constraint() {
        // §4.1 — `Any` is the top of the entailment order automatically, with no
        // declared edge, because `fields(Any) = ∅` is a subset of everything.
        let l = layer();
        for c in ["urn:t:Both", "urn:t:JustName", "urn:t:Nothing"] {
            assert!(
                entails(&i(c), &i("urn:t:Nothing"), &l).unwrap(),
                "{c} must entail the empty constraint"
            );
        }
        assert!(
            entails(&i("urn:t:Both"), &i("urn:eigenius:core:Resource"), &l).unwrap(),
            "core:Resource is the shipped `Any` and requires nothing"
        );
    }

    #[test]
    fn a_conjunction_entails_what_no_member_does_alone() {
        // The case with no structural guarantee, and the reason the judgment
        // exists: neither `JustName` nor `JustBreed` covers `Both`, but together
        // they do.
        let l = layer();
        let both = i("urn:t:Both");
        assert!(!entails(&i("urn:t:JustName"), &both, &l).unwrap());
        assert!(!entails(&i("urn:t:JustBreed"), &both, &l).unwrap());

        let pair: BTreeSet<Iri> = [i("urn:t:JustName"), i("urn:t:JustBreed")]
            .into_iter()
            .collect();
        assert!(
            conjunction_entails(&pair, &both, &l).unwrap(),
            "fields(⋀S) is the union of the members' fields"
        );
    }

    #[test]
    fn a_declared_subclass_entails_its_parent_automatically() {
        // Why D78 ships no validation rule over `subclass_of` (§4.1):
        // `collect_properties` walks the relation, so the inclusion holds by
        // construction and a rule would always pass.
        let l = build_test_layer();
        let dog = i("urn:eigenius:example:Dog");
        let animal = i("urn:eigenius:example:Animal");
        assert!(
            constraint_fields(&animal, &l)
                .unwrap()
                .is_subset(&constraint_fields(&dog, &l).unwrap()),
            "Dog inherits Animal's requirements transitively"
        );
        assert!(entails(&dog, &animal, &l).unwrap());
    }

    #[test]
    fn recommends_does_not_enter_the_field_set() {
        // D78 §1.1 — a recommended property is not a field of the constraint.
        let l = build_test_layer();
        let class_iri = i(wk::CLASS);
        let fields = constraint_fields(&class_iri, &l).unwrap();
        let (required, recommended) = collect_properties(&class_iri, &l).unwrap();

        assert_eq!(
            fields, required,
            "constraint_fields is exactly the required set"
        );

        // `core:Class` recommends `subclass_of`, `requires`, `recommends` and
        // more, none of which it requires — so the recommended-only set is
        // non-empty and disjoint from the constraint's fields.
        let recommended_only: BTreeSet<&Iri> = recommended.difference(&required).collect();
        assert!(
            !recommended_only.is_empty(),
            "core:Class must recommend something it does not require, or this proves nothing"
        );
        for r in recommended_only {
            assert!(
                !fields.contains(r),
                "a recommended-only property must not be a constraint field: {r}"
            );
        }
    }
}
