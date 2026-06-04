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

//! `eigenius-config` — layered configuration loader for the Eigenius
//! platform.
//!
//! Resolution order, lowest precedence first:
//!
//! 1. **Schema [`Default`]s.** Every section has a default that
//!    matches v1's previously-hardcoded constants; a fresh checkout
//!    boots without writing any config file.
//! 2. **TOML file.** First match in the search path
//!    `$EIGENIUS_CONFIG → ./eigenius.toml → ~/.config/eigenius/config.toml`
//!    (or an explicit path passed to [`Loader::with_file`]).
//! 3. **Environment variables.** Flat translation of the TOML keys
//!    into screaming snake case (`EIGENIUS_DOCKER_DAEMON_SOCKET`, …).
//! 4. **Construction-time overrides** ([`Loader::with_overrides`]) —
//!    used by tests and as a programmatic escape hatch.
//!
//! What this crate is *not*: hot-reload, audit log, per-namespace
//! overrides. Those land in a follow-on phase if and when concrete
//! needs surface — v1 ships the primitive only.
//!
//! Per-spawn env vars (`EIGENIUS_RUNTIME_ENV_DIGEST`, …) are *not*
//! config: they are per-invocation parameters set by the substrate
//! when spawning workers. They stay as direct `std::env::var` reads
//! in the worker bootstrap.

mod config;
pub mod embedder;
mod loader;
pub mod substrate;

pub use config::Config;
pub use embedder::{DeviceSelection, EmbedderConfig};
pub use loader::{Loader, LoaderError};
pub use substrate::{DockerConfig, ImageConfig, LocalConfig, SubstrateConfig};
