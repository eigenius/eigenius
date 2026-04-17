//! Component types and registry for program execution.
//!
//! Defines the `BuiltinComponent` trait, `ComponentRegistry`, and error types.
//! Execution is handled by `eval_io::execute_program_nbe` via NbE in IO mode.

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use crate::program::trace::ComponentMetrics;
use std::collections::BTreeMap;
use std::fmt;

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
pub struct ComponentRegistry {
    components: BTreeMap<String, Box<dyn BuiltinComponent>>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, name: String, component: Box<dyn BuiltinComponent>) {
        self.components.insert(name, component);
    }

    pub fn get(&self, name: &str) -> Option<&dyn BuiltinComponent> {
        self.components.get(name).map(|b| b.as_ref())
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(
            "urn:eigenius:program:components:Identity".to_string(),
            Box::new(IdentityComponent),
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
