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

//! **Constructor classes, derived from `core:ctors`** — D85 §6.1.
//!
//! An inductive VALUE is a resource whose `is_a` names its constructor's class and whose
//! arguments are named properties on that class. This module materialises those classes and
//! properties from the inductive's own `core:ctors`, so they are a PROJECTION of the
//! declaration rather than a second copy of it.
//!
//! **Why derived and not authored.** `core:ctors` holds EMBEDDED `InductiveCtor` resources —
//! no `@id`, inside the inductive's own resource — so an inductive is closed structurally:
//! there is nowhere to add a constructor. Moving constructors to top-level classes would open
//! that, because a top-level resource is addable by anyone. Deriving them keeps closedness
//! structural: a constructor class exists because `core:ctors` has an entry, and there is no
//! other way for one to come into being. Rule 25 answers a class someone writes by hand
//! anyway; on the normal path nothing trips it.
//!
//! **Why at build, before the content hash.** These are ordinary persisted resources, hashed
//! with the layer and identical on reload — not an in-memory convenience. The distinction was
//! learned from `canonicalise_resource_refs`, a build-time rewrite that did NOT survive CBOR,
//! so a reloaded chain carried a shape no reader could rely on (D85 §6.2).
//!
//! **Naming: `<inductive>-<Ctor>` and `<inductive>-<Ctor>-<arg>`.** The separator is forced.
//! ESL admits `[A-Za-z0-9_]` bare and `[A-Za-z0-9_-]` quoted, so `.` is unspellable and
//! `esl::print` hard-errors on it; `_` is spellable but ambiguous, because constructor names
//! contain underscores (`cat_np`, `conn_and`) and `A_B` + `C` would collide with `A` + `B_C`.
//! No component may contain `-`, so splitting on it recovers the parts.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::Layer;

use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI")
}

fn is_a(class: &str) -> Value {
    Value::Array(vec![Value::String(class.to_string())])
}

/// What KIND of thing a `ConstRef` target is, which decides the property's `data_type`.
///
/// The discriminator is what the target IS, not a list of names. Enumerating the primitives
/// missed `core:value_array` — equally a DataType, used by the statistics inductives — and
/// produced a property declaring `class_types: [core:value_array]`, a DataType where a class
/// belongs.
///
/// The Inductive/Class split matters just as much, and follows the ontology's own convention:
/// `core:type_name` and `core:param_kind` both hold a value of `eigentt:Term` and both declare
/// `data_type: core:inductive` with `class_types: [eigentt:Term]`. A `core:resource` slot with
/// an inductive in `class_types` takes a different validation path — Rule 8 dispatches it to the
/// JSON walker — so an argument typed by an inductive must say `core:inductive` or the resource
/// form D85 §1 specifies is rejected in the very slots that exist to hold it.
enum TargetKind {
    DataType,
    Inductive,
    Class,
}

fn target_kind(target: &Iri, own: &BTreeMap<Iri, Resource>, parents: &[Arc<Layer>]) -> TargetKind {
    let found = own
        .get(target)
        .cloned()
        .map(Arc::new)
        .or_else(|| parents.iter().find_map(|p| p.resolve(target)));
    match found {
        Some(r) if r.is_a().contains(&iri(wk::DATA_TYPE)) => TargetKind::DataType,
        Some(r) if r.is_a().contains(&iri(wk::INDUCTIVE_TYPE)) => TargetKind::Inductive,
        _ => TargetKind::Class,
    }
}

