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
use crate::nbe::term::{
    CodataDecl, Exp, InductiveCtorDecl, InductiveDecl, Observation, Patt, PrimitiveType,
};
use crate::nbe::val::{Clos, Val};
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

    // Codata types resolve to Val::Codata with each observation's
    // result type embedded as a syntactic Exp (evaluated in Rho::Nil
    // since observation types are fully resolved IRIs — no free
    // variables). See D11 §3.
    if is_codata_type(resource) {
        return resolve_codata_type(class_iri, resource, layer);
    }

    // Inductive types resolve to Val::InductiveType with the full
    // Arc<InductiveDecl> built from the resource's params + ctors
    // embedded shape (Phase 11b step 9, D19 §10). The value returned
    // is the unapplied type former — `Val::InductiveType { decl,
    // params: vec![] }`. Parameter application is the job of Step 10
    // (constructor application resolution).
    if is_inductive_type(resource) {
        return resolve_inductive_type(class_iri, resource, layer);
    }

    let (required, recommended) = collect_properties(class_iri, layer)?;

    let mut props: Vec<(Iri, Val)> = Vec::new();

    // Required properties — direct types
    for prop_iri in &required {
        let prop_type = resolve_property_type(prop_iri, layer)?;
        props.push((prop_iri.clone(), prop_type));
    }

    // Recommended properties — wrapped in Option (Sum(some T | none 1))
    for prop_iri in &recommended {
        if required.contains(prop_iri) {
            continue; // Already included as required
        }
        let prop_type = resolve_property_type(prop_iri, layer)?;
        let option_type = make_option_type(prop_type);
        props.push((prop_iri.clone(), option_type));
    }

    if props.is_empty() {
        return Ok(Val::One);
    }

    build_sigma_chain(&props)
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
        None => return Ok(Val::Sort(1)), // Unknown data type
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

            Ok(Val::Sort(1)) // Untyped resource reference
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
                    _ => Val::Sort(1),
                }
            } else {
                Val::Sort(1)
            };
            Ok(make_list_type(elem_type))
        }

        _ => Ok(Val::Sort(1)), // Unknown data type
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
    Ok(Val::Sort(1))
}

/// Make an Option type: Sum(some T | none 1)
fn make_option_type(inner: Val) -> Val {
    // Store the inner type as a value in the environment rather than
    // round-tripping through readback, which can introduce generated
    // variable names (e.g. __data_0) that fail to resolve in Rho::Nil.
    let var_name = "__option_inner".to_string();
    let rho = Rho::Nil.extend(Patt::Var(var_name.clone()), inner);
    Val::Data(
        vec![
            ("some".to_string(), Exp::Var(var_name)),
            ("none".to_string(), Exp::One),
        ],
        rho,
    )
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

/// Build a nested Sigma chain from a list of (property_iri, type) pairs.
fn build_sigma_chain(props: &[(Iri, Val)]) -> Result<Val, String> {
    if props.is_empty() {
        return Ok(Val::One);
    }
    let (prop_iri, prop_type) = &props[0];
    let rest_type = build_sigma_chain(&props[1..])?;
    // Store the rest type in the closure's environment rather than
    // round-tripping through readback. The rest type doesn't depend on
    // the current property's value, but we still need a well-formed closure.
    let rest_var = "__sigma_rest".to_string();
    let rho = Rho::Nil.extend(Patt::Var(rest_var.clone()), rest_type);
    let closure = Clos::new(
        Patt::Var(prop_iri.local_name().to_string()),
        Exp::Var(rest_var),
        rho,
    );
    Ok(Val::Sig(Box::new(prop_type.clone()), closure))
}

/// Check whether a resource represents a codata type declaration.
fn is_codata_type(resource: &crate::ontology::resource::Resource) -> bool {
    let is_a = resource.is_a();
    is_a.iter().any(|c| c.as_str() == wk::CODATA_TYPE)
}

/// Resolve a CodataType resource into a `Val` form.
///
/// Non-parameterised codata resolves to `Val::Codata(observations,
/// Rho::Nil)` — the anonymous structural form. Parameterised codata
/// resolves to `Val::CodataType { decl, params: vec![] }` — the
/// unapplied type former; applying it via `Exp::CodataType(decl,
/// args)` produces the concrete applied codata type.
///
/// Self-references inside observation types (e.g. `tail : Stream(A,
/// j)` on a `codata Stream(A, i)`) use the name-only stub pattern:
/// the decl's first `Arc` is built with empty observations, threaded
/// through the observation decoder as `self_ref`, and the full decl
/// is then reconstructed with the decoded observations. Name-based
/// `PartialEq` on `CodataDecl` unifies the stub with the full decl at
/// evaluation time.
fn resolve_codata_type(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    layer: &Layer,
) -> Result<Val, String> {
    let short_name = match resource.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
        Some(Value::String(s)) => s.clone(),
        _ => shortname_of(class_iri),
    };

    let observations_iri = Iri::parse("urn:eigenius:core:observations").unwrap();
    let obs_array = match resource.get(&observations_iri) {
        Some(Value::Array(arr)) => arr,
        _ => {
            return Err(format!(
                "codata type '{class_iri}' missing 'observations' array"
            ))
        }
    };

    // Type parameter telescope (empty for non-parameterised codata).
    let params_telescope = decode_params(class_iri, resource)?;

    // Self-reference stub — mirrors `resolve_inductive_type`'s use of
    // a name-only `InductiveDecl`.
    let self_ref: Arc<CodataDecl> = Arc::new(CodataDecl {
        iri: class_iri.clone(),
        name: short_name.clone(),
        params: params_telescope.clone(),
        sort: Exp::Sort(1),
        observations: Vec::new(),
    });

    let mut observations: Vec<Observation> = Vec::new();
    for entry in obs_array {
        let obs_res = match entry {
            Value::Embedded(r) => r.as_ref(),
            _ => {
                return Err(format!(
                    "codata type '{class_iri}' observations must be embedded Observation resources"
                ))
            }
        };
        let name_iri = Iri::parse("urn:eigenius:core:observation_name").unwrap();
        let type_iri = Iri::parse("urn:eigenius:core:observation_type").unwrap();
        let name = match obs_res.get(&name_iri) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "codata type '{class_iri}' observation missing 'observation_name'"
                ))
            }
        };
        let type_value = obs_res.get(&type_iri).ok_or_else(|| {
            format!("codata type '{class_iri}' observation '{name}' missing 'observation_type'")
        })?;
        let type_exp = decode_codata_observation_type(class_iri, &self_ref, layer, type_value)?;
        observations.push(Observation {
            name,
            typ: type_exp,
        });
    }

    // Non-parameterised codata: produce the legacy `Val::Codata` shape
    // used throughout D11-era code. Observation types that are
    // self-references have been encoded as `Exp::CodataType(self_ref,
    // [])` which evaluates correctly under `Rho::Nil`.
    if params_telescope.is_empty() {
        return Ok(Val::Codata(
            observations.into_iter().map(|o| (o.name, o.typ)).collect(),
            Rho::Nil,
        ));
    }

    // Parameterised codata — return the unapplied type former. The
    // full decl carries the decoded observations; consumers apply it
    // via `Exp::CodataType(decl, args)`.
    let decl = Arc::new(CodataDecl {
        iri: class_iri.clone(),
        name: short_name,
        params: params_telescope,
        sort: Exp::Sort(1),
        observations,
    });
    Ok(Val::CodataType {
        decl,
        params: Vec::new(),
    })
}

