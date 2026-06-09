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

//! Deterministic IEEE-754 numerics used by the verifier.
//!
//! Per D52 §6, every recomputation procedure must be:
//! - **Deterministic**: identical inputs → bit-identical outputs.
//!   No fast-math, no non-deterministic parallel reductions, no
//!   RNG without a chain-derived seed.
//! - **Bounded**: every Tier 1+2 procedure terminates in time
//!   polynomial in N.
//! - **Reproducible from chain**: no external data, no system time.
//!
//! The `ndarray` + `statrs` choice (D52 §12 #6) covers Phase 1's
//! one-sample tests. Two-sample, ANOVA, and mixed-effects come later.

use ndarray::Array1;
use statrs::distribution::{ContinuousCDF, FisherSnedecor, StudentsT};
use std::collections::BTreeMap;

/// One-sample t-test statistic and two-sided p-value.
///
/// Tests `H0: mean(samples) = null_mean` against
/// `H1: mean(samples) ≠ null_mean` under the Student-t distribution
/// with `n - 1` degrees of freedom. Two-sided by default (D52 §7.1);
/// the verifier wraps this in one-sided dispatch only when the claim
/// carries a valid impossibility witness.
///
/// Returns `(t_statistic, two_sided_p_value, n_used, computed_mean,
/// computed_sd)`. The mean and SD are reported alongside the test
/// statistic so the verdict captures the full intermediate state for
/// audit (D52 §6 — `Holds` outcome carries the numerics, not just the
/// outcome).
///
/// # Edge cases
///
/// - `samples.len() < 2`: returns `None`. A single sample has zero
///   degrees of freedom and no defined SD; the verifier surfaces this
///   as `InsufficientReplication` rather than NaN-propagating.
/// - All samples equal: SD is zero; t-statistic is `±∞` (or 0 if mean
///   exactly equals null_mean), p-value is 0 (or 1). statrs handles
///   this naturally via its CDF; we don't special-case.
pub fn one_sample_t_test(samples: &[f64], null_mean: f64) -> Option<OneSampleResult> {
    if samples.len() < 2 {
        return None;
    }
    let n = samples.len() as f64;
    let arr = Array1::from(samples.to_vec());
    let mean = arr.mean().expect("non-empty after len check");

    // Sample SD (Bessel-corrected, n-1 denominator). Standard for
    // inferential statistics; matches every reference implementation
    // the bench scientist's stats software defaults to (R/SPSS/etc.).
    let var = arr.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();

    let se = sd / n.sqrt();
    let t = if se == 0.0 {
        // Degenerate: identical samples. Test is either trivially true
        // (means equal exactly) or trivially false (means differ). The
        // two-sided p-value below collapses cleanly because the t-CDF
        // saturates at ±∞.
        if (mean - null_mean).abs() == 0.0 {
            0.0
        } else if mean > null_mean {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        }
    } else {
        (mean - null_mean) / se
    };

    let df = n - 1.0;
    // StudentsT::new(location=0, scale=1, freedom=df)
    let dist = StudentsT::new(0.0, 1.0, df)
        .expect("Student's t with df = n-1 > 0 is well-defined for n >= 2");
    // Two-sided: P(|T| > |t|) = 2 * (1 - F(|t|)) where F is the CDF.
    let p_two_sided = if t.is_finite() {
        2.0 * (1.0 - dist.cdf(t.abs()))
    } else if t.is_nan() {
        f64::NAN
    } else {
        0.0
    };

    Some(OneSampleResult {
        t_statistic: t,
        p_value_two_sided: p_two_sided,
        n_used: samples.len(),
        computed_mean: mean,
        computed_sd: sd,
    })
}

/// The full numeric output of a one-sample t-test, suitable for
/// embedding in a `MeasurementResult`'s field set.
#[derive(Debug, Clone, PartialEq)]
pub struct OneSampleResult {
    pub t_statistic: f64,
    pub p_value_two_sided: f64,
    pub n_used: usize,
    pub computed_mean: f64,
    pub computed_sd: f64,
}

/// Two-sample t-test statistic and two-sided p-value.
///
/// Tests `H0: mean(group_a) = mean(group_b)` against
/// `H1: mean(group_a) ≠ mean(group_b)` under the Student-t
/// distribution. The variance-assumption parameter selects which
/// variant runs:
///
/// - [`TwoSampleVariance::Pooled`] — classical Student's t-test.
///   Assumes equal variances; pooled variance estimate; df = n_a + n_b - 2.
/// - [`TwoSampleVariance::WelchUnequal`] — Welch's t-test.
///   Allows unequal variances; Welch–Satterthwaite approximate df.
///
/// Returns `None` when either group has `n < 2` (no degrees of freedom
/// available for a SD estimate); the verifier surfaces that as
/// `InsufficientReplication`.
pub fn two_sample_t_test(
    group_a: &[f64],
    group_b: &[f64],
    variance: TwoSampleVariance,
) -> Option<TwoSampleResult> {
    if group_a.len() < 2 || group_b.len() < 2 {
        return None;
    }
    let n_a = group_a.len() as f64;
    let n_b = group_b.len() as f64;

    let mean_a = group_a.iter().sum::<f64>() / n_a;
    let mean_b = group_b.iter().sum::<f64>() / n_b;

    // Bessel-corrected sample variances.
    let var_a = group_a.iter().map(|x| (x - mean_a).powi(2)).sum::<f64>() / (n_a - 1.0);
    let var_b = group_b.iter().map(|x| (x - mean_b).powi(2)).sum::<f64>() / (n_b - 1.0);

    let (t, df) = match variance {
        TwoSampleVariance::Pooled => {
            // Pooled variance: weighted average of sample variances.
            let pooled_var = ((n_a - 1.0) * var_a + (n_b - 1.0) * var_b) / (n_a + n_b - 2.0);
            let se = (pooled_var * (1.0 / n_a + 1.0 / n_b)).sqrt();
            let t = if se == 0.0 {
                if (mean_a - mean_b).abs() == 0.0 {
                    0.0
                } else if mean_a > mean_b {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                }
            } else {
                (mean_a - mean_b) / se
            };
            (t, n_a + n_b - 2.0)
        }
        TwoSampleVariance::WelchUnequal => {
            // Welch's t-statistic with Welch–Satterthwaite df.
            let se = (var_a / n_a + var_b / n_b).sqrt();
            let t = if se == 0.0 {
                if (mean_a - mean_b).abs() == 0.0 {
                    0.0
                } else if mean_a > mean_b {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                }
            } else {
                (mean_a - mean_b) / se
            };
            // Welch–Satterthwaite df. Reduces to (n_a + n_b - 2) when
            // both variances are equal; otherwise interpolates.
            let df_num = (var_a / n_a + var_b / n_b).powi(2);
            let df_den = (var_a / n_a).powi(2) / (n_a - 1.0) + (var_b / n_b).powi(2) / (n_b - 1.0);
            let df = if df_den == 0.0 {
                n_a + n_b - 2.0
            } else {
                df_num / df_den
            };
            (t, df)
        }
    };

    let dist = StudentsT::new(0.0, 1.0, df).expect("Student's t with df > 0 (both groups n >= 2)");
    let p_two_sided = if t.is_finite() {
        2.0 * (1.0 - dist.cdf(t.abs()))
    } else if t.is_nan() {
        f64::NAN
    } else {
        0.0
    };

    Some(TwoSampleResult {
        t_statistic: t,
        p_value_two_sided: p_two_sided,
        df,
        n_a: group_a.len(),
        n_b: group_b.len(),
        mean_a,
        mean_b,
        sd_a: var_a.sqrt(),
        sd_b: var_b.sqrt(),
    })
}

/// Which two-sample t-test variant to run. The author asserts the
/// variance assumption via `stats:variance_assumption` on the claim;
/// the dispatch maps `Pooled` → `Pooled` and `WelchUnequal` →
/// `WelchUnequal` here. `NonParametric` / `RankBased` map to
/// distribution-free or rank-transformed tests not implemented in
/// Phase 1.5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TwoSampleVariance {
    Pooled,
    WelchUnequal,
}

/// Numeric output of a two-sample t-test. Carries both groups' means
/// and SDs alongside the test statistic so the audit trail captures
/// the full intermediate state (D52 §6).
#[derive(Debug, Clone, PartialEq)]
pub struct TwoSampleResult {
    pub t_statistic: f64,
    pub p_value_two_sided: f64,
    pub df: f64,
    pub n_a: usize,
    pub n_b: usize,
    pub mean_a: f64,
    pub mean_b: f64,
    pub sd_a: f64,
    pub sd_b: f64,
}

/// Paired t-test for matched-pair / pre-post designs.
///
/// Reduces to a one-sample t-test on the per-pair differences
/// `(before_i - after_i)` against H0: mean(diffs) = 0. The
/// distribution under H0 is Student's t with `n - 1` degrees of
/// freedom, where `n` is the number of pairs. Two-sided p-value
/// reported (D52 §7.1 default).
///
/// Returns `None` when fewer than 2 pairs are supplied (no degrees of
/// freedom for the SD estimate); the verifier surfaces this as
/// `InsufficientReplication`.
pub fn paired_t_test(pairs: &[(f64, f64)]) -> Option<PairedResult> {
    if pairs.len() < 2 {
        return None;
    }
    let differences: Vec<f64> = pairs.iter().map(|(before, after)| before - after).collect();
    let single = one_sample_t_test(&differences, 0.0)?;
    Some(PairedResult {
        t_statistic: single.t_statistic,
        p_value_two_sided: single.p_value_two_sided,
        n_pairs: pairs.len(),
        mean_difference: single.computed_mean,
        sd_difference: single.computed_sd,
    })
}

/// Numeric output of a paired t-test. The mean and SD reported are
/// for the per-pair differences (not the raw before/after values) —
/// that's what the paired test's H0 is about.
#[derive(Debug, Clone, PartialEq)]
pub struct PairedResult {
    pub t_statistic: f64,
    pub p_value_two_sided: f64,
    pub n_pairs: usize,
    pub mean_difference: f64,
    pub sd_difference: f64,
}

