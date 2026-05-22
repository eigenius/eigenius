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

//! Module assembly + integrity hashing for the generated mirror
//! package (D30 §§2, 10).
//!
//! Pairs with the per-class emitters: while `structure_emitter` +
//! `codec_emitter` produce one Lean block per class, this module
//! glues every block into a complete Lake-buildable package:
//!
//! ```text
//! EigeniusFFI/
//! ├── lakefile.lean
//! ├── lean-toolchain
//! └── EigeniusFFI/
//!     ├── Basic.lean
//!     └── Mirror.lean
//! ```
//!
//! The [`assemble_mirror_package`] entry point returns the file
//! list in declaration order — `lakefile.lean`, `lean-toolchain`,
//! `EigeniusFFI/Basic.lean`, `EigeniusFFI/Mirror.lean` — matching
//! D30 §10.1's determinism convention. [`library_content_hash`]
//! then computes the SHA-256 digest per D30 §10.2.

use super::structure_emitter::ClassNameLookup;
use super::{emit_class_block, ClassDecl};
use eigenius_kernel::ontology::iri::Iri;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// One file in the materialised mirror package. Local mirror of
/// `eigenius_runtime_substrate::mirror_generator::LibraryFile` so
/// this module stays self-contained for tests; the trait impl
/// converts to the substrate type at the API boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledFile {
    pub path: String,
    pub content: Vec<u8>,
}

const LAKEFILE_PATH: &str = "lakefile.lean";
const TOOLCHAIN_PATH: &str = "lean-toolchain";
const BASIC_PATH: &str = "EigeniusFFI/Basic.lean";
const MIRROR_PATH: &str = "EigeniusFFI/Mirror.lean";

/// Pinned `EigeniusLeanCommon` Git tag the lakefile depends on
/// (D30 §2.1). The hand-authored package lives at
/// `lean/common/EigeniusLeanCommon/`; this constant is the tag
/// the spec freezes — bumping the common package's version means
/// bumping both this constant and the matching spec section.
const COMMON_PACKAGE_TAG: &str = "v0.1.0";

/// Assemble the complete mirror package as a deterministic list of
/// files. Order matches D30 §10.1 (declaration order: lakefile,
/// toolchain, Basic, Mirror); the same `(decls, order, lookup,
/// source_layer, toolchain)` tuple produces byte-identical output.
pub fn assemble_mirror_package(
    decls: &BTreeMap<Iri, ClassDecl>,
    order: &[Iri],
    lookup: &ClassNameLookup,
    source_layer: &Iri,
    lean_toolchain_version: &str,
) -> Vec<AssembledFile> {
    vec![
        AssembledFile {
            path: LAKEFILE_PATH.to_string(),
            content: lakefile_content().into_bytes(),
        },
        AssembledFile {
            path: TOOLCHAIN_PATH.to_string(),
            content: toolchain_content(lean_toolchain_version).into_bytes(),
        },
        AssembledFile {
            path: BASIC_PATH.to_string(),
            content: basic_module_content().into_bytes(),
        },
        AssembledFile {
            path: MIRROR_PATH.to_string(),
            content: mirror_module_content(decls, order, lookup, source_layer).into_bytes(),
        },
    ]
}

