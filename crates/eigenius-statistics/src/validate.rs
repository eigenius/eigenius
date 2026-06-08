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

//! `ValidateMeasurementClaim` handler (D52 §6).
//!
//! Algorithm:
//!
//! 1. Read the claim's `sample_set` IRI; resolve to the
//!    [`stats:SampleSetResource`] on chain.
//! 2. Read the SampleSetResource's `sample_set_value` — a
//!    `Value::Json` carrying the `Bundle` ctor over the 5-axis
//!    product + observations.
//! 3. Decode the Bundle's axis slots; keep the observations slot
//!    raw — each dispatch arm decodes per its expected shape.
//! 4. Dispatch on the product position (D52 §5.4 table). Phase 1
//!    wired SingleSampleEstimate; Phase 1.5 added IID. Unsupported
//!    positions return `Verdict::Fails(WrongTestForDesign)`.
//! 5. Read the claim's `alpha`, `effect_size`, `directionality`,
//!    `variance_assumption` fields. Run the dispatch arm's numerics
//!    routine. Each arm reduces to a `(t_statistic, p_value)` tuple
//!    for the common verdict-building step.
//! 6. Run the §7.4 epistemic-scope check against the
//!    `canonical_proposition`'s head predicate's `is_a` markers.
//! 7. Build the verdict resource — Holds when p < alpha, Fails with
//!    structured diagnostic otherwise. Both outcomes carry the
//!    computed numerics for audit.
//!
//! Phase 1 + 1.5 coverage: SingleSampleEstimate + IID (Welch +
//! Pooled). Phase 2 adds Paired + Factorial. §7.2 non-Identity
//! outlier dual-verdict, §7.3 Passing-Bablok for method-comparison,
//! and §7.1 OneSidedWitnessed impossibility-witness validation are
//! Phase 5 hardening; the surfaces are in place but enforcement is
//! deferred until the basic dispatch table is wider.

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::QueryOutcome;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use crate::institution::iris;
use crate::institution::StatisticsInstitution;
use crate::numerics::{
    factorial_omnibus_anova, one_sample_t_test, paired_t_test, rcbd_anova,
    repeated_measures_cs_anova, splitplot_anova, two_sample_t_test, TwoSampleVariance,
};

