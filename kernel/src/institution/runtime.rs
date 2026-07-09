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

//! institution runtime — the trait an institution implements and
//! the registry the kernel dispatches through.
//!
//! [`Institution`] is the minimal trait surface from D14 §8: two
//! mandatory boundary methods (`extract_typed`, `reify`) and one
//! optional reasoning method (`query`). Institutions whose QueryClasses
//! are all Component-implemented never see `query` called.
//!
//! [`InstitutionRuntime`] is a typed registry of trait objects keyed
//! by institution IRI. The kernel uses [`registry::InstitutionIndex`]
//! to resolve a procedure / class / comorphism IRI to a *declaring
//! institution IRI*, then this runtime to dispatch the call to the
//! actual implementation.
//!
//! M3 of the implementation plan: trait + runtime + tests, no dispatch sites yet.
//! M5–M7 wire `Exp::InstitutionInvoke`, `Exp::NativeDecide`, and
//! AutoOnLoad Load handling onto this surface.

use crate::context::ExecutionContext;
use crate::institution::error::InstitutionError;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::collections::BTreeMap;

/// Outcome of an `Institution::query` dispatch.
///
/// Carries the result Resource the institution produced plus an
/// optional substrate-captured `partial_invocation` Resource describing
/// the dispatch (D26 §5.5 — language, image_digest, started_at,
/// completed_at, numerical_metadata, dispatched_to). The kernel's
/// commit pipeline folds the partial into a full `RuntimeInvocation`
/// resource by stamping the IRIs only the kernel knows (`script` ←
/// signature_iri, `environment` ← env_iri, `inputs` ← gated resource
/// IRI, `output` ← Verdict IRI) and commits all three resources
/// (gated subject + Verdict + RuntimeInvocation) in the same kernel
/// transaction per [D31 §6.3](../../docs/design/d31-external-institution-lifecycle.md#63-verdict-commit-semantics).
///
/// Only external-runtime institutions populate the partial; in-process
/// institutions return `partial_invocation: None` because
/// their dispatch happens entirely inside the kernel/host process and
/// the kernel records its own trace via the program-level trace store
/// rather than as a chain-committed `RuntimeInvocation`.
#[derive(Debug, Clone)]
pub struct QueryOutcome {
    /// The institution-side dispatch result (e.g. a Verdict for an
    /// AutoOnLoad / Decidable QueryClass). This is the pass/fail gate
    /// the commit pipeline reads via [`parse_verdict`] — Holds admits
    /// the gated commit, Fails rejects it. The institution-level
    /// Verdict carries no `canonical_proposition`; derivations are
    /// separate.
    pub output: Resource,
    /// Side-effect resources the institution produced *as artefacts
    /// of validation*. Empty for institutions whose only job is the
    /// pass/fail gate (e.g. Reasoning / Lean). Statistics emits one
    /// `StatisticalAnalysisResult` per ANOVA effect; each derivation is a
    /// `reflection:InstitutionEmittedDerivation` whose
    /// `canonical_proposition` the chain ends up attesting (D49
    /// §6 IsDerivedAs witness target). The kernel commits each
    /// derivation alongside the gate-Verdict when the gate Holds;
    /// derivations are dropped when the gate Fails.
    ///
    /// The institution sets each derivation's `@id` to its intended
    /// chain IRI (typically suffixed off the gated subject — e.g.
    /// `{analysis_iri}:result:main_A`), and sets the domain-specific
    /// properties. The kernel stamps the cross-resource linkage
    /// properties (`reflection:from_subject`, `reflection:runtime_invocation`)
    /// so every derivation can navigate back to its producer.
    pub derivations: Vec<Resource>,
    /// Substrate-captured provenance fields, ready to be folded into a
    /// full `RuntimeInvocation` by the kernel commit pipeline. Always
    /// `None` from non-external runtimes.
    pub partial_invocation: Option<Resource>,
}

impl QueryOutcome {
    /// Plain `output`-only outcome — what every non-external
    /// institution returns when it has no derivations to emit. Keeps
    /// the call site short.
    pub fn from_output(output: Resource) -> Self {
        Self {
            output,
            derivations: Vec::new(),
            partial_invocation: None,
        }
    }
}

