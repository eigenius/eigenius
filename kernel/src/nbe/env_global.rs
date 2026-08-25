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

/// Memo for [`Env::lookup`], keyed by layer then IRI.
///
/// **Phase B creates the need for this, not Phase D.** §4.3 deferred the memo on
/// the argument that `check` was its only consumer and already caches through
/// `type_cache`. De-inlining changes the arithmetic: a lookup for an inductive
/// runs `resolve_class_type` → `resolve_inductive_type`, which decodes params,
/// indices, and every constructor type. `RESOLVE_MEMO` does not cover that — it
/// caches `Layer::resolve`, the resource fetch beneath the decode. While the
/// declaration is inlined in the term the decode happens **once**, at
/// `resolve_const_ref`; de-inlined, it happens once per occurrence per
/// evaluation, in the evaluator's inner loop.
///
/// Shape follows [`crate::validation::ClassFieldsScope`] (D78 Phase D):
/// thread-local, RAII scope, `BTreeMap` at both levels, no-op when no scope is
/// installed. Soundness is `ResolveMemoScope`'s — correct while the chain does
/// not change, which holds across a pass over immutable `Arc<Layer>`s.
///
/// **Boundedness differs from `CLASS_FIELDS_MEMO`'s and is the cost §4.2 flags.**
/// That memo is keyed by class — ~894 of them against millions of instances. This
/// one is keyed by every IRI looked up, including those that resolve to
/// [`Global::Absent`]. The key set is bounded by the distinct IRIs *appearing in
/// terms*, not by chain size, but that is an argument and not a measurement; the
/// reseed measures it.
type GlobalMemo =
    std::collections::BTreeMap<crate::layer::LayerId, std::collections::BTreeMap<Iri, Global>>;

