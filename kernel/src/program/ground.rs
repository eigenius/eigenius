//! Ground type resolution: bridge between Eigon ontology and Mini-TT types.
//!
//! Resolves class IRIs from the layer chain into Mini-TT Sigma types.
//! Required properties map to direct Sigma components.
//! Recommended properties map to Option (Sum(some T | none 1)) components.
//! Constraints (allows_only, class_types) map to Sum types.

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Exp, Patt, PrimitiveType};
use crate::nbe::val::{Clos, Val};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use std::collections::BTreeSet;

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
/// Uses the same encoding as `Exp::list()`: `Data[nil : 1, cons : A × __list_tail]`.
/// The element type is stored in the closure's environment to avoid
/// round-tripping through readback. A dummy `__list_tail` binding is
/// included so that readback (which evaluates summand types) does not
/// hit an unbound variable. Phase 11b replaces this with a proper
/// inductive type.
fn make_list_type(elem: Val) -> Val {
    let var_name = "__list_elem".to_string();
    let rho = Rho::Nil
        .extend(Patt::Var(var_name.clone()), elem)
        .extend(Patt::Var("__list_tail".to_string()), Val::Set);
    Val::Data(
        vec![
            ("nil".to_string(), Exp::One),
            (
                "cons".to_string(),
                Exp::times(Exp::Var(var_name), Exp::Var("__list_tail".to_string())),
            ),
        ],
        rho,
    )
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
}
