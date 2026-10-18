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

//! Registry of `LanguageRuntime` implementations keyed by language ID.
//!
//! The substrate's component dispatch ([`crate::facade`]) reads the
//! `language` property off an incoming `RuntimeScript` /
//! `RuntimeMethodSignature` resource and looks up the matching
//! [`LanguageRuntime`] here. Per-language crates register their
//! implementations at orchestrator startup; failure to find a runtime
//! becomes [`crate::error::RunError::RuntimeError`] with a clear
//! "no runtime registered for language X" diagnostic.

use crate::language_runtime::LanguageRuntime;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("language runtime `{0}` is already registered")]
    AlreadyRegistered(String),
}

/// Registry of `LanguageRuntime` implementations keyed by
/// [`LanguageRuntime::language_id`].
#[derive(Default)]
pub struct LanguageRuntimeRegistry {
    runtimes: BTreeMap<String, Box<dyn LanguageRuntime>>,
}

impl LanguageRuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a runtime. Errors if a runtime with the same
    /// `language_id` is already present — replacement is an explicit
    /// [`replace`](Self::replace) call.
    pub fn register(&mut self, runtime: Box<dyn LanguageRuntime>) -> Result<(), RegistryError> {
        let id = runtime.language_id().to_string();
        if self.runtimes.contains_key(&id) {
            return Err(RegistryError::AlreadyRegistered(id));
        }
        self.runtimes.insert(id, runtime);
        Ok(())
    }

    /// Replace an existing runtime, or insert if missing. Used during
    /// orchestrator-side rehydration when a re-registration is expected.
    pub fn replace(&mut self, runtime: Box<dyn LanguageRuntime>) {
        let id = runtime.language_id().to_string();
        self.runtimes.insert(id, runtime);
    }

    /// Look up a runtime by language ID.
    pub fn get(&self, language_id: &str) -> Option<&dyn LanguageRuntime> {
        self.runtimes.get(language_id).map(|b| b.as_ref())
    }

    pub fn is_empty(&self) -> bool {
        self.runtimes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.runtimes.len()
    }

    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.runtimes.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{BuildError, RunError};
    use crate::invocation::RunOutcome;
    use crate::types::{DockerfileFragments, ImageDigest};
    use eigenius_kernel::ontology::resource::Resource;

    struct StubRuntime {
        id: &'static str,
    }

    impl LanguageRuntime for StubRuntime {
        fn language_id(&self) -> &str {
            self.id
        }
        fn build_environment_image(
            &self,
            _: &Resource,
            _: &[Resource],
            _: Option<&Resource>,
        ) -> Result<ImageDigest, BuildError> {
            unimplemented!()
        }
        fn run_script(
            &self,
            _: &Resource,
            _: &Resource,
            _: &[Resource],
        ) -> Result<RunOutcome, RunError> {
            unimplemented!()
        }
        fn call_method(
            &self,
            _: &Resource,
            _: &Resource,
            _: &[Resource],
        ) -> Result<RunOutcome, RunError> {
            unimplemented!()
        }
        fn dockerfile_fragments(&self, _: &Resource) -> DockerfileFragments {
            DockerfileFragments::default()
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = LanguageRuntimeRegistry::new();
        reg.register(Box::new(StubRuntime { id: "test" })).unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.get("test").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn duplicate_register_rejected() {
        let mut reg = LanguageRuntimeRegistry::new();
        reg.register(Box::new(StubRuntime { id: "test" })).unwrap();
        let err = reg
            .register(Box::new(StubRuntime { id: "test" }))
            .expect_err("duplicate should fail");
        assert!(matches!(err, RegistryError::AlreadyRegistered(_)));
    }

    #[test]
    fn replace_overwrites() {
        let mut reg = LanguageRuntimeRegistry::new();
        reg.register(Box::new(StubRuntime { id: "test" })).unwrap();
        reg.replace(Box::new(StubRuntime { id: "test" }));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn languages_iterates_in_sorted_order() {
        let mut reg = LanguageRuntimeRegistry::new();
        reg.register(Box::new(StubRuntime { id: "julia" })).unwrap();
        reg.register(Box::new(StubRuntime { id: "lean" })).unwrap();
        reg.register(Box::new(StubRuntime { id: "test" })).unwrap();
        let langs: Vec<_> = reg.languages().collect();
        assert_eq!(langs, vec!["julia", "lean", "test"]);
    }
}
