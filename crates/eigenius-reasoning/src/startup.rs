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

//! Startup hook for the kernel binary: register
//! [`ReasoningInstitution`](crate::ReasoningInstitution) into the
//! per-process in-process registry. Mirrors `eigenius_lean::startup`.
//!
//! The CLI's `serve` subcommand calls [`register`] once per process,
//! before any chain-walk runs. The chain-scan registration pass then
//! looks the institution up by its IRI when it encounters the
//! `reasoning:reasoning_institution` declaration on chain.

use eigenius_kernel::server::EigeniusService;

use crate::ReasoningInstitution;

/// Register the Reasoning institution into the service's process-global
/// in-process registry. Idempotent — re-calling replaces the prior
/// entry, matching the registry's `replace`-on-re-register discipline.
pub fn register(service: &EigeniusService) {
    service.register_in_process_institution(ReasoningInstitution::arc());
}