/// The IRI a `ConstRef` value resource names, or `None` if it is not a `ConstRef`.
///
/// The resource form of `ConstRef(X)` is `is_a: [<Term>-ConstRef]` with the target on
/// `<Term>-ConstRef-<arg>`. Read structurally — the constructor is the class `is_a` names, and
/// the single argument is the one property besides `is_a` — so this does not hard-code which
/// inductive or which argument name, and works for any term language with a `ConstRef`.
fn const_ref_target(r: &Resource) -> Option<Iri> {
    let is_ctor_ref = r
        .is_a()
        .first()
        .is_some_and(|c| c.as_str().ends_with("-ConstRef"));
    if !is_ctor_ref {
        return None;
    }
    let is_a = iri(wk::IS_A);
    r.properties()
        .iter()
        .find(|(k, _)| **k != is_a)
        .and_then(|(_, v)| v.as_iri())
}

/// What an argument's `core:type_name` says the property should declare.
///
/// Measured over the 88 JSON-declared arguments in the tree, `type_name` takes exactly two
/// shapes: 86 `ConstRef` and 2 `Var`.
///
/// A `Var` is the element type of a PARAMETRIC inductive (`core:List.cons.head`,
/// `core:Option.some.value`). A property cannot carry a type parameter, and it does not need
/// to: R4 says a parametric constructor's argument type is the constructor APPLIED to its type
/// arguments, which is a typing fact rather than a schema one. The property declares what is
/// structurally checkable — that the argument is a value — and the instantiation is checked by
/// Rule 21 and the NbE checker.
fn arg_property_type(
    type_name: Option<&Value>,
    own: &BTreeMap<Iri, Resource>,
    parents: &[Arc<Layer>],
) -> (String, Option<String>) {
    let fallback = || (wk::RESOURCE.to_string(), None);
    // Both shapes, because step 3 migrates these very values: a `type_name` may still be a
    // tagged dict or may already be the value resource. Only `ConstRef` is read either way —
    // `Var` and everything else take the fallback.
    let target = match type_name {
        Some(Value::Json(j)) => {
            if j.get("ctor").and_then(serde_json::Value::as_str) != Some("ConstRef") {
                return fallback();
            }
            j.get("args")
                .and_then(serde_json::Value::as_array)
                .and_then(|a| a.first())
                .and_then(serde_json::Value::as_str)
                .and_then(|s| Iri::parse(s).ok())
        }
        Some(Value::Embedded(r)) => const_ref_target(r),
        _ => return fallback(),
    };
    let Some(target) = target else {
        return fallback();
    };
    match target_kind(&target, own, parents) {
        TargetKind::DataType => (target.as_str().to_string(), None),
        TargetKind::Inductive => (wk::INDUCTIVE.to_string(), Some(target.as_str().to_string())),
        TargetKind::Class => (wk::RESOURCE.to_string(), Some(target.as_str().to_string())),
    }
}

