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
//! [`LeanInstitution`](crate::LeanInstitution) into the per-process
//! [`InProcessInstitutionRegistry`](eigenius_kernel::institution::in_process_registry::InProcessInstitutionRegistry).
//!
//! The CLI's `serve` subcommand calls [`register`] once per process,
//! before any chain-walk runs. The chain-scan
//! `register_in_process_institutions` pass
//! (kernel-side, Phase 20a.1) then looks the institution up by its
//! IRI when it encounters the `lean:lean_institution` declaration in
//! the bootstrapped chain.

use eigenius_kernel::server::EigeniusService;

use crate::LeanInstitution;

/// Register the Lean institution into the service's process-global
/// in-process registry. Idempotent — calling more than once replaces
/// the prior entry, matching the registry's `replace`-on-re-register
/// discipline.
///
/// The verdict path stays inside the kernel process: this registration
/// is what makes `service.rebuild_institution_index(...)` able to
/// dispatch AutoOnLoad `qc_proof_check` firings through
/// `LeanInstitution::query` as a direct function call (no IPC, per
/// [D28](../../docs/design/d28-lean-4-as-institution.md) §2.3 / §10.2).
pub fn register(service: &EigeniusService) {
    service.register_in_process_institution(LeanInstitution::arc());
}
