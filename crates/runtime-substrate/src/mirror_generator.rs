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

//! Per-language mirror-generator integration points (D26 §7, Phase 18b).
//!
//! Per-language crates (Phase 19b for `eigon-julia-gen`, Phase 20b for
//! `eigon-ffi-gen`, …) implement [`MirrorGenerator`] and register it
//! into a [`MirrorGeneratorRegistry`] keyed on the `language_id`. The
//! substrate's image-build pipeline (Phase 18c) takes a seed of class
//! IRIs, asks the matching generator to produce a language-side
//! library archive, and bakes the result into a worker image.
//!
//! ## What a generator produces
//!
//! Per D26 §7: a *language-side library source archive* — a directory
//! tree of files (`.jl` for Julia, `.py` for Python, …) that, when
//! installed by the worker's package manager, gives user scripts
//! typed access to Eigon class structure. For each class in the
//! mirror's closure, the library carries a native struct/dataclass
//! with one field per required property, validators for format
//! constraints, and codecs to round-trip Eigon-CBOR at the worker
//! boundary.
//!
//! The mirror is **structural, not propositional** (D26 §7.2) —
//! format constraints get checked at construction; behavioural specs
//! and refinement types live in the Lean integration's `EigonFFI`
//! (D28), not here.
//!
//! ## Closure semantics
//!
//! The caller declares a *seed* of class IRIs ([`MirrorGenerationRequest::seed_classes`]);
//! the generator walks structural references — subclass parents,
//! `class_types` of resource-typed properties, enum values reachable
//! via `allows_only` — and returns the closure as
//! [`MirrorGenerationOutput::mirrored_classes`]. The substrate's
//! integrity chain (`generator_content_hash` + `library_content_hash`)
//! pins the produced library against the closure for audit.
//!
//! ## Where the seed comes from
//!
//! Open per D26 §14.3. The recommended sources, in increasing
//! automation:
//!
//! - **Manual** — `eigenius env create --mirror-class urn:foo:Bar`.
//! - **Script-driven** — union of `requires_mirror_classes` from
//!   every script that declares this env.
//! - **EigenQL filter** — a query against `source_layer` that yields
//!   a set of class IRIs (e.g. namespace-scoped, subclass-of-X). The
//!   query gets evaluated once at generation time; results are
//!   resolved into [`MirrorGenerationRequest::seed_classes`]; the
//!   query itself can optionally be recorded on the resulting
//!   `RuntimePackageMirror` for audit / regeneration.
//!
//! In all three the substrate's trait surface stays class-list-typed
//! — the seed is a `Vec<Iri>`. Per-language CLI tooling decides
//! which source to read.

use crate::chain::ChainAccessor;
use eigenius_kernel::ontology::iri::Iri;
use std::collections::BTreeMap;
use thiserror::Error;

/// Trait every language's mirror generator implements. Per-language
/// crates wrap their generator binary (or in-process Rust generator)
/// in a `MirrorGenerator` impl and register it via
/// [`MirrorGeneratorRegistry::register`].
pub trait MirrorGenerator: Send + Sync {
    /// Stable identifier for this generator — `"eigon-julia-gen"`,
    /// `"eigon-python-gen"`, `"eigon-ffi-gen"`, … Recorded on every
    /// `RuntimePackageMirror` this generator produces.
    fn generator_identifier(&self) -> &str;

    /// Version string for this generator implementation. Combined
    /// with `generator_content_hash` on the produced mirror.
    fn generator_version(&self) -> &str;

    /// SHA-256 of the generator binary, in `sha256:<64-hex>` form.
    /// Closes the integrity chain — given the same content_hash,
    /// the same source_layer, and the same seed, the generator
    /// produces byte-identical output.
    ///
    /// For Rust-only generators that don't wrap a separate binary,
    /// this can be the hash of the crate's `Cargo.lock` digest +
    /// version, or any other stable identifier the generator owner
    /// commits to.
    fn generator_content_hash(&self) -> &str;

