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

//! Decidable / FIBER input-resource marshaling.
//!
//! Both surfaces — `kernel::nbe::eval::try_institution_decide` (kernel-internal
//! NbE Decidable) and `kernel::query::evaluate::try_dispatch_decidable`
//! (EigenQL Decidable) — synthesise a single typed input resource from
//! positional args before calling `Institution::query`. The work is
//! the same in both sites:
//!
//! 1. Resolve the QueryClass's declared input class on the layer.
//! 2. Read its `requires` list, dropping kernel-managed `is_a` /
//!    `short_name` (the same exclusion the FIBER type-checker uses,
//!    `query/type_check.rs`).
//! 3. Verify positional arg count matches the typed-required count.
//! 4. Populate each typed required property with the matching
//!    positional arg, dereferencing IRI-shaped values into embedded
//!    resources for `core:resource`-typed properties (so the
//!    institution's mirror codec sees a fully-embedded map instead of
//!    a bare IRI string).
//!
//! Centralising this logic here keeps both Decidable paths in lockstep
//! — adding a new marshaling rule (e.g. `core:resource_array` element
//! deref) lands in one place rather than two.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;

/// Errors surfaced by [`marshal_decidable_input`]. Callers wrap into
/// their site-specific error type (`QueryError`, `EvalError`, …).
#[derive(Debug, Clone)]
pub enum MarshalError {
    /// The QueryClass's declared input class doesn't resolve in the
    /// layer chain. Always a kernel-state error — the chain is
    /// missing a declaration the QueryClass referenced.
    UnresolvedInputClass(Iri),
    /// Positional arg count doesn't match the input class's typed-
    /// required count (excluding kernel-managed `is_a`/`short_name`).
    ArityMismatch {
        input_class: Iri,
        expected: usize,
        got: usize,
    },
    /// An IRI-shaped arg value targeting a `core:resource` property
    /// doesn't resolve in the layer chain.
    UnresolvedReference { property: Iri, target: Iri },
}

impl std::fmt::Display for MarshalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvedInputClass(iri) => write!(
                f,
                "input class `{iri}` does not resolve in the layer chain — \
                 cannot marshal positional args onto typed properties"
            ),
            Self::ArityMismatch {
                input_class,
                expected,
                got,
            } => write!(
                f,
                "input class `{input_class}` has {expected} typed required properties \
                 (excluding kernel-managed) but {got} positional args were supplied; \
                 the user-facing call signature must match the declared input shape"
            ),
            Self::UnresolvedReference { property, target } => write!(
                f,
                "FIBER param `{property}`: resource `{target}` does not resolve in \
                 the layer chain"
            ),
        }
    }
}

impl std::error::Error for MarshalError {}

/// Marshal positional args onto a synthetic input resource of the
/// QueryClass's declared input class. See module docstring.
pub fn marshal_decidable_input(
    input_class_iri: &Iri,
    args: &[Value],
    layer: &Layer,
) -> Result<Resource, MarshalError> {
    let input_class = layer
        .resolve(input_class_iri)
        .ok_or_else(|| MarshalError::UnresolvedInputClass(input_class_iri.clone()))?;
    let typed_required = required_typed_properties(&input_class);
    if typed_required.len() != args.len() {
        return Err(MarshalError::ArityMismatch {
            input_class: input_class_iri.clone(),
            expected: typed_required.len(),
            got: args.len(),
        });
    }

    let mut input = Resource::new_embedded();
    input.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(input_class_iri.as_str().into())]),
    );

    for (prop_iri, arg) in typed_required.iter().zip(args.iter()) {
        let marshaled = embed_typed_resource_arg(prop_iri, arg.clone(), layer)?;
        input.set(prop_iri.clone(), marshaled);
    }

    Ok(input)
}

/// Read an input class's `requires` list and return the property IRIs
/// the user must supply positionally. Kernel-managed `is_a` (auto-
/// stamped from the input class IRI) and `short_name` (chain
/// bookkeeping irrelevant to a transient Decidable input) are
/// excluded — same exclusion the FIBER type-checker applies. Order
/// matches `requires` declaration order, which is also the order
/// the mirror generator emits typed struct fields.
pub fn required_typed_properties(input_class: &Resource) -> Vec<Iri> {
    let requires_iri = Iri::parse(wk::REQUIRES).expect("well-known IRI");
    let raw = match input_class.get(&requires_iri) {
        Some(Value::Array(arr)) => arr,
        _ => return Vec::new(),
    };
    raw.iter()
        .filter_map(|v| match v {
            Value::String(s) => Iri::parse(s).ok(),
            Value::ResourceRef(i) => Some(i.clone()),
            _ => None,
        })
        .filter(|iri| iri.as_str() != wk::IS_A && iri.as_str() != wk::SHORT_NAME)
        .collect()
}

