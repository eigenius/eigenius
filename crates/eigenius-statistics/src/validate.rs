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

//! `ValidateStatisticalAnalysisPlan` handler (D52 §6).
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
    classification_metrics, crossed_two_way_anova, esd_filter, nested_group_anova,
    one_sample_t_test, paired_t_test, passing_bablok_regression, rcbd_anova,
    repeated_measures_cs_anova, spearman_correlation, splitplot_anova, two_sample_t_test,
    wilcoxon_rank_sum, TwoSampleVariance,
};

// File-backed SampleSet observations (D53 §6.1).
const PROP_OBSERVATIONS_SOURCE: &str = "urn:eigenius:measurements:observations_source";
const PROP_OBSERVATIONS_COLUMN: &str = "urn:eigenius:measurements:observations_column";
const INGEST_REFERENCE: &str = "urn:eigenius:ingest:reference";
const INGEST_CONTENT_HASH: &str = "urn:eigenius:ingest:content_hash";
const INGEST_MEDIA_TYPE: &str = "urn:eigenius:ingest:media_type";

/// Resolve a flat (single-array) SampleSet's observations. If the
/// SampleSetResource declares `observations_source` (a `PinnedExternalFile`
/// IRI) + `observations_column`, read the content-verified column from the
/// materialized file (D53 §6.1 native-over-file); otherwise decode the inline
/// observations slot. Either way the recompute over the returned array is the
/// same deterministic Rust — native grade is set by the method, not the storage
/// (D53 §6).
fn resolve_flat_observations(
    sample_set_res: &Resource,
    bundle: &DecodedBundle,
    store: &eigenius_kernel::storage::content_array::ContentArrayStore,
    ctx: &ExecutionContext,
) -> Result<Vec<f64>, String> {
    let source =
        read_iri_property(sample_set_res, PROP_OBSERVATIONS_SOURCE).map_err(|e| e.to_string())?;
    let Some(source_iri) = source else {
        // Inline observations (the default / existing path).
        return decode_flat_observations(&bundle.observations_raw);
    };

    let column = read_iri_property(sample_set_res, PROP_OBSERVATIONS_COLUMN)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "observations_source set but observations_column missing (D53 §6.1)".to_string()
        })?;
    let file_iri = Iri::parse(&source_iri)
        .map_err(|e| format!("observations_source `{source_iri}` is not a valid IRI: {e}"))?;
    let file_res = ctx
        .resolve(&file_iri)
        .ok_or_else(|| format!("PinnedExternalFile `{source_iri}` not found on chain"))?;

    let reference = read_iri_property(&file_res, INGEST_REFERENCE)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("PinnedExternalFile `{source_iri}` missing ingest:reference"))?;
    let content_hash = read_iri_property(&file_res, INGEST_CONTENT_HASH)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("PinnedExternalFile `{source_iri}` missing ingest:content_hash"))?;
    let media_type = read_iri_property(&file_res, INGEST_MEDIA_TYPE)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "text/csv".to_string());

    store
        .read_column(&reference, &content_hash, &media_type, &column)
        .map_err(|e| format!("file-backed observations ({source_iri}): {e}"))
}

