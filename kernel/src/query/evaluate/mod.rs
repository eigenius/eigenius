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
//! (`server::mod`, the `d14_dock_assay_demo` integration test).

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

    // 1. Evaluate DEFINE rules with seminaive fixpoint
    if !program.definitions.is_empty() {
        // Initial pass: evaluate all rules from base facts
        for def in &program.definitions {
            let bindings = evaluate_match_part(&def.body, layer, &derived)?;
            let entry = derived.entry(def.name.clone()).or_default();
            entry.extend(bindings);
        }

        // Fixpoint loop: keep evaluating until no new facts are derived
        let max_iterations = 1000; // Safety bound
        for _ in 0..max_iterations {
            let mut new_facts = false;
            for def in &program.definitions {
                let bindings = evaluate_match_part(&def.body, layer, &derived)?;
                let entry = derived.entry(def.name.clone()).or_default();
                let prev_len = entry.len();
                // Add only truly new bindings (not already present)
                for binding in bindings {
                    if !entry.contains(&binding) {
                        entry.push(binding);
                        new_facts = true;
                    }
                }
                if entry.len() > prev_len {
                    new_facts = true;
                }
            }
            if !new_facts {
                break; // Fixpoint reached
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
