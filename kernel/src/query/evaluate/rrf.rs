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

//! D43 §6.4 / M7.2 — RRF rank-materialisation pre-pass.
//!
//! `RRF` is the only D43 retrieval primitive that is *not* row-local
//! — its value for a row depends on the row's rank in each source's
//! full ordering. The evaluator therefore can't compute RRF inside
//! the row-by-row [`super::return_shape::shape_result`] loop without
//! first walking every row to materialise the per-source ranks.
//!
//! This module is that pre-pass. It walks the typed AST for every
//! [`Expression::Rrf`] node reachable from RETURN or TOP K BY,
//! computes each source expression's score for every binding,
//! materialises ranks via [`crate::query::rank::assign_ranks_desc`],
//! and stores the result keyed by the RRF AST node's address. Row-by-
//! row evaluation then looks up the per-source rank for the current
//! binding by `(rrf_node_ptr, source_idx, binding_idx)` and computes
//! `sum_i 1 / (k + rank_i)` per the §3.6 formula.
//!
//! The "binding_idx" key is the index into the bindings vector that
//! enters RETURN shaping. The evaluator's row-by-row loop threads
//! the index through [`super::FiberRuntime::current_binding_idx`].
//!
//! The "rrf_node_ptr" key is `*const Expression` — sound because the
//! borrowed `&Program` lives for the full evaluation; no AST node
//! is reallocated while RrfContext is alive.

use std::collections::HashMap;

use crate::layer::Layer;
use crate::ontology::resource::Value;
use crate::query::ast::{Expression, Program};
use crate::query::error::QueryError;
use crate::query::evaluate::expression::eval_expression;
use crate::query::evaluate::pattern::Binding;
use crate::query::evaluate::FiberRuntime;
use crate::query::rank::{assign_ranks_desc, rrf_score};

/// Materialised ranks for one RRF call. For each source `i`, holds a
/// map from binding index to that binding's 1-indexed rank under
/// source `i`'s ordering. A binding absent from the map has rank ∞
/// (per D43 §6.4: "missing-source rank = infinity (contributes 0)").
#[derive(Debug, Default)]
pub struct RrfMaterialised {
    /// `per_source[i][binding_idx] = rank`. The outer vec is indexed
    /// by source position in the `Expression::Rrf::sources` list.
    /// `BTreeMap` (not HashMap) because rank materialisation is
    /// deterministic per-run, and BTreeMap gives O(log n) lookups
    /// without the HashMap iteration-order non-determinism — useful
    /// when tracing reproduction.
    per_source: Vec<std::collections::BTreeMap<usize, usize>>,
    /// RRF constant `k` (defaults to 60 from the AST).
    k: u32,
}

impl RrfMaterialised {
    /// Look up the per-source ranks for a single binding index and
    /// compute the §3.6 fused score. Missing-source contributions
    /// are 0 per the spec.
    pub fn score_for(&self, binding_idx: usize) -> f64 {
        let ranks: Vec<Option<usize>> = self
            .per_source
            .iter()
            .map(|src| src.get(&binding_idx).copied())
            .collect();
        rrf_score(&ranks, self.k)
    }
}

/// Lookup table keyed by RRF AST-node pointer. Built by
/// [`prepare_rrf_context`] before row-by-row RETURN shaping.
#[derive(Debug, Default)]
pub struct RrfContext {
    by_addr: HashMap<usize, RrfMaterialised>,
}

impl RrfContext {
    /// Look up an `Expression::Rrf` node's materialised ranks by its
    /// AST address. Returns `None` only if a planner bug skipped the
    /// pre-pass for this node (which would mean the eval path saw a
    /// new RRF node that wasn't in the program's RETURN / TOP K BY
    /// expression set — a structural error worth crashing on, but
    /// returning `None` lets the caller surface a typed evaluation
    /// error instead).
    pub fn get(&self, rrf_expr: &Expression) -> Option<&RrfMaterialised> {
        self.by_addr.get(&Self::addr_of(rrf_expr))
    }

    fn addr_of(e: &Expression) -> usize {
        e as *const Expression as usize
    }

    fn insert(&mut self, rrf_expr: &Expression, materialised: RrfMaterialised) {
        self.by_addr.insert(Self::addr_of(rrf_expr), materialised);
    }

    pub fn is_empty(&self) -> bool {
        self.by_addr.is_empty()
    }
}

