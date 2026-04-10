//! Layer system for stratified ontology composition and reasoning.
//!
//! Layer architecture (§7) enables composable layers of ontologies with
//! different semantics, constraints, and reasoning modes. Each layer represents
//! a distinct reasoning plane, with a LayerStack managing composition.

use async_trait::async_trait;

/// A single layer of ontological definitions and constraints.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Unique identifier for this layer
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Layer-specific ontology data (placeholder)
    pub data: Vec<u8>,
}

impl Layer {
    /// Creates a new Layer.
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            data: Vec::new(),
        }
    }
}

/// Stack of layers providing stratified composition.
#[derive(Debug, Clone)]
pub struct LayerStack {
    /// Layers ordered from base to top
    pub layers: Vec<Layer>,
}

impl LayerStack {
    /// Creates a new empty LayerStack.
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
        }
    }

    /// Pushes a layer onto the stack.
    pub fn push(&mut self, layer: Layer) {
        self.layers.push(layer);
    }

    /// Pops the top layer from the stack.
    pub fn pop(&mut self) -> Option<Layer> {
        self.layers.pop()
    }
}

/// Trait for layer storage and retrieval.
#[async_trait]
pub trait LayerStore {
    /// Loads a layer by ID.
    async fn load_layer(&self, id: &str) -> Result<Layer, String>;

    /// Stores a layer.
    async fn store_layer(&self, layer: Layer) -> Result<(), String>;

    /// Lists all available layers.
    async fn list_layers(&self) -> Result<Vec<String>, String>;
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}
