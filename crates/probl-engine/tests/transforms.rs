//! Nonlinear functions of a continuous draw when enumerating: squares,
//! square roots, exponentials, logarithms and reciprocals keep the draw's
//! identity, with CDFs from inverse images and moments from formulas
//! (docs/semantics.md, section 13).
mod common;
use common::*;
use probl_engine::Outcome;
use probl_engine::continuous::{Family, Mixture};
use probl_engine::report::analytic_mixture;
use probl_engine::value::Value;

/// A function of the draw, as the reference integrates it.
type Function<'a> = &'a dyn Fn(f64) -> f64;

fn marginal(out: &Outcome, i: usize) -> Mixture {
    analytic_mixture(&out.reports[i].distribution()).expect("a continuous report")
}

/// The mean and variance of `g(X)` for X from `family`, where X is above
/// `above`, by the midpoint rule over a fine grid: a reference computed
/// independently of the formulas.
fn integrate(family: Family, above: f64, g: impl Fn(f64) -> f64) -> (f64, f64) {
    let (lo, hi) = (family.quantile(1e-13).max(above), family.quantile(1.0 - 1e-13));
    // A heavy integrand, like e^(0.4x) against a gamma density, reaches
    // well past where the density's own tail ends.
    let hi = if family.support().1.is_infinite() {
        4.0 * hi.abs() + 10.0
    } else {
        hi
    };
    // On positive numbers, in t = ln x, which reaches toward 0, where
    // 1 / x² can be integrable but steep.
    let positive = family.support().0 == 0.0 && above <= 0.0;
    let (from, to) = if positive { (lo.ln() - 40.0, hi.ln()) } else { (lo, hi) };
    let n = 400_000;
    let h = (to - from) / n as f64;
    let (mut p, mut m1, mut m2) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let t = from + (i as f64 + 0.5) * h;
        let (x, dx) = if positive { (t.exp(), t.exp() * h) } else { (t, h) };
        let w = family.pdf(x) * dx;
        let y = g(x);
        p += w;
        m1 += w * y;
        m2 += w * y * y;
    }
    let mean = m1 / p;
    (mean, m2 / p - mean * mean)
}

#[track_caller]
fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance * (1.0 + expected.abs()),
        "{actual} vs {expected}"
    );
}