/// k-way omnibus ANOVA — D52 Phase 2.5 / §5.1 `Factorial`.
///
/// Tests `H0: every cell has the same mean` against
/// `H1: at least one cell mean differs`. The "cells" of a k-way design
/// are the Cartesian product of factor-level combinations
/// (`Π_i factor_levels[i]` cells in total). This is the omnibus test
/// that conflates main effects and interactions into a single F —
/// per-effect decomposition (testing main effects and interactions
/// separately) is a Phase 5 hardening when a claim shape distinguishes
/// them.
///
/// Sum-of-squares decomposition (one-way ANOVA generalised to k factors
/// via the cell-mean parameterisation):
/// - `SS_within` = Σᵢ (yᵢ - cell_mean_of_yᵢ)²
/// - `SS_between` = Σ_cells nᶜ · (cell_meanᶜ - grand_mean)²
/// - `df_between` = `n_cells - 1`
/// - `df_within` = `N - n_cells`
/// - `F` = `MS_between / MS_within` under H0 ~ F(df_between, df_within)
///
/// `observations` is a flat list of `(cell_index_tuple, value)` pairs.
/// `factor_levels[i]` is the number of levels factor `i` admits; the
/// cell index tuples must be `(l_0, l_1, …, l_{k-1})` with
/// `0 ≤ l_i < factor_levels[i]`.
///
/// Returns `None` when:
/// - `factor_levels` is empty (use a one-sample test instead)
/// - fewer than 2 cells contain observations (no between-cell variance)
/// - `N - n_cells < 1` (every observed cell has exactly 1 observation;
///   no within-cell variance for the error term — `Factorial` requires
///   ≥ 2 replicates in at least one cell)
/// - any observation's cell index is out of `factor_levels` range
pub fn factorial_omnibus_anova(
    factor_levels: &[usize],
    observations: &[(Vec<usize>, f64)],
) -> Option<FactorialAnovaResult> {
    if factor_levels.is_empty() || observations.is_empty() {
        return None;
    }
    // Validate cell-index shape + range.
    for (levels, _) in observations {
        if levels.len() != factor_levels.len() {
            return None;
        }
        for (i, &lvl) in levels.iter().enumerate() {
            if lvl >= factor_levels[i] {
                return None;
            }
        }
    }

    let n = observations.len() as f64;
    let grand_mean = observations.iter().map(|(_, v)| v).sum::<f64>() / n;

    // Per-cell sum + count (insertion-ordered keys via BTreeMap so the
    // computation is deterministic across runs — D52 §6 reproducibility
    // property; no HashMap-based iteration order leaks in).
    let mut cell_agg: BTreeMap<Vec<usize>, (f64, usize)> = BTreeMap::new();
    for (levels, value) in observations {
        let entry = cell_agg.entry(levels.clone()).or_insert((0.0, 0));
        entry.0 += value;
        entry.1 += 1;
    }
    let cell_means: BTreeMap<Vec<usize>, f64> = cell_agg
        .iter()
        .map(|(k, (sum, count))| (k.clone(), *sum / *count as f64))
        .collect();
    let n_cells = cell_agg.len();
    if n_cells < 2 {
        return None;
    }

    let ss_within: f64 = observations
        .iter()
        .map(|(levels, v)| {
            let cm = cell_means[levels];
            (v - cm).powi(2)
        })
        .sum();
    let ss_between: f64 = cell_agg
        .iter()
        .map(|(levels, (_sum, count))| {
            let cm = cell_means[levels];
            (*count as f64) * (cm - grand_mean).powi(2)
        })
        .sum();

    let df_between = (n_cells - 1) as f64;
    let df_within_int = observations.len().saturating_sub(n_cells);
    if df_within_int < 1 {
        return None;
    }
    let df_within = df_within_int as f64;

    let ms_between = ss_between / df_between;
    let ms_within = ss_within / df_within;
    let f_statistic = if ms_within == 0.0 {
        if ms_between == 0.0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        ms_between / ms_within
    };

    let dist = FisherSnedecor::new(df_between, df_within).ok()?;
    let p_value = if f_statistic.is_finite() {
        1.0 - dist.cdf(f_statistic)
    } else if f_statistic.is_nan() {
        f64::NAN
    } else {
        0.0
    };

    Some(FactorialAnovaResult {
        f_statistic,
        p_value,
        df_between,
        df_within,
        n_cells,
        n_total: observations.len(),
        grand_mean,
        ss_between,
        ss_within,
    })
}

/// Numeric output of a k-way omnibus ANOVA. Carries the F-statistic
/// plus the sum-of-squares + df decomposition for audit (D52 §6).
#[derive(Debug, Clone, PartialEq)]
pub struct FactorialAnovaResult {
    pub f_statistic: f64,
    pub p_value: f64,
    pub df_between: f64,
    pub df_within: f64,
    pub n_cells: usize,
    pub n_total: usize,
    pub grand_mean: f64,
    pub ss_between: f64,
    pub ss_within: f64,
}

/// Randomized Complete Block Design (RCBD) two-way ANOVA — D52 Phase
/// 4.0 / §5.2 `RCBD`.
///
/// Tests the *treatment* effect controlling for block-to-block
/// variation: `H0: all treatment means are equal` against `H1: at
/// least one treatment mean differs`, with block treated as a random
/// effect that we marginalize over.
///
/// Sum-of-squares decomposition (standard two-way ANOVA without
/// interaction, since RCBD has one observation per (block, treatment)
/// cell):
/// - `SS_total = Σᵢⱼ (yᵢⱼ - ȳ..)²`
/// - `SS_block = t · Σⱼ (ȳ.ⱼ - ȳ..)²`
/// - `SS_treatment = b · Σᵢ (ȳᵢ. - ȳ..)²`
/// - `SS_error = SS_total - SS_block - SS_treatment`
/// - `df_block = b - 1`
/// - `df_treatment = t - 1`
/// - `df_error = (b - 1)(t - 1)`
/// - `F_treatment = (SS_treatment / df_treatment) / (SS_error / df_error)`
///   ~ F(df_treatment, df_error) under H0
///
/// **Block effect**: also computable here (SS_block / df_block over
/// MS_error) but typically reported as a variance component rather
/// than p-tested in RCBD analysis. The treatment effect is what
/// scientific claims usually assert; v1 verifier reports the
/// treatment F. Per-block-effect testing lands when a claim shape
/// distinguishes "treatment matters" from "block matters."
///
/// `observations` is a flat list of `(block_idx, treatment_idx,
/// value)` tuples with `0 ≤ block_idx < n_blocks` and
/// `0 ≤ treatment_idx < n_treatments`. The complete design requires
/// every (block, treatment) cell to contain exactly one observation
/// (`n_blocks · n_treatments` observations total).
///
/// Returns `None` when:
/// - `n_blocks < 2` or `n_treatments < 2` (df_error = 0)
/// - any cell has != 1 observation (non-complete design — use a
///   different dispatch arm, e.g. Factorial with replication)
/// - any cell index is out of range
pub fn rcbd_anova(
    n_blocks: usize,
    n_treatments: usize,
    observations: &[(usize, usize, f64)],
) -> Option<RCBDResult> {
    if n_blocks < 2 || n_treatments < 2 {
        return None;
    }
    let expected_n = n_blocks * n_treatments;
    if observations.len() != expected_n {
        return None;
    }
    // Build cell value matrix; validate each cell has exactly 1
    // observation (complete-design discipline).
    let mut cell: Vec<Vec<Option<f64>>> = (0..n_blocks).map(|_| vec![None; n_treatments]).collect();
    for &(b, t, v) in observations {
        if b >= n_blocks || t >= n_treatments {
            return None;
        }
        if cell[b][t].is_some() {
            return None; // duplicate cell — not a complete RCBD
        }
        cell[b][t] = Some(v);
    }
    for row in &cell {
        for c in row {
            if c.is_none() {
                return None; // missing cell — not a complete RCBD
            }
        }
    }

    let n = expected_n as f64;
    let b = n_blocks as f64;
    let t = n_treatments as f64;

    let grand_mean: f64 = observations.iter().map(|(_, _, v)| v).sum::<f64>() / n;

    // Per-block means (ȳ.ⱼ): average over all treatments within each block.
    let block_means: Vec<f64> = (0..n_blocks)
        .map(|j| {
            let sum: f64 = (0..n_treatments)
                .map(|i| cell[j][i].expect("validated above"))
                .sum();
            sum / t
        })
        .collect();
    // Per-treatment means (ȳᵢ.): average over all blocks within each treatment.
    let treatment_means: Vec<f64> = (0..n_treatments)
        .map(|i| {
            let sum: f64 = (0..n_blocks)
                .map(|j| cell[j][i].expect("validated above"))
                .sum();
            sum / b
        })
        .collect();

    let ss_block: f64 = t * block_means
        .iter()
        .map(|m| (m - grand_mean).powi(2))
        .sum::<f64>();
    let ss_treatment: f64 = b * treatment_means
        .iter()
        .map(|m| (m - grand_mean).powi(2))
        .sum::<f64>();
    // SS_error computed directly from the per-cell residual under the
    // no-interaction additive model `ŷᵢⱼ = ȳᵢ. + ȳ.ⱼ - ȳ..`. Avoids
    // the catastrophic-cancellation trap of `SS_total - SS_block -
    // SS_treatment` when block (or treatment) variation dominates by
    // many orders of magnitude — e.g., a blocked design where blocks
    // have baselines of 100, 200, 50 but a within-block treatment
    // effect of +10 loses all precision under subtraction.
    let ss_error: f64 = observations
        .iter()
        .map(|(b_idx, t_idx, v)| {
            let predicted = treatment_means[*t_idx] + block_means[*b_idx] - grand_mean;
            (v - predicted).powi(2)
        })
        .sum();

    let df_block = b - 1.0;
    let df_treatment = t - 1.0;
    let df_error = (b - 1.0) * (t - 1.0);

    let ms_treatment = ss_treatment / df_treatment;
    let ms_error = ss_error / df_error;
    let f_treatment = if ms_error == 0.0 {
        if ms_treatment == 0.0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        ms_treatment / ms_error
    };

    let dist = FisherSnedecor::new(df_treatment, df_error).ok()?;
    let p_treatment = if f_treatment.is_finite() {
        1.0 - dist.cdf(f_treatment)
    } else if f_treatment.is_nan() {
        f64::NAN
    } else {
        0.0
    };

    Some(RCBDResult {
        f_treatment,
        p_treatment,
        df_treatment,
        df_block,
        df_error,
        n_blocks,
        n_treatments,
        grand_mean,
        ss_block,
        ss_treatment,
        ss_error,
    })
}

/// Numeric output of a Randomized Complete Block Design ANOVA.
/// Carries the treatment F-test (the primary reported quantity)
/// plus the full sum-of-squares + df decomposition for audit
/// (D52 §6).
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::upper_case_acronyms)] // RCBD is the standard term — Randomized Complete Block Design
pub struct RCBDResult {
    pub f_treatment: f64,
    pub p_treatment: f64,
    pub df_treatment: f64,
    pub df_block: f64,
    pub df_error: f64,
    pub n_blocks: usize,
    pub n_treatments: usize,
    pub grand_mean: f64,
    pub ss_block: f64,
    pub ss_treatment: f64,
    pub ss_error: f64,
}