/// D30 §10.2 — SHA-256 over a length-prefixed framing of the
/// archive bytes. Per-file framing: `u64(path.len) || path ||
/// u64(content.len) || content`. Files are sorted by path before
/// hashing so the digest is order-independent at the input level.
pub fn library_content_hash(files: &[AssembledFile]) -> String {
    let mut sorted: Vec<&AssembledFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = Sha256::new();
    for f in sorted {
        hasher.update((f.path.len() as u64).to_be_bytes());
        hasher.update(f.path.as_bytes());
        hasher.update((f.content.len() as u64).to_be_bytes());
        hasher.update(&f.content);
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// D30 §10.3 — `urn:eigenius:runtime:mirror:lean:<first 16 hex
/// of digest>`. Two byte-identical mirrors produce identical IRIs;
/// chain dedupe is intentional.
pub fn derive_mirror_iri(library_content_hash: &str) -> Iri {
    let stripped = library_content_hash
        .strip_prefix("sha256:")
        .unwrap_or(library_content_hash);
    let prefix = &stripped[..16.min(stripped.len())];
    Iri::parse(&format!("urn:eigenius:runtime:mirror:lean:{prefix}"))
        .expect("static IRI shape is well-formed by construction")
}

// ---------------------------------------------------------------------------
// File content emitters
// ---------------------------------------------------------------------------

fn lakefile_content() -> String {
    // D30 §2.1 — pinned package metadata. The require is emitted on
    // a single line so the substrate's `install_mirror` step can
    // sed-rewrite it to a path-require pointing at the in-image
    // baked `EigeniusLeanCommon` (Phase 20a.6.x). D30's prose
    // example uses a two-line layout; that's presentation only,
    // not load-bearing, and the single-line form is the same Lake
    // syntax.
    format!(
        "-- Auto-generated by eigon-ffi-gen — DO NOT EDIT.\n\
         import Lake\n\
         open Lake DSL\n\
         \n\
         package EigeniusFFI where\n\
         \n\
         require EigeniusLeanCommon from git \"https://github.com/eigenius/EigeniusLeanCommon.git\" @ \"{COMMON_PACKAGE_TAG}\"\n\
         \n\
         @[default_target]\n\
         lean_lib EigeniusFFI where\n  \
           roots := #[`EigeniusFFI.Basic, `EigeniusFFI.Mirror]\n"
    )
}

fn toolchain_content(version: &str) -> String {
    // D30 §2.2 — single-line file pinning the Lean toolchain.
    // Trailing newline matches elan's convention.
    let v = version.trim();
    format!("{v}\n")
}

fn basic_module_content() -> String {
    // D30 §2.3 — re-export every EigeniusLeanCommon symbol the
    // Mirror module's generated code refers to. The list reflects
    // the codec emitter's call shape: validators (refinement + runtime),
    // decode helpers, the union type, and the error type.
    "-- Auto-generated by eigon-ffi-gen — DO NOT EDIT.\n\
     import EigeniusLeanCommon\n\
     \n\
     namespace EigeniusFFI\n\
     \n\
     export EigeniusLeanCommon (\n  \
       EigeniusUnion\n  \
       EigenValidationError\n  \
       validateMinValueFloat\n  \
       validateMaxValueFloat\n  \
       validateMinValueInt\n  \
       validateMaxValueInt\n  \
       validateMinLength\n  \
       validateMaxLength\n  \
       validatePattern\n  \
       validateFormat\n  \
       validateOptional\n  \
       withRefinement\n  \
       withOptionalRefinement\n  \
       decodeRequiredPrim\n  \
       decodeOptionalPrim\n  \
       decodeRequiredResource\n  \
       decodeOptionalResource\n  \
       decodeRequiredPrimList\n  \
       decodeRequiredResourceList\n  \
       isAHead\n\
     )\n\
     \n\
     end EigeniusFFI\n"
        .to_string()
}

fn mirror_module_content(
    decls: &BTreeMap<Iri, ClassDecl>,
    order: &[Iri],
    lookup: &ClassNameLookup,
    source_layer: &Iri,
) -> String {
    let mut out = String::new();
    // D30 §2.4 header — auto-generated banner + provenance comments
    // duplicating what the `LeanPackageMirror` resource's
    // `mirrored_classes` and `source_layer` properties carry.
    out.push_str("-- Auto-generated by eigon-ffi-gen — DO NOT EDIT.\n");
    out.push_str("-- Regenerate via the substrate's image-build pipeline.\n");
    out.push_str(&format!("-- source_layer: {}\n", source_layer.as_str()));
    out.push_str("-- mirrored_classes:\n");
    for iri in order {
        out.push_str(&format!("--   - {}\n", iri.as_str()));
    }
    out.push_str("\nimport EigeniusFFI.Basic\n\nnamespace EigeniusFFI\n\n");

    // Per-class blocks in topological order — structure + coercions
    // + decodeC + encodeC (D30 §§6–8).
    for iri in order {
        let decl = decls
            .get(iri)
            .expect("topological order yields only closure members");
        out.push_str(&emit_class_block(decl, decls, lookup));
        out.push('\n');
    }

    // D30 §8.5 — module-level decoder registry. v1 ships the
    // round-trip form (`decodeC >>= encodeC`) keyed by class IRI:
    // a `Lean.Json → Except String Lean.Json` table whose entries
    // validate that the incoming JSON parses as the named class
    // and return its canonical re-encoding. The Sigma-existential
    // form D30 §8.5 sketches lives at `Type 1`, which
    // `Std.HashMap`'s value slot (`Type`) can't hold; the
    // round-trip form is well-typed and gives substrate-side
    // dispatchers the same shape of "decode by IRI" lookup.
    out.push_str(
        "def eigeniusDecoders : Std.HashMap String (Lean.Json → Except String Lean.Json) :=\n",
    );
    out.push_str("  Std.HashMap.ofList [\n");
    for (i, iri) in order.iter().enumerate() {
        let decl = decls
            .get(iri)
            .expect("topological order yields only closure members");
        let trailing = if i + 1 == order.len() { "" } else { "," };
        out.push_str(&format!(
            "    (\"{}\", fun j => encode{name} <$> decode{name} j){trailing}\n",
            iri.as_str(),
            name = decl.short_name,
        ));
    }
    out.push_str("  ]\n\nend EigeniusFFI\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror_gen::{LeanType, PropertyConstraints, PropertyDecl};

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("test IRI")
    }

    fn prop(short: &str, ty: LeanType) -> PropertyDecl {
        PropertyDecl {
            property_iri: iri(&format!("urn:test:{short}")),
            short_name: short.to_string(),
            lean_type: ty,
            constraints: PropertyConstraints::default(),
            description: None,
        }
    }

    fn cls(short: &str, requires: Vec<PropertyDecl>) -> ClassDecl {
        ClassDecl {
            class_iri: iri(&format!("urn:test:{short}")),
            short_name: short.to_string(),
            description: None,
            parents: vec![],
            requires,
            recommends: vec![],
        }
    }

    fn lookup_for(decls: &BTreeMap<Iri, ClassDecl>) -> ClassNameLookup {
        decls
            .iter()
            .map(|(i, d)| (i.clone(), d.short_name.clone()))
            .collect()
    }

    // ─── Determinism ────────────────────────────────────────────────

    #[test]
    fn assembly_is_byte_identical_across_invocations() {
        let person = cls("Person", vec![prop("name", LeanType::String)]);
        let mut decls = BTreeMap::new();
        decls.insert(person.class_iri.clone(), person.clone());
        let lookup = lookup_for(&decls);
        let order = vec![person.class_iri.clone()];
        let layer = iri("urn:test:layer");
        let a =
            assemble_mirror_package(&decls, &order, &lookup, &layer, "leanprover/lean4:v4.29.1");
        let b =
            assemble_mirror_package(&decls, &order, &lookup, &layer, "leanprover/lean4:v4.29.1");
        assert_eq!(
            a, b,
            "D30 §10.1 — same inputs must produce byte-identical output"
        );
    }

    #[test]
    fn library_content_hash_is_stable_under_input_reordering() {
        let f1 = AssembledFile {
            path: "a.lean".to_string(),
            content: b"first".to_vec(),
        };
        let f2 = AssembledFile {
            path: "b.lean".to_string(),
            content: b"second".to_vec(),
        };
        // Sorting happens inside `library_content_hash` so the
        // input order doesn't change the digest.
        let h_ab = library_content_hash(&[f1.clone(), f2.clone()]);
        let h_ba = library_content_hash(&[f2, f1]);
        assert_eq!(h_ab, h_ba);
        assert!(h_ab.starts_with("sha256:"));
        assert_eq!(h_ab.len(), "sha256:".len() + 64);
    }

    #[test]
    fn library_content_hash_changes_on_content_edit() {
        let original = vec![AssembledFile {
            path: "a.lean".to_string(),
            content: b"hello".to_vec(),
        }];
        let edited = vec![AssembledFile {
            path: "a.lean".to_string(),
            content: b"hello!".to_vec(),
        }];
        assert_ne!(
            library_content_hash(&original),
            library_content_hash(&edited),
            "any byte edit must produce a different digest"
        );
    }

    #[test]
    fn derive_mirror_iri_uses_first_16_hex_chars_of_digest() {
        // Pin the IRI shape — D30 §10.3.
        let digest = "sha256:abcdef0123456789ffffffffffffffffffffffffffffffffffffffffffffffff";
        let derived = derive_mirror_iri(digest);
        assert_eq!(
            derived.as_str(),
            "urn:eigenius:runtime:mirror:lean:abcdef0123456789"
        );
    }

    // ─── File contents ──────────────────────────────────────────────

    #[test]
    fn lakefile_pins_common_package_tag_and_lib_roots() {
        let body = lakefile_content();
        assert!(body.contains("package EigeniusFFI"));
        assert!(body.contains("require EigeniusLeanCommon from git"));
        assert!(body.contains(COMMON_PACKAGE_TAG));
        assert!(body.contains("`EigeniusFFI.Basic"));
        assert!(body.contains("`EigeniusFFI.Mirror"));
    }

    #[test]
    fn lakefile_marks_eigenius_ffi_as_default_target() {
        // Without `@[default_target]`, `lake build` with no explicit
        // target reports "0 jobs" and skips the lib (caught by the
        // Phase 20a.6.x in-image build integration test). Pin the
        // attribute here so a regeneration that drops it fails the
        // unit suite, not just the slow Docker e2e.
        let body = lakefile_content();
        assert!(
            body.contains("@[default_target]\nlean_lib EigeniusFFI"),
            "lakefile must mark `lean_lib EigeniusFFI` as @[default_target] \
             so `lake build` picks it up without an explicit target"
        );
    }

    #[test]
    fn toolchain_content_strips_input_whitespace() {
        // The toolchain file is the elan-consumed version pin —
        // any whitespace at the edges must be trimmed.
        assert_eq!(
            toolchain_content("  leanprover/lean4:v4.29.1  \n"),
            "leanprover/lean4:v4.29.1\n"
        );
    }

    #[test]
    fn basic_module_exports_every_helper_the_emitter_calls() {
        let body = basic_module_content();
        // Spot-check key symbols from each helper family.
        for sym in [
            "EigeniusUnion",
            "withRefinement",
            "withOptionalRefinement",
            "validateOptional",
            "validatePattern",
            "validateFormat",
            "decodeRequiredPrim",
            "decodeOptionalPrim",
            "decodeRequiredResource",
            "decodeOptionalResource",
            "isAHead",
        ] {
            assert!(body.contains(sym), "Basic.lean must export `{sym}`");
        }
    }

    #[test]
    fn mirror_module_emits_header_provenance_and_per_class_blocks() {
        let person = cls("Person", vec![prop("name", LeanType::String)]);
        let mut decls = BTreeMap::new();
        decls.insert(person.class_iri.clone(), person.clone());
        let lookup = lookup_for(&decls);
        let layer = iri("urn:test:layer");
        let body = mirror_module_content(
            &decls,
            std::slice::from_ref(&person.class_iri),
            &lookup,
            &layer,
        );

        // Provenance comments — D30 §2.4.
        assert!(body.contains("-- source_layer: urn:test:layer"));
        assert!(body.contains("-- mirrored_classes:"));
        assert!(body.contains("--   - urn:test:Person"));

        // Imports + namespace open + close.
        assert!(body.contains("import EigeniusFFI.Basic"));
        assert!(body.contains("namespace EigeniusFFI"));
        assert!(body.trim_end().ends_with("end EigeniusFFI"));

        // Per-class block present.
        assert!(body.contains("structure Person where"));
        assert!(body.contains("def decodePerson"));
        assert!(body.contains("def encodePerson"));

        // Registry entry.
        assert!(body.contains("def eigeniusDecoders"));
        assert!(body.contains("(\"urn:test:Person\", fun j => encodePerson <$> decodePerson j)"));
    }
}
