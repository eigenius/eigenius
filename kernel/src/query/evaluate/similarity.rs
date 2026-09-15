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

//! D43 §6 — similarity operator evaluator pre-pass.
//!
//! Walks the program AST once before pattern matching, discovers
//! every `Expression::Similarity` node, dispatches its text and/or
//! vector probes against the layer's active Index Resources, and
//! caches the fused subject → score map. Per-row evaluation then
//! becomes an O(map lookup) on the row's source subject IRI.
//!
//! The pre-pass is structured so the entire I/O cost of similarity
//! retrieval is paid once per query, not once per row — fundamental
//! when a `~` operator can produce thousands of candidates.

use crate::layer::{
    resolve_active_text_indexes, resolve_active_vector_indexes, ActiveTextIndex, ActiveVectorIndex,
    Layer, LayerId,
};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::program::embedder::EmbedderRegistry;
use crate::query::ast::{
    BinaryOp, Expression, HintSet, Literal, MatchPart, Program, Resolved, ValueOrVariable,
    Variable, Via,
};
use crate::query::error::QueryError;
use crate::query::text::analyzer::registry as analyzer_registry;
use crate::query::text::search::{run_text_search, TextScoredHit};
use crate::query::vector::cache::SegmentCache;
use crate::query::vector::distance::Metric;
use crate::query::vector::search::{top_k_subjects, VectorScoredHit};
use std::collections::BTreeMap;

/// D43 §3.5 default RRF smoothing constant.
const DEFAULT_RRF_K: usize = 60;
/// D43 §6.4 floor under the per-source candidate pool. The pool is
/// `max(TOP K * OVER_FETCH_FACTOR, DEFAULT_LIMIT)`; this is what it takes when the
/// derived bound is smaller, and what an unranked vector probe takes when nothing
/// derives a bound at all.
const DEFAULT_LIMIT: usize = 200;

/// One similarity operator's resolved probe state. Built once per
/// `Expression::Similarity` node by [`SimilarityContext::new`].
///
/// `subject_var` is the variable bound by the property pattern that
/// introduced the operator's LHS — e.g. for
/// `MATCH ?d { description: ?desc } WHERE ?desc ~ "q"` the subject
/// variable is `d`. Per-row evaluation looks up the row's `?d`
/// binding, projects it to an IRI, and probes `scores` to decide
/// whether the row participated in the similarity result.
#[derive(Debug, Clone)]
pub(super) struct SimilarityProbe {
    /// Name of the row-subject variable (e.g. `"d"` for `?d`).
    pub subject_var: String,
    /// Fused subject → score map. Only IRIs in this map count as
    /// matches; non-members produce `Boolean(false)` at eval.
    pub scores: BTreeMap<Iri, f64>,
}

/// Per-query similarity-probe cache. Threaded through
/// [`crate::query::evaluate::FiberRuntime`] so per-row eval can
/// resolve a `Similarity` node back to its precomputed score map in
/// O(1).
///
/// Pointer identity (`*const Expression<Resolved>`) keys the map. The AST is
/// owned by the [`Program<Resolved>`] for the duration of evaluation, so
/// `&Expression<Resolved>` references remain stable and the pointer is a
/// stable identifier — equivalent to a node ID without needing a
/// parallel index walk on the eval side.
#[derive(Debug, Default)]
pub struct SimilarityContext {
    probes: BTreeMap<usize, SimilarityProbe>,
}

impl SimilarityContext {
    /// Build the pre-pass. Walks the program for similarity nodes
    /// and probes each one. Errors surface to the caller, which
    /// short-circuits evaluation with a `QueryError` before any
    /// per-row work runs.
    pub fn new(
        program: &Program<Resolved>,
        layer: &Layer,
        embedders: Option<&EmbedderRegistry>,
        vector_segment_cache: Option<&SegmentCache>,
    ) -> Result<Self, QueryError> {
        let prop_var_index = build_property_variable_index(program);
        let text_indexes = resolve_active_text_indexes(layer);
        let vector_indexes = resolve_active_vector_indexes(layer);

        let mut probes: BTreeMap<usize, SimilarityProbe> = BTreeMap::new();
        let mut collected: Vec<&Expression<Resolved>> = Vec::new();
        collect_similarity_nodes(&program.query.body, &mut collected);
        for item in &program.query.result {
            collect_in_expression(&item.expression, &mut collected);
        }
        for expr in &program.query.group_by {
            collect_in_expression(expr, &mut collected);
        }
        for item in &program.query.order_by {
            collect_in_expression(&item.expression, &mut collected);
        }
        for def in &program.definitions {
            collect_similarity_nodes(&def.body, &mut collected);
        }
        for expr in collected {
            if let Expression::Similarity {
                property,
                query,
                hints,
            } = expr
            {
                let probe = build_probe(
                    property,
                    query,
                    hints,
                    program.query.top,
                    &prop_var_index,
                    &text_indexes,
                    &vector_indexes,
                    layer,
                    embedders,
                    vector_segment_cache,
                )?;
                probes.insert(expr as *const _ as usize, probe);
            }
        }
        Ok(Self { probes })
    }

    /// Resolve a `Similarity` AST node to its precomputed probe
    /// state, if any. Returns `None` for non-similarity expressions
    /// (callers gate on the AST variant).
    pub(super) fn probe_for(&self, expr: &Expression<Resolved>) -> Option<&SimilarityProbe> {
        self.probes.get(&(expr as *const _ as usize))
    }