/// Decode a codata observation's type value to an `Exp`.
///
/// Three shapes (Phase 11b step 15h.3):
/// - `Value::String`: legacy plain-IRI reference or bare `Inf`/`Size`
///   / parameter name. Self-references (class IRI equals the
///   enclosing codata's IRI) resolve to `Exp::EigonClass` so the
///   type checker can look them up lazily.
/// - Embedded `InductiveArgType`: parameterised type reference
///   (e.g. `ex:List(A)`). Reuses `decode_arg_type`.
/// - Embedded `TypeArrow`: non-dependent arrow.
/// - Embedded `TypeBinderArrow`: size-binder arrow, becomes
///   `Exp::SizedPi` when kind is `Size` and a bound is present.
fn decode_codata_observation_type(
    class_iri: &Iri,
    self_ref: &Arc<CodataDecl>,
    layer: &Layer,
    value: &Value,
) -> Result<Exp, String> {
    match value {
        // `ResourceRef` is the canonical post-`canonicalise_resource_refs`
        // shape for an IRI value; the bare-name forms (`Inf`, `Size`,
        // identifier `Var`) only ever come through `Value::String`
        // because they're not parseable as IRIs.
        Value::ResourceRef(parsed) => {
            if parsed == class_iri {
                Ok(Exp::CodataType(self_ref.clone(), Vec::new()))
            } else {
                let v = resolve_class_type(parsed, layer)?;
                Ok(crate::nbe::readback::readback_val(0, &v))
            }
        }
        Value::String(s) => {
            // Bare name forms first.
            if !s.contains(':') {
                return Ok(match s.as_str() {
                    "Inf" => Exp::SizeInf,
                    "Size" => Exp::SizeSort,
                    other => Exp::Var(other.to_string()),
                });
            }
            let parsed =
                Iri::parse(s).map_err(|e| format!("invalid observation type IRI '{s}': {e}"))?;
            // Self-reference — encode as a proper parameterised codata
            // application with no args. Name-based `PartialEq` on
            // `CodataDecl` unifies this stub with the full decl at
            // evaluation time.
            if parsed == *class_iri {
                Ok(Exp::CodataType(self_ref.clone(), Vec::new()))
            } else {
                let v = resolve_class_type(&parsed, layer)?;
                Ok(crate::nbe::readback::readback_val(0, &v))
            }
        }
        Value::Embedded(r) => {
            let is_a: Vec<String> = r.is_a().iter().map(|i| i.as_str().to_string()).collect();
            // Dispatch on the embedded resource's is_a marker.
            if is_a.iter().any(|s| s == wk::TYPE_ARROW) {
                let dom_v = r
                    .get(&Iri::parse(wk::ARROW_DOMAIN).unwrap())
                    .ok_or_else(|| "TypeArrow missing `arrow_domain`".to_string())?;
                let cod_v = r
                    .get(&Iri::parse(wk::ARROW_CODOMAIN).unwrap())
                    .ok_or_else(|| "TypeArrow missing `arrow_codomain`".to_string())?;
                let dom = decode_codata_observation_type(class_iri, self_ref, layer, dom_v)?;
                let cod = decode_codata_observation_type(class_iri, self_ref, layer, cod_v)?;
                return Ok(Exp::Pi(
                    crate::nbe::term::Patt::Unit,
                    Box::new(dom),
                    Box::new(cod),
                ));
            }
            if is_a.iter().any(|s| s == wk::TYPE_BINDER_ARROW) {
                let name = r
                    .get(&Iri::parse(wk::BINDER_NAME).unwrap())
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .ok_or_else(|| "TypeBinderArrow missing `binder_name`".to_string())?;
                let kind_str = r
                    .get(&Iri::parse(wk::BINDER_KIND).unwrap())
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "TypeBinderArrow missing `binder_kind`".to_string())?;
                let kind_is_size = kind_str == "Size" || kind_str.ends_with(":Size");
                let bound_opt = r
                    .get(&Iri::parse(wk::BINDER_BOUND).unwrap())
                    .and_then(|v| v.as_str());
                let body_v = r
                    .get(&Iri::parse(wk::BINDER_BODY).unwrap())
                    .ok_or_else(|| "TypeBinderArrow missing `binder_body`".to_string())?;
                let body = decode_codata_observation_type(class_iri, self_ref, layer, body_v)?;
                match (kind_is_size, bound_opt) {
                    (true, Some(bstr)) => Ok(Exp::SizedPi {
                        patt: crate::nbe::term::Patt::Var(name),
                        upper: Box::new(decode_bare_size_ref(bstr)),
                        body: Box::new(body),
                    }),
                    (true, None) => Ok(Exp::Pi(
                        crate::nbe::term::Patt::Var(name),
                        Box::new(Exp::SizeSort),
                        Box::new(body),
                    )),
                    (false, Some(_)) => Err(format!(
                        "TypeBinderArrow `{name}` has a bound but its kind is not Size"
                    )),
                    (false, None) => Ok(Exp::Pi(
                        crate::nbe::term::Patt::Var(name),
                        Box::new(Exp::EigonClass(
                            Iri::parse(kind_str)
                                .map_err(|e| format!("invalid binder kind '{kind_str}': {e}"))?,
                        )),
                        Box::new(body),
                    )),
                }
            } else if is_a.iter().any(|s| s == wk::INDUCTIVE_ARG_TYPE) {
                // Parameterised type reference (`ex:List(A)` or a
                // self-reference `ex:Stream(A, j)`). Intercept
                // self-references and emit `Exp::CodataType(self_ref,
                // args)`; otherwise delegate to `decode_arg_type`
                // with a never-matching dummy inductive stub (its
                // name can't collide with any real inductive, so
                // its self-ref branch never fires).
                let type_name = r
                    .get(&Iri::parse(wk::TYPE_NAME).unwrap())
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "InductiveArgType missing `type_name`".to_string())?;
                if type_name.contains(':') {
                    if let Ok(parsed) = Iri::parse(type_name) {
                        if parsed == *class_iri {
                            let type_args_arr = match r.get(&Iri::parse(wk::TYPE_ARGS).unwrap()) {
                                Some(Value::Array(a)) => a.clone(),
                                _ => Vec::new(),
                            };
                            let args: Result<Vec<Exp>, String> = type_args_arr
                                .iter()
                                .map(|a| {
                                    decode_codata_observation_type(class_iri, self_ref, layer, a)
                                })
                                .collect();
                            return Ok(Exp::CodataType(self_ref.clone(), args?));
                        }
                    }
                }
                let dummy = Arc::new(InductiveDecl {
                    iri: Iri::parse("urn:eigenius:_internal:__not_a_real_inductive__")
                        .expect("static sentinel IRI"),
                    name: "__not_a_real_inductive__".to_string(),
                    params: Vec::new(),
                    indices: Vec::new(),
                    sort: Exp::Sort(1),
                    ctors: Vec::new(),
                });
                decode_arg_type(class_iri, &dummy, value, layer)
            } else {
                Err(format!(
                    "unrecognised codata observation type resource: is_a {:?}",
                    is_a
                ))
            }
        }
        _ => Err("observation_type must be a String or an embedded TypeExpr resource".to_string()),
    }
}

