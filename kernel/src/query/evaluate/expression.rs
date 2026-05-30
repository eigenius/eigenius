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

//! Expression evaluation: arithmetic, comparison, function calls,
//! Decidable QueryClass dispatch, Verdict postfix predicates,
//! aggregates and GROUP BY.

use crate::institution::registry::DispatchRole;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::error::QueryError;
use crate::query::functions::{self, like_match, to_f64, values_compare, values_equal};
use std::collections::BTreeMap;

use super::pattern::{find_property_by_shortname, literal_to_value, Binding};
use super::FiberRuntime;

/// Evaluate an expression against a binding.
pub(super) fn eval_expression(
    expr: &Expression,
    binding: &Binding,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    match expr {
        Expression::Literal(lit) => Ok(literal_to_value(lit)),
        Expression::Variable(var) => binding
            .get(&var.name)
            .cloned()
            .ok_or_else(|| QueryError::evaluation(format!("unbound variable: ?{}", var.name))),
        Expression::Binary { op, left, right } => {
            let l = eval_expression(left, binding, layer, runtime)?;
            let r = eval_expression(right, binding, layer, runtime)?;
            eval_binary(*op, &l, &r)
        }
        Expression::Unary { op, operand } => {
            let v = eval_expression(operand, binding, layer, runtime)?;
            eval_unary(*op, &v)
        }
        Expression::VerdictPredicate { kind, operand } => {
            let v = eval_expression(operand, binding, layer, runtime)?;
            eval_verdict_predicate(*kind, &v, layer, runtime)
        }
        Expression::NotExists(var) => Ok(Value::Boolean(!binding.contains_key(&var.name))),
        Expression::FunctionCall { name, args } => {
            // D43 §4.6 — TEXT_MATCH / TEXT_SCORE dispatch through the
            // query-scoped retrieval context, which holds the
            // property-bound `?var` map and a per-`(index, query)`
            // probe cache. Evaluated before the generic arg-vector
            // path so the AST args (not the row-evaluated values)
            // can supply the property-variable name.
            if name == "TEXT_MATCH" || name == "TEXT_SCORE" {
                return eval_text_retrieval(name, args, binding, runtime);
            }
            if name == "EMBED" {
                return eval_embed(args, binding, layer, runtime);
            }
            if name == "VECTOR_NEAR" || name == "VECTOR_SIM" {
                return eval_vector_retrieval(name, args, binding, layer, runtime);
            }
            let arg_vals: Result<Vec<Value>, QueryError> = args
                .iter()
                .map(|a| eval_expression(a, binding, layer, runtime))
                .collect();
            let arg_vals = arg_vals?;
            // D2 §3.8: qualified-name function calls dispatch as a
            // Decidable QueryClass invocation. The result is a
            // Verdict-typed resource (Value::Embedded). Comorphism
            // dispatch in expression position is not supported under
            // D14 — comorphisms surface only as FIBER param coercion
            // (D2 §3.5).
            if name.contains(':') {
                if let Ok(iri_parsed) = Iri::parse(name) {
                    if let Some(verdict) = try_dispatch_decidable(&iri_parsed, &arg_vals, runtime)?
                    {
                        return Ok(verdict);
                    }
                }
            }
            functions::call_function(name, &arg_vals)
        }
        Expression::Aggregate { .. } => {
            // Aggregates are handled during GROUP BY, not per-binding
            Err(QueryError::evaluation(
                "aggregate function outside GROUP BY context",
            ))
        }
        Expression::DotPath { root, segments } => {
            // Resolve the root variable to a resource IRI
            let root_val = binding.get(&root.name).ok_or_else(|| {
                QueryError::evaluation(format!("unbound variable: ?{}", root.name))
            })?;
            let mut current_iri = match root_val {
                Value::String(s) => Iri::parse(s).map_err(|_| {
                    QueryError::evaluation(format!("variable ?{} is not a resource IRI", root.name))
                })?,
                _ => {
                    return Err(QueryError::evaluation(format!(
                        "variable ?{} is not a resource IRI",
                        root.name
                    )))
                }
            };

            // Walk each segment except the last — resolve intermediate
            // resources via the overlay (FIBER responses) ∪ layer
            // chain, in that order. Without the overlay check, a
            // dot-path on a `FIBER … AS ?bound` variable would fail
            // to find ?bound because the synthesized response IRI
            // lives in the transient overlay, not the chain.
            for (i, segment) in segments.iter().enumerate() {
                let resource = resolve_iri_string(current_iri.as_str(), layer, runtime)
                    .ok_or_else(|| {
                        QueryError::evaluation(format!(
                            "resource '{}' not found in layer chain or FIBER overlay",
                            current_iri
                        ))
                    })?;
                let prop_iri = find_property_by_shortname(segment, resource.properties())
                    .ok_or_else(|| {
                        QueryError::evaluation(format!(
                            "property '{}' not found on resource '{}'",
                            segment, current_iri
                        ))
                    })?;
                let value = resource.get(&prop_iri).ok_or_else(|| {
                    QueryError::evaluation(format!(
                        "property '{}' has no value on resource '{}'",
                        segment, current_iri
                    ))
                })?;

                if i < segments.len() - 1 {
                    // Intermediate segment: must be a resource reference
                    current_iri = match value {
                        Value::String(s) => Iri::parse(s).map_err(|_| {
                            QueryError::evaluation(format!(
                                "property '{}' on '{}' is not a resource reference",
                                segment, current_iri
                            ))
                        })?,
                        _ => {
                            return Err(QueryError::evaluation(format!(
                                "property '{}' on '{}' is not a resource reference",
                                segment, current_iri
                            )))
                        }
                    };
                } else {
                    // Final segment: return the value
                    return Ok(value.clone());
                }
            }
            Err(QueryError::evaluation("empty dot-path segments"))
        }
        Expression::Array(elements) => {
            let vals: Result<Vec<Value>, QueryError> = elements
                .iter()
                .map(|e| eval_expression(e, binding, layer, runtime))
                .collect();
            Ok(Value::Array(vals?))
        }
        Expression::Object(_) => Err(QueryError::evaluation(
            "object literals in expressions not yet implemented",
        )),
    }
}