    /// Sum the fused similarity scores from every registered probe
    /// for the binding's row. For each probe, project the binding's
    /// `subject_var` to an IRI and add the probe's score if the IRI
    /// is in its candidate set. Rows that none of the probes ranked
    /// score 0.0; rows ranked by multiple probes accumulate (the
    /// design's "rows satisfying both rank higher than rows
    /// satisfying one" — §3.3).
    /// The subjects the similarity CONJUNCTS of `conditions` admit for `subject_var`, if
    /// any constrain it.
    ///
    /// **This is the narrowing set candidate enumeration should start from.** The pre-pass
    /// already computed it, one probe per similarity node paid once per query, and per-row
    /// evaluation then answers `false` for every subject outside it. Enumerating the whole
    /// chain and filtering afterwards does that work twice, the second time over every
    /// resource in the chain.
    ///
    /// **Position is what makes it sound, and position is why this takes the conditions
    /// rather than a variable name.** A subject outside the probe's map is dropped only
    /// when the `~` is a CONJUNCT. Under a disjunction the other branch may still admit the
    /// row; under a negation the predicate rejects exactly the set the probe returned; and
    /// in `RETURN` or `ORDER BY` the operator is a score projection that filters nothing at
    /// all. The pre-pass registers a probe for every similarity node in the program
    /// including all of those, so asking it by variable NAME answers a question the name
    /// cannot answer — an earlier version did that and returned the intersection of a
    /// disjunction's branches, and nothing at all for a negation.
    ///
    /// Descending only through `And` from the top-level conditions makes the property hold
    /// by construction, and it is the same discipline `extract_subject_constraint` applies
    /// to the subject pushdown beside it.
    pub(super) fn conjunct_subjects_for(
        &self,
        subject_var: &str,
        conditions: &[Expression<Resolved>],
    ) -> Option<Vec<Iri>> {
        let mut conjuncts: Vec<&Expression<Resolved>> = Vec::new();
        for cond in conditions {
            collect_similarity_conjuncts(cond, &mut conjuncts);
        }
        let mut out: Option<Vec<Iri>> = None;
        for expr in conjuncts {
            let Some(probe) = self.probe_for(expr) else {
                continue;
            };
            if probe.subject_var != subject_var {
                continue;
            }
            let these: Vec<Iri> = probe.scores.keys().cloned().collect();
            // Conjuncts, so the admissible set is their INTERSECTION. That is true here
            // because the walk above admits only conjuncts.
            out = Some(match out {
                None => these,
                Some(prev) => {
                    let keep: std::collections::BTreeSet<&Iri> = these.iter().collect();
                    prev.into_iter().filter(|i| keep.contains(i)).collect()
                }
            });
        }
        out
    }

    pub(crate) fn aggregate_score(&self, binding: &super::pattern::Binding) -> f64 {
        let mut sum = 0.0_f64;
        for probe in self.probes.values() {
            let v = match binding.get(&probe.subject_var) {
                Some(v) => v,
                None => continue,
            };
            let iri = match v {
                Value::String(s) => match Iri::parse(s) {
                    Ok(i) => i,
                    Err(_) => continue,
                },
                _ => continue,
            };
            if let Some(score) = probe.scores.get(&iri) {
                sum += *score;
            }
        }
        sum
    }
}

/// Schema view a similarity probe needs: which property a variable
/// was bound to, plus the row-subject variable so per-row eval can
/// project the binding to an IRI. Mirrors the typecheck-side
/// [`crate::query::type_check`] index but extended with the subject
/// variable.
struct PropertyVarBinding {
    property_iri: Iri,
    subject_var: String,
}

/// The `variable → property_iri` map over every `MATCH` brace key that binds a variable.
///
/// Infallible, and takes no layer: `resolve_property_names` already resolved every key
/// against the full scope rule and reported what it could not. This reads the answer.
fn build_property_variable_index(
    program: &Program<Resolved>,
) -> BTreeMap<String, PropertyVarBinding> {
    let mut out: BTreeMap<String, PropertyVarBinding> = BTreeMap::new();
    let mut visit = |part: &MatchPart<Resolved>| {
        for pat in part.patterns() {
            for pp in &pat.properties {
                if let ValueOrVariable::Variable(var) = &pp.object {
                    if let Some(property_iri) = Some(pp.property.clone()) {
                        out.entry(var.name.clone()).or_insert(PropertyVarBinding {
                            property_iri,
                            subject_var: pat.subject.name.clone(),
                        });
                    }
                }
            }
        }
    };
    visit(&program.query.body);
    for def in &program.definitions {
        visit(&def.body);
    }
    out
}

fn collect_similarity_nodes<'a>(
    part: &'a MatchPart<Resolved>,
    out: &mut Vec<&'a Expression<Resolved>>,
) {
    for cond in &part.conditions {
        collect_in_expression(cond, out);
    }
}

/// The similarity nodes that are CONJUNCTS of `expr`.
///
/// Descends only through `And`. A `~` under `Or`, under `Not`, or under any other operator
/// does not constrain every row the expression admits, so it must not narrow the candidate
/// set. Mirrors `extract_subject_constraint`'s reading of the same condition list.
fn collect_similarity_conjuncts<'a>(
    expr: &'a Expression<Resolved>,
    out: &mut Vec<&'a Expression<Resolved>>,
) {
    match expr {
        Expression::Similarity { .. } => out.push(expr),
        Expression::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            collect_similarity_conjuncts(left, out);
            collect_similarity_conjuncts(right, out);
        }
        _ => {}
    }
}

