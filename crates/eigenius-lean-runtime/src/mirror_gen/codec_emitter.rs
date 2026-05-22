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

//! Lean JSON codec emitter (D30 §8).
//!
//! Pairs with the structure emitter: for each class C the per-class
//! block ships `structure C` + `CoeOut` instances (D30 §§5–7) +
//! `decodeC : Lean.Json → Except String C` (§8.1) +
//! `encodeC : C → Lean.Json` (§8.2). This file owns the latter two.
//!
//! ## Inheritance
//!
//! `extends`-inherited fields are read off the same JSON object by
//! the parent's decoder, then C's decoder constructs the child from
//! the parent value + own fields:
//!
//! ```lean
//! def decodeChild (j : Json) : Except String Child := do
//!   let parent ← decodeParent j
//!   let ownField ← decodeRequiredPrim j "Child" "urn:proj:ownField" "ownField"
//!   return { toParent := parent, ownField }
//! ```
//!
//! For multi-supertype, each parent gets its own bind plus an
//! explicit `toParentN := parentN` assignment. Inheritance-aware
//! `is_a` rendering on encode walks the transitive field surface
//! (parents' fields then own fields, parent-declaration order); the
//! `is_a` value itself is C's own IRI regardless of ancestry.
//!
//! ## EigeniusUnion (D30 §8.3)
//!
//! Decode reads the inner resource's `is_a[0]`, dispatches to the
//! matching class's decoder, wraps the result in the union's
//! position-indexed constructor (`inl` for class 0, `inr inl` for
//! class 1, …). Encode pattern-matches on the union constructor and
//! delegates to the chosen class's encoder.
//!
//! ## v1 scope notes
//!
//! D30 §8.5's module-level `eigeniusDecoders` registry isn't emitted
//! here — it sits at the module assembly layer (next milestone)
//! since it indexes over every class in the closure, not just the
//! current one. This file's helpers (`emit_decoder` / `emit_encoder`)
//! produce the per-class block, and the module assembler stitches
//! the registry at the file footer.

use super::structure_emitter::{render_lean_type, ClassNameLookup};
use super::{ClassDecl, LeanType, PropertyDecl};
use eigenius_kernel::ontology::iri::Iri;
use std::collections::BTreeMap;

