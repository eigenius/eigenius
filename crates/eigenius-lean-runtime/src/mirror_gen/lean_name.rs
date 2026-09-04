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

//! The chain IRI → Lean `Name` map (D74 §3.3, eigenius#208).
//!
//! **This is the single authority.** D30's emitter spells a class's Lean name through here, and
//! D74's externalizer reads it back through here, so the two agree by construction rather than
//! by lookup. Two implementations of "the same" mangling is the naming disagreement D74 §3.3
//! exists to prevent, and it would surface as a `def_eq` mismatch that gives no indication
//! naming was the cause.
//!
//! # Why qualification at all
//!
//! D30 §7.1 emitted `structure <short_name>` flat inside `namespace EigeniusFFI`, so a class's
//! Lean name was its `core:short_name`. That is not injective: measured over every ontology in
//! the repo on `2026-08-22`, 948 class short_names carried 8 collisions — `Person`
//! (`reflection:` and `schema_org:`), `Organization`, `Axiom`, `Observation`, `DecisionPoint`,
//! `CutItem`, `Hypothesis`, `Map`. A closure holding both members of a pair emitted two
//! `structure Person` declarations into one namespace.
//!
//! A non-injective map means externalization can name the wrong class, which is D74 §5's
//! failure mode: proving the wrong theorem soundly.
//!
//! # The mapping
//!
//! The IRI's namespace segments become Lean namespace components, and `core:short_name` is the
//! declaration name:
//!
//! ```text
//! urn:eigenius:reflection:Person  + short_name "Person"  ->  eigenius.reflection.Person
//! urn:schema_org:Person           + short_name "Person"  ->  schema_org.Person
//! ```
//!
//! Relative to `EigeniusFFI`, which the emitted module opens as a single block, so the full
//! Lean names are `EigeniusFFI.eigenius.reflection.Person` and `EigeniusFFI.schema_org.Person`.
//!
//! The `urn` scheme segment is dropped — every chain IRI carries it, so it distinguishes
//! nothing and only lengthens every name.
//!
//! # Why `short_name` and not the IRI's local name
//!
//! D74 §6.1 settles the namespace half on injectivity-by-construction, and the local name would
//! extend that reasoning to the declaration half. It is not the better choice here. Measured
//! over the same corpus: of 928 classes carrying a `short_name`, 8 have a local name that
//! differs, and every one is a deliberate disambiguation for the flat space —
//! `program:components:completion:Arguments` declares `CompletionArguments`. Namespacing makes
//! those overrides redundant but not wrong, and discarding them renames classes for no gain.
//!
//! `short_name` also already carries the validation the local name does not:
//! `validate_class_identifier` checks it is a Lean identifier starting with a capital. Local
//! names have no such guarantee — eigenius#31 would admit `assay:EIG-0042`, and a hyphen is not
//! a Lean identifier character.
//!
//! Injectivity is therefore *enforced* rather than structural: [`check_injective`] rejects a
//! closure in which two classes map to one Lean name. Measured over the corpus that check finds
//! 0 violations today, which is what makes it cheap to require rather than a migration.

use std::collections::BTreeMap;

use eigenius_kernel::ontology::iri::Iri;

/// The Lean package every mirrored class lives under. The emitted module opens this as one
/// `namespace` block and declares dotted names inside it, so the qualified names this module
/// produces are relative to it.
pub const MIRROR_ROOT: &str = "EigeniusFFI";

/// The URN scheme segment, dropped from every path — it distinguishes nothing.
const URN_SCHEME: &str = "urn";

/// The namespace path of `iri`, as Lean namespace components.
///
/// `urn:eigenius:reflection:Person` yields `["eigenius", "reflection"]`. The local name is not
/// included — [`class_lean_name`] appends the `short_name` in its place.
pub fn namespace_path(iri: &Iri) -> Vec<&str> {
    let ns = iri.namespace();
    // `Iri::namespace` keeps the trailing separator; the split then yields a final empty
    // segment, which `filter` drops along with the scheme.
    ns.split(':')
        .filter(|s| !s.is_empty() && *s != URN_SCHEME)
        .collect()
}

/// The Lean name for a class, relative to [`MIRROR_ROOT`].
///
/// This is the function both sides call — see the module docs. A class with no namespace
/// segments (an IRI that is a bare local name) yields the `short_name` alone, which is the
/// pre-#208 behaviour and stays correct for it.
pub fn class_lean_name(iri: &Iri, short_name: &str) -> String {
    let mut parts = namespace_path(iri);
    parts.push(short_name);
    parts.join(".")
}

/// The fully-qualified Lean name, including [`MIRROR_ROOT`].
///
/// What appears in a `lean4export` payload's `Const` nodes, and so what D74's externalizer
/// must produce. The emitter writes the relative form from [`class_lean_name`] because it
/// declares inside the `namespace EigeniusFFI` block.
pub fn class_lean_name_absolute(iri: &Iri, short_name: &str) -> String {
    format!("{MIRROR_ROOT}.{}", class_lean_name(iri, short_name))
}