/// The interface an institution implements at runtime. Three methods,
/// of which only the two boundary methods are mandatory.
///
/// **Boundary methods** (`extract_typed`, `reify`) translate between
/// the institution's resource form and the typed EigenTT `Val` form
/// the kernel manipulates internally. Every institution must implement
/// these — they are how the kernel reaches the institution's data at
/// all.
///
/// **Reasoning method** (`query`) is the institution's escape hatch
/// for QueryClasses whose implementation is opaque code (e.g. an
/// LLM, an external prover). QueryClasses whose `query_handler` IRI
/// resolves to a kernel-registered Component are dispatched entirely
/// through EigenTT (extract → component → reify) and never call this
/// method. The default impl returns
/// [`InstitutionError::NotImplemented`].
///
/// `procedure_iri` in each method is the dispatch key declared on the
/// `ExportFormat` / `ImportFormat` / `QueryClass` resource. The same
/// institution may handle multiple procedures and dispatch internally
/// on the IRI.
pub trait Institution: Send + Sync {
    /// The institution's IRI. Used as the key in
    /// [`InstitutionRuntime`].
    fn institution_iri(&self) -> &Iri;

    /// Boundary: extract a typed EigenTT value from a resource, via a
    /// procedure declared by an `ExportFormat` resource owned by this
    /// institution. The returned `Val` must inhabit the type declared
    /// by the matching `ExportFormat.payload_type`.
    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError>;

    /// Boundary: construct a target-class resource from a typed value,
    /// via a procedure declared by an `ImportFormat` resource owned by
    /// this institution. The input `Val` is guaranteed to inhabit the
    /// type declared by the matching `ImportFormat.payload_type`.
    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;

    /// Apply an institution-defined query — input resource of one
    /// class, output resource of another. Subsumes the prior
    /// `validate_morphism` / `decide` / `query` / `discover_morphisms`
    /// trichotomy; the dispatch role is determined by the QueryClass
    /// resource, not the trait method.
    ///
    /// Returns a [`QueryOutcome`] bundling the result Resource with an
    /// optional substrate-captured `partial_invocation`. External-
    /// runtime institutions populate the partial so the kernel's
    /// commit pipeline can fold it into a full `RuntimeInvocation`
    /// resource (D31 §6.3); in-process institutions return
    /// `partial_invocation: None`.
    ///
    /// Default impl: return `NotImplemented`. Institutions whose
    /// QueryClasses are all Component-implemented need not override.
    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        let _ = (input, ctx);
        Err(InstitutionError::NotImplemented(format!(
            "institution `{}` has no runtime query handler for `{procedure_iri}`",
            self.institution_iri()
        )))
    }
}

/// Blanket impl so `Arc<I>` is itself an `Institution` whenever `I` is.
///
/// Lets `Phase 20a`'s in-process registry hold institutions as shared
/// `Arc<dyn Institution>`s and re-register a fresh boxed wrapper into
/// [`InstitutionRuntime`] each time the chain is re-walked, without
/// reconstructing per-institution state (e.g. an in-process Lean
/// checker's parsed-environment cache). Mirrors the same pattern
/// [`crate::runtime_substrate::LanguageRuntime`] uses.
impl<I: Institution + ?Sized> Institution for std::sync::Arc<I> {
    fn institution_iri(&self) -> &Iri {
        (**self).institution_iri()
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        (**self).extract_typed(procedure_iri, resource, ctx)
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        (**self).reify(procedure_iri, value, ctx)
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        (**self).query(procedure_iri, input, ctx)
    }
}

/// Registry of institution implementations keyed by institution IRI.
///
/// The runtime is the dispatch table for kernel ↔ institution calls.
/// It is rebuilt at startup (bootstrap), refreshed on Phase 9a
/// rehydration, and updated when a new institution is installed via
/// the Load path.
///
/// The runtime does **not** mirror the chain — it carries
/// implementations only. Declaration data (Institution metadata,
/// ExportFormat / ImportFormat / QueryClass / Comorphism resources)
/// lives in the layer chain and is summarised by
/// [`registry::InstitutionIndex`]. The two are looked up together: the
/// index resolves a procedure / class IRI to a declaring institution
/// IRI, the runtime dispatches the call.
#[derive(Default)]
pub struct InstitutionRuntime {
    institutions: BTreeMap<Iri, Box<dyn Institution>>,
}

