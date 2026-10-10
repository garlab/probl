//! Exact posteriors of continuous draws when enumerating: conjugate
//! observations, and branches and draws on a drawn probability
//! (docs/semantics.md, section 13).
mod common;
use common::*;
use probl_engine::Outcome;
use probl_engine::continuous::{Family, Mixture};
use probl_engine::report::analytic_mixture;
use probl_engine::value::Value;

fn marginal(out: &Outcome, i: usize) -> Mixture {
    analytic_mixture(&out.reports[i].distribution()).expect("a continuous report")
}

/// A report's marginal has `family`'s quantiles and moments.
#[track_caller]
fn is(out: &Outcome, i: usize, family: Family) {
    let m = marginal(out, i);
    close(m.mean(), family.mean());
    close(m.variance(), family.variance());
    for q in [0.05, 0.5, 0.95] {
        assert!((m.quantile(q) - family.quantile(q)).abs() < 1e-9, "quantile {q}");
    }
}

fn beta(a: f64, b: f64) -> Family {
    Family::beta(a, b).unwrap()
}

#[test]
fn conjugate_observations_update_the_draw() {
    let o = outcome("let p ~ beta(2, 3)\nobserve 3 from binomial(5, p)\nreport p");
    close(o.evidence.unwrap().to_f64(), 4.0 / 21.0);
    is(&o, 0, beta(5.0, 5.0));

    let o = outcome("let rate ~ gamma(2, 3)\nobserve 4 from poisson(rate)\nreport rate");
    is(&o, 0, Family::gamma(6.0, 0.75).unwrap());
    // The negative binomial: Γ(6) / (Γ(2) 4!) (3/4)⁴ (1/4)².
    close(o.evidence.unwrap().to_f64(), 5.0 * 0.75f64.powi(4) * 0.0625);

    let o = outcome("let mu ~ normal(1, 2)\nobserve 1.5 from normal(mu, 0.5)\nreport mu");
    let spread = 4.25f64.sqrt();
    is(
        &o,
        0,
        Family::normal(1.0 + 0.5 * 4.0 / 4.25, 2.0 * 0.5 / spread).unwrap(),
    );
    assert!(o.densities);
    close(
        o.evidence.unwrap().to_f64(),
        Family::normal(1.0, spread).unwrap().pdf(1.5),
    );

    // `score p` is `observe true from bernoulli(p)`; a uniform probability
    // is a beta(1, 1), and an exponential rate a gamma with shape 1.
    let o = outcome("let p ~ uniform(0, 1)\nscore p\nscore p\nreport p");
    is(&o, 0, beta(3.0, 1.0));
    close(o.evidence.unwrap().to_f64(), 1.0 / 3.0);
    let o = outcome("let rate ~ exponential(1)\nobserve 2 from poisson(rate)\nreport rate");
    is(&o, 0, Family::gamma(3.0, 0.5).unwrap());
    close(o.evidence.unwrap().to_f64(), 0.125);
}

#[test]
fn repeated_observations_add_up() {
    // A loop, a call and the whole data at once agree.
    let a = outcome("let p ~ uniform(0, 1)\nfor k in [1, 0, 1, 1] { observe k == 1 from bernoulli(p) }\nreport p");
    let b = outcome("let p ~ uniform(0, 1)\nfn see(k) { observe k from binomial(4, p) }\nsee(3)\nreport p");
    is(&a, 0, beta(4.0, 2.0));
    is(&b, 0, beta(4.0, 2.0));
    // The same posterior; the evidence of the sequence leaves out C(4, 3).
    close(a.evidence.unwrap().to_f64(), 1.0 / 20.0);
    close(b.evidence.unwrap().to_f64(), 1.0 / 5.0);
}

#[test]
fn restrictions_and_updates_commute() {
    // beta(5, 5) on p > 0.5, with half of its probability there.
    let before = "let p ~ beta(2, 3)\nobserve p > 0.5\nobserve 3 from binomial(5, p)\nreport p\nreport p > 0.7";
    let after = "let p ~ beta(2, 3)\nobserve 3 from binomial(5, p)\nobserve p > 0.5\nreport p\nreport p > 0.7";
    let posterior = beta(5.0, 5.0);
    for src in [before, after] {
        let o = outcome(src);
        close(o.evidence.unwrap().to_f64(), 2.0 / 21.0);
        close(o.reports[1].chance().unwrap(), (1.0 - posterior.cdf(0.7)) / 0.5);
        let m = marginal(&o, 0);
        close(m.quantile(0.0), 0.5);
        close(m.cdf(0.6), (posterior.cdf(0.6) - 0.5) / 0.5);
    }
    // A uniform on part of [0, 1] is a beta(1, 1) there.
    let o = outcome("let p ~ uniform(0.2, 0.6)\nobserve true from bernoulli(p)\nreport p");
    close(o.evidence.unwrap().to_f64(), 0.4);
    let m = marginal(&o, 0);
    // The density is 2p / (0.6² − 0.2²) on [0.2, 0.6].
    close(m.cdf(0.4), (0.4f64.powi(2) - 0.04) / 0.32);
}