/// The Lean name of a codec function — `decodePerson` becomes
/// `eigenius.reflection.decodePerson`.
///
/// Codecs are `def`s in the same namespace as the structure they encode, so a flat
/// `decodePerson` collides exactly as a flat `structure Person` does. The `verb` is
/// `"decode"` or `"encode"`.
pub fn codec_lean_name(iri: &Iri, short_name: &str, verb: &str) -> String {
    let mut parts = namespace_path(iri);
    let leaf = format!("{verb}{short_name}");
    parts.push(&leaf);
    parts.join(".")
}

/// The codec name for a class already rendered as a Lean type name.
///
/// `("eigenius.reflection.Person", "decode")` yields
/// `"eigenius.reflection.decodePerson"`. The emitters thread the rendered type name (through
/// `ClassNameLookup`) rather than the `(Iri, short_name)` pair, and a codec `def` must land in
/// the same namespace as its structure — so the verb goes before the *leaf*, not before the
/// path. Concatenating verb and qualified name would produce `decodeeigenius.reflection.Person`,
/// which names nothing.
pub fn codec_name_for_type(type_name: &str, verb: &str) -> String {
    match split_lean_name(type_name) {
        Some((path, leaf)) => {
            let mut parts = path;
            let leaf = format!("{verb}{leaf}");
            parts.push(&leaf);
            parts.join(".")
        }
        // No namespace component — a bare-local-name IRI. Pre-#208 behaviour, still correct.
        None => format!("{verb}{type_name}"),
    }
}

/// The declaration name at the end of a (possibly qualified) Lean name.
///
/// `"eigenius.reflection.Person"` yields `"Person"`. Needed wherever a name is used to *build a
/// new identifier* rather than to refer to a declaration, because a dotted name is a path and
/// cannot appear inside one:
///
/// - Lean's `extends` projection. Verified against Lean 4: `structure eigenius.reflection.Employee
///   extends eigenius.reflection.Person` generates `Employee.toPerson`, keyed on the parent's last
///   component, not its path.
/// - Local `let` bindings the codec emitter names after a parent.
pub fn leaf_of(type_name: &str) -> &str {
    type_name.rsplit('.').next().unwrap_or(type_name)
}

/// A Lean name this mangling produced, split back into its namespace path and declaration
/// name. The inverse of [`class_lean_name`], for a reader holding a name from an export.
///
/// Returns `None` for a name with no namespace component, which this mangling only produces
/// for a bare-local-name IRI and which an export's own Lean-side constants (`Nat`, `Eq`) look
/// like. Accepts either the relative or the [`MIRROR_ROOT`]-qualified form.
pub fn split_lean_name(name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = name
        .strip_prefix(&format!("{MIRROR_ROOT}."))
        .unwrap_or(name);
    let mut parts: Vec<&str> = rest.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let leaf = parts.pop()?;
    Some((parts, leaf))
}