/// Top-level handler called by `StatisticsInstitution::query`.
pub fn do_validate_measurement_claim(
    _inst: &StatisticsInstitution,
    claim: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    // ── Step 1: read sample_set IRI from the claim ────────────────────
    let sample_set_iri_str = match read_iri_property(claim, iris::PROP_SAMPLE_SET)? {
        Some(s) => s,
        None => {
            return Ok(verdict_fails(
                "MeasurementClaim missing required `sample_set` property".into(),
            ));
        }
    };
    let sample_set_iri = match Iri::parse(&sample_set_iri_str) {
        Ok(i) => i,
        Err(e) => {
            return Ok(verdict_fails(format!(
                "MeasurementClaim's sample_set value `{sample_set_iri_str}` is not a valid IRI: {e}"
            )));
        }
    };

    // ── Step 2: resolve the SampleSetResource and its inductive value ─
    let sample_set_res = match ctx.resolve(&sample_set_iri) {
        Some(r) => r,
        None => {
            return Ok(verdict_fails(format!(
                "SampleSetResource `{sample_set_iri}` not found on chain"
            )));
        }
    };
    let sample_set_value_iri =
        Iri::parse("urn:eigenius:measurements:sample_set_value").expect("static IRI");
    let bundle_value = match sample_set_res.get(&sample_set_value_iri) {
        Some(v) => v,
        None => {
            return Ok(verdict_fails(format!(
                "SampleSetResource `{sample_set_iri}` missing required \
                 `sample_set_value` property"
            )));
        }
    };
    let bundle_json = match bundle_value {
        Value::Json(j) => j,
        other => {
            return Ok(verdict_fails(format!(
                "SampleSetResource `{sample_set_iri}`'s sample_set_value is not a chain \
                 inductive value (got {other:?})"
            )));
        }
    };

    // ── Step 3: decode the Bundle ctor's args ─────────────────────────
    let bundle = match decode_bundle(bundle_json) {
        Ok(b) => b,
        Err(diag) => return Ok(verdict_fails(diag)),
    };

    // ── Step 4: dispatch on the product position ──────────────────────
    let dispatch = match dispatch_product_position(&bundle) {
        Some(d) => d,
        None => {
            return Ok(verdict_fails(format!(
                "WrongTestForDesign: product position {:?} has no Phase 1 verifier procedure \
                 (Phase 1 implements only SingleSampleEstimate; other Tier 1+2 designs land \
                 in follow-on commits)",
                (
                    &bundle.randomization,
                    &bundle.blocking,
                    &bundle.factor,
                    &bundle.repeated_measures,
                ),
            )));
        }
    };

    // ── Step 5: read claim parameters ─────────────────────────────────
    let alpha = match read_float_property(claim, iris::PROP_ALPHA)? {
        Some(a) => a,
        None => return Ok(verdict_fails("claim missing `alpha`".into())),
    };
    let directionality = match read_json_property(claim, iris::PROP_DIRECTIONALITY)? {
        Some(j) => j,
        None => return Ok(verdict_fails("claim missing `directionality`".into())),
    };
    let effect_size = match read_json_property(claim, iris::PROP_EFFECT_SIZE)? {
        Some(j) => j,
        None => return Ok(verdict_fails("claim missing `effect_size`".into())),
    };
    // Phase 1 supports only TwoSided directionality. OneSidedWitnessed
    // requires §7.1 impossibility-witness validation, deferred to a
    // follow-on.
    let directionality_ctor = json_ctor_name(&directionality);
    if directionality_ctor != Some("TwoSided") {
        return Ok(verdict_fails(format!(
            "directionality `{directionality_ctor:?}` not supported in Phase 1 \
             (only TwoSided wired; OneSidedWitnessed requires §7.1 impossibility-witness \
             validation, deferred)"
        )));
    }

    // Read variance_assumption for the IID dispatch arm (one-sample
    // dispatch ignores it — there's only one variance parameter to
    // estimate there).
    let variance_assumption = read_json_property(claim, iris::PROP_VARIANCE_ASSUMPTION)?;

    // ── Step 6: run the test (dispatch-specific) ──────────────────────
    //
    // Each arm decodes the observations payload for its expected
    // shape, runs the matching numerics routine, and reduces to a
    // `(t_statistic, p_value_two_sided)` tuple the common verdict
    // builder consumes. Per-arm error returns short-circuit with a
    // structured-diagnostic Fails verdict (§6).
    // Each arm returns `(statistic, p_value, diagnostic_note)`.
    // The diagnostic_note is `None` for arms with a single F/t-test;
    // SplitPlot uses it to name which of its three F-tests produced
    // the reported p-value.
    let (t_statistic, p_value_two_sided, diagnostic_note): (f64, f64, Option<String>) =
        match dispatch {
            DispatchPos::SingleSampleEstimate => {
                // Only `EffectSize.Absolute(magnitude, units)` is wired in
                // Phase 1. The one-sample test checks whether the
                // SampleSet's mean falls on the asserted threshold's side.
                let (magnitude, _units) = match parse_effect_size_absolute(&effect_size) {
                    Some(p) => p,
                    None => {
                        return Ok(verdict_fails(
                            "Phase 1 only supports EffectSize.Absolute(magnitude, units); \
                         StandardizedCohensD/HedgesG and Relative not yet wired"
                                .into(),
                        ));
                    }
                };
                let samples = match decode_flat_observations(&bundle.observations_raw) {
                    Ok(s) => s,
                    Err(diag) => return Ok(verdict_fails(diag)),
                };
                let r = match one_sample_t_test(&samples, magnitude) {
                    Some(r) => r,
                    None => {
                        return Ok(verdict_fails(format!(
                        "InsufficientReplication: one-sample t-test requires n >= 2, got n = {}",
                        samples.len()
                    )));
                    }
                };
                (r.t_statistic, r.p_value_two_sided, None)
            }
            DispatchPos::IID => {
                // IID two-sample: observations is `[group_a, group_b]`
                // (nested value-array). The two groups go to the
                // two-sample t-test under the claim's variance assumption.
                // EffectSize is read for the verdict's audit trail but the
                // two-sample H0 (mean_a = mean_b) doesn't carry a
                // numerical threshold — the "effect size" is the asserted
                // *minimum* mean difference; v1 dispatches on p < alpha
                // alone and notes the threshold in the diagnostic.
                let (group_a, group_b) =
                    match decode_two_group_observations(&bundle.observations_raw) {
                        Ok(pair) => pair,
                        Err(diag) => return Ok(verdict_fails(diag)),
                    };
                let variance = match variance_assumption.as_ref().and_then(json_ctor_name) {
                    Some("Pooled") => TwoSampleVariance::Pooled,
                    Some("WelchUnequal") => TwoSampleVariance::WelchUnequal,
                    Some(other) => {
                        return Ok(verdict_fails(format!(
                            "IID two-sample with variance_assumption `{other}` not yet wired \
                         (Phase 1.5 supports Pooled / WelchUnequal; NonParametric / RankBased \
                         are follow-on)"
                        )));
                    }
                    None => TwoSampleVariance::WelchUnequal,
                };
                let r = match two_sample_t_test(&group_a, &group_b, variance) {
                    Some(r) => r,
                    None => {
                        return Ok(verdict_fails(format!(
                            "InsufficientReplication: two-sample t-test requires n >= 2 in each \
                         group, got n_a = {}, n_b = {}",
                            group_a.len(),
                            group_b.len()
                        )));
                    }
                };
                (r.t_statistic, r.p_value_two_sided, None)
            }
            DispatchPos::Paired => {
                // Paired: observations is a flat array `[b0, a0, b1, a1,
                // ..., bn, an]` of before/after pairs interleaved. Chunk
                // into (before, after) tuples and run the paired t-test
                // (= one-sample t-test on the per-pair differences vs 0).
                let pairs = match decode_paired_observations(&bundle.observations_raw) {
                    Ok(p) => p,
                    Err(diag) => return Ok(verdict_fails(diag)),
                };
                let r = match paired_t_test(&pairs) {
                    Some(r) => r,
                    None => {
                        return Ok(verdict_fails(format!(
                            "InsufficientReplication: paired t-test requires n_pairs >= 2, got {}",
                            pairs.len()
                        )));
                    }
                };
                (r.t_statistic, r.p_value_two_sided, None)
            }
            DispatchPos::Factorial => {
                // Factorial: observations is `[factor_levels,
                // flat_observations]` where flat_observations is a flat
                // float array `[level_00, level_01, ..., level_0{k-1},
                // value_0, level_10, ..., value_n]` — k+1 floats per
                // observation. The verifier chunks it accordingly and
                // runs the omnibus k-way ANOVA.
                //
                // Verdict reports the F-statistic as `computed_statistic`
                // and the one-sided F-p-value as `computed_p_value`. The
                // common verdict-builder is agnostic about whether the
                // statistic is t or F; "computed_statistic" is the
                // domain-neutral name.
                let (factor_levels, observations) =
                    match decode_factorial_observations(&bundle.observations_raw) {
                        Ok(p) => p,
                        Err(diag) => return Ok(verdict_fails(diag)),
                    };
                let r = match factorial_omnibus_anova(&factor_levels, &observations) {
                    Some(r) => r,
                    None => {
                        return Ok(verdict_fails(format!(
                            "Factorial ANOVA preconditions failed: need ≥ 2 cells observed and \
                         ≥ 1 within-cell df (factor_levels = {factor_levels:?}, n_obs = {})",
                            observations.len()
                        )));
                    }
                };
                (r.f_statistic, r.p_value, None)
            }
            DispatchPos::RCBD => {
                // RCBD: observations is a flat float array `[block_0,
                // treatment_0, value_0, block_1, treatment_1, value_1,
                // ...]`. The block-size argument on RCB(k) ctor in the
                // blocking axis gives n_blocks; n_treatments is read off
                // the dispatch's parallel state. Verifier runs two-way
                // ANOVA with block as random and treatment as fixed;
                // reports the treatment F-test.
                let n_blocks = match decode_rcb_block_count(&bundle.blocking_raw) {
                    Some(b) => b,
                    None => {
                        return Ok(verdict_fails(
                            "RCBD requires RCB(n_blocks) in the blocking slot with n_blocks ≥ 3 \
                         (PairedBlocking dispatches via stats:Paired)"
                                .into(),
                        ));
                    }
                };
                let observations = match decode_rcbd_observations(&bundle.observations_raw) {
                    Ok(o) => o,
                    Err(diag) => return Ok(verdict_fails(diag)),
                };
                // n_treatments is inferred from observations: total_n /
                // n_blocks must equal n_treatments and divide evenly.
                if observations.len() % n_blocks != 0 {
                    return Ok(verdict_fails(format!(
                        "RCBD observation count ({}) is not a multiple of n_blocks ({n_blocks}); \
                     each block must contain every treatment exactly once (complete design)",
                        observations.len()
                    )));
                }
                let n_treatments = observations.len() / n_blocks;
                let r = match rcbd_anova(n_blocks, n_treatments, &observations) {
                    Some(r) => r,
                    None => {
                        return Ok(verdict_fails(format!(
                            "RCBD ANOVA preconditions failed: complete design requires every \
                         (block, treatment) cell to have exactly one observation \
                         (n_blocks = {n_blocks}, n_treatments = {n_treatments}, \
                         n_obs = {})",
                            observations.len()
                        )));
                    }
                };
                (r.f_treatment, r.p_treatment, None)
            }
            DispatchPos::SplitPlot => {
                // Split-plot: observations is a flat float array
                // `[whole_plot_0, w_0, s_0, value_0, whole_plot_1, ...]`
                // — 4 floats per observation. The `SplitPlotBlocking(a, r)`
                // ctor in the blocking slot carries the whole-plot-factor
                // level count `a` and the whole-plot-replicates-per-W-level
                // count `r`. The subplot factor level count `b` is inferred
                // from `observations.len() / (a * r)`.
                //
                // The verifier produces three F-tests (W, S, W×S) with
                // nested error strata. v1 verdict reports the smallest
                // p-value across the three with a diagnostic naming which
                // effect produced it — omnibus-style "any effect
                // significant." Per-effect claim shapes (D52 §5.2's
                // false-positive shield in full) are a Phase 5 hardening.
                let (a, r) = match decode_splitplot_blocking(&bundle.blocking_raw) {
                    Some(p) => p,
                    None => {
                        return Ok(verdict_fails(
                            "SplitPlot requires SplitPlotBlocking(a, r) in the blocking slot with \
                         a ≥ 2 and r ≥ 2"
                                .into(),
                        ));
                    }
                };
                let observations = match decode_splitplot_observations(&bundle.observations_raw) {
                    Ok(o) => o,
                    Err(diag) => return Ok(verdict_fails(diag)),
                };
                let n_per_whole_plot = a.checked_mul(r).and_then(|n_wp| {
                    if n_wp == 0 || observations.len() % n_wp != 0 {
                        None
                    } else {
                        Some(observations.len() / n_wp)
                    }
                });
                let b = match n_per_whole_plot {
                    Some(b) if b >= 2 => b,
                    _ => {
                        return Ok(verdict_fails(format!(
                            "SplitPlot observation count ({}) is not a*r*b for a={a}, r={r} \
                         (subplot factor level count b must be ≥ 2 and divide evenly)",
                            observations.len()
                        )));
                    }
                };
                let res = match splitplot_anova(a, b, r, &observations) {
                    Some(r) => r,
                    None => {
                        return Ok(verdict_fails(format!(
                            "SplitPlot ANOVA preconditions failed: each whole plot must have a \
                         consistent W level and contain every S level exactly once; each W \
                         level must have exactly r={r} whole-plot replicates \
                         (a={a}, b={b}, r={r}, n_obs = {})",
                            observations.len()
                        )));
                    }
                };
                // Pick the smallest p-value across the three F-tests as
                // the verdict's primary statistic. Diagnostic names which
                // effect produced it plus the other two F-tests for
                // audit. NaN is treated as "no rejection."
                let candidates = [
                    ("whole_plot_main_effect", res.f_w, res.p_w),
                    ("subplot_main_effect", res.f_s, res.p_s),
                    ("interaction", res.f_ws, res.p_ws),
                ];
                let (effect, f_stat, p_value) = candidates
                    .iter()
                    .copied()
                    .filter(|(_, _, p)| !p.is_nan())
                    .min_by(|(_, _, p1), (_, _, p2)| {
                        p1.partial_cmp(p2).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(("all_nan", f64::NAN, f64::NAN));
                let note = format!(
                    "SplitPlot omnibus: reported statistic is `{effect}` (F = {f_stat:.4}, \
                 p = {p_value:.6}). All three F-tests: \
                 W (F = {:.4}, p = {:.6}), \
                 S (F = {:.4}, p = {:.6}), \
                 W×S (F = {:.4}, p = {:.6})",
                    res.f_w, res.p_w, res.f_s, res.p_s, res.f_ws, res.p_ws
                );
                (f_stat, p_value, Some(note))
            }
            DispatchPos::RepeatedMeasures => {
                // RepeatedMeasures: see the dispatch matrix in D52 §9
                // for the (autocorrelation × k_between_factors) cell
                // coverage. The observations slot is the wrapper
                // `[factor_levels, flat_observations]`; the verifier
                // cross-checks `factor_levels.len() ==
                // k_between_factors` from the FullFactorial(k) ctor on
                // the factor slot, then routes on the (autocorrelation,
                // k_between_factors) pair. Each unwired cell rejects
                // with a diagnostic naming the unimplemented
                // combination and the GitHub issue tracking it; that's
                // the structural alternative to phase-numbering each
                // cell as future work.
                let n_timepoints =
                    match decode_longitudinal_timepoints(&bundle.repeated_measures_raw) {
                        Some(t) => t,
                        None => {
                            return Ok(verdict_fails(
                                "RepeatedMeasures requires Longitudinal(n_timepoints) in the \
                             repeated_measures slot with n_timepoints ≥ 2"
                                    .into(),
                            ));
                        }
                    };
                let k_between = match decode_full_factorial_k(&bundle.factor_raw) {
                    Some(k) => k,
                    None => {
                        return Ok(verdict_fails(
                            "RepeatedMeasures requires FullFactorial(k_between_factors) in the \
                             factor slot with k ≥ 0 (k = 0 is the time-only RM case)"
                                .into(),
                        ));
                    }
                };
                let (factor_levels, inner_observations_raw) =
                    match decode_rm_observations_wrapped(&bundle.observations_raw) {
                        Ok(p) => p,
                        Err(diag) => return Ok(verdict_fails(diag)),
                    };
                if factor_levels.len() != k_between {
                    return Ok(verdict_fails(format!(
                        "RepeatedMeasures factor_levels.len() ({}) must equal \
                         k_between_factors ({k_between}) declared on the FullFactorial ctor",
                        factor_levels.len()
                    )));
                }
                // Read the claim's `autocorrelation_structure`; absent
                // defaults to CompoundSymmetry (the assumption a flat
                // RM-ANOVA implicitly makes).
                let autocorr = read_json_property(claim, iris::PROP_AUTOCORRELATION_STRUCTURE)?;
                let autocorr_ctor = autocorr.as_ref().and_then(json_ctor_name);
                let autocorr_name = autocorr_ctor.unwrap_or("CompoundSymmetry");
                match (autocorr_name, k_between) {
                    ("CompoundSymmetry", 0) => {
                        let observations =
                            match decode_rm_simple_observations(inner_observations_raw) {
                                Ok(o) => o,
                                Err(diag) => return Ok(verdict_fails(diag)),
                            };
                        if observations.len() % n_timepoints != 0 {
                            return Ok(verdict_fails(format!(
                                "RepeatedMeasures observation count ({}) is not a multiple of \
                                 n_timepoints ({n_timepoints}); each subject must be measured at \
                                 every timepoint exactly once (complete design)",
                                observations.len()
                            )));
                        }
                        let n_subjects = observations.len() / n_timepoints;
                        let res = match repeated_measures_cs_anova(
                            n_subjects,
                            n_timepoints,
                            &observations,
                        ) {
                            Some(r) => r,
                            None => {
                                return Ok(verdict_fails(format!(
                                    "RepeatedMeasures (CompoundSymmetry) preconditions failed: \
                                     complete design requires every (subject, timepoint) cell to \
                                     have exactly one observation (n_subjects = {n_subjects}, \
                                     n_timepoints = {n_timepoints}, n_obs = {})",
                                    observations.len()
                                )));
                            }
                        };
                        let note = format!(
                            "RepeatedMeasures (CompoundSymmetry, k_between = 0): time-effect F = \
                             {:.4}, df = ({}, {}), n_subjects = {}, n_timepoints = {}",
                            res.f_time,
                            res.df_time as usize,
                            res.df_error as usize,
                            n_subjects,
                            n_timepoints,
                        );
                        (res.f_time, res.p_time, Some(note))
                    }
                    ("CompoundSymmetry", k) => {
                        return Ok(verdict_fails(format!(
                            "RepeatedMeasures (CompoundSymmetry, k_between = {k}) not yet wired \
                             — factorial-RM needs a multi-factor fixed-effect decomposition on \
                             top of the subject random effect (factor_levels = {factor_levels:?}). \
                             Tracked in GitHub issue: factorial-RM (CompoundSymmetry covariance)."
                        )));
                    }
                    ("AR1", k) => {
                        return Ok(verdict_fails(format!(
                            "RepeatedMeasures (AR1, k_between = {k}) not yet wired — AR(1) \
                             covariance needs the ρ parameter and generalized least squares \
                             rather than the RCBD-equivalent univariate RM-ANOVA path. Tracked \
                             in GitHub issue: RM with AR(1) covariance."
                        )));
                    }
                    ("Unstructured", k) => {
                        return Ok(verdict_fails(format!(
                            "RepeatedMeasures (Unstructured, k_between = {k}) not yet wired — \
                             Unstructured covariance needs MANOVA-style multivariate tests with \
                             a free T×T within-subject covariance matrix. Tracked in GitHub \
                             issue: RM with Unstructured covariance."
                        )));
                    }
                    (other, _) => {
                        return Ok(verdict_fails(format!(
                            "unknown autocorrelation_structure ctor `{other}` (expected \
                             CompoundSymmetry / AR1 / Unstructured)"
                        )));
                    }
                }
            }
        };

    // ── Step 7: §7.4 epistemic-scope check ────────────────────────────
    //
    // Decode the derived_proposition's head predicate IRI, look up its
    // `is_a` markers, and admit/reject per the replication kind:
    //
    //   BiologicalReplication / NestedReplication — any scope ok
    //   TechnicalWithinRun                         — only MeasurementLevel
    //
    // Phase 1 implements the simple form: read the predicate's class
    // memberships and check for the marker. A predicate with no scope
    // marker defaults to PopulationLevel (the more restrictive admissibility).
    let scope_diag = check_epistemic_scope(claim, &bundle.replication, ctx)?;
    if let Some(d) = scope_diag {
        return Ok(verdict_fails(d));
    }

    // ── Step 8: compare computed statistic against asserted threshold ─
    //
    // For SingleSampleEstimate with EffectSize.Absolute(threshold) and
    // TwoSided directionality, the claim holds when p < alpha AND the
    // computed mean falls on the asserted side of the threshold (i.e.,
    // the test rejects H0 in the direction the claim asserts). For a
    // "< 100 nM" IC50 claim, the asserted side is mean < threshold.
    //
    // Phase 1 simplification: we only check p < alpha, not the
    // direction. The directional refinement lands when the §7.1
    // OneSidedWitnessed path is wired (since direction matters most
    // for one-sided claims). Two-sided rejection of "mean = threshold"
    // doesn't tell us *which* side; the author's derived_proposition
    // implicitly fixes the direction, and the verifier is honest about
    // the limited inference.
    if p_value_two_sided < alpha {
        // Holds: include the per-dispatch diagnostic note if present
        // (currently only SplitPlot uses this — to name which of its
        // three F-tests produced the reported p-value).
        let diag = diagnostic_note.as_deref();
        Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_HOLDS,
            diag,
            Some((t_statistic, p_value_two_sided)),
        )))
    } else {
        // Fails: combine the AlphaNotCrossed framing with the
        // per-dispatch note if present.
        let fail_diag = match diagnostic_note.as_deref() {
            Some(note) => format!(
                "AlphaNotCrossed: computed p = {p_value_two_sided:.6}, threshold alpha = {alpha}. {note}"
            ),
            None => format!(
                "AlphaNotCrossed: computed p = {p_value_two_sided:.6}, threshold alpha = {alpha}"
            ),
        };
        Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_FAILS,
            Some(&fail_diag),
            Some((t_statistic, p_value_two_sided)),
        )))
    }
}

