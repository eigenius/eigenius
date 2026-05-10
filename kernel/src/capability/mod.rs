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

//! WASM capability hosting via Wasmtime Component Model.
//!
//! Hosts pure/read WASM components in the kernel. IO WASM components are
//! hosted by the orchestrator. Institution implementations follow the D14
//! triadic-comorphism contract via [`wasm_institution_d14`].
//!
//! See design document D12 (capability hosting) and D14 (institutions).

pub mod external_institution;
pub mod registration;
pub mod wasm_component;
pub mod wasm_institution_d14;

#[cfg(test)]
mod tests;