/// Derive every constructor class and argument property for the inductives in `resources`.
///
/// Returns them in IRI order. An IRI already present in `resources` is skipped: an author who
/// wrote the class by hand keeps it, and Rule 25 is what judges whether it is well-formed.
pub(crate) fn derive(resources: &BTreeMap<Iri, Resource>, parents: &[Arc<Layer>]) -> Vec<Resource> {
    let inductive = iri(wk::INDUCTIVE_TYPE);
    let mut out: BTreeMap<Iri, Resource> = BTreeMap::new();

    for (ind_iri, ind) in resources {
        if !ind.is_a().contains(&inductive) {
            continue;
        }
        let Some(Value::Array(ctors)) = ind.get(&iri(wk::CTORS)) else {
            continue;
        };
        let ns = ind_iri.namespace().to_string();
        let ind_local = ind_iri.local_name().to_string();

        for ctor_val in ctors {
            let Value::Embedded(ctor) = ctor_val else {
                continue;
            };
            let Some(ctor_name) = ctor.get(&iri(wk::CTOR_NAME)).and_then(|v| v.as_str()) else {
                continue;
            };
            let class_iri_str = format!("{ns}{ind_local}-{ctor_name}");
            let Ok(class_iri) = Iri::parse(&class_iri_str) else {
                continue;
            };

            // Arguments become named properties on the class, in declaration order — which is
            // where argument ORDER lives now, so a value cannot get it wrong.
            let mut arg_prop_iris: Vec<Value> = Vec::new();
            if let Some(Value::Array(args)) = ctor.get(&iri(wk::ARG_TYPES)) {
                for (i, arg_val) in args.iter().enumerate() {
                    let Value::Embedded(arg) = arg_val else {
                        continue;
                    };
                    // `core:arg_name` is only a `recommends`; ESL's positional form emits none,
                    // so fall back to the `arg_N` convention the Julia mirror generator already
                    // defines.
                    let arg_name = arg
                        .get(&iri(wk::ARG_NAME))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("arg_{i}"));
                    let prop_iri_str = format!("{class_iri_str}-{arg_name}");
                    let Ok(prop_iri) = Iri::parse(&prop_iri_str) else {
                        continue;
                    };
                    arg_prop_iris.push(Value::String(prop_iri_str.clone()));
                    if resources.contains_key(&prop_iri) || out.contains_key(&prop_iri) {
                        continue;
                    }

                    let (data_type, class_type) =
                        arg_property_type(arg.get(&iri(wk::TYPE_NAME)), resources, parents);
                    let mut p = Resource::new(prop_iri.clone());
                    p.set(iri(wk::IS_A), is_a(wk::PROPERTY));
                    p.set(
                        iri(wk::SHORT_NAME),
                        Value::String(format!("{ind_local}-{ctor_name}-{arg_name}")),
                    );
                    p.set(
                        iri(wk::DESCRIPTION),
                        Value::String(format!(
                            "Argument `{arg_name}` of constructor `{ctor_name}` on \
                             `{ind_local}`. Derived from the inductive's core:ctors (D85 §6.1); \
                             not authored, and not editable except by editing the constructor."
                        )),
                    );
                    p.set(iri(wk::DATA_TYPE_PROP), Value::String(data_type.clone()));
                    if data_type == wk::VALUE_ARRAY {
                        // `core:value_array` conditionally requires `core:element_type`, and the
                        // CONSTRUCTOR DOES NOT SAY: `Units(core:value_array)` declares an array
                        // and nothing about its elements. `core:json` is element_type's escape
                        // hatch and records that absence honestly, rather than inventing
                        // `core:string` from the argument's name. Three constructor arguments in
                        // the tree are in this position, all in `statistics.esl`; tightening them
                        // is an ontology edit on those declarations, not a change here.
                        p.set(iri(wk::ELEMENT_TYPE), Value::String(wk::JSON.to_string()));
                    }
                    if let Some(ct) = class_type {
                        p.set(iri(wk::CLASS_TYPES), Value::Array(vec![Value::String(ct)]));
                    }
                    p.set(
                        iri(wk::DOMAIN),
                        Value::Array(vec![Value::String(class_iri_str.clone())]),
                    );
                    out.insert(prop_iri, p);
                }
            }

            if resources.contains_key(&class_iri) || out.contains_key(&class_iri) {
                continue;
            }
            let mut c = Resource::new(class_iri.clone());
            c.set(iri(wk::IS_A), is_a(wk::CLASS));
            c.set(
                iri(wk::SHORT_NAME),
                Value::String(format!("{ind_local}-{ctor_name}")),
            );
            c.set(
                iri(wk::DESCRIPTION),
                Value::String(format!(
                    "Constructor `{ctor_name}` of `{ind_local}`, as a class. A value built by \
                     this constructor carries it in `is_a`; its arguments are the properties \
                     this class requires. Derived from the inductive's core:ctors (D85 §6.1)."
                )),
            );
            c.set(
                iri(wk::PARENT_CLASSES),
                Value::Array(vec![Value::String(ind_iri.as_str().to_string())]),
            );
            if !arg_prop_iris.is_empty() {
                c.set(iri(wk::REQUIRES), Value::Array(arg_prop_iris));
            }
            out.insert(class_iri, c);
        }
    }
    out.into_values().collect()
}
