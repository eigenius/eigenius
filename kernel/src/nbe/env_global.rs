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

//! D76 Phase C — the typing environment, `Γ_env`.
//!
//! The chain of layers *is* the environment of the judgment `Γ_env; Γ ⊢ e : T`
//! (D75 §4). This module is the interface the judgment holds, replacing three
//! divergent arrangements (D75 §2): a partial one on `check`
//! (`CheckCtx.layer: Option<Arc<Layer>>` + `CheckHooks::resolve_class`, classes
//! only), none on `eval`, and none on `eq_nf`.
//!
//! **Emptiness is not optionality.** An environment is a component of the
//! judgment, so a caller with nothing to resolve against holds an *empty* one
//! rather than `None`. A caller cannot ask "do I have a layer" — only look up
//! and get [`Global::Absent`]. `CheckCtx`'s `Option` and its *"no layer access
//! in pure check mode"* error are what let the three surfaces diverge.

use crate::layer::Layer;
use crate::nbe::term::InductiveDecl;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use std::sync::Arc;

/// What the environment knows about an IRI.
///
/// One variant per kind D76 §5 names, each carrying what *its* consumer needs.
/// The distinction that matters is [`Definition`](Global::Definition) versus
/// [`Constraint`](Global::Constraint): both resolve to a `Val`, and only the
/// first may be unfolded during conversion. A single `Transparent` variant — as
/// an earlier draft of §4 had — would make that a caller convention instead of a
/// type distinction.
#[derive(Debug, Clone)]
pub enum Global {
    /// A **transparent** definition: unfolds to this during conversion.
    ///
    /// Transparency is per-declaration, not per-kind: a `Definition` resource
    /// carrying `opaque = true` is rigid and resolves to [`Global::Axiom`]
    /// instead (`program::eigentt_type_mirror::definition_is_opaque`, D66 / #95).
    /// Returning `Definition` for one of those would silently unfold a
    /// definition the author marked rigid.
    Definition(Val),
    /// A class. Resolves to its record, and **never unfolds in conversion**
    /// (D75 §8 Q2): 749 of 894 shipped classes have identical (empty) field
    /// sets, so unfolding would make class identity structural. `check` still
    /// needs the record to project a field.
    Constraint(Val),
    /// Postulated, or a definition marked opaque. Nothing to unfold.
    Axiom,
    /// An inductive, carrying the declaration `iota_reduce` needs.
    ///
    /// This variant is why `eval` is a consumer of the environment at all
    /// (§1, Q1 correction): `iota_reduce_impl` reads `decl.ctors` to reduce, and
    /// needs no environment today only because the declaration is inlined in the
    /// term.
    Inductive(Arc<InductiveDecl>),
    /// Not resolvable in this environment.
    Absent,
}

/// `Γ_env` — the global environment of a typing judgment.
///
/// Cheap to clone: an `Option<Arc<Layer>>`.
#[derive(Debug, Clone, Default)]
pub struct Env {
    layer: Option<Arc<Layer>>,
}

impl Env {
    /// The environment that knows nothing. Every lookup is
    /// [`Global::Absent`].
    pub fn empty() -> Self {
        Self { layer: None }
    }

    /// The environment a layer chain provides.
    pub fn of(layer: Arc<Layer>) -> Self {
        Self { layer: Some(layer) }
    }

    /// The layer this environment reads, for consumers that still need it
    /// directly during the migration.
    pub fn layer(&self) -> Option<&Arc<Layer>> {
        self.layer.as_ref()
    }

    /// Is this the empty environment? For diagnostics only — a *judgment* must
    /// not branch on this, or the "emptiness is not optionality" property is
    /// lost.
    pub fn is_empty(&self) -> bool {
        self.layer.is_none()
    }

    /// Is `sub` a declared subclass of `sup`?
    ///
    /// The **nominal** relation — `subclass_of`, walked transitively — as
    /// distinct from D78's structural `entails`. They are the two halves of D75
    /// §8 Q10, and D78 §8a argues the nominal one is load-bearing: 749 of 894
    /// shipped classes have identical field sets, so structure cannot tell them
    /// apart and only the declared relation can.
    ///
    /// On `Env` rather than reached through `layer()` because it is a fact about
    /// declarations that the judgment consults — three `check` sites use it for
    /// subsumption today.
    pub fn is_subclass_of(&self, sub: &Iri, sup: &Iri) -> bool {
        self.layer
            .as_ref()
            .is_some_and(|l| l.is_subclass_of(sub, sup))
    }

