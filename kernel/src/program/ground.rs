//! Ground type resolution: bridge between Eigon ontology and Mini-TT types.
//!
//! Resolves class IRIs from the layer chain into Mini-TT Sigma types.
//! Required properties map to direct Sigma components.
//! Recommended properties map to Option (Sum(some T | none 1)) components.
//! Constraints (allows_only, class_types) map to Sum types.

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, InductiveCtorDecl, InductiveDecl, Patt, PrimitiveType};
use crate::nbe::val::{Clos, Val};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Resolve a class IRI to a Mini-TT type.
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

    let resource = layer
        .resolve(class_iri)
        .ok_or_else(|| format!("class '{}' not found in layer chain", class_iri))?;

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
        return resolve_inductive_type(class_iri, resource);
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

/// Resolve a property's data_type to a Mini-TT Val.
///
/// Handles all data types including resource references (with class_types
/// and allows_only), arrays, and primitive types.
pub fn resolve_property_type(prop_iri: &Iri, layer: &Layer) -> Result<Val, String> {
    let resource = layer
        .resolve(prop_iri)
        .ok_or_else(|| format!("property '{}' not found", prop_iri))?;

    let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
    let data_type_str = match resource.get(&dt_iri) {
        Some(Value::String(s)) => s.clone(),
        _ => return Ok(Val::Set), // Unknown data type
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

            Ok(Val::Set) // Untyped resource reference
        }

        wk::RESOURCE_ARRAY => {
            // Array of resources — wrap element type in a list type
            let inner = resolve_array_element_type(resource, layer)?;
            Ok(make_list_type(inner))
        }

        wk::VALUE_ARRAY => {
            // Array of values — wrap element type in a list type
            let et_iri = Iri::parse(wk::ELEMENT_TYPE).unwrap();
            let elem_type = if let Some(Value::String(et_str)) = resource.get(&et_iri) {
                match et_str.as_str() {
                    wk::STRING => Val::EigonPrimitive(PrimitiveType::String),
                    wk::INTEGER => Val::EigonPrimitive(PrimitiveType::Integer),
                    wk::FLOAT => Val::EigonPrimitive(PrimitiveType::Float),
                    wk::BOOLEAN => Val::EigonPrimitive(PrimitiveType::Boolean),
                    _ => Val::Set,
                }
            } else {
                Val::Set
            };
            Ok(make_list_type(elem_type))
        }

        _ => Ok(Val::Set), // Unknown data type
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
    Ok(Val::Set)
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
    is_a.iter()
        .any(|c| c.as_str() == "urn:eigenius:core:CodataType")
}

/// Resolve a CodataType resource into a `Val::Codata` whose observation
/// types are resolved to the same Mini-TT forms as any other typed
/// field. See D11 §3.
fn resolve_codata_type(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    layer: &Layer,
) -> Result<Val, String> {
    let observations_iri = Iri::parse("urn:eigenius:core:observations").unwrap();
    let obs_array = match resource.get(&observations_iri) {
        Some(Value::Array(arr)) => arr,
        _ => {
            return Err(format!(
                "codata type '{class_iri}' missing 'observations' array"
            ))
        }
    };

    let mut observations = Vec::new();
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
        let type_ref = match obs_res.get(&type_iri) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "codata type '{class_iri}' observation '{name}' missing 'observation_type'"
                ))
            }
        };
        let type_iri_parsed = Iri::parse(&type_ref)
            .map_err(|e| format!("invalid observation type IRI '{type_ref}': {e}"))?;
        // Resolve the observation's type via the same machinery —
        // supports recursion through the codata type itself by
        // short-circuiting: self-references resolve to an EigonClass
        // marker that the evaluator can handle.
        let type_val = if type_iri_parsed == *class_iri {
            Val::EigonClass(type_iri_parsed.clone())
        } else {
            resolve_class_type(&type_iri_parsed, layer)?
        };
        // Read back the type value so we have a syntactic Exp for
        // Val::Codata's observation list (eval under Rho::Nil when
        // type checking against it).
        let type_exp = crate::nbe::readback::readback_val(0, &type_val);
        observations.push((name, type_exp));
    }

    Ok(Val::Codata(observations, Rho::Nil))
}

/// Check whether a resource represents an inductive type declaration
/// (Phase 11b step 9).
fn is_inductive_type(resource: &crate::ontology::resource::Resource) -> bool {
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
fn resolve_inductive_type(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
) -> Result<Val, String> {
    let short_name = match resource.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(format!("inductive type '{class_iri}' missing 'short_name'")),
    };

    // Build the self-reference stub used inside constructor types.
    // Empty `ctors` is fine — name-based lookup is all the kernel
    // needs for inner self-refs (see Phase 11b step 2 notes).
    let self_ref = Arc::new(InductiveDecl {
        name: short_name.clone(),
        params: Vec::new(),
        sort: Exp::Set,
        ctors: Vec::new(),
    });

    let params_telescope = decode_params(class_iri, resource)?;
    let ctors = decode_ctors(class_iri, resource, &self_ref, &params_telescope)?;

    let decl = Arc::new(InductiveDecl {
        name: short_name,
        params: params_telescope,
        sort: Exp::Set,
        ctors,
    });
    Ok(Val::InductiveType {
        decl,
        params: Vec::new(),
    })
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
        // Phase 11b v1 admits only kind `Set`; the resource carries the
        // kind for forward-compatibility but we don't branch on it yet.
        params.push((Patt::Var(name), Exp::Set));
    }
    Ok(params)
}

