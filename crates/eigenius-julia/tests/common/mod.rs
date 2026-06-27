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

//! Shared helpers for the Julia institution chain-validation tests.

/// A stub `runtime:RuntimeEnvironment` declaration JSON for `env_iri`.
///
/// Every Julia institution declares `institution:requires_environment`
/// pointing at its `*:env:v1` `RuntimeEnvironment`. Under closed-world
/// reference integrity (D62 Rule 22) that reference must resolve on the
/// chain. The live-stack demos (`demo/<inst>/run.sh`) commit the real
/// env — built from an actual language image — *before* installing the
/// institution; these chain-validation tests can't build an image, so
/// they commit this stub in the same position (env before institution).
///
/// The stub carries exactly the declaration-time `requires` fields of
/// `runtime:RuntimeEnvironment` (`language` / `runtime_version` /
/// `lockfile` / `lifecycle`); the deploy-time `image_digest` (a
/// `recommends` field produced by `env build`) is intentionally absent.
pub fn stub_env_json(env_iri: &str, language: &str) -> String {
    // `lifecycle` is a `class_types: RuntimeEnvironmentLifecycle` ref;
    // `runtime:lifecycle:Service` is declared in the runtime-substrate
    // ontology (on the chain via bootstrap), so it resolves.
    format!(
        r##"{{
  "@id": "{env_iri}",
  "urn:eigenius:core:is_a": ["urn:eigenius:runtime:RuntimeEnvironment"],
  "urn:eigenius:core:short_name": "{language}-env",
  "urn:eigenius:runtime:language": "{language}",
  "urn:eigenius:runtime:runtime_version": "1.12.0",
  "urn:eigenius:runtime:lockfile": "# stub Manifest.toml — real lockfile committed by the live-stack demo",
  "urn:eigenius:runtime:lifecycle": "urn:eigenius:runtime:lifecycle:Service"
}}"##
    )
}
