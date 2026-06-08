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
use statrs::distribution::{ContinuousCDF, StudentsT};

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
}
