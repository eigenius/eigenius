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

//! Process-global registry of in-process institution implementations
//! (Phase 20a.1).
//!
//! The `RuntimeKind::InProcess` variant has been parsed from chain
//! resources for institutions ([`crate::institution::registry`]) but had no
//! registration path until Phase 20a — only `External` (via
//! [`crate::capability::registration::register_external_institutions`])
//! actually populated [`InstitutionRuntime`]. This module closes that
//! gap.
//!
//! ## Why a separate registry
//!
//! External institutions are constructed from chain data plus a gRPC
//! client. **In-process institutions are
//! Rust code** linked into the kernel/orchestrator binary — there is
//! no chain-side data sufficient to construct them. The
//! `InProcessInstitutionRegistry` is the per-process container that
//! lets statically-linked institution crates (e.g. `eigenius-lean`)
//! pre-register their `Institution` impl at orchestrator startup, so
//! the chain-scan registration pass can look the impl up by IRI when
//! it walks a `runtime: in_process` declaration.
//!
//! ## Lifecycle
//!
//! 1. At process startup, each in-process institution crate calls
//!    [`InProcessInstitutionRegistry::register`] once, supplying an
//!    `Arc<dyn Institution>` keyed by its institution IRI.
//! 2. On every chain rebuild (boot, Load commit, rehydration), the
//!    server calls
//!    [`crate::capability::registration::register_in_process_institutions`]
//!    which walks the chain for `Institution` resources with
//!    `runtime: in_process` and registers the matching pre-registered
//!    impl into [`InstitutionRuntime`].
//! 3. Chain-declared `runtime: in_process` institutions that lack a
//!    matching pre-registered impl produce a registration error — same
//!    discipline the External path uses when its `--orchestrator`
//!    client is missing.
//!
//! ## Sharing
//!
//! Institutions are stored as `Arc<dyn Institution>` so the registry
//! can hand out cloneable handles. The blanket
//! `impl<I: Institution + ?Sized> Institution for Arc<I>` in
//! [`crate::institution::runtime`] lets each rebuild wrap a fresh
//! `Box::new(arc.clone())` into [`InstitutionRuntime`] without
//! reconstructing per-institution state (an in-process Lean checker's
//! parsed-environment LRU cache, for example).

use crate::institution::runtime::Institution;
use crate::ontology::iri::Iri;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Process-global registry of in-process institution implementations,
/// keyed by institution IRI.
///
/// Owned by the orchestrator's startup code; institution crates pre-
/// register their impl via [`Self::register`] before the kernel server
/// begins walking the chain.
///
/// All methods are thread-safe — registration may happen on any
/// startup thread; lookups happen during chain-scan rebuilds from the
/// server's tokio runtime.
#[derive(Default)]
pub struct InProcessInstitutionRegistry {
    institutions: Mutex<BTreeMap<Iri, Arc<dyn Institution>>>,
}

impl InProcessInstitutionRegistry {
    /// Build a fresh, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-register an in-process institution impl. Idempotent —
    /// re-registering the same IRI replaces the prior entry, matching
    /// [`InstitutionRuntime::replace`]'s rehydration discipline.
    ///
    /// The IRI keying the registry is taken from
    /// [`Institution::institution_iri`] on the supplied impl.
    pub fn register(&self, institution: Arc<dyn Institution>) {
        let iri = institution.institution_iri().clone();
        self.institutions
            .lock()
            .expect("InProcessInstitutionRegistry mutex poisoned")
            .insert(iri, institution);
    }

    /// Look up a pre-registered impl by its institution IRI. Returns a
    /// cloned `Arc` so the caller can register it into
    /// [`InstitutionRuntime`] independently of subsequent rebuilds.
    pub fn get(&self, iri: &Iri) -> Option<Arc<dyn Institution>> {
        self.institutions
            .lock()
            .expect("InProcessInstitutionRegistry mutex poisoned")
            .get(iri)
            .cloned()
    }

