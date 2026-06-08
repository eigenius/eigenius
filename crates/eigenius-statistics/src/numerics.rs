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
/// embedding in a `MeasurementVerdict`'s field set.
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
}
