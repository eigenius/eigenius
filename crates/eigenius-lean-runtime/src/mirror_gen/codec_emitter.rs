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

use super::structure_emitter::{refinement_predicate, render_lean_type, ClassNameLookup};
use super::{ClassDecl, LeanType, PropertyConstraints, PropertyDecl};
use eigenius_kernel::ontology::iri::Iri;
use std::collections::BTreeMap;

const CORE_FORMATS_PREFIX: &str = "urn:eigenius:core:formats:";

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
    // D30 §9 constraint chain.
    //
    // - §9.1: min/max value/length constraints lift into a
    //   refinement-typed subtype (`{ x : T // pred }`). The
    //   decoder constructs the subtype value via `withRefinement`
    //   in a single bind that shadows the raw value.
    // - §9.2: pattern + format constraints stay runtime-only.
    //   On refined fields they read `field.val`; on unrefined
    //   string fields they shadow the binding (validator returns
    //   the value unchanged on success).
    if let Some(pred) = refinement_predicate(prop) {
        let err_msg = refinement_error_message(class_name, prop);
        out.push_str(&format!(
            "  let {fname} ← withRefinement (fun x => {pred}) {fname} \"{err_msg}\"\n"
        ));
        for call in runtime_check_calls(class_name, prop, &format!("{fname}.val")) {
            // Side-effect: discard the validator's String return;
            // the refined `fname` already has the correct type.
            out.push_str(&format!("  let _ ← {call}\n"));
        }
    } else {
        for call in runtime_check_calls(class_name, prop, fname) {
            out.push_str(&format!("  let {fname} ← {call}\n"));
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
            if let Some(pred) = refinement_predicate(prop) {
                let err_msg = refinement_error_message(class_name, prop);
                out.push_str(&format!(
                    "  let {fname} ← withOptionalRefinement (fun x => {pred}) {fname} \"{err_msg}\"\n"
                ));
                // Runtime checks on the inner `.val`, threaded
                // through `validateOptional` so `none` short-circuits.
                // Wrap each call in `(call).map (fun _ => v)` to
                // preserve the refined inner type.
                for call in runtime_check_calls(class_name, prop, "v.val") {
                    out.push_str(&format!(
                        "  let {fname} ← validateOptional {fname} (fun v => ({call}).map (fun _ => v))\n"
                    ));
                }
            } else {
                // Unrefined: validators that return the value
                // chain naturally through `validateOptional`.
                for call in runtime_check_calls(class_name, prop, "v") {
                    out.push_str(&format!(
                        "  let {fname} ← validateOptional {fname} (fun v => {call})\n"
                    ));
                }
            }
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
    // D30 §9.1 — refinement-typed fields carry their underlying
    // value at `.val`. The encoder unwraps before handing to
    // `Lean.toJson` (or the per-class encoder); the structural
    // refinement on a `Subtype` doesn't have a `ToJson` instance,
    // and even if it did the JSON form would echo the bare value.
    let refined = refinement_predicate(prop).is_some();

    if optional {
        let inner_var = if refined { "v.val" } else { "v" };
        out.push_str(&format!(
            "    ++ (match c.{fname} with | some v => [(\"{iri}\", {})] | none => [])\n",
            encode_value_expr(inner_var, &prop.lean_type, lookup)
        ));
    } else {
        let var = if refined {
            format!("c.{fname}.val")
        } else {
            format!("c.{fname}")
        };
        let value_expr = encode_value_expr(&var, &prop.lean_type, lookup);
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

// ---------------------------------------------------------------------------
// D30 §9 — constraint validator chain rendering
// ---------------------------------------------------------------------------

/// Render the chain of *runtime* validator calls (D30 §9.2 —
/// pattern + format) a constrained field needs after its raw decode
/// (and refinement-typed lift, when applicable). `value_var` is the
/// Lean variable name the validator reads from — the field's own
/// short_name for unrefined required fields, `"v"` for optional
/// fields inside a `validateOptional … fun v => …` lambda, or
/// `"<fname>.val"` / `"v.val"` for the refined cases.
///
/// Numeric and length constraints (D30 §9.1) do **not** appear here —
/// they're lifted into the refinement subtype via
/// `refinement_predicate` + `withRefinement`. Order within this
/// chain: pattern, then format.
///
/// Returns an empty vec when the property has no runtime checks.
fn runtime_check_calls(class_name: &str, prop: &PropertyDecl, value_var: &str) -> Vec<String> {
    let mut calls = Vec::new();
    let constraints = &prop.constraints;
    let field_ctx = format!("\"{class_name}.{}\"", prop.short_name);

    if matches!(prop.lean_type, LeanType::String) {
        if let Some(pattern) = &constraints.pattern {
            calls.push(format!(
                "validatePattern {field_ctx} {value_var} \"{}\"",
                lean_string_escape(pattern)
            ));
        }
        if let Some(format_iri) = &constraints.format {
            calls.push(format!(
                "validateFormat {field_ctx} {value_var} {}",
                lean_format_symbol(format_iri)
            ));
        }
    }

    calls
}

/// Build the error message string for a `withRefinement` call.
/// Describes the failing constraint surface for the decoded value
/// so production diagnostics point at the chain-side cause without
/// re-reading the spec. Format: `<ClassName>.<fieldName>: out of
/// range [lo, hi] / wrong length / …` — matches the D30 §8.1
/// per-field error shape.
fn refinement_error_message(class_name: &str, prop: &PropertyDecl) -> String {
    let mut parts: Vec<String> = Vec::new();
    let c = &prop.constraints;
    let is_numeric = matches!(prop.lean_type, LeanType::Float | LeanType::Int);
    if is_numeric && (c.min_value.is_some() || c.max_value.is_some()) {
        let lo = c
            .min_value
            .map(|v| {
                if matches!(prop.lean_type, LeanType::Int) {
                    format!("{}", v as i64)
                } else {
                    lean_float_literal(v)
                }
            })
            .unwrap_or_else(|| "-∞".to_string());
        let hi = c
            .max_value
            .map(|v| {
                if matches!(prop.lean_type, LeanType::Int) {
                    format!("{}", v as i64)
                } else {
                    lean_float_literal(v)
                }
            })
            .unwrap_or_else(|| "∞".to_string());
        parts.push(format!("out of range [{lo}, {hi}]"));
    }
    let is_string = matches!(prop.lean_type, LeanType::String);
    if is_string && (c.min_length.is_some() || c.max_length.is_some()) {
        let lo = c
            .min_length
            .map(|n| n.to_string())
            .unwrap_or_else(|| "0".to_string());
        let hi = c
            .max_length
            .map(|n| n.to_string())
            .unwrap_or_else(|| "∞".to_string());
        parts.push(format!("length out of range [{lo}, {hi}]"));
    }
    let body = if parts.is_empty() {
        "refinement failed".to_string()
    } else {
        parts.join("; ")
    };
    format!("{class_name}.{}: {body}", prop.short_name)
}

/// Render an `f64` as a Lean `Float` literal (D30 §9.3 — `0` → `0.0`,
/// `100.0` stays `100.0`, `0.5` stays `0.5`). Rust's `{:?}` on
/// `f64` produces a decimal representation with at least one
/// fractional digit, which matches Lean's `Float` literal syntax.
fn lean_float_literal(v: f64) -> String {
    format!("{v:?}")
}

/// Escape a chain-side pattern string for Lean's double-quoted
/// string literal syntax. Backslashes are doubled, double quotes
/// are escaped, control characters use Lean's escape sequences.
///
/// D30 §9.4 also pinned `\$` ("because Lean's string macros
/// recognise `$` for antiquotation"); we deliberately diverge from
/// that point because Lean 4's *plain* string-literal lexer (which
/// is what the emitter targets) rejects `\$` as an invalid escape.
/// `$` only has antiquotation semantics inside the `s!"..."`
/// interpolation form, which the emitter doesn't use. Spec
/// erratum filed for D30 v1.x.
fn lean_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Render a `core:format` IRI as a Lean `Name`-typed argument to
/// `validateFormat` (D30 §9.5).
/// `urn:eigenius:core:formats:<name>` → `` `<name> `` (a Name
/// literal); any other IRI → `Name.mkSimple "<full IRI>"`. The
/// validator's parameter type is `Name`, so both forms unify.
fn lean_format_symbol(format_iri: &str) -> String {
    if let Some(stripped) = format_iri.strip_prefix(CORE_FORMATS_PREFIX) {
        // Bare-name literal — Lean's `` ` `` syntax produces a `Name`
        // whose ToString prints just `<name>`.
        format!("`{stripped}")
    } else {
        format!("(Name.mkSimple \"{format_iri}\")")
    }
}

#[allow(dead_code)]
fn _drop_unused_property_constraints_warning(_pc: &PropertyConstraints) {}

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

    // ─── Constraint validator chain (D30 §9) ───────────────────────

    fn prop_with_constraints(short: &str, ty: LeanType, c: PropertyConstraints) -> PropertyDecl {
        let mut p = prop(short, ty);
        p.constraints = c;
        p
    }

    #[test]
    fn constraint_emits_no_calls_for_unconstrained_property() {
        let c = cls(
            "Person",
            vec![],
            vec![prop("name", LeanType::String)],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        // No validator calls land in the decoder body — only the
        // raw decode.
        assert!(!out.contains("validateMinValue"));
        assert!(!out.contains("validateMaxValue"));
        assert!(!out.contains("validateMinLength"));
        assert!(!out.contains("validateMaxLength"));
        assert!(!out.contains("validatePattern"));
        assert!(!out.contains("validateFormat"));
    }

    #[test]
    fn constraint_min_value_on_float_lifts_to_refinement_predicate() {
        // D30 §9.1: min_value on Float becomes a refinement-typed
        // subtype. The decoder swaps the validateMinValueFloat
        // chain for a single `withRefinement` call.
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "weight",
                LeanType::Float,
                PropertyConstraints {
                    min_value: Some(0.0),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        // The validator-chain form must NOT appear — it's been
        // replaced by the refinement-typed subtype.
        assert!(
            !out.contains("validateMinValueFloat"),
            "min_value should land as a refinement predicate, not a runtime validator"
        );
        // The withRefinement call carries the predicate.
        assert!(out.contains(
            "let weight ← withRefinement (fun x => 0.0 ≤ x) weight \"Sample.weight: out of range [0.0, ∞]\""
        ));
    }

    #[test]
    fn constraint_min_max_value_on_int_lifts_to_int_refinement() {
        // Int range constraints lift to `{ x : Int // lo ≤ x ∧ x ≤ hi }`.
        // D30 §9.3 — no Float-cast through Int; the predicate uses
        // Int comparisons natively.
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "count",
                LeanType::Int,
                PropertyConstraints {
                    min_value: Some(1.0),
                    max_value: Some(100.0),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(
            !out.contains("validateMinValueInt") && !out.contains("validateMaxValueInt"),
            "Int range constraints should be refinement predicates"
        );
        assert!(out.contains(
            "let count ← withRefinement (fun x => 1 ≤ x ∧ x ≤ 100) count \"Sample.count: out of range [1, 100]\""
        ));
    }

    #[test]
    fn constraint_string_length_lifts_to_refinement_pattern_format_stay_runtime() {
        // D30 §9.1: min/max_length lift to refinement predicates
        // on `x.length`. §9.2: pattern + format stay runtime, run
        // after the refinement on `field.val`.
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "name",
                LeanType::String,
                PropertyConstraints {
                    min_length: Some(1),
                    max_length: Some(100),
                    pattern: Some("[A-Z][a-z]+".to_string()),
                    format: Some("urn:eigenius:core:formats:iri".to_string()),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        // Length constraints lifted into the refinement.
        assert!(
            !out.contains("validateMinLength") && !out.contains("validateMaxLength"),
            "length constraints should be refinement predicates"
        );
        assert!(out.contains(
            "let name ← withRefinement (fun x => 1 ≤ x.length ∧ x.length ≤ 100) name \"Sample.name: length out of range [1, 100]\""
        ));
        // Pattern + format still runtime — on .val of the refined value.
        assert!(out.contains("let _ ← validatePattern \"Sample.name\" name.val "));
        assert!(out.contains("let _ ← validateFormat \"Sample.name\" name.val `iri"));
        let pat = out.find("validatePattern").expect("pattern");
        let fmt = out.find("validateFormat").expect("format");
        let refine = out.find("withRefinement").expect("refinement comes first");
        assert!(
            refine < pat && pat < fmt,
            "ordering: refinement → pattern → format"
        );
    }

    #[test]
    fn constraint_pattern_only_without_refinement_stays_runtime_validator() {
        // Pattern with no length/range constraint → no refinement
        // lift; the runtime validator stays as a shadowing bind.
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "tag",
                LeanType::String,
                PropertyConstraints {
                    pattern: Some("[a-z]+".to_string()),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(!out.contains("withRefinement"), "no refinement applies");
        // Pattern shadows the binding rather than reading `.val`.
        assert!(out.contains("let tag ← validatePattern \"Sample.tag\" tag "));
    }

    #[test]
    fn constraint_refinement_renders_field_type_as_subtype() {
        // The structure declaration must wrap the field type in
        // `{ x : T // pred }` whenever a refinement applies.
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "weight",
                LeanType::Float,
                PropertyConstraints {
                    min_value: Some(0.0),
                    max_value: Some(100.0),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = crate::mirror_gen::emit_class_block(&c, &decls, &lookup);
        // structure field: refinement subtype.
        assert!(out.contains("  weight : { x : Float // 0.0 ≤ x ∧ x ≤ 100.0 }\n"));
        // encoder: project .val.
        assert!(out.contains(", Lean.toJson c.weight.val"));
    }

    #[test]
    fn constraint_pattern_string_is_lean_escaped() {
        // Original pattern: `^foo\\".*$\n`
        //   - Rust source `\\\\` → one runtime `\\` (two chars: backslash, backslash)
        //     wait, that's `\\` (two chars in source = two backslashes in runtime).
        //     Actually Rust `"\\\\"` = 2 backslashes at runtime.
        //   - Then `"` (one quote)
        //   - Then `.*$\n` (literal chars + newline)
        // After Lean escape:
        //   - `\\` runtime → `\\\\` in source string (each `\` doubled to `\\`)
        //   - `"` → `\"`
        //   - `$` → `$` (NOT escaped — see comment in `lean_string_escape`)
        //   - `\n` (LF) → `\n` (escape sequence)
        let pattern = "^foo\\\\\".*$\n";
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "name",
                LeanType::String,
                PropertyConstraints {
                    pattern: Some(pattern.to_string()),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        // The expected Lean source literal is `"^foo\\\\\".*$\n"` —
        // four backslash-chars-in-source for two runtime backslashes,
        // `\"` for the quote, plain `$`, `\n` for LF.
        let expected_in_source = "\"^foo\\\\\\\\\\\".*$\\n\"";
        assert!(
            out.contains(expected_in_source),
            "expected escape sequence not found.\nexpected literal: {expected_in_source}\ngot:\n{out}"
        );
    }

    #[test]
    fn constraint_format_with_core_prefix_renders_bare_name_literal() {
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "url",
                LeanType::String,
                PropertyConstraints {
                    format: Some("urn:eigenius:core:formats:iri".to_string()),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        // D30 §9.5: known core format → backtick Name literal.
        assert!(out.contains("validateFormat \"Sample.url\" url `iri"));
    }

    #[test]
    fn constraint_format_outside_core_prefix_renders_name_mksimple() {
        let c = cls(
            "Sample",
            vec![],
            vec![prop_with_constraints(
                "ext",
                LeanType::String,
                PropertyConstraints {
                    format: Some("urn:project:custom:phone".to_string()),
                    ..Default::default()
                },
            )],
            vec![],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        // D30 §9.5: other IRI → `Name.mkSimple "<full IRI>"`.
        assert!(out.contains(
            "validateFormat \"Sample.ext\" ext (Name.mkSimple \"urn:project:custom:phone\")"
        ));
    }

    #[test]
    fn constraint_on_optional_refined_field_uses_with_optional_refinement() {
        // Recommended (Option-typed) field with a refinable
        // constraint — `withOptionalRefinement` short-circuits on
        // `none` and constructs the refined subtype on `some`.
        let c = cls(
            "Sample",
            vec![],
            vec![],
            vec![prop_with_constraints(
                "weight",
                LeanType::Float,
                PropertyConstraints {
                    min_value: Some(0.0),
                    ..Default::default()
                },
            )],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains(
            "let weight ← withOptionalRefinement (fun x => 0.0 ≤ x) weight \"Sample.weight: out of range [0.0, ∞]\""
        ));
    }

    #[test]
    fn constraint_on_optional_unrefined_field_uses_validate_optional() {
        // Pattern with no refinement — falls back to the original
        // `validateOptional fname (fun v => validatePattern …)`
        // form so `none` short-circuits and the value type stays
        // bare `String`.
        let c = cls(
            "Sample",
            vec![],
            vec![],
            vec![prop_with_constraints(
                "tag",
                LeanType::String,
                PropertyConstraints {
                    pattern: Some("[a-z]+".to_string()),
                    ..Default::default()
                },
            )],
        );
        let decls = decls_for(vec![c.clone()]);
        let lookup = lookup_for(&decls);
        let out = emit_codec_block(&c, &decls, &lookup);
        assert!(out.contains(
            "let tag ← validateOptional tag (fun v => validatePattern \"Sample.tag\" v "
        ));
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
