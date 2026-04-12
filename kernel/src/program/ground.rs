//! Ground type resolution: bridge between Eigon ontology and Mini-TT types.
//!
//! Resolves class IRIs from the layer chain into Mini-TT Sigma types
//! (nested dependent pairs over the class's required properties).

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::term::{Patt, PrimitiveType};
use crate::nbe::val::{Clos, Val};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use std::collections::BTreeSet;

/// Resolve a class IRI to a Mini-TT type (nested Sigma over required properties).
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

    let _resource = layer
        .resolve(class_iri)
        .ok_or_else(|| format!("class '{}' not found in layer chain", class_iri))?;

    let required = collect_required_properties(class_iri, layer)?;
    if required.is_empty() {
        return Ok(Val::One);
    }

    let props: Vec<(Iri, Val)> = required
        .into_iter()
        .filter_map(|prop_iri| {
            let prop_type = resolve_property_type(&prop_iri, layer).ok()?;
            Some((prop_iri, prop_type))
        })
        .collect();

    build_sigma_chain(&props)
}

fn collect_required_properties(class_iri: &Iri, layer: &Layer) -> Result<BTreeSet<Iri>, String> {
    let mut required = BTreeSet::new();
    let mut visited = BTreeSet::new();
    collect_required_inner(class_iri, layer, &mut required, &mut visited)?;
    Ok(required)
}

fn collect_required_inner(
    class_iri: &Iri,
    layer: &Layer,
    required: &mut BTreeSet<Iri>,
    visited: &mut BTreeSet<Iri>,
) -> Result<(), String> {
    if !visited.insert(class_iri.clone()) {
        return Ok(());
    }
    let resource = match layer.resolve(class_iri) {
        Some(r) => r,
        None => return Ok(()),
    };
    let requires_iri = Iri::parse(wk::REQUIRES).unwrap();
    if let Some(requires_val) = resource.get(&requires_iri) {
        for prop_iri in requires_val.as_iri_array() {
            required.insert(prop_iri);
        }
    }
    let subclass_iri = Iri::parse(wk::PARENT_CLASSES).unwrap();
    if let Some(parents_val) = resource.get(&subclass_iri) {
        for parent_iri in parents_val.as_iri_array() {
            collect_required_inner(&parent_iri, layer, required, visited)?;
        }
    }
    Ok(())
}

fn resolve_property_type(prop_iri: &Iri, layer: &Layer) -> Result<Val, String> {
    let resource = layer
        .resolve(prop_iri)
        .ok_or_else(|| format!("property '{}' not found", prop_iri))?;
    let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
    let data_type_str = match resource.get(&dt_iri) {
        Some(Value::String(s)) => s.clone(),
        _ => return Ok(Val::Set),
    };
    match data_type_str.as_str() {
        wk::STRING => Ok(Val::EigonPrimitive(PrimitiveType::String)),
        wk::INTEGER => Ok(Val::EigonPrimitive(PrimitiveType::Integer)),
        wk::FLOAT => Ok(Val::EigonPrimitive(PrimitiveType::Float)),
        wk::BOOLEAN => Ok(Val::EigonPrimitive(PrimitiveType::Boolean)),
        wk::JSON => Ok(Val::EigonPrimitive(PrimitiveType::Json)),
        wk::RESOURCE => {
            let ct_iri = Iri::parse(wk::CLASS_TYPES).unwrap();
            if let Some(ct_val) = resource.get(&ct_iri) {
                let class_iris = ct_val.as_iri_array();
                if let Some(first) = class_iris.first() {
                    return Ok(Val::EigonClass(first.clone()));
                }
            }
            Ok(Val::Set)
        }
        _ => Ok(Val::Set),
    }
}

fn build_sigma_chain(props: &[(Iri, Val)]) -> Result<Val, String> {
    if props.is_empty() {
        return Ok(Val::One);
    }
    let (prop_iri, prop_type) = &props[0];
    let rest_type = build_sigma_chain(&props[1..])?;
    let rest_exp = crate::nbe::readback::readback_val(0, &rest_type);
    let closure = Clos::new(
        Patt::Var(prop_iri.local_name().to_string()),
        rest_exp,
        Rho::Nil,
    );
    Ok(Val::Sig(Box::new(prop_type.clone()), closure))
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
        assert!(matches!(typ, Val::Sig(_, _)));
    }

    #[test]
    fn resolve_nonexistent() {
        let layer = build_test_layer();
        let iri = Iri::parse("urn:eigenius:nonexistent:Foo").unwrap();
        assert!(resolve_class_type(&iri, &layer).is_err());
    }
}