// ────────────────────────────────────────────────────────────────────
// Bundle decoding (the SampleSet's `Bundle` ctor → typed struct)
// ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct DecodedBundle {
    randomization: String,
    blocking: String,
    /// Raw blocking-slot JSON. Phase 4's RCBD dispatch reads the
    /// `RCB(n_blocks)` ctor's integer arg from here; future blocking
    /// ctors with parameters (e.g., `Incomplete(block_size)`) will
    /// extract similarly.
    blocking_raw: serde_json::Value,
    factor: String,
    /// Raw factor-slot JSON. RepeatedMeasures reads
    /// `FullFactorial(k_between_factors)`'s integer arg from here to
    /// route across the (autocorrelation × k_between_factors)
    /// dispatch matrix; other factor ctors with parameters extract
    /// similarly.
    factor_raw: serde_json::Value,
    replication: ReplicationKind,
    repeated_measures: String,
    /// Raw repeated-measures-slot JSON. Phase 4.9's RepeatedMeasures
    /// dispatch reads the `Longitudinal(n_timepoints)` ctor's integer
    /// arg from here; future repeated-measures variants with extra
    /// parameters will extract similarly.
    repeated_measures_raw: serde_json::Value,
    /// D52 §5.3 / Phase 3 MAE-style biological-unit list.
    /// Empty (`Units([])`) for Tier 1 dispatches where unit identity
    /// is implicit in observation row order. Populated by Phase 4
    /// Tier 2 smart constructors when the verifier needs explicit
    /// per-observation unit identification.
    #[allow(dead_code)]
    units: Vec<String>,
    /// D52 §5.3 / Phase 3 MAE-style assay columns — flat
    /// `[assay_0, col_0, assay_1, col_1, …]` pairs. Empty for Tier 1.
    #[allow(dead_code)]
    columns: Vec<String>,
    /// D52 §5.3 / Phase 3 MAE-style sampleMap entries. Each entry is
    /// a `(assay_id, primary_iri, col_name)` triple linking a primary
    /// biological unit to a specific assay column. Empty for Tier 1.
    #[allow(dead_code)]
    sample_map: Vec<SampleMapEntry>,
    /// Raw observations slot from the Bundle ctor — each dispatch arm
    /// decodes it per its expected shape:
    ///  - SingleSampleEstimate expects a flat float array
    ///  - IID expects `[group_a, group_b]` (nested float arrays)
    ///  - Paired expects a flat interleaved `[b_0, a_0, …]` array
    ///  - Factorial expects `[factor_levels, flat_observations]`
    ///  - RCBD / SplitPlot / RepeatedMeasures will expect richer shapes
    ///    when they land
    observations_raw: serde_json::Value,
}

