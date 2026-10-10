//! `@integrate`: observations that aren't conjugate, and moments without
//! formulas, by numerical integration to a tolerance, which the results
//! say (docs/semantics.md, section 13).
mod common;
use common::*;
use probl_engine::continuous::{Family, Mixture};
use probl_engine::report::analytic_mixture;
use probl_engine::{ErrorKind, Options, Outcome};
use probl_sema::ir::Mode;

fn marginal(out: &Outcome, i: usize) -> Mixture {
    analytic_mixture(&out.reports[i].distribution()).expect("a continuous report")
}

#[track_caller]
fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance * expected.abs().max(1.0),
        "expected {expected}, got {actual}"
    );
}

/// ∫ f over [lo, hi] by the midpoint rule on `n` points: a reference that
/// shares nothing with the engine's quadrature.
fn midpoint(f: impl Fn(f64) -> f64, lo: f64, hi: f64, n: usize) -> f64 {
    let h = (hi - lo) / n as f64;
    (0..n).map(|i| f(lo + (i as f64 + 0.5) * h)).sum::<f64>() * h
}

#[test]
fn integration_is_opt_in_and_says_so() {
    let src = "let p ~ beta(2, 3)\nobserve true from bernoulli(1 - p)\nreport p";
    let e = exec_raw(src, &Options::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert!(e.help.as_deref().unwrap_or("").contains("@integrate"), "{e:?}");
    let o = outcome(&format!("@integrate\n{src}"));
    assert_eq!(o.integration, Some(1e-9));
    assert!(o.output.starts_with("enumerated · integrated to 1e-9"), "{}", o.output);
    let o = exec(
        src,
        &Options {
            integrate: Some(1e-6),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(o.integration, Some(1e-6));
    // Exact results aren't marked, with `@integrate` or without.
    let o = outcome("@integrate\nlet p ~ beta(2, 3)\nobserve 3 from binomial(5, p)\nreport p");
    assert_eq!(o.integration, None);
    // A tolerance it can't use.
    assert!(compile_error("@integrate 0.5\nreport 1").contains("tolerance"));
}

#[test]
fn a_conjugate_pair_in_disguise_matches_its_formula() {
    // Seeing `true` from bernoulli(1 - p) is seeing a failure: beta(2, 4).
    let o = outcome("@integrate\nlet p ~ beta(2, 3)\nobserve true from bernoulli(1 - p)\nreport p\nreport p > 0.5");
    let m = marginal(&o, 0);
    let beta = Family::beta(2.0, 4.0).unwrap();
    near(m.mean(), beta.mean(), 1e-8);
    near(m.sd(), beta.sd(), 1e-8);
    for q in [0.05, 0.5, 0.95] {
        near(m.quantile(q), beta.quantile(q), 1e-7);
    }
    near(o.reports[1].chance().unwrap(), 6.0 / 32.0, 1e-8);
    near(o.evidence.unwrap().to_f64(), 0.6, 1e-8);
}

#[test]
fn a_uniform_prior_and_a_normal_likelihood_make_a_truncated_normal() {
    let o = outcome("@integrate\nlet x ~ uniform(0, 10)\nobserve 4.1 from normal(x, 1)\nreport x");
    let n = Family::normal(4.1, 1.0).unwrap();
    let (mean, variance) = n.interval_moments(0.0, 10.0);
    let m = marginal(&o, 0);
    near(m.mean(), mean, 1e-8);
    near(m.variance(), variance, 1e-7);
    near(o.evidence.unwrap().to_f64(), (n.cdf(10.0) - n.cdf(0.0)) / 10.0, 1e-8);
    assert!(o.densities);
}

#[test]
fn a_lognormal_rate_seen_through_poisson_counts() {
    let o = outcome(
        "@integrate\nlet rate ~ lognormal(0, 1)\nobserve 3 from poisson(rate)\nobserve 5 from poisson(rate)\nreport rate\nreport rate > 2",
    );
    // The posterior, unnormalized, in log(rate): a normal prior times both
    // likelihoods.
    let prior = Family::normal(0.0, 1.0).unwrap();
    let ln_poisson = |k: f64, r: f64| k * r.ln() - r - libm::lgamma(k + 1.0);
    let post = |z: f64| {
        let r = z.exp();
        prior.pdf(z) * (ln_poisson(3.0, r) + ln_poisson(5.0, r)).exp()
    };
    let n = 400_000;
    let z = midpoint(post, -8.0, 8.0, n);
    let mean = midpoint(|t| t.exp() * post(t), -8.0, 8.0, n) / z;
    let above = midpoint(|t| if t.exp() > 2.0 { post(t) } else { 0.0 }, -8.0, 8.0, n) / z;
    near(marginal(&o, 0).mean(), mean, 1e-7);
    near(o.reports[1].chance().unwrap(), above, 1e-5);
    near(o.evidence.unwrap().to_f64(), z, 1e-7);
}

#[test]
fn a_nonlinear_mean_gives_a_bimodal_posterior() {
    let o = outcome(
        "@integrate\nlet mu ~ normal(0, 2)\nobserve 1.2 from normal(mu * mu, 0.5)\nreport mu\nreport mu > 0\nreport abs(mu)",
    );
    let prior = Family::normal(0.0, 2.0).unwrap();
    let like = Family::normal(0.0, 0.5).unwrap();
    let post = |m: f64| prior.pdf(m) * like.pdf(1.2 - m * m);
    let n = 400_000;
    let z = midpoint(post, -12.0, 12.0, n);
    let abs_mean = midpoint(|m| m.abs() * post(m), -12.0, 12.0, n) / z;
    near(marginal(&o, 0).mean(), 0.0, 1e-8);
    near(o.reports[1].chance().unwrap(), 0.5, 1e-8);
    near(marginal(&o, 2).mean(), abs_mean, 1e-7);
    near(o.evidence.unwrap().ln(), z.ln(), 1e-7);
}

#[test]
fn restrictions_numerical_and_conjugate_observations_combine() {
    let o = outcome(
        "@integrate\nlet p ~ beta(2, 3)\nobserve true from bernoulli(1 - p)\nobserve p > 0.2\nobserve 3 from binomial(5, p)\nreport p\nreport p > 0.5",
    );
    // beta(2, 3) × (1 − p) × p³(1 − p)² × 10, on p > 0.2.
    let prior = Family::beta(2.0, 3.0).unwrap();
    let post = |p: f64| prior.pdf(p) * (1.0 - p) * 10.0 * p.powi(3) * (1.0 - p).powi(2);
    let n = 400_000;
    let z = midpoint(post, 0.2, 1.0, n);
    let mean = midpoint(|p| p * post(p), 0.2, 1.0, n) / z;
    let above = midpoint(post, 0.5, 1.0, n) / z;
    near(marginal(&o, 0).mean(), mean, 1e-7);
    near(o.reports[1].chance().unwrap(), above, 1e-7);
    near(o.evidence.unwrap().to_f64(), z, 1e-7);
}

#[test]
fn moments_without_formulas_are_integrated() {
    let o = outcome("@integrate\nlet p ~ beta(2, 3)\nreport exp(p)");
    let beta = Family::beta(2.0, 3.0).unwrap();
    let n = 200_000;
    let mean = midpoint(|p| p.exp() * beta.pdf(p), 0.0, 1.0, n);
    let second = midpoint(|p| (2.0 * p).exp() * beta.pdf(p), 0.0, 1.0, n);
    let numeric = o.results[0].groups[0].numeric.as_ref().unwrap();
    assert!(numeric.integrated);
    near(numeric.mean.point.unwrap(), mean, 1e-8);
    near(numeric.sd.point.unwrap(), (second - mean * mean).sqrt(), 1e-6);
    assert_eq!(o.integration, Some(1e-9));
    // Without `@integrate`, unavailable.
    let o = outcome("let p ~ beta(2, 3)\nreport exp(p)");
    assert!(o.results[0].groups[0].numeric.as_ref().unwrap().mean.point.is_none());
    assert_eq!(o.integration, None);
    // A mean that doesn't exist isn't given: E[1/x] diverges when the
    // posterior doesn't vanish at 0.
    let o = outcome("@integrate\nlet x ~ uniform(0, 1)\nobserve true from bernoulli(1 - x)\nreport 1 / x");
    assert!(o.output.contains("unavailable"), "{}", o.output);
}

#[test]
fn a_local_posterior_by_integration_is_a_recipe() {
    let src = "@integrate\nlet d = simulate {\n let p ~ beta(2, 3)\n observe true from bernoulli(1 - p)\n p\n}\nlet a ~ d\nlet b ~ d\nreport mean(d)\nreport a\nreport a > 0.5 and b > 0.5";
    let o = outcome(src);
    near(o.reports[0].distribution()[0].0.as_f64().unwrap(), 1.0 / 3.0, 1e-8);
    near(marginal(&o, 1).mean(), 1.0 / 3.0, 1e-8);
    let p = 6.0 / 32.0;
    near(o.reports[2].chance().unwrap(), p * p, 1e-8);
    let sampled = exec(
        src,
        &Options {
            mode: Some(Mode::Sample { runs: 20_000, seed: 4 }),
            ..Options::default()
        },
    )
    .unwrap();
    let mean: f64 = sampled.reports[1]
        .distribution()
        .iter()
        .map(|(v, p)| v.as_f64().unwrap() * p)
        .sum();
    assert!((mean - 1.0 / 3.0).abs() < 0.01, "{mean}");
}

#[test]
fn parameters_outside_their_range_fault_or_are_unsupported() {
    // Outside [0, 1] on part of the draw's range.
    let e = error("@integrate\nlet x ~ normal(0.5, 0.2)\nobserve 3 from binomial(5, x)\nreport x");
    assert!(e.contains("outside its range on part"), "{e}");
    // Everywhere: the fault a number would make, which `catch` can take.
    let e = exec_raw(
        "@integrate\nlet x ~ normal(-5, 0.1)\nobserve 1 from poisson(x)\nreport x",
        &Options::default(),
    )
    .unwrap_err();
    assert!(e.fault.is_some(), "{e:?}");
    // Draws that depend on each other, and sums, have several dimensions.
    let e = error(
        "@integrate\nlet x ~ uniform(0, 1)\nlet y ~ uniform(0, 1)\nobserve true from bernoulli(x * 0 + y * 0 + 0.5)\nobserve 1 from poisson(x + y)\nreport x",
    );
    assert!(e.contains("isn't supported"), "{e}");
}

#[test]
fn densities_keep_their_units() {
    // A normal likelihood is a density, counted as one.
    let e = error(
        "@integrate\nlet k ~ one_of([0, 1])\nlet x ~ uniform(0, 1)\nif k == 1 { observe 0.5 from normal(x * x, 1) }\nreport x",
    );
    assert!(e.contains("different numbers of continuous values"), "{e}");
    let e = error("@integrate\nlet x ~ uniform(0, 1)\nfn see() { observe 0.5 from normal(x * x, 1) }\nsee()\nreport x");
    assert!(e.contains("inside a function"), "{e}");
}