fn shortname_of(iri: &Iri) -> String {
    iri.as_str()
        .rsplit(':')
        .next()
        .unwrap_or(iri.as_str())
        .to_string()
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

    let params_telescope = decode_params(class_iri, resource)?;
    let indices_telescope = decode_indices(class_iri, resource)?;
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
        let kind_str = match pr.get(&Iri::parse(wk::PARAM_KIND).unwrap()) {
            Some(Value::String(s)) => s.as_str(),
            _ => "urn:eigenius:core:Set",
        };
        let kind_exp = decode_index_kind_str(kind_str);
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
    let sort_iri = Iri::parse(wk::RESULT_SORT).unwrap();
    match resource.get(&sort_iri) {
        Some(Value::String(s)) => match s.as_str() {
            "Prop" => Ok(Exp::Sort(0)),
            "Set" => Ok(Exp::Sort(1)),
            other if other.starts_with("Type:") => {
                let n: usize = other["Type:".len()..].parse().map_err(|_| {
                    format!("inductive type '{class_iri}' has malformed `result_sort` '{other}'")
                })?;
                Ok(Exp::Sort(n + 1))
            }
            other => Err(format!(
                "inductive type '{class_iri}' has unrecognised `result_sort` '{other}' \
                 (expected `Prop`, `Set`, or `Type:N`)"
            )),
        },
        Some(_) => Err(format!(
            "inductive type '{class_iri}' has non-string `result_sort`"
        )),
        None => Ok(Exp::Sort(1)),
    }
}

fn decode_params(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
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
        let kind_str = match pr.get(&Iri::parse(wk::PARAM_KIND).unwrap()) {
            Some(Value::String(s)) => s.as_str(),
            _ => "urn:eigenius:core:Set",
        };
        let kind_exp = decode_param_kind_str(kind_str);
        params.push((Patt::Var(name), kind_exp));
    }
    Ok(params)
}

/// Map a chain-side `param_kind` string from a `core:type_params` entry
/// to its kernel-side `Exp`. Recognises:
///
/// - `core:Size` (and the bare `Size`) → `Exp::SizeSort` (sized binders).
/// - `core:Prop` (and bare `Prop`) → `Exp::Sort(0)` (D46 §3).
/// - The four primitive type IRIs `core:string` / `core:integer` /
///   `core:float` / `core:boolean` → `Exp::EigonPrimitive(...)`. This
///   is what makes value-parameter inductives like D39 §4.1's
///   `Asserts(iri) : Prop` decodable — the `iri` parameter is typed
///   at `core:string` and the chain-resident declaration carries that
///   IRI verbatim.
///
/// Everything else falls through to `Exp::Sort(1)` (Set) — the
/// forward-compat default that preserves pre-D49 decoder behaviour.
fn decode_param_kind_str(kind_str: &str) -> Exp {
    match kind_str {
        s if s.ends_with(":Size") || s == "Size" => Exp::SizeSort,
        s if s.ends_with(":Prop") || s == "Prop" => Exp::Sort(0),
        "Set" => Exp::Sort(1),
        s if s.starts_with("Type:") => {
            let n: usize = s["Type:".len()..].parse().unwrap_or(0);
            Exp::Sort(n + 1)
        }
        wk::STRING => Exp::EigonPrimitive(PrimitiveType::String),
        wk::INTEGER => Exp::EigonPrimitive(PrimitiveType::Integer),
        wk::FLOAT => Exp::EigonPrimitive(PrimitiveType::Float),
        wk::BOOLEAN => Exp::EigonPrimitive(PrimitiveType::Boolean),
        _ => Exp::Sort(1),
    }
}

