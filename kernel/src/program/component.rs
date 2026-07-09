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

//! Component types and registry for program execution.
//!
//! Defines the `BuiltinComponent` trait, `ComponentRegistry`, and error types.
//! Execution is handled by `eval_io::execute_program_nbe` via NbE in IO mode.

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use crate::program::trace::ComponentMetrics;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Errors during program execution.
#[derive(Debug)]
pub enum ProgramError {
    Parse(String),
    TypeCheck(String),
    Execution(String),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramError::Parse(msg) => write!(f, "parse error: {msg}"),
            ProgramError::TypeCheck(msg) => write!(f, "type error: {msg}"),
            ProgramError::Execution(msg) => write!(f, "execution error: {msg}"),
        }
    }
}

impl std::error::Error for ProgramError {}

/// Result of executing a component: output resource plus optional metrics.
pub struct ComponentResult {
    pub output: Resource,
    pub metrics: Option<ComponentMetrics>,
}

/// A built-in component implementation.
pub trait BuiltinComponent: Send + Sync {
    /// Whether this component performs IO (non-deterministic, cacheable).
    fn is_io(&self) -> bool {
        false
    }

    /// Execute the component.
    ///
    /// - `input`: the evaluated argument expression (data flowing through the program)
    /// - `argument`: static component configuration (e.g., prompt template, model params).
    ///   Comes from `component_argument` on the Apply node. `None` if not provided.
    /// - `layer`: the current layer chain for resolution
    fn execute(
        &self,
        input: &Resource,
        argument: Option<&Resource>,
        layer: &Layer,
    ) -> Result<ComponentResult, String>;
}

/// Registry of built-in components.
///
/// Supports an optional parent pointer so that new registrations can stack
/// on top of an existing (immutable, shared) registry. Lookups walk the
/// parent chain: local entries shadow parent entries with the same IRI.
/// This avoids needing `BuiltinComponent: Clone` when stacking a new
/// layer's component registrations on top of a shared base registry.
pub struct ComponentRegistry {
    components: BTreeMap<String, Box<dyn BuiltinComponent>>,
    parent: Option<Arc<ComponentRegistry>>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: BTreeMap::new(),
            parent: None,
        }
    }

    /// Create a new registry layered on top of an existing one.
    /// Local registrations shadow parent entries with the same IRI.
    pub fn new_with_parent(parent: Arc<ComponentRegistry>) -> Self {
        Self {
            components: BTreeMap::new(),
            parent: Some(parent),
        }
    }

    pub fn register(&mut self, name: String, component: Box<dyn BuiltinComponent>) {
        self.components.insert(name, component);
    }

    pub fn get(&self, name: &str) -> Option<&dyn BuiltinComponent> {
        if let Some(c) = self.components.get(name) {
            return Some(c.as_ref());
        }
        self.parent.as_ref().and_then(|p| p.get(name))
    }

    /// List all registered component IRIs (local + inherited, deduplicated).
    pub fn list(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> =
            self.components.keys().cloned().collect();
        if let Some(p) = &self.parent {
            for name in p.list() {
                names.insert(name);
            }
        }
        names.into_iter().collect()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(
            "urn:eigenius:program:components:Identity".to_string(),
            Box::new(IdentityComponent),
        );
        registry.register(
            "urn:eigenius:program:components:Checkpoint".to_string(),
            Box::new(CheckpointComponent),
        );
        registry
    }
}

// --- Built-in components ---

struct IdentityComponent;

impl BuiltinComponent for IdentityComponent {
    fn execute(
        &self,
        input: &Resource,
        _argument: Option<&Resource>,
        _layer: &Layer,
    ) -> Result<ComponentResult, String> {
        Ok(ComponentResult {
            output: input.clone(),
            metrics: None,
        })
    }
}

/// `components:Checkpoint` — persist the input resource as the
/// running task's current checkpoint (D21 §4) and return it
/// unchanged.
///
/// The component itself is an identity function at the execute()
/// level; the checkpoint write lives in `dispatch_component`, which
/// has access to the `TaskContext` that this trait doesn't.
/// Registering it here keeps the component registry complete and
/// makes the IRI resolvable to a real component definition.
struct CheckpointComponent;

impl BuiltinComponent for CheckpointComponent {
    fn is_io(&self) -> bool {
        // IO so the checkpoint write goes through the task-aware
        // dispatch path (positional step key + commit_step).
        true
    }

    fn execute(
        &self,
        input: &Resource,
        _argument: Option<&Resource>,
        _layer: &Layer,
    ) -> Result<ComponentResult, String> {
        // Identity. `dispatch_component` intercepts this component's
        // IRI and piggybacks the checkpoint write on the commit_step
        // call. Outside a TaskContext, Checkpoint is a no-op.
        Ok(ComponentResult {
            output: input.clone(),
            metrics: None,
        })
    }
}

/// IRI of the Checkpoint built-in — exported so `dispatch_component`
/// can recognize it without string duplication. See D21 §4.
pub const CHECKPOINT_COMPONENT_IRI: &str = "urn:eigenius:program:components:Checkpoint";