fn eval_binary(op: BinaryOp, left: &Value, right: &Value) -> Result<Value, QueryError> {
    match op {
        BinaryOp::Eq => Ok(Value::Boolean(values_equal(left, right))),
        BinaryOp::Neq => Ok(Value::Boolean(!values_equal(left, right))),
        BinaryOp::Lt => Ok(Value::Boolean(
            values_compare(left, right) == Some(std::cmp::Ordering::Less),
        )),
        BinaryOp::Lte => Ok(Value::Boolean(matches!(
            values_compare(left, right),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ))),
        BinaryOp::Gt => Ok(Value::Boolean(
            values_compare(left, right) == Some(std::cmp::Ordering::Greater),
        )),
        BinaryOp::Gte => Ok(Value::Boolean(matches!(
            values_compare(left, right),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ))),
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Mod
        | BinaryOp::Pow => {
            let a = to_f64(left)
                .ok_or_else(|| QueryError::evaluation("arithmetic requires numeric operands"))?;
            let b = to_f64(right)
                .ok_or_else(|| QueryError::evaluation("arithmetic requires numeric operands"))?;
            let result = match op {
                BinaryOp::Add => a + b,
                BinaryOp::Sub => a - b,
                BinaryOp::Mul => a * b,
                BinaryOp::Div => {
                    if b == 0.0 {
                        return Err(QueryError::evaluation("division by zero"));
                    }
                    a / b
                }
                BinaryOp::Mod => {
                    if b == 0.0 {
                        return Err(QueryError::evaluation("modulo by zero"));
                    }
                    a % b
                }
                BinaryOp::Pow => a.powf(b),
                _ => unreachable!(),
            };
            // Preserve integer type if both operands are integers and result is integral
            if matches!(left, Value::Integer(_))
                && matches!(right, Value::Integer(_))
                && result.fract() == 0.0
                && !matches!(op, BinaryOp::Pow)
            {
                Ok(Value::Integer(result as i64))
            } else {
                Ok(Value::Float(result))
            }
        }
        BinaryOp::StringConcat => {
            let a = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("|| requires string operands"))?;
            let b = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("|| requires string operands"))?;
            Ok(Value::String(format!("{a}{b}")))
        }
        BinaryOp::And => {
            let a = left
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("AND requires boolean operands"))?;
            let b = right
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("AND requires boolean operands"))?;
            Ok(Value::Boolean(a && b))
        }
        BinaryOp::Or => {
            let a = left
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("OR requires boolean operands"))?;
            let b = right
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("OR requires boolean operands"))?;
            Ok(Value::Boolean(a || b))
        }
        BinaryOp::In => {
            if let Value::Array(arr) = right {
                Ok(Value::Boolean(arr.iter().any(|v| values_equal(left, v))))
            } else {
                Err(QueryError::evaluation("IN requires array on right side"))
            }
        }
        BinaryOp::NotIn => {
            if let Value::Array(arr) = right {
                Ok(Value::Boolean(!arr.iter().any(|v| values_equal(left, v))))
            } else {
                Err(QueryError::evaluation(
                    "NOT IN requires array on right side",
                ))
            }
        }
        BinaryOp::Like => {
            let val = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("LIKE requires string operands"))?;
            let pat = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("LIKE requires string operands"))?;
            Ok(Value::Boolean(like_match(val, pat)))
        }
        BinaryOp::NotLike => {
            let val = left
                .as_str()
                .ok_or_else(|| QueryError::evaluation("NOT LIKE requires string operands"))?;
            let pat = right
                .as_str()
                .ok_or_else(|| QueryError::evaluation("NOT LIKE requires string operands"))?;
            Ok(Value::Boolean(!like_match(val, pat)))
        }
    }
}

fn eval_unary(op: UnaryOp, val: &Value) -> Result<Value, QueryError> {
    match op {
        UnaryOp::Not => {
            let b = val
                .as_boolean()
                .ok_or_else(|| QueryError::evaluation("NOT requires boolean"))?;
            Ok(Value::Boolean(!b))
        }
        UnaryOp::Pos => {
            let n = to_f64(val).ok_or_else(|| QueryError::evaluation("+ requires numeric"))?;
            Ok(Value::Float(n))
        }
        UnaryOp::Neg => match val {
            Value::Integer(n) => Ok(Value::Integer(-n)),
            Value::Float(f) => Ok(Value::Float(-f)),
            _ => Err(QueryError::evaluation("- requires numeric")),
        },
    }
}

/// Project a `Verdict`-typed value to `Boolean` per a postfix predicate
/// (D2 v2 §3.7 / §3.8). The operand is one of:
///
/// - `Value::Embedded(verdict)` — the Verdict resource directly.
/// - `Value::String(iri)` / `Value::ResourceRef(iri)` — a synthesized
///   IRI (typically from a FIBER `AS ?var` binding) that resolves to
///   the response resource through the runtime's transient overlay or
///   the layer chain.
fn eval_verdict_predicate(
    kind: crate::query::ast::VerdictPredicate,
    val: &Value,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    let resolved: Resource;
    let resource: &Resource = match val {
        Value::Embedded(r) => r.as_ref(),
        Value::String(s) => {
            resolved = resolve_iri_string(s, layer, runtime).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "{kw} operand IRI `{s}` does not resolve to a resource (FIBER overlay or layer chain)",
                    kw = kind.ctor_name(),
                ))
            })?;
            &resolved
        }
        Value::ResourceRef(iri) => {
            resolved = resolve_iri_string(iri.as_str(), layer, runtime).ok_or_else(|| {
                QueryError::evaluation(format!(
                    "{kw} operand IRI `{iri}` does not resolve to a resource",
                    kw = kind.ctor_name(),
                ))
            })?;
            &resolved
        }
        other => {
            return Err(QueryError::evaluation(format!(
                "{kw} expects a Verdict-typed operand; got {other:?}",
                kw = kind.ctor_name(),
            )));
        }
    };
    let ctor_iri = Iri::parse(wk::CTOR_NAME).expect("well-known IRI");
    let ctor = resource
        .get(&ctor_iri)
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| {
            QueryError::evaluation(
                "Verdict postfix predicate operand carries no `ctor_name` property",
            )
        })?;
    Ok(Value::Boolean(ctor == kind.ctor_name()))
}

/// Resolve a String IRI to a Resource — checks the FiberOverlay first
/// (so FIBER-bound responses are visible) then walks the layer chain.
pub(super) fn resolve_iri_string(
    s: &str,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Option<Resource> {
    let iri = Iri::parse(s).ok()?;
    if let Some(overlay) = runtime.overlay {
        for (entry_iri, entry_resource) in overlay {
            if entry_iri == &iri {
                return Some(entry_resource.clone());
            }
        }
    }
    layer.resolve(&iri).map(|r| (*r).clone())
}

/// D43 §4.6 — evaluate `TEXT_MATCH(?prop, "query")` (→ Boolean) or
/// `TEXT_SCORE(?prop, "query")` (→ Float) for one row's binding.
///
/// Semantics:
///
/// 1. The typechecker has already validated `?prop` is a property-
///    bound variable; the call's purpose is *"is this row's source
///    subject in the text-index hits, and what is its BM25 score?"*.
/// 2. Look up the property-binding info (subject_variable name +
///    property IRI) from the query-scoped
///    [`retrieval::RetrievalContext`].
/// 3. Find the active TextIndex Resource for that property; run
///    (or fetch from the per-query cache) the index probe via
///    [`crate::query::text::search::run_text_search`].
/// 4. Resolve this row's subject IRI from the binding map.
/// 5. For TEXT_MATCH: return true iff the subject appears in the
///    hit set. For TEXT_SCORE: return the hit's BM25 score, or 0.0
///    if the subject is not in the hit set.
fn eval_text_retrieval(
    fn_name: &str,
    args: &[Expression],
    binding: &Binding,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    use super::retrieval::TextSearchOutcome;

    let retrieval = runtime.retrieval.ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} requires a query-scoped retrieval context; \
             the evaluator entry point did not build one"
        ))
    })?;

    if args.len() != 2 {
        return Err(QueryError::evaluation(format!(
            "{fn_name} expects 2 arguments, got {}",
            args.len()
        )));
    }
    let var = match &args[0] {
        Expression::Variable(v) => v,
        _ => {
            return Err(QueryError::evaluation(format!(
                "{fn_name} first argument must be a property-bound variable"
            )));
        }
    };
    let query = match &args[1] {
        Expression::Literal(Literal::String(s)) => s.as_str(),
        _ => {
            return Err(QueryError::evaluation(format!(
                "{fn_name} second argument must be a literal query string"
            )));
        }
    };

    let binding_info = retrieval.binding_for(&var.name).ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} argument `?{}` is not bound by a MATCH property pattern",
            var.name
        ))
    })?;
    let active = retrieval
        .active_index_for(&binding_info.property_iri)
        .ok_or_else(|| {
            QueryError::evaluation(format!(
                "no active TextIndex for property `{}` at this head",
                binding_info.property_iri.as_str()
            ))
        })?;

    let outcome = retrieval.probe(active, query);
    let hits = match outcome.as_ref() {
        TextSearchOutcome::Ok(h) => h,
        TextSearchOutcome::Err(e) => {
            return Err(QueryError::evaluation(format!(
                "TextIndex probe failed: {e:?}"
            )));
        }
    };

    let subject_value = binding.get(&binding_info.subject_variable).ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} cannot resolve source subject `?{}` for `?{}`",
            binding_info.subject_variable, var.name
        ))
    })?;
    let subject_iri = match subject_value {
        Value::String(s) => Iri::parse(s).ok(),
        Value::ResourceRef(iri) => Some(iri.clone()),
        _ => None,
    }
    .ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} source subject `?{}` is not an IRI",
            binding_info.subject_variable
        ))
    })?;

    let hit = hits.iter().find(|h| h.subject == subject_iri);
    match fn_name {
        "TEXT_MATCH" => Ok(Value::Boolean(hit.is_some())),
        "TEXT_SCORE" => Ok(Value::Float(hit.map(|h| h.score as f64).unwrap_or(0.0))),
        _ => unreachable!("eval_text_retrieval dispatched on unknown name"),
    }
}