    /// All registered institution IRIs, sorted by IRI.
    pub fn iris(&self) -> Vec<Iri> {
        self.institutions
            .lock()
            .expect("InProcessInstitutionRegistry mutex poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Number of pre-registered impls.
    pub fn len(&self) -> usize {
        self.institutions
            .lock()
            .expect("InProcessInstitutionRegistry mutex poisoned")
            .len()
    }

    /// True if no impls have been registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Test-only minimal stub: an institution that returns `Verdict::Holds`
/// for every query, irrespective of input. Used in unit + integration
/// tests to exercise the in-process registration path's lookup + handoff
/// to [`crate::institution::runtime::InstitutionRuntime`] without
/// depending on Lean specifically. Lives at module-level (rather than
/// inside `#[cfg(test)] mod tests`) so other test modules in the crate
/// can reuse it via `EchoInstitution::new(iri)`.
#[cfg(test)]
pub(crate) struct EchoInstitution {
    iri: Iri,
}

#[cfg(test)]
impl EchoInstitution {
    pub(crate) fn new(iri: Iri) -> Self {
        Self { iri }
    }
}

#[cfg(test)]
impl Institution for EchoInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        _procedure_iri: &Iri,
        _resource: &crate::ontology::resource::Resource,
        _ctx: &crate::context::ExecutionContext,
    ) -> Result<crate::nbe::val::Val, crate::institution::error::InstitutionError> {
        Err(crate::institution::error::InstitutionError::NotImplemented(
            "EchoInstitution::extract_typed is a test stub".to_string(),
        ))
    }

    fn reify(
        &self,
        _procedure_iri: &Iri,
        _value: &crate::nbe::val::Val,
        _ctx: &crate::context::ExecutionContext,
    ) -> Result<crate::ontology::resource::Resource, crate::institution::error::InstitutionError>
    {
        Err(crate::institution::error::InstitutionError::NotImplemented(
            "EchoInstitution::reify is a test stub".to_string(),
        ))
    }

    fn query(
        &self,
        _procedure_iri: &Iri,
        _input: &crate::ontology::resource::Resource,
        _ctx: &crate::context::ExecutionContext,
    ) -> Result<
        crate::institution::runtime::QueryOutcome,
        crate::institution::error::InstitutionError,
    > {
        // Returns a hand-constructed `Verdict::Holds` resource — the
        // simplest in-process query result.
        use crate::ontology::resource::{Resource, Value};
        use crate::ontology::well_known as wk;
        let verdict_iri = Iri::parse("urn:eigenius:test:echo:verdict").expect("test IRI");
        let mut r = Resource::new(verdict_iri);
        r.set(
            Iri::parse(wk::IS_A).expect("IS_A IRI"),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse(wk::VERDICT).expect("VERDICT IRI"),
            )]),
        );
        r.set(
            Iri::parse(wk::CTOR_NAME).expect("CTOR_NAME IRI"),
            Value::String(wk::VERDICT_HOLDS.to_string()),
        );
        Ok(crate::institution::runtime::QueryOutcome {
            output: r,
            derivations: Vec::new(),
            partial_invocation: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let reg = InProcessInstitutionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.iris().is_empty());
    }

    #[test]
    fn register_then_lookup_returns_same_impl() {
        let reg = InProcessInstitutionRegistry::new();
        let iri = Iri::parse("urn:eigenius:test:echo").expect("test IRI");
        let inst: Arc<dyn Institution> = Arc::new(EchoInstitution::new(iri.clone()));
        reg.register(inst);

        assert_eq!(reg.len(), 1);
        assert_eq!(reg.iris(), vec![iri.clone()]);

        let retrieved = reg.get(&iri).expect("registered impl must look up");
        assert_eq!(retrieved.institution_iri(), &iri);
    }

    #[test]
    fn missing_lookup_returns_none() {
        let reg = InProcessInstitutionRegistry::new();
        let iri = Iri::parse("urn:eigenius:test:never_registered").expect("test IRI");
        assert!(reg.get(&iri).is_none());
    }

    #[test]
    fn re_register_replaces_existing_entry() {
        let reg = InProcessInstitutionRegistry::new();
        let iri = Iri::parse("urn:eigenius:test:echo").expect("test IRI");
        let first: Arc<dyn Institution> = Arc::new(EchoInstitution::new(iri.clone()));
        let second: Arc<dyn Institution> = Arc::new(EchoInstitution::new(iri.clone()));
        // Same IRI, two distinct allocations.
        let first_ptr = Arc::as_ptr(&first);
        let second_ptr = Arc::as_ptr(&second);
        assert_ne!(first_ptr, second_ptr);

        reg.register(Arc::clone(&first));
        reg.register(Arc::clone(&second));
        assert_eq!(reg.len(), 1);

        let retrieved = reg.get(&iri).expect("must look up");
        assert!(std::ptr::eq(
            Arc::as_ptr(&retrieved) as *const (),
            second_ptr as *const ()
        ));
    }
}
