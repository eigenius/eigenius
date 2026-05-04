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

//! Kernel-side hooks for the runtime substrate (Phase 18).
//!
//! The substrate itself lives in `crates/runtime-substrate` and is
//! reachable through the orchestrator. Phase 18a wired the napi
//! addon and TypeScript handlers. This module is for kernel-side
//! concerns that the substrate doesn't own — specifically the
//! boundary check from D26 §7.5 (Phase 18b), which needs the
//! kernel's layer chain to verify mirror anchors and class
//! compositionality.
//!
//! See [`boundary`] for the check itself. Other runtime-substrate
//! hooks (chain-resolution helpers, dispatch wiring) land here as
//! Phase 18b/c progress.

pub mod boundary;
