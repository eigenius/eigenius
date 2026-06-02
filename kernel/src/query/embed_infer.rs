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

//! D43 §4.4 / M5.7 — EMBED model inference.
//!
//! The user can write either form of an [`EMBED`] call:
//!
//! ```text
//! EMBED("text", "urn:eigenius:embed:model:v1")   // explicit model
//! EMBED("text")                                  // inferred from context
//! ```
//!
//! The 1-arg form expects the typechecker to look at the
//! `VECTOR_NEAR` / `VECTOR_SIM` call that consumes the `EMBED`'s
//! result and copy the `model_iri` of that call's first argument's
//! active `VectorIndex` Resource into a synthesized second positional
//! argument. After this pass the rest of the pipeline only ever sees
//! the 2-arg form — the evaluator (`eval_embed`) has no inference
//! responsibility.
//!
//! ## Inference scope (v1)
//!
//! v1 supports the **direct** flow only: `EMBED(...)` appears as the
//! second positional argument of `VECTOR_NEAR` / `VECTOR_SIM`. The
//! design (§4.4) anticipates transitive flow through let-bindings
//! and Vector arithmetic; EigenQL has neither in v1, so the direct
//! case is the full surface today. The inference rules from §4.4
//! still apply in form:
//!
//! | Candidate set | Result |
//! |---|---|
//! | Empty | `embed_no_inferred_model` typecheck error |
//! | Single | Inferred; the 1-arg call is rewritten to 2-arg |
//! | Multiple, all agree | Inferred; rewritten |
//! | Multiple, disagree | `embed_ambiguous_model` typecheck error |
//!
//! v1 only triggers the "single" branch in practice — every EMBED is
//! consumed by at most one VECTOR_NEAR / VECTOR_SIM call. The empty
//! and disagree branches still fire for: a stray `EMBED("text")` in
//! `RETURN` position with no surrounding retrieval context (empty),
//! and (when transitive flow lands) one EMBED feeding two
//! disagreeing-model retrieval calls.
//!
//! ## Explicit-model disagreement
//!
//! Per §4.4: an explicit `model:` argument on an `EMBED` that
//! disagrees with the model the surrounding context would have
//! inferred fails at typecheck. v1 enforces this: if EMBED has 2
//! args and is consumed by a VECTOR_NEAR / VECTOR_SIM whose
//! property's active VectorIndex declares a different model, the
//! pass surfaces `embed_explicit_model_mismatch`.
//!
//! The pass mutates the program in place; callers run it between
//! [`crate::query::type_check::type_check`] and
//! [`crate::query::evaluate::evaluate`].

use crate::layer::{resolve_active_vector_indexes, ActiveVectorIndex, Layer};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::error::QueryError;
use std::collections::BTreeMap;

/// Mutating pass. Walks every expression in `program` and rewrites
/// 1-arg `EMBED("text")` calls into 2-arg `EMBED("text", "<model>")`
/// using the surrounding `VECTOR_NEAR` / `VECTOR_SIM` context's
/// active VectorIndex `model_iri`. Returns the typecheck errors
/// produced when inference fails (empty / ambiguous / explicit
/// disagrees).
pub fn infer_embed_models(program: &mut Program, layer: &Layer) -> Vec<QueryError> {
    let prop_var_index = build_property_variable_index(program, layer);
    let vector_indexes = resolve_active_vector_indexes(layer);
    let mut errors = Vec::new();

    // Visit every expression position. The `surrounding_model`
    // tracks the model IRI that a containing VECTOR_NEAR /
    // VECTOR_SIM imposes on its second argument — `None`
    // everywhere else.
    let ctx = InferCtx {
        prop_var_index: &prop_var_index,
        vector_indexes: &vector_indexes,
    };

    // Main query body.
    for cond in &mut program.query.body.conditions {
        infer_in_expr(cond, None, &ctx, &mut errors);
    }
    for item in &mut program.query.result {
        infer_in_expr(&mut item.expression, None, &ctx, &mut errors);
    }
    for expr in &mut program.query.group_by {
        infer_in_expr(expr, None, &ctx, &mut errors);
    }
    for item in &mut program.query.order_by {
        infer_in_expr(&mut item.expression, None, &ctx, &mut errors);
    }
    if let Some(top_k) = program.query.top_k_by.as_mut() {
        infer_in_expr(&mut top_k.expression, None, &ctx, &mut errors);
    }
    // Rule bodies' WHERE.
    for def in &mut program.definitions {
        for cond in &mut def.body.conditions {
            infer_in_expr(cond, None, &ctx, &mut errors);
        }
    }
    errors
}