/// Walk the program's RETURN and TOP K BY expressions for every
/// [`Expression::Rrf`] node and build the corresponding
/// [`RrfMaterialised`] entry.
///
/// Side-effect-free with respect to the bindings (the per-source
/// score evaluation is read-only; the FiberRuntime is the same one
/// passed in, modulo `current_binding_idx` being set per evaluation).
///
/// Errors propagate from a source expression's evaluation — typically
/// only TEXT_SCORE / VECTOR_SIM dispatching against an out-of-scope
/// retrieval context, which fails the same way at row-by-row time.
pub fn prepare_rrf_context(
    program: &Program,
    bindings: &[Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<RrfContext, QueryError> {
    let mut ctx = RrfContext::default();

    // Walk RETURN expressions.
    for item in &program.query.result {
        collect_and_materialise(&item.expression, bindings, layer, runtime, &mut ctx)?;
    }

    // Walk TOP K BY's score expression.
    if let Some(top_k) = &program.query.top_k_by {
        collect_and_materialise(&top_k.expression, bindings, layer, runtime, &mut ctx)?;
    }

    Ok(ctx)
}

/// Recursively walk an expression tree; when an
/// [`Expression::Rrf`] is hit, compute and store its
/// [`RrfMaterialised`] entry. RRF inside RRF is technically valid
/// (an inner RRF returns a Float, and the outer RRF treats it as a
/// score expression) — handled by walking the source list recursively.
fn collect_and_materialise(
    expr: &Expression,
    bindings: &[Binding],
    layer: &Layer,
    runtime: FiberRuntime<'_>,
    ctx: &mut RrfContext,
) -> Result<(), QueryError> {
    match expr {
        Expression::Rrf { sources, k } => {
            // Recurse into sources first so inner RRF (if any) is
            // materialised before the outer call needs its score.
            for src in sources {
                collect_and_materialise(src, bindings, layer, runtime, ctx)?;
            }
            let mut materialised = RrfMaterialised {
                per_source: Vec::with_capacity(sources.len()),
                k: *k,
            };
            for src in sources {
                let mut scored: Vec<(usize, f64)> = Vec::with_capacity(bindings.len());
                for (i, binding) in bindings.iter().enumerate() {
                    let v = eval_expression(src, binding, layer, runtime)?;
                    let s = value_to_score(&v);
                    scored.push((i, s));
                }
                // Per D43 §6.4: only score-bearing rows participate
                // in the source's ranking. We retain every binding
                // (the over-fetch policy of M7.4 will trim the
                // candidate set at the index level later), and
                // treat NaN as the lowest possible value via
                // `assign_ranks_desc`.
                let rank_map = assign_ranks_desc(&scored);
                materialised.per_source.push(rank_map);
            }
            ctx.insert(expr, materialised);
            Ok(())
        }
        Expression::Binary { left, right, .. } => {
            collect_and_materialise(left, bindings, layer, runtime, ctx)?;
            collect_and_materialise(right, bindings, layer, runtime, ctx)
        }
        Expression::Unary { operand, .. } => {
            collect_and_materialise(operand, bindings, layer, runtime, ctx)
        }
        Expression::VerdictPredicate { operand, .. } => {
            collect_and_materialise(operand, bindings, layer, runtime, ctx)
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                collect_and_materialise(arg, bindings, layer, runtime, ctx)?;
            }
            Ok(())
        }
        Expression::Aggregate { arg, .. } => {
            collect_and_materialise(arg, bindings, layer, runtime, ctx)
        }
        Expression::Array(items) => {
            for it in items {
                collect_and_materialise(it, bindings, layer, runtime, ctx)?;
            }
            Ok(())
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                collect_and_materialise(v, bindings, layer, runtime, ctx)?;
            }
            Ok(())
        }
        Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::NotExists(_)
        | Expression::DotPath { .. } => Ok(()),
    }
}

/// Coerce a [`Value`] produced by a source expression to a comparable
/// score. Float and Integer are the canonical cases (TEXT_SCORE
/// returns Float, VECTOR_SIM returns Float, arithmetic combinations
/// preserve numeric types). Anything else maps to `NaN` so it sorts
/// last under [`assign_ranks_desc`].
fn value_to_score(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        Value::Integer(i) => *i as f64,
        Value::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// `RrfMaterialised::score_for` returns the right fused score
    /// given a binding's per-source ranks. Sanity check against the
    /// §3.6 formula.
    #[test]
    fn score_for_combines_per_source_ranks() {
        let mut src0 = BTreeMap::new();
        let mut src1 = BTreeMap::new();
        // Binding 0 ranks 1st in source A and 2nd in source B.
        src0.insert(0, 1);
        src1.insert(0, 2);
        // Binding 1 only ranks in source A.
        src0.insert(1, 2);
        let mat = RrfMaterialised {
            per_source: vec![src0, src1],
            k: 60,
        };
        let s0 = mat.score_for(0);
        let s1 = mat.score_for(1);
        // Binding 0: 1/(60+1) + 1/(60+2)
        let expected_0 = 1.0 / 61.0 + 1.0 / 62.0;
        // Binding 1: 1/(60+2) (only source A contributes)
        let expected_1 = 1.0 / 62.0;
        assert!((s0 - expected_0).abs() < 1e-12, "got {s0}");
        assert!((s1 - expected_1).abs() < 1e-12, "got {s1}");
    }

    /// `RrfContext::get` returns the entry inserted under the same
    /// `Expression` reference and `None` for a node with a different
    /// AST address.
    #[test]
    fn rrf_context_pointer_keyed_lookup() {
        let expr_a = Expression::Rrf {
            sources: vec![],
            k: 60,
        };
        let expr_b = Expression::Rrf {
            sources: vec![],
            k: 30,
        };
        let mut ctx = RrfContext::default();
        ctx.insert(
            &expr_a,
            RrfMaterialised {
                per_source: vec![BTreeMap::new()],
                k: 60,
            },
        );
        assert!(ctx.get(&expr_a).is_some());
        // `expr_b` lives at a different stack address so the lookup
        // misses, which is exactly the discrimination we need.
        assert!(ctx.get(&expr_b).is_none());
    }

    // Touch `Binding` and `BTreeMap` so the import sees use even
    // when integration tests live elsewhere — keeps the module
    // self-checking without dragging in the full evaluator setup.
    #[test]
    fn binding_type_is_btreemap() {
        let _b: Binding = BTreeMap::new();
    }
}