    /// Produce a mirror for the seed classes against the given
    /// source layer. The implementation is responsible for:
    ///
    /// 1. Walking [`request.chain`](MirrorGenerationRequest::chain)
    ///    from each seed class transitively to collect the structural
    ///    closure (subclass parents, `class_types`, `allows_only`
    ///    enum values).
    /// 2. Emitting language-side source for the closure as a
    ///    [`LibraryContent::Embedded`] archive (or a content-addressed
    ///    [`LibraryContent::External`] reference for large libraries
    ///    — Phase 19+).
    /// 3. Returning the resolved closure on
    ///    [`MirrorGenerationOutput::mirrored_classes`].
    fn generate(
        &self,
        request: &MirrorGenerationRequest,
    ) -> Result<MirrorGenerationOutput, MirrorGeneratorError>;
}

/// Inputs to [`MirrorGenerator::generate`].
pub struct MirrorGenerationRequest<'a> {
    /// IRI of the layer the mirror anchors to. Class definitions are
    /// resolved against this layer, and the resulting mirror's
    /// `source_layer` field is set to this value. Per-class
    /// definitions are read via `chain.resolve(source_layer, class)`.
    pub source_layer: &'a Iri,

    /// Caller-declared seed: the classes the user explicitly wants
    /// mirrored. The generator computes the structural closure on
    /// top of this set.
    pub seed_classes: &'a [Iri],

    /// Read access to the layer chain for class-definition lookup
    /// and closure walking. The generator uses this to resolve each
    /// class IRI to its `Resource` definition at `source_layer`.
    pub chain: &'a dyn ChainAccessor,
}

/// Result of a successful [`MirrorGenerator::generate`] call.
pub struct MirrorGenerationOutput {
    /// Resolved closure of `seed_classes` under structural references.
    /// This is what gets stamped onto `RuntimePackageMirror.mirrored_classes`
    /// and what the boundary check pins against. Sorted by IRI for
    /// deterministic output.
    pub mirrored_classes: Vec<Iri>,

    /// Generated library — language-side source files bundled into
    /// either an embedded archive or a content-addressed external
    /// reference.
    pub library: LibraryContent,
}

/// The library archive a generator emits. The substrate's image-build
/// pipeline (Phase 18c) materialises this into the worker image's
/// build context and registers it with the language's package manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryContent {
    /// Files carried inline. Use for small libraries — typical
    /// Eigon mirrors fit comfortably under 1 MiB.
    Embedded(Vec<LibraryFile>),

    /// Content-addressed reference to external storage. Phase 19+.
    /// The integrity chain still terminates at `content_hash`; the
    /// substrate fetches the bytes lazily at image-build time.
    External {
        /// Storage-backend reference (URL, blob-store IRI, …). The
        /// substrate's image-build pipeline knows how to fetch it.
        reference: String,
        /// SHA-256 of the referenced bytes, in `sha256:<64-hex>` form.
        content_hash: String,
    },
}

/// One file in an embedded library archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    /// Path within the language's package layout — relative,
    /// forward-slash separated. Examples:
    /// - `"src/Person.jl"` (Julia)
    /// - `"my_mirror/person.py"` (Python)
    pub path: String,
    /// File contents as bytes. UTF-8 source code in practice; bytes
    /// for forward-compatibility with binary artifacts (compiled
    /// catalogs, etc.).
    pub content: Vec<u8>,
}

/// Failure modes for [`MirrorGenerator::generate`].
#[derive(Debug, Error)]
pub enum MirrorGeneratorError {
    /// A seed class IRI couldn't be resolved at `source_layer`.
    #[error("class `{0}` not found in chain at source_layer")]
    UnknownClass(String),

    /// The generator can't represent a class structurally in its
    /// target language (e.g. an inductive type beyond what the
    /// language's type system supports). The mirror as a whole
    /// fails — partial mirrors aren't a thing.
    #[error("class `{class_iri}` is not representable in {language}: {reason}")]
    UnrepresentableClass {
        class_iri: String,
        language: String,
        reason: String,
    },