/// Per-property marshaling: when the target property declares
/// `data_type: core:resource`, dereference IRI-shaped values
/// (`Value::String("urn:...")` or `Value::ResourceRef`) into
/// embedded resources so the institution's mirror decoder sees a
/// fully-embedded map. Other property shapes pass through unchanged.
pub fn embed_typed_resource_arg(
    param_iri: &Iri,
    value: Value,
    layer: &Layer,
) -> Result<Value, MarshalError> {
    let Some(prop_def) = layer.resolve(param_iri) else {
        return Ok(value);
    };
    let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).expect("well-known IRI");
    let dt = match prop_def.get(&dt_iri) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::ResourceRef(i)) => i.as_str().to_string(),
        _ => return Ok(value),
    };
    match dt.as_str() {
        wk::RESOURCE => deref_resource_value(value, param_iri, layer),
        wk::RESOURCE_ARRAY => match value {
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(deref_resource_value(item, param_iri, layer)?);
                }
                Ok(Value::Array(out))
            }
            other => Ok(other),
        },
        _ => Ok(value),
    }
}

fn deref_resource_value(
    value: Value,
    param_iri: &Iri,
    layer: &Layer,
) -> Result<Value, MarshalError> {
    match value {
        Value::Embedded(r) => Ok(Value::Embedded(r)),
        Value::ResourceRef(iri) => deref_iri_to_embedded(&iri, param_iri, layer),
        Value::String(s) => match Iri::parse(&s) {
            Ok(iri) => deref_iri_to_embedded(&iri, param_iri, layer),
            Err(_) => Ok(Value::String(s)),
        },
        other => Ok(other),
    }
}

fn deref_iri_to_embedded(iri: &Iri, param_iri: &Iri, layer: &Layer) -> Result<Value, MarshalError> {
    match layer.resolve(iri) {
        Some(r) => Ok(Value::Embedded(Box::new((*r).clone()))),
        None => Err(MarshalError::UnresolvedReference {
            property: param_iri.clone(),
            target: iri.clone(),
        }),
    }
}

/// Walk a resource and dereference every IRI-shaped value sitting in
/// a property whose `data_type` is `core:resource` or
/// `core:resource_array` — recursively, so nested embedded resources
/// also have their IRI references inlined. Used by the AutoOnLoad /
/// post-translation dispatch path before the input resource is
/// serialised onto the wire: the worker's mirror decoders expect
/// embedded maps in resource-typed fields, not chain-bound IRI
/// strings.
///
/// Properties whose `data_type` is something else (`core:inductive`,
/// `core:json`, `core:value_array`, primitives, …) pass through
/// unchanged. Inductive payloads (`Value::Json` for FormulaTerm,
/// `Verdict`, etc.) are *not* walked — chain-bound IRI strings
/// inside an inductive payload don't denote chain references the
/// way `class_types: [...]`-typed properties do, and walking them
/// would corrupt FormulaTerm trees that legitimately carry IRI
/// strings as `OpRef` payloads.
pub fn embed_typed_resource_refs_recursively(
    resource: Resource,
    layer: &Layer,
) -> Result<Resource, MarshalError> {
    let id = resource.id().cloned();
    let mut out = match id {
        Some(iri) => Resource::new(iri),
        None => Resource::new_embedded(),
    };
    for (prop_iri, value) in resource.properties() {
        // Skip kernel-managed metadata properties — `core:is_a`,
        // `core:short_name`, etc. live in the `urn:eigenius:core:`
        // namespace and carry chain-bookkeeping shapes (class refs,
        // requires lists, …) the mirror decoder doesn't see.
        // Dereferencing `core:is_a` would try to resolve every class
        // IRI as a domain resource and fail noisily for classes the
        // chain references but doesn't define inline. Same convention
        // the mirror generator's `is_core_meta_iri` filter applies
        // (D29 §11; user data uses non-core namespaces).
        if prop_iri.as_str().starts_with("urn:eigenius:core:") {
            out.set(prop_iri.clone(), value.clone());
            continue;
        }
        let new_value = embed_typed_resource_arg(prop_iri, value.clone(), layer)?;
        let recursed = recurse_into_embedded(new_value, layer)?;
        out.set(prop_iri.clone(), recursed);
    }
    Ok(out)
}

fn recurse_into_embedded(value: Value, layer: &Layer) -> Result<Value, MarshalError> {
    match value {
        Value::Embedded(r) => {
            let recursed = embed_typed_resource_refs_recursively(*r, layer)?;
            Ok(Value::Embedded(Box::new(recursed)))
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(recurse_into_embedded(item, layer)?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other),
    }
}