/// D43 §3.5 — evaluate `EMBED("text", "<model_iri>")` → `Value::Vector`.
///
/// v1 signature is fully positional: `EMBED(text, model_iri)`. The
/// keyword form (`EMBED("text", model: M)`) and typecheck-time
/// model inference from the surrounding `VECTOR_NEAR` context
/// (D43 §4.4) are deferred — both add parser/typecheck work without
/// changing the dispatch shape.
///
/// Steps:
/// 1. Validate argument shape (count + both literal strings).
/// 2. Resolve the Embedder from [`FiberRuntime::embedders`].
/// 3. Call [`crate::program::embedder::Embedder::embed`] and verify
///    the returned length equals the declared dim — a length
///    mismatch is an implementation bug worth surfacing as a query
///    error rather than silently constructing a mis-sized vector.
/// 4. Wrap into [`Value::Vector`] for downstream consumers
///    (`VECTOR_NEAR` / `VECTOR_SIM`, M5).
fn eval_embed(
    args: &[Expression],
    binding: &Binding,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    if args.len() != 2 {
        return Err(QueryError::evaluation(format!(
            "EMBED expects 2 arguments (text, model_iri), got {}",
            args.len()
        )));
    }
    let text_val = eval_expression(&args[0], binding, layer, runtime)?;
    let text = text_val
        .as_str()
        .ok_or_else(|| QueryError::evaluation("EMBED first argument must evaluate to a string"))?;

    let model_val = eval_expression(&args[1], binding, layer, runtime)?;
    let model_str = model_val.as_iri_str().ok_or_else(|| {
        QueryError::evaluation("EMBED second argument must evaluate to an IRI-shaped string")
    })?;
    let model_iri = Iri::parse(model_str)
        .map_err(|e| QueryError::evaluation(format!("EMBED model_iri is not a valid IRI: {e}")))?;

    // D43 §5.3 — consult the content-addressed cache before paying
    // for an Embedder dispatch. A hit on `(content_hash, model_iri)`
    // returns the cached vector directly; a miss falls through to
    // dispatch and inserts the result.
    if let Some(cache) = runtime.embedding_cache {
        if let Some(cached) = cache.get(text, &model_iri) {
            return Ok(Value::Vector {
                model_iri,
                data: (*cached).clone(),
            });
        }
    }

    let registry = runtime.embedders.ok_or_else(|| {
        QueryError::evaluation(
            "EMBED requires an EmbedderRegistry on the query runtime; \
             none was supplied",
        )
    })?;
    let embedder = registry.get(&model_iri).ok_or_else(|| {
        QueryError::evaluation(format!(
            "EMBED model `{}` is not registered in the Embedder registry",
            model_iri.as_str()
        ))
    })?;

    let data = embedder
        .embed(text)
        .map_err(|e| QueryError::evaluation(format!("EMBED dispatch failed: {e}")))?;
    let declared_dim = embedder.dim() as usize;
    if data.len() != declared_dim {
        return Err(QueryError::evaluation(format!(
            "EMBED dispatch returned {} values but `{}` declares dim={}",
            data.len(),
            model_iri.as_str(),
            declared_dim
        )));
    }

    if let Some(cache) = runtime.embedding_cache {
        cache.insert(text, &model_iri, std::sync::Arc::new(data.clone()));
    }
    Ok(Value::Vector { model_iri, data })
}