#[test]
fn aliases_see_the_update() {
    let o = outcome(
        "let p ~ beta(2, 3)\nlet q = p\nlet e = p > 0.5\nlet f = (n) -> n * q\nlet xs = [p, 1]\nobserve 3 from binomial(5, q)\nreport e\nreport f(10)\nreport xs[0]",
    );
    close(o.reports[0].chance().unwrap(), 0.5);
    close(marginal(&o, 1).mean(), 5.0);
    is(&o, 2, beta(5.0, 5.0));
}

#[test]
fn branches_on_a_drawn_probability_update_it() {
    let o =
        outcome("let p ~ beta(2, 3)\nlet hit = if p { true } else { false }\nreport hit\nreport p\nreport p by hit");
    close(o.reports[0].chance().unwrap(), 0.4);
    // Together, the two branches give back the prior.
    is(&o, 1, beta(2.0, 3.0));
    let group = |key: bool| analytic_mixture(&o.reports[2].groups[&Value::Bool(key)].distribution()).unwrap();
    close(group(true).mean(), beta(3.0, 3.0).mean());
    close(group(false).mean(), beta(2.0, 4.0).mean());
    // Two branches on the same p agree more often than independent ones:
    // E[p²] + E[(1 − p)²] = 0.2 + 0.4, not 0.4² + 0.6².
    close(
        chance("let p ~ beta(2, 3)\nlet a = if p { 1 } else { 0 }\nlet b = if p { 1 } else { 0 }\nreport a == b"),
        0.6,
    );
}

#[test]
fn draws_from_a_drawn_probability_update_it() {
    let o = outcome("let p ~ beta(2, 3)\nlet k ~ binomial(5, p)\nlet b ~ bernoulli(p)\nreport k\nreport b by k");
    // The beta-binomial: mean 5 · 2/5, variance 5 · 0.24 · 10/6.
    let k = o.reports[0].distribution();
    let mean: f64 = k.iter().map(|(v, p)| v.as_f64().unwrap() * p).sum();
    let variance: f64 = k.iter().map(|(v, p)| (v.as_f64().unwrap() - mean).powi(2) * p).sum();
    close(mean, 2.0);
    close(variance, 2.0);
    for n in 0..=5 {
        let group = &o.reports[1].groups[&Value::Int(n.into())];
        close(group.chance(), (2.0 + n as f64) / 10.0);
    }
    close(
        chance("let p ~ beta(2, 3)\nobserve true from bernoulli(p)\nlet c ~ bernoulli(p)\nreport c"),
        0.5,
    );
}

#[test]
fn what_isnt_conjugate_is_unsupported() {
    for (src, what) in [
        (
            "let x ~ normal(0, 1)\nobserve true from bernoulli(x)\nreport x",
            "isn't supported",
        ),
        (
            "let x ~ uniform(0, 2)\nlet b = if x { 1 } else { 0 }\nreport b",
            "isn't supported",
        ),
        (
            "let p ~ beta(2, 3)\nobserve true from bernoulli(1 - p)\nreport p",
            "isn't supported",
        ),
        (
            "let p ~ beta(2, 3)\nlet c = chance { p => 1, else => 0 }\nreport c",
            "isn't supported",
        ),
        (
            "let rate ~ gamma(2, 1)\nlet k ~ poisson(rate)\nreport k",
            "isn't supported",
        ),
        (
            "let mu ~ normal(0, 1)\nfn see(y) { observe y from normal(mu, 1) }\nsee(1)\nreport mu",
            "inside a function isn't supported",
        ),
        // Not impossible, but far beyond what the coordinates can hold.
        (
            "let p ~ beta(1, 1)\nobserve p > 0.9\nobserve 0 from binomial(1000, p)\nreport p",
            "too little probability to represent",
        ),
    ] {
        let e = error(src);
        assert!(e.contains(what), "{src}: {e}");
    }
}
