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

//! D46 §10 — axiom-as-Resource framework.
//!
//! Axioms are chain-resident `eigentt:Axiom` resources. Each carries an
//! `axiom_statement` property whose value is an EigenTT type expression
//! (encoded as `eigentt:TypeExpr` per D47). At environment-build time,
//! the kernel walks the chain, collects `eigentt:Axiom` resources, decodes
//! each statement back to an [`Exp`], type-checks it inhabits some sort
//! (i.e. is a well-formed type), and registers the IRI → type binding.
//!
//! Voiding a layer that introduces an axiom removes the axiom from the
//! resolved environment — the chain's audit-trail invariant for axioms
//! is the chain's existing tombstone / layer-resolution machinery, with
//! no special handling here.
//!
//! See `docs/design/d46-prop-universe-and-proof-irrelevance.md` §10.

use crate::layer::Layer;
use crate::nbe::term::Exp;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use std::collections::BTreeMap;
use std::sync::Arc;

/// IRI of the `eigentt:Axiom` class.
const AXIOM_CLASS_IRI: &str = "urn:eigenius:eigentt:Axiom";

/// IRI of the `eigentt:axiom_statement` property.
const AXIOM_STATEMENT_IRI: &str = "urn:eigenius:eigentt:axiom_statement";

/// An admitted axiom registered in [`AxiomEnv`]. Carries the IRI it was
/// committed under, the decoded type it inhabits, and (optionally) the
/// free-form justification note from the chain resource.
#[derive(Debug, Clone)]
pub struct AxiomEntry {
    pub iri: Iri,
    pub typ: Val,
    pub justification: Option<String>,
}

/// Environment of admitted axioms collected from a layer chain.
#[derive(Debug, Clone, Default)]
pub struct AxiomEnv {
    axioms: BTreeMap<Iri, AxiomEntry>,
}

impl AxiomEnv {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up an admitted axiom by IRI.
    pub fn get(&self, iri: &Iri) -> Option<&AxiomEntry> {
        self.axioms.get(iri)
    }

    /// Iterate over all admitted axioms in IRI order.
    pub fn iter(&self) -> impl Iterator<Item = (&Iri, &AxiomEntry)> {
        self.axioms.iter()
    }

    /// Number of admitted axioms.
    pub fn len(&self) -> usize {
        self.axioms.len()
    }

    /// Whether the environment is empty.
    pub fn is_empty(&self) -> bool {
        self.axioms.is_empty()
    }
}

/// Errors raised when building an axiom environment from a chain.
#[derive(Debug, Clone)]
pub enum AxiomEnvError {
    /// The axiom resource lacks an `axiom_statement` property value.
    MissingStatement(Iri),
    /// The `axiom_statement` value isn't a valid EigenTT type expression
    /// (decoding failed via [`crate::program::eigentt_type_mirror::decode_type`]).
    DecodeFailed { axiom: Iri, details: String },
    /// The decoded statement fails to type-check as a well-formed type.
    NotAWellFormedType { axiom: Iri, details: String },
}

impl std::fmt::Display for AxiomEnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AxiomEnvError::MissingStatement(iri) => {
                write!(f, "axiom `{iri}` has no axiom_statement value")
            }
            AxiomEnvError::DecodeFailed { axiom, details } => {
                write!(f, "axiom `{axiom}` statement decode failed: {details}")
            }
            AxiomEnvError::NotAWellFormedType { axiom, details } => {
                write!(
                    f,
                    "axiom `{axiom}` statement is not a well-formed type: {details}"
                )
            }
        }
    }
}

impl std::error::Error for AxiomEnvError {}

/// Walk a layer chain, collect every `eigentt:Axiom` resource, decode
/// its `axiom_statement` back to an [`Exp`], type-check it as a
/// well-formed type, and register the resulting `IRI → type` binding
/// in a fresh [`AxiomEnv`].
///
/// Errors short-circuit on the first malformed axiom — chain commits
/// that get past the D47 §5 validator should never produce errors
/// here, so an error indicates either a validator bug or a manually-
/// crafted bad resource bypassing normal commit. The caller may
/// choose to treat this as fatal.
pub fn build_axiom_env(layer: &Arc<Layer>) -> Result<AxiomEnv, AxiomEnvError> {
    let axiom_class = wk::iri(AXIOM_CLASS_IRI);
    let stmt_prop = wk::iri(AXIOM_STATEMENT_IRI);
    let justification_prop = wk::iri("urn:eigenius:eigentt:axiom_justification");

    let mut env = AxiomEnv::new();

    for (iri, resource) in layer.iter_all_resources() {
        if !is_axiom(&resource, &axiom_class) {
            continue;
        }
        let statement_value = resource
            .get(&stmt_prop)
            .ok_or_else(|| AxiomEnvError::MissingStatement(iri.clone()))?;
        let exp = crate::program::eigentt_type_mirror::decode_type(statement_value, layer)
            .map_err(|e| AxiomEnvError::DecodeFailed {
                axiom: iri.clone(),
                details: e.to_string(),
            })?;
        let typ = type_check_axiom_statement(&exp, layer).map_err(|e| {
            AxiomEnvError::NotAWellFormedType {
                axiom: iri.clone(),
                details: e,
            }
        })?;
        let justification = resource.get(&justification_prop).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        });
        env.axioms.insert(
            iri.clone(),
            AxiomEntry {
                iri: iri.clone(),
                typ,
                justification,
            },
        );
    }

    Ok(env)
}

fn is_axiom(resource: &Resource, axiom_class: &Iri) -> bool {
    resource.is_a().iter().any(|c| c == axiom_class)
}