/// D43 §3.4 / §4.5 — evaluate `VECTOR_NEAR` (→ Boolean) or
/// `VECTOR_SIM` (→ Float) for one row's binding.
///
/// v1 positional signatures (kwarg `k:` is the M5 follow-up):
///
/// ```text
/// VECTOR_NEAR(?vec, query_vec, K) : Boolean
/// VECTOR_SIM(?vec, query_vec)     : Float
/// ```
///
/// Steps:
///
/// 1. Resolve the property-binding for `?vec` via the query-scoped
///    [`super::retrieval::RetrievalContext`] — same plumbing the
///    text retrieval primitives use to map a property-bound `?var`
///    back to its source subject variable.
/// 2. Find the active VectorIndex Resource for the bound property.
/// 3. Evaluate `query_vec` — must produce a [`Value::Vector`] (the
///    canonical producer is `EMBED(...)`) whose `model_iri` and
///    dimensionality match the active VectorIndex's declarations.
/// 4. For VECTOR_NEAR: run [`crate::query::vector::search::top_k_subjects`]
///    and check whether the row's source subject IRI is in the
///    top-K hit set. For VECTOR_SIM: run
///    [`crate::query::vector::search::subject_similarity`] for the
///    row's source subject directly.
fn eval_vector_retrieval(
    fn_name: &str,
    args: &[Expression],
    binding: &Binding,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Value, QueryError> {
    use crate::query::vector::distance::Metric;
    use crate::query::vector::search::{subject_similarity, top_k_subjects};

    let retrieval = runtime.retrieval.ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} requires a query-scoped retrieval context; \
             the evaluator entry point did not build one"
        ))
    })?;

    let expected_arity = if fn_name == "VECTOR_NEAR" { 3 } else { 2 };
    if args.len() != expected_arity {
        return Err(QueryError::evaluation(format!(
            "{fn_name} expects {expected_arity} arguments, got {}",
            args.len()
        )));
    }

    let var = match &args[0] {
        Expression::Variable(v) => v,
        _ => {
            return Err(QueryError::evaluation(format!(
                "{fn_name} first argument must be a property-bound variable"
            )));
        }
    };
    let binding_info = retrieval.binding_for(&var.name).ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} argument `?{}` is not bound by a MATCH property pattern",
            var.name
        ))
    })?;
    let active = retrieval
        .active_vector_index_for(&binding_info.property_iri)
        .ok_or_else(|| {
            QueryError::evaluation(format!(
                "no active VectorIndex for property `{}` at this head",
                binding_info.property_iri.as_str()
            ))
        })?;

    // Evaluate the query vector. Must produce a `Value::Vector`
    // whose model + dim match the active VectorIndex's declared
    // values (defence in depth — the typechecker is the primary
    // gate; this catches programmatic mis-construction).
    let qv = eval_expression(&args[1], binding, layer, runtime)?;
    let (qv_model, qv_data) = match qv.as_vector() {
        Some(pair) => pair,
        None => {
            return Err(QueryError::evaluation(format!(
                "{fn_name} second argument must evaluate to a Vector (e.g. via EMBED(...))"
            )));
        }
    };
    if qv_model != &active.model {
        return Err(QueryError::evaluation(format!(
            "{fn_name} model mismatch: query vector is `{}` but active VectorIndex declares `{}`",
            qv_model.as_str(),
            active.model.as_str()
        )));
    }
    if qv_data.len() != active.dim as usize {
        return Err(QueryError::evaluation(format!(
            "{fn_name} dim mismatch: query vector is dim={} but active VectorIndex declares dim={}",
            qv_data.len(),
            active.dim
        )));
    }

    let metric = Metric::from_short_name(active.distance.as_str()).ok_or_else(|| {
        QueryError::evaluation(format!(
            "active VectorIndex `{}` declares unknown distance `{}`",
            active.iri.as_str(),
            active.distance.as_str()
        ))
    })?;

    let vector_index = retrieval.layer().storage().vector_index.as_ref();

    // Resolve the row's source subject IRI from the binding map.
    let subject_value = binding.get(&binding_info.subject_variable).ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} cannot resolve source subject `?{}` for `?{}`",
            binding_info.subject_variable, var.name
        ))
    })?;
    let subject_iri = match subject_value {
        Value::String(s) => Iri::parse(s).ok(),
        Value::ResourceRef(iri) => Some(iri.clone()),
        _ => None,
    }
    .ok_or_else(|| {
        QueryError::evaluation(format!(
            "{fn_name} source subject `?{}` is not an IRI",
            binding_info.subject_variable
        ))
    })?;

    match fn_name {
        "VECTOR_NEAR" => {
            let k = match &args[2] {
                Expression::Literal(Literal::Integer(n)) if *n > 0 => *n as usize,
                _ => {
                    return Err(QueryError::evaluation(
                        "VECTOR_NEAR k must be a positive integer literal".to_string(),
                    ));
                }
            };
            let hits = top_k_subjects(
                retrieval.layer(),
                vector_index,
                runtime.vector_segment_cache,
                &active.iri,
                qv_data,
                k,
                &active.model,
                metric,
            )
            .map_err(|e| QueryError::evaluation(format!("VECTOR_NEAR probe failed: {e}")))?;
            let in_topk = hits.iter().any(|h| h.subject == subject_iri);
            Ok(Value::Boolean(in_topk))
        }
        "VECTOR_SIM" => {
            let sim = subject_similarity(
                retrieval.layer(),
                vector_index,
                runtime.vector_segment_cache,
                &active.iri,
                &subject_iri,
                qv_data,
                &active.model,
                metric,
            )
            .map_err(|e| QueryError::evaluation(format!("VECTOR_SIM probe failed: {e}")))?;
            Ok(Value::Float(sim.unwrap_or(0.0) as f64))
        }
        _ => unreachable!("eval_vector_retrieval dispatched on unknown name"),
    }
}

/// Check if any return item uses an aggregate function.
pub(super) fn has_aggregates(result: &[ReturnItem]) -> bool {
    result
        .iter()
        .any(|item| expr_has_aggregate(&item.expression))
}

fn expr_has_aggregate(expr: &Expression) -> bool {
    match expr {
        Expression::Aggregate { .. } => true,
        Expression::Binary { left, right, .. } => {
            expr_has_aggregate(left) || expr_has_aggregate(right)
        }
        Expression::Unary { operand, .. } => expr_has_aggregate(operand),
        Expression::VerdictPredicate { operand, .. } => expr_has_aggregate(operand),
        Expression::FunctionCall { args, .. } => args.iter().any(expr_has_aggregate),
        _ => false,
    }
}

/// Try to dispatch a qualified-name call as a Decidable QueryClass
/// invocation (D2 §3.8 / §6.13). Returns:
///
/// - `Ok(Some(verdict))` if the IRI resolved to a Decidable QueryClass
///   and dispatch ran end-to-end. The Verdict is returned as a
///   `Value::Embedded` resource carrying `is_a = [Verdict]` and
///   `ctor_name`.
/// - `Ok(None)` if the index/runtime aren't attached, the IRI doesn't
///   resolve to a QueryClass, or the resolved QueryClass has no
///   Decidable role. The caller falls through to builtin function
///   dispatch (which raises `unknown function`).
/// - `Err(_)` if the index *did* find a Decidable QueryClass but a
///   downstream step failed (missing institution registration,
///   handler failure, etc.). A configured-but-broken QueryClass is a
///   structural error, not a reason to silently fall through.
///
/// Comorphism dispatch is not available in expression position under
/// D14; comorphisms surface only as FIBER param coercion (D2 §3.5).
fn try_dispatch_decidable(
    iri: &Iri,
    args: &[Value],
    runtime: FiberRuntime<'_>,
) -> Result<Option<Value>, QueryError> {
    let (Some(index), Some(inst_runtime), Some(ctx)) =
        (runtime.index, runtime.runtime, runtime.ctx)
    else {
        return Ok(None);
    };
    let Some(qc_entry) = index.query_class(iri) else {
        return Ok(None);
    };
    if !qc_entry.dispatch_roles.contains(&DispatchRole::Decidable) {
        return Ok(None);
    }
    let institution = inst_runtime.get(&qc_entry.institution_ref).ok_or_else(|| {
        QueryError::evaluation(format!(
            "Decidable QueryClass `{iri}` declares institution `{}` not registered in runtime",
            qc_entry.institution_ref
        ))
    })?;

    // Marshal positional args onto a synthetic input resource of the
    // QueryClass's input class via the shared
    // `institution::marshal::marshal_decidable_input` helper. Same
    // logic as the kernel-side `nbe::eval::try_d14_decide` (D14 §9.2)
    // — typed required properties populated in `requires` order,
    // IRI-shaped args targeting `core:resource` properties
    // dereferenced to embedded resources.
    let input = crate::institution::marshal::marshal_decidable_input(
        &qc_entry.query_class,
        args,
        ctx.head(),
    )
    .map_err(|e| QueryError::evaluation(format!("Decidable QueryClass `{iri}`: {e}")))?;

    let outcome = institution
        .query(&qc_entry.query_handler, &input, ctx)
        .map_err(|e| {
            QueryError::evaluation(format!(
                "Decidable QueryClass `{iri}` handler `{}` failed: {e}",
                qc_entry.query_handler
            ))
        })?;

    // Decidable evaluation produces no chain-side RuntimeInvocation
    // commit — it's type-check-time reduction, not a Load. The
    // partial provenance (if any) is dropped here on purpose.
    Ok(Some(Value::Embedded(Box::new(outcome.output))))
}