    /// What does this environment know about `iri`?
    ///
    /// **Kind decides the default; a definition may override it.** The order of
    /// the checks below mirrors `decode_type`'s
    /// (`program::eigentt_type_mirror`), deliberately: that function is where
    /// the distinction is made today, and a lookup that classified differently
    /// would silently disagree with every already-decoded term.
    pub fn lookup(&self, iri: &Iri) -> Global {
        let Some(layer) = self.layer.as_ref() else {
            return Global::Absent;
        };
        let Some(resource) = layer.resolve(iri) else {
            return Global::Absent;
        };
        let classes = resource.is_a();
        let is = |s: &str| classes.iter().any(|c| c.as_str() == s);

        // A postulate.
        if is("urn:eigenius:eigentt:Axiom") {
            return Global::Axiom;
        }

        // A definition — transparent unless flagged. This is the per-declaration
        // override (D66, #95); classifying a rigid definition as
        // `Definition(..)` would unfold something the author marked opaque.
        if is("urn:eigenius:eigentt:Definition") {
            let opaque = matches!(
                resource.get(&iri_of("urn:eigenius:eigentt:definition_opaque")),
                Some(crate::ontology::resource::Value::Boolean(true))
            );
            if opaque {
                return Global::Axiom;
            }
            return match crate::program::eigentt_type_mirror::decode_type(
                &resource
                    .get(&iri_of("urn:eigenius:eigentt:definition_body"))
                    .cloned()
                    .unwrap_or(crate::ontology::resource::Value::Boolean(false)),
                layer,
            ) {
                Ok(body) => match crate::nbe::eval::eval(&body, &crate::nbe::env::Rho::Nil) {
                    Ok(v) => Global::Definition(v),
                    Err(_) => Global::Absent,
                },
                Err(_) => Global::Absent,
            };
        }

        // An inductive — carries the declaration `iota_reduce` needs.
        if is(crate::ontology::well_known::INDUCTIVE_TYPE) {
            return match crate::program::ground::resolve_class_type(iri, layer) {
                Ok(Val::InductiveType { decl, .. }) => Global::Inductive(decl),
                _ => Global::Absent,
            };
        }

        // A class — resolves to its record, and never unfolds (Q2).
        if is(crate::ontology::well_known::CLASS) {
            return match crate::program::ground::resolve_class_type(iri, layer) {
                Ok(v) => Global::Constraint(v),
                Err(_) => Global::Absent,
            };
        }

        Global::Absent
    }
}

fn iri_of(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::resource::{Resource, Value};
    use crate::ontology::well_known as wk;

    fn i(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn core() -> Arc<Layer> {
        let json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut b = LayerBuilder::new("core", None);
        for r in crate::ontology::eigon_json::parse_document(json).unwrap() {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    }

    fn with(extra: Vec<Resource>) -> Env {
        let mut b = LayerBuilder::new("env-test", Some(core()));
        for r in extra {
            b.add_resource(r).unwrap();
        }
        Env::of(Arc::new(b.build(LayerStorage::in_memory())))
    }

    fn definition(id: &str, opaque: bool) -> Resource {
        let mut r = Resource::new(i(id));
        r.set(
            i(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(i(
                "urn:eigenius:eigentt:Definition",
            ))]),
        );
        // Body: `Sort(1)` in D47 form — a term that decodes and evaluates.
        r.set(
            i("urn:eigenius:eigentt:definition_body"),
            Value::Json(serde_json::json!({
                "ctor": "Sort",
                "args": [{"ctor": "Succ", "args": [{"ctor": "Zero", "args": []}]}]
            })),
        );
        if opaque {
            r.set(
                i("urn:eigenius:eigentt:definition_opaque"),
                Value::Boolean(true),
            );
        }
        r
    }

    #[test]
    fn the_empty_environment_knows_nothing() {
        // Emptiness is not optionality: a caller with nothing to resolve gets
        // `Absent`, not an error and not a `None` it has to branch on.
        let e = Env::empty();
        assert!(matches!(e.lookup(&i(wk::CLASS)), Global::Absent));
        assert!(e.is_empty());
    }

    #[test]
    fn a_class_resolves_to_a_constraint_not_a_definition() {
        // The distinction that keeps Q2's opacity a type property rather than a
        // caller convention.
        let e = with(vec![]);
        match e.lookup(&i(wk::CLASS)) {
            Global::Constraint(_) => {}
            other => panic!("core:Class must be a Constraint, got {other:?}"),
        }
    }

    #[test]
    fn an_inductive_carries_the_declaration_iota_needs() {
        // Why `eval` is a consumer at all — the Q1 correction in D76 §1.
        let e = with(vec![]);
        match e.lookup(&i("urn:eigenius:core:Level")) {
            Global::Inductive(decl) => {
                assert!(
                    !decl.ctors.is_empty(),
                    "the declaration must carry its ctors, or iota cannot reduce"
                );
            }
            other => panic!("core:Level is an inductive, got {other:?}"),
        }
    }

    #[test]
    fn a_transparent_definition_unfolds_and_an_opaque_one_does_not() {
        // **The per-declaration override.** Same body, same kind; only the
        // `definition_opaque` flag differs. Classifying the rigid one as
        // `Definition(..)` would unfold something the author marked opaque
        // (D66, #95).
        let e = with(vec![
            definition("urn:t:clear", false),
            definition("urn:t:rigid", true),
        ]);
        assert!(
            matches!(e.lookup(&i("urn:t:clear")), Global::Definition(_)),
            "a transparent definition must unfold"
        );
        assert!(
            matches!(e.lookup(&i("urn:t:rigid")), Global::Axiom),
            "an opaque definition is rigid — it must classify as Axiom, not Definition"
        );
    }

    #[test]
    fn the_nominal_subclass_relation_is_on_the_environment() {
        // The counterpart to D78's structural `entails` — the two halves of Q10.
        // `core:Class` and `core:Property` are unrelated by `subclass_of`.
        let e = with(vec![]);
        assert!(
            !e.is_subclass_of(&i(wk::PROPERTY), &i(wk::CLASS)),
            "unrelated classes must not be in the relation"
        );
        // Reflexivity is the layer's business, not asserted here; what matters
        // is that the empty environment answers rather than panicking.
        assert!(
            !Env::empty().is_subclass_of(&i(wk::CLASS), &i(wk::CLASS)),
            "the empty environment knows no relation, and says so"
        );
    }

    #[test]
    fn an_unknown_iri_is_absent_rather_than_an_error() {
        let e = with(vec![]);
        assert!(matches!(e.lookup(&i("urn:t:nope")), Global::Absent));
    }
}