/// Reject a closure whose classes do not map injectively to Lean names.
///
/// The check #208 asks for, and what makes the choice of `short_name` over the IRI's local name
/// safe: with namespace qualification it finds 0 violations across every ontology in the repo,
/// so requiring it costs nothing today and it is what stops a future collision from reaching
/// the emitter.
///
/// `classes` is `(IRI, short_name)`. On a collision, returns the Lean name and the two IRIs
/// that produced it, lowest-IRI-first so the diagnostic is deterministic.
pub fn check_injective<'a, I>(classes: I) -> Result<(), (String, Iri, Iri)>
where
    I: IntoIterator<Item = (&'a Iri, &'a str)>,
{
    let mut seen: BTreeMap<String, &Iri> = BTreeMap::new();
    for (iri, short) in classes {
        let name = class_lean_name(iri, short);
        if let Some(prev) = seen.get(&name) {
            let (a, b) = if prev.as_str() <= iri.as_str() {
                ((*prev).clone(), iri.clone())
            } else {
                (iri.clone(), (*prev).clone())
            };
            return Err((name, a, b));
        }
        seen.insert(name, iri);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("static IRI")
    }

    /// The two examples D74 §3.3.1 gives, which are the pair that motivated #208.
    #[test]
    fn the_colliding_person_pair_separates() {
        let a = iri("urn:eigenius:reflection:Person");
        let b = iri("urn:schema_org:Person");
        assert_eq!(class_lean_name(&a, "Person"), "eigenius.reflection.Person");
        assert_eq!(class_lean_name(&b, "Person"), "schema_org.Person");
        assert_ne!(class_lean_name(&a, "Person"), class_lean_name(&b, "Person"));
    }

    #[test]
    fn the_urn_scheme_is_dropped() {
        assert!(!namespace_path(&iri("urn:eigenius:core:Thing")).contains(&"urn"));
        assert_eq!(
            namespace_path(&iri("urn:eigenius:core:Thing")),
            vec!["eigenius", "core"]
        );
    }

    /// `short_name` is the declaration name, not the IRI's local name — the
    /// `CompletionArguments` case from the module docs.
    #[test]
    fn a_short_name_override_is_honoured() {
        let i = iri("urn:eigenius:program:components:completion:Arguments");
        assert_eq!(
            class_lean_name(&i, "CompletionArguments"),
            "eigenius.program.components.completion.CompletionArguments"
        );
    }

    #[test]
    fn the_absolute_form_carries_the_package() {
        assert_eq!(
            class_lean_name_absolute(&iri("urn:eigenius:reflection:Person"), "Person"),
            "EigeniusFFI.eigenius.reflection.Person"
        );
    }

    /// A codec `def` sits beside its structure, so it qualifies the same way — a flat
    /// `decodePerson` collides exactly as a flat `structure Person` does.
    #[test]
    fn codecs_qualify_with_their_structure() {
        let a = iri("urn:eigenius:reflection:Person");
        let b = iri("urn:schema_org:Person");
        assert_eq!(
            codec_lean_name(&a, "Person", "decode"),
            "eigenius.reflection.decodePerson"
        );
        assert_ne!(
            codec_lean_name(&a, "Person", "decode"),
            codec_lean_name(&b, "Person", "decode")
        );
    }

    /// The emitters hold a rendered type name, not the `(Iri, short_name)` pair, so the two
    /// routes to a codec name must agree.
    #[test]
    fn the_two_routes_to_a_codec_name_agree() {
        let i = iri("urn:eigenius:reflection:Person");
        let type_name = class_lean_name(&i, "Person");
        assert_eq!(
            codec_name_for_type(&type_name, "decode"),
            codec_lean_name(&i, "Person", "decode")
        );
        assert_eq!(
            codec_name_for_type(&type_name, "encode"),
            codec_lean_name(&i, "Person", "encode")
        );
    }

    /// The verb goes before the leaf. Concatenating it onto the qualified name would produce
    /// `decodeeigenius.reflection.Person`, which names nothing.
    #[test]
    fn the_verb_lands_before_the_leaf_not_the_path() {
        assert_eq!(
            codec_name_for_type("eigenius.reflection.Person", "decode"),
            "eigenius.reflection.decodePerson"
        );
    }

    #[test]
    fn split_inverts_the_mangling() {
        let i = iri("urn:eigenius:reflection:Person");
        let name = class_lean_name(&i, "Person");
        let (path, leaf) = split_lean_name(&name).expect("mangled names split");
        assert_eq!(path, vec!["eigenius", "reflection"]);
        assert_eq!(leaf, "Person");

        // And through the absolute form, which is what an export carries.
        let abs = class_lean_name_absolute(&i, "Person");
        let (path, leaf) = split_lean_name(&abs).expect("absolute names split");
        assert_eq!(path, vec!["eigenius", "reflection"]);
        assert_eq!(leaf, "Person");
    }

    /// Lean's own constants have no namespace component and must not be mistaken for
    /// mirror references.
    /// Pinned against Lean 4 rather than assumed: `structure A.B.Employee extends A.B.Person`
    /// generates `toPerson`, keyed on the last component.
    #[test]
    fn leaf_of_is_what_an_extends_projection_uses() {
        assert_eq!(leaf_of("eigenius.reflection.Person"), "Person");
        assert_eq!(leaf_of("Person"), "Person");
    }

    #[test]
    fn a_bare_name_does_not_split() {
        assert!(split_lean_name("Nat").is_none());
        assert!(split_lean_name("Eq").is_none());
    }

    #[test]
    fn injectivity_holds_for_the_pair_that_collided_flat() {
        let a = iri("urn:eigenius:reflection:Person");
        let b = iri("urn:schema_org:Person");
        check_injective([(&a, "Person"), (&b, "Person")]).expect("namespaces separate them");
    }

    /// Two classes in ONE namespace sharing a `short_name` is what qualification cannot fix,
    /// and is exactly what the check is for. This is why `short_name` is admissible as the
    /// declaration name: the residual risk is checked rather than assumed.
    #[test]
    fn same_namespace_same_short_name_is_rejected() {
        let a = iri("urn:eigenius:reflection:Alpha");
        let b = iri("urn:eigenius:reflection:Beta");
        let (name, first, second) =
            check_injective([(&a, "Same"), (&b, "Same")]).expect_err("must reject");
        assert_eq!(name, "eigenius.reflection.Same");
        assert_eq!(first.as_str(), "urn:eigenius:reflection:Alpha");
        assert_eq!(second.as_str(), "urn:eigenius:reflection:Beta");
    }

    /// The diagnostic names the same two IRIs whichever order they arrive in.
    #[test]
    fn the_collision_diagnostic_is_order_independent() {
        let a = iri("urn:eigenius:reflection:Alpha");
        let b = iri("urn:eigenius:reflection:Beta");
        let forward = check_injective([(&a, "Same"), (&b, "Same")]).expect_err("must reject");
        let reverse = check_injective([(&b, "Same"), (&a, "Same")]).expect_err("must reject");
        assert_eq!(forward, reverse);
    }
}