    /// Generator-internal failure: subprocess crash, bad
    /// configuration, etc. Catch-all for backend-specific issues
    /// that don't map to a more specific variant.
    #[error("generator `{generator_identifier}` failed: {message}")]
    Internal {
        generator_identifier: String,
        message: String,
    },
}

/// Registry of `MirrorGenerator` implementations keyed by
/// [`MirrorGenerator::generator_identifier`].
///
/// The substrate's image-build pipeline looks up a generator by
/// identifier (from the env's per-language tooling configuration),
/// invokes it, and feeds the output through the rest of the build.
/// The registry parallels [`crate::registry::LanguageRuntimeRegistry`]
/// in shape and lifecycle: populated at orchestrator startup,
/// queried on demand.
#[derive(Default)]
pub struct MirrorGeneratorRegistry {
    generators: BTreeMap<String, Box<dyn MirrorGenerator>>,
}

#[derive(Debug, Error)]
pub enum MirrorRegistryError {
    #[error("mirror generator `{0}` is already registered")]
    AlreadyRegistered(String),
}

impl MirrorGeneratorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a generator. Errors if a generator with the same
    /// identifier is already present.
    pub fn register(
        &mut self,
        generator: Box<dyn MirrorGenerator>,
    ) -> Result<(), MirrorRegistryError> {
        let id = generator.generator_identifier().to_string();
        if self.generators.contains_key(&id) {
            return Err(MirrorRegistryError::AlreadyRegistered(id));
        }
        self.generators.insert(id, generator);
        Ok(())
    }

    /// Replace an existing generator, or insert if missing. Used
    /// during orchestrator-side rehydration when a re-registration
    /// is expected.
    pub fn replace(&mut self, generator: Box<dyn MirrorGenerator>) {
        let id = generator.generator_identifier().to_string();
        self.generators.insert(id, generator);
    }

    pub fn get(&self, generator_identifier: &str) -> Option<&dyn MirrorGenerator> {
        self.generators
            .get(generator_identifier)
            .map(|b| b.as_ref())
    }

    pub fn is_empty(&self) -> bool {
        self.generators.is_empty()
    }

    pub fn len(&self) -> usize {
        self.generators.len()
    }

    pub fn identifiers(&self) -> impl Iterator<Item = &str> {
        self.generators.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::ontology::resource::Resource;

    /// Stub `MirrorGenerator` for tests. The closure walk is a no-op
    /// (returns the seed verbatim); the produced library archive is
    /// a single fixed file. Real generators land in 19b/20b and walk
    /// the chain properly.
    struct StubGenerator {
        identifier: &'static str,
        version: &'static str,
        content_hash: &'static str,
    }

    impl MirrorGenerator for StubGenerator {
        fn generator_identifier(&self) -> &str {
            self.identifier
        }

        fn generator_version(&self) -> &str {
            self.version
        }

        fn generator_content_hash(&self) -> &str {
            self.content_hash
        }

        fn generate(
            &self,
            request: &MirrorGenerationRequest,
        ) -> Result<MirrorGenerationOutput, MirrorGeneratorError> {
            // No real closure walking — just echo the seed, sorted.
            let mut closure = request.seed_classes.to_vec();
            closure.sort_by(|a, b| a.as_str().cmp(b.as_str()));

            let library = LibraryContent::Embedded(vec![LibraryFile {
                path: "src/stub.txt".to_string(),
                content: format!("stub: {} class(es)", closure.len()).into_bytes(),
            }]);

            Ok(MirrorGenerationOutput {
                mirrored_classes: closure,
                library,
            })
        }
    }

    /// `ChainAccessor` impl that always returns None — sufficient for
    /// the stub generator which doesn't actually walk the chain.
    struct EmptyChain;
    impl ChainAccessor for EmptyChain {
        fn resolve(&self, _claim_layer: &Iri, _target: &Iri) -> Option<Resource> {
            None
        }
        fn is_ancestor_or_equal(&self, _anchor: &Iri, _candidate: &Iri) -> bool {
            false
        }
        fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
            false
        }
    }

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn registry_register_and_lookup() {
        let mut reg = MirrorGeneratorRegistry::new();
        reg.register(Box::new(StubGenerator {
            identifier: "eigon-julia-gen",
            version: "0.1.0",
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        }))
        .unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.get("eigon-julia-gen").is_some());
        assert!(reg.get("eigon-python-gen").is_none());
    }

    #[test]
    fn registry_rejects_duplicate() {
        let mut reg = MirrorGeneratorRegistry::new();
        reg.register(Box::new(StubGenerator {
            identifier: "eigon-julia-gen",
            version: "0.1.0",
            content_hash: "sha256:00",
        }))
        .unwrap();
        let err = reg
            .register(Box::new(StubGenerator {
                identifier: "eigon-julia-gen",
                version: "0.2.0",
                content_hash: "sha256:11",
            }))
            .expect_err("duplicate should fail");
        assert!(matches!(err, MirrorRegistryError::AlreadyRegistered(_)));
    }

    #[test]
    fn registry_replace_overwrites() {
        let mut reg = MirrorGeneratorRegistry::new();
        reg.register(Box::new(StubGenerator {
            identifier: "eigon-julia-gen",
            version: "0.1.0",
            content_hash: "sha256:00",
        }))
        .unwrap();
        reg.replace(Box::new(StubGenerator {
            identifier: "eigon-julia-gen",
            version: "0.2.0",
            content_hash: "sha256:11",
        }));
        assert_eq!(reg.len(), 1);
        assert_eq!(
            reg.get("eigon-julia-gen").unwrap().generator_version(),
            "0.2.0"
        );
    }

    #[test]
    fn registry_identifiers_iterates_in_sorted_order() {
        let mut reg = MirrorGeneratorRegistry::new();
        reg.register(Box::new(StubGenerator {
            identifier: "eigon-python-gen",
            version: "0.1.0",
            content_hash: "sha256:00",
        }))
        .unwrap();
        reg.register(Box::new(StubGenerator {
            identifier: "eigon-julia-gen",
            version: "0.1.0",
            content_hash: "sha256:00",
        }))
        .unwrap();
        let ids: Vec<_> = reg.identifiers().collect();
        assert_eq!(ids, vec!["eigon-julia-gen", "eigon-python-gen"]);
    }

    #[test]
    fn generate_returns_seed_as_closure_and_library() {
        let gen = StubGenerator {
            identifier: "eigon-test-gen",
            version: "0.1.0",
            content_hash: "sha256:00",
        };
        let chain = EmptyChain;
        let source_layer = iri("urn:eigenius:test:layer:l0");
        let seed = vec![
            iri("urn:eigenius:test:class:Beta"),
            iri("urn:eigenius:test:class:Alpha"),
        ];
        let request = MirrorGenerationRequest {
            source_layer: &source_layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let out = gen.generate(&request).expect("generate");
        // Sorted closure for determinism.
        assert_eq!(
            out.mirrored_classes,
            vec![
                iri("urn:eigenius:test:class:Alpha"),
                iri("urn:eigenius:test:class:Beta"),
            ]
        );
        match out.library {
            LibraryContent::Embedded(files) => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].path, "src/stub.txt");
                let body = String::from_utf8(files[0].content.clone()).unwrap();
                assert!(body.contains("2 class"));
            }
            other => panic!("expected Embedded, got {other:?}"),
        }
    }

    #[test]
    fn library_content_external_round_trip() {
        let ext = LibraryContent::External {
            reference: "blob://store/abc".to_string(),
            content_hash: "sha256:11".to_string(),
        };
        // Just exercise the variant and confirm equality semantics.
        let same = LibraryContent::External {
            reference: "blob://store/abc".to_string(),
            content_hash: "sha256:11".to_string(),
        };
        assert_eq!(ext, same);
    }
}