/// Index-kind variant of [`decode_param_kind_str`]. Adds the bare-name
/// → `Exp::Var(name)` path the ESL compiler uses for index telescopes
/// whose entries reference a parameter by name (e.g. `data Eq(A) : A
/// -> A -> Prop` records the index kind as `"A"` — a bare name —
/// rather than a fully-qualified IRI). Any fully-qualified IRI flows
/// through the param-kind matcher; un-`urn:`-prefixed names that
/// don't parse as IRIs and aren't bare names fall through to
/// `Sort(1)` as a forward-compat default.
fn decode_index_kind_str(kind_str: &str) -> Exp {
    match kind_str {
        s if s.ends_with(":Size") || s == "Size" => Exp::SizeSort,
        s if s.ends_with(":Prop") || s == "Prop" => Exp::Sort(0),
        "Set" => Exp::Sort(1),
        s if s.starts_with("Type:") => {
            let n: usize = s["Type:".len()..].parse().unwrap_or(0);
            Exp::Sort(n + 1)
        }
        wk::STRING => Exp::EigonPrimitive(PrimitiveType::String),
        wk::INTEGER => Exp::EigonPrimitive(PrimitiveType::Integer),
        wk::FLOAT => Exp::EigonPrimitive(PrimitiveType::Float),
        wk::BOOLEAN => Exp::EigonPrimitive(PrimitiveType::Boolean),
        _ => {
            if let Ok(iri) = Iri::parse(kind_str) {
                Exp::EigonClass(iri)
            } else if !kind_str.contains(':') {
                Exp::Var(kind_str.to_string())
            } else {
                Exp::Sort(1)
            }
        }
    }
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
            DecodedArg::SizedBinder { name, upper } => Exp::SizedPi {
                patt: Patt::Var(name),
                upper: Box::new(upper),
                body: Box::new(result),
            },
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
    /// Bounded size binder — the binder's kind is always `SizeSort`
    /// (implicit; not carried in the variant) and the `upper`
    /// expression must normalise to a rigid size variable or ∞.
    SizedBinder { name: String, upper: Exp },
}

/// Decode a constructor-arg resource into a `DecodedArg`.
///
/// Binder-shaped resources carry `binder_name`; everything else is
/// positional. A binder whose kind is `core:Size`/`Size` and that
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
        let bound_str = r
            .get(&Iri::parse(wk::BINDER_BOUND).unwrap())
            .and_then(|v| v.as_str());
        if let Some(bstr) = bound_str {
            if !matches!(kind_exp, Exp::SizeSort) {
                return Err(format!(
                    "ctor binder `{name}` has `binder_bound` but its kind is not `Size`"
                ));
            }
            let upper = decode_bare_size_ref(bstr);
            Ok(DecodedArg::SizedBinder { name, upper })
        } else {
            Ok(DecodedArg::PiBinder {
                name,
                kind: kind_exp,
            })
        }
    } else {
        Ok(DecodedArg::Positional(decode_arg_type(
            class_iri, self_ref, value, layer,
        )?))
    }
}

