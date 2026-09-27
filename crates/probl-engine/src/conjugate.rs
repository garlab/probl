//! Exact updates for conjugate priors (docs/semantics.md, section 14): the
//! probability of an observation when its parameter is unknown, and the
//! parameter's distribution after it.
//!
//! Probabilities are computed as logarithms. A run's weight has an extended
//! exponent, but one observation's probability can already be far below the
//! smallest `f64`: `0` successes out of 100,000 with a rate near 50% has a
//! probability of e^−4234.

use crate::continuous::{Family, ln_beta};

/// What an observation saw, with its parameters besides the variable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seen {
    /// `k` successes from `binomial(trials, x)`.
    Binomial { trials: u64, k: f64 },
    /// A fact from `bernoulli(x)`.
    Bernoulli(bool),
    /// `k` from `poisson(x)`.
    Poisson { k: f64 },
    /// `y` from `normal(x, sd)`.
    Normal { y: f64, sd: f64 },
}

/// Whether a draw from `family` may be delayed: it's the prior of a
/// conjugate pair.
pub fn is_prior(family: &Family) -> bool {
    matches!(
        family,
        Family::Beta { .. } | Family::Gamma { .. } | Family::Normal { .. }
    )
}

/// Observing `seen`, with the variable distributed as `prior`: the natural
/// logarithm of the observation's probability (a density for `Normal`), and
/// the variable's distribution after it. `None` if the two aren't a
/// conjugate pair. A logarithm of −∞ means the observation is impossible,
/// like a count above the number of trials; the prior is returned then.
pub fn update(prior: &Family, seen: Seen) -> Option<(f64, Family)> {
    let impossible = Some((f64::NEG_INFINITY, *prior));
    let count = |k: f64| k >= 0.0 && k.fract() == 0.0;
    match (*prior, seen) {
        (Family::Beta { a, b }, Seen::Binomial { trials, k }) => {
            let n = trials as f64;
            if !count(k) || k > n {
                return impossible;
            }
            // C(n, k) B(a + k, b + n − k) / B(a, b), where
            // C(n, k) = 1 / ((n + 1) B(k + 1, n − k + 1)).
            let ln = -libm::log1p(n) - ln_beta(k + 1.0, n - k + 1.0) + ln_beta(a + k, b + n - k) - ln_beta(a, b);
            Some((ln, Family::Beta { a: a + k, b: b + n - k }))
        }
        (Family::Beta { a, b }, Seen::Bernoulli(yes)) => Some(if yes {
            (libm::log(a / (a + b)), Family::Beta { a: a + 1.0, b })
        } else {
            (libm::log(b / (a + b)), Family::Beta { a, b: b + 1.0 })
        }),
        (Family::Gamma { shape, scale }, Seen::Poisson { k }) => {
            if !count(k) || !k.is_finite() {
                return impossible;
            }
            // Γ(shape + k) / (Γ(shape) k!) · (scale / (1 + scale))^k
            // · (1 + scale)^−shape, where the gammas and the factorial make
            // 1 / (k B(k, shape)) for k ≥ 1.
            let mut ln = -shape * libm::log1p(scale);
            if k > 0.0 {
                // ln(scale / (1 + scale)), without cancelling for a large scale.
                let odds = if scale > 1.0 {
                    -libm::log1p(1.0 / scale)
                } else {
                    libm::log(scale) - libm::log1p(scale)
                };
                ln += -libm::log(k) - ln_beta(k, shape) + k * odds;
            }
            let posterior = Family::Gamma {
                shape: shape + k,
                scale: scale / (1.0 + scale),
            };
            Some((ln, posterior))
        }
        (Family::Normal { mean, sd }, Seen::Normal { y, sd: noise }) => {
            // y is normal with variance sd² + noise²; √ of it without
            // overflow, and the posterior in terms of shares of it.
            let spread = libm::hypot(sd, noise);
            let z = (y - mean) / spread;
            let ln = -libm::log(spread) - 0.5 * libm::log(2.0 * std::f64::consts::PI) - z * z / 2.0;
            let (s, t) = (sd / spread, noise / spread);
            let posterior = Family::Normal {
                mean: mean + (y - mean) * s * s,
                sd: sd * t,
            };
            Some((if ln.is_nan() { f64::NEG_INFINITY } else { ln }, posterior))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * (1.0 + b.abs()), "{a} vs {b}");
    }

    fn beta(a: f64, b: f64) -> Family {
        Family::beta(a, b).unwrap()
    }

    /// ∫ f(x) dx over (lo, hi), by the midpoint rule on a fine grid.
    fn integrate(f: impl Fn(f64) -> f64, lo: f64, hi: f64) -> f64 {
        let n = 200_000;
        let h = (hi - lo) / n as f64;
        (0..n).map(|i| f(lo + (i as f64 + 0.5) * h) * h).sum()
    }

    #[test]
    fn log_beta_is_accurate_for_small_and_large_arguments() {
        // Values from mpmath, with 50 digits.
        close(ln_beta(2.0, 3.0), -2.484_906_649_788_000_3, 1e-15);
        close(ln_beta(0.5, 0.5), 1.144_729_885_849_400_2, 1e-15);
        close(ln_beta(1000.0, 1000.0), -1_388.482_601_635_902_3, 1e-15);
        close(ln_beta(1000.0, 101_000.0), -5_622.584_683_645_091, 1e-15);
        close(ln_beta(3.0, 1e12), -82.199_916_167_228_7, 1e-15);
        close(ln_beta(1e10, 1e10), -13_862_943_621.446_32, 1e-15);
        close(ln_beta(12.5, 9.0), -14.512_975_445_993_465, 1e-15);
    }

    /// The marginal probability is the prior average of the likelihood, and
    /// the posterior is the prior times the likelihood, normalized.
    #[test]
    fn updates_agree_with_integrating_over_the_prior() {
        let cases = [
            (beta(2.0, 50.0), Seen::Binomial { trials: 400, k: 12.0 }),
            (beta(1.0, 1.0), Seen::Binomial { trials: 10, k: 0.0 }),
            (beta(3.0, 2.0), Seen::Bernoulli(true)),
            (beta(3.0, 2.0), Seen::Bernoulli(false)),
            (Family::gamma(2.0, 3.0).unwrap(), Seen::Poisson { k: 4.0 }),
            (Family::gamma(1.5, 0.2).unwrap(), Seen::Poisson { k: 0.0 }),
            (Family::normal(1.0, 2.0).unwrap(), Seen::Normal { y: 1.5, sd: 0.5 }),
            (Family::normal(-3.0, 0.1).unwrap(), Seen::Normal { y: 4.0, sd: 3.0 }),
        ];
        for (prior, seen) in cases {
            let likelihood = |x: f64| match seen {
                Seen::Binomial { trials, k } => crate::dist::Counts::Binomial { n: trials, p: x }.pmf(k),
                Seen::Bernoulli(yes) => {
                    if yes {
                        x
                    } else {
                        1.0 - x
                    }
                }
                Seen::Poisson { k } => crate::dist::Counts::Poisson { rate: x }.pmf(k),
                Seen::Normal { y, sd } => Family::normal(x, sd).unwrap().pdf(y),
            };
            let (lo, hi) = (prior.quantile(1e-13), prior.quantile(1.0 - 1e-13));
            let joint = |x: f64| prior.pdf(x) * likelihood(x);
            let marginal = integrate(joint, lo, hi);
            let (ln, posterior) = update(&prior, seen).unwrap();
            close(ln.exp(), marginal, 1e-6);
            // The posterior's density and moments.
            for q in [0.1, 0.5, 0.9] {
                let x = posterior.quantile(q);
                close(posterior.pdf(x), joint(x) / marginal, 1e-5);
            }
            close(posterior.mean(), integrate(|x| x * joint(x), lo, hi) / marginal, 1e-6);
        }
    }

    #[test]
    fn impossible_observations_have_no_probability() {
        let prior = beta(2.0, 3.0);
        for k in [11.0, -1.0, 2.5, f64::NAN] {
            let (ln, after) = update(&prior, Seen::Binomial { trials: 10, k }).unwrap();
            assert_eq!(ln, f64::NEG_INFINITY);
            assert_eq!(after, prior);
        }
        let gamma = Family::gamma(2.0, 1.0).unwrap();
        for k in [-1.0, 0.5, f64::INFINITY, f64::NAN] {
            assert_eq!(update(&gamma, Seen::Poisson { k }).unwrap().0, f64::NEG_INFINITY);
        }
        let normal = Family::normal(0.0, 1.0).unwrap();
        for y in [f64::INFINITY, f64::NAN] {
            assert_eq!(
                update(&normal, Seen::Normal { y, sd: 1.0 }).unwrap().0,
                f64::NEG_INFINITY
            );
        }
        // Pairs that aren't conjugate.
        assert!(update(&normal, Seen::Poisson { k: 1.0 }).is_none());
        assert!(update(&gamma, Seen::Bernoulli(true)).is_none());
        assert!(update(&Family::lognormal(0.0, 1.0).unwrap(), Seen::Poisson { k: 1.0 }).is_none());
    }

    #[test]
    fn extreme_observations_keep_their_probability() {
        // The review's case: far below the smallest f64, but exact.
        let (ln, after) = update(
            &beta(1000.0, 1000.0),
            Seen::Binomial {
                trials: 100_000,
                k: 0.0,
            },
        )
        .unwrap();
        close(ln, -4_234.102_082_009_189, 1e-15);
        assert_eq!(after, beta(1000.0, 101_000.0));
        // A million trials, and a count far out in the tail.
        let (ln, _) = update(
            &beta(2.0, 50.0),
            Seen::Binomial {
                trials: 1_000_000,
                k: 900_000.0,
            },
        )
        .unwrap();
        close(ln, -118.892_768_879_054_9, 1e-12);
        // Priors whose densities are infinite at an end.
        let (ln, _) = update(&beta(0.5, 0.5), Seen::Binomial { trials: 10, k: 0.0 }).unwrap();
        close(ln, -1.736_152_296_596_451_7, 1e-15);
        let (ln, _) = update(&Family::gamma(0.7, 0.2).unwrap(), Seen::Poisson { k: 5.0 }).unwrap();
        close(ln, -9.850_813_770_178_176, 1e-15);
        // A density above 1.
        let (ln, _) = update(&Family::normal(0.0, 0.001).unwrap(), Seen::Normal { y: 0.0, sd: 0.001 }).unwrap();
        close(ln, 5.642_243_155_497_492, 1e-15);
        // A huge scale, and a tiny one.
        let (ln, _) = update(&Family::gamma(2.0, 1e12).unwrap(), Seen::Poisson { k: 3.0 }).unwrap();
        close(ln, -53.875_747_870_742_21, 1e-14);
        let (ln, _) = update(&Family::gamma(2.0, 1e-12).unwrap(), Seen::Poisson { k: 3.0 }).unwrap();
        close(ln, -81.506_768_986_670_75, 1e-14);
    }

    /// Updating one observation at a time gives the probability of all of
    /// them together, constants included.
    #[test]
    fn sequential_updates_give_the_whole_evidence() {
        let data = [(400u64, 12.0), (400, 9.0), (350, 15.0), (410, 0.0), (1, 1.0)];
        let (a, b) = (2.0, 50.0);
        let mut prior = beta(a, b);
        let mut total = 0.0;
        for (trials, k) in data {
            let (ln, after) = update(&prior, Seen::Binomial { trials, k }).unwrap();
            total += ln;
            prior = after;
        }
        let (n, k): (f64, f64) = data.iter().fold((0.0, 0.0), |(n, s), &(t, k)| (n + t as f64, s + k));
        let choose: f64 = data
            .iter()
            .map(|&(t, k)| -libm::log1p(t as f64) - ln_beta(k + 1.0, t as f64 - k + 1.0))
            .sum();
        close(total, choose + ln_beta(a + k, b + n - k) - ln_beta(a, b), 1e-13);
        assert_eq!(prior, beta(a + k, b + n - k));
    }
}