/// Apply GROUP BY and aggregation.
pub(super) fn apply_group_by(
    group_by: &[Expression],
    result: &[ReturnItem],
    bindings: &[Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Vec<Binding>, QueryError> {
    // Group bindings by their group key values
    let mut groups: BTreeMap<Vec<String>, Vec<&Binding>> = BTreeMap::new();

    for binding in bindings {
        let key: Vec<String> = group_by
            .iter()
            .map(|expr| {
                eval_expression(expr, binding, layer, runtime)
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_default()
            })
            .collect();
        groups.entry(key).or_default().push(binding);
    }

    let mut result_bindings = Vec::new();
    for group in groups.values() {
        let mut binding = group[0].clone(); // Start with first binding for non-aggregate values

        // Compute aggregates
        for item in result {
            if let Some((agg_name, agg_val)) =
                eval_aggregate(&item.expression, group, layer, runtime)?
            {
                binding.insert(agg_name, agg_val);
            }
        }

        result_bindings.push(binding);
    }

    Ok(result_bindings)
}

/// Evaluate an aggregate expression over a group of bindings.
fn eval_aggregate(
    expr: &Expression,
    group: &[&Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Option<(String, Value)>, QueryError> {
    if let Expression::Aggregate { op, arg } = expr {
        let values: Vec<Value> = group
            .iter()
            .filter_map(|b| eval_expression(arg, b, layer, runtime).ok())
            .collect();

        let result = match op {
            AggregateOp::Count => Value::Integer(values.len() as i64),
            AggregateOp::Sum => {
                let sum: f64 = values.iter().filter_map(to_f64).sum();
                if values.iter().all(|v| matches!(v, Value::Integer(_))) {
                    Value::Integer(sum as i64)
                } else {
                    Value::Float(sum)
                }
            }
            AggregateOp::Avg => {
                let vals: Vec<f64> = values.iter().filter_map(to_f64).collect();
                if vals.is_empty() {
                    Value::Float(0.0)
                } else {
                    Value::Float(vals.iter().sum::<f64>() / vals.len() as f64)
                }
            }
            AggregateOp::Min => values
                .iter()
                .min_by(|a, b| values_compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Integer(0)),
            AggregateOp::Max => values
                .iter()
                .max_by(|a, b| values_compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                .cloned()
                .unwrap_or(Value::Integer(0)),
        };

        // Use a synthetic name for the aggregate in the binding
        let name = format!("__agg_{op:?}");
        Ok(Some((name, result)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ExecutionContext, ExecutionMode};
    use crate::institution::error::InstitutionError;
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::layer::LayerBuilder;
    use crate::nbe::val::Val;
    use crate::ontology::eigon_json;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    const Q_INST_IRI: &str = "urn:eigenius:test:q_inst";
    const Q_POSITIVE_IRI: &str = "urn:eigenius:test:q_positive";
    const Q_INPUT_CLASS_IRI: &str = "urn:eigenius:test:QPositiveInput";
    const Q_HANDLER_IRI: &str = "urn:eigenius:test:proc:q_positive";

    /// Test institution implementing one Decidable QueryClass.
    /// `q_positive` returns Holds for a positive Integer on the
    /// input class's typed `arg_0` property, Fails otherwise. Phase
    /// 19d.7 dropped the `decide_args` array — args ride on typed
    /// required properties.
    struct QueryCapInst;

    const Q_ARG_0_PROP: &str = "urn:eigenius:test:QPositiveInput:arg_0";

    impl Institution for QueryCapInst {
        fn institution_iri(&self) -> &Iri {
            static INST: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
            INST.get_or_init(|| Iri::parse(Q_INST_IRI).unwrap())
        }
        fn extract_typed(
            &self,
            _: &Iri,
            _: &Resource,
            _: &ExecutionContext,
        ) -> Result<Val, InstitutionError> {
            unreachable!("test fixture only implements query")
        }
        fn reify(
            &self,
            _: &Iri,
            _: &Val,
            _: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!("test fixture only implements query")
        }
        fn query(
            &self,
            _procedure_iri: &Iri,
            input: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
            // Read the typed `arg_0` property the kernel populates
            // from the first positional ESL arg.
            let arg_0_iri = Iri::parse(Q_ARG_0_PROP).unwrap();
            let ok = match input.get(&arg_0_iri) {
                Some(v) => v.as_integer().is_some_and(|n| n > 0),
                _ => false,
            };
            // Build a Verdict response carrying ctor_name.
            let mut r = Resource::new_embedded();
            r.set(
                Iri::parse(wk::IS_A).unwrap(),
                Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
            );
            r.set(
                Iri::parse(wk::CTOR_NAME).unwrap(),
                Value::String(if ok { "Holds" } else { "Fails" }.into()),
            );
            Ok(crate::institution::runtime::QueryOutcome::from_output(r))
        }
    }

    /// Build a layer carrying the core ontology + the q_test fixtures
    /// (Institution, QueryClass, typed input class) and an
    /// `InstitutionIndex` over it. Phase 19d.7 typed-marshaling needs
    /// the input class to resolve on the layer the dispatch sees, so
    /// the test layer must include both the q_test resources and the
    /// core ontology — the previous split-layer setup (q_test parent
    /// = None, separately-built core layer for the ExecutionContext)
    /// no longer works.
    fn q_index() -> (
        Arc<crate::layer::Layer>,
        crate::layer::LayerStorage,
        Arc<InstitutionIndex>,
    ) {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_b = LayerBuilder::new("core", None);
        for r in core_resources {
            core_b.add_resource(r).unwrap();
        }
        let core_layer = Arc::new(core_b.build(storage.clone()));

        let mut b = LayerBuilder::new("q_test", Some(Arc::clone(&core_layer)));

        let mut inst = Resource::new(Iri::parse(Q_INST_IRI).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:Institution".to_string(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(Q_INST_IRI.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("QueryCapInst".to_string()),
        );
        b.add_resource(inst).unwrap();

        // Declare arg_0 property + the typed input class with
        // requires=[arg_0]. Phase 19d.7 typed-marshaling needs the
        // input class to resolve on the layer.
        let mut arg_prop = Resource::new(Iri::parse(Q_ARG_0_PROP).unwrap());
        arg_prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::PROPERTY.into())]),
        );
        b.add_resource(arg_prop).unwrap();
        let mut input_class = Resource::new(Iri::parse(Q_INPUT_CLASS_IRI).unwrap());
        input_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.into())]),
        );
        input_class.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            Value::Array(vec![Value::String(Q_ARG_0_PROP.into())]),
        );
        b.add_resource(input_class).unwrap();

        let mut qc = Resource::new(Iri::parse(Q_POSITIVE_IRI).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            Value::String(Q_INPUT_CLASS_IRI.to_string()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            Value::String(wk::VERDICT.to_string()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.to_string())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            Value::String(Q_HANDLER_IRI.to_string()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(Q_INST_IRI.to_string()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "fixture index errors: {errors:?}");
        (layer, storage, Arc::new(idx))
    }

    fn q_runtime() -> Arc<InstitutionRuntime> {
        let mut runtime = InstitutionRuntime::new();
        runtime.register(Box::new(QueryCapInst)).unwrap();
        Arc::new(runtime)
    }

    fn q_exec_ctx(
        layer: Arc<crate::layer::Layer>,
        storage: crate::layer::LayerStorage,
    ) -> ExecutionContext {
        ExecutionContext::new(layer, "q_test", ExecutionMode::ReadOnly, storage)
    }

    #[test]
    fn parser_accepts_qualified_function_calls() {
        // Parse-only: the parser must accept `ns:local(args)`
        // without requiring institution registration.
        let source = r#"
            MATCH ?x {}
            WHERE cap:q_positive(42)
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let _program = parser::parse(tokens).expect("parse qualified call");
    }

    #[test]
    fn where_clause_decide_dispatch_returns_verdict() {
        // Under D14, a Decidable QueryClass call returns a Verdict
        // resource (not a Boolean). The postfix predicate (D2 §3.8)
        // is what projects to Boolean — a separate parser concern.
        let (layer, storage, index) = q_index();
        let inst_runtime = q_runtime();
        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);

        let runtime = FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
            retrieval: None,
            embedders: None,
            embedding_cache: None,
            vector_segment_cache: None,
        };

        // Use FunctionCall directly at eval_expression level for a
        // focused test — the full-query integration would need more
        // pattern-matching infrastructure. This verifies the core
        // dispatch path.
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: Q_POSITIVE_IRI.to_string(),
            args: vec![Expression::Literal(Literal::Integer(42))],
        };
        let v = eval_expression(&expr, &binding, &layer, runtime).expect("eval");
        let verdict = match v {
            Value::Embedded(r) => r,
            other => panic!("expected embedded Verdict, got {other:?}"),
        };
        let ctor = verdict
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(ctor.as_deref(), Some("Holds"));

        // Negative arg → Fails.
        let expr_neg = Expression::FunctionCall {
            name: Q_POSITIVE_IRI.to_string(),
            args: vec![Expression::Literal(Literal::Integer(-5))],
        };
        let v = eval_expression(&expr_neg, &binding, &layer, runtime).expect("eval");
        let verdict = match v {
            Value::Embedded(r) => r,
            other => panic!("expected embedded Verdict, got {other:?}"),
        };
        let ctor = verdict
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(ctor.as_deref(), Some("Fails"));
    }

    #[test]
    fn unknown_iri_falls_through_to_builtin_error() {
        // An IRI that doesn't resolve to a Decidable QueryClass falls
        // through to `functions::call_function`, which errors with
        // "no such function."
        let (layer, storage, index) = q_index();
        let inst_runtime = q_runtime();
        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);

        let runtime = FiberRuntime {
            index: Some(&index),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
            retrieval: None,
            embedders: None,
            embedding_cache: None,
            vector_segment_cache: None,
        };

        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "urn:eigenius:test:unknown_fn".to_string(),
            args: vec![],
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown") || msg.contains("function"));
    }

    /// Phase 19d.7 follow-on: when an EigenQL Decidable predicate
    /// receives an IRI-shaped arg targeting a typed `core:resource`
    /// property, the kernel dereferences the IRI to the embedded
    /// chain resource before serialising for the institution. This
    /// is the same plumbing fix that landed for FIBER param values
    /// in `embed_typed_resource_param` — both surfaces now share
    /// `institution::marshal::embed_typed_resource_arg`.
    #[test]
    fn decide_dereferences_iri_args_for_typed_resource_props() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static OBSERVED_EMBEDDED: AtomicBool = AtomicBool::new(false);

        const DEREF_INST_IRI: &str = "urn:eigenius:test:deref_inst";
        const DEREF_QC_IRI: &str = "urn:eigenius:test:deref_qc";
        const DEREF_INPUT_CLASS_IRI: &str = "urn:eigenius:test:DerefInput";
        const DEREF_TARGET_PROP_IRI: &str = "urn:eigenius:test:DerefInput:target";
        const DEREF_TARGET_INSTANCE_IRI: &str = "urn:eigenius:test:deref_target";

        struct DerefInst;
        impl Institution for DerefInst {
            fn institution_iri(&self) -> &Iri {
                static INST: std::sync::OnceLock<Iri> = std::sync::OnceLock::new();
                INST.get_or_init(|| Iri::parse(DEREF_INST_IRI).unwrap())
            }
            fn extract_typed(
                &self,
                _: &Iri,
                _: &Resource,
                _: &ExecutionContext,
            ) -> Result<Val, InstitutionError> {
                unreachable!()
            }
            fn reify(
                &self,
                _: &Iri,
                _: &Val,
                _: &ExecutionContext,
            ) -> Result<Resource, InstitutionError> {
                unreachable!()
            }
            fn query(
                &self,
                _: &Iri,
                input: &Resource,
                _: &ExecutionContext,
            ) -> Result<crate::institution::runtime::QueryOutcome, InstitutionError> {
                // The target property must be Embedded (the kernel
                // dereferenced the IRI), NOT String — that's the
                // entire point.
                let target = input.get(&Iri::parse(DEREF_TARGET_PROP_IRI).unwrap());
                if matches!(target, Some(Value::Embedded(_))) {
                    OBSERVED_EMBEDDED.store(true, Ordering::SeqCst);
                }
                let mut r = Resource::new_embedded();
                r.set(
                    Iri::parse(wk::IS_A).unwrap(),
                    Value::Array(vec![Value::String(wk::VERDICT.into())]),
                );
                r.set(
                    Iri::parse(wk::CTOR_NAME).unwrap(),
                    Value::String("Holds".into()),
                );
                Ok(crate::institution::runtime::QueryOutcome::from_output(r))
            }
        }

        // Layer carries:
        //   - a target instance the IRI arg references
        //   - the typed `target: core:resource` property
        //   - the input class with `requires: [target]`
        //   - the Decidable QueryClass + Institution
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_b = LayerBuilder::new("core", None);
        for r in core_resources {
            core_b.add_resource(r).unwrap();
        }
        let core_layer = Arc::new(core_b.build(storage.clone()));

        let mut b = LayerBuilder::new("deref_test", Some(Arc::clone(&core_layer)));

        // Target instance (some chain-committed resource).
        let mut target = Resource::new(Iri::parse(DEREF_TARGET_INSTANCE_IRI).unwrap());
        target.set(
            Iri::parse(wk::SHORT_NAME).unwrap(),
            Value::String("the_target".into()),
        );
        b.add_resource(target).unwrap();

        // Property declaration with `data_type: core:resource`.
        let mut prop = Resource::new(Iri::parse(DEREF_TARGET_PROP_IRI).unwrap());
        prop.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::PROPERTY.into())]),
        );
        prop.set(
            Iri::parse(wk::DATA_TYPE_PROP).unwrap(),
            Value::String(wk::RESOURCE.into()),
        );
        b.add_resource(prop).unwrap();

        // Input class.
        let mut input_class = Resource::new(Iri::parse(DEREF_INPUT_CLASS_IRI).unwrap());
        input_class.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::CLASS.into())]),
        );
        input_class.set(
            Iri::parse(wk::REQUIRES).unwrap(),
            Value::Array(vec![Value::String(DEREF_TARGET_PROP_IRI.into())]),
        );
        b.add_resource(input_class).unwrap();

        // Institution.
        let mut inst = Resource::new(Iri::parse(DEREF_INST_IRI).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:institution:Institution".into(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(DEREF_INST_IRI.into()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("DerefInst".into()),
        );
        b.add_resource(inst).unwrap();

        // QueryClass.
        let mut qc = Resource::new(Iri::parse(DEREF_QC_IRI).unwrap());
        qc.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::QUERY_CLASS_CLASS.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_CLASS).unwrap(),
            Value::String(DEREF_INPUT_CLASS_IRI.into()),
        );
        qc.set(
            Iri::parse(wk::RESULT_CLASS).unwrap(),
            Value::String(wk::VERDICT.into()),
        );
        qc.set(
            Iri::parse(wk::DISPATCH_ROLE).unwrap(),
            Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.into())]),
        );
        qc.set(
            Iri::parse(wk::QUERY_HANDLER).unwrap(),
            Value::String("urn:eigenius:test:deref:handler".into()),
        );
        qc.set(
            Iri::parse("urn:eigenius:institution:institution_ref").unwrap(),
            Value::String(DEREF_INST_IRI.into()),
        );
        b.add_resource(qc).unwrap();

        let layer = Arc::new(b.build(storage.clone()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(errors.is_empty(), "{errors:?}");
        let mut rt = InstitutionRuntime::new();
        rt.register(Box::new(DerefInst)).unwrap();

        let exec_ctx = q_exec_ctx(Arc::clone(&layer), storage);
        let inst_runtime = Arc::new(rt);
        let runtime = FiberRuntime {
            index: Some(&idx),
            runtime: Some(&inst_runtime),
            components: None,
            overlay: None,
            ctx: Some(&exec_ctx),
            retrieval: None,
            embedders: None,
            embedding_cache: None,
            vector_segment_cache: None,
        };

        // Pass the IRI as a String literal — same shape MATCH
        // bindings produce when binding `?var` to a chain resource
        // subject. The kernel must dereference it before the
        // institution sees the input.
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: DEREF_QC_IRI.to_string(),
            args: vec![Expression::Literal(Literal::String(
                DEREF_TARGET_INSTANCE_IRI.into(),
            ))],
        };
        let _ = eval_expression(&expr, &binding, &layer, runtime).expect("eval");
        assert!(
            OBSERVED_EMBEDDED.load(Ordering::SeqCst),
            "institution must have observed the typed property as Embedded — \
             the kernel's IRI-dereference pass should have unwrapped the IRI"
        );
    }

    // ─── D2 v2 §3.7 / §3.8 — postfix Verdict predicate ─────────────────

    #[test]
    fn parser_accepts_postfix_verdict_predicates() {
        // The grammar verdict_term ::= primary_expr (verdict_predicate)?
        // sits between unary and primary. All three postfix tokens must
        // parse, AND combinations across postfix-projected operands must
        // still parse.
        let source = r#"
            MATCH ?x {}
            WHERE cap:q_positive(42) HOLDS
              AND cap:other(?x) FAILS
              AND cap:third(?x) UNDECIDABLE
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let _program = parser::parse(tokens).expect("parse postfix predicates");
    }

    #[test]
    fn parser_postfix_binds_tighter_than_not() {
        // `NOT qc:check(?x) HOLDS` should parse as `NOT (qc:check(?x) HOLDS)`,
        // not `(NOT qc:check(?x)) HOLDS`. Verify by inspecting the AST shape.
        use crate::query::ast::{Expression, UnaryOp, VerdictPredicate};
        let source = r#"
            MATCH ?x {}
            WHERE NOT cap:q_positive(?x) HOLDS
            RETURN [] { ok: ?x }
        "#;
        let tokens = tokenize(source).unwrap();
        let program = parser::parse(tokens).expect("parse NOT-postfix");
        let cond = program
            .query
            .body
            .conditions
            .first()
            .expect("WHERE condition");
        match cond {
            Expression::Unary { op, operand } => {
                assert_eq!(*op, UnaryOp::Not);
                match operand.as_ref() {
                    Expression::VerdictPredicate { kind, .. } => {
                        assert_eq!(*kind, VerdictPredicate::Holds);
                    }
                    other => panic!("expected `NOT (qc HOLDS)`, got NOT followed by {other:?}"),
                }
            }
            other => panic!("expected `NOT …`, got {other:?}"),
        }
    }

    #[test]
    fn postfix_holds_projects_verdict_to_boolean() {
        // Build a Verdict resource with ctor_name = "Holds" and project
        // it through each of the three postfix predicates.
        use crate::query::ast::{Expression, VerdictPredicate};
        let mut verdict = Resource::new_embedded();
        verdict.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
        );
        verdict.set(
            Iri::parse(wk::CTOR_NAME).unwrap(),
            Value::String("Holds".into()),
        );
        let layer = Arc::new(
            LayerBuilder::new("postfix-test", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let runtime = FiberRuntime::default();
        let mut binding: BTreeMap<String, Value> = BTreeMap::new();
        binding.insert("v".into(), Value::Embedded(Box::new(verdict)));

        let var_v = Expression::Variable(crate::query::ast::Variable { name: "v".into() });
        let project = |kind: VerdictPredicate| -> Value {
            eval_expression(
                &Expression::VerdictPredicate {
                    kind,
                    operand: Box::new(var_v.clone()),
                },
                &binding,
                &layer,
                runtime,
            )
            .expect("eval verdict predicate")
        };
        assert_eq!(project(VerdictPredicate::Holds), Value::Boolean(true));
        assert_eq!(project(VerdictPredicate::Fails), Value::Boolean(false));
        assert_eq!(
            project(VerdictPredicate::Undecidable),
            Value::Boolean(false)
        );
    }

    #[test]
    fn postfix_predicate_rejects_non_verdict_operand() {
        // A non-Verdict operand (e.g. an Integer) should error with a
        // type-mismatch evaluation error rather than silently returning
        // false. Type-checker enforcement of this rule lands as part of
        // §5.9 rule coverage; the runtime guard is the floor.
        use crate::query::ast::{Expression, Literal, VerdictPredicate};
        let layer = Arc::new(
            LayerBuilder::new("postfix-test", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let runtime = FiberRuntime::default();
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::VerdictPredicate {
            kind: VerdictPredicate::Holds,
            operand: Box::new(Expression::Literal(Literal::Integer(42))),
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Verdict-typed operand"),
            "unexpected message: {msg}"
        );
    }

    // ─── D43 §3.5 — EMBED evaluator tests ─────────────────────────────

    /// Trivial layer used by EMBED tests — `eval_embed` doesn't
    /// actually consult the layer, but `eval_expression` needs one
    /// for its signature.
    fn empty_layer() -> Arc<crate::layer::Layer> {
        Arc::new(
            LayerBuilder::new("embed-test", None).build(crate::layer::LayerStorage::in_memory()),
        )
    }

    #[test]
    fn embed_dispatches_through_registry_and_returns_vector() {
        use crate::program::embedder::registry_with_dummy;
        let reg = registry_with_dummy();
        let layer = empty_layer();
        let runtime = FiberRuntime {
            embedders: Some(&reg),
            ..FiberRuntime::default()
        };
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "EMBED".into(),
            args: vec![
                Expression::Literal(Literal::String("hello world".into())),
                Expression::Literal(Literal::String("urn:eigenius:embed:dummy:v1".into())),
            ],
        };
        let v = eval_expression(&expr, &binding, &layer, runtime).expect("EMBED dispatch");
        let (model, data) = v.as_vector().expect("EMBED should produce Value::Vector");
        assert_eq!(model.as_str(), "urn:eigenius:embed:dummy:v1");
        assert_eq!(data.len(), 8);
    }

    #[test]
    fn embed_is_deterministic_via_dummy() {
        use crate::program::embedder::registry_with_dummy;
        let reg = registry_with_dummy();
        let layer = empty_layer();
        let runtime = FiberRuntime {
            embedders: Some(&reg),
            ..FiberRuntime::default()
        };
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "EMBED".into(),
            args: vec![
                Expression::Literal(Literal::String("identical input".into())),
                Expression::Literal(Literal::String("urn:eigenius:embed:dummy:v1".into())),
            ],
        };
        let a = eval_expression(&expr, &binding, &layer, runtime).expect("first EMBED");
        let b = eval_expression(&expr, &binding, &layer, runtime).expect("second EMBED");
        // The dummy embedder is deterministic, so two calls with the
        // same input produce the same vector. Real embedders are
        // NonDeterministic (D43 §5.2) and would not satisfy this.
        assert_eq!(a, b);
    }

    #[test]
    fn embed_rejects_unknown_model_iri() {
        use crate::program::embedder::registry_with_dummy;
        let reg = registry_with_dummy();
        let layer = empty_layer();
        let runtime = FiberRuntime {
            embedders: Some(&reg),
            ..FiberRuntime::default()
        };
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "EMBED".into(),
            args: vec![
                Expression::Literal(Literal::String("hello".into())),
                Expression::Literal(Literal::String("urn:eigenius:embed:missing".into())),
            ],
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        assert!(
            format!("{err}").contains("not registered"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn embed_without_registry_errors_clearly() {
        let layer = empty_layer();
        let runtime = FiberRuntime::default(); // embedders: None
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "EMBED".into(),
            args: vec![
                Expression::Literal(Literal::String("hello".into())),
                Expression::Literal(Literal::String("urn:eigenius:embed:dummy:v1".into())),
            ],
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        assert!(
            format!("{err}").contains("EmbedderRegistry"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn embed_wrong_arity_errors() {
        use crate::program::embedder::registry_with_dummy;
        let reg = registry_with_dummy();
        let layer = empty_layer();
        let runtime = FiberRuntime {
            embedders: Some(&reg),
            ..FiberRuntime::default()
        };
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "EMBED".into(),
            args: vec![Expression::Literal(Literal::String("hello".into()))],
        };
        let err = eval_expression(&expr, &binding, &layer, runtime).unwrap_err();
        assert!(
            format!("{err}").contains("EMBED expects 2 arguments"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn embed_caches_repeat_calls() {
        // An Embedder that counts how many times `embed()` runs.
        // Two identical EMBED calls through a runtime with a cache
        // should hit dispatch exactly once.
        use crate::program::embedder::{Embedder, EmbedderError, EmbedderRegistry};
        use crate::program::embedding_cache::EmbeddingCache;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingEmbedder {
            iri: Iri,
            dim: u32,
            calls: AtomicUsize,
        }
        impl Embedder for CountingEmbedder {
            fn model_iri(&self) -> &Iri {
                &self.iri
            }
            fn dim(&self) -> u32 {
                self.dim
            }
            fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedderError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![1.0, 2.0, 3.0, 4.0])
            }
        }

        let counter = Arc::new(CountingEmbedder {
            iri: Iri::parse("urn:eigenius:embed:counting").unwrap(),
            dim: 4,
            calls: AtomicUsize::new(0),
        });
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::clone(&counter) as Arc<dyn Embedder>);
        let cache = EmbeddingCache::new(16);
        let layer = empty_layer();
        let runtime = FiberRuntime {
            embedders: Some(&reg),
            embedding_cache: Some(&cache),
            ..FiberRuntime::default()
        };
        let binding: BTreeMap<String, Value> = BTreeMap::new();
        let expr = Expression::FunctionCall {
            name: "EMBED".into(),
            args: vec![
                Expression::Literal(Literal::String("same input".into())),
                Expression::Literal(Literal::String("urn:eigenius:embed:counting".into())),
            ],
        };
        let a = eval_expression(&expr, &binding, &layer, runtime).expect("first EMBED");
        let b = eval_expression(&expr, &binding, &layer, runtime).expect("second EMBED");
        assert_eq!(a, b, "cached EMBED must return the same vector");
        assert_eq!(
            counter.calls.load(Ordering::SeqCst),
            1,
            "second EMBED should be served from cache without dispatching"
        );
    }

    #[test]
    fn embed_cache_keys_by_text_and_model() {
        // Two distinct inputs (or two distinct models) must each
        // pay one dispatch — verifies the cache key is the union
        // of (content, model_iri), not either alone.
        use crate::program::embedder::{Embedder, EmbedderError, EmbedderRegistry};
        use crate::program::embedding_cache::EmbeddingCache;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingEmbedder {
            iri: Iri,
            dim: u32,
            calls: Arc<AtomicUsize>,
        }
        impl Embedder for CountingEmbedder {
            fn model_iri(&self) -> &Iri {
                &self.iri
            }
            fn dim(&self) -> u32 {
                self.dim
            }
            fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedderError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![0.0; self.dim as usize])
            }
        }

        let shared_counter = Arc::new(AtomicUsize::new(0));
        let mut reg = EmbedderRegistry::new();
        reg.register(Arc::new(CountingEmbedder {
            iri: Iri::parse("urn:eigenius:embed:m1").unwrap(),
            dim: 2,
            calls: Arc::clone(&shared_counter),
        }));
        reg.register(Arc::new(CountingEmbedder {
            iri: Iri::parse("urn:eigenius:embed:m2").unwrap(),
            dim: 2,
            calls: Arc::clone(&shared_counter),
        }));
        let cache = EmbeddingCache::new(16);
        let layer = empty_layer();
        let runtime = FiberRuntime {
            embedders: Some(&reg),
            embedding_cache: Some(&cache),
            ..FiberRuntime::default()
        };
        let binding: BTreeMap<String, Value> = BTreeMap::new();

        // Three distinct cache keys → three dispatches.
        let call = |text: &str, model: &str| {
            let expr = Expression::FunctionCall {
                name: "EMBED".into(),
                args: vec![
                    Expression::Literal(Literal::String(text.into())),
                    Expression::Literal(Literal::String(model.into())),
                ],
            };
            eval_expression(&expr, &binding, &layer, runtime).expect("EMBED")
        };
        call("alpha", "urn:eigenius:embed:m1");
        call("alpha", "urn:eigenius:embed:m2"); // different model
        call("beta", "urn:eigenius:embed:m1"); // different content
        assert_eq!(shared_counter.load(Ordering::SeqCst), 3);
        // Repeats hit the cache.
        call("alpha", "urn:eigenius:embed:m1");
        call("beta", "urn:eigenius:embed:m1");
        assert_eq!(shared_counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn embed_parses_through_full_pipeline() {
        // The parser routes `EMBED(...)` to a `FunctionCall` with
        // name "EMBED". Verify the AST shape end-to-end so a future
        // refactor doesn't silently drop the routing.
        use crate::query::lexer::tokenize;
        use crate::query::parser;
        let tokens = tokenize(
            r#"
            MATCH ?x {}
            RETURN [] { v: EMBED("hi", "urn:eigenius:embed:dummy:v1") }
            "#,
        )
        .expect("tokenize");
        let program = parser::parse(tokens).expect("parse");
        let ret_expr = &program.query.result[0].expression;
        match ret_expr {
            Expression::FunctionCall { name, args } => {
                assert_eq!(name, "EMBED");
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected FunctionCall, got {other:?}"),
        }
    }
}