/// Decode a size-reference string used in `binder_bound` position
/// to its corresponding kernel `Exp`.
///
/// Mirrors the bare-name branch of `decode_arg_type`, restricted to
/// the values we actually accept as SizedPi upper bounds: `Inf`,
/// `Size`, or a bare parameter/variable name (emits `Exp::Var`).
fn decode_bare_size_ref(s: &str) -> Exp {
    match s {
        "Inf" => Exp::SizeInf,
        "Size" => Exp::SizeSort,
        other if !other.contains(':') => Exp::Var(other.to_string()),
        // An IRI here would be unusual (resolved bounds don't really
        // make sense), but fall back to treating it as a named
        // reference via `Var` — the check-time validation will
        // reject it if the upper-bound shape is wrong.
        other => Exp::Var(other.to_string()),
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
    let type_name = match r.get(&Iri::parse(wk::TYPE_NAME).unwrap()) {
        Some(Value::String(s)) => s.as_str(),
        _ => return Err("InductiveArgType missing `type_name`".to_string()),
    };
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
    // Bare `Inf` and `Size` are reserved literals for the size sort:
    // the ESL compile step lets them through un-resolved so this
    // decoder can turn them into their corresponding kernel Exp.
    if !type_name.contains(':') {
        if !type_args_arr.is_empty() {
            return Err(format!(
                "bare parameter reference `{type_name}` cannot take type arguments"
            ));
        }
        return Ok(match type_name {
            "Inf" => Exp::SizeInf,
            "Size" => Exp::SizeSort,
            other => Exp::Var(other.to_string()),
        });
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
    if let Some(other_resource_arc) = layer.resolve(&arg_iri) {
        let other_resource: &crate::ontology::resource::Resource = &other_resource_arc;
        if is_inductive_type(other_resource) {
            let other_name = match other_resource.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
                Some(Value::String(s)) => s.clone(),
                _ => arg_iri.local_name().to_string(),
            };
            let stub = Arc::new(InductiveDecl {
                iri: arg_iri.clone(),
                name: other_name,
                params: Vec::new(),
                indices: Vec::new(),
                sort: Exp::Sort(1),
                ctors: Vec::new(),
            });
            let sub_args: Result<Vec<Exp>, String> = type_args_arr
                .iter()
                .map(|a| decode_arg_type(class_iri, self_ref, a, layer))
                .collect();
            return Ok(Exp::InductiveType(stub, sub_args?));
        }
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

    fn build_test_layer() -> Arc<Layer> {
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
        // Dog has 2 required properties (name from Animal, breed from Dog)
        assert!(matches!(typ, Val::Sig(_, _)));
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

    #[test]
    fn option_type_has_two_constructors() {
        let opt = make_option_type(Val::EigonPrimitive(PrimitiveType::String));
        match opt {
            Val::Data(summands, _) => {
                assert_eq!(summands.len(), 2);
                assert_eq!(summands[0].0, "some");
                assert_eq!(summands[1].0, "none");
            }
            _ => panic!("expected Sum type for Option"),
        }
    }

    #[test]
    fn readback_class_with_recommends_roundtrips() -> Result<(), Box<dyn std::error::Error>> {
        // This tests the exact path that caused the __data_0 crash:
        // resolve a class with recommends → readback → re-evaluate
        let layer = build_test_layer();
        // core:Class has recommends, so it will have Option types
        let iri = Iri::parse(wk::CLASS).unwrap();
        let typ = resolve_class_type(&iri, &layer)?;

        // Readback to expression
        let exp = crate::nbe::readback::readback_val(0, &typ);

        // Re-evaluate — this is what parse_program does, and it used to crash
        let val = crate::nbe::eval::eval(&exp, &Rho::Nil)?;

        // Should still be a Sigma type
        assert!(
            matches!(val, Val::Sig(_, _)),
            "re-evaluated class type should be Sig, got {:?}",
            val
        );
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
                    matches!(decl.sort, Exp::Sort(0)),
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
    fn resolve_list_parametric_inductive_from_esl() {
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:List(A : core:Set) {
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
                        assert!(matches!(dom.as_ref(), Exp::Sort(1)));
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

    #[test]
    fn resolve_sized_inductive_with_size_kind_param() {
        // ESL source declaring an inductive with a `Size` parameter.
        // The ground decoder should resolve the param kind to
        // `Exp::SizeSort`, enabling sized-type subtyping at the
        // kernel level.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:SizedNat(i : core:Size) {
                zero,
                succ(ex:SizedNat(i)),
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedNat").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SizedNat");

        match val {
            Val::InductiveType { decl, .. } => {
                assert_eq!(decl.name, "SizedNat");
                assert_eq!(decl.params.len(), 1);
                assert!(
                    matches!(decl.params[0].1, Exp::SizeSort),
                    "Size-kinded param must decode to Exp::SizeSort, got {:?}",
                    decl.params[0].1
                );
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

    #[test]
    fn sized_nat_with_bounded_binder_decodes_to_sized_pi() {
        // Full sized Nat from ESL: the ctor binder `{j < i}` must
        // decode to `Exp::SizedPi` in the constructor's telescope.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:SizedNat(i : core:Size) {
                zero,
                succ({j < i}, ex:SizedNat(j)),
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedNat").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SizedNat");
        let decl = match val {
            Val::InductiveType { decl, .. } => decl,
            other => panic!("expected Val::InductiveType, got {other:?}"),
        };

        // succ's type telescope should be:
        //   Π i : Size. SizedPi{j < i}. Π _ : SizedNat(j). SizedNat(i)
        let succ = &decl.ctors[1];
        let after_params = match &succ.typ {
            Exp::Pi(Patt::Var(p), dom, body) => {
                assert_eq!(p, "i");
                assert!(matches!(**dom, Exp::SizeSort));
                body.as_ref()
            }
            other => panic!("expected outer Pi on succ, got {other:?}"),
        };
        match after_params {
            Exp::SizedPi { patt, upper, body } => {
                assert!(matches!(patt, Patt::Var(n) if n == "j"));
                assert!(matches!(upper.as_ref(), Exp::Var(n) if n == "i"));
                // Body should be `Π _ : SizedNat(j). SizedNat(i)`.
                match body.as_ref() {
                    Exp::Pi(_, arg_dom, arg_body) => {
                        match arg_dom.as_ref() {
                            Exp::InductiveType(d, args) => {
                                assert_eq!(d.name, "SizedNat");
                                assert!(matches!(&args[0], Exp::Var(v) if v == "j"));
                            }
                            other => panic!("expected InductiveType arg dom, got {other:?}"),
                        }
                        match arg_body.as_ref() {
                            Exp::InductiveType(_, args) => {
                                assert!(matches!(&args[0], Exp::Var(v) if v == "i"));
                            }
                            other => panic!("expected result InductiveType, got {other:?}"),
                        }
                    }
                    other => panic!("expected Pi after SizedPi, got {other:?}"),
                }
            }
            other => panic!("expected SizedPi after params, got {other:?}"),
        }
    }

    #[test]
    fn sized_nat_esl_succ_at_non_decreasing_size_rejected() {
        // End-to-end rejection: with ESL-declared sized Nat, invoking
        // `succ(i, zero(i))` at the outer size `i` fails because the
        // ctor's SizedPi requires the size arg strictly below `i`.
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::{gen_val, up_gamma, Rho};
        use crate::nbe::term::Patt;

        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:SizedNat(i : core:Size) {
                zero,
                succ({j < i}, ex:SizedNat(j)),
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedNat").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SizedNat");
        let decl = match val {
            Val::InductiveType { decl, .. } => decl,
            other => panic!("expected Val::InductiveType, got {other:?}"),
        };

        // Bind `i : Size` in the context.
        let i_val = gen_val(&Rho::Nil);
        let rho = Rho::Nil.extend(Patt::Var("i".to_string()), i_val.clone());
        let gamma = up_gamma(
            &Vec::new(),
            &Patt::Var("i".to_string()),
            &Val::SizeSort,
            &i_val,
        )
        .unwrap();
        let mut c = CheckCtx::with_layer(rho, gamma, layer);

        // `zero` and `succ(i, zero)` at expected SizedNat(i).
        let ty = Val::InductiveType {
            decl: decl.clone(),
            params: vec![i_val],
            indices: Vec::new(),
        };
        let zero = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        let bad = Exp::InductiveCtor(
            decl,
            "succ".to_string(),
            vec![Exp::Var("i".to_string()), zero],
        );
        let err = check(&mut c, &bad, &ty).unwrap_err().to_string();
        assert!(
            err.contains("not strictly below"),
            "expected sized-bound error, got: {err}"
        );
    }

    #[test]
    fn sized_codata_with_bounded_binder_from_esl() {
        // ESL source declares a sized codata with a SizedPi in an
        // observation type. Round-trips through compile → layer →
        // resolve to a kernel type that, when applied to concrete
        // size/type arguments, yields a `Val::Codata` whose tail
        // observation has `SizedPi` in its expression.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:SizedBox(i : core:Size, A : core:Set) {
                get : A;
                shrink : {j < i} -> A;
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedBox").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SizedBox");

        // Parameterised codata resolves to the unapplied type former
        // `Val::CodataType { decl, params: [] }`. Applying the decl
        // to concrete arguments via `Exp::CodataType(decl, [...])`
        // produces the applied codata type.
        let decl = match &val {
            Val::CodataType { decl, params } => {
                assert!(params.is_empty(), "unapplied type former");
                decl.clone()
            }
            other => panic!("expected Val::CodataType for parameterised codata, got {other:?}"),
        };
        assert_eq!(decl.name, "SizedBox");
        assert_eq!(decl.params.len(), 2);

        // Apply `SizedBox(Inf, One)` and inspect the `shrink`
        // observation's type after parameter substitution.
        use crate::nbe::check::lookup_codata_observation;
        use crate::nbe::eval::eval;
        let applied_ty_val = eval(
            &Exp::CodataType(decl.clone(), vec![Exp::SizeInf, Exp::One]),
            &Rho::Nil,
        )
        .expect("apply codata params");
        let (decl_applied, params_applied) = match &applied_ty_val {
            Val::CodataType { decl, params } => (decl.clone(), params.clone()),
            other => panic!("expected Val::CodataType after applying params, got {other:?}"),
        };
        assert_eq!(params_applied.len(), 2);

        let shrink_ty =
            lookup_codata_observation(&decl_applied, &params_applied, "shrink", 0).unwrap();
        let shrink_exp = crate::nbe::readback::readback_val(0, &shrink_ty);
        match shrink_exp {
            Exp::SizedPi { patt, upper, body } => {
                assert!(matches!(patt, Patt::Var(_)));
                assert!(
                    matches!(*upper, Exp::SizeInf),
                    "upper should resolve to SizeInf after applying i=Inf, got {:?}",
                    upper
                );
                assert!(matches!(*body, Exp::One));
            }
            other => panic!("expected SizedPi, got {other:?}"),
        }
    }

    #[test]
    fn sized_codata_corecord_inhabits_sized_type_from_esl() {
        // The productivity-by-typing unlock, end-to-end through ESL:
        // declare a sized codata, construct the concrete codata type
        // at specific size/type arguments, and type-check a corecord
        // value against it. The corecord's `shrink` field must be a
        // lambda whose body type-checks under the TSO hypothesis the
        // SizedPi introduces.
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::eval::eval;

        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:SizedBox(i : core:Size, A : core:Set) {
                get : A;
                shrink : {j < i} -> A;
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedBox").unwrap();
        let codata_former = resolve_class_type(&iri, &layer).expect("resolve SizedBox");
        let decl = match codata_former {
            Val::CodataType { decl, .. } => decl,
            other => panic!("expected Val::CodataType, got {other:?}"),
        };

        // Apply `SizedBox(Inf, One)` to get a concrete applied codata.
        let ty = eval(
            &Exp::CodataType(decl, vec![Exp::SizeInf, Exp::One]),
            &Rho::Nil,
        )
        .expect("apply codata params");

        // Corecord `{ get = Unit, shrink = λ_. Unit }` : SizedBox(∞, 1).
        let corecord = Exp::CoRecord(vec![
            crate::nbe::term::CoField {
                name: "get".to_string(),
                body: Exp::Unit,
            },
            crate::nbe::term::CoField {
                name: "shrink".to_string(),
                body: Exp::Lam(Patt::Var("j".to_string()), Box::new(Exp::Unit)),
            },
        ]);
        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        check(&mut c, &corecord, &ty).expect("sized corecord from ESL-declared codata type-checks");
    }

    // --- Self-referential parameterised codata (Phase 11b step 15j, D19 §8) ---

    #[test]
    fn self_referential_sized_stream_from_esl() {
        // The D19 §8.2 motivating example — self-referential sized
        // stream. The `tail` observation's type references the
        // enclosing codata itself, applied to a strictly-smaller
        // size. Verifies that the resolver emits
        // `Exp::CodataType(self_ref, [A, j])` and that applying the
        // outer codata to concrete args yields a well-typed tail
        // observation.
        use crate::nbe::check::lookup_codata_observation;
        use crate::nbe::eval::eval;

        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            codata ex:Stream(A : core:Set, i : core:Size) {
                head : A;
                tail : {j < i} -> ex:Stream(A, j);
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:Stream").unwrap();
        let former = resolve_class_type(&iri, &layer).expect("resolve Stream");
        let decl = match former {
            Val::CodataType { decl, .. } => decl,
            other => panic!("expected Val::CodataType, got {other:?}"),
        };
        assert_eq!(decl.name, "Stream");

        // `tail`'s declared type should be
        //   SizedPi { j < i }. CodataType(self_ref, [A, j])
        // with the inner CodataType carrying a self-reference.
        let tail = decl
            .observations
            .iter()
            .find(|o| o.name == "tail")
            .expect("tail present");
        match &tail.typ {
            Exp::SizedPi { upper, body, .. } => {
                assert!(
                    matches!(upper.as_ref(), Exp::Var(v) if v == "i"),
                    "upper should be `i`, got {:?}",
                    upper
                );
                match body.as_ref() {
                    Exp::CodataType(inner_decl, args) => {
                        assert_eq!(inner_decl.name, "Stream", "self-ref resolves to Stream");
                        assert_eq!(args.len(), 2);
                        assert!(matches!(&args[0], Exp::Var(v) if v == "A"));
                        assert!(matches!(&args[1], Exp::Var(v) if v == "j"));
                    }
                    other => panic!("tail body should be CodataType self-ref, got {other:?}"),
                }
            }
            other => panic!("tail type should be SizedPi, got {other:?}"),
        }

        // Apply `Stream(One, Inf)` and verify `tail` evaluates to a
        // SizedPi returning Stream(One, j) — the whole self-ref
        // chain rounds trips through eval.
        let applied_val = eval(
            &Exp::CodataType(decl.clone(), vec![Exp::One, Exp::SizeInf]),
            &Rho::Nil,
        )
        .expect("apply Stream(One, Inf)");
        let (d, p) = match &applied_val {
            Val::CodataType { decl, params } => (decl.clone(), params.clone()),
            other => panic!("expected Val::CodataType, got {other:?}"),
        };
        let tail_ty = lookup_codata_observation(&d, &p, "tail", 0).unwrap();
        let tail_exp = crate::nbe::readback::readback_val(0, &tail_ty);
        match tail_exp {
            Exp::SizedPi { body, upper, .. } => {
                assert!(matches!(*upper, Exp::SizeInf));
                match *body {
                    Exp::CodataType(d2, args) => {
                        assert_eq!(d2.name, "Stream");
                        assert_eq!(args.len(), 2);
                        assert!(matches!(&args[0], Exp::One));
                        // Inner `j` is a neutral Var from the SizedPi binder.
                        assert!(matches!(&args[1], Exp::Var(_)));
                    }
                    other => panic!("expected CodataType self-ref, got {other:?}"),
                }
            }
            other => panic!("expected SizedPi, got {other:?}"),
        }
    }

    #[test]
    fn mixed_sized_inductive_and_codata_from_esl() {
        // D19 §13 step 18 — the mixed-kinds test. Declare a sized
        // inductive and a (non-self-ref) codata side by side; apply
        // the codata at a type whose shape is an inductive value;
        // type-check a corecord that produces inductive values
        // through the codata's observations.
        //
        // Kept non-self-referential for the codata so the test is a
        // single-level corecord — self-referential sized codata is
        // covered by `self_referential_sized_stream_from_esl` above.
        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::eval::eval;

        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:SizedNat(i : core:Size) {
                zero,
                succ({j < i}, ex:SizedNat(j)),
            }

            codata ex:Container(A : core:Set) {
                get : A;
            }
            "#,
        );
        let nat_iri = Iri::parse("urn:eigenius:example:SizedNat").unwrap();
        let container_iri = Iri::parse("urn:eigenius:example:Container").unwrap();

        let nat_decl = match resolve_class_type(&nat_iri, &layer).unwrap() {
            Val::InductiveType { decl, .. } => decl,
            other => panic!("expected InductiveType, got {other:?}"),
        };
        let container_decl = match resolve_class_type(&container_iri, &layer).unwrap() {
            Val::CodataType { decl, .. } => decl,
            other => panic!("expected CodataType, got {other:?}"),
        };

        // `Container(SizedNat(Inf))` — codata parameterised over an
        // inductive type value, showing mixed-kinds type formation.
        let element_ty = Exp::InductiveType(nat_decl.clone(), vec![Exp::SizeInf]);
        let ty = eval(
            &Exp::CodataType(container_decl, vec![element_ty]),
            &Rho::Nil,
        )
        .expect("apply Container params");

        // `corecord { get = zero[Inf] }` — the get observation
        // produces an inductive value. Exercises: codata observation
        // type carries a parameter that evaluates to an inductive
        // type, and the corecord's field body is an inductive ctor
        // application which check_inductive_ctor_args verifies.
        let corecord = Exp::CoRecord(vec![crate::nbe::term::CoField {
            name: "get".to_string(),
            body: Exp::InductiveCtor(nat_decl, "zero".to_string(), Vec::new()),
        }]);
        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer);
        check(&mut c, &corecord, &ty)
            .expect("mixed sized-inductive + codata corecord type-checks end-to-end");
    }

    #[test]
    fn sized_types_end_to_end_esl_to_check() {
        // End-to-end exercise (Phase 11b step 15i):
        //   1. ESL source declares `data ex:SizedNat(i : core:Size)`
        //      with `zero` and `succ(ex:SizedNat(i))` constructors.
        //   2. Layer-build + ground resolution yields an
        //      `InductiveDecl` whose one parameter has `Exp::SizeSort`.
        //   3. The kernel type-checker admits the constructors when
        //      they're checked at the type `SizedNat(Inf)` — the
        //      `Inf` param applying to `SizeSort` exercises the whole
        //      sized-type chain from ESL surface down through
        //      `subtype_of` on the constructor's result type.
        //   4. A stronger "productive" check: the variable `x :
        //      SizedNat(Inf)` can be used where `SizedNat(Inf)` is
        //      expected (reflexive subtyping), exercising the sized
        //      subtyping branch of the type checker on values that
        //      originated from real ESL code.

        use crate::nbe::check::{check, CheckCtx};
        use crate::nbe::env::{up_gamma, Rho};
        use crate::nbe::eval::eval;
        use crate::nbe::term::Patt;

        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:SizedNat(i : core:Size) {
                zero,
                succ(ex:SizedNat(i)),
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedNat").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SizedNat");
        let decl = match val {
            Val::InductiveType { decl, .. } => decl,
            other => panic!("expected Val::InductiveType, got {other:?}"),
        };
        // Sanity: size parameter decoded correctly.
        assert!(matches!(decl.params[0].1, Exp::SizeSort));

        // 3. Build `SizedNat(Inf)` as a target type and check `zero` against it.
        let snat_inf = Val::InductiveType {
            decl: decl.clone(),
            params: vec![Val::SizeInf],
            indices: Vec::new(),
        };
        let mut c = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        let zero_exp = Exp::InductiveCtor(decl.clone(), "zero".to_string(), Vec::new());
        check(&mut c, &zero_exp, &snat_inf).expect("zero : SizedNat(Inf) via ESL pipeline");

        // `succ(zero)` at SizedNat(Inf).
        let succ_zero = Exp::InductiveCtor(decl.clone(), "succ".to_string(), vec![zero_exp]);
        check(&mut c, &succ_zero, &snat_inf).expect("succ(zero) : SizedNat(Inf) via ESL pipeline");

        // 4. Reflexive subtyping via the checker fallthrough: put
        //    `x : SizedNat(Inf)` into gamma, then check `x` against
        //    the same type. This path goes through `subtype_of_with_hyps`
        //    → InductiveType branch → size_le_with_hyps on the size
        //    param position.
        let x_val = crate::nbe::env::gen_val(&c.rho);
        let rho2 = c
            .rho
            .clone()
            .extend(Patt::Var("x".to_string()), x_val.clone());
        let gamma2 = up_gamma(&c.gamma, &Patt::Var("x".to_string()), &snat_inf, &x_val).unwrap();
        let mut c2 = CheckCtx::with_layer(rho2, gamma2, layer);
        check(&mut c2, &Exp::Var("x".to_string()), &snat_inf)
            .expect("x : SizedNat(Inf) checks against SizedNat(Inf)");

        // Also validate that ESL's `Inf` literal end-to-end yields a
        // value that evaluates to `Val::SizeInf`, not a phantom Var.
        let inf_exp = Exp::SizeInf;
        let inf_val = eval(&inf_exp, &c2.rho).expect("eval Inf");
        assert!(matches!(inf_val, Val::SizeInf));
    }

    #[test]
    fn resolve_inductive_with_inf_literal_in_ctor_arg() {
        // ESL source where a ctor arg type uses the `Inf` literal
        // in place of a parameter-variable size position — the
        // decoder must emit `Exp::SizeInf` in that position.
        let layer = build_layer_with_esl(
            r#"
            namespace core = "urn:eigenius:core";
            namespace ex = "urn:eigenius:example";

            data ex:SizedBox(i : core:Size) {
                mk(ex:SizedBox(Inf)),
            }
            "#,
        );
        let iri = Iri::parse("urn:eigenius:example:SizedBox").unwrap();
        let val = resolve_class_type(&iri, &layer).expect("resolve SizedBox");
        match val {
            Val::InductiveType { decl, .. } => {
                assert_eq!(decl.ctors.len(), 1);
                // mk's type: Π i:Size. Π _:SizedBox(Inf). SizedBox(i)
                // — drill into the inner InductiveType's first param.
                let mk = &decl.ctors[0];
                // Peel outer Π i:Size.
                let inner = match &mk.typ {
                    Exp::Pi(_, _, body) => body.as_ref(),
                    other => panic!("expected outer Pi, got {other:?}"),
                };
                // Next Π _:SizedBox(Inf).
                let arg_ty = match inner {
                    Exp::Pi(_, dom, _) => dom.as_ref(),
                    other => panic!("expected arg Pi, got {other:?}"),
                };
                match arg_ty {
                    Exp::InductiveType(d, sub_args) => {
                        assert_eq!(d.name, "SizedBox");
                        assert_eq!(sub_args.len(), 1);
                        assert!(
                            matches!(sub_args[0], Exp::SizeInf),
                            "ctor arg at size-position should be SizeInf, got {:?}",
                            sub_args[0]
                        );
                    }
                    other => panic!("expected InductiveType for arg, got {other:?}"),
                }
            }
            other => panic!("expected Val::InductiveType, got {other:?}"),
        }
    }

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
                    Exp::Sort(0) => {}
                    other => panic!("index 0: expected Sort(0) for Prop, got {other:?}"),
                }
                match &decl.sort {
                    Exp::Sort(1) => {}
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

    #[test]
    fn decode_param_kind_str_maps_sort_literals() {
        // D39 §5 / D49 ChainWitness predicates need the kernel decoder
        // to recognise the Sort-literal kind strings the ESL compiler
        // emits for intermediate index positions ("Prop" / "Set" /
        // "Type:N"). Without this mapping, JustifiedBy and similar
        // sort-indexed predicates can't round-trip through the codec.
        assert!(matches!(decode_param_kind_str("Prop"), Exp::Sort(0)));
        assert!(matches!(decode_param_kind_str("Set"), Exp::Sort(1)));
        assert!(matches!(decode_param_kind_str("Type:0"), Exp::Sort(1)));
        assert!(matches!(decode_param_kind_str("Type:2"), Exp::Sort(3)));
        assert!(matches!(decode_param_kind_str("Type:7"), Exp::Sort(8)));
    }

    #[test]
    fn decode_index_kind_str_maps_sort_literals() {
        // Index-kind variant — same Sort-literal coverage as
        // `decode_param_kind_str` plus the bare-name and qualified-IRI
        // paths the index telescope can exercise.
        assert!(matches!(decode_index_kind_str("Prop"), Exp::Sort(0)));
        assert!(matches!(decode_index_kind_str("Set"), Exp::Sort(1)));
        assert!(matches!(decode_index_kind_str("Type:0"), Exp::Sort(1)));
        assert!(matches!(decode_index_kind_str("Type:5"), Exp::Sort(6)));
        // Confirm the bare-name path still resolves to a variable so the
        // new Sort-literal arms don't shadow legitimate index references.
        assert!(matches!(decode_index_kind_str("A"), Exp::Var(ref s) if s == "A"));
    }
}