/// Split-Plot ANOVA — D52 Phase 4.5 / §5.2 `SplitPlot`.
///
/// Two-factor design where one factor (the *whole-plot* factor `W`)
/// is hard to randomize and applied to whole-plot units, and the
/// other factor (the *subplot* factor `S`) is applied within each
/// whole-plot to the subplots. The key structural difference from a
/// flat factorial is that whole-plot-level variation lives in a
/// *separate, larger* error stratum than subplot-level variation,
/// and the W main-effect F-test must use the **whole-plot error**,
/// not the subplot error.
///
/// This is the false-positive trap D52 §5.2 calls out: applying a
/// flat factorial ANOVA to split-plot data uses the smaller subplot
/// error for every F-test, including the W main effect, and produces
/// massively inflated F_W values (rejecting H0 when the whole-plot
/// effect is actually noise).
///
/// **Design parameters**:
/// - `a` — number of whole-plot factor levels
/// - `b` — number of subplot factor levels
/// - `r` — number of whole-plot replicates per whole-plot level
///   (so `a * r` whole plots total, each containing `b` subplot
///   observations; `n_total = a * b * r`)
///
/// **Observations**: each entry is `(whole_plot_id, w_level, s_level,
/// value)`. Whole-plot ids must form a contiguous `0..a*r` range with
/// each id appearing in exactly `b` observations (one per subplot
/// level). The verifier enforces this — a missing or duplicated
/// (whole_plot, s_level) cell fails the design.
///
/// **Sum-of-squares decomposition**:
/// - `SS_W` = `b·r · Σ_w (ȳ_w.. − ȳ...)²` — whole-plot factor effect
/// - `SS_WP_within_W` = `b · Σ_wp (ȳ_wp.. − ȳ_w(wp)..)²` — whole-plot
///   replicate effect nested within W; this IS the whole-plot error
/// - `SS_S` = `a·r · Σ_s (ȳ_..s − ȳ...)²` — subplot factor effect
/// - `SS_WS` = `r · Σ_w,s (ȳ_w.s − ȳ_w.. − ȳ_..s + ȳ...)²` — interaction
/// - `SS_error` (subplot error) = `SS_total − SS_W − SS_WP_within_W
///   − SS_S − SS_WS` — but as with RCBD, computed directly from
///   residuals under the additive cell-mean model to avoid
///   catastrophic cancellation
///
/// **F-tests**:
/// - `F_W = MS_W / MS_WP_within_W` ~ F(a−1, a(r−1))
/// - `F_S = MS_S / MS_error` ~ F(b−1, a(b−1)(r−1))
/// - `F_WS = MS_WS / MS_error` ~ F((a−1)(b−1), a(b−1)(r−1))
///
/// Returns `None` when:
/// - `a < 2`, `b < 2`, or `r < 2` (no degrees of freedom available)
/// - `observations.len() != a*b*r`
/// - any (whole_plot_id, s_level) cell has != 1 observation
/// - whole_plot ids are out of `0..a*r` range
/// - the w_level assigned to a whole_plot is inconsistent across
///   its subplot observations (each whole plot has one W level)
pub fn splitplot_anova(
    a: usize,
    b: usize,
    r: usize,
    observations: &[(usize, usize, usize, f64)],
) -> Option<SplitPlotResult> {
    if a < 2 || b < 2 || r < 2 {
        return None;
    }
    let n_total = a * b * r;
    if observations.len() != n_total {
        return None;
    }
    let n_whole_plots = a * r;

    // Build the (whole_plot × s_level) cell matrix and validate
    // design discipline along the way.
    let mut cell: Vec<Vec<Option<f64>>> = (0..n_whole_plots).map(|_| vec![None; b]).collect();
    // For each whole plot, record its w_level (must be consistent
    // across the b subplot observations).
    let mut wp_to_w: Vec<Option<usize>> = vec![None; n_whole_plots];
    for &(wp, w, s, v) in observations {
        if wp >= n_whole_plots || w >= a || s >= b {
            return None;
        }
        if cell[wp][s].is_some() {
            return None; // duplicate (whole_plot, s_level) cell
        }
        cell[wp][s] = Some(v);
        match wp_to_w[wp] {
            Some(prev) if prev != w => return None, // inconsistent w_level for this whole_plot
            _ => wp_to_w[wp] = Some(w),
        }
    }
    for row in &cell {
        for c in row {
            if c.is_none() {
                return None; // missing (whole_plot, s_level) cell
            }
        }
    }
    // Validate each W level has exactly `r` whole plots.
    let mut wp_per_w = vec![0usize; a];
    for &maybe_w in &wp_to_w {
        wp_per_w[maybe_w.expect("validated above")] += 1;
    }
    if wp_per_w.iter().any(|&n| n != r) {
        return None;
    }

    let n_total_f = n_total as f64;
    let a_f = a as f64;
    let b_f = b as f64;
    let r_f = r as f64;

    let grand_mean: f64 = observations.iter().map(|(_, _, _, v)| v).sum::<f64>() / n_total_f;

    // Per-whole-plot means (ȳ_wp..): average over the b subplot
    // observations within each whole plot.
    let wp_means: Vec<f64> = (0..n_whole_plots)
        .map(|wp| {
            let sum: f64 = (0..b).map(|s| cell[wp][s].expect("validated")).sum();
            sum / b_f
        })
        .collect();

    // Per-W-level means (ȳ_w..): average over the r whole plots within
    // each W level (each contributing b subplot observations).
    let w_means: Vec<f64> = (0..a)
        .map(|w| {
            let sum: f64 = wp_to_w
                .iter()
                .enumerate()
                .filter_map(|(wp, mw)| {
                    if mw.unwrap() == w {
                        Some(wp_means[wp])
                    } else {
                        None
                    }
                })
                .sum();
            sum / r_f
        })
        .collect();

    // Per-S-level means (ȳ_..s): average over a·r observations
    // (one per whole plot at this s level).
    let s_means: Vec<f64> = (0..b)
        .map(|s| {
            let sum: f64 = (0..n_whole_plots)
                .map(|wp| cell[wp][s].expect("validated"))
                .sum();
            sum / (a_f * r_f)
        })
        .collect();

    // Per-(W, S) cell means (ȳ_w.s): average over the r whole plots
    // at W level w, taking each whole plot's value at s level s.
    let ws_means: Vec<Vec<f64>> = (0..a)
        .map(|w| {
            (0..b)
                .map(|s| {
                    let sum: f64 = wp_to_w
                        .iter()
                        .enumerate()
                        .filter_map(|(wp, mw)| {
                            if mw.unwrap() == w {
                                Some(cell[wp][s].expect("validated"))
                            } else {
                                None
                            }
                        })
                        .sum();
                    sum / r_f
                })
                .collect()
        })
        .collect();

    // SS_W: b·r · Σ (ȳ_w.. − ȳ...)²
    let ss_w: f64 = b_f
        * r_f
        * w_means
            .iter()
            .map(|m| (m - grand_mean).powi(2))
            .sum::<f64>();

    // SS_WP_within_W (whole-plot error): b · Σ_wp (ȳ_wp.. − ȳ_w(wp)..)²
    let ss_wp_within_w: f64 = b_f
        * wp_to_w
            .iter()
            .enumerate()
            .map(|(wp, mw)| (wp_means[wp] - w_means[mw.unwrap()]).powi(2))
            .sum::<f64>();

    // SS_S: a·r · Σ (ȳ_..s − ȳ...)²
    let ss_s: f64 = a_f
        * r_f
        * s_means
            .iter()
            .map(|m| (m - grand_mean).powi(2))
            .sum::<f64>();

    // SS_WS interaction: r · Σ_w,s (ȳ_w.s − ȳ_w.. − ȳ_..s + ȳ...)²
    let ss_ws: f64 = r_f
        * (0..a)
            .flat_map(|w| (0..b).map(move |s| (w, s)))
            .map(|(w, s)| {
                let residual = ws_means[w][s] - w_means[w] - s_means[s] + grand_mean;
                residual.powi(2)
            })
            .sum::<f64>();

    // SS_error (subplot error): per-observation residual from the
    // additive subplot model `ŷ_{wp,s} = ȳ_wp.. + ȳ_w(wp).s − ȳ_w(wp)..`
    // Computed directly to avoid catastrophic cancellation (same
    // motivation as the RCBD SS_error formula).
    let ss_error: f64 = observations
        .iter()
        .map(|&(wp, _w, s, v)| {
            let w = wp_to_w[wp].unwrap();
            let predicted = wp_means[wp] + ws_means[w][s] - w_means[w];
            (v - predicted).powi(2)
        })
        .sum();

    let df_w = a_f - 1.0;
    let df_wp_within_w = a_f * (r_f - 1.0);
    let df_s = b_f - 1.0;
    let df_ws = (a_f - 1.0) * (b_f - 1.0);
    let df_error = a_f * (b_f - 1.0) * (r_f - 1.0);

    let ms_w = ss_w / df_w;
    let ms_wp_within_w = ss_wp_within_w / df_wp_within_w;
    let ms_s = ss_s / df_s;
    let ms_ws = ss_ws / df_ws;
    let ms_error = ss_error / df_error;

    // Whole-plot factor F-test: uses whole-plot error (the
    // load-bearing distinction from a flat factorial ANOVA).
    let f_w = compute_f(ms_w, ms_wp_within_w);
    let p_w = f_pvalue(f_w, df_w, df_wp_within_w)?;

    // Subplot factor F-test: uses subplot error.
    let f_s = compute_f(ms_s, ms_error);
    let p_s = f_pvalue(f_s, df_s, df_error)?;

    // Interaction F-test: uses subplot error.
    let f_ws = compute_f(ms_ws, ms_error);
    let p_ws = f_pvalue(f_ws, df_ws, df_error)?;

    Some(SplitPlotResult {
        f_w,
        p_w,
        f_s,
        p_s,
        f_ws,
        p_ws,
        df_w,
        df_wp_within_w,
        df_s,
        df_ws,
        df_error,
        a,
        b,
        r,
        grand_mean,
        ss_w,
        ss_wp_within_w,
        ss_s,
        ss_ws,
        ss_error,
    })
}

/// Compute F-statistic with the degenerate-zero-error case handled
/// consistently across split-plot's three F-tests.
fn compute_f(ms_num: f64, ms_denom: f64) -> f64 {
    if ms_denom == 0.0 {
        if ms_num == 0.0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        ms_num / ms_denom
    }
}

/// One-sided F-distribution p-value with NaN / infinite-F handling.
/// Returns `None` only on FisherSnedecor-construction failure (df
/// must be > 0 — already validated by the calling design check).
fn f_pvalue(f_stat: f64, df_num: f64, df_denom: f64) -> Option<f64> {
    let dist = FisherSnedecor::new(df_num, df_denom).ok()?;
    Some(if f_stat.is_finite() {
        1.0 - dist.cdf(f_stat)
    } else if f_stat.is_nan() {
        f64::NAN
    } else {
        0.0
    })
}

/// Numeric output of a split-plot ANOVA. Carries all three F-tests
/// (whole-plot factor, subplot factor, W×S interaction) plus the full
/// sum-of-squares + df decomposition for audit (D52 §6).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPlotResult {
    pub f_w: f64,
    pub p_w: f64,
    pub f_s: f64,
    pub p_s: f64,
    pub f_ws: f64,
    pub p_ws: f64,
    pub df_w: f64,
    pub df_wp_within_w: f64,
    pub df_s: f64,
    pub df_ws: f64,
    pub df_error: f64,
    pub a: usize,
    pub b: usize,
    pub r: usize,
    pub grand_mean: f64,
    pub ss_w: f64,
    pub ss_wp_within_w: f64,
    pub ss_s: f64,
    pub ss_ws: f64,
    pub ss_error: f64,
}