fn decode_ctors(
    class_iri: &Iri,
    resource: &crate::ontology::resource::Resource,
    self_ref: &Arc<InductiveDecl>,
    params: &[(Patt, Exp)],
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
        let arg_types_arr = match cr.get(&Iri::parse(wk::ARG_TYPES).unwrap()) {
            Some(Value::Array(a)) => a.as_slice(),
            None => &[],
            Some(_) => {
                return Err(format!(
                    "inductive type '{class_iri}.{name}' has non-array `arg_types`"
                ))
            }
        };
        let ctor_typ = build_ctor_type(class_iri, self_ref, params, arg_types_arr)?;
        out.push(InductiveCtorDecl {
            name,
            typ: ctor_typ,
        });
    }
    Ok(out)
}

/// Assemble a constructor's full type expression:
/// `Π params. Π args. Self(params)`.
fn build_ctor_type(
    class_iri: &Iri,
    self_ref: &Arc<InductiveDecl>,
    params: &[(Patt, Exp)],
    arg_types: &[Value],
) -> Result<Exp, String> {
    // Result type: Self(param₁, param₂, ...) — each applied param
    // becomes a Var reference to the enclosing binder.
    let param_vars: Vec<Exp> = params
        .iter()
        .map(|(p, _)| match p {
            Patt::Var(n) => Exp::Var(n.clone()),
            _ => Exp::Unit,
        })
        .collect();
    let mut result = Exp::InductiveType(self_ref.clone(), param_vars);

    // Wrap each arg binder in reverse so the first arg is outermost.
    // Arg binders are anonymous (`Patt::Unit`) because the surface
    // syntax doesn't name them — Phase 11b v1 treats constructors as
    // positional.
    for arg in arg_types.iter().rev() {
        let arg_exp = decode_arg_type(class_iri, self_ref, arg)?;
        result = Exp::Pi(Patt::Unit, Box::new(arg_exp), Box::new(result));
    }

    // Wrap each parameter binder in reverse.
    for (patt, kind) in params.iter().rev() {
        result = Exp::Pi(patt.clone(), Box::new(kind.clone()), Box::new(result));
    }

    Ok(result)
}

/// Decode one `InductiveArgType` resource to its `Exp`.
///
/// Three cases driven by the encoded `type_name`:
/// - Bare string (no namespace separator): a parameter reference,
///   emitted as `Exp::Var`.
/// - IRI equal to the enclosing inductive's IRI: a self-reference,
///   emitted as `Exp::InductiveType(self_ref, type_args...)`.
/// - Any other IRI: a class reference; emitted as the matching
///   primitive or `Exp::EigonClass(iri)` to let the type checker
///   resolve it via the layer chain.
fn decode_arg_type(
    class_iri: &Iri,
    self_ref: &Arc<InductiveDecl>,
    value: &Value,
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
            .map(|a| decode_arg_type(class_iri, self_ref, a))
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

    // Any other class IRI: emit an EigonClass marker. The type
    // checker resolves this against the layer chain at use time.
    // Parameterised references (e.g. `Foo(A)`) currently drop their
    // type args on this path — a follow-up will handle external
    // parameterised inductives once cross-inductive references matter.
    if !type_args_arr.is_empty() {
        return Err(format!(
            "cross-inductive parameterised references (`{type_name}(...)`) are not yet \
             supported — only self-references may take type arguments in Phase 11b"
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
        let core = Arc::new(builder.build());

        let animals_json = include_str!("../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        Arc::new(domain_builder.build())
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
        let core = Arc::new(core_builder.build());

        let user_resources = crate::esl::compile(esl_source).expect("ESL compile failed");
        let mut user_builder = LayerBuilder::new("user", Some(core));
        for r in user_resources {
            user_builder.add_resource(r).unwrap();
        }
        Arc::new(user_builder.build())
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
            Val::InductiveType { decl, params } => {
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
            Val::InductiveType { decl, params } => {
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
            Val::InductiveType { decl, params } => {
                assert!(params.is_empty());
                assert_eq!(decl.name, "List");
                assert_eq!(decl.params.len(), 1);
                assert!(matches!(&decl.params[0].0, Patt::Var(n) if n == "A"));

                // nil's type: Π A:Set. List(A)
                match &decl.ctors[0].typ {
                    Exp::Pi(Patt::Var(pn), dom, body) => {
                        assert_eq!(pn, "A");
                        assert!(matches!(dom.as_ref(), Exp::Set));
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
}