// ---------------- internals ----------------

struct InferCtx<'a> {
    prop_var_index: &'a BTreeMap<String, Iri>,
    vector_indexes: &'a [ActiveVectorIndex],
}

impl<'a> InferCtx<'a> {
    /// Model IRI imposed by a VECTOR_NEAR / VECTOR_SIM whose first
    /// argument is `?var`. Returns `None` when the variable isn't
    /// property-bound or the property has no active VectorIndex —
    /// those cases are reported by the typechecker (M5.4) and don't
    /// need to be re-flagged here.
    fn model_for_property_var(&self, var_name: &str) -> Option<&Iri> {
        let property = self.prop_var_index.get(var_name)?;
        self.vector_indexes
            .iter()
            .find(|vi| vi.target_property == *property)
            .map(|vi| &vi.model)
    }
}

fn infer_in_expr(
    expr: &mut Expression,
    surrounding_model: Option<&Iri>,
    ctx: &InferCtx<'_>,
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::FunctionCall { name, args }
            if name == "VECTOR_NEAR" || name == "VECTOR_SIM" =>
        {
            // arg[0] is the property variable; resolve to a model
            // IRI that the second argument inherits as its
            // surrounding context. The other args inherit `None`
            // (the K literal in VECTOR_NEAR, etc.).
            let imposed = match args.first() {
                Some(Expression::Variable(v)) => ctx.model_for_property_var(&v.name).cloned(),
                _ => None,
            };
            for (i, arg) in args.iter_mut().enumerate() {
                let ctx_for_arg = if i == 1 { imposed.as_ref() } else { None };
                infer_in_expr(arg, ctx_for_arg, ctx, errors);
            }
        }
        Expression::FunctionCall { name, args } if name == "EMBED" => {
            // Apply inference to this call's own argument shape
            // before recursing into nested expressions (the text
            // argument can itself contain EMBED in principle, but
            // v1 strings are literals so the recursion is a no-op).
            match args.len() {
                1 => match surrounding_model {
                    Some(model) => {
                        args.push(Expression::Literal(Literal::String(
                            model.as_str().to_string(),
                        )));
                    }
                    None => {
                        errors.push(QueryError::type_check(
                            "embed_no_inferred_model",
                            "EMBED without explicit model_iri requires a surrounding \
                             VECTOR_NEAR / VECTOR_SIM context whose property has an \
                             active VectorIndex"
                                .to_string(),
                        ));
                    }
                },
                2 => {
                    // Explicit-model agreement check. Only enforced
                    // when the surrounding context offers an
                    // inferred model AND the explicit argument is a
                    // literal IRI string (the typechecker has
                    // already verified arg[1]'s shape elsewhere).
                    if let (Some(expected), Some(provided)) = (
                        surrounding_model,
                        match &args[1] {
                            Expression::Literal(Literal::String(s)) => Iri::parse(s).ok(),
                            _ => None,
                        },
                    ) {
                        if &provided != expected {
                            errors.push(QueryError::type_check(
                                "embed_explicit_model_mismatch",
                                format!(
                                    "EMBED was given explicit model `{}` but the \
                                     surrounding VECTOR_NEAR / VECTOR_SIM expects \
                                     model `{}` (active VectorIndex declares it)",
                                    provided.as_str(),
                                    expected.as_str()
                                ),
                            ));
                        }
                    }
                }
                _ => {
                    // Other arities are handled by the evaluator
                    // (and a typecheck rule we may want to add as
                    // a sibling to TEXT_MATCH's arity check). Don't
                    // duplicate-emit here.
                }
            }
            // Recurse into the text argument; the model arg, if
            // present, has no further structure to infer.
            if let Some(first) = args.first_mut() {
                infer_in_expr(first, None, ctx, errors);
            }
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                infer_in_expr(arg, None, ctx, errors);
            }
        }
        Expression::Binary { left, right, .. } => {
            infer_in_expr(left, None, ctx, errors);
            infer_in_expr(right, None, ctx, errors);
        }
        Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
            infer_in_expr(operand, None, ctx, errors);
        }
        Expression::Aggregate { arg, .. } => {
            infer_in_expr(arg, None, ctx, errors);
        }
        Expression::Array(items) => {
            for it in items {
                infer_in_expr(it, None, ctx, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                infer_in_expr(v, None, ctx, errors);
            }
        }
        Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::NotExists(_)
        | Expression::DotPath { .. } => {}
    }
}

