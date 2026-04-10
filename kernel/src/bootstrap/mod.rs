//! Bootstrap sequence for kernel initialization.
//!
//! The bootstrap() function (§2.5) orchestrates the two-phase initialization:
//! Phase 1 loads the Core Ontology (self-describing schema), then Phase 2 loads
//! the Foundation Layer providing essential semantic constraints and operations.

use crate::ontology::CoreOntology;
use crate::layer::{Layer, LayerStack};

/// Bootstrap the Eigenius kernel.
///
/// Phase 1: Load Core Ontology (self-describing schema)
/// Phase 2: Load Foundation Layer (essential constraints)
///
/// Returns a LayerStack ready for ontological reasoning.
pub fn bootstrap() -> Result<LayerStack, String> {
    // Phase 1: Initialize Core Ontology
    let _core_ontology = CoreOntology::init();

    // Phase 2: Create and load Foundation Layer
    let foundation_layer = Layer::new(
        "eigenius:FoundationLayer".to_string(),
        "Foundation Layer".to_string(),
    );

    let mut layer_stack = LayerStack::new();
    layer_stack.push(foundation_layer);

    Ok(layer_stack)
}

/// Validates that bootstrap completed successfully.
pub fn validate_bootstrap(layer_stack: &LayerStack) -> bool {
    !layer_stack.layers.is_empty()
}