/// Type-check the decoded axiom statement to verify it inhabits some
/// sort (i.e., is a well-formed type). Returns the inferred universe.
fn type_check_axiom_statement(exp: &Exp, layer: &Arc<Layer>) -> Result<Val, String> {
    use crate::nbe::check::{check_infer, CheckCtx};
    use crate::nbe::env::Rho;
    let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(layer));
    check_infer(&mut ctx, exp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::nbe::term::Patt;
    use crate::ontology::well_known::iri;
    use crate::program::eigentt_type_mirror::encode_type;

    /// Build a chain on top of the bootstrap chain (which already has
    /// `eigentt:Axiom` declared after Phase H) plus a top layer carrying
    /// one or more `eigentt:Axiom` resources.
    fn chain_with_axioms(axioms: Vec<(&str, Exp, Option<&str>)>) -> Arc<Layer> {
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut top = LayerBuilder::new("test_axioms_top", Some(head));
        for (axiom_iri, statement_exp, justification) in axioms {
            let mut r = Resource::new(iri(axiom_iri));
            r.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(AXIOM_CLASS_IRI))]),
            );
            let encoded = encode_type(&statement_exp).expect("encode statement");
            r.set(iri(AXIOM_STATEMENT_IRI), encoded);
            if let Some(j) = justification {
                r.set(
                    iri("urn:eigenius:eigentt:axiom_justification"),
                    Value::String(j.to_string()),
                );
            }
            top.add_resource(r).unwrap();
        }
        Arc::new(top.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn empty_env_when_no_axioms() {
        // Bootstrap chain has no axioms; env is empty.
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let env = build_axiom_env(&head).unwrap();
        assert!(
            env.is_empty(),
            "bootstrap chain should have no axioms by default"
        );
    }

    #[test]
    fn registers_propext_style_axiom() {
        // propext : ∀ {P : Prop}, ∀ {Q : Prop}, ((P → Q) × (Q → P)) → Id Prop P Q
        // Build the type, commit it as an axiom, verify the env.
        let p_var = || Exp::Var("P".to_string());
        let q_var = || Exp::Var("Q".to_string());
        let prop = || Exp::Sort(0);
        let p_to_q = Exp::Arrow(Box::new(p_var()), Box::new(q_var()));
        let q_to_p = Exp::Arrow(Box::new(q_var()), Box::new(p_var()));
        let iff = Exp::Times(Box::new(p_to_q), Box::new(q_to_p));
        let id_prop = Exp::Id(Box::new(prop()), Box::new(p_var()), Box::new(q_var()));
        let inner = Exp::Arrow(Box::new(iff), Box::new(id_prop));
        let propext = Exp::Pi(
            Patt::Var("P".to_string()),
            Box::new(prop()),
            Box::new(Exp::Pi(
                Patt::Var("Q".to_string()),
                Box::new(prop()),
                Box::new(inner),
            )),
        );

        let chain = chain_with_axioms(vec![(
            "urn:eigenius:test:propext",
            propext,
            Some("Propositional extensionality (D46 §10.1)"),
        )]);
        let env = build_axiom_env(&chain).unwrap();
        assert_eq!(env.len(), 1, "expected one registered axiom");
        let entry = env
            .get(&iri("urn:eigenius:test:propext"))
            .expect("propext should be registered");
        // propext's type should inhabit Prop (the impredicative Pi rule
        // collapses everything quantifying over Prop into Prop).
        assert!(
            matches!(entry.typ, Val::Sort(0)),
            "propext should inhabit Prop (Sort(0)); got {:?}",
            entry.typ
        );
        assert_eq!(
            entry.justification.as_deref(),
            Some("Propositional extensionality (D46 §10.1)")
        );
    }

    #[test]
    fn rejects_axiom_missing_statement() {
        // An eigentt:Axiom resource with no axiom_statement should be
        // rejected by build_axiom_env (it'd also fail the validator's
        // required-property check, but build_axiom_env defends in depth).
        let head = Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut top = LayerBuilder::new("missing_stmt", Some(head));
        let mut r = Resource::new(iri("urn:eigenius:test:no_stmt_axiom"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(AXIOM_CLASS_IRI))]),
        );
        top.add_resource(r).unwrap();
        let chain = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        let err = build_axiom_env(&chain).unwrap_err();
        assert!(matches!(err, AxiomEnvError::MissingStatement(_)));
    }

    #[test]
    fn registers_id_axiom_inhabiting_prop() {
        // Trivial axiom: ∀ (x : 1). ∀ (y : 1). Id 1 x y
        // — Id over Var-references (no term-level literals needed; D47 is
        // type-level only). Well-formed, inhabits Prop via the impredicative
        // rule (codomain is Id which lives in Prop).
        let id_xy = Exp::Id(
            Box::new(Exp::One),
            Box::new(Exp::Var("x".to_string())),
            Box::new(Exp::Var("y".to_string())),
        );
        let inner_pi = Exp::Pi(
            Patt::Var("y".to_string()),
            Box::new(Exp::One),
            Box::new(id_xy),
        );
        let outer_pi = Exp::Pi(
            Patt::Var("x".to_string()),
            Box::new(Exp::One),
            Box::new(inner_pi),
        );
        let chain = chain_with_axioms(vec![("urn:eigenius:test:trivial_id_axiom", outer_pi, None)]);
        let env = build_axiom_env(&chain).unwrap();
        let entry = env.get(&iri("urn:eigenius:test:trivial_id_axiom")).unwrap();
        assert!(matches!(entry.typ, Val::Sort(0)));
    }
}