/// D52 §5.3 / Phase 3 — decoded `(assay_id, primary_iri, col_name)`
/// from a `SampleMapEntry` ctor. The MAE bipartite-graph element type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
struct SampleMapEntry {
    assay_id: String,
    primary_iri: String,
    col_name: String,
}

#[derive(Debug, Clone, PartialEq)]
enum ReplicationKind {
    BiologicalReplication,
    TechnicalWithinRun,
    NestedReplication {
        biological_n: i64,
        technical_per_biological: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::upper_case_acronyms)] // "IID" and "RCBD" are the standard statistics terms
enum DispatchPos {
    SingleSampleEstimate,
    IID,
    Paired,
    Factorial,
    RCBD,
    SplitPlot,
    RepeatedMeasures,
}

fn decode_bundle(j: &serde_json::Value) -> Result<DecodedBundle, String> {
    let ctor = json_ctor_name(j).unwrap_or("?");
    if ctor != "Bundle" {
        return Err(format!("expected SampleSet `Bundle` ctor, got `{ctor}`"));
    }
    let args = j["args"]
        .as_array()
        .ok_or_else(|| "Bundle args field missing or not an array".to_string())?;
    if args.len() != 9 {
        return Err(format!("Bundle expects 9 args, got {}", args.len()));
    }
    let randomization = json_ctor_name(&args[0])
        .ok_or_else(|| "randomization slot is not a ctor".to_string())?
        .to_string();
    let blocking = json_ctor_name(&args[1])
        .ok_or_else(|| "blocking slot is not a ctor".to_string())?
        .to_string();
    let blocking_raw = args[1].clone();
    let factor = json_ctor_name(&args[2])
        .ok_or_else(|| "factor slot is not a ctor".to_string())?
        .to_string();
    let factor_raw = args[2].clone();
    let replication = decode_replication_kind(&args[3])?;
    let repeated_measures = json_ctor_name(&args[4])
        .ok_or_else(|| "repeated_measures slot is not a ctor".to_string())?
        .to_string();
    let repeated_measures_raw = args[4].clone();
    let units = decode_biological_units(&args[5])?;
    let columns = decode_assay_columns(&args[6])?;
    let sample_map = decode_sample_map(&args[7])?;
    // Keep observations raw — the per-dispatch arm decodes per its
    // expected shape (flat float array for SingleSampleEstimate, nested
    // for IID, richer for Tier-2 designs).
    let observations_raw = args[8].clone();
    Ok(DecodedBundle {
        randomization,
        blocking,
        blocking_raw,
        factor,
        factor_raw,
        replication,
        repeated_measures,
        repeated_measures_raw,
        units,
        columns,
        sample_map,
        observations_raw,
    })
}