/// Build `?var → property_iri` for every property-bound variable in
/// `program`. Lighter than the typechecker's [`PropertyBinding`]
/// because this pass only needs the property IRI (the active
/// VectorIndex lookup goes through that).
fn build_property_variable_index(program: &Program, layer: &Layer) -> BTreeMap<String, Iri> {
    let mut out: BTreeMap<String, Iri> = BTreeMap::new();
    let mut visit = |patterns: &[Pattern]| {
        for pat in patterns {
            for pp in &pat.properties {
                if let ValueOrVariable::Variable(var) = &pp.object {
                    if let Some(property_iri) = resolve_property_name(&pp.property, layer) {
                        out.entry(var.name.clone()).or_insert(property_iri);
                    }
                }
            }
        }
    };
    let collect = |part: &MatchPart| -> Vec<Pattern> { part.patterns().cloned().collect() };
    visit(&collect(&program.query.body));
    for def in &program.definitions {
        visit(&collect(&def.body));
    }
    out
}

fn resolve_property_name(name: &Name, layer: &Layer) -> Option<Iri> {
    match name {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(s) => {
            let prop_class = Iri::parse(wk::PROPERTY).ok()?;
            let short_prop = Iri::parse(wk::SHORT_NAME).ok()?;
            for (iri, res) in layer.iter_all_resources() {
                if !res.is_instance_of(&prop_class) {
                    continue;
                }
                if let Some(Value::String(sn)) = res.get(&short_prop) {
                    if sn == s {
                        return Some(iri.clone());
                    }
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::bootstrap;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Resource;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a layer with a string Property + a VectorIndex whose
    /// `vec_model` is a fixed model IRI. The fixture is the minimum
    /// the inference pass needs to walk a head.
    fn build_layer_with_vector_index(model: &str) -> Arc<crate::layer::Layer> {
        let ctx = bootstrap().expect("bootstrap");
        let parent = Arc::clone(ctx.head());
        let mut b = LayerBuilder::new("embed-infer", Some(parent));

        let body_iri = "urn:eigenius:test:body";
        let mut prop = Resource::new(iri(body_iri));
        prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        b.add_resource(prop).unwrap();

        let mut vi = Resource::new(iri("urn:eigenius:test:vi"));
        vi.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::VECTOR_INDEX_CLASS))]),
        );
        vi.set(iri(wk::TARGET_PROPERTY), Value::ResourceRef(iri(body_iri)));
        vi.set(iri(wk::VEC_MODEL), Value::ResourceRef(iri(model)));
        vi.set(iri(wk::VEC_DIM), Value::Integer(8));
        b.add_resource(vi).unwrap();

        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    fn parse(src: &str) -> Program {
        parser::parse(tokenize(src).unwrap()).unwrap()
    }

    /// Helper: drill into the AST to the EMBED call's args. Assumes
    /// the program's `RETURN [] { x: <expr> }` shape with EMBED
    /// somewhere inside `<expr>`. Walks until it finds EMBED.
    fn find_embed_args(expr: &Expression) -> Vec<Expression> {
        match expr {
            Expression::FunctionCall { name, args } if name == "EMBED" => args.clone(),
            Expression::FunctionCall { args, .. } => {
                args.iter().flat_map(find_embed_args).collect()
            }
            Expression::Binary { left, right, .. } => {
                let mut l = find_embed_args(left);
                l.extend(find_embed_args(right));
                l
            }
            Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
                find_embed_args(operand)
            }
            Expression::Aggregate { arg, .. } => find_embed_args(arg),
            Expression::Array(items) => items.iter().flat_map(find_embed_args).collect(),
            Expression::Object(pairs) => {
                pairs.iter().flat_map(|(_, v)| find_embed_args(v)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Locate the first EMBED call's args by walking every
    /// expression position in the program.
    fn first_embed_args(program: &Program) -> Vec<Expression> {
        for cond in &program.query.body.conditions {
            let v = find_embed_args(cond);
            if !v.is_empty() {
                return v;
            }
        }
        for item in &program.query.result {
            let v = find_embed_args(&item.expression);
            if !v.is_empty() {
                return v;
            }
        }
        Vec::new()
    }

    #[test]
    fn single_context_infers_model_and_rewrites_to_two_args() {
        let model = "urn:eigenius:embed:m1";
        let layer = build_layer_with_vector_index(model);
        let mut program = parse(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            WHERE VECTOR_NEAR(?vec, EMBED("hello"), 5)
            RETURN [] { d: ?d }
            "#,
        );
        let errors = infer_embed_models(&mut program, &layer);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        let args = first_embed_args(&program);
        assert_eq!(args.len(), 2, "EMBED should now have 2 args");
        match &args[1] {
            Expression::Literal(Literal::String(s)) => assert_eq!(s, model),
            other => panic!("expected literal-string model arg, got {other:?}"),
        }
    }

    #[test]
    fn explicit_model_passes_through_unchanged() {
        let model = "urn:eigenius:embed:m1";
        let layer = build_layer_with_vector_index(model);
        // Same model explicitly — should not flag.
        let mut program = parse(&format!(
            r#"
            MATCH ?d {{ "urn:eigenius:test:body": ?vec }}
            WHERE VECTOR_NEAR(?vec, EMBED("hello", "{model}"), 5)
            RETURN [] {{ d: ?d }}
            "#,
        ));
        let errors = infer_embed_models(&mut program, &layer);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        let args = first_embed_args(&program);
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn explicit_model_mismatch_is_rejected() {
        let model = "urn:eigenius:embed:m1";
        let other = "urn:eigenius:embed:other";
        let layer = build_layer_with_vector_index(model);
        let mut program = parse(&format!(
            r#"
            MATCH ?d {{ "urn:eigenius:test:body": ?vec }}
            WHERE VECTOR_NEAR(?vec, EMBED("hello", "{other}"), 5)
            RETURN [] {{ d: ?d }}
            "#,
        ));
        let errors = infer_embed_models(&mut program, &layer);
        assert!(
            errors
                .iter()
                .any(|e| e.rule == "embed_explicit_model_mismatch"),
            "expected embed_explicit_model_mismatch; got {errors:?}"
        );
    }

    #[test]
    fn embed_with_no_context_fails_inference() {
        let model = "urn:eigenius:embed:m1";
        let layer = build_layer_with_vector_index(model);
        // EMBED in RETURN position with no surrounding VECTOR_NEAR /
        // VECTOR_SIM — no candidate model.
        let mut program = parse(
            r#"
            MATCH ?d {}
            RETURN [] { e: EMBED("orphan") }
            "#,
        );
        let errors = infer_embed_models(&mut program, &layer);
        assert!(
            errors.iter().any(|e| e.rule == "embed_no_inferred_model"),
            "expected embed_no_inferred_model; got {errors:?}"
        );
    }

    #[test]
    fn embed_inside_vector_sim_also_infers() {
        let model = "urn:eigenius:embed:m1";
        let layer = build_layer_with_vector_index(model);
        let mut program = parse(
            r#"
            MATCH ?d { "urn:eigenius:test:body": ?vec }
            RETURN [] { d: ?d, s: VECTOR_SIM(?vec, EMBED("query")) }
            "#,
        );
        let errors = infer_embed_models(&mut program, &layer);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        let args = first_embed_args(&program);
        assert_eq!(args.len(), 2);
        match &args[1] {
            Expression::Literal(Literal::String(s)) => assert_eq!(s, model),
            other => panic!("expected literal-string model arg, got {other:?}"),
        }
    }

    #[test]
    fn vector_near_first_arg_unrelated_does_not_impose_context() {
        // VECTOR_NEAR is structurally well-formed but its first arg
        // isn't a property-bound variable (it's a literal). The
        // surrounding-model context resolves to None, and the EMBED
        // in arg[1] gets the empty-candidates path.
        let model = "urn:eigenius:embed:m1";
        let layer = build_layer_with_vector_index(model);
        let mut program = parse(
            r#"
            MATCH ?d {}
            WHERE VECTOR_NEAR("not a var", EMBED("text"), 5)
            RETURN [] { d: ?d }
            "#,
        );
        let errors = infer_embed_models(&mut program, &layer);
        assert!(
            errors.iter().any(|e| e.rule == "embed_no_inferred_model"),
            "expected embed_no_inferred_model when VECTOR_NEAR's ?vec is unresolved; got {errors:?}"
        );
    }
}
