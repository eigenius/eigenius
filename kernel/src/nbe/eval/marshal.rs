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

//! Resource ⇄ Val marshalling at the Eigon boundary. Split from
//! `eval.rs`.

use crate::nbe::val::Val;
use crate::ontology::iri::Iri;

/// Convert an Eigon resource Value to a EigenTT Val.
///
/// Uses a heuristic IRI check: strings starting with "urn:" or "http"
/// are treated as class references (`Val::EigonClass`). This can
/// misclassify string property values that happen to look like IRIs.
/// The principled fix is type-directed conversion consulting the
/// property's declared `data_type` — deferred to Phase 11+ when the
/// type checker has full property-type awareness during evaluation.
pub fn resource_value_to_val(v: &crate::ontology::resource::Value) -> Val {
    use crate::ontology::resource::Value as RVal;
    match v {
        RVal::String(s) => {
            // Check if it looks like an IRI reference
            if let Ok(iri) = Iri::parse(s) {
                if s.starts_with("urn:") || s.starts_with("http") {
                    return Val::EigonClass(iri);
                }
            }
            Val::ResourceVal(Box::new({
                let mut r = crate::ontology::resource::Resource::new_embedded();
                let str_iri = Iri::parse("urn:eigenius:core:string").unwrap();
                r.set(str_iri, RVal::String(s.clone()));
                r
            }))
        }
        RVal::Integer(_) | RVal::Float(_) | RVal::Boolean(_) => {
            Val::ResourceVal(Box::new(crate::ontology::resource::Resource::new_embedded()))
        }
        RVal::Embedded(r) => Val::ResourceVal(r.clone()),
        RVal::Array(items) => Val::List(items.iter().map(resource_value_to_val).collect()),
        RVal::ResourceRef(iri) => Val::EigonClass(iri.clone()),
        RVal::Json(_) => Val::Unit,
        // D43 §4.1: Vector values are transient compute outputs of
        // EMBED that flow into VECTOR_NEAR / VECTOR_SIM at the query
        // surface. They have no inhabitant in the EigenTT type
        // system — opaque to NBE.
        RVal::Vector { .. } => Val::Unit,
    }
}

/// Convert a EigenTT Val to an Eigon resource Value (for Construct).
pub fn val_to_resource_value(val: &Val) -> crate::ontology::resource::Value {
    use crate::ontology::resource::Value as RVal;
    match val {
        Val::ResourceVal(r) => {
            // If the resource has a single string value (e.g. CompleteText output),
            // extract it. Otherwise embed the full resource.
            let props: Vec<_> = r.properties().iter().collect();
            if props.len() == 1 {
                if let (_, RVal::String(s)) = props[0] {
                    return RVal::String(s.clone());
                }
            }
            RVal::Embedded(r.clone())
        }
        Val::Unit => RVal::String(String::new()),
        Val::EigonClass(iri) => RVal::String(iri.as_str().to_string()),
        Val::List(items) => RVal::Array(items.iter().map(val_to_resource_value).collect()),
        Val::Con(ref name, _) if name == "nil" || name == "cons" => {
            match crate::nbe::val::cons_to_vec(val) {
                Some(items) => RVal::Array(items.iter().map(val_to_resource_value).collect()),
                None => {
                    RVal::Embedded(Box::new(crate::ontology::resource::Resource::new_embedded()))
                }
            }
        }
        // Phase 11c: marshal inductive constructor values to embedded
        // resources so institution-registered decide can pattern-match
        // on them. The ctor name is stamped as is_a and each argument
        // recursively marshalled under a positional `ctor_arg_{i}`
        // property. This keeps the shape stable across decl changes —
        // institutions inspect by position, not by user-chosen names
        // (which the kernel doesn't record on ctor args).
        Val::InductiveVal {
            decl,
            ctor_name,
            args,
        } => {
            use crate::ontology::well_known as wk;
            let mut r = crate::ontology::resource::Resource::new_embedded();
            let qualified = format!("{}:{}", decl.name, ctor_name);
            r.set(
                crate::ontology::iri::Iri::parse(wk::IS_A).unwrap(),
                RVal::Array(vec![RVal::String(qualified)]),
            );
            for (i, arg) in args.iter().enumerate() {
                let key_iri =
                    crate::ontology::iri::Iri::parse(&format!("urn:eigenius:kernel:ctor_arg_{i}"))
                        .unwrap();
                r.set(key_iri, val_to_resource_value(arg));
            }
            RVal::Embedded(Box::new(r))
        }
        _ => RVal::Embedded(Box::new(crate::ontology::resource::Resource::new_embedded())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::nbe::eval::testutil::cons_list;

    #[test]
    fn resource_value_array_to_list_val() {
        use crate::ontology::resource::Value as RVal;
        let arr = RVal::Array(vec![RVal::Integer(1), RVal::Integer(2), RVal::Integer(3)]);
        let v = resource_value_to_val(&arr);
        match v {
            Val::List(items) => assert_eq!(items.len(), 3),
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn list_val_to_resource_value_array() {
        use crate::ontology::resource::Value as RVal;
        let list = Val::List(vec![Val::Unit, Val::Unit]);
        let rv = val_to_resource_value(&list);
        match rv {
            RVal::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn cons_list_to_resource_value_array() {
        use crate::ontology::resource::Value as RVal;
        let list = cons_list(vec![Val::Unit, Val::Unit]);
        let rv = val_to_resource_value(&list);
        match rv {
            RVal::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected Array, got {other:?}"),
        }
    }

    // --- Inductive recursor (iota reduction) tests (Phase 11b step 2) ---
}