/// Decode the SingleSampleEstimate's observations payload: a flat
/// JSON array of numbers.
fn decode_flat_observations(j: &serde_json::Value) -> Result<Vec<f64>, String> {
    let arr = j
        .as_array()
        .ok_or_else(|| format!("observations slot is not an array: {j:?}"))?;
    arr.iter()
        .enumerate()
        .map(|(i, v)| {
            v.as_f64()
                .ok_or_else(|| format!("observation index {i} is not a number: {v:?}"))
        })
        .collect()
}

/// Decode the IID two-sample observations payload: `[group_a, group_b]`
/// where each group is a flat JSON array of numbers. Returns the two
/// groups as separate Vec<f64>.
fn decode_two_group_observations(j: &serde_json::Value) -> Result<(Vec<f64>, Vec<f64>), String> {
    let outer = j
        .as_array()
        .ok_or_else(|| format!("IID observations slot is not an array: {j:?}"))?;
    if outer.len() != 2 {
        return Err(format!(
            "IID expects exactly 2 groups in observations (got {})",
            outer.len()
        ));
    }
    let group_a = decode_flat_observations(&outer[0]).map_err(|e| format!("IID group A: {e}"))?;
    let group_b = decode_flat_observations(&outer[1]).map_err(|e| format!("IID group B: {e}"))?;
    Ok((group_a, group_b))
}

/// Decode the Paired observations payload: a flat float array of
/// length `2 * n_pairs`, interleaved as `[b0, a0, b1, a1, …, bn, an]`.
/// Returns the chunked `(before, after)` pairs.
fn decode_paired_observations(j: &serde_json::Value) -> Result<Vec<(f64, f64)>, String> {
    let flat = decode_flat_observations(j).map_err(|e| format!("Paired observations: {e}"))?;
    if flat.len() % 2 != 0 {
        return Err(format!(
            "Paired observations must have an even number of floats (got {} — \
             interleaved `[before_0, after_0, before_1, after_1, …]`)",
            flat.len()
        ));
    }
    Ok(flat.chunks_exact(2).map(|c| (c[0], c[1])).collect())
}

/// Per-observation entry the Factorial decoder produces: the cell
/// index (k-tuple of factor-level indices) paired with the measurement
/// value. Typed-alias kept local to validate.rs to satisfy clippy's
/// type-complexity lint on the decoder's return type.
type FactorialObservation = (Vec<usize>, f64);