fn collect_in_expression<'a>(
    expr: &'a Expression<Resolved>,
    out: &mut Vec<&'a Expression<Resolved>>,
) {
    match expr {
        Expression::Similarity { query, .. } => {
            out.push(expr);
            collect_in_expression(query, out);
        }
        Expression::Binary { left, right, .. } => {
            collect_in_expression(left, out);
            collect_in_expression(right, out);
        }
        Expression::Unary { operand, .. } | Expression::VerdictPredicate { operand, .. } => {
            collect_in_expression(operand, out);
        }
        Expression::FunctionCall { args, .. } => {
            for a in args {
                collect_in_expression(a, out);
            }
        }
        Expression::Aggregate { arg, .. } => collect_in_expression(arg, out),
        Expression::Array(es) => {
            for e in es {
                collect_in_expression(e, out);
            }
        }
        Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::NotExists(_)
        | Expression::DotPath { .. } => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn build_probe(
    property: &Variable,
    query: &Expression<Resolved>,
    hints: &HintSet,
    // `TOP N` from the query, which sizes the candidate pool when no explicit hint says
    // otherwise. `None` means the query does not rank, so the probe must not truncate.
    top: Option<usize>,
    prop_var_index: &BTreeMap<String, PropertyVarBinding>,
    text_indexes: &[ActiveTextIndex],
    vector_indexes: &[ActiveVectorIndex],
    layer: &Layer,
    embedders: Option<&EmbedderRegistry>,
    vector_segment_cache: Option<&SegmentCache>,
) -> Result<SimilarityProbe, QueryError> {
    // Typecheck guarantees the LHS is property-bound and the RHS
    // is a string literal; the unwraps below would only fire if
    // pre-pass were invoked on un-typechecked input.
    let binding = prop_var_index.get(&property.name).ok_or_else(|| {
        QueryError::evaluation(format!(
            "similarity LHS '?{}' has no property binding (typecheck should have caught this)",
            property.name
        ))
    })?;
    let query_string = match query {
        Expression::Literal(Literal::String(s)) => s.clone(),
        _ => {
            return Err(QueryError::evaluation(
                "similarity RHS must evaluate to a string literal in v1 (typecheck gate)",
            ));
        }
    };

    let text_active = text_indexes
        .iter()
        .find(|i| i.target_property == binding.property_iri);
    let vector_active = vector_indexes
        .iter()
        .find(|i| i.target_property == binding.property_iri);

    let ranked_pool = candidate_pool(hints.limit, top);

    // **Text, unranked, is a FILTER and must not truncate.** Matching is a predicate —
    // the document contains the terms or it does not — so a bound drops rows that match.
    // Using `DEFAULT_LIMIT` here silently returned 200 of 250 matching documents.
    let text_limit = ranked_pool.unwrap_or(usize::MAX);

    // **Vector is a top-k by construction and has no unranked form.** Every subject has a
    // distance, so "all matches" is not a set the index can produce; the search allocates
    // a heap of `k + 1` and derives its search width from `k`. `DEFAULT_LIMIT` is D43's
    // candidate-pool size and stays the floor when nothing asks for more.
    let vector_limit = ranked_pool.unwrap_or(DEFAULT_LIMIT);
    let k = hints.k.unwrap_or(DEFAULT_RRF_K);

    // §3.5 strategy selection: explicit `via:` wins; otherwise the
    // active index set determines the path. `model:` forces vector
    // even without an explicit `via:`.
    let use_text;
    let use_vector;
    match hints.via {
        Some(Via::Text) => {
            use_text = true;
            use_vector = false;
        }
        Some(Via::Vector) => {
            use_text = false;
            use_vector = true;
        }
        Some(Via::Hybrid) => {
            use_text = true;
            use_vector = true;
        }
        None => {
            if hints.model.is_some() {
                use_text = false;
                use_vector = true;
            } else {
                use_text = text_active.is_some();
                use_vector = vector_active.is_some();
            }
        }
    }

    let text_hits: Vec<TextScoredHit> = if use_text {
        let idx = text_active.ok_or_else(|| {
            QueryError::evaluation(format!(
                "no active TextIndex for property '{}' (planner gate missed)",
                binding.property_iri
            ))
        })?;
        let analyzer = analyzer_registry::analyzer_for(&idx.analyzer).ok_or_else(|| {
            QueryError::evaluation(format!(
                "analyzer '{}' for TextIndex '{}' not registered",
                idx.analyzer, idx.iri
            ))
        })?;
        let text_backend = layer.storage().text_index.clone();
        run_text_search(
            layer,
            text_backend.as_ref(),
            &idx.iri,
            analyzer.as_ref(),
            &query_string,
            text_limit,
        )
        .map_err(|e| QueryError::evaluation(format!("text probe failed: {e}")))?
    } else {
        Vec::new()
    };

    let vector_hits: Vec<VectorScoredHit> = if use_vector {
        let idx = vector_active.ok_or_else(|| {
            QueryError::evaluation(format!(
                "no active VectorIndex for property '{}' (planner gate missed)",
                binding.property_iri
            ))
        })?;
        let embedders = embedders.ok_or_else(|| {
            QueryError::evaluation(
                "no Embedder registry available for the `~` operator's vector path",
            )
        })?;
        let embedder = embedders.get(&idx.model).ok_or_else(|| {
            QueryError::evaluation(format!(
                "no Embedder registered for model '{}' (required by VectorIndex '{}')",
                idx.model, idx.iri
            ))
        })?;
        let query_vec = embedder
            .embed(&query_string)
            .map_err(|e| QueryError::evaluation(format!("embedder dispatch failed: {e}")))?;
        let metric = Metric::from_short_name(idx.distance.as_str()).ok_or_else(|| {
            QueryError::evaluation(format!(
                "VectorIndex '{}' declares unknown distance metric '{}'",
                idx.iri, idx.distance
            ))
        })?;
        let vec_backend = layer.storage().vector_index.clone();
        top_k_subjects(
            layer,
            vec_backend.as_ref(),
            vector_segment_cache,
            &idx.iri,
            &query_vec,
            vector_limit,
            None,
            &idx.model,
            metric,
        )
        .map_err(|e| QueryError::evaluation(format!("vector probe failed: {e}")))?
    } else {
        Vec::new()
    };

    let scores = fuse_rrf(&text_hits, &vector_hits, k);
    Ok(SimilarityProbe {
        subject_var: binding.subject_var.clone(),
        scores,
    })
}

/// D43 §6.2 / §6.4 — how many candidates a probe must return, for a query that ranks.
/// `None` where nothing derives a bound, which leaves each probe to its own default.
///
/// An explicit `{ limit: N }` is the author saying what they want, so it wins outright
/// (§3.4). Otherwise `TOP N` derives the bound, and it is `N * OVER_FETCH_FACTOR` rather
/// than `N`: over-fetch exists so that structural filters applied AFTER the probe can eat
/// into the candidate set without the survivor count falling below N. Fetching exactly N
/// leaves nothing for them to eat.
///
/// `DEFAULT_LIMIT` is a floor under that, not the policy. An earlier version had it the
/// other way around — `max(N, DEFAULT_LIMIT)` — which over-fetches a `TOP 10` to 200 and
/// under-fetches a `TOP 1000` to exactly 1000. That was reasoning about a floor where the
/// spec asks for a factor (eigenius#62).
fn candidate_pool(explicit_limit: Option<usize>, top: Option<usize>) -> Option<usize> {
    explicit_limit.or_else(|| {
        top.map(|n| {
            n.saturating_mul(crate::query::OVER_FETCH_FACTOR)
                .max(DEFAULT_LIMIT)
        })
    })
}

/// Reciprocal Rank Fusion across the ranked sources (D43 §3.5 / §6.4). Each source
/// contributes `1 / (k + rank)` to every subject it ranked; a subject a source did not
/// rank contributes nothing from it.
///
/// **It calls `query::rank`, which is the point.** That module is the tested
/// implementation of §6.4 — 1-indexed ranks, deterministic tie-breaking, and an explicit
/// missing-rank representation — and nothing called it while this function reimplemented
/// the same formula (eigenius#125). Two implementations of one specification drift.
///
/// **Ranks come from the scores, not from each probe's return order, and that settles a
/// disagreement.** The two probes tie-break in OPPOSITE directions: the text probe sorts
/// score descending then `defining_layer` ascending then subject ascending, while the
/// vector probe's `BinaryHeap<Reverse<HeapEntry>>::into_sorted_vec()` yields the whole
/// tuple descending — layer and subject included. Nothing documented the difference and
/// there is no reason for two sources feeding one fusion to order ties opposite ways.
/// Keying `assign_ranks_desc` on `(LayerId, Iri)` gives both the text probe's order.
///
/// The `&[Option<usize>]` shape is also what D43 §3.6.4 describes — each `~` operator
/// contributes a ranked source, fused by one mechanism — and what a `weights:` hint
/// (§3.4, reserved) would extend. A fixed pair of lists has to be rewritten for that; a
/// slice of per-source ranks gains an element.
fn fuse_rrf(
    text_hits: &[TextScoredHit],
    vector_hits: &[VectorScoredHit],
    k: usize,
) -> BTreeMap<Iri, f64> {
    // Keyed by `(defining_layer, subject)` so ties break on the layer first, then the
    // IRI. `is_shadowed` drops a subject's hits from every layer but the one defining it,
    // so the key is unique within a source.
    let rank_by_subject = |scored: Vec<((LayerId, Iri), f64)>| -> BTreeMap<Iri, usize> {
        crate::query::rank::assign_ranks_desc(&scored)
            .into_iter()
            .map(|((_layer, subject), rank)| (subject, rank))
            .collect()
    };
    let text_ranks = rank_by_subject(
        text_hits
            .iter()
            .map(|h| {
                (
                    (h.defining_layer.clone(), h.subject.clone()),
                    h.score as f64,
                )
            })
            .collect(),
    );
    let vector_ranks = rank_by_subject(
        vector_hits
            .iter()
            .map(|h| {
                (
                    (h.defining_layer.clone(), h.subject.clone()),
                    h.similarity as f64,
                )
            })
            .collect(),
    );

    let mut scores: BTreeMap<Iri, f64> = BTreeMap::new();
    for subject in text_ranks.keys().chain(vector_ranks.keys()) {
        if scores.contains_key(subject) {
            continue;
        }
        let per_source = [
            text_ranks.get(subject).copied(),
            vector_ranks.get(subject).copied(),
        ];
        scores.insert(
            subject.clone(),
            crate::query::rank::rrf_score(&per_source, k as u32),
        );
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("valid iri")
    }

    #[test]
    fn rrf_combines_two_sources_under_default_k() {
        let text = vec![
            TextScoredHit {
                subject: iri("urn:ex:a"),
                score: 5.0,
                defining_layer: crate::layer::LayerId([0; 32]),
            },
            TextScoredHit {
                subject: iri("urn:ex:b"),
                score: 2.0,
                defining_layer: crate::layer::LayerId([0; 32]),
            },
        ];
        let vector = vec![
            VectorScoredHit {
                subject: iri("urn:ex:b"),
                similarity: 0.9,
                defining_layer: crate::layer::LayerId([0; 32]),
            },
            VectorScoredHit {
                subject: iri("urn:ex:c"),
                similarity: 0.5,
                defining_layer: crate::layer::LayerId([0; 32]),
            },
        ];
        let scores = fuse_rrf(&text, &vector, 60);
        // a only in text@rank 1 → 1/61
        let a = *scores.get(&iri("urn:ex:a")).unwrap();
        // b in both: text@rank 2 + vector@rank 1 → 1/62 + 1/61
        let b = *scores.get(&iri("urn:ex:b")).unwrap();
        // c only in vector@rank 2 → 1/62
        let c = *scores.get(&iri("urn:ex:c")).unwrap();
        assert!((a - 1.0 / 61.0).abs() < 1e-9);
        assert!((b - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-9);
        assert!((c - 1.0 / 62.0).abs() < 1e-9);
        // b appears in both → highest fused score.
        assert!(b > a && b > c);
    }

    /// **The candidate pool is a FACTOR over `TOP K`, with a floor — not a floor alone.**
    ///
    /// D43 §6.2 exists because structural filters run after the probe: fetching exactly K
    /// leaves nothing for them to eat, so the survivor count can fall below K. An earlier
    /// version took `max(K, DEFAULT_LIMIT)`, which is the floor doing all the work — fine
    /// at `TOP 10`, and at `TOP 1000` it fetches exactly 1000 (eigenius#62).
    #[test]
    fn the_candidate_pool_over_fetches_by_the_factor() {
        // Below the floor, the floor wins: 10 * 4 = 40 < 200.
        assert_eq!(candidate_pool(None, Some(10)), Some(DEFAULT_LIMIT));
        // Above it, the factor does: this is the case `max(K, DEFAULT_LIMIT)` got wrong.
        assert_eq!(candidate_pool(None, Some(100)), Some(400));
        assert_eq!(candidate_pool(None, Some(1000)), Some(4000));
        // An explicit hint is the author overriding the derivation, in either direction.
        assert_eq!(candidate_pool(Some(5), Some(1000)), Some(5));
        assert_eq!(candidate_pool(Some(9_000), None), Some(9_000));
        // Nothing to derive from: each probe keeps its own default.
        assert_eq!(candidate_pool(None, None), None);
    }

    /// **A tie fuses the same way whichever order a probe emitted it in.**
    ///
    /// Rank came from each hit's POSITION in its probe's returned slice, so the fused
    /// score of a tied pair was a property of how the probe happened to emit it — and the
    /// two probes emit ties in opposite directions, text ascending by `defining_layer`
    /// then subject, vector descending by both because its heap yields the whole tuple
    /// reversed. Ranks are derived from the scores now, under one key, so emission order
    /// does not reach the result.
    ///
    /// The tie is still broken, not collapsed: `urn:ex:a` sorts before `urn:ex:b` in both
    /// sources, so it takes rank 1 in both. Under the old path the two probes disagreed
    /// and the scores cancelled to equal — which looks fairer and was an accident of the
    /// disagreement, not a decision.
    #[test]
    fn a_tie_fuses_independently_of_the_order_a_probe_emitted_it() {
        let layer = crate::layer::LayerId([0; 32]);
        let text = |first: &str, second: &str| {
            vec![
                TextScoredHit {
                    subject: iri(first),
                    score: 1.0,
                    defining_layer: layer.clone(),
                },
                TextScoredHit {
                    subject: iri(second),
                    score: 1.0,
                    defining_layer: layer.clone(),
                },
            ]
        };
        let vector = |first: &str, second: &str| {
            vec![
                VectorScoredHit {
                    subject: iri(first),
                    similarity: 1.0,
                    defining_layer: layer.clone(),
                },
                VectorScoredHit {
                    subject: iri(second),
                    similarity: 1.0,
                    defining_layer: layer.clone(),
                },
            ]
        };

        // The probes' actual directions: text ascending, vector descending.
        let as_emitted = fuse_rrf(
            &text("urn:ex:a", "urn:ex:b"),
            &vector("urn:ex:b", "urn:ex:a"),
            60,
        );
        // Both flipped.
        let flipped = fuse_rrf(
            &text("urn:ex:b", "urn:ex:a"),
            &vector("urn:ex:a", "urn:ex:b"),
            60,
        );
        assert_eq!(
            as_emitted, flipped,
            "emission order must not reach the fused score"
        );

        // One key orders the tie, so `a` is rank 1 in both sources and `b` rank 2.
        let a = *as_emitted.get(&iri("urn:ex:a")).unwrap();
        let b = *as_emitted.get(&iri("urn:ex:b")).unwrap();
        assert!((a - 2.0 / 61.0).abs() < 1e-12, "a is rank 1 in both: {a}");
        assert!((b - 2.0 / 62.0).abs() < 1e-12, "b is rank 2 in both: {b}");
    }

    #[test]
    fn rrf_with_smaller_k_emphasises_rank_one() {
        // Default k=60 produces a flatter distribution; smaller k
        // weights the top rank more heavily. Verify the relative
        // weighting changes as expected.
        let text = vec![TextScoredHit {
            subject: iri("urn:ex:a"),
            score: 1.0,
            defining_layer: crate::layer::LayerId([0; 32]),
        }];
        let default_score = *fuse_rrf(&text, &[], 60).get(&iri("urn:ex:a")).unwrap();
        let tight_score = *fuse_rrf(&text, &[], 10).get(&iri("urn:ex:a")).unwrap();
        assert!(tight_score > default_score);
    }

    // ─── D43 §3.6 — end-to-end `~` operator evaluator tests ────────────

    use crate::bootstrap::bootstrap;
    use crate::layer::{Layer, LayerBuilder};
    use crate::ontology::resource::Resource;
    use crate::program::embedder::registry_with_dummy;
    use crate::query::execute_with;
    use std::sync::Arc;

    fn make_resource(id: &str, class_iri: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::iri(&iri(class_iri))]),
        );
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    /// Bootstrap, declare a `description` Property + a TextIndex on it,
    /// and add three Documents. Auto-indexing at commit time populates
    /// the TextIndex so a `~` probe against this head returns rows.
    fn build_text_corpus() -> Arc<Layer> {
        let ctx = bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("text-corpus", Some(head));

        b.add_resource(make_resource(
            "urn:ex:description",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_description".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::iri(&iri("urn:eigenius:core:string")),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:Document",
            "urn:eigenius:core:Class",
            vec![(
                "urn:eigenius:core:short_name",
                Value::String("Document".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:ti_desc",
            "urn:eigenius:core:TextIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::iri(&iri("urn:ex:description")),
                ),
                (
                    "urn:eigenius:core:text_analyzer",
                    Value::String("en-stem-v1".into()),
                ),
            ],
        ))
        .unwrap();

        b.add_resource(make_resource(
            "urn:ex:d1",
            "urn:ex:Document",
            vec![(
                "urn:ex:description",
                Value::String("kernel layer chain consolidation".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:d2",
            "urn:ex:Document",
            vec![(
                "urn:ex:description",
                Value::String("walk the chain and apply shadow filter".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:d3",
            "urn:ex:Document",
            vec![(
                "urn:ex:description",
                Value::String("WAL truncation under concurrent commit".into()),
            )],
        ))
        .unwrap();

        Arc::new(b.build(storage))
    }

    /// Find the synthesized row-property IRI for the RETURN item
    /// named `short_name` by scanning the wrapped result's Property
    /// metadata resources. Mirrors `QueryFingerprint::row_property_iri`
    /// but without needing to recompute the query hash.
    fn row_property_iri_for(wrapped: &[Resource], short_name: &str) -> Iri {
        let short_prop = Iri::parse(crate::ontology::well_known::SHORT_NAME).unwrap();
        wrapped
            .iter()
            .find(|r| {
                matches!(r.get(&short_prop), Some(Value::String(s)) if s == short_name)
                    && r.id().is_some()
                    && r.id().unwrap().as_str().contains(":row:")
            })
            .and_then(|r| r.id().cloned())
            .unwrap_or_else(|| panic!("no row Property with short_name '{short_name}'"))
    }

    /// Extract the per-row subject IRIs from a wrapped query result.
    /// Reaches into the ResultSet Resource's `urn:eigenius:query:rows`
    /// array (each entry an embedded row Resource) and projects the
    /// requested `slot`'s value to an IRI string. Wrap structure is
    /// per D2 Appendix A.
    fn matched_subject_iris(wrapped: &[Resource], slot: &str) -> Vec<String> {
        let prop = row_property_iri_for(wrapped, slot);
        let rows_prop = Iri::parse("urn:eigenius:query:rows").unwrap();
        let result_set = wrapped
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str().ends_with(":result"))
                    .unwrap_or(false)
            })
            .expect("result set Resource");
        let rows = match result_set.get(&rows_prop) {
            Some(Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };
        rows.iter()
            .filter_map(|v| match v {
                Value::Embedded(r) => r.get(&prop).cloned(),
                _ => None,
            })
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// **Seeding must not fire for a `~` that is not a conjunct.**
    ///
    /// The pre-pass registers a probe for EVERY similarity node in the program — under
    /// `OR`, under `NOT`, and in `RETURN` / `ORDER BY` where the operator is a score
    /// projection that filters nothing. Narrowing candidates from any of those is a wrong
    /// answer, and an earlier version did exactly that by matching probes on the variable
    /// NAME, having discarded where in the expression they came from.
    ///
    /// Each case below returned fewer rows than it should. They are the discriminating
    /// tests for the fix: against the name-based version, every one fails.
    #[test]
    fn a_similarity_under_a_disjunction_does_not_narrow() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { "urn:ex:description": ?desc }
            WHERE ?desc ~ "kernel" OR ?desc ~ "chain"
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        // d1 carries "kernel", d2 carries "chain". Seeding from either branch alone, or
        // from their intersection, loses one of them.
        assert_eq!(
            matched_subject_iris(&rows, "d").len(),
            2,
            "a disjunction admits the union of its branches"
        );
    }

    #[test]
    fn a_similarity_under_a_negation_does_not_narrow() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { "urn:ex:description": ?desc }
            WHERE NOT (?desc ~ "kernel")
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        // Seeding here would start from exactly the set the predicate then rejects, so
        // the query returned nothing at all.
        let matched = matched_subject_iris(&rows, "d");
        assert!(
            !matched.is_empty(),
            "a negation admits the rows the probe did NOT rank, got {matched:?}"
        );
        assert!(!matched.iter().any(|s| s == "urn:ex:d1"));
    }

    #[test]
    fn a_similarity_in_return_position_does_not_narrow() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { "urn:ex:description": ?desc }
            RETURN [] { d: ?d, s: ?desc ~ "kernel" }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        // A score projection filters nothing: every document is still a row.
        assert_eq!(
            matched_subject_iris(&rows, "d").len(),
            3,
            "a `~` in RETURN ranks rows, it does not select them"
        );
    }

    /// The conjunctive case, which is the one seeding is FOR: same answer as before, and
    /// the untyped shape is the one that used to reach the full-chain scan.
    #[test]
    fn a_conjunctive_similarity_narrows_without_changing_the_answer() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING NAMESPACE "urn:ex:"
            MATCH ?d { "urn:ex:description": ?desc }
            WHERE ?desc ~ "WAL truncation"
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        assert_eq!(
            matched_subject_iris(&rows, "d"),
            vec!["urn:ex:d3".to_string()]
        );
    }

    #[test]
    fn end_to_end_text_only_filters_matching_subjects() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "WAL truncation"
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        assert_eq!(matched, vec!["urn:ex:d3".to_string()]);
    }

    #[test]
    fn end_to_end_text_filters_out_non_matching() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "chain"
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        // d1 and d2 contain "chain"; d3 does not.
        assert_eq!(matched.len(), 2);
        assert!(matched.iter().any(|s| s == "urn:ex:d1"));
        assert!(matched.iter().any(|s| s == "urn:ex:d2"));
        assert!(!matched.iter().any(|s| s == "urn:ex:d3"));
    }

    #[test]
    fn end_to_end_via_text_hint_takes_text_path_when_only_text_active() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "WAL" { via: text }
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        assert_eq!(matched, vec!["urn:ex:d3".to_string()]);
    }

    #[test]
    fn end_to_end_via_vector_without_vector_index_fails_at_typecheck() {
        let layer = build_text_corpus();
        let errs = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "kernel" { via: vector }
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect_err("typecheck should reject via: vector with no VectorIndex");
        assert!(
            errs.iter()
                .any(|e| e.rule == "similarity_hint_via_vector_no_vector_index"),
            "unexpected errors: {errs:?}"
        );
    }

    /// Vector path requires an Embedder; without one registered, the
    /// pre-pass should surface a clear evaluator error rather than
    /// silently returning zero hits.
    #[test]
    fn vector_path_without_embedder_registry_errors() {
        let ctx = bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("vector-corpus", Some(head));
        b.add_resource(make_resource(
            "urn:ex:description",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_description".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::iri(&iri("urn:eigenius:core:string")),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:Document",
            "urn:eigenius:core:Class",
            vec![(
                "urn:eigenius:core:short_name",
                Value::String("Document".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:vi_desc",
            "urn:eigenius:core:VectorIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::iri(&iri("urn:ex:description")),
                ),
                (
                    "urn:eigenius:core:vec_model",
                    Value::iri(&iri("urn:eigenius:embed:dummy:v1")),
                ),
                ("urn:eigenius:core:vec_dim", Value::Integer(8)),
                (
                    "urn:eigenius:core:vec_distance",
                    Value::String(
                        iri("urn:eigenius:core:distances:cosine")
                            .as_str()
                            .to_string(),
                    ),
                ),
            ],
        ))
        .unwrap();
        let layer = Arc::new(b.build(storage));

        let errs = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "anything"
            RETURN [] { d: ?d }
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect_err("evaluator should reject vector probe with no embedders");
        assert!(
            errs.iter().any(|e| e.message.contains("Embedder registry")),
            "unexpected errors: {errs:?}"
        );
    }

    /// With an Embedder registered, the vector path runs even without
    /// any pre-indexed segments — the result is just empty. Sanity
    /// check that the pipeline doesn't fail short of any matches.
    #[test]
    fn vector_path_with_embedder_returns_empty_when_no_segments() {
        let ctx = bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("vector-corpus", Some(head));
        b.add_resource(make_resource(
            "urn:ex:description",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_description".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::iri(&iri("urn:eigenius:core:string")),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:Document",
            "urn:eigenius:core:Class",
            vec![(
                "urn:eigenius:core:short_name",
                Value::String("Document".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:vi_desc",
            "urn:eigenius:core:VectorIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::iri(&iri("urn:ex:description")),
                ),
                (
                    "urn:eigenius:core:vec_model",
                    Value::iri(&iri("urn:eigenius:embed:dummy:v1")),
                ),
                ("urn:eigenius:core:vec_dim", Value::Integer(8)),
                (
                    "urn:eigenius:core:vec_distance",
                    Value::String(
                        iri("urn:eigenius:core:distances:cosine")
                            .as_str()
                            .to_string(),
                    ),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:d1",
            "urn:ex:Document",
            vec![("urn:ex:description", Value::String("foo bar".into()))],
        ))
        .unwrap();
        let layer = Arc::new(b.build(storage));

        let embedders = registry_with_dummy();
        let runtime = crate::query::evaluate::FiberRuntime {
            embedders: Some(&embedders),
            ..crate::query::evaluate::FiberRuntime::default()
        };
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "anything"
            RETURN [] { d: ?d }
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        // No segment has been written for the VectorIndex, so the
        // probe returns no matches — `matched_subject_iris` is empty.
        let matched = matched_subject_iris(&rows, "d");
        assert!(matched.is_empty(), "expected no matches, got {matched:?}");
    }

    // ─── D43 §3.3 — TOP K end-to-end ──────────────────────────────────

    #[test]
    fn top_truncates_after_similarity_ranking() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "chain"
            RETURN [] { d: ?d }
            TOP 1
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        // Both d1 and d2 contain "chain"; TOP 1 keeps the one with
        // the higher BM25 score. Either is a valid winner under the
        // tie-breaking rules, but the candidate set is bounded to 1.
        assert_eq!(matched.len(), 1);
        assert!(matched[0] == "urn:ex:d1" || matched[0] == "urn:ex:d2");
    }

    #[test]
    fn top_keeps_only_ranked_subjects() {
        let layer = build_text_corpus();
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "chain"
            RETURN [] { d: ?d }
            TOP 5
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        // Asking for TOP 5 when only 2 rows match returns 2 — TOP
        // is a ceiling, not a floor, and only ranked subjects
        // (those in any probe's score map) survive into RETURN.
        assert_eq!(matched.len(), 2);
        assert!(matched.iter().any(|s| s == "urn:ex:d1"));
        assert!(matched.iter().any(|s| s == "urn:ex:d2"));
    }

    /// Build a corpus with **both** a TextIndex and a VectorIndex
    /// active on the same property, plus three Documents whose vector
    /// segments are populated via the post-Load sweep. Used by the
    /// hybrid-fusion integration tests.
    fn build_hybrid_corpus() -> (Arc<Layer>, EmbedderRegistry) {
        let ctx = bootstrap().expect("bootstrap should succeed");
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("hybrid-corpus", Some(head));

        b.add_resource(make_resource(
            "urn:ex:description",
            "urn:eigenius:core:Property",
            vec![
                (
                    "urn:eigenius:core:short_name",
                    Value::String("test_description".into()),
                ),
                (
                    "urn:eigenius:core:data_type",
                    Value::iri(&iri("urn:eigenius:core:string")),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:Document",
            "urn:eigenius:core:Class",
            vec![(
                "urn:eigenius:core:short_name",
                Value::String("Document".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:ti_desc",
            "urn:eigenius:core:TextIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::iri(&iri("urn:ex:description")),
                ),
                (
                    "urn:eigenius:core:text_analyzer",
                    Value::String("en-stem-v1".into()),
                ),
            ],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:vi_desc",
            "urn:eigenius:core:VectorIndex",
            vec![
                (
                    "urn:eigenius:core:target_property",
                    Value::iri(&iri("urn:ex:description")),
                ),
                (
                    "urn:eigenius:core:vec_model",
                    Value::iri(&iri("urn:eigenius:embed:dummy:v1")),
                ),
                ("urn:eigenius:core:vec_dim", Value::Integer(8)),
                (
                    "urn:eigenius:core:vec_distance",
                    Value::String(
                        iri("urn:eigenius:core:distances:cosine")
                            .as_str()
                            .to_string(),
                    ),
                ),
            ],
        ))
        .unwrap();

        b.add_resource(make_resource(
            "urn:ex:d1",
            "urn:ex:Document",
            vec![(
                "urn:ex:description",
                Value::String("kernel layer chain consolidation".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:d2",
            "urn:ex:Document",
            vec![(
                "urn:ex:description",
                Value::String("walk the chain and apply shadow filter".into()),
            )],
        ))
        .unwrap();
        b.add_resource(make_resource(
            "urn:ex:d3",
            "urn:ex:Document",
            vec![(
                "urn:ex:description",
                Value::String("WAL truncation under concurrent commit".into()),
            )],
        ))
        .unwrap();

        let layer = Arc::new(b.build(storage));
        let embedders = registry_with_dummy();
        // D43 §5.5 — post-Load sweep populates the VectorIndex
        // segments. Without it the vector probe sees an empty
        // candidate set and the hybrid path degenerates to text-only.
        crate::query::vector::indexing::sweep_layer_vectors(&layer, &embedders, None)
            .expect("vector sweep should succeed");
        (layer, embedders)
    }

    /// Hybrid retrieval (TextIndex + VectorIndex on the same
    /// property) fuses both probe contributions via RRF. The query
    /// "chain" matches d1 and d2 textually; the vector probe under
    /// the DummyEmbedder ranks all 3 documents (the corpus is below
    /// the default per-source limit). The fused ranking is
    /// dominated by the text contribution: d1 and d2 both receive
    /// text + vector contributions and outrank d3, which receives
    /// only the vector contribution. TOP 2 returns {d1, d2}.
    #[test]
    fn hybrid_text_plus_vector_ranks_text_matches_above_vector_only() {
        let (layer, embedders) = build_hybrid_corpus();
        let runtime = crate::query::evaluate::FiberRuntime {
            embedders: Some(&embedders),
            ..crate::query::evaluate::FiberRuntime::default()
        };
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "chain"
            RETURN [] { d: ?d }
            TOP 2
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        assert_eq!(
            matched.len(),
            2,
            "expected TOP 2 to keep the two text-matching docs, got {matched:?}"
        );
        assert!(matched.iter().any(|s| s == "urn:ex:d1"));
        assert!(matched.iter().any(|s| s == "urn:ex:d2"));
        assert!(!matched.iter().any(|s| s == "urn:ex:d3"));
    }

    /// `via: hybrid` forces both probes even when defaults would
    /// route differently. Same expected ranking as the default-
    /// hybrid case above; the test asserts that the explicit hint
    /// routes correctly and produces the same fused result.
    #[test]
    fn hybrid_explicit_via_hint_matches_default_path() {
        let (layer, embedders) = build_hybrid_corpus();
        let runtime = crate::query::evaluate::FiberRuntime {
            embedders: Some(&embedders),
            ..crate::query::evaluate::FiberRuntime::default()
        };
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "chain" { via: hybrid }
            RETURN [] { d: ?d }
            TOP 2
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        assert_eq!(matched.len(), 2);
        assert!(matched.iter().any(|s| s == "urn:ex:d1"));
        assert!(matched.iter().any(|s| s == "urn:ex:d2"));
    }

    /// `via: text` on a property that has both indexes skips the
    /// vector probe entirely. The result must match the text-only
    /// candidate set; d3 (which only the vector probe ranks under
    /// the default-hybrid path) drops out.
    #[test]
    fn hybrid_corpus_with_via_text_skips_vector_probe() {
        let (layer, embedders) = build_hybrid_corpus();
        let runtime = crate::query::evaluate::FiberRuntime {
            embedders: Some(&embedders),
            ..crate::query::evaluate::FiberRuntime::default()
        };
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "chain" { via: text }
            RETURN [] { d: ?d }
            "#,
            &layer,
            runtime,
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        assert_eq!(matched.len(), 2);
        assert!(matched.iter().any(|s| s == "urn:ex:d1"));
        assert!(matched.iter().any(|s| s == "urn:ex:d2"));
        assert!(!matched.iter().any(|s| s == "urn:ex:d3"));
    }

    /// With two `~` operators in disjunction, rows that satisfy both
    /// accumulate score and rank higher than rows that satisfy only
    /// one (D43 §3.3 — "rows satisfying both rank higher").
    #[test]
    fn top_with_disjunctive_similarity_ranks_overlap_highest() {
        let layer = build_text_corpus();
        // d1: "kernel layer chain consolidation"  — matches "kernel" AND "chain"
        // d2: "walk the chain and apply shadow filter" — matches "chain" only
        // d3: "WAL truncation under concurrent commit" — matches neither
        let rows = execute_with(
            r#"
            USING "urn:ex:Document"
            USING NAMESPACE "urn:ex:"
            MATCH Document(?d) { "urn:ex:description": ?desc }
            WHERE ?desc ~ "kernel" OR ?desc ~ "chain"
            RETURN [] { d: ?d }
            TOP 1
            "#,
            &layer,
            crate::query::evaluate::FiberRuntime::default(),
        )
        .expect("query should succeed");
        let matched = matched_subject_iris(&rows, "d");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0], "urn:ex:d1");
    }
}