thread_local! {
    static GLOBAL_MEMO: std::cell::RefCell<Option<GlobalMemo>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard installing a [`GLOBAL_MEMO`] for its lifetime. Nesting-safe: the
/// previous memo is restored on drop.
pub struct GlobalMemoScope {
    prev: Option<GlobalMemo>,
}

impl GlobalMemoScope {
    pub fn new() -> Self {
        let prev = GLOBAL_MEMO.with(|m| m.borrow_mut().replace(GlobalMemo::new()));
        Self { prev }
    }

    /// Number of memoized entries, for the boundedness measurement. `None` when
    /// no scope is installed.
    pub fn entry_count() -> Option<usize> {
        GLOBAL_MEMO.with(|m| {
            m.borrow()
                .as_ref()
                .map(|memo| memo.values().map(|per_layer| per_layer.len()).sum())
        })
    }
}

impl Default for GlobalMemoScope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GlobalMemoScope {
    fn drop(&mut self) {
        GLOBAL_MEMO.with(|m| *m.borrow_mut() = self.prev.take());
    }
}

/// `Γ_env` — the global environment of a typing judgment.
///
/// Cheap to clone: an `Option<Arc<Layer>>`.
#[derive(Debug, Clone, Default)]
pub struct Env {
    layer: Option<Arc<Layer>>,
    /// Declarations in scope that the chain does not hold — nanoda's
    /// `temp_declars` (`references/nanoda_lib/src/env.rs:221`), consulted before
    /// the committed ones (`:259`).
    ///
    /// A declaration being checked is in scope for its own constructor types
    /// before it is committed anywhere. The stub existed to paper over exactly
    /// that: it stood in for a declaration that was not yet resolvable. Naming it
    /// as a `Const` moves the problem to the environment, which is where it
    /// belongs and where nanoda already solved it.
    locals: std::collections::BTreeMap<Iri, Arc<InductiveDecl>>,
}

impl Env {
    /// The environment that knows nothing. Every lookup is
    /// [`Global::Absent`].
    pub fn empty() -> Self {
        Self {
            layer: None,
            locals: Default::default(),
        }
    }

    /// The environment a layer chain provides.
    pub fn of(layer: Arc<Layer>) -> Self {
        Self {
            layer: Some(layer),
            locals: Default::default(),
        }
    }

    /// This environment plus one declaration the chain does not hold.
    ///
    /// Shadows both the chain and the intrinsics, per nanoda's ordering: a
    /// declaration in progress is the one in scope.
    pub fn declaring(mut self, decl: Arc<InductiveDecl>) -> Self {
        self.locals.insert(decl.iri.clone(), decl);
        self
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
        self.layer.is_none() && self.locals.is_empty()
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
        if let Some(decl) = self.locals.get(iri) {
            return Global::Inductive(Arc::clone(decl));
        }
        let Some(layer) = self.layer.as_ref() else {
            return Self::intrinsic(iri).unwrap_or(Global::Absent);
        };
        let key = layer.id().clone();
        if let Some(hit) = GLOBAL_MEMO.with(|m| {
            m.borrow()
                .as_ref()
                .and_then(|memo| memo.get(&key)?.get(iri).cloned())
        }) {
            return hit;
        }
        let computed = self.lookup_uncached(layer, iri);
        GLOBAL_MEMO.with(|m| {
            if let Some(memo) = m.borrow_mut().as_mut() {
                memo.entry(key)
                    .or_default()
                    .insert(iri.clone(), computed.clone());
            }
        });
        computed
    }

    /// The declarations the kernel provides itself, which no layer declares.
    ///
    /// `core:List` is built in `nbe::term::list_decl` and is **not** a chain
    /// resource, so a chain lookup for it returns nothing. `decode_type`'s
    /// `ConstRef` arm has always special-cased it; the environment did not, which
    /// is one of the divergences D76 exists to remove — de-inlining `list_decl`'s
    /// own constructor types is what surfaced it, as a `Const` naming `List`
    /// evaluating to a neutral.
    ///
    /// Answered by *every* environment, the empty one included: these are not
    /// chain content, so "knows nothing" means nothing about the chain. A
    /// declaration in progress ([`Env::declaring`]) still shadows them, per
    /// nanoda's `temp_declars` ordering. `core:Option` is deliberately **not** here — it *is* a
    /// chain resource, and taking the kernel's copy would hide any disagreement
    /// between the two rather than surface it
    /// (`the_chain_and_the_kernel_agree_about_option`).
    fn intrinsic(iri: &Iri) -> Option<Global> {
        if iri.as_str() == crate::ontology::well_known::LIST {
            return Some(Global::Inductive(crate::nbe::term::list_decl()));
        }
        None
    }

    fn lookup_uncached(&self, layer: &Arc<Layer>, iri: &Iri) -> Global {
        if let Some(g) = Self::intrinsic(iri) {
            return g;
        }
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
                // In *this* environment, not an empty one: a definition body may
                // name other declarations, and an env-less eval would leave each
                // as a neutral.
                Ok(body) => match crate::nbe::eval::eval_env(
                    &body,
                    &crate::nbe::env::Rho::Nil,
                    &Env::of(Arc::clone(layer)),
                ) {
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
    /// A layer plus three IRIs the environment classifies differently: an
    /// inductive, a class, and one that resolves to nothing.
    fn memo_fixture() -> (Arc<Layer>, Iri, Iri, Iri) {
        let b = LayerBuilder::new("memo-test", Some(core()));
        let layer = Arc::new(b.build(LayerStorage::in_memory()));
        (
            layer,
            i("urn:eigenius:core:Level"),
            i(wk::CLASS),
            i("urn:t:not-there"),
        )
    }

    #[test]
    fn the_memo_answers_identically_and_is_installed_by_scope() {
        // The memo must not change what the environment says, only how often it
        // computes it. Same lookups, once outside a scope and once inside.
        let (layer, inductive, klass, missing) = memo_fixture();
        let env = Env::of(layer);

        let uncached: Vec<String> = [&inductive, &klass, &missing]
            .iter()
            .map(|i| format!("{:?}", env.lookup(i)))
            .collect();

        assert_eq!(
            GlobalMemoScope::entry_count(),
            None,
            "no scope installed → no memo"
        );
        let _scope = GlobalMemoScope::new();
        assert_eq!(GlobalMemoScope::entry_count(), Some(0));

        let cached: Vec<String> = [&inductive, &klass, &missing]
            .iter()
            .map(|i| format!("{:?}", env.lookup(i)))
            .collect();
        assert_eq!(uncached, cached, "the memo changes no answer");
        assert_eq!(
            GlobalMemoScope::entry_count(),
            Some(3),
            "including the Absent one — that is the boundedness cost \u{a7}4.2 flags"
        );

        // A second round of the same lookups adds no entries.
        for i in [&inductive, &klass, &missing] {
            let _ = env.lookup(i);
        }
        assert_eq!(GlobalMemoScope::entry_count(), Some(3));
    }

    #[test]
    fn the_memo_scope_nests_and_restores() {
        let (layer, inductive, _klass, _missing) = memo_fixture();
        let env = Env::of(layer);
        let _outer = GlobalMemoScope::new();
        let _ = env.lookup(&inductive);
        assert_eq!(GlobalMemoScope::entry_count(), Some(1));
        {
            let _inner = GlobalMemoScope::new();
            assert_eq!(
                GlobalMemoScope::entry_count(),
                Some(0),
                "inner starts empty"
            );
        }
        assert_eq!(
            GlobalMemoScope::entry_count(),
            Some(1),
            "the outer memo is restored on drop"
        );
    }

    #[test]
    fn an_effect_free_evaluation_still_resolves_a_name_through_its_environment() {
        // **The defect D76 Phase B's audit found.** `EvalCtx` filed chain access
        // under the effect capability, so the type checker's evaluator — always
        // effect-free — had no environment. A de-inlined `Exp::Const` would
        // evaluate to a neutral instead of the declaration it names, and every
        // inductive reference would stop being a type the moment it stopped
        // being inlined.
        use crate::nbe::env::Rho;
        use crate::nbe::eval::{eval_ctx, EvalCtx};
        use crate::nbe::term::Exp;

        let (layer, inductive, _, _) = memo_fixture();
        let reference = Exp::Const(inductive.clone(), Vec::new());

        let without = eval_ctx(&reference, &Rho::Nil, &EvalCtx::pure()).expect("eval");
        assert!(
            matches!(without, Val::Nt(crate::nbe::val::Neut::Const(..))),
            "with no environment the name is inert — a neutral, not an error: {without:?}"
        );

        let with_env =
            eval_ctx(&reference, &Rho::Nil, &EvalCtx::in_env(Env::of(layer))).expect("eval");
        match with_env {
            Val::InductiveType { decl, .. } => assert_eq!(decl.iri, inductive),
            other => panic!("an effect-free context with an environment must resolve: {other:?}"),
        }
    }

    #[test]
    fn effects_and_the_environment_are_independent() {
        // The shape claim behind the fix: `hooks` is the capability, `env` is not.
        // An effect-free context can carry an environment, which the old enum
        // could not express.
        use crate::nbe::eval::EvalCtx;
        let (layer, _, _, _) = memo_fixture();

        let plain = EvalCtx::pure();
        assert!(plain.hooks().is_none() && plain.env().is_empty());

        let environed = EvalCtx::in_env(Env::of(layer));
        assert!(
            environed.hooks().is_none() && !environed.env().is_empty(),
            "effect-free, yet it has an environment"
        );
    }
    #[test]
    fn the_kernel_s_own_declarations_are_in_every_environment() {
        // `core:List` is built in `nbe::term::list_decl` and is not a chain
        // resource. Before this, a chain lookup returned `Absent` and a `Const`
        // naming it evaluated to a neutral — which broke felicity filtering the
        // moment `list_decl`'s constructor types stopped inlining the stub.
        let list = i(crate::ontology::well_known::LIST);
        for (label, env) in [("empty", Env::empty()), ("chain", with(vec![]))] {
            match env.lookup(&list) {
                Global::Inductive(decl) => assert_eq!(
                    decl.iri, list,
                    "{label}: the intrinsic declaration answers for its own IRI"
                ),
                other => panic!("{label}: core:List must resolve, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_chain_and_the_kernel_agree_about_option() {
        // `core:Option` exists twice — as a chain resource and as
        // `nbe::term::option_decl`. The environment answers from the chain, so
        // that a disagreement shows up here rather than being papered over by
        // preferring the kernel's copy.
        let chain_decl = match with(vec![]).lookup(&i(crate::ontology::well_known::OPTION)) {
            Global::Inductive(d) => d,
            other => panic!("core:Option is a chain inductive, got {other:?}"),
        };
        let kernel_decl = crate::nbe::term::option_decl();

        assert_eq!(chain_decl.iri, kernel_decl.iri);
        assert_eq!(
            chain_decl.params.len(),
            kernel_decl.params.len(),
            "parameter counts differ: chain {:?} vs kernel {:?}",
            chain_decl.params,
            kernel_decl.params
        );
        let names = |d: &InductiveDecl| d.ctors.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&chain_decl),
            names(&kernel_decl),
            "constructor sets differ between the chain's Option and the kernel's"
        );
    }
}