#[test]
fn moments_agree_with_integration() {
    let cases: &[(&str, Family, &str, Function)] = &[
        ("uniform(-1, 1)", Family::uniform(-1.0, 1.0).unwrap(), "x * x", &|x| {
            x * x
        }),
        ("uniform(1, 2)", Family::uniform(1.0, 2.0).unwrap(), "sqrt(x)", &|x| {
            x.sqrt()
        }),
        ("uniform(1, 2)", Family::uniform(1.0, 2.0).unwrap(), "ln(x)", &|x| {
            x.ln()
        }),
        ("uniform(1, 2)", Family::uniform(1.0, 2.0).unwrap(), "1 / x", &|x| {
            1.0 / x
        }),
        ("uniform(1, 2)", Family::uniform(1.0, 2.0).unwrap(), "2 ^ x", &|x| {
            2f64.powf(x)
        }),
        (
            "uniform(1, 2)",
            Family::uniform(1.0, 2.0).unwrap(),
            "(x + 1) * (x - 2)",
            &|x| (x + 1.0) * (x - 2.0),
        ),
        ("normal(0.5, 1.2)", Family::normal(0.5, 1.2).unwrap(), "exp(x)", &|x| {
            x.exp()
        }),
        (
            "normal(0.5, 1.2)",
            Family::normal(0.5, 1.2).unwrap(),
            "(x - 1) ^ 2",
            &|x| (x - 1.0).powi(2),
        ),
        (
            "normal(0.5, 1.2)",
            Family::normal(0.5, 1.2).unwrap(),
            "exp(-x / 2)",
            &|x| (-x / 2.0).exp(),
        ),
        (
            "lognormal(0.2, 0.5)",
            Family::lognormal(0.2, 0.5).unwrap(),
            "ln(x)",
            &|x| x.ln(),
        ),
        (
            "lognormal(0.2, 0.5)",
            Family::lognormal(0.2, 0.5).unwrap(),
            "sqrt(x)",
            &|x| x.sqrt(),
        ),
        (
            "lognormal(0.2, 0.5)",
            Family::lognormal(0.2, 0.5).unwrap(),
            "1 / x",
            &|x| 1.0 / x,
        ),
        ("gamma(2.5, 1.5)", Family::gamma(2.5, 1.5).unwrap(), "sqrt(x)", &|x| {
            x.sqrt()
        }),
        ("gamma(2.5, 1.5)", Family::gamma(2.5, 1.5).unwrap(), "x * x", &|x| x * x),
        ("gamma(2.5, 1.5)", Family::gamma(2.5, 1.5).unwrap(), "1 / x", &|x| {
            1.0 / x
        }),
        (
            "gamma(2.5, 1.5)",
            Family::gamma(2.5, 1.5).unwrap(),
            "exp(0.2 * x)",
            &|x| (0.2 * x).exp(),
        ),
        ("exponential(2)", Family::exponential(2.0).unwrap(), "x * x + x", &|x| {
            x * x + x
        }),
        ("beta(2, 3)", Family::beta(2.0, 3.0).unwrap(), "sqrt(x)", &|x| x.sqrt()),
        ("beta(2, 3)", Family::beta(2.0, 3.0).unwrap(), "x * x", &|x| x * x),
    ];
    for &(recipe, family, expr, g) in cases {
        let o = outcome(&format!("let x ~ {recipe}\nreport {expr}"));
        let m = marginal(&o, 0);
        let (mean, variance) = integrate(family, f64::NEG_INFINITY, g);
        near(m.mean(), mean, 1e-6);
        near(m.variance(), variance, 1e-5);
    }
    // On part of the draw's range.
    let cases: &[(&str, Family, f64, &str, Function)] = &[
        ("normal(0, 1)", Family::normal(0.0, 1.0).unwrap(), 0.5, "exp(x)", &|x| {
            x.exp()
        }),
        ("normal(0, 1)", Family::normal(0.0, 1.0).unwrap(), 0.5, "x * x", &|x| {
            x * x
        }),
        (
            "uniform(-1, 2)",
            Family::uniform(-1.0, 2.0).unwrap(),
            0.0,
            "sqrt(x)",
            &|x| x.sqrt(),
        ),
        (
            "gamma(2, 1)",
            Family::gamma(2.0, 1.0).unwrap(),
            1.0,
            "ln(x) * 1",
            &|x| x.ln(),
        ),
    ];
    for &(recipe, family, above, expr, g) in cases {
        let src = format!("let x ~ {recipe}\nobserve x > {above}\nreport {expr}");
        let o = outcome(&src);
        let (mean, variance) = integrate(family, above, g);
        let m = marginal(&o, 0);
        if expr.starts_with("ln") {
            // No formula for the log of a gamma: quantiles only.
            assert!(m.mean().is_nan(), "{src}");
            assert!(o.output.contains("mean unavailable"), "{}", o.output);
            continue;
        }
        near(m.mean(), mean, 1e-6);
        near(m.variance(), variance, 1e-5);
    }
}

#[test]
fn cdfs_and_quantiles_follow_inverse_images() {
    let o = outcome("let x ~ uniform(-1, 1)\nreport x * x");
    let m = marginal(&o, 0);
    for y in [0.01, 0.25, 0.5, 0.81] {
        near(m.cdf(y), f64::sqrt(y), 1e-12);
    }
    for q in [0.05, 0.5, 0.95] {
        near(m.quantile(q), q * q, 1e-9);
    }
    // e^Z is lognormal, quantile for quantile.
    let o = outcome("let z ~ normal(0, 1)\nreport exp(z)");
    let lognormal = Family::lognormal(0.0, 1.0).unwrap();
    for q in [0.05, 0.5, 0.95] {
        near(marginal(&o, 0).quantile(q), lognormal.quantile(q), 1e-12);
    }
    // 1/x for x from 1 to 2: P(1/x ≤ y) = 2 − 1/y.
    let o = outcome("let x ~ uniform(1, 2)\nreport 1 / x");
    near(marginal(&o, 0).cdf(0.8), 2.0 - 1.0 / 0.8, 1e-12);
    // Across a pole: half below −1, half above 1.
    let o = outcome("let x ~ uniform(-1, 1)\nreport 1 / x\nreport 1 / x > 2\nreport 1 / x < -10");
    near(marginal(&o, 0).quantile(0.05), -10.0, 1e-9);
    near(o.reports[1].chance().unwrap(), 0.25, 1e-12);
    near(o.reports[2].chance().unwrap(), 0.05, 1e-12);
    assert!(o.output.contains("mean unavailable"), "{}", o.output);
}