/// Top-level handler called by `StatisticsInstitution::query`.
pub fn do_validate_analysis_plan(
    inst: &StatisticsInstitution,
    claim: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    // ── Step 1: read sample_set IRI from the claim ────────────────────
    let sample_set_iri_str = match read_iri_property(claim, iris::PROP_SAMPLE_SET)? {
        Some(s) => s,
        None => {
            return Ok(gate_fails(
                "StatisticalAnalysisPlan missing required `sample_set` property".into(),
            ));
        }
    };
    let sample_set_iri = match Iri::parse(&sample_set_iri_str) {
        Ok(i) => i,
        Err(e) => {
            return Ok(gate_fails(format!(
                "StatisticalAnalysisPlan's sample_set value `{sample_set_iri_str}` is not a valid IRI: {e}"
            )));
        }
    };

    // ── Step 2: resolve the SampleSetResource and its inductive value ─
    let sample_set_res = match ctx.resolve(&sample_set_iri) {
        Some(r) => r,
        None => {
            return Ok(gate_fails(format!(
                "SampleSetResource `{sample_set_iri}` not found on chain"
            )));
        }
    };
    let sample_set_value_iri =
        Iri::parse("urn:eigenius:measurements:sample_set_value").expect("static IRI");
    let bundle_value = match sample_set_res.get(&sample_set_value_iri) {
        Some(v) => v,
        None => {
            return Ok(gate_fails(format!(
                "SampleSetResource `{sample_set_iri}` missing required \
                 `sample_set_value` property"
            )));
        }
    };
    let bundle_json = match bundle_value {
        Value::Json(j) => j,
        other => {
            return Ok(gate_fails(format!(
                "SampleSetResource `{sample_set_iri}`'s sample_set_value is not a chain \
                 inductive value (got {other:?})"
            )));
        }
    };

    // ── Step 3: decode the Bundle ctor's args ─────────────────────────
    let bundle = match decode_bundle(bundle_json) {
        Ok(b) => b,
        Err(diag) => return Ok(gate_fails(diag)),
    };

    // ── §7.3 class-based early dispatch: MethodComparisonAnalysisPlan ────────
    //
    // When the claim's is_a list contains stats:MethodComparisonAnalysisPlan,
    // skip the SampleSet-shape dispatch and route to Passing-Bablok
    // regression. OLS regression is rejected for method comparison —
    // it assumes zero measurement error on the X-axis, which is
    // structurally false for two biological measurements compared
    // against each other (CLSI EP09).
    let method_comparison_iri =
        Iri::parse(iris::METHOD_COMPARISON_ANALYSIS_PLAN).expect("static IRI");
    if claim.is_instance_of(&method_comparison_iri) {
        return recompute_method_comparison_claim(claim, &bundle, ctx);
    }

    // ── §2.2 class-based early dispatch: ClassificationAnalysisPlan ──────────
    //
    // A threshold classifier's PPV/sensitivity is a deterministic count
    // over a two-group SampleSet, not an inferential test — it has no
    // alpha / effect_size / directionality. Route it to its own emitter
    // before the hypothesis-test parameter reads below.
    let classification_iri = Iri::parse(iris::CLASSIFICATION_ANALYSIS_PLAN).expect("static IRI");
    if claim.is_instance_of(&classification_iri) {
        return recompute_classification_quality_claim(claim, &bundle, &sample_set_iri_str);
    }

    // ── §2.2 SampleSet-based early dispatch: nested two-way ANOVA ────────────
    //
    // A nested fixed-effects two-way ANOVA (`value ~ group + subgroup`): the
    // binary group effect tested against the within-subgroup residual, with
    // subgroup a nuisance nested in group. Expressed as a `stats:Nested(...)`
    // SampleSet — the blocking ctor is `NestedBlocking` and the observations
    // wrapper carries `[group_a, group_b, subgroup_sizes_a, subgroup_sizes_b]`.
    // The plan is a plain StatisticalAnalysisPlan, so dispatch keys on the
    // SampleSet's blocking ctor (not the plan's is_a). Has its own dispatch +
    // directionality handling, so route here before the standard reads.
    if bundle.blocking == "NestedBlocking" {
        return recompute_nested_anova_claim(claim, &bundle, &sample_set_iri_str, ctx);
    }

    // ── §2.2 SampleSet-based early dispatch: crossed two-way ANOVA ───────────
    //
    // A crossed additive two-way ANOVA (`value ~ group + block`): the binary
    // group effect tested against the additive-model residual, with `block`
    // CROSSED (the same block levels appear in both groups, unlike the nested
    // case). Expressed as a `stats:Crossed(...)` SampleSet — blocking ctor
    // `CrossedBlocking`, observations wrapper `[group_a, group_b,
    // subgroup_sizes_a, subgroup_sizes_b]` with the two size arrays paired by
    // index. Routes to its own emitter before the standard reads.
    if bundle.blocking == "CrossedBlocking" {
        return recompute_crossed_anova_claim(claim, &bundle, &sample_set_iri_str, ctx);
    }

    // ── Step 4: dispatch on the product position ──────────────────────
    let dispatch = match dispatch_product_position(&bundle) {
        Some(d) => d,
        None => {
            return Ok(gate_fails(format!(
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
        None => return Ok(gate_fails("claim missing `alpha`".into())),
    };
    let directionality = match read_json_property(claim, iris::PROP_DIRECTIONALITY)? {
        Some(j) => j,
        None => return Ok(gate_fails("claim missing `directionality`".into())),
    };
    let effect_size = match read_json_property(claim, iris::PROP_EFFECT_SIZE)? {
        Some(j) => j,
        None => return Ok(gate_fails("claim missing `effect_size`".into())),
    };
    // §7.1 directionality routing. TwoSided proceeds with the standard
    // two-sided p-value path. OneSidedWitnessed(witness_iri) requires:
    //   (a) the dispatch produces a signed test statistic (t-based
    //       dispatches only — F-based ANOVA omnibus tests reject);
    //   (b) the witness IRI resolves to a chain resource marked
    //       `is_a stats:ImpossibilityWitness` (the §7.1 ARRIVE-aligned
    //       proof-of-inverse-direction-impossibility surface).
    // When both gates pass, the verifier halves the two-sided p-value
    // for the alpha comparison; the witness's structural existence is
    // what justifies the halving, not the test statistic's sign.
    let directionality_ctor = json_ctor_name(&directionality);
    let one_sided_witnessed = match directionality_ctor {
        Some("TwoSided") => false,
        Some("OneSidedWitnessed") => {
            if !dispatch.supports_one_sided_directionality() {
                return Ok(gate_fails(format!(
                    "directionality = OneSidedWitnessed is incompatible with the {dispatch:?} \
                     dispatch — F-based omnibus ANOVA tests produce intrinsically non-negative \
                     statistics and the one-sided/two-sided distinction does not refine them \
                     (D52 §7.1). Use TwoSided directionality, or assert per-effect t-tests when \
                     the per-effect verdict shape lands."
                )));
            }
            let witness_iri_str = match directionality["args"]
                .get(0)
                .and_then(serde_json::Value::as_str)
            {
                Some(s) => s.to_string(),
                None => {
                    return Ok(gate_fails(
                        "directionality = OneSidedWitnessed requires a witness IRI as its first \
                         argument (D52 §7.1)"
                            .into(),
                    ));
                }
            };
            if let Some(diag) = check_impossibility_witness(&witness_iri_str, ctx)? {
                return Ok(gate_fails(diag));
            }
            true
        }
        other => {
            return Ok(gate_fails(format!(
                "unknown directionality ctor `{other:?}` (expected TwoSided / OneSidedWitnessed)"
            )));
        }
    };

    // Read variance_assumption for the IID dispatch arm (one-sample
    // dispatch ignores it — there's only one variance parameter to
    // estimate there).
    let variance_assumption = read_json_property(claim, iris::PROP_VARIANCE_ASSUMPTION)?;

    // §7.2 outlier-exclusion dispatch matrix. Phase 5 v1 wires the
    // `(SingleSampleEstimate, ESD(k, alpha))` cell — the cell that
    // matches the running IC50 example and exercises the dual-verdict
    // commit shape end-to-end. All other (dispatch × non-Identity-
    // exclusion) combinations reject up front with a diagnostic
    // referencing the tracked follow-on. Identity exclusion takes the
    // standard single-verdict path on any dispatch.
    let outlier_exclusion = read_json_property(claim, iris::PROP_OUTLIER_EXCLUSION)?;
    let exclusion_ctor = outlier_exclusion
        .as_ref()
        .and_then(json_ctor_name)
        .unwrap_or("Identity");
    let esd_params: Option<(usize, f64)> = match (dispatch, exclusion_ctor) {
        (_, "Identity") => None,
        (DispatchPos::SingleSampleEstimate, "ESD") => {
            let args = outlier_exclusion
                .as_ref()
                .and_then(|j| j["args"].as_array());
            let (k, alpha_esd) = match args {
                Some(a) if a.len() == 2 => {
                    let k = a[0].as_f64();
                    let alpha = a[1].as_f64();
                    match (k, alpha) {
                        (Some(k), Some(alpha)) if k.fract() == 0.0 && k >= 0.0 => {
                            (k as usize, alpha)
                        }
                        _ => {
                            return Ok(gate_fails(format!(
                                "outlier_exclusion = ESD requires (max_outliers : integer, \
                                 alpha : float); got args = {a:?}"
                            )));
                        }
                    }
                }
                other => {
                    return Ok(gate_fails(format!(
                        "outlier_exclusion = ESD requires exactly 2 args (max_outliers, alpha); \
                         got args = {other:?}"
                    )));
                }
            };
            Some((k, alpha_esd))
        }
        (DispatchPos::SingleSampleEstimate, "PassingBablokResidual") => {
            return Ok(gate_fails(
                "outlier_exclusion = PassingBablokResidual is meaningful only on method-\
                 comparison data and is not wired for the SingleSampleEstimate dispatch \
                 (D52 §7.2 / §7.3 — use MethodComparisonAnalysisPlan for that path)"
                    .into(),
            ));
        }
        (DispatchPos::SingleSampleEstimate, "Manual") => {
            return Ok(gate_fails(
                "outlier_exclusion = Manual requires §11 assay-quality observation \
                 institutions to validate each excluded unit's typed quality-check witness; \
                 those institutions have not landed yet (D52 §7.2 deferral)"
                    .into(),
            ));
        }
        (_, "ESD") | (_, "PassingBablokResidual") | (_, "Manual") => {
            return Ok(gate_fails(format!(
                "outlier_exclusion = `{exclusion_ctor}` is not yet wired for the {dispatch:?} \
                 dispatch (D52 §7.2 Phase 5 v1 wires only the SingleSampleEstimate + ESD cell; \
                 other (dispatch, exclusion) cells are tracked as follow-on GitHub issues). \
                 Use outlier_exclusion = Identity for this dispatch."
            )));
        }
        (_, other) => {
            return Ok(gate_fails(format!(
                "unknown outlier_exclusion ctor `{other}` (expected Identity / ESD / \
                 PassingBablokResidual / Manual)"
            )));
        }
    };

    // ── Step 5.5: multi-effect dispatches short-circuit out ───────────
    //
    // Factorial / SplitPlot / RepeatedMeasures designs decompose into
    // multiple effects, each with its own per-effect F-test and per-
    // effect StatisticalAnalysisResult derivation. They don't fit the single-
    // `(t, p, diag)` reduction below; route them through dedicated
    // multi-result emitters. Each emitter handles its own canonical-
    // proposition derivation per effect and returns a gate-Holds
    // outcome carrying N derivations (one per effect).
    if matches!(dispatch, DispatchPos::Factorial) {
        return do_factorial_per_effect(claim, &bundle, alpha, &effect_size);
    }
    if matches!(dispatch, DispatchPos::SplitPlot) {
        return do_splitplot_per_effect(claim, &bundle, alpha);
    }

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
                        return Ok(gate_fails(
                            "Phase 1 only supports EffectSize.Absolute(magnitude, units); \
                         StandardizedCohensD/HedgesG and Relative not yet wired"
                                .into(),
                        ));
                    }
                };
                // D53 §6.1: observations may be inline or read from a
                // content-verified PinnedExternalFile column (native-over-file).
                let samples = match resolve_flat_observations(
                    &sample_set_res,
                    &bundle,
                    inst.content_store(),
                    ctx,
                ) {
                    Ok(s) => s,
                    Err(diag) => return Ok(gate_fails(diag)),
                };
                if let Some((max_outliers, alpha_esd)) = esd_params {
                    // §7.2 dual-verdict: compute the test twice — once
                    // with the ESD filter applied (the verdict the
                    // claim's exclusion functor asserts), once on the
                    // raw samples (the Identity comparator). Primary
                    // numerics are with-exclusion; the diagnostic
                    // carries the comparator numerics + excluded
                    // indices for audit visibility into both branches.
                    let excluded = esd_filter(&samples, max_outliers, alpha_esd);
                    let filtered: Vec<f64> = samples
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !excluded.contains(i))
                        .map(|(_, &x)| x)
                        .collect();
                    let r_with = match one_sample_t_test(&filtered, magnitude) {
                        Some(r) => r,
                        None => {
                            return Ok(gate_fails(format!(
                                "InsufficientReplication: ESD-filtered one-sample t-test \
                                 requires n_filtered >= 2 (got n_raw = {}, excluded = {}, \
                                 n_filtered = {})",
                                samples.len(),
                                excluded.len(),
                                filtered.len(),
                            )));
                        }
                    };
                    let r_without = match one_sample_t_test(&samples, magnitude) {
                        Some(r) => r,
                        None => {
                            return Ok(gate_fails(format!(
                                "InsufficientReplication: comparator one-sample t-test \
                                 requires n_raw >= 2, got n = {}",
                                samples.len()
                            )));
                        }
                    };
                    let note = format!(
                        "DualVerdict (ESD max_outliers = {}, alpha = {:.4}): with-exclusion \
                         t = {:.4}, p = {:.6} (n = {} after excluding {} index{} {:?}); \
                         without-exclusion t = {:.4}, p = {:.6} (n = {})",
                        max_outliers,
                        alpha_esd,
                        r_with.t_statistic,
                        r_with.p_value_two_sided,
                        filtered.len(),
                        excluded.len(),
                        if excluded.len() == 1 { "" } else { "es" },
                        excluded,
                        r_without.t_statistic,
                        r_without.p_value_two_sided,
                        samples.len(),
                    );
                    (r_with.t_statistic, r_with.p_value_two_sided, Some(note))
                } else {
                    let r = match one_sample_t_test(&samples, magnitude) {
                        Some(r) => r,
                        None => {
                            return Ok(gate_fails(format!(
                                "InsufficientReplication: one-sample t-test requires n >= 2, \
                                 got n = {}",
                                samples.len()
                            )));
                        }
                    };
                    (r.t_statistic, r.p_value_two_sided, None)
                }
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
                        Err(diag) => return Ok(gate_fails(diag)),
                    };
                match variance_assumption.as_ref().and_then(json_ctor_name) {
                    // Rank-based / distribution-free two-sample: the
                    // Wilcoxon rank-sum (Mann–Whitney U) test. Reported
                    // statistic is the normal-approximation z; the note
                    // carries U + the group medians for audit. Same H1
                    // shape as the t-test (group A vs group B central
                    // tendency), so Step 6.5 derives the same two-sample
                    // proposition for it.
                    Some("RankBased") | Some("NonParametric") => {
                        let r = match wilcoxon_rank_sum(&group_a, &group_b) {
                            Some(r) => r,
                            None => {
                                return Ok(gate_fails(
                                    "InsufficientReplication: Wilcoxon rank-sum requires \
                                     non-empty groups"
                                        .into(),
                                ));
                            }
                        };
                        let note = format!(
                            "Wilcoxon rank-sum (Mann–Whitney): U = {:.1}, z = {:.4}, \
                             n_a = {}, n_b = {}, median_a = {:.4}, median_b = {:.4}",
                            r.u_statistic, r.z_statistic, r.n_a, r.n_b, r.median_a, r.median_b
                        );
                        (r.z_statistic, r.p_value_two_sided, Some(note))
                    }
                    other => {
                        let variance = match other {
                            Some("Pooled") => TwoSampleVariance::Pooled,
                            Some("WelchUnequal") | None => TwoSampleVariance::WelchUnequal,
                            Some(o) => {
                                return Ok(gate_fails(format!(
                                    "IID two-sample with variance_assumption `{o}` not recognised \
                                     (expected Pooled / WelchUnequal / RankBased / NonParametric)"
                                )));
                            }
                        };
                        let r = match two_sample_t_test(&group_a, &group_b, variance) {
                            Some(r) => r,
                            None => {
                                return Ok(gate_fails(format!(
                                    "InsufficientReplication: two-sample t-test requires n >= 2 \
                                     in each group, got n_a = {}, n_b = {}",
                                    group_a.len(),
                                    group_b.len()
                                )));
                            }
                        };
                        (r.t_statistic, r.p_value_two_sided, None)
                    }
                }
            }
            DispatchPos::Paired => {
                // Paired: observations is a flat array `[b0, a0, b1, a1,
                // ..., bn, an]` of before/after pairs interleaved. Chunk
                // into (before, after) tuples and run the paired t-test
                // (= one-sample t-test on the per-pair differences vs 0).
                let pairs = match decode_paired_observations(&bundle.observations_raw) {
                    Ok(p) => p,
                    Err(diag) => return Ok(gate_fails(diag)),
                };
                match variance_assumption.as_ref().and_then(json_ctor_name) {
                    // Rank-based bivariate: Spearman rank correlation. The
                    // Paired pairs are read as (x, y) observations; the
                    // reported statistic is the t-approximation, the note
                    // carries rho.
                    Some("RankBased") | Some("NonParametric") => {
                        let r = match spearman_correlation(&pairs) {
                            Some(r) => r,
                            None => {
                                return Ok(gate_fails(format!(
                                    "InsufficientReplication: Spearman correlation requires \
                                     n_pairs >= 3, got {}",
                                    pairs.len()
                                )));
                            }
                        };
                        let note = format!(
                            "Spearman rank correlation: rho = {:.4}, t = {:.4}, n_pairs = {}",
                            r.rho, r.t_statistic, r.n_pairs
                        );
                        (r.t_statistic, r.p_value_two_sided, Some(note))
                    }
                    _ => {
                        let r = match paired_t_test(&pairs) {
                            Some(r) => r,
                            None => {
                                return Ok(gate_fails(format!(
                                    "InsufficientReplication: paired t-test requires \
                                     n_pairs >= 2, got {}",
                                    pairs.len()
                                )));
                            }
                        };
                        (r.t_statistic, r.p_value_two_sided, None)
                    }
                }
            }
            DispatchPos::Factorial => {
                // Unreachable: the Factorial dispatch short-circuits at
                // Step 5.5 (above the match) via `do_factorial_per_effect`
                // because it emits one StatisticalAnalysisResult per effect
                // (2^k - 1 derivations) rather than fitting the single
                // (t, p, diag) reduction this match produces.
                unreachable!("Factorial dispatch handled by do_factorial_per_effect");
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
                        return Ok(gate_fails(
                            "RCBD requires RCB(n_blocks) in the blocking slot with n_blocks ≥ 3 \
                         (PairedBlocking dispatches via stats:Paired)"
                                .into(),
                        ));
                    }
                };
                let observations = match decode_rcbd_observations(&bundle.observations_raw) {
                    Ok(o) => o,
                    Err(diag) => return Ok(gate_fails(diag)),
                };
                // n_treatments is inferred from observations: total_n /
                // n_blocks must equal n_treatments and divide evenly.
                if observations.len() % n_blocks != 0 {
                    return Ok(gate_fails(format!(
                        "RCBD observation count ({}) is not a multiple of n_blocks ({n_blocks}); \
                     each block must contain every treatment exactly once (complete design)",
                        observations.len()
                    )));
                }
                let n_treatments = observations.len() / n_blocks;
                let r = match rcbd_anova(n_blocks, n_treatments, &observations) {
                    Some(r) => r,
                    None => {
                        return Ok(gate_fails(format!(
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
                // Unreachable: the SplitPlot dispatch short-circuits at
                // Step 5.5 (above the match) via `do_splitplot_per_effect`
                // because it emits one StatisticalAnalysisResult per effect
                // (W, S, W×S) rather than the single (t, p, diag)
                // reduction this match produces.
                unreachable!("SplitPlot dispatch handled by do_splitplot_per_effect");
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
                            return Ok(gate_fails(
                                "RepeatedMeasures requires Longitudinal(n_timepoints) in the \
                             repeated_measures slot with n_timepoints ≥ 2"
                                    .into(),
                            ));
                        }
                    };
                let k_between = match decode_full_factorial_k(&bundle.factor_raw) {
                    Some(k) => k,
                    None => {
                        return Ok(gate_fails(
                            "RepeatedMeasures requires FullFactorial(k_between_factors) in the \
                             factor slot with k ≥ 0 (k = 0 is the time-only RM case)"
                                .into(),
                        ));
                    }
                };
                let (factor_levels, inner_observations_raw) =
                    match decode_rm_observations_wrapped(&bundle.observations_raw) {
                        Ok(p) => p,
                        Err(diag) => return Ok(gate_fails(diag)),
                    };
                if factor_levels.len() != k_between {
                    return Ok(gate_fails(format!(
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
                                Err(diag) => return Ok(gate_fails(diag)),
                            };
                        if observations.len() % n_timepoints != 0 {
                            return Ok(gate_fails(format!(
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
                                return Ok(gate_fails(format!(
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
                        return Ok(gate_fails(format!(
                            "RepeatedMeasures (CompoundSymmetry, k_between = {k}) not yet wired \
                             — factorial-RM needs a multi-factor fixed-effect decomposition on \
                             top of the subject random effect (factor_levels = {factor_levels:?}). \
                             Tracked in GitHub issue: factorial-RM (CompoundSymmetry covariance)."
                        )));
                    }
                    ("AR1", k) => {
                        return Ok(gate_fails(format!(
                            "RepeatedMeasures (AR1, k_between = {k}) not yet wired — AR(1) \
                             covariance needs the ρ parameter and generalized least squares \
                             rather than the RCBD-equivalent univariate RM-ANOVA path. Tracked \
                             in GitHub issue: RM with AR(1) covariance."
                        )));
                    }
                    ("Unstructured", k) => {
                        return Ok(gate_fails(format!(
                            "RepeatedMeasures (Unstructured, k_between = {k}) not yet wired — \
                             Unstructured covariance needs MANOVA-style multivariate tests with \
                             a free T×T within-subject covariance matrix. Tracked in GitHub \
                             issue: RM with Unstructured covariance."
                        )));
                    }
                    (other, _) => {
                        return Ok(gate_fails(format!(
                            "unknown autocorrelation_structure ctor `{other}` (expected \
                             CompoundSymmetry / AR1 / Unstructured)"
                        )));
                    }
                }
            }
        };

    // ── Step 6.5: derive the canonical proposition ────────────────────
    //
    // The verifier — not the author — constructs the canonical
    // proposition the verdict attests. The author supplies the
    // statistical parameters (effect_size, directionality, dispatch
    // position); the verifier derives the Prop expression from those
    // parameters via the §3 parameter-symbol axioms (stats:mean_of,
    // stats:lt, stats:False, ...). One source of truth: the alternative
    // hypothesis IS the canonical proposition, derivable deterministically
    // from (dispatch, effect_size, directionality). When the derivation
    // shape isn't yet wired for a dispatch arm, this returns `None` —
    // the verdict skips the canonical_proposition slot and the D49
    // witness emitter won't admit `IsDerivedAs` against it, which is
    // the correct fail-closed behaviour until those arms' derivations
    // land.
    let derived_proposition: Option<serde_json::Value> = match dispatch {
        DispatchPos::SingleSampleEstimate => derive_canonical_proposition_singlesample(
            &sample_set_iri_str,
            &effect_size,
            &directionality,
        ),
        DispatchPos::IID => {
            derive_canonical_proposition_twosample(&sample_set_iri_str, &directionality)
        }
        DispatchPos::Paired => match variance_assumption.as_ref().and_then(json_ctor_name) {
            // RankBased Paired = Spearman correlation; derive the rho prop.
            // (The paired t-test's difference proposition is a follow-on.)
            Some("RankBased") | Some("NonParametric") => {
                derive_canonical_proposition_correlation(&sample_set_iri_str, &directionality)
            }
            _ => None,
        },
        _ => None,
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
    let scope_diag = check_epistemic_scope(derived_proposition.as_ref(), &bundle.replication, ctx)?;
    if let Some(d) = scope_diag {
        return Ok(gate_fails(d));
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
    // direction. The directional refinement lands when richer claim
    // shapes carry explicit signed effect-size assertions; two-sided
    // rejection of "mean = threshold" doesn't tell us *which* side, and
    // the author's derived_proposition implicitly fixes the direction.
    //
    // §7.1 Phase 5 hardening: when directionality = OneSidedWitnessed
    // (and the dispatch is t-based and the witness validated above),
    // halve the two-sided p-value for the alpha comparison. The
    // witness's existence on chain is what authorizes the halving;
    // chain-resident proof-of-inverse-direction-impossibility replaces
    // the silent one-sided-by-default of legacy software.
    let p_value_for_alpha = if one_sided_witnessed {
        p_value_two_sided / 2.0
    } else {
        p_value_two_sided
    };
    // The verdict's `computed_p_value` reports the p-value the alpha
    // decision was made against — the halved one-sided value when
    // OneSidedWitnessed is in force, the raw two-sided value otherwise.
    // Per-dispatch diagnostic notes and the one-sided derivation note
    // are concatenated for the human-readable diagnostic field.
    let one_sided_note = if one_sided_witnessed {
        Some(format!(
            "OneSidedWitnessed: alpha comparison used p_one_sided = {p_value_for_alpha:.6} \
             (= p_two_sided / 2; raw two-sided p = {p_value_two_sided:.6})"
        ))
    } else {
        None
    };
    let combined_diag = match (diagnostic_note.as_deref(), one_sided_note.as_deref()) {
        (Some(a), Some(b)) => Some(format!("{a}. {b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    };
    // The SAP itself ran (well-formed parameters, dispatch matched,
    // test executed) — gate Holds regardless of whether the test
    // rejected H0. The per-effect statistical decision lives on the
    // StatisticalAnalysisResult derivation. `result_ctor` reflects the per-
    // effect Holds/Fails under the SAP's alpha; `canonical_proposition`
    // attaches only on per-effect Holds (the chain attests a positive
    // statistical claim) — a per-effect Fails carries no
    // canonical_proposition, matching the D49 witness emitter's
    // structural filter.
    let test_rejected = p_value_for_alpha < alpha;
    let result_ctor = if test_rejected {
        wk::VERDICT_HOLDS
    } else {
        wk::VERDICT_FAILS
    };
    let result_diag = if test_rejected {
        combined_diag.clone()
    } else {
        Some(match combined_diag.as_deref() {
            Some(note) => format!(
                "AlphaNotCrossed: computed p = {p_value_for_alpha:.6}, \
                 threshold alpha = {alpha}. {note}"
            ),
            None => format!(
                "AlphaNotCrossed: computed p = {p_value_for_alpha:.6}, \
                 threshold alpha = {alpha}"
            ),
        })
    };
    let canonical_for_result = if test_rejected {
        derived_proposition.as_ref()
    } else {
        None
    };
    Ok(gate_holds_with_result(
        claim.id(),
        "main_effect",
        result_ctor,
        result_diag.as_deref(),
        (t_statistic, p_value_for_alpha),
        canonical_for_result,
    ))
}

// ────────────────────────────────────────────────────────────────────
// §7.3 — MethodComparisonAnalysisPlan / Passing-Bablok dispatch
// ────────────────────────────────────────────────────────────────────

/// Recompute a `stats:MethodComparisonAnalysisPlan` via Passing-Bablok
/// regression. The bundle must be at the Paired position (PairedBlocking
/// blocking, SingleFactor factor, CrossSectional) — that's the same
/// authoring surface as `stats:Paired(...)`. Pairs are decoded with the
/// existing `decode_paired_observations` helper (the `[method_a_0,
/// method_b_0, method_a_1, method_b_1, ...]` interleaved layout).
///
/// Verdict: Holds when both the 95% slope CI contains 1.0 AND the 95%
/// intercept CI contains 0.0 (CLSI EP09 method-agreement criterion).
/// The verdict's `computed_statistic` field reports the median slope;
/// `computed_p_value` reports a binary disagreement indicator (0.0 on
/// agreement, 1.0 on disagreement) so the field stays load-bearing for
/// downstream consumers while the structural verdict is the CI check.
/// The diagnostic carries (slope, intercept, slope_CI, intercept_CI)
/// for audit. OneSidedWitnessed directionality is rejected for this
/// dispatch — Passing-Bablok is a CI-based agreement test, not a
/// sign-of-effect test. Outlier-exclusion functors other than Identity
/// are deferred to the §7.2 dual-verdict path (a separate GitHub
/// issue tracks PassingBablokResidual exclusion landing here).
fn recompute_method_comparison_claim(
    claim: &Resource,
    bundle: &DecodedBundle,
    _ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    if bundle.blocking != "PairedBlocking" {
        return Ok(gate_fails(format!(
            "MethodComparisonAnalysisPlan requires a Paired SampleSet (blocking = PairedBlocking, \
             factor = SingleFactor, repeated_measures = CrossSectional); got blocking = `{}`, \
             factor = `{}`, repeated_measures = `{}` (D52 §7.3 CLSI EP09)",
            bundle.blocking, bundle.factor, bundle.repeated_measures,
        )));
    }
    let directionality = match read_json_property(claim, iris::PROP_DIRECTIONALITY)? {
        Some(j) => j,
        None => return Ok(gate_fails("claim missing `directionality`".into())),
    };
    if json_ctor_name(&directionality) != Some("TwoSided") {
        return Ok(gate_fails(
            "MethodComparisonAnalysisPlan requires directionality = TwoSided — Passing-Bablok is a \
             CI-based agreement test, not a sign-of-effect test, and OneSidedWitnessed does \
             not refine it (D52 §7.3)"
                .into(),
        ));
    }
    let outlier = read_json_property(claim, iris::PROP_OUTLIER_EXCLUSION)?;
    if let Some(ctor) = outlier.as_ref().and_then(json_ctor_name) {
        if ctor != "Identity" {
            return Ok(gate_fails(format!(
                "MethodComparisonAnalysisPlan with outlier_exclusion = `{ctor}` not yet wired — the \
                 §7.2 dual-verdict outlier-exclusion path is implemented for the \
                 SingleSampleEstimate dispatch in Phase 5 v1; PassingBablokResidual /  ESD on \
                 method-comparison data is tracked as a follow-on issue"
            )));
        }
    }
    let pairs = match decode_paired_observations(&bundle.observations_raw) {
        Ok(p) => p,
        Err(diag) => return Ok(gate_fails(diag)),
    };
    let method_a: Vec<f64> = pairs.iter().map(|&(a, _b)| a).collect();
    let method_b: Vec<f64> = pairs.iter().map(|&(_a, b)| b).collect();
    let res = match passing_bablok_regression(&method_a, &method_b) {
        Some(r) => r,
        None => {
            return Ok(gate_fails(format!(
                "Passing-Bablok regression preconditions failed: need n ≥ 3 samples with at \
                 least one defined pairwise slope (no constant method-A column); got n = {}",
                pairs.len(),
            )));
        }
    };
    let methods_agree = res.slope_ci_low <= 1.0
        && 1.0 <= res.slope_ci_high
        && res.intercept_ci_low <= 0.0
        && 0.0 <= res.intercept_ci_high;
    let diag = format!(
        "Passing-Bablok regression: slope = {:.6} [95% CI {:.6}, {:.6}], intercept = {:.6} \
         [95% CI {:.6}, {:.6}], n_samples = {}, n_slopes = {}. Methods {} (Holds requires \
         1.0 ∈ slope_CI AND 0.0 ∈ intercept_CI per CLSI EP09)",
        res.slope,
        res.slope_ci_low,
        res.slope_ci_high,
        res.intercept,
        res.intercept_ci_low,
        res.intercept_ci_high,
        res.n_samples,
        res.n_slopes,
        if methods_agree { "agree" } else { "disagree" },
    );
    let p_indicator = if methods_agree { 0.0 } else { 1.0 };
    let ctor = if methods_agree {
        wk::VERDICT_HOLDS
    } else {
        wk::VERDICT_FAILS
    };
    let diag_string = if methods_agree {
        diag
    } else {
        format!("MethodComparisonDisagreement: {diag}")
    };
    // MethodComparison: gate Holds (the SAP ran). The per-effect
    // StatisticalAnalysisResult carries the agreement decision under
    // `methods_agree` and the slope / intercept numerics; canonical-
    // proposition derivation isn't wired yet for this dispatch — the
    // §7.3 Phase 5 v1 hardening lands when this gets its
    // `(slope = 1.0 ∧ intercept = 0.0)` Prop derivation. Without
    // canonical_proposition the witness emitter skips this result.
    Ok(gate_holds_with_result(
        claim.id(),
        "methods_agree",
        ctor,
        Some(&diag_string),
        (res.slope, p_indicator),
        None,
    ))
}

// ────────────────────────────────────────────────────────────────────
// §2.2 — ClassificationAnalysisPlan / PPV-sensitivity dispatch
// ────────────────────────────────────────────────────────────────────

/// Recompute a `stats:ClassificationAnalysisPlan` — the PPV/sensitivity of
/// a threshold classifier over a two-group (IID) SampleSet (D52 §2.2).
///
/// `group_a` is the **test-positive** group (the predictor under
/// evaluation flags these — e.g. MSI cell lines), `group_b` the
/// **test-negative** rest (MSS). A unit is **condition-positive** (the
/// trait predicted — e.g. WRN-dependent) when its value is below the
/// declared `classification_threshold` (dependency scores: lower = more
/// dependent).
///
/// Emits two `StatisticalAnalysisResult` derivations:
///   - `{plan}:result:ppv`         carrying `stats:ge(stats:ppv(s), min_ppv)`
///   - `{plan}:result:sensitivity` carrying `stats:ge(stats:sensitivity(s), min_sensitivity)`
///
/// Each result Holds iff its metric meets the author-declared minimum;
/// the canonical proposition (and thus the D49 `IsDerivedAs`) attaches
/// only on Holds. Downstream D39 reasoning composes both via a declared
/// statistical→domain bridge into a domain biomarker conclusion.
fn recompute_classification_quality_claim(
    claim: &Resource,
    bundle: &DecodedBundle,
    sample_set_iri_str: &str,
) -> Result<QueryOutcome, InstitutionError> {
    let (group_a, group_b) = match decode_two_group_observations(&bundle.observations_raw) {
        Ok(pair) => pair,
        Err(diag) => return Ok(gate_fails(diag)),
    };
    let threshold = match read_float_property(claim, iris::PROP_CLASSIFICATION_THRESHOLD)? {
        Some(t) => t,
        None => {
            return Ok(gate_fails(
                "ClassificationAnalysisPlan missing required `classification_threshold`".into(),
            ))
        }
    };
    let min_ppv = match read_float_property(claim, iris::PROP_MIN_PPV)? {
        Some(v) => v,
        None => {
            return Ok(gate_fails(
                "ClassificationAnalysisPlan missing required `min_ppv`".into(),
            ))
        }
    };
    let min_sensitivity = match read_float_property(claim, iris::PROP_MIN_SENSITIVITY)? {
        Some(v) => v,
        None => {
            return Ok(gate_fails(
                "ClassificationAnalysisPlan missing required `min_sensitivity`".into(),
            ))
        }
    };
    if group_a.is_empty() {
        return Ok(gate_fails(
            "ClassificationAnalysisPlan: test-positive group (group A) is empty — \
             PPV is undefined"
                .into(),
        ));
    }
    let m = classification_metrics(&group_a, &group_b, threshold);

    // PPV result: stats:ge(stats:ppv(s), min_ppv)
    let ppv_holds = m.ppv >= min_ppv;
    let ppv_prop = encode_classification_proposition(iris::STATS_PPV, sample_set_iri_str, min_ppv);
    let ppv_diag = format!(
        "Classification @ threshold {threshold}: PPV = {:.4} ({}/{} test-positive are \
         condition-positive); criterion PPV >= {min_ppv}",
        m.ppv, m.tp, m.n_test_positive,
    );
    // Sensitivity result: stats:ge(stats:sensitivity(s), min_sensitivity)
    let sens_holds = m.sensitivity >= min_sensitivity;
    let sens_prop = encode_classification_proposition(
        iris::STATS_SENSITIVITY,
        sample_set_iri_str,
        min_sensitivity,
    );
    let sens_diag = format!(
        "Classification @ threshold {threshold}: sensitivity = {:.4} ({}/{} condition-positive \
         are test-positive; FN = {}); criterion sensitivity >= {min_sensitivity}",
        m.sensitivity, m.tp, m.n_condition_positive, m.fn_,
    );

    let results = vec![
        PerEffectResult {
            effect_name: "ppv".to_string(),
            result_ctor: if ppv_holds {
                wk::VERDICT_HOLDS
            } else {
                wk::VERDICT_FAILS
            },
            diagnostic: Some(if ppv_holds {
                ppv_diag
            } else {
                format!("CriterionNotMet: {ppv_diag}")
            }),
            numerics: (m.ppv, 1.0 - m.ppv),
            canonical_proposition: if ppv_holds { Some(ppv_prop) } else { None },
        },
        PerEffectResult {
            effect_name: "sensitivity".to_string(),
            result_ctor: if sens_holds {
                wk::VERDICT_HOLDS
            } else {
                wk::VERDICT_FAILS
            },
            diagnostic: Some(if sens_holds {
                sens_diag
            } else {
                format!("CriterionNotMet: {sens_diag}")
            }),
            numerics: (m.sensitivity, 1.0 - m.sensitivity),
            canonical_proposition: if sens_holds { Some(sens_prop) } else { None },
        },
    ];
    Ok(gate_holds_with_results(claim.id(), results))
}

/// Build `stats:ge(stats:<metric>(s), threshold)` as a D47 type-fragment
/// JSON tree — the canonical proposition a ClassificationAnalysisPlan
/// result carries. `metric_iri` is `stats:ppv` or `stats:sensitivity`.
fn encode_classification_proposition(
    metric_iri: &str,
    sample_set_iri: &str,
    threshold: f64,
) -> serde_json::Value {
    use crate::institution::iris as i;
    let metric_of_s = encode_app(
        encode_const_ref(metric_iri),
        encode_lit_string(sample_set_iri),
    );
    encode_app(
        encode_app(encode_const_ref(i::STATS_GE), metric_of_s),
        encode_lit_float(threshold),
    )
}

// ────────────────────────────────────────────────────────────────────
// §2.2 — stats:Nested SampleSet / nested two-way ANOVA dispatch
// ────────────────────────────────────────────────────────────────────

/// Recompute a nested fixed-effects two-way ANOVA `value ~ group + subgroup`
/// (D52 §2.2), expressed as a `stats:Nested(...)` SampleSet (blocking ctor
/// `NestedBlocking`). The SampleSet's observations wrapper carries
/// `[group_a, group_b, subgroup_sizes_a, subgroup_sizes_b]`: the two groups
/// (A = first arm, B = second arm), each a flat array grouped by subgroup,
/// and the per-arm subgroup partition. The binary group main effect is
/// tested against the within-subgroup residual (subgroup = fixed nuisance).
/// On a directional Holds it emits `stats:lt(stats:mean_diff_of(s), 0)` —
/// group A mean strictly below group B, the same proposition shape as the
/// two-sample dispatch, so the same statistical→domain bridges consume it.
fn recompute_nested_anova_claim(
    claim: &Resource,
    bundle: &DecodedBundle,
    sample_set_iri_str: &str,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let (flat_a, flat_b, sizes_a, sizes_b) =
        match decode_nested_observations(&bundle.observations_raw) {
            Ok(p) => p,
            Err(diag) => return Ok(gate_fails(diag)),
        };
    let group_a = match partition_by_sizes(&flat_a, &sizes_a) {
        Ok(g) => g,
        Err(d) => return Ok(gate_fails(format!("subgroup_sizes_a: {d}"))),
    };
    let group_b = match partition_by_sizes(&flat_b, &sizes_b) {
        Ok(g) => g,
        Err(d) => return Ok(gate_fails(format!("subgroup_sizes_b: {d}"))),
    };
    let alpha = match read_float_property(claim, iris::PROP_ALPHA)? {
        Some(a) => a,
        None => return Ok(gate_fails("claim missing `alpha`".into())),
    };
    let directionality = match read_json_property(claim, iris::PROP_DIRECTIONALITY)? {
        Some(j) => j,
        None => return Ok(gate_fails("claim missing `directionality`".into())),
    };
    let r = match nested_group_anova(&group_a, &group_b) {
        Some(r) => r,
        None => {
            return Ok(gate_fails(
                "InsufficientReplication: nested ANOVA needs both groups non-empty and total \
                 N > #subgroups (at least one within-subgroup residual df)"
                    .into(),
            ))
        }
    };
    // Directionality (D52 §7.1). The group-effect F has 1 df (= t²), so the
    // one-sided p is the two-sided F p halved; OneSidedWitnessed also requires
    // the asserted direction (group A mean below group B — the lt claim) and a
    // resolving impossibility witness. TwoSided uses the raw two-way-ANOVA p.
    let dctor = json_ctor_name(&directionality);
    let (p_for_alpha, one_sided, direction_ok) = match dctor {
        Some("TwoSided") => (r.p_two_sided, false, true),
        Some("OneSidedWitnessed") => {
            let witness = directionality["args"]
                .get(0)
                .and_then(serde_json::Value::as_str);
            let witness = match witness {
                Some(s) => s.to_string(),
                None => {
                    return Ok(gate_fails(
                        "directionality = OneSidedWitnessed requires a witness IRI (D52 §7.1)"
                            .into(),
                    ))
                }
            };
            if let Some(diag) = check_impossibility_witness(&witness, ctx)? {
                return Ok(gate_fails(diag));
            }
            (r.p_two_sided / 2.0, true, r.mean_a < r.mean_b)
        }
        other => {
            return Ok(gate_fails(format!(
                "unknown directionality ctor `{other:?}` (expected TwoSided / OneSidedWitnessed)"
            )))
        }
    };
    let derived = derive_canonical_proposition_twosample(sample_set_iri_str, &directionality);
    let rejected = p_for_alpha < alpha && direction_ok;
    let note = format!(
        "Nested two-way ANOVA (value ~ group + subgroup): F({}, {}) = {:.4}, p_two_sided = {:.6}{}; \
         group_a mean = {:.4} (n = {}), group_b mean = {:.4} (n = {}); {} subgroups",
        r.df_group as usize,
        r.df_resid as usize,
        r.f_group,
        r.p_two_sided,
        if one_sided {
            format!(", p_one_sided = {:.6}", r.p_two_sided / 2.0)
        } else {
            String::new()
        },
        r.mean_a,
        r.n_a,
        r.mean_b,
        r.n_b,
        r.n_subgroups,
    );
    let ctor = if rejected {
        wk::VERDICT_HOLDS
    } else {
        wk::VERDICT_FAILS
    };
    let diag = if rejected {
        note
    } else {
        format!(
            "AlphaNotCrossed: computed p = {p_for_alpha:.6}, alpha = {alpha}, \
             direction_ok = {direction_ok}. {note}"
        )
    };
    let canonical = if rejected { derived.as_ref() } else { None };
    Ok(gate_holds_with_result(
        claim.id(),
        "main_effect",
        ctor,
        Some(&diag),
        (r.f_group, p_for_alpha),
        canonical,
    ))
}

/// Recompute a crossed additive two-way ANOVA `value ~ group + block`
/// (block crossed), expressed as a `stats:Crossed(...)` SampleSet (blocking
/// ctor `CrossedBlocking`): the binary group main effect against the
/// additive-model residual. Mirrors [`recompute_nested_anova_claim`] but
/// reads the same `[group_a, group_b, subgroup_sizes_a, subgroup_sizes_b]`
/// observations wrapper and pairs the two arms' block partitions by index
/// (crossed ⇒ same block levels in both groups, so the two size arrays must
/// have equal length) and calls [`crossed_two_way_anova`].
fn recompute_crossed_anova_claim(
    claim: &Resource,
    bundle: &DecodedBundle,
    sample_set_iri_str: &str,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let (flat_a, flat_b, sizes_a, sizes_b) =
        match decode_nested_observations(&bundle.observations_raw) {
            Ok(p) => p,
            Err(diag) => return Ok(gate_fails(diag)),
        };
    // Crossed design: the two arms must declare the SAME block levels, so the
    // partitions are index-aligned and equal in count.
    if sizes_a.len() != sizes_b.len() {
        return Ok(gate_fails(format!(
            "stats:Crossed requires the same crossed block levels in both groups: \
             subgroup_sizes_a has {} blocks but subgroup_sizes_b has {}",
            sizes_a.len(),
            sizes_b.len()
        )));
    }
    let group_a = match partition_by_sizes(&flat_a, &sizes_a) {
        Ok(g) => g,
        Err(d) => return Ok(gate_fails(format!("subgroup_sizes_a: {d}"))),
    };
    let group_b = match partition_by_sizes(&flat_b, &sizes_b) {
        Ok(g) => g,
        Err(d) => return Ok(gate_fails(format!("subgroup_sizes_b: {d}"))),
    };
    let alpha = match read_float_property(claim, iris::PROP_ALPHA)? {
        Some(a) => a,
        None => return Ok(gate_fails("claim missing `alpha`".into())),
    };
    let directionality = match read_json_property(claim, iris::PROP_DIRECTIONALITY)? {
        Some(j) => j,
        None => return Ok(gate_fails("claim missing `directionality`".into())),
    };
    let r = match crossed_two_way_anova(&group_a, &group_b) {
        Some(r) => r,
        None => {
            return Ok(gate_fails(
                "InsufficientReplication: crossed ANOVA needs equal block counts (each block \
                 present), both groups non-empty, and total N > #blocks + 1 (≥1 residual df)"
                    .into(),
            ))
        }
    };
    // Directionality (D52 §7.1): the group-effect F has 1 df (= t²), so the
    // one-sided p is the two-sided F p halved; OneSidedWitnessed also requires
    // the asserted direction (group A mean below group B) and a resolving
    // impossibility witness. TwoSided uses the raw two-way-ANOVA p.
    let dctor = json_ctor_name(&directionality);
    let (p_for_alpha, one_sided, direction_ok) = match dctor {
        Some("TwoSided") => (r.p_two_sided, false, true),
        Some("OneSidedWitnessed") => {
            let witness = directionality["args"]
                .get(0)
                .and_then(serde_json::Value::as_str);
            let witness = match witness {
                Some(s) => s.to_string(),
                None => {
                    return Ok(gate_fails(
                        "directionality = OneSidedWitnessed requires a witness IRI (D52 §7.1)"
                            .into(),
                    ))
                }
            };
            if let Some(diag) = check_impossibility_witness(&witness, ctx)? {
                return Ok(gate_fails(diag));
            }
            (r.p_two_sided / 2.0, true, r.mean_a < r.mean_b)
        }
        other => {
            return Ok(gate_fails(format!(
                "unknown directionality ctor `{other:?}` (expected TwoSided / OneSidedWitnessed)"
            )))
        }
    };
    let derived = derive_canonical_proposition_twosample(sample_set_iri_str, &directionality);
    let rejected = p_for_alpha < alpha && direction_ok;
    let note = format!(
        "Crossed two-way ANOVA (value ~ group + block): F({}, {}) = {:.4}, p_two_sided = {:.6e}{}; \
         group_a mean = {:.4} (n = {}), group_b mean = {:.4} (n = {}); {} crossed blocks",
        r.df_group as usize,
        r.df_resid as usize,
        r.f_group,
        r.p_two_sided,
        if one_sided {
            format!(", p_one_sided = {:.6e}", r.p_two_sided / 2.0)
        } else {
            String::new()
        },
        r.mean_a,
        r.n_a,
        r.mean_b,
        r.n_b,
        r.n_blocks,
    );
    let ctor = if rejected {
        wk::VERDICT_HOLDS
    } else {
        wk::VERDICT_FAILS
    };
    let diag = if rejected {
        note
    } else {
        format!(
            "AlphaNotCrossed: computed p = {p_for_alpha:.6e}, alpha = {alpha}, \
             direction_ok = {direction_ok}. {note}"
        )
    };
    let canonical = if rejected { derived.as_ref() } else { None };
    Ok(gate_holds_with_result(
        claim.id(),
        "main_effect",
        ctor,
        Some(&diag),
        (r.f_group, p_for_alpha),
        canonical,
    ))
}

/// Partition a flat value vector into subgroups of the given sizes. Errors if
/// the sizes don't sum to the vector length (the SampleSet arm and the
/// declared subgroup partition disagree).
fn partition_by_sizes(flat: &[f64], sizes: &[usize]) -> Result<Vec<Vec<f64>>, String> {
    let total: usize = sizes.iter().sum();
    if total != flat.len() {
        return Err(format!(
            "sizes sum to {total} but the SampleSet group has {} values",
            flat.len()
        ));
    }
    let mut out = Vec::with_capacity(sizes.len());
    let mut off = 0;
    for &s in sizes {
        out.push(flat[off..off + s].to_vec());
        off += s;
    }
    Ok(out)
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

impl DispatchPos {
    /// Whether this dispatch's test statistic carries a meaningful sign
    /// (t-based: SingleSampleEstimate, IID, Paired) — these support the
    /// §7.1 OneSidedWitnessed directionality routing. F-based dispatches
    /// (Factorial / RCBD / SplitPlot / RepeatedMeasures) produce
    /// intrinsically non-negative F-statistics; "one-sided" is not a
    /// refinement available to them and the verifier rejects
    /// OneSidedWitnessed on those dispatches with a structured
    /// diagnostic.
    fn supports_one_sided_directionality(self) -> bool {
        matches!(
            self,
            DispatchPos::SingleSampleEstimate | DispatchPos::IID | DispatchPos::Paired
        )
    }
}

// ────────────────────────────────────────────────────────────────────
// Multiple-comparison correction (D52 §3 / ICH E9 R1)
// ────────────────────────────────────────────────────────────────────

/// Family-wise alpha control policy. Decoded from the SAP's
/// `stats:multiple_comparison_correction` slot; default is
/// `NoCorrection` when the slot is absent. Multi-effect dispatches
/// apply the correction before deciding per-effect Holds/Fails;
/// single-effect dispatches ignore it (correction is a no-op at N=1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultipleComparisonCorrection {
    NoCorrection,
    Bonferroni,
    Holm,
    BenjaminiHochberg,
}

impl MultipleComparisonCorrection {
    fn ctor_name(self) -> &'static str {
        match self {
            Self::NoCorrection => "NoCorrection",
            Self::Bonferroni => "Bonferroni",
            Self::Holm => "Holm",
            Self::BenjaminiHochberg => "BenjaminiHochberg",
        }
    }
}

/// Read the correction policy off the claim. Returns
/// `Ok(Ok(method))` for a valid policy (default `NoCorrection` when
/// the slot is absent), `Ok(Err(diag))` when the slot is present but
/// malformed (caller surfaces as gate-Fails), or `Err(_)` for genuine
/// institutional failures.
fn read_multiple_comparison_correction(
    claim: &Resource,
) -> Result<Result<MultipleComparisonCorrection, String>, InstitutionError> {
    let raw = match read_json_property(claim, iris::PROP_MULTIPLE_COMPARISON_CORRECTION)? {
        Some(j) => j,
        None => return Ok(Ok(MultipleComparisonCorrection::NoCorrection)),
    };
    match json_ctor_name(&raw) {
        Some("NoCorrection") => Ok(Ok(MultipleComparisonCorrection::NoCorrection)),
        Some("Bonferroni") => Ok(Ok(MultipleComparisonCorrection::Bonferroni)),
        Some("Holm") => Ok(Ok(MultipleComparisonCorrection::Holm)),
        Some("BenjaminiHochberg") => Ok(Ok(MultipleComparisonCorrection::BenjaminiHochberg)),
        Some(other) => Ok(Err(format!(
            "unknown multiple_comparison_correction ctor `{other}` (expected NoCorrection / \
             Bonferroni / Holm / BenjaminiHochberg)"
        ))),
        None => Ok(Err(format!(
            "multiple_comparison_correction is not a chain-inductive value: {raw}"
        ))),
    }
}

/// Apply a multiple-comparison correction to a vector of raw per-effect
/// p-values and return per-effect rejection decisions at the family-
/// wise alpha. Returned vector has the same length and ordering as
/// the input; `rejected[i] == true` iff effect `i`'s p-value crosses
/// its correction-adjusted threshold. NaN / infinite p-values do not
/// reject.
fn apply_correction(
    raw_p_values: &[f64],
    alpha: f64,
    method: MultipleComparisonCorrection,
) -> Vec<bool> {
    let n = raw_p_values.len();
    if n == 0 {
        return Vec::new();
    }
    match method {
        MultipleComparisonCorrection::NoCorrection => raw_p_values
            .iter()
            .map(|p| p.is_finite() && *p < alpha)
            .collect(),
        MultipleComparisonCorrection::Bonferroni => {
            let bonf_alpha = alpha / n as f64;
            raw_p_values
                .iter()
                .map(|p| p.is_finite() && *p < bonf_alpha)
                .collect()
        }
        MultipleComparisonCorrection::Holm => {
            // Step-down Holm-Bonferroni. Sort p-values ascending; the
            // k-th smallest (rank k, 1-indexed) compares against
            // alpha / (n − k + 1). The first p that fails to cross
            // stops the chain — all higher-rank p-values are also
            // non-rejected.
            let mut indexed: Vec<(usize, f64)> = raw_p_values.iter().copied().enumerate().collect();
            indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut rejected = vec![false; n];
            for (rank, (orig_idx, p)) in indexed.iter().enumerate() {
                if !p.is_finite() {
                    break;
                }
                let threshold = alpha / (n - rank) as f64;
                if *p < threshold {
                    rejected[*orig_idx] = true;
                } else {
                    break;
                }
            }
            rejected
        }
        MultipleComparisonCorrection::BenjaminiHochberg => {
            // Step-up BH-FDR. Sort p-values ascending; find the largest
            // rank k (1-indexed) such that p_(k) ≤ k · alpha / n;
            // reject ranks 1..k. The `≤` here matches the standard BH
            // formulation (threshold-hit-rejects, unlike strict-less-
            // than at raw alpha).
            let mut indexed: Vec<(usize, f64)> = raw_p_values.iter().copied().enumerate().collect();
            indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut max_k = 0;
            for (rank, (_, p)) in indexed.iter().enumerate() {
                if !p.is_finite() {
                    continue;
                }
                let k = rank + 1;
                let threshold = (k as f64) * alpha / (n as f64);
                if *p <= threshold {
                    max_k = k;
                }
            }
            let mut rejected = vec![false; n];
            for (rank, (orig_idx, _)) in indexed.iter().enumerate() {
                if rank < max_k {
                    rejected[*orig_idx] = true;
                }
            }
            rejected
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Factorial multi-effect dispatch
// ────────────────────────────────────────────────────────────────────

/// Validate a Factorial-dispatch claim by running a per-effect ANOVA
/// decomposition and emitting one StatisticalAnalysisResult per effect. Used
/// in lieu of the single-`(t,p)` reduction the other dispatch arms
/// flow through because Factorial naturally produces 2^k − 1 effects
/// (k main effects + interactions), each with its own F-test, p-value,
/// and per-effect Holds/Fails decision.
///
/// Each per-effect StatisticalAnalysisResult carries:
/// - `canonical_proposition` = `factor_effect_of(s, "A")` or
///   `interaction_effect_of(s, "A:B")` (D52 §3 axioms) — set only when
///   that effect's p-value rejects the per-effect alpha
/// - `verdict_ctor` = Holds when the effect rejects, Fails otherwise
/// - `(F, p)` numerics
/// - `effect_name` = `"main_A"`, `"main_B"`, `"interaction_A_B"`, ...
///   (mirroring the IRI suffix)
///
/// Step 4 (multiple-comparison correction) will adjust alpha per
/// effect; for now NoCorrection is implicit — each effect uses the
/// SAP's raw alpha.
fn do_factorial_per_effect(
    claim: &Resource,
    bundle: &DecodedBundle,
    alpha: f64,
    effect_size: &serde_json::Value,
) -> Result<QueryOutcome, InstitutionError> {
    // Factorial requires Absolute / EtaSquared / OmegaSquared / None
    // EffectSize ctors. Standardized Cohen's d / Hedges' g are
    // two-sample shapes; reject those up front so the author sees
    // the diagnostic immediately.
    if let Some(ctor) = json_ctor_name(effect_size) {
        if !matches!(
            ctor,
            "Absolute" | "Relative" | "EtaSquared" | "OmegaSquared" | "NoneSpecified"
        ) {
            return Ok(gate_fails(format!(
                "Factorial dispatch with EffectSize `{ctor}` not yet wired; supported \
                 ctors: Absolute, Relative, EtaSquared, OmegaSquared, NoneSpecified"
            )));
        }
    }

    let (factor_levels, observations) =
        match decode_factorial_observations(&bundle.observations_raw) {
            Ok(p) => p,
            Err(diag) => return Ok(gate_fails(diag)),
        };
    let per_effect =
        match crate::numerics::factorial_anova_per_effect(&factor_levels, &observations) {
            Some(r) => r,
            None => {
                return Ok(gate_fails(format!(
                    "Factorial ANOVA preconditions failed: per-effect decomposition requires a \
                 balanced full-factorial with ≥ 2 levels per factor and ≥ 1 within-cell df \
                 (factor_levels = {factor_levels:?}, n_obs = {})",
                    observations.len()
                )));
            }
        };

    // Apply the SAP's multiple-comparison correction (NoCorrection
    // when the slot is absent) to the raw per-effect p-values; the
    // per-effect Holds/Fails decision uses the corrected rejection.
    let correction = match read_multiple_comparison_correction(claim)? {
        Ok(m) => m,
        Err(diag) => return Ok(gate_fails(diag)),
    };
    let raw_p_values: Vec<f64> = per_effect.effects.iter().map(|e| e.p_value).collect();
    let rejected = apply_correction(&raw_p_values, alpha, correction);

    let sample_set_iri_str = read_iri_property(claim, iris::PROP_SAMPLE_SET)?.unwrap_or_default();
    let mut results: Vec<PerEffectResult> = Vec::with_capacity(per_effect.effects.len());
    for (i, eff) in per_effect.effects.iter().enumerate() {
        let effect_name = effect_iri_suffix(&eff.factor_indices);
        let canonical_key = effect_canonical_key(&eff.factor_indices);
        let test_rejected = rejected[i];
        let result_ctor = if test_rejected {
            wk::VERDICT_HOLDS
        } else {
            wk::VERDICT_FAILS
        };
        let canonical_proposition = if test_rejected {
            Some(if eff.factor_indices.len() == 1 {
                derive_factor_effect_proposition(&sample_set_iri_str, &canonical_key)
            } else {
                derive_interaction_effect_proposition(&sample_set_iri_str, &canonical_key)
            })
        } else {
            None
        };
        let diagnostic = if test_rejected {
            format!(
                "factor_indices = {:?}, F = {:.6}, p = {:.6e} (df = {}, {}); rejected at alpha = \
                 {alpha} under {} correction",
                eff.factor_indices,
                eff.f_statistic,
                eff.p_value,
                eff.df_effect,
                eff.df_error,
                correction.ctor_name(),
            )
        } else {
            format!(
                "AlphaNotCrossed: factor_indices = {:?}, F = {:.6}, p = {:.6e} (df = {}, {}); \
                 threshold alpha = {alpha} under {} correction",
                eff.factor_indices,
                eff.f_statistic,
                eff.p_value,
                eff.df_effect,
                eff.df_error,
                correction.ctor_name(),
            )
        };
        results.push(PerEffectResult {
            effect_name,
            result_ctor,
            diagnostic: Some(diagnostic),
            numerics: (eff.f_statistic, eff.p_value),
            canonical_proposition,
        });
    }
    Ok(gate_holds_with_results(claim.id(), results))
}

/// Validate a SplitPlot-dispatch claim by running the three-F-test
/// ANOVA decomposition and emitting one StatisticalAnalysisResult per effect.
/// SplitPlot's classical decomposition has three nested error strata:
///
/// - Whole-plot main effect (W) — uses the whole-plot error term
/// - Subplot main effect (S) — uses the subplot error term
/// - W × S interaction — uses the subplot error term
///
/// Each effect's per-effect Holds/Fails decision compares its F-test
/// p-value against the SAP's alpha. The W effect's canonical
/// proposition is `factor_effect_of(s, "whole_plot")`; S's is
/// `factor_effect_of(s, "subplot")`; W×S's is
/// `interaction_effect_of(s, "whole_plot:subplot")`. NaN F-tests
/// (degenerate strata: no within-stratum df) record Undecidable
/// individual effects but still let the gate verdict pass — the SAP
/// itself ran.
fn do_splitplot_per_effect(
    claim: &Resource,
    bundle: &DecodedBundle,
    alpha: f64,
) -> Result<QueryOutcome, InstitutionError> {
    let (a, r) = match decode_splitplot_blocking(&bundle.blocking_raw) {
        Some(p) => p,
        None => {
            return Ok(gate_fails(
                "SplitPlot requires SplitPlotBlocking(a, r) in the blocking slot with a ≥ 2 and \
                 r ≥ 2"
                    .into(),
            ));
        }
    };
    let observations = match decode_splitplot_observations(&bundle.observations_raw) {
        Ok(o) => o,
        Err(diag) => return Ok(gate_fails(diag)),
    };
    let b = match a.checked_mul(r).and_then(|n_wp| {
        if n_wp == 0 || observations.len() % n_wp != 0 {
            None
        } else {
            Some(observations.len() / n_wp)
        }
    }) {
        Some(b) if b >= 2 => b,
        _ => {
            return Ok(gate_fails(format!(
                "SplitPlot observation count ({}) is not a*r*b for a={a}, r={r} \
                 (subplot factor level count b must be ≥ 2 and divide evenly)",
                observations.len()
            )));
        }
    };
    let res = match splitplot_anova(a, b, r, &observations) {
        Some(rr) => rr,
        None => {
            return Ok(gate_fails(format!(
                "SplitPlot ANOVA preconditions failed: each whole plot must have a consistent W \
                 level and contain every S level exactly once; each W level must have exactly \
                 r={r} whole-plot replicates (a={a}, b={b}, r={r}, n_obs = {})",
                observations.len()
            )));
        }
    };

    let correction = match read_multiple_comparison_correction(claim)? {
        Ok(m) => m,
        Err(diag) => return Ok(gate_fails(diag)),
    };

    let sample_set_iri_str = read_iri_property(claim, iris::PROP_SAMPLE_SET)?.unwrap_or_default();

    // Apply the correction across the three F-tests' raw p-values.
    // NaN p-values (degenerate strata) carry through as Undecidable
    // rather than being included in the correction's denominator —
    // the `apply_correction` helper treats them as "did not reject"
    // for ranking purposes.
    let effects = [
        ("main_whole_plot", "whole_plot", res.f_w, res.p_w, false),
        ("main_subplot", "subplot", res.f_s, res.p_s, false),
        (
            "interaction_whole_plot_subplot",
            "whole_plot:subplot",
            res.f_ws,
            res.p_ws,
            true,
        ),
    ];
    let raw_p_values: Vec<f64> = effects.iter().map(|(_, _, _, p, _)| *p).collect();
    let rejected = apply_correction(&raw_p_values, alpha, correction);

    let mut results = Vec::with_capacity(3);
    for (i, (name, key, f_stat, p_value, is_interaction)) in effects.iter().enumerate() {
        let undecidable = p_value.is_nan();
        let test_rejected = !undecidable && rejected[i];
        let result_ctor = if undecidable {
            wk::VERDICT_UNDECIDABLE
        } else if test_rejected {
            wk::VERDICT_HOLDS
        } else {
            wk::VERDICT_FAILS
        };
        let canonical_proposition = if test_rejected {
            Some(if *is_interaction {
                derive_interaction_effect_proposition(&sample_set_iri_str, key)
            } else {
                derive_factor_effect_proposition(&sample_set_iri_str, key)
            })
        } else {
            None
        };
        let diagnostic = if undecidable {
            format!(
                "SplitPlot effect `{name}`: F = NaN, p = NaN — degenerate error stratum (no df)"
            )
        } else if test_rejected {
            format!(
                "SplitPlot effect `{name}`: F = {f_stat:.6}, p = {p_value:.6e}; rejected at \
                 alpha = {alpha} under {} correction",
                correction.ctor_name()
            )
        } else {
            format!(
                "AlphaNotCrossed: SplitPlot effect `{name}`: F = {f_stat:.6}, p = {p_value:.6e}; \
                 threshold alpha = {alpha} under {} correction",
                correction.ctor_name()
            )
        };
        results.push(PerEffectResult {
            effect_name: name.to_string(),
            result_ctor,
            diagnostic: Some(diagnostic),
            numerics: (*f_stat, *p_value),
            canonical_proposition,
        });
    }
    Ok(gate_holds_with_results(claim.id(), results))
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

/// Decoded nested/crossed two-way-ANOVA observations:
/// `(group_a, group_b, subgroup_sizes_a, subgroup_sizes_b)`. Typed alias to
/// satisfy clippy's type-complexity lint on the decoder's return type.
type NestedObservations = (Vec<f64>, Vec<f64>, Vec<usize>, Vec<usize>);

/// Decode the nested/crossed two-way-ANOVA observations payload:
/// `[group_a, group_b, subgroup_sizes_a, subgroup_sizes_b]` (D52 §2.2 /
/// §4.2). Elements [0],[1] are the two flat group arrays (group A = the
/// treatment / first-context arm, group B = control / second-context arm);
/// elements [2],[3] are the per-subgroup observation counts that partition
/// each arm. The subgroup partition lives in the SampleSet (carried by the
/// `stats:Nested` / `stats:Crossed` smart-constructors) rather than on the
/// plan. Returns `(group_a, group_b, subgroup_sizes_a, subgroup_sizes_b)`.
fn decode_nested_observations(j: &serde_json::Value) -> Result<NestedObservations, String> {
    let outer = j
        .as_array()
        .ok_or_else(|| format!("Nested/Crossed observations slot is not an array: {j:?}"))?;
    if outer.len() != 4 {
        return Err(format!(
            "Nested/Crossed expects observations = [group_a, group_b, subgroup_sizes_a, \
             subgroup_sizes_b] (4 elements; got {})",
            outer.len()
        ));
    }
    let group_a =
        decode_flat_observations(&outer[0]).map_err(|e| format!("Nested/Crossed group A: {e}"))?;
    let group_b =
        decode_flat_observations(&outer[1]).map_err(|e| format!("Nested/Crossed group B: {e}"))?;
    let sizes_a = decode_usize_array(&outer[2])
        .map_err(|e| format!("Nested/Crossed subgroup_sizes_a: {e}"))?;
    let sizes_b = decode_usize_array(&outer[3])
        .map_err(|e| format!("Nested/Crossed subgroup_sizes_b: {e}"))?;
    Ok((group_a, group_b, sizes_a, sizes_b))
}

/// Decode a flat JSON array of non-negative whole numbers into
/// `Vec<usize>` — the subgroup partition sizes carried in the
/// nested/crossed SampleSet observations wrapper. Rejects negatives /
/// non-integral / non-array values.
fn decode_usize_array(j: &serde_json::Value) -> Result<Vec<usize>, String> {
    let arr = j.as_array().ok_or_else(|| format!("not an array: {j:?}"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let n = v
            .as_f64()
            .ok_or_else(|| format!("element {i} is not a number: {v:?}"))?;
        if n < 0.0 || n.fract() != 0.0 {
            return Err(format!("element {i} is not a non-negative integer: {n}"));
        }
        out.push(n as usize);
    }
    Ok(out)
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
// §7.1 impossibility-witness validation (OneSidedWitnessed gate)
// ────────────────────────────────────────────────────────────────────

/// Validate that a `Directionality.OneSidedWitnessed(witness_iri)`
/// claim's witness resolves to a chain resource carrying
/// `is_a stats:ImpossibilityWitness`. Returns `Ok(None)` if the witness
/// is admissible, `Ok(Some(diag))` if the claim must be rejected, and
/// `Err(_)` only for genuine institutional failures.
fn check_impossibility_witness(
    witness_iri_str: &str,
    ctx: &ExecutionContext,
) -> Result<Option<String>, InstitutionError> {
    let witness_iri = match Iri::parse(witness_iri_str) {
        Ok(i) => i,
        Err(e) => {
            return Ok(Some(format!(
                "OneSidedWitnessed witness IRI `{witness_iri_str}` does not parse: {e:?} \
                 (D52 §7.1 requires a chain-resident impossibility witness)"
            )));
        }
    };
    let witness_res = match ctx.resolve(&witness_iri) {
        Some(r) => r,
        None => {
            return Ok(Some(format!(
                "OneSidedWitnessed witness `{witness_iri_str}` is not committed on chain \
                 (D52 §7.1 — the one-sided p-value path requires a chain-resident proof that \
                 the inverse direction is impossible within the system under study)"
            )));
        }
    };
    let marker_iri = Iri::parse(iris::IMPOSSIBILITY_WITNESS).expect("static IRI");
    let has_marker = witness_res.is_a().iter().any(|c| c == &marker_iri);
    if has_marker {
        Ok(None)
    } else {
        Ok(Some(format!(
            "OneSidedWitnessed witness `{witness_iri_str}` exists on chain but does not carry \
             `is_a stats:ImpossibilityWitness` — the verifier admits the one-sided p-value \
             path only when the witness resource is explicitly marked as an impossibility \
             witness (D52 §7.1). Mark the resource with the ImpossibilityWitness class, or \
             use Directionality.TwoSided"
        )))
    }
}

// ────────────────────────────────────────────────────────────────────
// §7.4 epistemic-scope check
// ────────────────────────────────────────────────────────────────────

/// Extract the canonical proposition's head predicate IRI, look up its
/// `is_a` markers, and check admissibility against the SampleSet's
/// replication kind.
///
/// `derived_proposition` is the JSON D47 value the verifier just
/// constructed (the verdict's canonical_proposition slot). When the
/// dispatch arm hasn't yet wired derivation, the parameter is `None`
/// and the scope check treats it as inconclusive — the dispatch arm's
/// own rejection diagnostic is the load-bearing one in that case.
///
/// Returns `Ok(None)` if the scope is admissible; `Ok(Some(diag))` with
/// a diagnostic string if the institution must reject the claim per
/// §7.4. `Err(_)` only for genuine institutional failures (resolution
/// errors etc.), not scope mismatches.
fn check_epistemic_scope(
    derived_proposition: Option<&serde_json::Value>,
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
    let derived_prop = match derived_proposition {
        Some(j) => j,
        None => {
            // No derived canonical_proposition — the dispatch arm
            // didn't wire its derivation, so there's nothing to scope-
            // check. The arm's own rejection diagnostic carries the
            // load-bearing reason; this branch returns "inconclusive."
            return Ok(None);
        }
    };
    let predicate_iri = match extract_head_predicate_iri(derived_prop) {
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

// ────────────────────────────────────────────────────────────────────
// Canonical-proposition derivation (D52 §3 revision)
// ────────────────────────────────────────────────────────────────────
//
// The verifier — not the author — constructs the alternative
// hypothesis from the claim's statistical parameters. One canonical
// proposition per (dispatch, effect_size, directionality) triple.
// Each value is a D47 chain-mirrored type-fragment JSON tree whose
// hash the D49 witness index keys on; consumer-side reasoning
// (D39 reasoning institution + ESL `DerivedEvidence`) reconstructs
// the same Exp from a proof term, encodes via the same `encode_type`
// path, and arrives at the same hash. The hash equality is the
// soundness guarantee tying chain-resident verdict to chain-resident
// citation.

/// Derive the canonical proposition for a `SingleSampleEstimate`
/// dispatch. Returns `None` for parameter shapes that aren't yet wired
/// (the verdict skips its `canonical_proposition` slot in that case).
///
/// `TwoSided + Absolute(T)`           ⇒  `¬(stats:mean_of(s) = T)`
///                                     ≡ `Pi(_, Id(core:float, mean_of(s), T), stats:False)`
/// `OneSidedWitnessed + Absolute(T)`  ⇒  `stats:lt(stats:mean_of(s), T)`
///                                     ≡ `App(App(ConstRef(stats:lt), mean_of(s)), T)`
///
/// The OneSidedWitnessed arm defaults to `stats:lt` (less-than)
/// because the `Directionality.OneSidedWitnessed(witness_iri)` ctor
/// doesn't yet carry the direction. The running IC50 example is a
/// `< T` claim, so this default is consistent with the only
/// OneSidedWitnessed shape exercised in v1. When the ctor gets a
/// direction parameter, this arm reads it and routes to `stats:lt`
/// vs `stats:gt` accordingly.
fn derive_canonical_proposition_singlesample(
    sample_set_iri: &str,
    effect_size: &serde_json::Value,
    directionality: &serde_json::Value,
) -> Option<serde_json::Value> {
    use crate::institution::iris as i;
    let (magnitude, _units) = parse_effect_size_absolute(effect_size)?;
    let mean_of_s = encode_app(
        encode_const_ref(i::STATS_MEAN_OF),
        encode_lit_string(sample_set_iri),
    );
    let threshold = encode_lit_float(magnitude);
    match json_ctor_name(directionality)? {
        "TwoSided" => {
            // ¬(mean_of(s) = T) ≡ (mean_of(s) = T) → False
            // ≡ Pi("", Id(core:float, mean_of(s), T), False)
            let eq = encode_id(encode_const_ref(wk::FLOAT), mean_of_s, threshold);
            let false_ = encode_const_ref(i::STATS_FALSE);
            Some(encode_pi("", eq, false_))
        }
        "OneSidedWitnessed" => {
            // stats:lt(mean_of(s), T) — see fn-level note on the
            // direction default.
            Some(encode_app(
                encode_app(encode_const_ref(i::STATS_LT), mean_of_s),
                threshold,
            ))
        }
        _ => None,
    }
}

/// Derive the canonical proposition for a two-sample (IID) comparison —
/// shared by the t-test and Wilcoxon rank-sum paths (same H1 shape). The
/// proposition is about the sample set's group mean-difference
/// `stats:mean_diff_of(s)` = mean(group_a) − mean(group_b):
///
///   TwoSided          → ¬(mean_diff_of(s) = 0)
///   OneSidedWitnessed → stats:lt(mean_diff_of(s), 0)   (group A below group B)
///
/// Authoring convention: place the hypothesised-lower group first
/// (`group_a`) so the one-sided `lt` reads in the asserted direction.
/// (As with the one-sample case, v1's verdict checks p < alpha but not
/// the sign of the observed difference — the directional refinement is a
/// shared follow-on; the WRN MSI<MSS direction holds regardless.)
fn derive_canonical_proposition_twosample(
    sample_set_iri: &str,
    directionality: &serde_json::Value,
) -> Option<serde_json::Value> {
    use crate::institution::iris as i;
    let mean_diff = encode_app(
        encode_const_ref(i::STATS_MEAN_DIFF_OF),
        encode_lit_string(sample_set_iri),
    );
    let zero = encode_lit_float(0.0);
    match json_ctor_name(directionality)? {
        "TwoSided" => {
            // ¬(mean_diff_of(s) = 0) ≡ (mean_diff_of(s) = 0) → False
            let eq = encode_id(encode_const_ref(wk::FLOAT), mean_diff, zero);
            Some(encode_pi("", eq, encode_const_ref(i::STATS_FALSE)))
        }
        "OneSidedWitnessed" => Some(encode_app(
            encode_app(encode_const_ref(i::STATS_LT), mean_diff),
            zero,
        )),
        _ => None,
    }
}

/// Derive the canonical proposition for a Spearman rank correlation
/// (Paired + RankBased) over `stats:spearman_rho(s)`:
///
///   TwoSided          → ¬(spearman_rho(s) = 0)   (some monotone association)
///   OneSidedWitnessed → stats:lt(spearman_rho(s), 0)   (negative correlation)
///
/// Authoring convention: the one-sided form asserts *anti*-correlation
/// (rho < 0); the WRN dependency ~ #MS-deletions claim is of this form.
/// (As elsewhere, v1's verdict checks p < alpha but not the sign of the
/// observed rho — a shared directional follow-on; the WRN rho < 0 holds.)
fn derive_canonical_proposition_correlation(
    sample_set_iri: &str,
    directionality: &serde_json::Value,
) -> Option<serde_json::Value> {
    use crate::institution::iris as i;
    let rho = encode_app(
        encode_const_ref(i::STATS_SPEARMAN_RHO),
        encode_lit_string(sample_set_iri),
    );
    let zero = encode_lit_float(0.0);
    match json_ctor_name(directionality)? {
        "TwoSided" => {
            let eq = encode_id(encode_const_ref(wk::FLOAT), rho, zero);
            Some(encode_pi("", eq, encode_const_ref(i::STATS_FALSE)))
        }
        "OneSidedWitnessed" => Some(encode_app(
            encode_app(encode_const_ref(i::STATS_LT), rho),
            zero,
        )),
        _ => None,
    }
}

/// Build `stats:factor_effect_of(sample_iri, factor_key)` as a D47
/// type-fragment JSON tree. Used by the Factorial / SplitPlot / RCBD
/// / RM dispatches when emitting a main-effect StatisticalAnalysisResult.
fn derive_factor_effect_proposition(sample_iri: &str, factor_key: &str) -> serde_json::Value {
    use crate::institution::iris as i;
    encode_app(
        encode_app(
            encode_const_ref(i::STATS_FACTOR_EFFECT_OF),
            encode_lit_string(sample_iri),
        ),
        encode_lit_string(factor_key),
    )
}

/// Build `stats:interaction_effect_of(sample_iri, interaction_key)`
/// as a D47 type-fragment JSON tree. The interaction key is a colon-
/// separated list of factor letters (`"A:B"` for the AB two-way,
/// `"A:B:C"` for the three-way ABC) — see the ESL axiom comment for
/// the convention.
fn derive_interaction_effect_proposition(
    sample_iri: &str,
    interaction_key: &str,
) -> serde_json::Value {
    use crate::institution::iris as i;
    encode_app(
        encode_app(
            encode_const_ref(i::STATS_INTERACTION_EFFECT_OF),
            encode_lit_string(sample_iri),
        ),
        encode_lit_string(interaction_key),
    )
}

/// Map factor index → letter for the canonical effect-name convention.
/// `0 → "A"`, `1 → "B"`, …, `25 → "Z"`, then `26 → "F26"`, etc. for the
/// rare designs with >26 factors. Used in both the StatisticalAnalysisResult
/// IRI suffix and the canonical_proposition's effect-key string.
fn factor_letter(idx: usize) -> String {
    if idx < 26 {
        ((b'A' + idx as u8) as char).to_string()
    } else {
        format!("F{idx}")
    }
}

/// Effect-name suffix used on the result IRI (`{analysis_iri}:result:{name}`).
/// Main effects: `main_A`, `main_B`, etc. Interactions:
/// `interaction_A_B`, `interaction_A_B_C`. Underscore separators in
/// the IRI suffix (URN-safe).
fn effect_iri_suffix(factor_indices: &[usize]) -> String {
    let letters: Vec<String> = factor_indices.iter().copied().map(factor_letter).collect();
    if letters.len() == 1 {
        format!("main_{}", letters[0])
    } else {
        format!("interaction_{}", letters.join("_"))
    }
}

/// Effect key used inside the canonical_proposition: `"A"`, `"A:B"`,
/// `"A:B:C"`. Colon-separated factor letters — see the ESL
/// `stats:interaction_effect_of` axiom comment for the convention.
fn effect_canonical_key(factor_indices: &[usize]) -> String {
    factor_indices
        .iter()
        .copied()
        .map(factor_letter)
        .collect::<Vec<_>>()
        .join(":")
}

fn encode_const_ref(iri: &str) -> serde_json::Value {
    serde_json::json!({"ctor": "ConstRef", "args": [iri]})
}

fn encode_lit_string(s: &str) -> serde_json::Value {
    serde_json::json!({"ctor": "LitString", "args": [s]})
}

fn encode_lit_float(f: f64) -> serde_json::Value {
    serde_json::json!({"ctor": "LitFloat", "args": [f]})
}

fn encode_app(head: serde_json::Value, arg: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"ctor": "App", "args": [head, arg]})
}

fn encode_pi(binder: &str, dom: serde_json::Value, body: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"ctor": "Pi", "args": [binder, dom, body]})
}

fn encode_id(
    ty: serde_json::Value,
    lhs: serde_json::Value,
    rhs: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({"ctor": "Id", "args": [ty, lhs, rhs]})
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
            "StatisticalAnalysisPlan `{prop_iri}` is not a string/IRI: {other:?}"
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
            "StatisticalAnalysisPlan `{prop_iri}` is not a number: {other:?}"
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
            "StatisticalAnalysisPlan `{prop_iri}` is not a chain-inductive value: {other:?}"
        ))),
        None => Ok(None),
    }
}

/// Build a gate-Fails QueryOutcome — the SAP couldn't run (missing
/// field, malformed bundle, unwired dispatch, scope violation, etc.).
/// No StatisticalAnalysisResult derivations are emitted because no test ran.
fn gate_fails(diagnostic: String) -> QueryOutcome {
    QueryOutcome::from_output(gate_verdict_resource(wk::VERDICT_FAILS, Some(&diagnostic)))
}

/// Build a gate-Holds QueryOutcome carrying a single StatisticalAnalysisResult
/// derivation — the SAP ran successfully and produced a per-effect
/// statistical decision. The per-effect decision lives on the
/// StatisticalAnalysisResult (`verdict_ctor` property), independent of the
/// gate verdict which attests only "the SAP was structurally
/// runnable." Non-rejecting tests (AlphaNotCrossed) still gate-Hold;
/// the chain attests the negative result as a typed artefact.
///
/// `analysis_iri` is `None` for embedded subjects (post-translation
/// validation) — in that case no derivation is emitted because there's
/// no chain IRI to attach to, but the gate verdict still goes through.
fn gate_holds_with_result(
    analysis_iri: Option<&Iri>,
    effect_name: &str,
    result_ctor: &str,
    diagnostic: Option<&str>,
    numerics: (f64, f64),
    canonical_proposition: Option<&serde_json::Value>,
) -> QueryOutcome {
    let mut out = QueryOutcome::from_output(gate_verdict_resource(wk::VERDICT_HOLDS, None));
    if let Some(iri) = analysis_iri {
        out.derivations.push(measurement_result_resource(
            iri,
            effect_name,
            result_ctor,
            diagnostic,
            numerics,
            canonical_proposition,
        ));
    }
    out
}

/// Single per-effect derivation slot for [`gate_holds_with_results`].
/// Holds owned `String`s so callers can build the per-effect names
/// dynamically (from factor indices) without lifetime gymnastics.
struct PerEffectResult {
    effect_name: String,
    /// One of `wk::VERDICT_HOLDS` / `wk::VERDICT_FAILS` — `&'static str`.
    result_ctor: &'static str,
    diagnostic: Option<String>,
    numerics: (f64, f64),
    canonical_proposition: Option<serde_json::Value>,
}

/// Gate-Holds outcome carrying multiple StatisticalAnalysisResult derivations
/// — the shape multi-effect dispatches (Factorial, SplitPlot,
/// RepeatedMeasures, multi-factor RCBD) produce. The kernel commits
/// each derivation independently at its own
/// `{analysis_iri}:result:{effect_name}` IRI and the witness emitter
/// admits one `IsDerivedAs` witness per derivation that carries a
/// canonical_proposition (D52 §6 / D49 §6).
fn gate_holds_with_results(
    analysis_iri: Option<&Iri>,
    results: Vec<PerEffectResult>,
) -> QueryOutcome {
    let mut out = QueryOutcome::from_output(gate_verdict_resource(wk::VERDICT_HOLDS, None));
    if let Some(iri) = analysis_iri {
        for r in results {
            out.derivations.push(measurement_result_resource(
                iri,
                &r.effect_name,
                r.result_ctor,
                r.diagnostic.as_deref(),
                r.numerics,
                r.canonical_proposition.as_ref(),
            ));
        }
    }
    out
}

/// Build the minimal institutional gate Verdict resource. The kernel
/// stamps `verdict_subject`, `verdict_query_class`,
/// `runtime_invocation`, `is_a [Verdict, DerivedResource]`,
/// `dispatched_to`; we set only the per-Verdict-instance fields:
/// `ctor_name` (Holds / Fails / Undecidable) and optional diagnostic.
fn gate_verdict_resource(ctor_name: &str, diagnostic: Option<&str>) -> Resource {
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
    r
}

/// Build the StatisticalAnalysisResult derivation for one effect. IRI is
/// `{analysis_iri}:result:{effect_name}` — deterministic from the
/// (analysis, effect) pair so re-runs collapse idempotently. The
/// kernel adds `is_a [DerivedResource, InstitutionEmittedDerivation]`
/// + `reflection:from_subject` + `reflection:runtime_invocation`; we
///   set the domain class (`stats:StatisticalAnalysisResult`) plus the per-effect
///   payload.
fn measurement_result_resource(
    analysis_iri: &Iri,
    effect_name: &str,
    result_ctor: &str,
    diagnostic: Option<&str>,
    (t_statistic, p_value): (f64, f64),
    canonical_proposition: Option<&serde_json::Value>,
) -> Resource {
    const DIAGNOSTIC_IRI: &str = "urn:eigenius:institution:diagnostic";
    let result_iri = Iri::parse(&format!("{}:result:{}", analysis_iri.as_str(), effect_name))
        .expect("result IRI parses");
    let mut r = Resource::new(result_iri);
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(
            iris::STATISTICAL_ANALYSIS_RESULT.to_string(),
        )]),
    );
    r.set(
        Iri::parse(iris::PROP_VERDICT_CTOR).expect("static IRI"),
        Value::String(result_ctor.to_string()),
    );
    r.set(
        Iri::parse(iris::PROP_EFFECT_NAME).expect("static IRI"),
        Value::String(effect_name.to_string()),
    );
    r.set(
        Iri::parse(iris::PROP_COMPUTED_STATISTIC).expect("static IRI"),
        Value::Float(t_statistic),
    );
    r.set(
        Iri::parse(iris::PROP_COMPUTED_P_VALUE).expect("static IRI"),
        Value::Float(p_value),
    );
    if let Some(d) = diagnostic {
        r.set(
            Iri::parse(DIAGNOSTIC_IRI).expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    if let Some(prop) = canonical_proposition {
        r.set(
            Iri::parse(iris::PROP_CANONICAL_PROPOSITION).expect("static IRI"),
            Value::Json(prop.clone()),
        );
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_no_correction_uses_raw_alpha() {
        let ps = vec![0.001, 0.04, 0.06, 0.5];
        let r = apply_correction(&ps, 0.05, MultipleComparisonCorrection::NoCorrection);
        assert_eq!(r, vec![true, true, false, false]);
    }

    #[test]
    fn correction_bonferroni_divides_alpha() {
        // alpha = 0.05, N = 4 → threshold 0.0125
        let ps = vec![0.001, 0.04, 0.06, 0.5];
        let r = apply_correction(&ps, 0.05, MultipleComparisonCorrection::Bonferroni);
        assert_eq!(r, vec![true, false, false, false]);
    }

    #[test]
    fn correction_holm_step_down() {
        // alpha = 0.05, N = 4. Sort ascending: 0.001, 0.01, 0.04, 0.5.
        //  rank 1: threshold = 0.05/4 = 0.0125 → 0.001 < 0.0125 → reject
        //  rank 2: threshold = 0.05/3 ≈ 0.01667 → 0.01 < 0.01667 → reject
        //  rank 3: threshold = 0.05/2 = 0.025  → 0.04 not < 0.025 → stop
        let ps = vec![0.01, 0.001, 0.04, 0.5];
        let r = apply_correction(&ps, 0.05, MultipleComparisonCorrection::Holm);
        // Indices 1 (0.001) and 0 (0.01) reject; 2 (0.04) and 3 (0.5) don't.
        assert_eq!(r, vec![true, true, false, false]);
    }

    #[test]
    fn correction_bh_step_up() {
        // alpha = 0.05, N = 4. Sort ascending: 0.001, 0.01, 0.04, 0.5.
        //  rank 1: 1·0.05/4 = 0.0125 → 0.001 ≤ 0.0125 → max_k = 1
        //  rank 2: 2·0.05/4 = 0.025  → 0.01 ≤ 0.025  → max_k = 2
        //  rank 3: 3·0.05/4 = 0.0375 → 0.04 > 0.0375 → max_k stays 2
        //  rank 4: 4·0.05/4 = 0.05   → 0.5 > 0.05 → max_k stays 2
        // Reject ranks 1, 2 → indices 1 (0.001) and 0 (0.01).
        let ps = vec![0.01, 0.001, 0.04, 0.5];
        let r = apply_correction(&ps, 0.05, MultipleComparisonCorrection::BenjaminiHochberg);
        assert_eq!(r, vec![true, true, false, false]);
    }

    #[test]
    fn correction_nan_p_values_dont_reject() {
        let ps = vec![0.001, f64::NAN, 0.04];
        let r = apply_correction(&ps, 0.05, MultipleComparisonCorrection::Bonferroni);
        // NaN is not rejected; raw threshold = 0.05/3 ≈ 0.0167.
        assert_eq!(r, vec![true, false, false]);
    }

    #[test]
    fn correction_empty_input_returns_empty() {
        let r = apply_correction(&[], 0.05, MultipleComparisonCorrection::Holm);
        assert!(r.is_empty());
    }

    #[test]
    fn correction_bh_includes_threshold_equal() {
        // BH uses ≤ (vs raw alpha's <). At alpha=0.05, N=2,
        // p = [0.025, 0.05] should yield both rejected:
        //  rank 1: 1·0.05/2 = 0.025 → 0.025 ≤ 0.025 → max_k = 1
        //  rank 2: 2·0.05/2 = 0.05  → 0.05 ≤ 0.05 → max_k = 2
        let ps = vec![0.025, 0.05];
        let r = apply_correction(&ps, 0.05, MultipleComparisonCorrection::BenjaminiHochberg);
        assert_eq!(r, vec![true, true]);
    }
}
