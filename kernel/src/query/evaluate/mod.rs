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

//! EigenQL evaluator: pattern matching, fixpoint, aggregation, result shaping.
//!
//! The evaluator is split by phase:
//!
//! - [`pattern`] — positive / negated pattern matching and candidate
//!   collection (plus the shared [`Binding`] alias and the small
//!   `literal_to_value` / `find_property_by_shortname` helpers).
//! - [`expression`] — `eval_expression`, binary / unary / verdict
//!   evaluators, GROUP BY + aggregation, Decidable QueryClass dispatch.
//! - [`fiber`] — FIBER clause dispatch, [`FiberRuntime`] surface,
//!   comorphism coercion, transient overlay management,
//!   [`evaluate_match_part`] and [`evaluate_match_part_with_fiber`].
//! - [`return_shape`] — RETURN projection, DISTINCT, ORDER BY.
//!
//! This module exposes only the public surface used by
//! [`crate::query::execute_with_into`] and external callers
//! (`server::mod`, the `dock_assay_demo` integration test).

mod expression;
mod fiber;
mod pattern;
mod return_shape;
mod similarity;

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use crate::query::ast::Program;
use crate::query::document::QueryFingerprint;
use crate::query::error::QueryError;
use std::collections::BTreeMap;

pub use fiber::{eval_comorphism_coercion, FiberRuntime};

use expression::{apply_group_by, has_aggregates};
use fiber::{evaluate_match_part, evaluate_match_part_with_fiber, FiberOverlay};
use pattern::Binding;
use return_shape::{binding_to_resource, deduplicate, shape_result, sort_results};

/// Evaluate a parsed and validated EigenQL program against a layer.
///
/// Row Property IRIs for RETURN items are synthesized using `fp`, so that
/// the downstream `document::wrap` step produces Property/Class metadata
/// resources that line up with the row keys.
///
/// FIBER clauses require `runtime` to carry both an institution registry
/// and an execution context; otherwise they error at dispatch time.
/// Project DEFINE-body bindings onto the rule's head variables, re-keyed by
/// positional index ("0", "1", …). A relation defined by multiple rules with
/// differently-named head variables thus yields one consistent positional tuple
/// shape, and rule-local (non-head) variables are dropped. A row that fails to
/// bind every head variable is dropped (an under-bound head is not a valid fact).
fn project_onto_head(bindings: Vec<Binding>, head: &[crate::query::ast::Variable]) -> Vec<Binding> {
    bindings
        .into_iter()
        .filter_map(|b| {
            let mut row = Binding::new();
            for (i, var) in head.iter().enumerate() {
                let val = b.get(&var.name)?;
                row.insert(i.to_string(), val.clone());
            }
            Some(row)
        })
        .collect()
}