#[test]
fn observations_restrict_through_the_transform() {
    let phi = |z: f64| 0.5 * libm::erfc(-z / std::f64::consts::SQRT_2);
    let density = |z: f64| (-z * z / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt();
    let o = outcome("let z ~ normal(0, 1)\nobserve exp(z) > 2\nreport z");
    let tail = 1.0 - phi(2f64.ln());
    near(o.evidence.unwrap().to_f64(), tail, 1e-12);
    near(marginal(&o, 0).mean(), density(2f64.ln()) / tail, 1e-9);
    // Through an alias: x² < 1/4 is |x| < 1/2.
    let o = outcome("let x ~ uniform(-1, 1)\nlet y = x * x\nobserve y < 0.25\nreport x\nreport y < 0.0625");
    near(o.evidence.unwrap().to_f64(), 0.5, 1e-12);
    let m = marginal(&o, 0);
    near(m.quantile(0.0), -0.5, 1e-12);
    near(m.quantile(1.0), 0.5, 1e-12);
    near(o.reports[1].chance().unwrap(), 0.5, 1e-12);
    // A comparison of two functions of the draw: x² < x is 0 < x < 1.
    near(chance("let x ~ uniform(-1, 2)\nreport x * x < x"), 1.0 / 3.0, 1e-12);
}

#[test]
fn compositions_and_kin_simplify() {
    let o = outcome(
        "let x ~ uniform(-1, 1)\nlet z ~ normal(0, 1)\nlet u ~ uniform(1, 3)\nreport sqrt(x * x) - abs(x)\nreport ln(exp(z)) - z\nreport exp(ln(u)) - u",
    );
    for i in 0..3 {
        assert_eq!(
            o.reports[i].distribution(),
            vec![(Value::Float(0.0), 1.0)],
            "report {i}"
        );
    }
    // The other logarithms and exponentials are these, rescaled.
    let o = outcome(
        "let u ~ uniform(1, 3)\nreport log10(u)\nreport log2(u)\nreport log1p(u)\nreport expm1(u)\nreport exp2(u)",
    );
    let u = Family::uniform(1.0, 3.0).unwrap();
    let gs: [&dyn Fn(f64) -> f64; 5] = [&|x| x.log10(), &|x| x.log2(), &|x| x.ln_1p(), &|x| x.exp_m1(), &|x| {
        x.exp2()
    }];
    for (i, g) in gs.iter().enumerate() {
        near(marginal(&o, i).mean(), integrate(u, f64::NEG_INFINITY, g).0, 1e-6);
    }
    // Rounding and extremes work on any monotone pieces.
    let o = outcome("let x ~ uniform(-1, 2)\nreport floor(x * x)\nreport max(x * x, 0.5)");
    let d = o.reports[0].distribution();
    near(d[0].1, 2.0 / 3.0, 1e-12);
    near(d[1].1, (2f64.sqrt() - 1.0) / 3.0, 1e-12);
    let reference = integrate(Family::uniform(-1.0, 2.0).unwrap(), f64::NEG_INFINITY, |x| {
        (x * x).max(0.5)
    });
    near(marginal(&o, 1).mean(), reference.0, 1e-6);
}

#[test]
fn what_transforms_cant_do_yet() {
    for (src, what) in [
        ("let x ~ uniform(0, 1)\nreport sin(x)", "isn't supported"),
        ("let x ~ uniform(0, 1)\nreport x * x * x", "nonlinear arithmetic"),
        (
            "let x ~ uniform(0, 1)\nreport exp(x * x)",
            "composition of continuous transforms",
        ),
        (
            "let x ~ uniform(-1, 1)\nreport sqrt(x)",
            "outside its domain on part of its range",
        ),
        (
            "let x ~ normal(0, 1000)\nreport exp(x)",
            "outside its domain on part of its range",
        ),
        ("let x ~ uniform(1, 2)\nreport x ^ 3", "this power"),
    ] {
        let e = error(src);
        assert!(e.contains(what), "{src}: {e}");
    }
    // Outside the domain everywhere is the fault a number would make.
    let o = outcome("let x ~ uniform(-2, -1)\nlet y = try { sqrt(x) } catch DomainError { -1 }\nreport y");
    assert_eq!(o.reports[0].distribution(), vec![(Value::Int((-1).into()), 1.0)]);
    // Formulas cover some families' moments, not all: the quantiles remain.
    let o = outcome("let b ~ beta(2, 3)\nreport exp(b)");
    assert!(
        o.output.contains("mean unavailable · sd unavailable · 5% 1.10"),
        "{}",
        o.output
    );
}