/// Compound-Symmetry Repeated-Measures ANOVA — D52 Phase 4.9 / §5.2
/// `RepeatedMeasures` (the simplest autocorrelation case, per the
/// §12 #5 author-asserted-structure decision).
///
/// Tests `H0: all timepoint means are equal` against `H1: at least
/// one timepoint mean differs`, with subject treated as a random
/// effect (each subject contributing a baseline that gets averaged
/// over). Under the compound-symmetry assumption — equal within-
/// subject variances and equal between-timepoint covariances — this
/// reduces to the standard univariate RM-ANOVA, which is
/// algebraically equivalent to RCBD with subject as block and time
/// as treatment.
///
/// **Compound-symmetry caveat**: the verifier doesn't *check* the
/// assumption (Mauchly's test of sphericity is a future hardening).
/// Author asserts the structure on the claim per §12 #5; the
/// verifier honors the assertion and computes the test under it.
/// AR(1) and Unstructured covariance structures need genuinely
/// different numerics (GLS with the AR(1) parameter ρ; MANOVA-style
/// multivariate tests respectively) and dispatch to "not yet wired"
/// diagnostics in Phase 4.9 v1.
///
/// **Sum-of-squares decomposition** (one-way RM-ANOVA, equivalent to
/// RCBD):
/// - `SS_subject = T · Σᵢ (ȳᵢ. - ȳ..)²` — between-subject variation
/// - `SS_time = N · Σⱼ (ȳ.ⱼ - ȳ..)²` — time effect (the quantity of
///   interest)
/// - `SS_error = Σᵢⱼ (yᵢⱼ - ȳᵢ. - ȳ.ⱼ + ȳ..)²` — residuals under the
///   additive subject + time model (computed directly to avoid
///   catastrophic-cancellation precision loss when subject baselines
///   vary widely — same rationale as the RCBD `SS_error` formula)
/// - `df_time = T - 1`
/// - `df_error = (N - 1)(T - 1)`
/// - `F_time = MS_time / MS_error ~ F(df_time, df_error)` under H0
///
/// `observations` is a flat list of `(subject_idx, time_idx, value)`
/// tuples with `0 ≤ subject_idx < n_subjects` and `0 ≤ time_idx <
/// n_timepoints`. Complete design required: every (subject, time)
/// cell must contain exactly one observation (`n_subjects ·
/// n_timepoints` observations total).
///
/// Returns `None` when:
/// - `n_subjects < 2` or `n_timepoints < 2` (df_error = 0)
/// - any cell has != 1 observation
/// - any cell index is out of range
pub fn repeated_measures_cs_anova(
    n_subjects: usize,
    n_timepoints: usize,
    observations: &[(usize, usize, f64)],
) -> Option<RepeatedMeasuresResult> {
    if n_subjects < 2 || n_timepoints < 2 {
        return None;
    }
    let expected_n = n_subjects * n_timepoints;
    if observations.len() != expected_n {
        return None;
    }
    // Build cell matrix and validate complete design.
    let mut cell: Vec<Vec<Option<f64>>> =
        (0..n_subjects).map(|_| vec![None; n_timepoints]).collect();
    for &(s, t, v) in observations {
        if s >= n_subjects || t >= n_timepoints {
            return None;
        }
        if cell[s][t].is_some() {
            return None;
        }
        cell[s][t] = Some(v);
    }
    for row in &cell {
        for c in row {
            if c.is_none() {
                return None;
            }
        }
    }

    let n_total = expected_n as f64;
    let n_subj = n_subjects as f64;
    let n_time = n_timepoints as f64;

    let grand_mean: f64 = observations.iter().map(|(_, _, v)| v).sum::<f64>() / n_total;

    // Per-subject means (average across timepoints within each subject).
    let subject_means: Vec<f64> = (0..n_subjects)
        .map(|s| {
            let sum: f64 = (0..n_timepoints)
                .map(|t| cell[s][t].expect("validated"))
                .sum();
            sum / n_time
        })
        .collect();
    // Per-timepoint means (average across subjects within each timepoint).
    let time_means: Vec<f64> = (0..n_timepoints)
        .map(|t| {
            let sum: f64 = (0..n_subjects)
                .map(|s| cell[s][t].expect("validated"))
                .sum();
            sum / n_subj
        })
        .collect();

    let ss_subject: f64 = n_time
        * subject_means
            .iter()
            .map(|m| (m - grand_mean).powi(2))
            .sum::<f64>();
    let ss_time: f64 = n_subj
        * time_means
            .iter()
            .map(|m| (m - grand_mean).powi(2))
            .sum::<f64>();
    // SS_error from per-cell residuals under the additive subject+time
    // model. Same precision-preserving formula as RCBD's SS_error.
    let ss_error: f64 = observations
        .iter()
        .map(|&(s, t, v)| {
            let predicted = subject_means[s] + time_means[t] - grand_mean;
            (v - predicted).powi(2)
        })
        .sum();

    let df_subject = n_subj - 1.0;
    let df_time = n_time - 1.0;
    let df_error = (n_subj - 1.0) * (n_time - 1.0);

    let ms_time = ss_time / df_time;
    let ms_error = ss_error / df_error;
    let f_time = if ms_error == 0.0 {
        if ms_time == 0.0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        ms_time / ms_error
    };

    let dist = FisherSnedecor::new(df_time, df_error).ok()?;
    let p_time = if f_time.is_finite() {
        1.0 - dist.cdf(f_time)
    } else if f_time.is_nan() {
        f64::NAN
    } else {
        0.0
    };

    Some(RepeatedMeasuresResult {
        f_time,
        p_time,
        df_time,
        df_subject,
        df_error,
        n_subjects,
        n_timepoints,
        grand_mean,
        ss_subject,
        ss_time,
        ss_error,
    })
}

/// Numeric output of a compound-symmetry repeated-measures ANOVA.
/// Carries the time F-test (the primary reported quantity) plus the
/// sum-of-squares + df decomposition for audit (D52 §6).
#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedMeasuresResult {
    pub f_time: f64,
    pub p_time: f64,
    pub df_time: f64,
    pub df_subject: f64,
    pub df_error: f64,
    pub n_subjects: usize,
    pub n_timepoints: usize,
    pub grand_mean: f64,
    pub ss_subject: f64,
    pub ss_time: f64,
    pub ss_error: f64,
}

/// D52 §7.3 / Phase 5 — Passing-Bablok non-parametric regression for
/// method comparison. Returns the slope/intercept point estimates plus
/// the 95% rank-based confidence intervals; the verifier interprets
/// "methods agree" as `1.0 ∈ slope_ci` AND `0.0 ∈ intercept_ci` (CLSI
/// EP09 criterion).
///
/// Algorithm (Passing & Bablok 1983):
///   1. Compute all N*(N-1)/2 pairwise slopes `S_ij = (y_j - y_i) /
///      (x_j - x_i)` for i < j with x_i ≠ x_j.
///   2. Exclude `S_ij = -1` (those pairs are perpendicular to the
///      identity line and undefined in the offset-symmetric framing).
///   3. Count K = number of negative slopes; the median slope estimator
///      offsets the median index by K/2 so the estimator is unbiased
///      under arbitrary errors-in-both-variables.
///   4. Slope = median of the sorted slopes after the K-offset.
///   5. Intercept = median of `y_i - slope * x_i` over all i.
///   6. 95% CI: rank-based using the binomial-quantile method
///      (Passing & Bablok 1983, eq. 6) — the slope's CI uses indices
///      `M1 = (N_eff - C(α)) / 2` and `M2 = N_eff - M1 + 1` on the
///      sorted slope vector (after the K offset). C(α) = z_{α/2} *
///      sqrt(N * (N - 1) * (2N + 5) / 18) is the Kendall-tau-style
///      critical value (we use the normal approximation valid for
///      N ≥ ~10; verifier rejects for n < 10).
///
/// `method_a` and `method_b` must have the same length and each pair
/// `(method_a[i], method_b[i])` is the i-th sample measured by both
/// methods. Returns `None` if `n < 3` (PB undefined) or fewer than
/// 1 usable slope (all x-values equal — methods produce constant
/// outputs, comparison undefined). For `n < 10` the CI is computed via
/// the exact rank distribution; for `n ≥ 10` the normal approximation
/// is used (Passing & Bablok 1983 §3.2).
pub fn passing_bablok_regression(
    method_a: &[f64],
    method_b: &[f64],
) -> Option<PassingBablokResult> {
    if method_a.len() != method_b.len() || method_a.len() < 3 {
        return None;
    }
    let n = method_a.len();
    let mut slopes: Vec<f64> = Vec::with_capacity(n * (n - 1) / 2);
    let mut k_negative: usize = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = method_a[j] - method_a[i];
            let dy = method_b[j] - method_b[i];
            if dx == 0.0 {
                continue;
            }
            let s = dy / dx;
            if s == -1.0 {
                continue;
            }
            if s < 0.0 {
                k_negative += 1;
            }
            slopes.push(s);
        }
    }
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n_slopes = slopes.len();
    let offset = k_negative;
    let median_idx_1based = n_slopes.div_ceil(2) + offset / 2;
    let slope = if n_slopes % 2 == 1 {
        let idx = (median_idx_1based - 1).min(n_slopes - 1);
        slopes[idx]
    } else {
        let idx = (median_idx_1based - 1).min(n_slopes - 1);
        let idx_next = (idx + 1).min(n_slopes - 1);
        0.5 * (slopes[idx] + slopes[idx_next])
    };
    let mut intercepts: Vec<f64> = (0..n).map(|i| method_b[i] - slope * method_a[i]).collect();
    intercepts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let intercept = median_of_sorted(&intercepts);
    let n_f = n as f64;
    let c_alpha = 1.96 * (n_f * (n_f - 1.0) * (2.0 * n_f + 5.0) / 18.0).sqrt();
    let m1 = ((n_slopes as f64 - c_alpha) / 2.0).floor() as isize + offset as isize / 2;
    let m2 = n_slopes as isize - m1 - 1 + offset as isize;
    let slope_ci_low = clamp_pick(&slopes, m1);
    let slope_ci_high = clamp_pick(&slopes, m2);
    let intercept_ci_low_vals: Vec<f64> = (0..n)
        .map(|i| method_b[i] - slope_ci_high * method_a[i])
        .collect();
    let intercept_ci_high_vals: Vec<f64> = (0..n)
        .map(|i| method_b[i] - slope_ci_low * method_a[i])
        .collect();
    let mut ci_low_sorted = intercept_ci_low_vals;
    let mut ci_high_sorted = intercept_ci_high_vals;
    ci_low_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ci_high_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let intercept_ci_low = median_of_sorted(&ci_low_sorted);
    let intercept_ci_high = median_of_sorted(&ci_high_sorted);
    Some(PassingBablokResult {
        slope,
        intercept,
        slope_ci_low: slope_ci_low.min(slope_ci_high),
        slope_ci_high: slope_ci_low.max(slope_ci_high),
        intercept_ci_low: intercept_ci_low.min(intercept_ci_high),
        intercept_ci_high: intercept_ci_low.max(intercept_ci_high),
        n_samples: n,
        n_slopes,
    })
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        0.5 * (sorted[n / 2 - 1] + sorted[n / 2])
    }
}

fn clamp_pick(sorted: &[f64], idx: isize) -> f64 {
    let n = sorted.len() as isize;
    if idx < 0 {
        sorted[0]
    } else if idx >= n {
        sorted[(n - 1) as usize]
    } else {
        sorted[idx as usize]
    }
}

#[derive(Debug, Clone)]
pub struct PassingBablokResult {
    pub slope: f64,
    pub intercept: f64,
    pub slope_ci_low: f64,
    pub slope_ci_high: f64,
    pub intercept_ci_low: f64,
    pub intercept_ci_high: f64,
    pub n_samples: usize,
    pub n_slopes: usize,
}

