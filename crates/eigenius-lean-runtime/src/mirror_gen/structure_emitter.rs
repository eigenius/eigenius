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

//! Lean `structure` + coercion-instance emitter (D30 §§5–7).
//!
//! The closure walker resolves each class into a [`ClassDecl`]; the
//! topological sort orders them so a structure's referenced classes
//! are declared earlier in the module. This file turns one resolved
//! `ClassDecl` into the two D30 §6 emission steps that don't touch
//! JSON: the `structure` declaration and the `CoeOut` instances.
//!
//! ## Coverage
//!
//! - §5 field ordering: parent-extends slot first (Lean's `extends`
//!   places inherited fields ahead of own fields), then required in
//!   chain-declared order, then recommended.
//! - §7.1 deriving: `Repr` always. `Inhabited` is *omitted* in v1 —
//!   D30 says "when derivable", but determining derivability
//!   transitively requires a fixpoint pass over the class graph
//!   (every required field type must itself be `Inhabited`). The
//!   safer move per D30 §7.1 is to skip; users who need
//!   `Inhabited` for a specific mirror type declare it themselves.
//! - §7.2 reserved `_id`: only declared on root classes (no
//!   parents). Subclasses inherit through `extends`. Multi-supertype
//!   with two `_id`-carrying parents is the closure walker's
//!   duplicate-field check's responsibility once that case appears
//!   in a chain — for now no test exercises it.
//! - §7.3 empty class: still gets `_id` + `deriving Repr`.
//! - §7.5 derived instances: explicit `CoeOut C P` per parent. The
//!   `extends` mechanism gives the `Coe C P` direction implicitly;
//!   the explicit `CoeOut` covers the dual.

use super::{ClassDecl, LeanType, PropertyDecl};
use eigenius_kernel::ontology::iri::Iri;
use std::collections::BTreeMap;

