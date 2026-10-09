//! Count laws: `binomial`, `poisson` and `geometric` keep their law, so that
//! queries about them are exact, and a law too broad to list is still
//! drawn from and queried (docs/semantics.md, section 2).
mod common;
use common::*;
use probl_engine::Options;
use probl_engine::value::Value;

/// The value of a program's single report.
fn value(src: &str) -> Value {
    let d = distribution(src);
    assert_eq!(d.len(), 1, "{src}: {d:?}");
    d[0].0.clone()
}

fn prob(src: &str) -> f64 {
    match value(src) {
        Value::Prob(p) => p,
        v => panic!("{src}: {v:?}"),
    }
}

#[test]
fn law_queries_are_exact_despite_the_tail() {
    let e3 = (-3.0f64).exp();
    close(prob("report cdf(poisson(3), 2)"), e3 * 8.5);
    close(prob("report pmf(poisson(3), 2)"), e3 * 4.5);
    close(prob("report cdf(poisson(3), 2.5)"), e3 * 8.5);
    close(prob("report cdf(geometric(0.1), 10)"), 1.0 - 0.9f64.powi(10));
    close(prob("report pmf(geometric(0.5), 3)"), 0.125);
    // C(10, k) 0.3^k 0.7^(10 − k), summed to 3.
    let b = |k: i32| [1.0, 10.0, 45.0, 120.0][k as usize] * 0.3f64.powi(k) * 0.7f64.powi(10 - k);
    close(prob("report cdf(binomial(10, 0.3), 3)"), (0..4).map(b).sum());
    // An alias asks the same law, and `pmf` is of ints, as of the outcomes.
    close(prob("let d = poisson(3)\nreport pmf(d, 2)"), e3 * 4.5);
    assert_eq!(prob("report pmf(poisson(3), 2.0)"), 0.0);
    // What's computed from the outcomes keeps their tail.
    assert!(error("report P(poisson(3) > 2)").contains("unresolved"));
}

#[test]
fn a_law_too_broad_to_list_is_drawn_and_queried() {
    let d = "let d = geometric(0.000000000001)\n";
    assert_eq!(value(&format!("{d}report mean(d)")), Value::Float(1e12));
    close(prob(&format!("{d}report cdf(d, 1000000000000)")), 1.0 - (-1.0f64).exp());
    assert_eq!(value(&format!("{d}report pmf(d, 1)")), Value::Prob(1e-12));
    // ⌈ln(1/2) / ln(1 − 10⁻¹²)⌉.
    assert_eq!(
        value(&format!("{d}report median_low(d)")),
        Value::Int(693_147_180_560i64.into())
    );
    assert_eq!(value(&format!("{d}report typeof d")), Value::str("dist[int]"));
    assert_eq!(
        value("let d: dist[int] = geometric(0.000000000001)\nreport mean(d) > 0"),
        Value::Bool(true)
    );
    // Sampling draws from it directly, whether it's named or not.
    let sampled = |src: &str| {
        exec(
            &format!("@mode sample(runs: 2000, seed: 3)\n{src}"),
            &Options::default(),
        )
        .unwrap()
        .output
    };
    let named = sampled(&format!("{d}let n ~ d\nreport n > 1000000000000"));
    let direct = sampled("let n ~ geometric(0.000000000001)\nreport n > 1000000000000");
    assert_eq!(named.replace("n ~ d", ""), direct);
    // Anything that needs its outcomes fails as listing them did.
    for tail in [
        "let n ~ d\nreport n",
        "report d + 1",
        "report d",
        "report one_of([d, 1])",
    ] {
        let e = error(&format!("{d}{tail}"));
        assert!(e.contains("over the limit"), "{tail}: {e}");
    }
    // Observing from it uses its probabilities.
    let o = outcome("let r ~ one_of([0.000000000001, 0.5])\nobserve 3 from geometric(r)\nreport r == 0.5");
    let tiny = 1e-12 * (1.0 - 1e-12f64).powi(2);
    close(o.reports[0].chance().unwrap(), 0.125 / (0.125 + tiny));
}

#[test]
fn broad_poisson_and_binomial_laws_have_exact_moments_only() {
    assert_eq!(value("report mean(poisson(1000000000000))"), Value::Float(1e12));
    close(
        match value("report sd(binomial(1000000000000000, 0.5))") {
            Value::Float(x) => x,
            v => panic!("{v:?}"),
        },
        (2.5e14f64).sqrt(),
    );
    for query in ["median(poisson(1000000000000))", "cdf(poisson(1000000000000), 5)"] {
        let e = error(&format!("report {query}"));
        assert!(
            e.contains("too many outcomes to list, isn't supported yet"),
            "{query}: {e}"
        );
    }
}