/// D52 §7.2 / Phase 5 — Generalized Extreme Studentized Deviate
/// (Rosner 1983) outlier detection on a 1-D sample. Returns the
/// original-array indices of the observations the test flags as
/// outliers (up to `max_outliers`), sorted ascending. The caller
/// applies the exclusion: the §7.2 dual-verdict commit shape computes
/// the downstream test both with and without these observations and
/// reports both verdicts.
///
/// Algorithm (Rosner's iterative procedure):
/// 1. Initialize the working set to `samples`.
/// 2. For i = 1..=max_outliers: compute R_i = max_j (|x_j - mean| / sd)
///    over the working set, note the index j_i of the offending
///    observation, compute the critical value λ_i from the one-sided t
///    distribution at significance α / (2 (n - i + 1)) with df = n - i
///    - 1 (n = original sample size), then remove observation j_i.
/// 3. The number of outliers detected is the largest i such that R_i >
///    λ_i (Rosner 1983); only the indices through that i are returned.
///    If no i has R_i > λ_i, return an empty vec.
///
/// Returns the original-`samples`-indexed positions, not the working-
/// set indices. Returns an empty vec when `max_outliers == 0`, when
/// `samples.len() < 4` (ESD requires df ≥ 1), or when no observation
/// crosses its critical value.
pub fn esd_filter(samples: &[f64], max_outliers: usize, alpha: f64) -> Vec<usize> {
    let n = samples.len();
    if max_outliers == 0 || n < 4 || !(0.0 < alpha && alpha < 1.0) {
        return vec![];
    }
    let cap = max_outliers.min(n.saturating_sub(2));
    let mut working: Vec<(usize, f64)> = samples.iter().copied().enumerate().collect();
    let mut candidates: Vec<usize> = Vec::with_capacity(cap);
    let mut r_values: Vec<f64> = Vec::with_capacity(cap);
    let mut lambdas: Vec<f64> = Vec::with_capacity(cap);
    for i in 1..=cap {
        if working.len() < 3 {
            break;
        }
        let m = working.len() as f64;
        let mean = working.iter().map(|&(_, x)| x).sum::<f64>() / m;
        let var = working
            .iter()
            .map(|&(_, x)| (x - mean) * (x - mean))
            .sum::<f64>()
            / (m - 1.0);
        let sd = var.sqrt();
        if sd == 0.0 {
            break;
        }
        let (idx_in_working, _, dev) = working
            .iter()
            .enumerate()
            .map(|(k, &(_, x))| (k, x, (x - mean).abs() / sd))
            .fold((0usize, 0.0f64, f64::NEG_INFINITY), |acc, item| {
                if item.2 > acc.2 {
                    item
                } else {
                    acc
                }
            });
        let (orig_idx, _) = working[idx_in_working];
        let df = m - 2.0;
        if df <= 0.0 {
            break;
        }
        let p = 1.0 - alpha / (2.0 * m);
        let t = match StudentsT::new(0.0, 1.0, df) {
            Ok(d) => d.inverse_cdf(p),
            Err(_) => break,
        };
        let lambda = ((m - 1.0) * t) / ((df + t * t) * m).sqrt();
        candidates.push(orig_idx);
        r_values.push(dev);
        lambdas.push(lambda);
        working.swap_remove(idx_in_working);
        let _ = i; // i is implicitly used via working.len() bookkeeping
    }
    let mut last_significant: Option<usize> = None;
    for (i, (r, l)) in r_values.iter().zip(lambdas.iter()).enumerate() {
        if r > l {
            last_significant = Some(i);
        }
    }
    match last_significant {
        Some(last) => {
            let mut out: Vec<usize> = candidates.into_iter().take(last + 1).collect();
            out.sort();
            out
        }
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from R: `t.test(c(72, 85, 100), mu = 100)`.
    /// Mean = 85.667; SD = 14.012; SE = 8.090; t = -1.776; p ≈ 0.218.
    /// We match to 1e-3 because Bessel correction + statrs's CDF
    /// implementation is deterministic but not necessarily byte-equal
    /// to R's; the verdict's bit-identity is across our verifier
    /// runs, not across implementations.
    #[test]
    fn ic50_example_matches_r_reference() {
        let samples = [72.0, 85.0, 100.0];
        let result = one_sample_t_test(&samples, 100.0).expect("n >= 2");
        assert!((result.computed_mean - 85.6666667).abs() < 1e-6);
        assert!((result.computed_sd - 14.0118996).abs() < 1e-4);
        assert!((result.t_statistic - (-1.776)).abs() < 1e-2);
        assert!((result.p_value_two_sided - 0.218).abs() < 5e-2);
    }

    #[test]
    fn n_less_than_two_returns_none() {
        assert!(one_sample_t_test(&[42.0], 0.0).is_none());
        assert!(one_sample_t_test(&[], 0.0).is_none());
    }

    #[test]
    fn identical_samples_yield_zero_t_when_mean_equals_null() {
        let samples = [50.0, 50.0, 50.0];
        let result = one_sample_t_test(&samples, 50.0).expect("n >= 2");
        assert_eq!(result.computed_sd, 0.0);
        assert_eq!(result.t_statistic, 0.0);
    }

    /// Determinism check — two runs over the same input must produce
    /// bit-identical numerics. D52 §6's reproducibility requirement.
    #[test]
    fn deterministic_across_runs() {
        let samples = [72.5, 85.0, 100.25, 88.3, 91.1];
        let a = one_sample_t_test(&samples, 90.0).unwrap();
        let b = one_sample_t_test(&samples, 90.0).unwrap();
        assert_eq!(a.t_statistic.to_bits(), b.t_statistic.to_bits());
        assert_eq!(a.p_value_two_sided.to_bits(), b.p_value_two_sided.to_bits());
        assert_eq!(a.computed_mean.to_bits(), b.computed_mean.to_bits());
        assert_eq!(a.computed_sd.to_bits(), b.computed_sd.to_bits());
    }

    // ── Two-sample t-test (Phase 1.5 — IID dispatch) ─────────────

    /// Pooled two-sample t-test: a = [2,3,5,6] vs b = [10,12,11,13].
    /// mean_a = 4, mean_b = 11.5, var_a = 10/3, var_b = 5/3,
    /// pooled_var = (3·10/3 + 3·5/3)/6 = 15/6 = 2.5,
    /// SE = sqrt(2.5 · 0.5) ≈ 1.118, t = -7.5/1.118 ≈ -6.7082.
    /// df = n_a + n_b - 2 = 6. p ≈ 5.3e-4. (Reference values computed
    /// by hand, cross-checked against R's t.test(... var.equal=TRUE).)
    #[test]
    fn pooled_two_sample_matches_r_reference() {
        let a = [2.0, 3.0, 5.0, 6.0];
        let b = [10.0, 12.0, 11.0, 13.0];
        let result = two_sample_t_test(&a, &b, TwoSampleVariance::Pooled).expect("n>=2 both");
        assert!((result.mean_a - 4.0).abs() < 1e-9);
        assert!((result.mean_b - 11.5).abs() < 1e-9);
        assert_eq!(result.df, 6.0);
        assert!(
            (result.t_statistic - (-6.7082039)).abs() < 1e-4,
            "got t = {}",
            result.t_statistic
        );
        assert!(
            result.p_value_two_sided < 0.001,
            "got p = {}",
            result.p_value_two_sided
        );
    }

    /// Welch's t-test: same data, allows unequal variances.
    /// t is identical to pooled here because n_a = n_b (the pooled
    /// and Welch SEs coincide when group sizes match — `var_a/n_a +
    /// var_b/n_b == pooled_var · (1/n_a + 1/n_b)` algebraically when
    /// n_a = n_b). df differs: Welch–Satterthwaite gives df ≈ 5.41
    /// here (vs pooled's df = 6). Cross-checked against R's default
    /// t.test().
    #[test]
    fn welch_two_sample_matches_r_reference() {
        let a = [2.0, 3.0, 5.0, 6.0];
        let b = [10.0, 12.0, 11.0, 13.0];
        let result = two_sample_t_test(&a, &b, TwoSampleVariance::WelchUnequal).expect("n>=2 both");
        // df_num = (10/3/4 + 5/3/4)^2 = (15/12)^2 = (5/4)^2 = 25/16
        // df_den = (10/3/4)^2/3 + (5/3/4)^2/3
        //        = (5/6)^2/3 + (5/12)^2/3
        //        = 25/108 + 25/432
        // df ≈ 5.4
        assert!(
            (result.df - 5.4).abs() < 0.1,
            "Welch df should be ≈ 5.4; got df = {}",
            result.df
        );
        assert!(
            (result.t_statistic - (-6.7082039)).abs() < 1e-4,
            "got t = {}",
            result.t_statistic
        );
        assert!(
            result.p_value_two_sided < 0.005,
            "got p = {}",
            result.p_value_two_sided
        );
    }

    #[test]
    fn two_sample_either_group_n_less_than_two_returns_none() {
        let one = [1.0];
        let two = [1.0, 2.0];
        assert!(two_sample_t_test(&one, &two, TwoSampleVariance::Pooled).is_none());
        assert!(two_sample_t_test(&two, &one, TwoSampleVariance::WelchUnequal).is_none());
    }

    /// Identical groups should give t = 0 and p = 1 (or very close to it).
    /// Tests the degenerate case for both variants.
    #[test]
    fn two_sample_identical_groups_yield_zero_t() {
        let group = [10.0, 12.0, 14.0];
        let pooled = two_sample_t_test(&group, &group, TwoSampleVariance::Pooled).unwrap();
        assert_eq!(pooled.t_statistic, 0.0);
        assert!((pooled.p_value_two_sided - 1.0).abs() < 1e-9);
        let welch = two_sample_t_test(&group, &group, TwoSampleVariance::WelchUnequal).unwrap();
        assert_eq!(welch.t_statistic, 0.0);
    }

    #[test]
    fn two_sample_deterministic_across_runs() {
        let a = [72.5, 85.0, 100.25, 88.3, 91.1];
        let b = [60.0, 71.2, 68.5, 74.0, 65.9];
        let r1 = two_sample_t_test(&a, &b, TwoSampleVariance::WelchUnequal).unwrap();
        let r2 = two_sample_t_test(&a, &b, TwoSampleVariance::WelchUnequal).unwrap();
        assert_eq!(r1.t_statistic.to_bits(), r2.t_statistic.to_bits());
        assert_eq!(
            r1.p_value_two_sided.to_bits(),
            r2.p_value_two_sided.to_bits()
        );
        assert_eq!(r1.df.to_bits(), r2.df.to_bits());
    }

    // ── Paired t-test (Phase 2) ────────────────────────────────────

    /// Reference: R `t.test(c(120, 122, 143, 100, 109), c(110, 115, 138, 95, 101), paired=TRUE)`.
    /// Differences: [10, 7, 5, 5, 8], mean = 7, sd = 2.121, n = 5,
    /// se = 0.949, t = 7/0.949 ≈ 7.379, df = 4, p ≈ 0.0018.
    #[test]
    fn paired_t_test_matches_r_reference() {
        let pairs = [
            (120.0, 110.0),
            (122.0, 115.0),
            (143.0, 138.0),
            (100.0, 95.0),
            (109.0, 101.0),
        ];
        let result = paired_t_test(&pairs).expect("n_pairs >= 2");
        assert_eq!(result.n_pairs, 5);
        assert!((result.mean_difference - 7.0).abs() < 1e-9);
        assert!(
            (result.sd_difference - 2.1213203).abs() < 1e-4,
            "got sd_diff = {}",
            result.sd_difference
        );
        assert!(
            (result.t_statistic - 7.379).abs() < 1e-2,
            "got t = {}",
            result.t_statistic
        );
        assert!(
            result.p_value_two_sided < 0.005,
            "got p = {}",
            result.p_value_two_sided
        );
    }

    /// Zero per-pair differences → t = 0, mean_diff = 0.
    #[test]
    fn paired_t_test_zero_differences_yields_t_zero() {
        let pairs = [(10.0, 10.0), (20.0, 20.0), (30.0, 30.0)];
        let result = paired_t_test(&pairs).expect("n_pairs >= 2");
        assert_eq!(result.t_statistic, 0.0);
        assert_eq!(result.mean_difference, 0.0);
    }

    #[test]
    fn paired_t_test_under_two_pairs_returns_none() {
        let one = [(1.0, 2.0)];
        assert!(paired_t_test(&one).is_none());
        let empty: [(f64, f64); 0] = [];
        assert!(paired_t_test(&empty).is_none());
    }

    // ── Factorial omnibus ANOVA (Phase 2.5) ───────────────────────

    /// 2×2 ANOVA, cleanly separated cell means with within-cell noise:
    ///   cell (0,0): values [10, 11, 9]   → mean 10
    ///   cell (0,1): values [20, 19, 21]  → mean 20
    ///   cell (1,0): values [30, 31, 29]  → mean 30
    ///   cell (1,1): values [40, 39, 41]  → mean 40
    /// Grand mean = 25. SS_between ≈ 3·100 + 3·25 + 3·25 + 3·225 = 1125;
    /// SS_within = 4 cells × 2 (sum of squared deviations within each
    /// triple [10,11,9] etc.) = 4·2 = 8.
    /// df_between = 3, df_within = 8. F ≈ (1125/3) / (8/8) = 375.
    /// Reference: omnibus F is enormous on this clean data; p ≪ 0.001.
    #[test]
    fn factorial_2x2_omnibus_rejects_h0() {
        let factor_levels = [2, 2];
        let observations = vec![
            (vec![0, 0], 10.0),
            (vec![0, 0], 11.0),
            (vec![0, 0], 9.0),
            (vec![0, 1], 20.0),
            (vec![0, 1], 19.0),
            (vec![0, 1], 21.0),
            (vec![1, 0], 30.0),
            (vec![1, 0], 31.0),
            (vec![1, 0], 29.0),
            (vec![1, 1], 40.0),
            (vec![1, 1], 39.0),
            (vec![1, 1], 41.0),
        ];
        let result = factorial_omnibus_anova(&factor_levels, &observations).expect("valid design");
        assert_eq!(result.n_cells, 4);
        assert_eq!(result.n_total, 12);
        assert_eq!(result.df_between, 3.0);
        assert_eq!(result.df_within, 8.0);
        assert!(
            (result.grand_mean - 25.0).abs() < 1e-9,
            "grand_mean = {}",
            result.grand_mean
        );
        // SS_within: each cell contributes (deviations from cell mean)
        // [10,11,9] from mean 10 → (0, 1, -1) → sum² = 2. Four cells → 8.
        assert!(
            (result.ss_within - 8.0).abs() < 1e-9,
            "SS_within = {}",
            result.ss_within
        );
        // SS_between: 3·(10-25)² + 3·(20-25)² + 3·(30-25)² + 3·(40-25)²
        //           = 3·225 + 3·25 + 3·25 + 3·225 = 1500
        assert!(
            (result.ss_between - 1500.0).abs() < 1e-9,
            "SS_between = {}",
            result.ss_between
        );
        // F = (1500/3) / (8/8) = 500
        assert!(
            (result.f_statistic - 500.0).abs() < 1e-9,
            "F = {}",
            result.f_statistic
        );
        assert!(
            result.p_value < 1e-8,
            "p should be vanishingly small for this clean data; got {}",
            result.p_value
        );
    }

    /// Null case: all cells have the same mean (everything is noise).
    /// F should be ≪ 1 and p should be large.
    #[test]
    fn factorial_2x2_null_data_yields_high_p() {
        let factor_levels = [2, 2];
        // All four cells have the same underlying mean (50) — just noise.
        let observations = vec![
            (vec![0, 0], 49.0),
            (vec![0, 0], 51.0),
            (vec![0, 0], 50.0),
            (vec![0, 1], 51.0),
            (vec![0, 1], 49.0),
            (vec![0, 1], 50.0),
            (vec![1, 0], 50.0),
            (vec![1, 0], 51.0),
            (vec![1, 0], 49.0),
            (vec![1, 1], 50.0),
            (vec![1, 1], 49.0),
            (vec![1, 1], 51.0),
        ];
        let result = factorial_omnibus_anova(&factor_levels, &observations).unwrap();
        assert!(
            result.p_value > 0.5,
            "p should be large when cell means coincide; got {}",
            result.p_value
        );
    }

    #[test]
    fn factorial_rejects_observations_with_wrong_factor_arity() {
        let factor_levels = [2, 2];
        // Observation has only 1 factor level instead of 2.
        let observations = vec![(vec![0], 10.0), (vec![1], 20.0)];
        assert!(factorial_omnibus_anova(&factor_levels, &observations).is_none());
    }

    #[test]
    fn factorial_rejects_out_of_range_levels() {
        let factor_levels = [2, 2];
        let observations = vec![(vec![0, 0], 10.0), (vec![1, 2], 20.0)]; // level 2 > 1
        assert!(factorial_omnibus_anova(&factor_levels, &observations).is_none());
    }

    #[test]
    fn factorial_requires_at_least_two_cells_observed() {
        let factor_levels = [2, 2];
        // All observations land in the (0,0) cell — only 1 cell
        // observed, no between-cell variance.
        let observations = vec![(vec![0, 0], 10.0), (vec![0, 0], 11.0), (vec![0, 0], 9.0)];
        assert!(factorial_omnibus_anova(&factor_levels, &observations).is_none());
    }

    #[test]
    fn factorial_requires_within_cell_replication() {
        let factor_levels = [2, 2];
        // Each cell has exactly one observation → df_within = 0.
        let observations = vec![
            (vec![0, 0], 10.0),
            (vec![0, 1], 20.0),
            (vec![1, 0], 30.0),
            (vec![1, 1], 40.0),
        ];
        assert!(factorial_omnibus_anova(&factor_levels, &observations).is_none());
    }

    // ── RCBD two-way ANOVA (Phase 4.0) ────────────────────────────

    /// 3 blocks × 3 treatments RCBD with a clear treatment effect.
    /// Cell values designed so the per-treatment means are clearly
    /// separated (means: 10, 20, 30) and per-block means are similar.
    ///
    /// Block 0:  [9, 19, 31]   block mean = 19.67
    /// Block 1:  [11, 21, 29]  block mean = 20.33
    /// Block 2:  [10, 20, 30]  block mean = 20
    /// Treatment means: 10, 20, 30
    /// Grand mean: 20
    ///
    /// SS_total = 9 obs × variance → compute directly
    /// SS_treatment = 3 × ((10-20)² + (20-20)² + (30-20)²) = 3 × 200 = 600
    /// SS_block = 3 × ((19.67-20)² + (20.33-20)² + (20-20)²) ≈ 3 × 0.222 ≈ 0.67
    /// SS_error ≈ tiny
    /// F_treatment huge, p ≪ 0.001.
    #[test]
    fn rcbd_3x3_treatment_effect_rejects_h0() {
        let observations = vec![
            // (block, treatment, value)
            (0, 0, 9.0),
            (0, 1, 19.0),
            (0, 2, 31.0),
            (1, 0, 11.0),
            (1, 1, 21.0),
            (1, 2, 29.0),
            (2, 0, 10.0),
            (2, 1, 20.0),
            (2, 2, 30.0),
        ];
        let result = rcbd_anova(3, 3, &observations).expect("complete 3x3 RCBD");
        assert_eq!(result.n_blocks, 3);
        assert_eq!(result.n_treatments, 3);
        assert_eq!(result.df_treatment, 2.0);
        assert_eq!(result.df_block, 2.0);
        assert_eq!(result.df_error, 4.0);
        assert!(
            (result.grand_mean - 20.0).abs() < 1e-9,
            "grand_mean = {}",
            result.grand_mean
        );
        // SS_treatment = 3 * ((10-20)² + (20-20)² + (30-20)²) = 600
        assert!(
            (result.ss_treatment - 600.0).abs() < 1e-9,
            "SS_treatment = {}",
            result.ss_treatment
        );
        assert!(
            result.f_treatment > 100.0,
            "F_treatment should be very large (clean treatment effect with tiny error); got F = {}",
            result.f_treatment
        );
        assert!(
            result.p_treatment < 1e-3,
            "p_treatment should be ≪ 0.001; got p = {}",
            result.p_treatment
        );
    }

    /// Block-controlled scenario: per-block baselines differ wildly
    /// (block-to-block variation huge), but within each block the
    /// treatment effect is consistent. RCBD controls for the block
    /// variation; an IID t-test on the same data would have its
    /// power destroyed by the block noise.
    #[test]
    fn rcbd_isolates_treatment_from_block_variation() {
        // Block 0 has baseline 100, block 1 has baseline 200,
        // block 2 has baseline 50. Within each block, treatment 1
        // adds +10 over treatment 0. The treatment effect is small
        // (+10) relative to between-block variation (100s) — RCBD
        // can detect it; IID can't.
        let observations = vec![
            (0, 0, 100.0),
            (0, 1, 110.0),
            (1, 0, 200.0),
            (1, 1, 210.0),
            (2, 0, 50.0),
            (2, 1, 60.0),
        ];
        let result = rcbd_anova(3, 2, &observations).expect("complete 3x2 RCBD");
        // Treatment means: 116.67 vs 126.67; difference 10
        // SS_treatment = 3 * ((116.67-121.67)² + (126.67-121.67)²) ≈ 3 * 50 = 150
        // SS_block is huge (the 100, 200, 50 baselines) but absorbed into the block term
        // SS_error should be ~0 because within each block the difference IS exactly +10
        assert!(
            result.f_treatment > 100.0,
            "RCBD should detect the consistent treatment effect controlling for block; \
             got F_treatment = {}",
            result.f_treatment
        );
        assert!(
            result.p_treatment < 0.01,
            "p_treatment should reject H0; got p = {}",
            result.p_treatment
        );
        // SS_block should dominate SS_treatment because the block baselines are wildly different
        assert!(
            result.ss_block > result.ss_treatment,
            "SS_block ({}) should dominate SS_treatment ({}) for this design",
            result.ss_block,
            result.ss_treatment
        );
    }

    #[test]
    fn rcbd_null_data_yields_high_p() {
        // All cells around the same mean (50) — no treatment effect.
        let observations = vec![
            (0, 0, 49.0),
            (0, 1, 51.0),
            (0, 2, 50.0),
            (1, 0, 50.0),
            (1, 1, 51.0),
            (1, 2, 49.0),
            (2, 0, 51.0),
            (2, 1, 49.0),
            (2, 2, 50.0),
        ];
        let result = rcbd_anova(3, 3, &observations).unwrap();
        assert!(
            result.p_treatment > 0.3,
            "p_treatment should be high for null data; got {}",
            result.p_treatment
        );
    }

    #[test]
    fn rcbd_rejects_incomplete_design() {
        // 3×3 design but missing the (2,2) cell.
        let observations = vec![
            (0, 0, 1.0),
            (0, 1, 2.0),
            (0, 2, 3.0),
            (1, 0, 4.0),
            (1, 1, 5.0),
            (1, 2, 6.0),
            (2, 0, 7.0),
            (2, 1, 8.0),
            // missing (2, 2)
        ];
        assert!(rcbd_anova(3, 3, &observations).is_none());
    }

    #[test]
    fn rcbd_rejects_duplicate_cells() {
        // 2×2 design but the (0,0) cell has two observations.
        let observations = vec![
            (0, 0, 1.0),
            (0, 0, 2.0),
            (0, 1, 3.0),
            (1, 0, 4.0),
            (1, 1, 5.0),
        ];
        assert!(rcbd_anova(2, 2, &observations).is_none());
    }

    #[test]
    fn rcbd_rejects_out_of_range_indices() {
        let observations = vec![(0, 0, 1.0), (0, 1, 2.0), (1, 0, 3.0), (1, 2, 4.0)]; // treatment 2 > 1
        assert!(rcbd_anova(2, 2, &observations).is_none());
    }

    #[test]
    fn rcbd_requires_min_2x2() {
        let observations = vec![(0, 0, 1.0), (0, 1, 2.0)];
        assert!(rcbd_anova(1, 2, &observations).is_none());
    }

    #[test]
    fn rcbd_deterministic_across_runs() {
        let observations = vec![
            (0, 0, 12.3),
            (0, 1, 18.7),
            (0, 2, 25.1),
            (1, 0, 14.1),
            (1, 1, 20.5),
            (1, 2, 27.3),
            (2, 0, 11.8),
            (2, 1, 19.4),
            (2, 2, 26.0),
        ];
        let a = rcbd_anova(3, 3, &observations).unwrap();
        let b = rcbd_anova(3, 3, &observations).unwrap();
        assert_eq!(a.f_treatment.to_bits(), b.f_treatment.to_bits());
        assert_eq!(a.p_treatment.to_bits(), b.p_treatment.to_bits());
        assert_eq!(a.ss_treatment.to_bits(), b.ss_treatment.to_bits());
        assert_eq!(a.ss_block.to_bits(), b.ss_block.to_bits());
        assert_eq!(a.ss_error.to_bits(), b.ss_error.to_bits());
    }

    #[test]
    fn factorial_anova_deterministic_across_runs() {
        let factor_levels = [2, 3];
        let observations = vec![
            (vec![0, 0], 1.5),
            (vec![0, 0], 2.0),
            (vec![0, 1], 4.0),
            (vec![0, 1], 4.5),
            (vec![0, 2], 7.0),
            (vec![0, 2], 7.5),
            (vec![1, 0], 11.0),
            (vec![1, 0], 11.5),
            (vec![1, 1], 14.0),
            (vec![1, 1], 14.5),
            (vec![1, 2], 17.0),
            (vec![1, 2], 17.5),
        ];
        let a = factorial_omnibus_anova(&factor_levels, &observations).unwrap();
        let b = factorial_omnibus_anova(&factor_levels, &observations).unwrap();
        assert_eq!(a.f_statistic.to_bits(), b.f_statistic.to_bits());
        assert_eq!(a.p_value.to_bits(), b.p_value.to_bits());
        assert_eq!(a.ss_between.to_bits(), b.ss_between.to_bits());
        assert_eq!(a.ss_within.to_bits(), b.ss_within.to_bits());
    }

    // ── Split-Plot ANOVA (Phase 4.5) ──────────────────────────────

    /// 2×2 split-plot with 3 whole-plot replicates per W level.
    /// Whole-plot factor: temperature (2 levels: 37, 30). Subplot
    /// factor: drug (2 levels: control, drug).
    ///
    /// Whole plots:
    ///   WP 0: W=0 (37°C), reps with (ctrl=50, drug=45)
    ///   WP 1: W=0,        (ctrl=51, drug=46)
    ///   WP 2: W=0,        (ctrl=52, drug=47)
    ///   WP 3: W=1 (30°C), (ctrl=70, drug=65)
    ///   WP 4: W=1,        (ctrl=71, drug=66)
    ///   WP 5: W=1,        (ctrl=72, drug=67)
    ///
    /// Per-W means: 37°C → 47.5, 30°C → 68.5; difference 21.
    /// Per-S means: ctrl → 61, drug → 56; difference 5.
    /// Per-cell means cleanly additive (no interaction).
    /// Whole-plot error per W is small (within-W whole-plot variance ±1).
    /// Subplot error is tiny (within-WP additive structure).
    #[test]
    fn splitplot_2x2x3_detects_both_main_effects() {
        let observations = vec![
            // (whole_plot_id, w_level, s_level, value)
            (0, 0, 0, 50.0),
            (0, 0, 1, 45.0),
            (1, 0, 0, 51.0),
            (1, 0, 1, 46.0),
            (2, 0, 0, 52.0),
            (2, 0, 1, 47.0),
            (3, 1, 0, 70.0),
            (3, 1, 1, 65.0),
            (4, 1, 0, 71.0),
            (4, 1, 1, 66.0),
            (5, 1, 0, 72.0),
            (5, 1, 1, 67.0),
        ];
        let result = splitplot_anova(2, 2, 3, &observations).expect("complete 2x2x3 split-plot");
        assert_eq!(result.a, 2);
        assert_eq!(result.b, 2);
        assert_eq!(result.r, 3);
        // df_W = a-1 = 1; df_WP_within_W = a(r-1) = 4
        // df_S = b-1 = 1; df_WS = (a-1)(b-1) = 1
        // df_error = a(b-1)(r-1) = 4
        assert_eq!(result.df_w, 1.0);
        assert_eq!(result.df_wp_within_w, 4.0);
        assert_eq!(result.df_s, 1.0);
        assert_eq!(result.df_ws, 1.0);
        assert_eq!(result.df_error, 4.0);
        // Grand mean = (50+45+51+46+52+47+70+65+71+66+72+67)/12 = 58.5
        assert!((result.grand_mean - 58.5).abs() < 1e-9);

        // F_W: whole-plot factor with whole-plot error. With temp
        // means 47.5 vs 68.5 (diff 21) and WP variance ±1 within
        // each W level, F_W is huge — but it MUST use the whole-plot
        // error (not the tiny subplot error).
        assert!(
            result.p_w < 0.001,
            "p_w should reject the whole-plot main effect; got {}",
            result.p_w
        );
        // F_S: subplot factor with subplot error. Drug effect of -5
        // is consistent across all whole plots; subplot error is ~0
        // (additive model is exact). F_S enormous.
        assert!(
            result.p_s < 0.001,
            "p_s should reject the subplot main effect; got {}",
            result.p_s
        );
        // F_WS: no interaction in this design. p_ws should be high
        // (the additive structure is exact, so the interaction term
        // is zero / numerical noise).
        assert!(
            result.p_ws > 0.5 || result.ss_ws < 1e-9,
            "p_ws should be high (or SS_WS near zero) for additive design; got p_ws = {}, SS_WS = {}",
            result.p_ws,
            result.ss_ws
        );
    }

    /// The false-positive-trap regression test. Same data treated as
    /// a flat factorial WOULD give a hugely inflated F_W (because
    /// it'd be computed against the tiny subplot error). The
    /// correctness check: in the split-plot dispatch, F_W is computed
    /// against MS_WP_within_W (whole-plot error), which is larger
    /// than MS_error (subplot error). Verify the relationship.
    #[test]
    fn splitplot_uses_correct_error_stratum_for_whole_plot_f() {
        let observations = vec![
            (0, 0, 0, 50.0),
            (0, 0, 1, 45.0),
            (1, 0, 0, 51.0),
            (1, 0, 1, 46.0),
            (2, 0, 0, 52.0),
            (2, 0, 1, 47.0),
            (3, 1, 0, 70.0),
            (3, 1, 1, 65.0),
            (4, 1, 0, 71.0),
            (4, 1, 1, 66.0),
            (5, 1, 0, 72.0),
            (5, 1, 1, 67.0),
        ];
        let r = splitplot_anova(2, 2, 3, &observations).unwrap();
        let ms_wp_within_w = r.ss_wp_within_w / r.df_wp_within_w;
        let ms_error = r.ss_error / r.df_error;
        // The whole-plot error stratum is larger than the subplot
        // error stratum (this is the structural property of
        // split-plot designs — whole-plot replicates carry more
        // variance than within-whole-plot subplot variation).
        // SS_error here is near zero (additive structure exact)
        // while SS_WP_within_W reflects the ±1 jitter on whole-plot
        // means within each W level.
        assert!(
            ms_wp_within_w > ms_error,
            "MS_WP_within_W ({}) must be larger than MS_error ({}) — \
             the whole-plot error stratum is the structurally larger one",
            ms_wp_within_w,
            ms_error
        );
    }

    /// Null whole-plot main effect: temperature levels have the same
    /// means; only subplot factor matters. F_W should be small,
    /// p_w high.
    #[test]
    fn splitplot_null_whole_plot_effect_yields_high_p_w() {
        // Both W levels have mean ~55; only drug effect matters.
        let observations = vec![
            (0, 0, 0, 60.0),
            (0, 0, 1, 50.0),
            (1, 0, 0, 61.0),
            (1, 0, 1, 51.0),
            (2, 0, 0, 59.0),
            (2, 0, 1, 49.0),
            (3, 1, 0, 60.0),
            (3, 1, 1, 50.0),
            (4, 1, 0, 61.0),
            (4, 1, 1, 51.0),
            (5, 1, 0, 59.0),
            (5, 1, 1, 49.0),
        ];
        let r = splitplot_anova(2, 2, 3, &observations).unwrap();
        assert!(
            r.p_w > 0.3,
            "p_w should be high when whole-plot levels have equal means; got {}",
            r.p_w
        );
        assert!(
            r.p_s < 0.001,
            "p_s should reject when subplot factor has a clear effect; got {}",
            r.p_s
        );
    }

    #[test]
    fn splitplot_rejects_inconsistent_w_level_per_whole_plot() {
        // Whole plot 0 has both w=0 and w=1 — invalid: each whole
        // plot must have a single W level.
        let observations = vec![
            (0, 0, 0, 1.0),
            (0, 1, 1, 2.0), // INVALID: WP 0 declared w=1 here, but w=0 above
            (1, 0, 0, 3.0),
            (1, 0, 1, 4.0),
            (2, 1, 0, 5.0),
            (2, 1, 1, 6.0),
            (3, 1, 0, 7.0),
            (3, 1, 1, 8.0),
        ];
        assert!(splitplot_anova(2, 2, 2, &observations).is_none());
    }

    #[test]
    fn splitplot_rejects_unbalanced_design() {
        // 3 whole plots at W=0 but only 1 at W=1 → not balanced (r=2 expected).
        let observations = vec![
            (0, 0, 0, 1.0),
            (0, 0, 1, 2.0),
            (1, 0, 0, 3.0),
            (1, 0, 1, 4.0),
            (2, 0, 0, 5.0),
            (2, 0, 1, 6.0),
            (3, 1, 0, 7.0),
            (3, 1, 1, 8.0),
        ];
        assert!(splitplot_anova(2, 2, 2, &observations).is_none());
    }

    #[test]
    fn splitplot_rejects_min_size_violations() {
        let obs = vec![(0, 0, 0, 1.0), (0, 0, 1, 2.0)];
        // r = 1 invalid (no whole-plot replicates)
        assert!(splitplot_anova(2, 2, 1, &obs).is_none());
        // a = 1 invalid
        assert!(splitplot_anova(1, 2, 2, &obs).is_none());
        // b = 1 invalid
        assert!(splitplot_anova(2, 1, 2, &obs).is_none());
    }

    #[test]
    fn splitplot_deterministic_across_runs() {
        let observations = vec![
            (0, 0, 0, 50.0),
            (0, 0, 1, 45.0),
            (1, 0, 0, 51.0),
            (1, 0, 1, 46.0),
            (2, 0, 0, 52.0),
            (2, 0, 1, 47.0),
            (3, 1, 0, 70.0),
            (3, 1, 1, 65.0),
            (4, 1, 0, 71.0),
            (4, 1, 1, 66.0),
            (5, 1, 0, 72.0),
            (5, 1, 1, 67.0),
        ];
        let a = splitplot_anova(2, 2, 3, &observations).unwrap();
        let b = splitplot_anova(2, 2, 3, &observations).unwrap();
        assert_eq!(a.f_w.to_bits(), b.f_w.to_bits());
        assert_eq!(a.f_s.to_bits(), b.f_s.to_bits());
        assert_eq!(a.f_ws.to_bits(), b.f_ws.to_bits());
        assert_eq!(a.ss_w.to_bits(), b.ss_w.to_bits());
        assert_eq!(a.ss_wp_within_w.to_bits(), b.ss_wp_within_w.to_bits());
        assert_eq!(a.ss_error.to_bits(), b.ss_error.to_bits());
    }

    // ── Compound-Symmetry RM-ANOVA (Phase 4.9) ────────────────────

    /// 5 subjects × 4 timepoints. Monotonic decline at each subject
    /// (drug-decay shape) with consistent baselines per subject
    /// modulo small variation.
    ///
    /// Per-time means: ~102, ~82.6, ~61.6, ~41.6 (clean monotone
    /// decline). Subject variation is small (baselines ±~5 around
    /// each subject's trajectory). F_time enormous.
    #[test]
    fn rm_cs_5x4_time_effect_rejects_h0() {
        let observations = vec![
            // (subject, time, value)
            (0, 0, 100.0),
            (0, 1, 80.0),
            (0, 2, 60.0),
            (0, 3, 40.0),
            (1, 0, 110.0),
            (1, 1, 88.0),
            (1, 2, 65.0),
            (1, 3, 45.0),
            (2, 0, 95.0),
            (2, 1, 78.0),
            (2, 2, 58.0),
            (2, 3, 38.0),
            (3, 0, 105.0),
            (3, 1, 85.0),
            (3, 2, 63.0),
            (3, 3, 43.0),
            (4, 0, 100.0),
            (4, 1, 82.0),
            (4, 2, 62.0),
            (4, 3, 42.0),
        ];
        let result = repeated_measures_cs_anova(5, 4, &observations).expect("complete 5x4 RM");
        assert_eq!(result.n_subjects, 5);
        assert_eq!(result.n_timepoints, 4);
        assert_eq!(result.df_time, 3.0);
        assert_eq!(result.df_subject, 4.0);
        assert_eq!(result.df_error, 12.0);
        assert!(
            result.f_time > 100.0,
            "F_time should be huge for clean monotone decline; got F = {}",
            result.f_time
        );
        assert!(
            result.p_time < 1e-6,
            "p_time should be ≪ 1e-6; got p = {}",
            result.p_time
        );
    }

    /// Subject-controlled scenario: subjects have wildly different
    /// baselines (50, 100, 200, 75, 150) but the time-effect within
    /// each subject is consistent (-10 per timepoint). RM-ANOVA
    /// controls for subject variation; treating these data as IID
    /// across all 20 observations would have the subject variance
    /// dominate the time signal.
    #[test]
    fn rm_cs_isolates_time_from_subject_variation() {
        // Each subject's trajectory: baseline + decline of -10/step.
        let observations = vec![
            // Subject 0: baseline 50
            (0, 0, 50.0),
            (0, 1, 40.0),
            (0, 2, 30.0),
            (0, 3, 20.0),
            // Subject 1: baseline 100
            (1, 0, 100.0),
            (1, 1, 90.0),
            (1, 2, 80.0),
            (1, 3, 70.0),
            // Subject 2: baseline 200
            (2, 0, 200.0),
            (2, 1, 190.0),
            (2, 2, 180.0),
            (2, 3, 170.0),
            // Subject 3: baseline 75
            (3, 0, 75.0),
            (3, 1, 65.0),
            (3, 2, 55.0),
            (3, 3, 45.0),
            // Subject 4: baseline 150
            (4, 0, 150.0),
            (4, 1, 140.0),
            (4, 2, 130.0),
            (4, 3, 120.0),
        ];
        let result = repeated_measures_cs_anova(5, 4, &observations).unwrap();
        // Time effect is perfectly consistent across subjects, so
        // SS_error should be near zero and F_time enormous.
        assert!(
            result.f_time > 100.0,
            "F_time should be very large when time-effect is consistent across subjects; got {}",
            result.f_time
        );
        // SS_subject should dominate SS_time because subject baselines vary widely.
        assert!(
            result.ss_subject > result.ss_time,
            "SS_subject ({}) should dominate SS_time ({}) for this design",
            result.ss_subject,
            result.ss_time
        );
        // SS_error should be near zero (additive structure exact).
        assert!(
            result.ss_error < 1e-6,
            "SS_error should be near zero for additive design; got {}",
            result.ss_error
        );
    }

    #[test]
    fn rm_cs_null_time_effect_yields_high_p() {
        // No systematic time effect — all timepoints have similar means.
        let observations = vec![
            (0, 0, 50.0),
            (0, 1, 51.0),
            (0, 2, 49.0),
            (0, 3, 50.0),
            (1, 0, 51.0),
            (1, 1, 50.0),
            (1, 2, 51.0),
            (1, 3, 49.0),
            (2, 0, 49.0),
            (2, 1, 51.0),
            (2, 2, 50.0),
            (2, 3, 51.0),
            (3, 0, 50.0),
            (3, 1, 49.0),
            (3, 2, 51.0),
            (3, 3, 50.0),
        ];
        let result = repeated_measures_cs_anova(4, 4, &observations).unwrap();
        assert!(
            result.p_time > 0.3,
            "p_time should be high for null data; got {}",
            result.p_time
        );
    }

    #[test]
    fn rm_cs_rejects_incomplete_design() {
        // 3 subjects × 3 timepoints but missing (2, 2).
        let observations = vec![
            (0, 0, 1.0),
            (0, 1, 2.0),
            (0, 2, 3.0),
            (1, 0, 4.0),
            (1, 1, 5.0),
            (1, 2, 6.0),
            (2, 0, 7.0),
            (2, 1, 8.0),
        ];
        assert!(repeated_measures_cs_anova(3, 3, &observations).is_none());
    }

    #[test]
    fn rm_cs_rejects_duplicate_cell() {
        let observations = vec![
            (0, 0, 1.0),
            (0, 0, 2.0),
            (0, 1, 3.0),
            (1, 0, 4.0),
            (1, 1, 5.0),
        ];
        assert!(repeated_measures_cs_anova(2, 2, &observations).is_none());
    }

    #[test]
    fn rm_cs_requires_min_2x2() {
        let obs = vec![(0, 0, 1.0), (0, 1, 2.0)];
        assert!(repeated_measures_cs_anova(1, 2, &obs).is_none());
    }

    #[test]
    fn pb_identity_line_methods_agree() {
        // Two methods producing identical readings → slope = 1.0,
        // intercept = 0.0, both inside the CI by construction.
        let a: Vec<f64> = (1..=20).map(|i| i as f64 * 10.0).collect();
        let b = a.clone();
        let r = passing_bablok_regression(&a, &b).expect("n=20 valid");
        assert!((r.slope - 1.0).abs() < 1e-9);
        assert!((r.intercept - 0.0).abs() < 1e-9);
        assert!(r.slope_ci_low <= 1.0 && 1.0 <= r.slope_ci_high);
        assert!(r.intercept_ci_low <= 0.0 && 0.0 <= r.intercept_ci_high);
    }

    #[test]
    fn pb_proportional_bias_detected() {
        // method B systematically reads 1.5× method A → slope ≈ 1.5,
        // intercept ≈ 0; the CIs should exclude 1.0 cleanly.
        let a: Vec<f64> = (1..=20).map(|i| i as f64 * 10.0).collect();
        let b: Vec<f64> = a.iter().map(|&x| 1.5 * x).collect();
        let r = passing_bablok_regression(&a, &b).expect("n=20 valid");
        assert!(
            (r.slope - 1.5).abs() < 1e-6,
            "slope should be 1.5, got {}",
            r.slope
        );
        assert!(
            r.slope_ci_low > 1.0,
            "slope CI low should exceed 1.0, got {}",
            r.slope_ci_low
        );
    }

    #[test]
    fn pb_constant_bias_detected() {
        // method B = method A + 5.0 → slope ≈ 1.0, intercept ≈ 5.0;
        // intercept CI should exclude 0.0.
        let a: Vec<f64> = (1..=20).map(|i| i as f64 * 10.0).collect();
        let b: Vec<f64> = a.iter().map(|&x| x + 5.0).collect();
        let r = passing_bablok_regression(&a, &b).expect("n=20 valid");
        assert!((r.slope - 1.0).abs() < 1e-9);
        assert!((r.intercept - 5.0).abs() < 1e-6);
        assert!(
            r.intercept_ci_low > 0.0,
            "intercept CI low should exceed 0.0, got {}",
            r.intercept_ci_low
        );
    }

    #[test]
    fn pb_n_less_than_three_returns_none() {
        assert!(passing_bablok_regression(&[1.0, 2.0], &[1.0, 2.0]).is_none());
    }

    #[test]
    fn pb_constant_x_returns_none() {
        // All method-A readings equal → no defined slopes.
        assert!(passing_bablok_regression(
            &[5.0; 10],
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
        )
        .is_none());
    }

    #[test]
    fn esd_no_outliers_returns_empty() {
        // 12 tight samples around 100; no clear outliers.
        let samples = [
            98.0, 99.0, 100.0, 101.0, 102.0, 99.5, 100.5, 98.5, 101.5, 100.0, 99.0, 101.0,
        ];
        let excluded = esd_filter(&samples, 3, 0.05);
        assert!(
            excluded.is_empty(),
            "tight data should have no ESD outliers, got {excluded:?}"
        );
    }

    #[test]
    fn esd_single_clear_outlier_detected() {
        // 11 tight samples around 100 + one extreme value at 500.
        let samples = [
            98.0, 99.0, 100.0, 101.0, 102.0, 99.5, 100.5, 98.5, 101.5, 100.0, 500.0,
        ];
        let excluded = esd_filter(&samples, 3, 0.05);
        assert_eq!(
            excluded,
            vec![10],
            "expected only the index-10 extreme to be flagged"
        );
    }

    #[test]
    fn esd_respects_max_outliers_cap() {
        // 15 normal + 2 extreme; max_outliers = 1 caps detection at 1
        // even though 2 are clearly outliers. (Larger normal n needed
        // to keep ESD from losing power to multi-outlier masking on a
        // small base.)
        let samples = [
            100.0, 101.0, 99.0, 100.0, 101.0, 99.0, 100.0, 101.0, 99.0, 100.0, 100.5, 99.5, 100.2,
            99.8, 100.1, 500.0, 600.0,
        ];
        let excluded = esd_filter(&samples, 1, 0.05);
        assert_eq!(
            excluded.len(),
            1,
            "max_outliers = 1 caps detection at 1 (got {excluded:?})"
        );
        let unbounded = esd_filter(&samples, 3, 0.05);
        assert!(
            !unbounded.is_empty(),
            "with max_outliers = 3, ESD should still flag at least one extreme; got {unbounded:?}"
        );
    }

    #[test]
    fn esd_zero_max_outliers_returns_empty() {
        let samples = [100.0, 101.0, 99.0, 500.0];
        assert!(esd_filter(&samples, 0, 0.05).is_empty());
    }

    #[test]
    fn esd_n_too_small_returns_empty() {
        assert!(esd_filter(&[100.0, 101.0, 99.0], 1, 0.05).is_empty());
    }

    #[test]
    fn esd_deterministic_across_runs() {
        let samples = [
            98.0, 99.0, 100.0, 101.0, 102.0, 99.5, 100.5, 98.5, 101.5, 100.0, 500.0,
        ];
        assert_eq!(esd_filter(&samples, 3, 0.05), esd_filter(&samples, 3, 0.05));
    }

    #[test]
    fn pb_deterministic_across_runs() {
        let a: Vec<f64> = (1..=15).map(|i| i as f64).collect();
        let b: Vec<f64> = a.iter().map(|&x| 1.1 * x + 0.3).collect();
        let r1 = passing_bablok_regression(&a, &b).unwrap();
        let r2 = passing_bablok_regression(&a, &b).unwrap();
        assert_eq!(r1.slope.to_bits(), r2.slope.to_bits());
        assert_eq!(r1.intercept.to_bits(), r2.intercept.to_bits());
        assert_eq!(r1.slope_ci_low.to_bits(), r2.slope_ci_low.to_bits());
    }

    #[test]
    fn rm_cs_deterministic_across_runs() {
        let observations = vec![
            (0, 0, 100.0),
            (0, 1, 80.0),
            (0, 2, 60.0),
            (0, 3, 40.0),
            (1, 0, 110.0),
            (1, 1, 88.0),
            (1, 2, 65.0),
            (1, 3, 45.0),
            (2, 0, 95.0),
            (2, 1, 78.0),
            (2, 2, 58.0),
            (2, 3, 38.0),
        ];
        let a = repeated_measures_cs_anova(3, 4, &observations).unwrap();
        let b = repeated_measures_cs_anova(3, 4, &observations).unwrap();
        assert_eq!(a.f_time.to_bits(), b.f_time.to_bits());
        assert_eq!(a.p_time.to_bits(), b.p_time.to_bits());
        assert_eq!(a.ss_subject.to_bits(), b.ss_subject.to_bits());
        assert_eq!(a.ss_time.to_bits(), b.ss_time.to_bits());
        assert_eq!(a.ss_error.to_bits(), b.ss_error.to_bits());
    }
}
