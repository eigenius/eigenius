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

//! Top-level [`Config`] aggregating every subsystem schema.

use crate::embedder::EmbedderConfig;
use crate::substrate::SubstrateConfig;
use serde::{Deserialize, Serialize};

/// Root config. Each subsystem (substrate, kernel, orchestrator)
/// gets its own field; new subsystems extend the struct rather than
/// reaching into a flat key/value bag.
///
/// `#[serde(default)]` at every level lets a TOML file specify any
/// subset of fields and have the rest filled in from
/// [`Default`] — the loader doesn't need a separate merge pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub substrate: SubstrateConfig,
    pub embedder: EmbedderConfig,
}
