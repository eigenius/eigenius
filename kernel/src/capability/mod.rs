//! WASM capability hosting via Wasmtime Component Model.
//!
//! Hosts pure/read WASM components and institution fiber reasoners
//! in the kernel. IO WASM components are hosted by the orchestrator.
//!
//! See design document D12 for the full specification.

pub mod registration;
pub mod wasm_component;
pub mod wasm_institution;

#[cfg(test)]
mod tests;
