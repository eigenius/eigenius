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

/// What a string-valued slot holds: a reference to a chain resource, or text.
///
/// The distinction used to be guessed from the text — `urn:` or `http` meant reference — which
/// reads a property value and answers a question about its DECLARATION. A string is text unless a
/// declaration says otherwise, the same rule `layer::term_mentions` applies after B6.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StringRole {
    /// The slot is declared `core:iri`, `core:resource` or `core:resource_array`.
    Reference,
    /// Anything else, including a slot whose declaration cannot be resolved.
    Text,
}

/// How a property's values should be read, from that property's declared `core:data_type`.
///
/// Unresolvable is [`StringRole::Text`] and that is the whole point: inventing a resource
/// reference out of a string nothing declared as one is the defect this replaced.
pub fn string_role_of(layer: &crate::layer::Layer, prop: &Iri) -> StringRole {
    use crate::ontology::well_known as wk;
    let data_type = layer
        .resolve(prop)
        .and_then(|p| {
            Iri::parse(wk::DATA_TYPE_PROP)
                .ok()
                .and_then(|k| p.get(&k).cloned())
        })
        .and_then(|v| v.as_str().map(str::to_string));
    match data_type.as_deref() {
        Some(wk::IRI_TYPE) | Some(wk::RESOURCE) | Some(wk::RESOURCE_ARRAY) => StringRole::Reference,
        _ => StringRole::Text,
    }
}

/// Convert an Eigon resource Value to a EigenTT Val.
///
/// `role` decides what a string means, and comes from the declaration of the property the value
/// sits under — see [`string_role_of`]. An array passes its own role down, which is right: the
/// elements of a `core:resource_array` are references and the elements of a string array are not.
///
/// Callers with no layer pass [`StringRole::Text`]. That is not a fallback to the old guess; it is
/// the same strict reading, applied where no declaration is reachable.
pub fn resource_value_to_val(v: &crate::ontology::resource::Value, role: StringRole) -> Val {
    use crate::ontology::resource::Value as RVal;
    match v {
        RVal::String(s) => {
            if role == StringRole::Reference {
                if let Ok(iri) = Iri::parse(s) {
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
        // eigenius#142 — the inbound mirror of the `val_to_resource_value`
        // literal arms. These three used to collapse to an EMPTY embedded
        // resource, so `Construct { p = 42 }` followed by `.p` read back
        // `Embedded({})`. `RVal::String` deliberately keeps its
        // one-property-wrapper shape: `val_to_resource_value` unwraps a
        // single-string-prop `ResourceVal` back to `RVal::String` (the
        // `CompleteText` output path) and `decide_structural`'s `as_str`
        // reads the payload out of it, so switching it to `Val::LitString`
        // is a separate change with its own blast radius.
        RVal::Integer(n) => Val::LitInt(*n),
        RVal::Float(f) => Val::LitFloat(*f),
        RVal::Boolean(b) => Val::LitBool(*b),
        RVal::Embedded(r) => Val::ResourceVal(r.clone()),
        RVal::Array(items) => Val::List(
            items
                .iter()
                .map(|i| resource_value_to_val(i, role))
                .collect(),
        ),
        RVal::Json(_) => Val::Unit,
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
        // eigenius#142: literal values carry their payload to the
        // Eigon side. Without these arms the catch-all below turns
        // every literal a `Construct` field evaluates to into an empty
        // embedded resource, so the value never reaches the caller
        // even once `parse_literal` decodes it.
        Val::LitString(s) => RVal::String(s.clone()),
        Val::LitInt(n) => RVal::Integer(*n),
        Val::LitFloat(f) => RVal::Float(*f),
        Val::LitBool(b) => RVal::Boolean(*b),
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
            iri,
            ctor_name,
            args,
        } => {
            use crate::ontology::well_known as wk;
            let mut r = crate::ontology::resource::Resource::new_embedded();
            let qualified = format!("{}:{}", iri.local_name(), ctor_name);
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
        let v = resource_value_to_val(&arr, StringRole::Text);
        match v {
            Val::List(items) => assert_eq!(items.len(), 3),
            other => panic!("expected List, got {other:?}"),
        }
    }

    /// The same IRI-shaped string is a reference or text depending on the DECLARATION, and on
    /// nothing about the string.
    ///
    /// Both slots below hold the identical text. The prefix test this replaced returned
    /// `EigonClass` for both, which invented a resource reference wherever a `core:string` slot
    /// happened to hold something IRI-shaped — a description quoting an IRI, a label, a lexical
    /// entry naming an ontology term.
    #[test]
    fn a_strings_role_comes_from_the_declaration_not_the_text() {
        use crate::ontology::resource::Value as RVal;
        use crate::ontology::well_known as wk;

        fn declare(builder: &mut crate::layer::LayerBuilder, iri_str: &str, data_type: &str) {
            let mut r = crate::ontology::resource::Resource::new(Iri::parse(iri_str).unwrap());
            r.set(
                Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
                RVal::String(data_type.to_string()),
            );
            builder.add_resource(r).unwrap();
        }

        let mut builder = crate::layer::LayerBuilder::new("decls", None);
        declare(&mut builder, "urn:eigenius:t:label", wk::STRING);
        declare(&mut builder, "urn:eigenius:t:target", wk::IRI_TYPE);
        let layer = builder.build(crate::layer::LayerStorage::in_memory());

        let label = Iri::parse("urn:eigenius:t:label").unwrap();
        let target = Iri::parse("urn:eigenius:t:target").unwrap();
        let undeclared = Iri::parse("urn:eigenius:t:never_declared").unwrap();

        assert_eq!(string_role_of(&layer, &label), StringRole::Text);
        assert_eq!(string_role_of(&layer, &target), StringRole::Reference);
        // Nothing declares it, so nothing says it is a reference.
        assert_eq!(string_role_of(&layer, &undeclared), StringRole::Text);

        let text = RVal::String("urn:eigenius:pub:wrn:dd_achilles".to_string());
        assert!(
            matches!(resource_value_to_val(&text, StringRole::Reference), Val::EigonClass(i) if i.as_str() == "urn:eigenius:pub:wrn:dd_achilles")
        );
        assert!(matches!(
            resource_value_to_val(&text, StringRole::Text),
            Val::ResourceVal(_)
        ));
    }

    /// An array passes its own role down: the elements of a `core:resource_array` are references,
    /// and the elements of a string array are not.
    #[test]
    fn an_arrays_elements_take_the_arrays_role() {
        use crate::ontology::resource::Value as RVal;
        let arr = RVal::Array(vec![RVal::String("urn:eigenius:t:a".to_string())]);
        match resource_value_to_val(&arr, StringRole::Reference) {
            Val::List(items) => assert!(matches!(items[0], Val::EigonClass(_))),
            other => panic!("expected List, got {other:?}"),
        }
        match resource_value_to_val(&arr, StringRole::Text) {
            Val::List(items) => assert!(matches!(items[0], Val::ResourceVal(_))),
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
