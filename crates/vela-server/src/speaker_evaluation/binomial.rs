//! Exact one-sided binomial confidence bounds for the pilot report.
//!
//! The pilot proves four independent error-rate targets with one-sided 95% upper bounds. The
//! bounds are the exact (Clopper-Pearson) ones, not a normal approximation: with `k` errors in
//! `n` eligible trials the bound `U` solves `P(Binomial(n, U) <= k) = 0.05`.
//!
//! See `docs/speaker-recognition-web-enrollment-implementation-guide` §11.2. The closed forms for
//! the `k == 0`, `k == n` and `n == 0` cases are handled directly; every other case is found by
//! bisection on the exact cumulative distribution.

/// Confidence level of the reported upper bounds (95%, so `alpha = 0.05`).
pub const ALPHA: f64 = 0.05;

/// One-sided exact 95% upper confidence bound for the error rate given `k` errors in `n`
/// eligible trials.
///
/// Returns `None` when there is no evidence (`n == 0`) — a bound cannot be claimed from zero
/// trials. `k >= n` yields `1.0` (all trials errored). `k == 0` uses the closed form
/// `1 - 0.05^(1/n)` so the 298/299 boundary is exact.
pub fn upper_bound(n: u64, k: u64) -> Option<f64> {
    if n == 0 {
        return None;
    }
    if k >= n {
        return Some(1.0);
    }
    if k == 0 {
        return Some(1.0 - ALPHA.powf(1.0 / n as f64));
    }
    // P(X <= k) is strictly decreasing in the success probability p, from 1 at p = 0 to 0 at
    // p = 1, so the level-p curve crosses ALPHA exactly once.
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cdf(n, k, mid) > ALPHA {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= 1e-13 {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Smallest number of zero-error trials that proves `error_rate <= target` at 95%.
///
/// Solving `1 - 0.05^(1/n) <= target` gives `n >= ln(0.05) / ln(1 - target)`. For a 1% target
/// that is 299 trials; for 10% it is 29.
pub fn min_zero_error_trials(target: f64) -> u64 {
    debug_assert!(target > 0.0 && target < 1.0, "target must be a probability");
    (ALPHA.ln() / (1.0 - target).ln()).ceil() as u64
}

/// Exact `P(X <= k)` for `X ~ Binomial(n, p)`, summed in log space to avoid overflow.
fn cdf(n: u64, k: u64, p: f64) -> f64 {
    if p <= 0.0 {
        return 1.0;
    }
    if p >= 1.0 {
        return if k >= n { 1.0 } else { 0.0 };
    }
    (0..=k).map(|i| ln_pmf(n, i, p).exp()).sum()
}

/// `ln(C(n, i) * p^i * (1-p)^(n-i))`.
fn ln_pmf(n: u64, i: u64, p: f64) -> f64 {
    ln_binomial(n, i) + i as f64 * p.ln() + (n - i) as f64 * (1.0 - p).ln()
}

/// `ln(C(n, i))` via log-gamma, stable for the trial counts a pilot can produce.
fn ln_binomial(n: u64, i: u64) -> f64 {
    ln_gamma(n as f64 + 1.0) - ln_gamma(i as f64 + 1.0) - ln_gamma((n - i) as f64 + 1.0)
}

/// Lanczos `ln(Gamma(x))` for `x >= 1` (all callers pass `n + 1 >= 1`).
fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    debug_assert!(x >= 1.0, "ln_gamma only used for x >= 1");
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values were produced independently (scipy `beta.ppf(0.95, k+1, n-k)` and the
    /// closed form for `k == 0`), not by this implementation.
    #[test]
    fn matches_independent_fixtures() {
        let cases = [
            (299, 0, 0.009_969_146_792_899_286),
            (298, 0, 0.010_002_432_437_677_733),
            (100, 1, 0.046_559_811_453_538_99),
            (50, 3, 0.147_837_176_364_181_2),
            (20, 2, 0.282_618_524_885_860_9),
            (200, 5, 0.051_843_339_120_290_544),
            (10, 1, 0.394_163_302_436_504_9),
        ];
        for (n, k, expected) in cases {
            let got = upper_bound(n, k).expect("bound");
            assert!(
                (got - expected).abs() < 1e-9,
                "upper_bound({n}, {k}) = {got}, expected {expected}"
            );
        }
    }

    #[test]
    fn zero_error_boundary_is_298_vs_299() {
        assert!(upper_bound(298, 0).unwrap() > 0.01);
        assert!(upper_bound(299, 0).unwrap() <= 0.01);
        assert_eq!(min_zero_error_trials(0.01), 299);
        assert_eq!(min_zero_error_trials(0.10), 29);
    }

    #[test]
    fn zero_trials_has_no_bound() {
        assert_eq!(upper_bound(0, 0), None);
    }

    #[test]
    fn all_error_trials_bound_at_one() {
        assert_eq!(upper_bound(5, 5), Some(1.0));
        assert_eq!(upper_bound(5, 9), Some(1.0));
    }
}