pub fn evaluate(
    program: &Program,
    layer: &Layer,
    fp: &QueryFingerprint,
    runtime: FiberRuntime<'_>,
) -> Result<(Vec<Resource>, Vec<Resource>), QueryError> {
    // D43 §6 — similarity-operator pre-pass: probe every active
    // similarity index referenced by a `~` operator in the program,
    // fuse the per-source rankings into a subject → score map, and
    // hand the resulting context to per-row evaluation. The I/O
    // cost is paid once per query, not once per row.
    let similarity_ctx = similarity::SimilarityContext::new(
        program,
        layer,
        runtime.embedders,
        runtime.vector_segment_cache,
    )?;
    let runtime = FiberRuntime {
        similarity: Some(&similarity_ctx),
        ..runtime
    };

    let mut derived: BTreeMap<String, Vec<Binding>> = BTreeMap::new();

    // 1. Evaluate DEFINE rules, stratum by stratum, with a seminaive fixpoint
    //    per stratum.
    //
    // Strata MUST be evaluated in order: a relation that negates another
    // (`NOT Reach(?x)`) sits in a strictly higher stratum, and its negated
    // dependency must be *fully computed* before it runs. Evaluating all rules
    // together in one fixpoint is unsound for negation — the negating relation
    // would see a partial lower relation early and, since the loop only adds
    // facts, those stale rows would never be retracted. (Stratification is
    // validated upstream in `query::mod`; here we use the same ordering.)
    //
    // Each rule's body bindings are *projected onto the rule's head variables*,
    // re-keyed by positional index ("0", "1", …), before being stored. This is
    // load-bearing: a relation defined by several rules may use differently-named
    // head variables (e.g. `Reach(?t)` in one rule and `Reach(?n)` in another),
    // so storing raw body bindings would yield inconsistent keys that the
    // consumer (collect_candidates) cannot map. Positional projection gives one
    // canonical tuple shape per relation and drops rule-local junk variables.
    if !program.definitions.is_empty() {
        let strata = crate::query::stratify::stratify(&program.definitions)?;
        let max_iterations = 1000; // Safety bound
        for stratum in &strata {
            let in_stratum: std::collections::BTreeSet<&str> =
                stratum.relations.iter().map(String::as_str).collect();
            let rules: Vec<&crate::query::ast::RuleDefinition> = program
                .definitions
                .iter()
                .filter(|d| in_stratum.contains(d.name.as_str()))
                .collect();

            // Seminaive fixpoint over this stratum only; lower strata are fixed.
            // The first iteration is the initial pass; stop when a full pass
            // adds no new facts.
            for _ in 0..=max_iterations {
                let mut new_facts = false;
                for def in &rules {
                    let bindings = evaluate_match_part(&def.body, layer, &derived)?;
                    let projected = project_onto_head(bindings, &def.variables);
                    let entry = derived.entry(def.name.clone()).or_default();
                    for binding in projected {
                        if !entry.contains(&binding) {
                            entry.push(binding);
                            new_facts = true;
                        }
                    }
                }
                if !new_facts {
                    break;
                }
            }
        }
    }

    // 2. Evaluate the query.
    //
    // The transient `overlay` holds every FIBER response (so
    // subsequent patterns and the WHERE/RETURN expression evaluator
    // can decompose them by IRI). The `into_collector` holds only
    // the responses committed by `FIBER ... INTO "<iri>"` — the
    // run-boundary lifts that subset to the regular chain.
    let mut overlay = FiberOverlay::default();
    let mut into_collector: Vec<Resource> = Vec::new();
    let mut bindings = evaluate_match_part_with_fiber(
        &program.query.body,
        layer,
        &derived,
        runtime,
        fp,
        &mut overlay,
        &mut into_collector,
    )?;

    // The overlay must remain visible to GROUP BY and RETURN shaping
    // — both can read FIBER-bound `?var.prop` projections via DotPath
    // and Verdict postfix predicates via `resolve_iri_string`. Layer
    // the populated overlay onto the user-supplied runtime once and
    // thread the result through both phases.
    let runtime_with_overlay = FiberRuntime {
        overlay: Some(&overlay.entries),
        ..runtime
    };

    // 3. GROUP BY + aggregation
    if !program.query.group_by.is_empty() || has_aggregates(&program.query.result) {
        bindings = apply_group_by(
            &program.query.group_by,
            &program.query.result,
            &bindings,
            layer,
            runtime_with_overlay,
        )?;
    }

    // D43 §3.3 — `TOP N` ranks the surviving bindings by their
    // aggregate similarity score before RETURN shaping, then keeps
    // the top N. Sorting before shaping is load-bearing: shaped
    // resources project away the subject-IRI binding the score
    // lookup needs. Stable sort + descending-score key keeps the
    // ordering deterministic across runs.
    if let Some(n) = program.query.top {
        bindings.sort_by(|a, b| {
            let sa = similarity_ctx.aggregate_score(a);
            let sb = similarity_ctx.aggregate_score(b);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        bindings.truncate(n);
    }

    // 4. RETURN shaping.
    let mut results: Vec<Resource> = if program.query.result.is_empty() {
        bindings
            .iter()
            .map(|b| binding_to_resource(b, &program.query.result_classes))
            .collect()
    } else {
        let mut out = Vec::with_capacity(bindings.len());
        for binding in &bindings {
            let resource = shape_result(
                binding,
                &program.query.result_classes,
                &program.query.result,
                layer,
                fp,
                runtime_with_overlay,
            )?;
            out.push(resource);
        }
        out
    };

    // 5. DISTINCT
    if program.query.distinct {
        results = deduplicate(results);
    }

    // 6. ORDER BY
    if !program.query.order_by.is_empty() {
        sort_results(&mut results, &program.query.order_by, fp);
    }

    // 7. OFFSET
    if let Some(offset) = program.query.offset {
        if offset < results.len() {
            results = results.into_iter().skip(offset).collect();
        } else {
            results.clear();
        }
    }

    // 8. LIMIT
    if let Some(limit) = program.query.limit {
        results.truncate(limit);
    }

    Ok((results, into_collector))
}