/// Decode the Factorial observations payload:
/// `[factor_levels, flat_observations]` where:
/// - `factor_levels` is a flat float array `[n_0, n_1, …, n_{k-1}]`
///   giving per-factor level counts (cast to `usize`)
/// - `flat_observations` is a flat float array containing `k + 1`
///   floats per observation: `k` factor-level indices (cast to `usize`)
///   plus the measurement value
///
/// Returns `(factor_levels, observations)` where each observation is a
/// `(cell_index_tuple, value)` pair ready for
/// [`factorial_omnibus_anova`].
fn decode_factorial_observations(
    j: &serde_json::Value,
) -> Result<(Vec<usize>, Vec<FactorialObservation>), String> {
    let outer = j
        .as_array()
        .ok_or_else(|| format!("Factorial observations slot is not an array: {j:?}"))?;
    if outer.len() != 2 {
        return Err(format!(
            "Factorial expects observations = [factor_levels, flat_observations] (got {} \
             outer elements)",
            outer.len()
        ));
    }
    let factor_levels_flat =
        decode_flat_observations(&outer[0]).map_err(|e| format!("Factorial factor_levels: {e}"))?;
    let factor_levels: Vec<usize> = factor_levels_flat
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if v < 0.0 || v.fract() != 0.0 {
                Err(format!(
                    "factor_levels[{i}] must be a non-negative integer, got {v}"
                ))
            } else {
                Ok(v as usize)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let k = factor_levels.len();
    if k == 0 {
        return Err("Factorial requires at least one factor".to_string());
    }
    let flat_obs =
        decode_flat_observations(&outer[1]).map_err(|e| format!("Factorial observations: {e}"))?;
    let row_width = k + 1;
    if flat_obs.len() % row_width != 0 {
        return Err(format!(
            "Factorial observations length ({}) must be a multiple of k+1 ({row_width}) \
             — each row is [level_0, …, level_{}, value]",
            flat_obs.len(),
            k - 1
        ));
    }
    let observations: Result<Vec<FactorialObservation>, String> = flat_obs
        .chunks_exact(row_width)
        .enumerate()
        .map(|(row_idx, chunk)| {
            let mut levels = Vec::with_capacity(k);
            for (i, &v) in chunk[..k].iter().enumerate() {
                if v < 0.0 || v.fract() != 0.0 {
                    return Err(format!(
                        "observation row {row_idx} factor[{i}] level must be a non-negative \
                         integer, got {v}"
                    ));
                }
                levels.push(v as usize);
            }
            Ok((levels, chunk[k]))
        })
        .collect();
    Ok((factor_levels, observations?))
}

/// D52 Phase 4.0 — extract the `RCB(n_blocks)` integer from the
/// blocking slot. Returns `Some(n_blocks)` only when the blocking
/// ctor is `RCB`; returns `None` for `PairedBlocking` / `Unblocked`
/// / `Incomplete` / etc. (which dispatch elsewhere).
fn decode_rcb_block_count(j: &serde_json::Value) -> Option<usize> {
    if json_ctor_name(j)? != "RCB" {
        return None;
    }
    let args = j["args"].as_array()?;
    if args.len() != 1 {
        return None;
    }
    let n_i64 = args[0].as_i64()?;
    if n_i64 < 2 {
        return None;
    }
    Some(n_i64 as usize)
}

/// D52 Phase 4.0 — decode the RCBD observations payload: a flat
/// float array of `[block_0, treatment_0, value_0, block_1,
/// treatment_1, value_1, ...]` — 3 floats per observation, total
/// length `3 * n_blocks * n_treatments`. Returns the parsed
/// `(block_idx, treatment_idx, value)` tuples ready for
/// [`rcbd_anova`]; treats fractional or negative block/treatment
/// indices as decode errors (those would silently mask design
/// errors otherwise).
fn decode_rcbd_observations(j: &serde_json::Value) -> Result<Vec<(usize, usize, f64)>, String> {
    let flat = decode_flat_observations(j).map_err(|e| format!("RCBD observations: {e}"))?;
    if flat.len() % 3 != 0 {
        return Err(format!(
            "RCBD observations must have a multiple of 3 floats (got {} — \
             each row is `[block_idx, treatment_idx, value]`)",
            flat.len()
        ));
    }
    flat.chunks_exact(3)
        .enumerate()
        .map(|(row_idx, chunk)| {
            let block = chunk[0];
            let treatment = chunk[1];
            if block < 0.0 || block.fract() != 0.0 {
                return Err(format!(
                    "RCBD row {row_idx} block_idx must be a non-negative integer, got {block}"
                ));
            }
            if treatment < 0.0 || treatment.fract() != 0.0 {
                return Err(format!(
                    "RCBD row {row_idx} treatment_idx must be a non-negative integer, got {treatment}"
                ));
            }
            Ok((block as usize, treatment as usize, chunk[2]))
        })
        .collect()
}

/// D52 Phase 4.5 — extract `(a, r)` from the
/// `SplitPlotBlocking(a, r)` ctor in the blocking slot. Returns
/// `Some((a, r))` only when the blocking ctor is `SplitPlotBlocking`
/// and both args are positive integers; returns `None` otherwise
/// (the dispatch arm surfaces a clean diagnostic).
fn decode_splitplot_blocking(j: &serde_json::Value) -> Option<(usize, usize)> {
    if json_ctor_name(j)? != "SplitPlotBlocking" {
        return None;
    }
    let args = j["args"].as_array()?;
    if args.len() != 2 {
        return None;
    }
    let a = args[0].as_i64()?;
    let r = args[1].as_i64()?;
    if a < 2 || r < 2 {
        return None;
    }
    Some((a as usize, r as usize))
}

/// D52 Phase 4.9 — extract `n_timepoints` from the
/// `Longitudinal(n_timepoints)` ctor in the repeated-measures slot.
/// Returns `None` for `CrossSectional` (which dispatches elsewhere)
/// or when the arg isn't a positive integer ≥ 2.
fn decode_longitudinal_timepoints(j: &serde_json::Value) -> Option<usize> {
    if json_ctor_name(j)? != "Longitudinal" {
        return None;
    }
    let args = j["args"].as_array()?;
    if args.len() != 1 {
        return None;
    }
    let n = args[0].as_i64()?;
    if n < 2 {
        return None;
    }
    Some(n as usize)
}

/// Extract `k_between_factors` from a `FullFactorial(k)` ctor on the
/// factor slot. Returns `Some(k)` for `FullFactorial(k)` with `k ≥ 0`
/// (k=0 is the time-only RM case), `None` otherwise.
fn decode_full_factorial_k(j: &serde_json::Value) -> Option<usize> {
    if json_ctor_name(j)? != "FullFactorial" {
        return None;
    }
    let args = j["args"].as_array()?;
    if args.len() != 1 {
        return None;
    }
    let k = args[0].as_i64()?;
    if k < 0 {
        return None;
    }
    Some(k as usize)
}

/// Decode the RepeatedMeasures wrapper `[factor_levels,
/// inner_observations]` slot. Returns the parsed factor-level counts
/// and a reference to the inner observations JSON value, which the
/// matching (autocorrelation × k_between_factors) cell decoder then
/// parses per its row shape (3 floats for k=0, 3+k for k≥1).
fn decode_rm_observations_wrapped(
    j: &serde_json::Value,
) -> Result<(Vec<usize>, &serde_json::Value), String> {
    let outer = j
        .as_array()
        .ok_or_else(|| format!("RepeatedMeasures observations slot is not an array: {j:?}"))?;
    if outer.len() != 2 {
        return Err(format!(
            "RepeatedMeasures expects observations = [factor_levels, flat_observations] \
             (got {} outer elements)",
            outer.len()
        ));
    }
    let factor_levels_flat = decode_flat_observations(&outer[0])
        .map_err(|e| format!("RepeatedMeasures factor_levels: {e}"))?;
    let factor_levels: Vec<usize> = factor_levels_flat
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if v < 1.0 || v.fract() != 0.0 {
                Err(format!(
                    "RepeatedMeasures factor_levels[{i}] must be a positive integer \
                     (level count ≥ 1), got {v}"
                ))
            } else {
                Ok(v as usize)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((factor_levels, &outer[1]))
}

/// Decode the inner observations payload for the time-only RM case
/// (k_between_factors = 0): a flat float array of `[subject_0,
/// time_0, value_0, subject_1, time_1, value_1, ...]` — 3 floats per
/// observation. Returns the parsed `(subject_idx, time_idx, value)`
/// tuples ready for [`repeated_measures_cs_anova`]; fractional or
/// negative indices are decode errors.
fn decode_rm_simple_observations(
    j: &serde_json::Value,
) -> Result<Vec<(usize, usize, f64)>, String> {
    let flat = decode_flat_observations(j)
        .map_err(|e| format!("RepeatedMeasures inner observations: {e}"))?;
    if flat.len() % 3 != 0 {
        return Err(format!(
            "RepeatedMeasures (k_between = 0) inner observations must have a multiple of 3 \
             floats (got {} — each row is `[subject_idx, time_idx, value]`)",
            flat.len()
        ));
    }
    flat.chunks_exact(3)
        .enumerate()
        .map(|(row_idx, chunk)| {
            let subject = chunk[0];
            let time = chunk[1];
            if subject < 0.0 || subject.fract() != 0.0 {
                return Err(format!(
                    "RepeatedMeasures row {row_idx} subject_idx must be a non-negative integer, got {subject}"
                ));
            }
            if time < 0.0 || time.fract() != 0.0 {
                return Err(format!(
                    "RepeatedMeasures row {row_idx} time_idx must be a non-negative integer, got {time}"
                ));
            }
            Ok((subject as usize, time as usize, chunk[2]))
        })
        .collect()
}

/// D52 Phase 4.5 — decode the SplitPlot observations payload: a flat
/// float array of `[whole_plot_0, w_0, s_0, value_0, whole_plot_1,
/// w_1, s_1, value_1, ...]` — 4 floats per observation. Returns the
/// parsed `(whole_plot_idx, w_level, s_level, value)` tuples ready
/// for [`splitplot_anova`]; fractional or negative indices are decode
/// errors.
fn decode_splitplot_observations(
    j: &serde_json::Value,
) -> Result<Vec<(usize, usize, usize, f64)>, String> {
    let flat = decode_flat_observations(j).map_err(|e| format!("SplitPlot observations: {e}"))?;
    if flat.len() % 4 != 0 {
        return Err(format!(
            "SplitPlot observations must have a multiple of 4 floats (got {} — \
             each row is `[whole_plot_idx, w_level, s_level, value]`)",
            flat.len()
        ));
    }
    flat.chunks_exact(4)
        .enumerate()
        .map(|(row_idx, chunk)| {
            let wp = chunk[0];
            let w = chunk[1];
            let s = chunk[2];
            for (name, v) in [("whole_plot_idx", wp), ("w_level", w), ("s_level", s)] {
                if v < 0.0 || v.fract() != 0.0 {
                    return Err(format!(
                        "SplitPlot row {row_idx} {name} must be a non-negative integer, got {v}"
                    ));
                }
            }
            Ok((wp as usize, w as usize, s as usize, chunk[3]))
        })
        .collect()
}

/// D52 §5.3 / Phase 3 — decode `BiologicalUnits.Units(iris)` ctor
/// into a flat vector of unit-IRI strings. Empty list (`Units([])`)
/// is the Tier 1 implicit case.
fn decode_biological_units(j: &serde_json::Value) -> Result<Vec<String>, String> {
    match json_ctor_name(j) {
        Some("Units") => {
            let args = j["args"]
                .as_array()
                .ok_or_else(|| "BiologicalUnits.Units args missing".to_string())?;
            if args.len() != 1 {
                return Err(format!(
                    "BiologicalUnits.Units expects 1 arg, got {}",
                    args.len()
                ));
            }
            let arr = args[0]
                .as_array()
                .ok_or_else(|| "BiologicalUnits.Units arg 0 must be an array".to_string())?;
            arr.iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("unit_iris[{i}] must be a string"))
                })
                .collect()
        }
        Some(other) => Err(format!("expected BiologicalUnits.Units, got `{other}`")),
        None => Err("BiologicalUnits slot is not a ctor".to_string()),
    }
}

