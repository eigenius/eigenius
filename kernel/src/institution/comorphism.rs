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

//! Comorphism trait and registry.
//!
//! A comorphism translates between two institutions:
//! signatures forward, models backward, preserving satisfaction.

use crate::context::ExecutionContext;
use crate::institution::error::InstitutionError;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;

/// A typed translation between two institutions.
pub trait Comorphism: Send + Sync {
    /// Source institution IRI.
    fn source(&self) -> &Iri;

    /// Target institution IRI.
    fn target(&self) -> &Iri;

    /// Translate a resource from the source institution's fiber
    /// into the target institution's fiber.
    fn translate_forward(
        &self,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;

    /// Translate a resource from the target institution's fiber
    /// back into the source institution's fiber.
    fn translate_backward(
        &self,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;
}

/// Registry of comorphisms between institutions.
pub struct ComorphismRegistry {
    comorphisms: Vec<Box<dyn Comorphism>>,
}

impl ComorphismRegistry {
    pub fn new() -> Self {
        Self {
            comorphisms: Vec::new(),
        }
    }

    pub fn register(&mut self, comorphism: Box<dyn Comorphism>) {
        self.comorphisms.push(comorphism);
    }

    /// Find a comorphism from source to target.
    pub fn find(&self, source: &Iri, target: &Iri) -> Option<&dyn Comorphism> {
        self.comorphisms
            .iter()
            .find(|c| c.source() == source && c.target() == target)
            .map(|c| c.as_ref())
    }

    /// List all registered comorphisms as (source, target) pairs.
    pub fn list(&self) -> Vec<(&Iri, &Iri)> {
        self.comorphisms
            .iter()
            .map(|c| (c.source(), c.target()))
            .collect()
    }
}

impl Default for ComorphismRegistry {
    fn default() -> Self {
        Self::new()
    }
}
