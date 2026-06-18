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

//! Startup hook for the kernel binary: register the
//! [`StatisticsInstitution`](crate::StatisticsInstitution) into the
//! per-process in-process registry. Mirrors `eigenius_reasoning::startup`.

use std::sync::Arc;

use eigenius_kernel::server::EigeniusService;
use eigenius_kernel::storage::content_array::ContentArrayStore;

use crate::StatisticsInstitution;

/// Env var naming the local content-addressed cache directory for file-backed
/// SampleSet observations (D53 §6.1) — the depot's `extfile-cache`, the same
/// directory the orchestrator materializes inputs into
/// (`<depot>/extfile-cache/<sha256-hex>/<name>`). When set, native recompute
/// reads observations from an Oxen-tracked `PinnedExternalFile` by hash; when
/// unset, only `file://` references on a shared volume are readable.
const EXTFILE_CACHE_DIR_ENV: &str = "EIGENIUS_EXTFILE_CACHE_DIR";

/// Register the Statistics institution into the service's process-
/// global in-process registry. Idempotent — re-calling replaces the
/// prior entry, matching the registry's `replace`-on-re-register
/// discipline.
///
/// If `EIGENIUS_EXTFILE_CACHE_DIR` is set, the institution is wired with a
/// content-array store backed by that cache, so file-backed SampleSets whose
/// bytes live in Oxen (materialized into the per-host depot cache) recompute
/// natively (D53 §6.1). Otherwise it resolves `file://` references only.
pub fn register(service: &EigeniusService) {
    let institution: Arc<dyn eigenius_kernel::institution::runtime::Institution> =
        match std::env::var(EXTFILE_CACHE_DIR_ENV) {
            Ok(dir) if !dir.trim().is_empty() => Arc::new(
                StatisticsInstitution::with_content_store(ContentArrayStore::with_cache_root(dir)),
            ),
            _ => Arc::new(StatisticsInstitution::new()),
        };
    service.register_in_process_institution(institution);
}
