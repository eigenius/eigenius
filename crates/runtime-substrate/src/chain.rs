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

//! Read access to the kernel's layer chain for the substrate's
//! boundary check (D26 §7.5).
//!
//! The boundary check needs three things from the chain: resolve an
//! IRI to its current Resource against a claim layer, walk
//! ancestor-descendant relationships, and detect class redefinitions
//! between two layers. None of these are specific to the substrate —
//! they're plumbing the kernel already does — but routing them
//! through a trait keeps the substrate crate testable with synthetic
//! chains and decoupled from kernel internals.
//!
//! Implementations:
//! - **Kernel-side** (`KernelChainAccessor`, lands in the kernel
//!   wiring slice of Phase 18b): wraps an `Arc<Layer>` and uses the
//!   kernel's existing layer-walk + content-hash machinery.
//! - **Test mocks**: synthetic chains in this crate's tests, so
//!   `boundary` can be exercised without a kernel running.

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;

/// Trait abstracting the kernel's layer chain for the substrate's
/// boundary check.
pub trait ChainAccessor: Send + Sync {
    /// Resolve `target` to its Resource definition as seen from
    /// `claim_layer` — i.e. walking `claim_layer` and its ancestors.
    /// Returns `None` if the IRI isn't defined anywhere in that
    /// chain.
    fn resolve(&self, claim_layer: &Iri, target: &Iri) -> Option<Resource>;

    /// True iff `candidate == anchor` or `candidate` is a descendant
    /// of `anchor` in the layer chain (D26 §7.3 compositionality).
    /// Used to verify a `RuntimePackageMirror`'s `source_layer` is
    /// ancestral-or-equal to the invocation's claim layer.
    fn is_ancestor_or_equal(&self, anchor: &Iri, candidate: &Iri) -> bool;

    /// True iff the byte-level definition of `class_iri` is
    /// identical between `mirror_layer` and `claim_layer`. False if
    /// the class has been redefined anywhere on the path from
    /// `mirror_layer` to `claim_layer`, or if the two layers are
    /// not chain-related.
    ///
    /// "Byte-level" means content-hash equality of the class
    /// resource as serialised by the kernel's canonical encoder
    /// (`eigon_cbor::canonicalize`). Equivalent definitions that
    /// differ only in property ordering or formatting are still
    /// considered equal.
    fn class_unchanged_between(
        &self,
        mirror_layer: &Iri,
        claim_layer: &Iri,
        class_iri: &Iri,
    ) -> bool;
}