/// D52 §5.3 / Phase 3 — decode `AssayColumns.Columns(pairs)` into a
/// flat vector. The interleaved encoding `[assay_0, col_0, assay_1,
/// col_1, …]` is preserved as-is here; Phase 4's RCBD / SplitPlot
/// decoders chunk it into pairs when they need to identify columns
/// per assay. Empty list for Tier 1.
fn decode_assay_columns(j: &serde_json::Value) -> Result<Vec<String>, String> {
    match json_ctor_name(j) {
        Some("Columns") => {
            let args = j["args"]
                .as_array()
                .ok_or_else(|| "AssayColumns.Columns args missing".to_string())?;
            if args.len() != 1 {
                return Err(format!(
                    "AssayColumns.Columns expects 1 arg, got {}",
                    args.len()
                ));
            }
            let arr = args[0]
                .as_array()
                .ok_or_else(|| "AssayColumns.Columns arg 0 must be an array".to_string())?;
            arr.iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| format!("assay_columns[{i}] must be a string"))
                })
                .collect()
        }
        Some(other) => Err(format!("expected AssayColumns.Columns, got `{other}`")),
        None => Err("AssayColumns slot is not a ctor".to_string()),
    }
}

/// D52 §5.3 / Phase 3 — decode `SampleMap.Entries(entries)` into a
/// vector of `SampleMapEntry` triples. The empty-list shape is the
/// Tier 1 implicit case; Phase 4 Tier 2 dispatches populate it.
fn decode_sample_map(j: &serde_json::Value) -> Result<Vec<SampleMapEntry>, String> {
    match json_ctor_name(j) {
        Some("Entries") => {
            let args = j["args"]
                .as_array()
                .ok_or_else(|| "SampleMap.Entries args missing".to_string())?;
            if args.len() != 1 {
                return Err(format!(
                    "SampleMap.Entries expects 1 arg, got {}",
                    args.len()
                ));
            }
            let entries = args[0]
                .as_array()
                .ok_or_else(|| "SampleMap.Entries arg 0 must be an array".to_string())?;
            entries
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    decode_sample_map_entry(entry).map_err(|e| format!("entry[{i}]: {e}"))
                })
                .collect()
        }
        Some(other) => Err(format!("expected SampleMap.Entries, got `{other}`")),
        None => Err("SampleMap slot is not a ctor".to_string()),
    }
}

fn decode_sample_map_entry(j: &serde_json::Value) -> Result<SampleMapEntry, String> {
    match json_ctor_name(j) {
        Some("Entry") => {
            let args = j["args"]
                .as_array()
                .ok_or_else(|| "SampleMapEntry.Entry args missing".to_string())?;
            if args.len() != 3 {
                return Err(format!(
                    "SampleMapEntry.Entry expects 3 args (assay_id, primary_iri, col_name), got {}",
                    args.len()
                ));
            }
            let assay_id = args[0]
                .as_str()
                .ok_or_else(|| {
                    "SampleMapEntry.Entry arg 0 (assay_id) must be a string".to_string()
                })?
                .to_string();
            let primary_iri = args[1]
                .as_str()
                .ok_or_else(|| {
                    "SampleMapEntry.Entry arg 1 (primary_iri) must be a string".to_string()
                })?
                .to_string();
            let col_name = args[2]
                .as_str()
                .ok_or_else(|| {
                    "SampleMapEntry.Entry arg 2 (col_name) must be a string".to_string()
                })?
                .to_string();
            Ok(SampleMapEntry {
                assay_id,
                primary_iri,
                col_name,
            })
        }
        Some(other) => Err(format!("expected SampleMapEntry.Entry, got `{other}`")),
        None => Err("SampleMapEntry slot is not a ctor".to_string()),
    }
}

fn decode_replication_kind(j: &serde_json::Value) -> Result<ReplicationKind, String> {
    match json_ctor_name(j) {
        Some("BiologicalReplication") => Ok(ReplicationKind::BiologicalReplication),
        Some("TechnicalWithinRun") => Ok(ReplicationKind::TechnicalWithinRun),
        Some("NestedReplication") => {
            let args = j["args"]
                .as_array()
                .ok_or_else(|| "NestedReplication args missing".to_string())?;
            if args.len() != 2 {
                return Err(format!(
                    "NestedReplication expects 2 args, got {}",
                    args.len()
                ));
            }
            let biological_n = args[0]
                .as_i64()
                .ok_or_else(|| "NestedReplication arg 0 must be integer".to_string())?;
            let technical_per_biological = args[1]
                .as_i64()
                .ok_or_else(|| "NestedReplication arg 1 must be integer".to_string())?;
            Ok(ReplicationKind::NestedReplication {
                biological_n,
                technical_per_biological,
            })
        }
        Some(other) => Err(format!("unknown Replication ctor `{other}`")),
        None => Err("replication slot is not a ctor".to_string()),
    }
}

fn dispatch_product_position(bundle: &DecodedBundle) -> Option<DispatchPos> {
    // Verifier dispatch table per D52 §5.4. Phase 1 wired
    // SingleSampleEstimate; Phase 1.5 added IID; Phase 2 added Paired;
    // Phase 2.5 added Factorial; Phase 4.0 adds RCBD.
    match (
        bundle.randomization.as_str(),
        bundle.blocking.as_str(),
        bundle.factor.as_str(),
        bundle.repeated_measures.as_str(),
    ) {
        ("CompleteRandom", "Unblocked", "NoFactor", "CrossSectional") => {
            Some(DispatchPos::SingleSampleEstimate)
        }
        ("CompleteRandom", "Unblocked", "SingleFactor", "CrossSectional") => Some(DispatchPos::IID),
        ("CompleteRandom", "PairedBlocking", "SingleFactor", "CrossSectional") => {
            Some(DispatchPos::Paired)
        }
        ("CompleteRandom", "Unblocked", "FullFactorial", "CrossSectional") => {
            Some(DispatchPos::Factorial)
        }
        ("Restricted", "RCB", "SingleFactor", "CrossSectional") => Some(DispatchPos::RCBD),
        ("Restricted", "SplitPlotBlocking", "FullFactorial", "CrossSectional") => {
            Some(DispatchPos::SplitPlot)
        }
        // RepeatedMeasures lives at FullFactorial(k_between_factors) on
        // the factor slot — k=0 (time-only RM), k=1 (single-treatment
        // RM), and k≥2 (factorial-RM) all share this dispatch position;
        // the k value is decoded from the FullFactorial ctor's integer
        // arg inside the RM arm and routed against the claim's
        // autocorrelation_structure via the dispatch matrix in D52 §9.
        ("CompleteRandom", "Unblocked", "FullFactorial", "Longitudinal") => {
            Some(DispatchPos::RepeatedMeasures)
        }
        _ => None,
    }
}

// ────────────────────────────────────────────────────────────────────
// §7.4 epistemic-scope check
// ────────────────────────────────────────────────────────────────────