/// Emit the per-class codec block: blank line, `decodeC`, blank
/// line, `encodeC`, trailing newline. Caller (the module
/// assembler) places this *after* the structure + coercion block.
pub(crate) fn emit_codec_block(
    decl: &ClassDecl,
    decls: &BTreeMap<Iri, ClassDecl>,
    lookup: &ClassNameLookup,
) -> String {
    let mut out = String::new();
    out.push('\n');
    push_decoder(&mut out, decl, decls, lookup);
    out.push('\n');
    push_encoder(&mut out, decl, decls, lookup);
    out
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

fn push_decoder(
    out: &mut String,
    decl: &ClassDecl,
    decls: &BTreeMap<Iri, ClassDecl>,
    lookup: &ClassNameLookup,
) {
    let cn = decl.short_name.as_str();
    out.push_str(&format!(
        "def decode{cn} (j : Lean.Json) : Except String {cn} := do\n"
    ));

    // Inheritance: bind each parent's decoded value first. Their
    // decoders also read `@id` (when the parent is a root), so we
    // don't redundantly read it here on non-root classes.
    for parent_iri in &decl.parents {
        let parent_name = lookup_or_panic(lookup, parent_iri);
        let bind = parent_bind_name(&parent_name);
        out.push_str(&format!("  let {bind} ← decode{parent_name} j\n"));
    }

    // Root class only: read @id into _id (D30 §7.2 / §8.1). For
    // non-root classes _id lives in the inherited parent struct.
    if decl.parents.is_empty() {
        out.push_str("  let _id ← match j.getObjValAs? String \"@id\" with\n");
        out.push_str("    | .ok v => pure (some v)\n");
        out.push_str("    | .error _ => pure none\n");
    }

    // Own required + recommended fields.
    for prop in &decl.requires {
        push_required_field_decode(out, cn, prop, lookup);
    }
    for prop in &decl.recommends {
        push_optional_field_decode(out, cn, prop, lookup);
    }

    // Construct the result. For inheritance, use explicit
    // `toParentN := <bind>` assignments; for root classes, list
    // `_id` plus the own field names.
    out.push_str("  return {");
    let mut needs_comma = false;
    for parent_iri in &decl.parents {
        let parent_name = lookup_or_panic(lookup, parent_iri);
        let bind = parent_bind_name(&parent_name);
        if needs_comma {
            out.push(',');
        }
        out.push_str(&format!(" to{parent_name} := {bind}"));
        needs_comma = true;
    }
    if decl.parents.is_empty() {
        if needs_comma {
            out.push(',');
        }
        out.push_str(" _id");
        needs_comma = true;
    }
    for prop in &decl.requires {
        if needs_comma {
            out.push(',');
        }
        out.push_str(&format!(" {}", prop.short_name));
        needs_comma = true;
    }
    for prop in &decl.recommends {
        if needs_comma {
            out.push(',');
        }
        out.push_str(&format!(" {}", prop.short_name));
        needs_comma = true;
    }
    out.push_str(" }\n");

    // Suppress an unused-binding lint when the only `do`-body
    // statement is the parent decode (no own/parent reads needed).
    // Lean's elaborator stays quiet without an explicit guard, but
    // the noop check is reserved for a future tightening pass.
    let _ = decls;
}

fn push_required_field_decode(
    out: &mut String,
    class_name: &str,
    prop: &PropertyDecl,
    lookup: &ClassNameLookup,
) {
    let fname = &prop.short_name;
    let iri = prop.property_iri.as_str();
    match &prop.lean_type {
        LeanType::String | LeanType::Int | LeanType::Float | LeanType::Bool | LeanType::Json => {
            let ty = render_lean_type(&prop.lean_type, lookup);
            out.push_str(&format!(
                "  let {fname} ← decodeRequiredPrim (α := {ty}) j \"{class_name}\" \"{iri}\" \"{fname}\"\n"
            ));
        }
        LeanType::ClassRef(class_iri) => {
            let cls = lookup_or_panic(lookup, class_iri);
            out.push_str(&format!(
                "  let {fname} ← decodeRequiredResource j \"{class_name}\" \"{iri}\" \"{fname}\" decode{cls}\n"
            ));
        }
        LeanType::ListPrimitive(inner) => {
            let ty = render_lean_type(inner, lookup);
            out.push_str(&format!(
                "  let {fname} ← decodeRequiredPrimList (α := {ty}) j \"{class_name}\" \"{iri}\" \"{fname}\"\n"
            ));
        }
        LeanType::ListClassRef(class_iri) => {
            let cls = lookup_or_panic(lookup, class_iri);
            out.push_str(&format!(
                "  let {fname} ← decodeRequiredResourceList j \"{class_name}\" \"{iri}\" \"{fname}\" decode{cls}\n"
            ));
        }
        LeanType::Union(iris) => {
            // Union-typed required field — read the inner Json,
            // dispatch on is_a[0], wrap in the matching ctor.
            out.push_str(&format!(
                "  let {fname} ← decodeRequiredResource j \"{class_name}\" \"{iri}\" \"{fname}\" (fun jv => {})\n",
                inline_union_decoder(class_name, &prop.short_name, iris, lookup)
            ));
        }
        LeanType::ListUnion(iris) => {
            // List of unions — for each inner Json, dispatch and wrap.
            out.push_str(&format!(
                "  let {fname} ← decodeRequiredResourceList j \"{class_name}\" \"{iri}\" \"{fname}\" (fun jv => {})\n",
                inline_union_decoder(class_name, &prop.short_name, iris, lookup)
            ));
        }
    }
}

fn push_optional_field_decode(
    out: &mut String,
    class_name: &str,
    prop: &PropertyDecl,
    lookup: &ClassNameLookup,
) {
    let fname = &prop.short_name;
    let iri = prop.property_iri.as_str();
    match &prop.lean_type {
        LeanType::String | LeanType::Int | LeanType::Float | LeanType::Bool | LeanType::Json => {
            let ty = render_lean_type(&prop.lean_type, lookup);
            out.push_str(&format!(
                "  let {fname} ← decodeOptionalPrim (α := {ty}) j \"{iri}\"\n"
            ));
        }
        LeanType::ClassRef(class_iri) => {
            let cls = lookup_or_panic(lookup, class_iri);
            out.push_str(&format!(
                "  let {fname} ← decodeOptionalResource j \"{iri}\" decode{cls}\n"
            ));
        }
        LeanType::ListPrimitive(_) | LeanType::ListClassRef(_) | LeanType::ListUnion(_) => {
            // Optional lists: absent → none; present → decode as
            // the required form, wrap in `some`. Re-use the
            // required helpers via a one-shot try/catch.
            out.push_str(&format!(
                "  let {fname} ← match (do {}) with\n",
                // The body is a pure expression form of the
                // required-list decoder so we can wrap it in
                // `match … | .ok v => some v | .error _ => none`.
                required_field_expr(class_name, prop, lookup)
            ));
            out.push_str("    | .ok v => pure (some v)\n");
            out.push_str("    | .error _ => pure none\n");
        }
        LeanType::Union(iris) => {
            out.push_str(&format!(
                "  let {fname} ← decodeOptionalResource j \"{iri}\" (fun jv => {})\n",
                inline_union_decoder(class_name, &prop.short_name, iris, lookup)
            ));
        }
    }
}

/// Body of a required-field decode used as a `do`-block expression
/// (no `let` binding). Mirrors `push_required_field_decode` but
/// produces an expression so `push_optional_field_decode` can
/// wrap it in `match … with | .ok v => some v | .error _ => none`.
fn required_field_expr(class_name: &str, prop: &PropertyDecl, lookup: &ClassNameLookup) -> String {
    let fname = &prop.short_name;
    let iri = prop.property_iri.as_str();
    match &prop.lean_type {
        LeanType::ListPrimitive(inner) => {
            let ty = render_lean_type(inner, lookup);
            format!("decodeRequiredPrimList (α := {ty}) j \"{class_name}\" \"{iri}\" \"{fname}\"")
        }
        LeanType::ListClassRef(class_iri) => {
            let cls = lookup_or_panic(lookup, class_iri);
            format!(
                "decodeRequiredResourceList j \"{class_name}\" \"{iri}\" \"{fname}\" decode{cls}"
            )
        }
        LeanType::ListUnion(iris) => {
            format!(
                "decodeRequiredResourceList j \"{class_name}\" \"{iri}\" \"{fname}\" (fun jv => {})",
                inline_union_decoder(class_name, fname, iris, lookup)
            )
        }
        _ => unreachable!("required_field_expr only handles list-typed properties"),
    }
}

/// Inline expression that decodes a single `EigeniusUnion`-typed
/// resource value. Dispatches on `is_a[0]` and wraps the inner
/// decoded value in the position-indexed ctor (`.inl` for class 0,
/// `.inr (.inl _)` for class 1, …). Returns a Lean *expression* —
/// usable on the RHS of `let ← ...` or inside a `fun jv => ...`.
fn inline_union_decoder(
    class_name: &str,
    field_name: &str,
    iris: &[Iri],
    lookup: &ClassNameLookup,
) -> String {
    let context = format!("{class_name}.{field_name}");
    let mut s = String::new();
    s.push_str(&format!("do\n    let disc ← isAHead jv \"{context}\"\n"));
    s.push_str("    match disc with\n");
    for (idx, iri) in iris.iter().enumerate() {
        let cls = lookup_or_panic(lookup, iri);
        let ctor = union_constructor_chain(idx);
        s.push_str(&format!(
            "    | \"{}\" => do let inner ← decode{cls} jv; pure ({ctor} inner)\n",
            iri.as_str()
        ));
    }
    s.push_str(&format!(
        "    | other => Except.error s!\"{context}: unknown discriminator: {{other}}\""
    ));
    s
}

/// Build the `EigeniusUnion` constructor chain for position `idx`
/// (0-indexed). Position 0 → `.inl`, position 1 → `.inr (.inl …)`,
/// position 2 → `.inr (.inr (.inl …))`, … The trailing `…` is the
/// value the caller wraps.
fn union_constructor_chain(idx: usize) -> String {
    // The expression has the shape `EigeniusUnion.inr (... (EigeniusUnion.inl x))`
    // — we emit it as an applied function with one open argument,
    // closed by the caller's value substitution. Simplest form: a
    // chain of `EigeniusUnion.inr ∘` and a trailing `EigeniusUnion.inl`.
    if idx == 0 {
        "EigeniusUnion.inl".to_string()
    } else {
        // For idx N, we want: `EigeniusUnion.inr (EigeniusUnion.inr ... (EigeniusUnion.inl X))`
        // Build by wrapping `EigeniusUnion.inl x` in N layers of
        // `EigeniusUnion.inr (…)`. The result needs to be invoked
        // as `<chain> inner` — Lean reads it as function application,
        // so we use `(fun x => …) inner` to keep precedence clear.
        let mut chain = "EigeniusUnion.inl x".to_string();
        for _ in 0..idx {
            chain = format!("EigeniusUnion.inr ({chain})");
        }
        format!("(fun x => {chain})")
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

fn push_encoder(
    out: &mut String,
    decl: &ClassDecl,
    decls: &BTreeMap<Iri, ClassDecl>,
    lookup: &ClassNameLookup,
) {
    let cn = decl.short_name.as_str();
    let class_iri = decl.class_iri.as_str();
    out.push_str(&format!("def encode{cn} (c : {cn}) : Lean.Json :=\n"));
    out.push_str("  Lean.Json.mkObj <|\n");

    // `@id` first (D30 §8.2 encode order). Inherited from a parent
    // when there are any; otherwise it lives on this class.
    out.push_str("    (match c._id with | some v => [(\"@id\", Lean.Json.str v)] | none => [])\n");

    // `is_a` — always this class's IRI.
    out.push_str(&format!(
        "    ++ [(\"urn:eigenius:core:is_a\", Lean.Json.arr #[Lean.Json.str \"{class_iri}\"])]\n"
    ));

    // Walk the transitive field surface — parents' required+recommended
    // first (parent-declaration order), then own.
    let all_fields = transitive_fields(decls, &decl.class_iri);
    for (prop, optional) in all_fields {
        push_field_encode(out, prop, optional, lookup);
    }
}

fn push_field_encode(
    out: &mut String,
    prop: &PropertyDecl,
    optional: bool,
    lookup: &ClassNameLookup,
) {
    let fname = &prop.short_name;
    let iri = prop.property_iri.as_str();
    let value_expr = encode_value_expr(&format!("c.{fname}"), &prop.lean_type, lookup);

    if optional {
        out.push_str(&format!(
            "    ++ (match c.{fname} with | some v => [(\"{iri}\", {})] | none => [])\n",
            encode_value_expr("v", &prop.lean_type, lookup)
        ));
    } else {
        out.push_str(&format!("    ++ [(\"{iri}\", {value_expr})]\n"));
    }
}

/// Render an expression that evaluates to `Lean.Json` for `var` of
/// the given `LeanType`. `var` is the Lean-side variable name to
/// project from (`c.fieldName`, `v` inside a `some v` match arm,
/// etc.). Always wraps in parens-as-needed so the caller can
/// embed it in a list-literal slot without precedence surprises.
fn encode_value_expr(var: &str, ty: &LeanType, lookup: &ClassNameLookup) -> String {
    match ty {
        LeanType::String | LeanType::Int | LeanType::Float | LeanType::Bool | LeanType::Json => {
            // Lean's `Lean.toJson` dispatches on `ToJson α` and
            // handles every primitive. Safer than per-type ctors.
            format!("Lean.toJson {var}")
        }
        LeanType::ClassRef(class_iri) => {
            let cls = lookup_or_panic(lookup, class_iri);
            format!("encode{cls} {var}")
        }
        LeanType::ListClassRef(class_iri) => {
            let cls = lookup_or_panic(lookup, class_iri);
            // List → Array → Json.arr by mapping the encoder over
            // each element.
            format!("Lean.Json.arr (({var}.map encode{cls}).toArray)")
        }
        LeanType::ListPrimitive(inner) => {
            // List of primitives — round-trip through Lean.toJson
            // (which handles List α when ToJson α). Same shape as
            // the singleton primitive case, just on the whole list.
            let _ = inner;
            format!("Lean.toJson {var}")
        }
        LeanType::Union(iris) => inline_union_encoder(var, iris, lookup),
        LeanType::ListUnion(iris) => {
            // For a list of unions, map the per-element encoder
            // over the list, wrap in Json.arr.
            let elem = inline_union_encoder("u", iris, lookup);
            format!("Lean.Json.arr (({var}.map (fun u => {elem})).toArray)")
        }
    }
}

/// Pattern-match a `EigeniusUnion`-typed value into the matching
/// inner encoder. Produces a Lean `match … with | … | …` expression
/// suitable as the RHS of a key/value tuple.
fn inline_union_encoder(var: &str, iris: &[Iri], lookup: &ClassNameLookup) -> String {
    let mut s = String::new();
    s.push_str(&format!("match {var} with\n"));
    for (idx, iri) in iris.iter().enumerate() {
        let cls = lookup_or_panic(lookup, iri);
        s.push_str("        ");
        // Position N → match against `EigeniusUnion.inr (... (.inl x))`.
        let pattern = union_match_pattern(idx);
        s.push_str(&format!("| {pattern} => encode{cls} x\n"));
    }
    // Trim the trailing newline so the caller can place it in a
    // list-literal slot without an extra line.
    s.trim_end().to_string()
}

/// Build the `EigeniusUnion` pattern at position `idx`. Same
/// structure as `union_constructor_chain` but as a match pattern
/// with a binder `x` for the inner value.
fn union_match_pattern(idx: usize) -> String {
    let mut chain = "EigeniusUnion.inl x".to_string();
    for _ in 0..idx {
        chain = format!("EigeniusUnion.inr ({chain})");
    }
    chain
}

// ---------------------------------------------------------------------------
// Internals shared with the structure emitter
// ---------------------------------------------------------------------------

/// Walk the transitive field surface for a class — parents first
/// (parent-declaration order), then own. Each entry carries the
/// optional flag so the caller can render `requires` vs.
/// `recommends` differently.
///
/// Order matches D30 §5 / §8.2. Required fields come before
/// recommended within each level.
fn transitive_fields<'a>(
    decls: &'a BTreeMap<Iri, ClassDecl>,
    iri: &Iri,
) -> Vec<(&'a PropertyDecl, bool)> {
    let mut out = Vec::new();
    walk(decls, iri, &mut out);
    out
}

fn walk<'a>(
    decls: &'a BTreeMap<Iri, ClassDecl>,
    iri: &Iri,
    out: &mut Vec<(&'a PropertyDecl, bool)>,
) {
    let Some(decl) = decls.get(iri) else { return };
    for parent in &decl.parents {
        walk(decls, parent, out);
    }
    for prop in &decl.requires {
        out.push((prop, false));
    }
    for prop in &decl.recommends {
        out.push((prop, true));
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

/// Generate the parent-decode binding name. For a parent named
/// `Animal`, we bind the decoded value to `parent_Animal` so the
/// constructor can reference it via `toAnimal := parent_Animal`.
fn parent_bind_name(parent_name: &str) -> String {
    format!("parent_{parent_name}")
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

    fn decls_for(classes: Vec<ClassDecl>) -> BTreeMap<Iri, ClassDecl> {
        classes
            .into_iter()
            .map(|c| (c.class_iri.clone(), c))
            .collect()
    }

    fn lookup_for(decls: &BTreeMap<Iri, ClassDecl>) -> ClassNameLookup {
        decls
            .iter()
            .map(|(i, d)| (i.clone(), d.short_name.clone()))
            .collect()
    }

    // ─── Decoder shape ──────────────────────────────────────────────

    #[test]
    fn decoder_for_empty_root_class_reads_only_id() {
        let c = cls("Empty", vec![], vec![], vec![]);
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains("def decodeEmpty (j : Lean.Json) : Except String Empty := do"));
        assert!(out.contains("j.getObjValAs? String \"@id\""));
        assert!(out.contains("return { _id }"));
    }

    #[test]
    fn decoder_emits_required_primitive_decode_with_class_field_diagnostic() {
        let c = cls(
            "Person",
            vec![],
            vec![prop("name", LeanType::String)],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains(
            "let name ← decodeRequiredPrim (α := String) j \"Person\" \"urn:test:name\" \"name\""
        ));
        assert!(out.contains("return { _id, name }"));
    }

    #[test]
    fn decoder_emits_optional_field_using_optional_helper() {
        let c = cls(
            "Person",
            vec![],
            vec![],
            vec![prop("nickname", LeanType::String)],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(
            out.contains("let nickname ← decodeOptionalPrim (α := String) j \"urn:test:nickname\"")
        );
    }

    #[test]
    fn decoder_emits_required_classref_using_inner_decoder() {
        let person = cls("Person", vec![], vec![], vec![]);
        let doc = cls(
            "Doc",
            vec![],
            vec![prop("author", LeanType::ClassRef(iri("urn:test:Person")))],
            vec![],
        );
        let decls = decls_for(vec![person, doc.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&doc, &decls, &lookup);
        assert!(out.contains(
            "let author ← decodeRequiredResource j \"Doc\" \"urn:test:author\" \"author\" decodePerson"
        ));
    }

    #[test]
    fn decoder_emits_list_primitive_using_list_helper() {
        let c = cls(
            "Bag",
            vec![],
            vec![prop(
                "tags",
                LeanType::ListPrimitive(Box::new(LeanType::String)),
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains(
            "let tags ← decodeRequiredPrimList (α := String) j \"Bag\" \"urn:test:tags\" \"tags\""
        ));
    }

    #[test]
    fn decoder_emits_list_classref_using_resource_list_helper() {
        let person = cls("Person", vec![], vec![], vec![]);
        let team = cls(
            "Team",
            vec![],
            vec![prop(
                "members",
                LeanType::ListClassRef(iri("urn:test:Person")),
            )],
            vec![],
        );
        let decls = decls_for(vec![person, team.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&team, &decls, &lookup);
        assert!(out.contains(
            "let members ← decodeRequiredResourceList j \"Team\" \"urn:test:members\" \"members\" decodePerson"
        ));
    }

    #[test]
    fn decoder_emits_union_dispatch_on_is_a_head() {
        let apple = cls("Apple", vec![], vec![], vec![]);
        let zebra = cls("Zebra", vec![], vec![], vec![]);
        let doc = cls(
            "Doc",
            vec![],
            vec![prop(
                "contributor",
                LeanType::Union(vec![iri("urn:test:Apple"), iri("urn:test:Zebra")]),
            )],
            vec![],
        );
        let decls = decls_for(vec![apple, zebra, doc.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&doc, &decls, &lookup);
        assert!(out.contains("isAHead jv \"Doc.contributor\""));
        assert!(out.contains(
            "\"urn:test:Apple\" => do let inner ← decodeApple jv; pure (EigeniusUnion.inl inner)"
        ));
        assert!(out.contains("\"urn:test:Zebra\" => do let inner ← decodeZebra jv"));
        // Second position: wraps in `inr (inl x)`.
        assert!(out.contains("EigeniusUnion.inr (EigeniusUnion.inl x)"));
        assert!(out.contains("unknown discriminator"));
    }

    #[test]
    fn decoder_for_subclass_delegates_to_parent_decoder() {
        let parent = cls(
            "Animal",
            vec![],
            vec![prop("species", LeanType::String)],
            vec![],
        );
        let child = cls(
            "Dog",
            vec![iri("urn:test:Animal")],
            vec![prop("breed", LeanType::String)],
            vec![],
        );
        let decls = decls_for(vec![parent, child.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&child, &decls, &lookup);
        // Parent decode bind.
        assert!(out.contains("let parent_Animal ← decodeAnimal j"));
        // No own @id read (inherited from parent).
        assert!(!out.contains("Dog := do\n  let _id"));
        // Construction uses explicit projection assignment for parent.
        assert!(out.contains("return { toAnimal := parent_Animal, breed }"));
    }

    // ─── Encoder shape ──────────────────────────────────────────────

    #[test]
    fn encoder_for_empty_root_class_emits_id_and_is_a() {
        let c = cls("Empty", vec![], vec![], vec![]);
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains("def encodeEmpty (c : Empty) : Lean.Json :="));
        assert!(out.contains("Lean.Json.mkObj"));
        assert!(
            out.contains("match c._id with | some v => [(\"@id\", Lean.Json.str v)] | none => []")
        );
        assert!(out.contains("Lean.Json.arr #[Lean.Json.str \"urn:test:Empty\"]"));
    }

    #[test]
    fn encoder_required_primitive_uses_lean_tojson() {
        let c = cls(
            "Person",
            vec![],
            vec![prop("name", LeanType::String)],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains("++ [(\"urn:test:name\", Lean.toJson c.name)]"));
    }

    #[test]
    fn encoder_optional_field_emits_only_when_some() {
        let c = cls(
            "Person",
            vec![],
            vec![],
            vec![prop("nickname", LeanType::String)],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains(
            "++ (match c.nickname with | some v => [(\"urn:test:nickname\", Lean.toJson v)] | none => [])"
        ));
    }

    #[test]
    fn encoder_classref_field_delegates_to_inner_encoder() {
        let person = cls("Person", vec![], vec![], vec![]);
        let doc = cls(
            "Doc",
            vec![],
            vec![prop("author", LeanType::ClassRef(iri("urn:test:Person")))],
            vec![],
        );
        let decls = decls_for(vec![person, doc.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&doc, &decls, &lookup);
        assert!(out.contains("++ [(\"urn:test:author\", encodePerson c.author)]"));
    }

    #[test]
    fn encoder_walks_transitive_fields_with_inherited_first() {
        // Parent has `species`, child has `breed`. The encoder must
        // emit `species` (inherited) before `breed` (own) — D30 §5.
        // We scope the position-search to the encoder body so the
        // decoder's `urn:test:breed` reference (used to read the own
        // field) doesn't shadow the encoder-order assertion.
        let parent = cls(
            "Animal",
            vec![],
            vec![prop("species", LeanType::String)],
            vec![],
        );
        let child = cls(
            "Dog",
            vec![iri("urn:test:Animal")],
            vec![prop("breed", LeanType::String)],
            vec![],
        );
        let decls = decls_for(vec![parent, child.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&child, &decls, &lookup);
        let encoder_start = out
            .find("def encodeDog")
            .expect("encoder definition present");
        let encoder_body = &out[encoder_start..];
        let species_pos = encoder_body
            .find("\"urn:test:species\"")
            .expect("inherited species encoded");
        let breed_pos = encoder_body
            .find("\"urn:test:breed\"")
            .expect("own breed encoded");
        assert!(
            species_pos < breed_pos,
            "D30 §5: inherited fields encode before own fields"
        );
        // is_a still references the child class.
        assert!(out.contains("\"urn:test:Dog\""));
    }

    #[test]
    fn encoder_union_field_dispatches_via_match() {
        let apple = cls("Apple", vec![], vec![], vec![]);
        let zebra = cls("Zebra", vec![], vec![], vec![]);
        let doc = cls(
            "Doc",
            vec![],
            vec![prop(
                "contributor",
                LeanType::Union(vec![iri("urn:test:Apple"), iri("urn:test:Zebra")]),
            )],
            vec![],
        );
        let decls = decls_for(vec![apple, zebra, doc.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&doc, &decls, &lookup);
        assert!(out.contains("match c.contributor with"));
        assert!(out.contains("| EigeniusUnion.inl x => encodeApple x"));
        assert!(out.contains("| EigeniusUnion.inr (EigeniusUnion.inl x) => encodeZebra x"));
    }

    #[test]
    fn encoder_list_classref_maps_inner_encoder() {
        let person = cls("Person", vec![], vec![], vec![]);
        let team = cls(
            "Team",
            vec![],
            vec![prop(
                "members",
                LeanType::ListClassRef(iri("urn:test:Person")),
            )],
            vec![],
        );
        let decls = decls_for(vec![person, team.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&team, &decls, &lookup);
        assert!(out.contains(
            "++ [(\"urn:test:members\", Lean.Json.arr ((c.members.map encodePerson).toArray))]"
        ));
    }
}