/// Render the Lean predicate body of a refinement subtype for a
/// constraint-carrying property — `0.0 ≤ x ∧ x ≤ 100.0` shape.
/// Returns `None` when the property has no constraints that lift
/// to refinements (D30 §9.1: min/max value for numerics,
/// min/max length for strings; pattern/format stay runtime-only
/// per §9.2).
///
/// Predicate ordering follows D30 §9.1: min-value, max-value,
/// min-length, max-length. Pinned in the rendered output so the
/// emitter's structure-decl and codec-decl render in lockstep.
pub(crate) fn refinement_predicate(prop: &PropertyDecl) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let c = &prop.constraints;
    if let Some(lo) = c.min_value {
        match &prop.lean_type {
            LeanType::Float => parts.push(format!("{} ≤ x", float_literal(lo))),
            LeanType::Int => parts.push(format!("{} ≤ x", lo as i64)),
            _ => {}
        }
    }
    if let Some(hi) = c.max_value {
        match &prop.lean_type {
            LeanType::Float => parts.push(format!("x ≤ {}", float_literal(hi))),
            LeanType::Int => parts.push(format!("x ≤ {}", hi as i64)),
            _ => {}
        }
    }
    if matches!(prop.lean_type, LeanType::String) {
        if let Some(lo) = c.min_length {
            parts.push(format!("{lo} ≤ x.length"));
        }
        if let Some(hi) = c.max_length {
            parts.push(format!("x.length ≤ {hi}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" ∧ "))
    }
}

/// Render the field type for `prop`. Wraps in `{ x : T // pred }`
/// when a refinement applies; otherwise returns the bare
/// `render_lean_type` output.
pub(crate) fn render_field_type(prop: &PropertyDecl, lookup: &ClassNameLookup) -> String {
    let base = render_lean_type(&prop.lean_type, lookup);
    match refinement_predicate(prop) {
        Some(pred) => format!("{{ x : {base} // {pred} }}"),
        None => base,
    }
}

/// Float-literal renderer shared between emitters — duplicates the
/// `lean_float_literal` in `codec_emitter` so each module is
/// self-contained against `PropertyDecl` access patterns. Rust's
/// `{:?}` on `f64` keeps a fractional digit so `0` renders as
/// `0.0` (Lean Float literal syntax, D30 §9.3).
fn float_literal(v: f64) -> String {
    format!("{v:?}")
}

/// Lookup from class IRI to Lean `short_name`. Built by the
/// generator once and threaded into every emitter step so type
/// rendering doesn't re-walk the resolution layer.
pub type ClassNameLookup = BTreeMap<Iri, String>;

/// Build the IRI→`short_name` lookup table from a set of resolved
/// declarations. Convenience for tests and the eventual top-level
/// emitter — both need the same shape.
pub(crate) fn class_name_lookup(decls: &BTreeMap<Iri, ClassDecl>) -> ClassNameLookup {
    decls
        .iter()
        .map(|(iri, d)| (iri.clone(), d.short_name.clone()))
        .collect()
}

/// Render a single Lean type expression (D30 §4 table). Class refs
/// resolve to their `short_name` via `lookup`; unresolved IRIs panic
/// — every class in a `LeanType::ClassRef`/`ListClassRef`/`Union`
/// position is in the closure by construction (`walk_closure` +
/// `resolve_class_types` enforce this), so a miss is an internal
/// invariant violation rather than a user-facing error.
pub(crate) fn render_lean_type(t: &LeanType, lookup: &ClassNameLookup) -> String {
    match t {
        LeanType::String => "String".to_string(),
        LeanType::Int => "Int".to_string(),
        LeanType::Float => "Float".to_string(),
        LeanType::Bool => "Bool".to_string(),
        LeanType::Json => "Lean.Json".to_string(),
        LeanType::ClassRef(iri) => lookup_or_panic(lookup, iri),
        LeanType::Union(iris) => format!("EigeniusUnion [{}]", join_class_names(iris, lookup)),
        LeanType::ListClassRef(iri) => format!("List {}", lookup_or_panic(lookup, iri)),
        LeanType::ListUnion(iris) => {
            format!("List (EigeniusUnion [{}])", join_class_names(iris, lookup))
        }
        LeanType::ListPrimitive(inner) => format!("List {}", render_lean_type(inner, lookup)),
    }
}

fn lookup_or_panic(lookup: &ClassNameLookup, iri: &Iri) -> String {
    lookup.get(iri).cloned().unwrap_or_else(|| {
        panic!(
            "class `{}` not in name lookup — closure-walk invariant violated",
            iri.as_str()
        )
    })
}

fn join_class_names(iris: &[Iri], lookup: &ClassNameLookup) -> String {
    iris.iter()
        .map(|i| lookup_or_panic(lookup, i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Emit one class's `structure` declaration and its `CoeOut`
/// instances. Output ends with a single trailing newline; the
/// caller (the per-class block writer) adds the blank-line gap to
/// the next class's emission.
pub(crate) fn emit_structure_block(decl: &ClassDecl, lookup: &ClassNameLookup) -> String {
    let mut out = String::new();
    push_structure(&mut out, decl, lookup);
    push_coercions(&mut out, decl, lookup);
    out
}

fn push_structure(out: &mut String, decl: &ClassDecl, lookup: &ClassNameLookup) {
    if let Some(d) = &decl.description {
        push_docstring(out, d);
    }
    out.push_str("structure ");
    out.push_str(&decl.short_name);
    if !decl.parents.is_empty() {
        out.push_str(" extends ");
        out.push_str(&join_class_names(&decl.parents, lookup));
    }
    out.push_str(" where\n");

    if decl.parents.is_empty() {
        // D30 §7.2 — `_id` lives at the top of every root structure.
        // Subclasses inherit it through `extends` and don't redeclare.
        out.push_str("  _id : Option String := none\n");
    }

    for prop in &decl.requires {
        push_field(out, prop, lookup, false);
    }
    for prop in &decl.recommends {
        push_field(out, prop, lookup, true);
    }

    out.push_str("  deriving Repr\n");
}

fn push_coercions(out: &mut String, decl: &ClassDecl, lookup: &ClassNameLookup) {
    for parent_iri in &decl.parents {
        let parent_name = lookup_or_panic(lookup, parent_iri);
        // Lean's `extends P` auto-generates a projection `c.toP : P`;
        // the explicit `CoeOut` ties it to coercion-resolution
        // unification (D30 §4.2 — the direction `extends` doesn't
        // give us automatically).
        out.push_str(&format!(
            "\ninstance : CoeOut {} {} where\n",
            decl.short_name, parent_name
        ));
        out.push_str(&format!("  coe c := c.to{}\n", parent_name));
    }
}

fn push_field(out: &mut String, prop: &PropertyDecl, lookup: &ClassNameLookup, optional: bool) {
    if let Some(d) = &prop.description {
        out.push_str("  ");
        push_docstring_inline(out, d);
    }
    out.push_str("  ");
    out.push_str(&prop.short_name);
    out.push_str(" : ");
    // D30 §9.1 — refinement-constrained fields render as a subtype
    // (`{ x : T // pred }`) at the structure-declaration level.
    // `render_field_type` returns the bare type when no refinement
    // applies, so unconstrained fields keep the v1 shape.
    let ty = render_field_type(prop, lookup);
    if optional {
        out.push_str(&format!("Option ({}) := none", ty));
    } else {
        out.push_str(&ty);
    }
    out.push('\n');
}

fn push_docstring(out: &mut String, raw: &str) {
    out.push_str("/-- ");
    out.push_str(&escape_docstring(raw));
    out.push_str(" -/\n");
}

/// Same as `push_docstring` but the caller has already written the
/// indent. Used for per-field docstrings inside a `where` clause.
fn push_docstring_inline(out: &mut String, raw: &str) {
    out.push_str("/-- ");
    out.push_str(&escape_docstring(raw));
    out.push_str(" -/\n");
}

/// Escape characters Lean's docstring (`/-- … -/`) lexer can't
/// tolerate. The only forbidden sequence inside `/-- … -/` is `-/`
/// (which would close the comment early); we split it via a
/// zero-width-equivalent insert that the renderer (`leandoc` /
/// `docgen4`) handles cleanly. Newlines and other characters pass
/// through verbatim — Lean docstrings are free-form Markdown.
fn escape_docstring(s: &str) -> String {
    s.replace("-/", "- /")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror_gen::{ClassDecl, LeanType, PropertyConstraints, PropertyDecl};

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

    fn cls(
        short: &str,
        parents: Vec<Iri>,
        requires: Vec<PropertyDecl>,
        recommends: Vec<PropertyDecl>,
    ) -> ClassDecl {
        ClassDecl {
            class_iri: iri(&format!("urn:test:{short}")),
            short_name: short.to_string(),
            description: None,
            parents,
            requires,
            recommends,
        }
    }

    fn lookup_for(decls: &[&ClassDecl]) -> ClassNameLookup {
        decls
            .iter()
            .map(|d| (d.class_iri.clone(), d.short_name.clone()))
            .collect()
    }

    // ─── type rendering ────────────────────────────────────────────

    #[test]
    fn render_primitives_use_lean_canonical_names() {
        let lookup = ClassNameLookup::new();
        assert_eq!(render_lean_type(&LeanType::String, &lookup), "String");
        assert_eq!(render_lean_type(&LeanType::Int, &lookup), "Int");
        assert_eq!(render_lean_type(&LeanType::Float, &lookup), "Float");
        assert_eq!(render_lean_type(&LeanType::Bool, &lookup), "Bool");
        assert_eq!(render_lean_type(&LeanType::Json, &lookup), "Lean.Json");
    }

    #[test]
    fn render_classref_resolves_via_lookup() {
        let mut lookup = ClassNameLookup::new();
        lookup.insert(iri("urn:test:Person"), "Person".to_string());
        assert_eq!(
            render_lean_type(&LeanType::ClassRef(iri("urn:test:Person")), &lookup),
            "Person"
        );
        assert_eq!(
            render_lean_type(&LeanType::ListClassRef(iri("urn:test:Person")), &lookup),
            "List Person"
        );
    }

    #[test]
    fn render_union_and_list_union_use_eigeniusunion() {
        let mut lookup = ClassNameLookup::new();
        lookup.insert(iri("urn:test:A"), "A".to_string());
        lookup.insert(iri("urn:test:B"), "B".to_string());
        let union = LeanType::Union(vec![iri("urn:test:A"), iri("urn:test:B")]);
        let list_union = LeanType::ListUnion(vec![iri("urn:test:A"), iri("urn:test:B")]);
        assert_eq!(render_lean_type(&union, &lookup), "EigeniusUnion [A, B]");
        assert_eq!(
            render_lean_type(&list_union, &lookup),
            "List (EigeniusUnion [A, B])"
        );
    }

    #[test]
    fn render_list_primitive_recurses_into_inner() {
        let lookup = ClassNameLookup::new();
        let v = LeanType::ListPrimitive(Box::new(LeanType::String));
        assert_eq!(render_lean_type(&v, &lookup), "List String");
    }

    // ─── structure shape ───────────────────────────────────────────

    #[test]
    fn empty_root_class_renders_with_id_only_and_repr() {
        // D30 §7.3 — empty class still has `_id` and `deriving Repr`.
        let c = cls("Empty", vec![], vec![], vec![]);
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        assert!(out.contains("structure Empty where\n"));
        assert!(out.contains("  _id : Option String := none\n"));
        assert!(out.contains("  deriving Repr\n"));
        // No parents → no CoeOut.
        assert!(!out.contains("CoeOut"));
    }

    #[test]
    fn root_class_with_required_field_renders_field_after_id() {
        let c = cls(
            "Person",
            vec![],
            vec![prop("name", LeanType::String)],
            vec![],
        );
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        let id_pos = out.find("_id").expect("_id present");
        let name_pos = out.find("name : String").expect("name field present");
        assert!(id_pos < name_pos, "D30 §7.2: _id must precede own fields");
        assert!(out.contains("  name : String\n"));
    }

    #[test]
    fn recommended_field_is_optional_with_none_default() {
        let c = cls(
            "Person",
            vec![],
            vec![],
            vec![prop("nickname", LeanType::String)],
        );
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        // Parens around the inner type so compound forms like
        // `List String` / `{ x : T // pred }` parse correctly when
        // composed with `Option …`.
        assert!(out.contains("  nickname : Option (String) := none\n"));
    }

    #[test]
    fn required_fields_emit_in_declared_order() {
        // D30 §5 — within `requires`, the order is the chain's order.
        let c = cls(
            "Person",
            vec![],
            vec![
                prop("first", LeanType::String),
                prop("middle", LeanType::String),
                prop("last", LeanType::String),
            ],
            vec![],
        );
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        let first = out.find("first : String").expect("first");
        let middle = out.find("middle : String").expect("middle");
        let last = out.find("last : String").expect("last");
        assert!(first < middle && middle < last);
    }

    #[test]
    fn required_emits_before_recommended() {
        let c = cls(
            "Person",
            vec![],
            vec![prop("name", LeanType::String)],
            vec![prop("nickname", LeanType::String)],
        );
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        let name = out.find("name : String").expect("name");
        let nick = out.find("nickname :").expect("nickname");
        assert!(name < nick, "required must precede recommended (D30 §5)");
    }

    #[test]
    fn structure_with_classref_field_uses_referenced_short_name() {
        let person = cls("Person", vec![], vec![], vec![]);
        let doc = cls(
            "Doc",
            vec![],
            vec![prop("author", LeanType::ClassRef(iri("urn:test:Person")))],
            vec![],
        );
        let lookup = lookup_for(&[&person, &doc]);
        let out = emit_structure_block(&doc, &lookup);
        assert!(
            out.contains("  author : Person\n"),
            "classref field must render the referenced class's Lean name; got:\n{out}"
        );
    }

    #[test]
    fn structure_with_union_field_uses_eigeniusunion_with_canonical_order() {
        // class_types is BTreeSet-sorted at resolution time, so the
        // emitter sees a canonically-ordered Vec<Iri>.
        let a = cls("Apple", vec![], vec![], vec![]);
        let z = cls("Zebra", vec![], vec![], vec![]);
        let doc = cls(
            "Doc",
            vec![],
            vec![prop(
                "contributor",
                LeanType::Union(vec![iri("urn:test:Apple"), iri("urn:test:Zebra")]),
            )],
            vec![],
        );
        let lookup = lookup_for(&[&a, &z, &doc]);
        let out = emit_structure_block(&doc, &lookup);
        assert!(out.contains("  contributor : EigeniusUnion [Apple, Zebra]\n"));
    }

    #[test]
    fn structure_with_list_primitive_field_renders_list_inner() {
        let c = cls(
            "Person",
            vec![],
            vec![prop(
                "tags",
                LeanType::ListPrimitive(Box::new(LeanType::String)),
            )],
            vec![],
        );
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        assert!(out.contains("  tags : List String\n"));
    }

    // ─── inheritance ───────────────────────────────────────────────

    #[test]
    fn class_with_parent_extends_and_omits_id() {
        let parent = cls("Animal", vec![], vec![], vec![]);
        let child = cls(
            "Dog",
            vec![iri("urn:test:Animal")],
            vec![prop("breed", LeanType::String)],
            vec![],
        );
        let lookup = lookup_for(&[&parent, &child]);
        let out = emit_structure_block(&child, &lookup);
        assert!(out.contains("structure Dog extends Animal where\n"));
        // Inherited from parent via extends — child must NOT redeclare.
        assert!(
            !out.contains("Dog where\n  _id"),
            "child must inherit _id through extends rather than redeclaring"
        );
        // Coercion instance present.
        assert!(out.contains("instance : CoeOut Dog Animal where\n"));
        assert!(out.contains("  coe c := c.toAnimal\n"));
    }

    #[test]
    fn class_with_multiple_parents_extends_each_and_emits_coercion_per_parent() {
        // Lean supports multi-supertype natively (D30 §3.2). The
        // emitter just lists every parent in `extends` order and
        // produces one `CoeOut` per.
        let a = cls("A", vec![], vec![], vec![]);
        let b = cls("B", vec![], vec![], vec![]);
        let c = cls(
            "C",
            vec![iri("urn:test:A"), iri("urn:test:B")],
            vec![],
            vec![],
        );
        let lookup = lookup_for(&[&a, &b, &c]);
        let out = emit_structure_block(&c, &lookup);
        assert!(out.contains("structure C extends A, B where\n"));
        assert!(out.contains("instance : CoeOut C A where\n"));
        assert!(out.contains("  coe c := c.toA\n"));
        assert!(out.contains("instance : CoeOut C B where\n"));
        assert!(out.contains("  coe c := c.toB\n"));
    }

    // ─── docstrings ────────────────────────────────────────────────

    #[test]
    fn class_description_renders_as_docstring_above_structure() {
        let mut c = cls("Person", vec![], vec![], vec![]);
        c.description = Some("A person resource carried on the chain.".to_string());
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        let docstring_pos = out.find("/-- A person resource").expect("docstring opens");
        let structure_pos = out.find("structure Person").expect("structure");
        assert!(docstring_pos < structure_pos);
        assert!(out.contains(" -/\n"), "docstring must close");
    }

    #[test]
    fn property_description_renders_as_docstring_above_field() {
        let mut name = prop("name", LeanType::String);
        name.description = Some("The person's full legal name.".to_string());
        let c = cls("Person", vec![], vec![name], vec![]);
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        let docstring_pos = out
            .find("/-- The person's full legal name.")
            .expect("field docstring opens");
        let field_pos = out.find("name : String").expect("name field");
        assert!(docstring_pos < field_pos);
    }

    #[test]
    fn description_with_close_comment_marker_is_escaped() {
        // `-/` inside a Lean docstring closes the comment early.
        // The emitter must rewrite `-/` to `- /` so the docstring
        // body is lexed as content rather than as a close-comment.
        let mut c = cls("Person", vec![], vec![], vec![]);
        c.description = Some("Contains a fragment like a-/b in prose.".to_string());
        let lookup = lookup_for(&[&c]);
        let out = emit_structure_block(&c, &lookup);
        assert!(
            !out.contains("a-/b"),
            "raw `-/` must not survive into the docstring; got:\n{out}"
        );
        assert!(out.contains("a- /b"));
    }
}