/// Decode the claim's `derived_proposition`, extract its head predicate
/// IRI, look up that predicate's `is_a` markers, and check
/// admissibility against the SampleSet's replication kind.
///
/// Returns `Ok(None)` if the scope is admissible; `Ok(Some(diag))` with
/// a diagnostic string if the institution must reject the claim per
/// §7.4. `Err(_)` only for genuine institutional failures (resolution
/// errors etc.), not scope mismatches.
fn check_epistemic_scope(
    claim: &Resource,
    replication: &ReplicationKind,
    ctx: &ExecutionContext,
) -> Result<Option<String>, InstitutionError> {
    // BiologicalReplication / NestedReplication admit any scope —
    // short-circuit before doing any chain lookup work.
    if !matches!(replication, ReplicationKind::TechnicalWithinRun) {
        return Ok(None);
    }
    // TechnicalWithinRun: the claim is admissible only if the
    // canonical_proposition's head predicate is marked MeasurementLevel.
    let derived_prop = match read_json_property(claim, iris::PROP_CANONICAL_PROPOSITION)? {
        Some(j) => j,
        None => {
            // No canonical_proposition — the claim's `requires` clause
            // catches this as a malformed-resource error before the
            // institution dispatches; treat here as "scope-check
            // inconclusive" since we have nothing to scope-check.
            return Ok(None);
        }
    };
    let predicate_iri = match extract_head_predicate_iri(&derived_prop) {
        Some(iri) => iri,
        None => {
            // Couldn't extract the head predicate (e.g., the prop is a
            // pure type-theoretic combinator like a Pi-arrow with no
            // ConstRef head). Default to fail-safe: reject as
            // population-level since we can't prove it isn't.
            return Ok(Some(
                "EpistemicScopeViolation: SampleSet has replication = TechnicalWithinRun, \
                 but the derived_proposition's scope could not be determined from its \
                 structure — defaulting to PopulationLevel admissibility (the more \
                 restrictive). To assert this claim from technical-only replicates, the \
                 predicate must explicitly carry `is_a stats:MeasurementLevel`."
                    .to_string(),
            ));
        }
    };
    let pred_iri_parsed = match Iri::parse(&predicate_iri) {
        Ok(i) => i,
        Err(_) => return Ok(None), // can't resolve — treat as inconclusive
    };
    let pred_resource = match ctx.resolve(&pred_iri_parsed) {
        Some(r) => r,
        None => {
            return Ok(Some(format!(
                "EpistemicScopeViolation: derived_proposition references predicate \
                 `{predicate_iri}` which is not committed on chain; cannot verify scope"
            )));
        }
    };
    let measurement_level_iri = Iri::parse(iris::MEASUREMENT_LEVEL).expect("static IRI");
    let is_measurement_level = pred_resource
        .is_a()
        .iter()
        .any(|c| c == &measurement_level_iri);
    if is_measurement_level {
        Ok(None)
    } else {
        Ok(Some(format!(
            "EpistemicScopeViolation: SampleSet has replication = TechnicalWithinRun, \
             but derived_proposition's predicate `{predicate_iri}` is not marked \
             `is_a stats:MeasurementLevel`. Technical-only replicates cannot support \
             population-level propositions (D52 §7.4). Either gather biological \
             replicates and recommit the SampleSet, or assert against a measurement-\
             scope predicate (e.g., `HasLowIC50_OnThisBatch`)."
        )))
    }
}

/// Extract the head predicate's IRI from a D47-encoded proposition.
/// The shape for a typical predicate application like `HasLowIC50(iri)`
/// is `App(ConstRef(HasLowIC50), LitString(iri))` — we walk the App
/// spine to the leftmost ConstRef and return its IRI. Returns `None`
/// for shapes that don't bottom out at a ConstRef (Pi-arrows, Sort
/// literals, etc. — those don't have a "predicate" to scope-check).
fn extract_head_predicate_iri(j: &serde_json::Value) -> Option<String> {
    let mut cursor = j;
    loop {
        match json_ctor_name(cursor)? {
            "App" => {
                cursor = cursor["args"].get(0)?;
            }
            "ConstRef" => {
                return cursor["args"].get(0)?.as_str().map(|s| s.to_string());
            }
            _ => return None,
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────

fn json_ctor_name(j: &serde_json::Value) -> Option<&str> {
    j.as_object()?.get("ctor")?.as_str()
}

fn parse_effect_size_absolute(j: &serde_json::Value) -> Option<(f64, String)> {
    if json_ctor_name(j)? != "Absolute" {
        return None;
    }
    let args = j["args"].as_array()?;
    if args.len() != 2 {
        return None;
    }
    let magnitude = args[0].as_f64()?;
    let units = args[1].as_str()?.to_string();
    Some((magnitude, units))
}

fn read_iri_property(claim: &Resource, prop_iri: &str) -> Result<Option<String>, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    match claim.get(&iri) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::ResourceRef(i)) => Ok(Some(i.as_str().to_string())),
        Some(other) => Err(InstitutionError::ComputationFailed(format!(
            "MeasurementClaim `{prop_iri}` is not a string/IRI: {other:?}"
        ))),
        None => Ok(None),
    }
}

fn read_float_property(claim: &Resource, prop_iri: &str) -> Result<Option<f64>, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    match claim.get(&iri) {
        Some(Value::Float(f)) => Ok(Some(*f)),
        Some(Value::Integer(n)) => Ok(Some(*n as f64)),
        Some(other) => Err(InstitutionError::ComputationFailed(format!(
            "MeasurementClaim `{prop_iri}` is not a number: {other:?}"
        ))),
        None => Ok(None),
    }
}

fn read_json_property(
    claim: &Resource,
    prop_iri: &str,
) -> Result<Option<serde_json::Value>, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    match claim.get(&iri) {
        Some(Value::Json(j)) => Ok(Some(j.clone())),
        Some(other) => Err(InstitutionError::ComputationFailed(format!(
            "MeasurementClaim `{prop_iri}` is not a chain-inductive value: {other:?}"
        ))),
        None => Ok(None),
    }
}

/// Build a Fails verdict carrying a diagnostic string. No computed
/// numerics (the failure happened before we ran the test).
fn verdict_fails(diagnostic: String) -> QueryOutcome {
    QueryOutcome::from_output(verdict_resource(wk::VERDICT_FAILS, Some(&diagnostic), None))
}

/// Build the Verdict::Holds | Fails resource shape the kernel's commit
/// pipeline expects. On Holds (and on Fails where the test actually
/// ran), the numerics are attached so downstream consumers see the
/// computed statistic + p-value alongside the outcome (D52 §6 — verdict
/// carries the full intermediate state for audit).
fn verdict_resource(
    ctor_name: &str,
    diagnostic: Option<&str>,
    numerics: Option<(f64, f64)>,
) -> Resource {
    const DIAGNOSTIC_IRI: &str = "urn:eigenius:institution:diagnostic";
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::VERDICT).expect("well-known IRI"),
        )]),
    );
    r.set(
        Iri::parse(wk::CTOR_NAME).expect("well-known IRI"),
        Value::String(ctor_name.to_string()),
    );
    if let Some(d) = diagnostic {
        r.set(
            Iri::parse(DIAGNOSTIC_IRI).expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    if let Some((t, p)) = numerics {
        r.set(
            Iri::parse(iris::PROP_COMPUTED_STATISTIC).expect("static IRI"),
            Value::Float(t),
        );
        r.set(
            Iri::parse(iris::PROP_COMPUTED_P_VALUE).expect("static IRI"),
            Value::Float(p),
        );
    }
    r
}
