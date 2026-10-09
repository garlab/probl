//! Observing values from continuous distributions when enumerating: each
//! finite world is weighed by its density (docs/semantics.md, section 13).
mod common;
use common::*;
use probl_engine::Options;

fn phi(z: f64) -> f64 {
    (-z * z / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

#[test]
fn finite_worlds_are_weighed_by_their_densities() {
    let o = outcome("let mu ~ one_of([0, 1])\nobserve 0 from normal(mu, 1)\nreport mu == 0");
    close(o.reports[0].chance().unwrap(), 1.0 / (1.0 + (-0.5f64).exp()));
    close(o.evidence.unwrap().ln(), (0.5 * (phi(0.0) + phi(1.0))).ln());
    assert!(o.densities);
    assert!(
        o.output.starts_with("enumerated · log evidence -1.1380"),
        "{}",
        o.output
    );
    // A density can be more than one.
    let o = outcome("observe 0 from normal(0, 0.1)\nreport true");
    close(o.evidence.unwrap().to_f64(), phi(0.0) / 0.1);
    assert!(o.output.starts_with("enumerated · log evidence 1.3836"), "{}", o.output);
}

#[test]
fn repeated_observations_and_aliases_multiply() {
    // ln L(0) − ln L(1) = Σ (1 − 2y) / 2 = 1 for these three.
    for src in [
        "let mu ~ one_of([0, 1])\nfor y in [0.2, -0.1, 0.4] { observe y from normal(mu, 1) }\nreport mu == 0",
        "let mu ~ one_of([0, 1])\nlet lik = normal(mu, 1)\nobserve 0.2 from lik\nobserve -0.1 from lik\nobserve 0.4 from lik\nreport mu == 0",
        "fn lik(m) { return normal(m, 1) }\nlet mu ~ one_of([0, 1])\nobserve 0.2 from lik(mu)\nobserve -0.1 from lik(mu)\nobserve 0.4 from lik(mu)\nreport mu == 0",
    ] {
        close(chance(src), 1.0 / (1.0 + (-1.0f64).exp()));
    }
    // A mixture of densities: 0.5 φ(x − μ) + 0.5 φ(x − μ − 3).
    let o =
        outcome("let mu ~ one_of([0, 1])\nobserve 2 from one_of([normal(mu, 1), normal(mu + 3, 1)])\nreport mu == 0");
    let l = |m: f64| 0.5 * phi(2.0 - m) + 0.5 * phi(2.0 - m - 3.0);
    close(o.reports[0].chance().unwrap(), l(0.0) / (l(0.0) + l(1.0)));
    // Densities and probabilities in the same world multiply.
    let o = outcome(
        "let mu ~ one_of([0, 1])\nlet c ~ bernoulli(0.5)\nobserve mu == 1 or c\nobserve 0 from normal(mu, 1)\nreport mu == 0",
    );
    close(
        o.reports[0].chance().unwrap(),
        0.5 * phi(0.0) / (0.5 * phi(0.0) + phi(1.0)),
    );
}

#[test]
fn tiny_densities_keep_their_ratios() {
    // Each density is about e^-800, which a float can't hold.
    let o = outcome(
        "let mu ~ one_of([0, 1])\nobserve 40 from normal(mu, 1)\nobserve -39 from normal(mu, 1)\nreport mu == 0",
    );
    close(o.reports[0].chance().unwrap(), 0.5);
    let ln_phi = |z: f64| -z * z / 2.0 - (2.0 * std::f64::consts::PI).sqrt().ln();
    let expected = ln_phi(40.0) + ln_phi(-39.0);
    assert!((o.evidence.unwrap().ln() - expected).abs() < 1e-9 * expected.abs());
    // Outside the support is impossible evidence.
    let e = error("let mu ~ one_of([0, 1])\nobserve 5 from uniform(mu, mu + 1)\nreport mu");
    assert!(e.contains("the evidence is impossible"), "{e}");
}

#[test]
fn what_the_weights_cant_bound_is_rejected() {
    for (src, what) in [
        (
            "let mu ~ one_of([0, 1])\nfn see(x) { observe x from normal(mu, 1) }\nsee(0)\nreport mu",
            "inside a function isn't supported when enumerating yet",
        ),
        (
            "let n ~ geometric(0.5)\nobserve 0.5 from normal(n, 1)\nreport n",
            "while some probability is unresolved",
        ),
        (
            "let c ~ bernoulli(0.5)\nif c { observe 0 from normal(0, 1) }\nreport c",
            "different numbers of continuous values",
        ),
        (
            "var i = 0\nwhile ~bernoulli(0.5) { i += 1\nobserve 0 from normal(0, 1) }\nreport i",
            "different numbers of continuous values",
        ),
        (
            "let mu ~ one_of([0, 1])\nobserve 1 from one_of([normal(mu, 1), 1])\nreport mu",
            "can't mix a density with the probabilities of single values",
        ),
    ] {
        let e = error(src);
        assert!(e.contains(what), "{src}: {e}");
    }
    // The same programs sample: these limits are enumeration's.
    for src in [
        "@mode sample(runs: 200, seed: 1)\nlet n ~ geometric(0.5)\nobserve 0.5 from normal(n, 1)\nreport n",
        "@mode sample(runs: 200, seed: 1)\nlet mu ~ one_of([0, 1])\nfn see(x) { observe x from normal(mu, 1) }\nsee(0)\nreport mu",
    ] {
        assert!(exec(src, &Options::default()).is_ok(), "{src}");
    }
}

#[test]
fn failed_weight_counts_when_it_has_as_many_densities() {
    let src =
        "@on_error partial\nlet mu ~ one_of([0, 1, 2])\nobserve 1 from normal(mu, 1)\nlet y = 1 / (mu - 2)\nreport mu";
    let o = exec_partial(src, &Options::default()).unwrap();
    assert!(o.failures.comparable());
    let failed = phi(1.0) / 3.0;
    let total = failed + (phi(1.0) + phi(0.0)) / 3.0;
    close(o.failures.weight.to_f64(), failed);
    assert!(
        o.output.contains(&format!("{:.2}% failed", 100.0 * failed / total)),
        "{}",
        o.output
    );
    // A world that failed instead of observing has fewer densities.
    let src = "@on_error partial\nlet c ~ bernoulli(0.5)\nif c { observe 0 from normal(0, 1) } else { let y = 1 / 0 }\nreport c";
    let o = exec_partial(src, &Options::default()).unwrap();
    assert!(!o.failures.comparable() && o.failures.other_units);
    assert!(o.output.contains("log evidence of the finished worlds"), "{}", o.output);
}