impl InstitutionRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an institution implementation. Returns an error if an
    /// institution with the same IRI is already registered — registry
    /// updates require an explicit replace via [`replace`].
    pub fn register(&mut self, institution: Box<dyn Institution>) -> Result<(), InstitutionError> {
        let iri = institution.institution_iri().clone();
        if self.institutions.contains_key(&iri) {
            return Err(InstitutionError::ComputationFailed(format!(
                "institution `{iri}` is already registered"
            )));
        }
        self.institutions.insert(iri, institution);
        Ok(())
    }

    /// Replace an existing institution implementation, or insert if
    /// missing. Used during rehydration when a re-registration is
    /// expected.
    pub fn replace(&mut self, institution: Box<dyn Institution>) {
        let iri = institution.institution_iri().clone();
        self.institutions.insert(iri, institution);
    }

    /// Look up an institution by its IRI.
    pub fn get(&self, iri: &Iri) -> Option<&dyn Institution> {
        self.institutions.get(iri).map(|b| b.as_ref())
    }

    /// True if no institutions are registered.
    pub fn is_empty(&self) -> bool {
        self.institutions.is_empty()
    }

    /// Number of registered institutions.
    pub fn len(&self) -> usize {
        self.institutions.len()
    }

    /// All registered institution IRIs, in iteration order.
    pub fn iris(&self) -> impl Iterator<Item = &Iri> {
        self.institutions.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::institution::error::InstitutionError;
    use crate::nbe::val::Val;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// A minimal institution implementation used to exercise the
    /// trait + runtime dispatch surface. Records every dispatched
    /// call so the test can assert on the routing.
    struct TestInstitution {
        iri: Iri,
        log: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Institution for TestInstitution {
        fn institution_iri(&self) -> &Iri {
            &self.iri
        }

        fn extract_typed(
            &self,
            procedure_iri: &Iri,
            _resource: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("extract:{procedure_iri}"));
            Ok(Val::Unit)
        }

        fn reify(
            &self,
            procedure_iri: &Iri,
            _value: &Val,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("reify:{procedure_iri}"));
            Ok(Resource::new_embedded())
        }

        // Deliberately *do not* override `query` — the test uses the
        // default impl to confirm `NotImplemented` is returned.
    }

    fn make_ctx() -> ExecutionContext {
        let storage = crate::layer::LayerStorage::in_memory();
        let layer = Arc::new(crate::layer::LayerBuilder::new("empty", None).build(storage.clone()));
        ExecutionContext::new(
            layer,
            "test",
            crate::context::ExecutionMode::ReadOnly,
            storage,
        )
    }

    #[test]
    fn registers_and_dispatches_through_runtime() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let inst = Box::new(TestInstitution {
            iri: iri("urn:eigenius:test:inst:dock"),
            log: Arc::clone(&log),
        });

        let mut runtime = InstitutionRuntime::new();
        runtime.register(inst).expect("register");

        assert_eq!(runtime.len(), 1);
        let dispatched = runtime
            .get(&iri("urn:eigenius:test:inst:dock"))
            .expect("registered institution looked up");

        let ctx = make_ctx();
        let resource = Resource::new_embedded();
        dispatched
            .extract_typed(&iri("urn:eigenius:test:proc:p1"), &resource, &ctx)
            .expect("extract_typed dispatched");
        dispatched
            .reify(&iri("urn:eigenius:test:proc:p2"), &Val::Unit, &ctx)
            .expect("reify dispatched");

        let entries = log.lock().unwrap();
        assert_eq!(
            *entries,
            vec![
                "extract:urn:eigenius:test:proc:p1".to_string(),
                "reify:urn:eigenius:test:proc:p2".to_string(),
            ]
        );
    }

    #[test]
    fn duplicate_register_rejected_without_replace() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(TestInstitution {
                iri: iri("urn:eigenius:test:inst:dup"),
                log: Arc::clone(&log),
            }))
            .expect("first register");

        let err = runtime
            .register(Box::new(TestInstitution {
                iri: iri("urn:eigenius:test:inst:dup"),
                log: Arc::clone(&log),
            }))
            .expect_err("duplicate register should fail");
        assert!(matches!(err, InstitutionError::ComputationFailed(_)));
        assert_eq!(runtime.len(), 1);
    }

    #[test]
    fn replace_overwrites_existing() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runtime = InstitutionRuntime::new();
        runtime
            .register(Box::new(TestInstitution {
                iri: iri("urn:eigenius:test:inst:rehydrate"),
                log: Arc::clone(&log),
            }))
            .expect("register");
        runtime.replace(Box::new(TestInstitution {
            iri: iri("urn:eigenius:test:inst:rehydrate"),
            log: Arc::clone(&log),
        }));
        assert_eq!(runtime.len(), 1);
    }

    #[test]
    fn default_query_returns_not_implemented() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let inst = TestInstitution {
            iri: iri("urn:eigenius:test:inst:noquery"),
            log,
        };
        let ctx = make_ctx();
        let err = inst
            .query(
                &iri("urn:eigenius:test:proc:any"),
                &Resource::new_embedded(),
                &ctx,
            )
            .expect_err("default query should error");
        assert!(matches!(err, InstitutionError::NotImplemented(_)));
    }

    #[test]
    fn missing_lookup_returns_none() {
        let runtime = InstitutionRuntime::new();
        assert!(runtime.get(&iri("urn:eigenius:test:inst:absent")).is_none());
        assert!(runtime.is_empty());
    }
}
