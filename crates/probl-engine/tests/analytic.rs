mod common;
use common::*;
use probl_engine::continuous::Mixture;
use probl_engine::report::analytic_mixture;
use probl_engine::value::Value;
use probl_engine::{Options, Outcome};

fn marginal(out: &Outcome, i: usize) -> Mixture {
    analytic_mixture(&out.reports[i].distribution()).expect("continuous numeric report")
}
fn stats(out: &Outcome, i: usize, mean: f64, variance: f64, lo: f64, hi: f64) {
    let m = marginal(out, i);
    close(m.mean(), mean);
    close(m.variance(), variance);
    close(m.quantile(0.0), lo);
    close(m.quantile(1.0), hi);
}

#[test]
fn affine_draws_are_scalars_with_shared_identity() {
    let o = outcome(
        "let x ~ uniform(0,2)\nlet y = x+1\nreport y\nreport x+x\nreport x-x\nreport 3-x/2\nreport typeof x\nreport typeof (x>1)",
    );
    stats(&o, 0, 2.0, 1.0 / 3.0, 1.0, 3.0);
    stats(&o, 1, 2.0, 4.0 / 3.0, 0.0, 4.0);
    stats(&o, 3, 2.5, 1.0 / 12.0, 2.0, 3.0);
    assert_eq!(o.reports[2].distribution(), vec![(Value::Float(0.0), 1.0)]);
    assert_eq!(o.reports[4].distribution(), vec![(Value::str("float"), 1.0)]);
    assert_eq!(o.reports[5].distribution(), vec![(Value::str("bool"), 1.0)]);
    close(marginal(&o, 0).cdf(2.0), 0.5);
    close(marginal(&o, 0).quantile(0.05), 1.1);
    assert!(
        o.output
            .contains("mean 2.00 · sd 0.58 · 5% 1.10 · median 2.00 · 95% 2.90"),
        "{}",
        o.output
    );
}

#[test]
fn evidence_restricts_all_aliases_and_stored_events() {
    let o =
        outcome("let x ~ uniform(0,2)\nlet y=x+1\nlet e=x>1\nobserve e\nobserve e\nreport y\nreport e\nreport x<=1");
    close(o.evidence.unwrap().to_f64(), 0.5);
    stats(&o, 0, 2.5, 1.0 / 12.0, 2.0, 3.0);
    close(o.reports[1].chance().unwrap(), 1.0);
    close(o.reports[2].chance().unwrap(), 0.0);
    assert!(error("let x ~ uniform(0,2)\nobserve x>1\nobserve x<1\nreport x").contains("ruled out"));
    let o = outcome("let x ~ uniform(0,2)\nobserve ~(x>1)\nreport x");
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
}

#[test]
fn branching_splits_the_latent_and_recombines_reports() {
    let src = "let x ~ uniform(0,2)\nlet y=if x>1 {x+1} else {x-1}\nreport x\nreport y\nreport y>1";
    for merging in [false, true] {
        let options = Options {
            merge: merging,
            ..Options::default()
        };
        let o = exec(src, &options).unwrap();
        stats(&o, 0, 1.0, 1.0 / 3.0, 0.0, 2.0);
        stats(&o, 1, 1.0, 7.0 / 3.0, -1.0, 3.0);
        close(o.reports[2].chance().unwrap(), 0.5);
        close(marginal(&o, 1).cdf(1.0), 0.5);
    }
}

#[test]
fn logical_intervals_and_negative_affine_quantiles() {
    let o = outcome("let x ~ uniform(0,4)\nlet e=x<1 or x>3\nobserve e\nreport x\nreport -x\nreport x<1\nreport e");
    close(o.evidence.unwrap().to_f64(), 0.5);
    stats(&o, 0, 2.0, 7.0 / 3.0, 0.0, 4.0);
    close(marginal(&o, 1).quantile(0.5), -3.0);
    close(o.reports[2].chance().unwrap(), 0.5);
    close(o.reports[3].chance().unwrap(), 1.0);
    let o = outcome("let x ~ uniform(0,4)\nobserve not (x<=1 or x>=3)\nreport x\nreport x>1 and x<3");
    stats(&o, 0, 2.0, 1.0 / 3.0, 1.0, 3.0);
    close(o.reports[1].chance().unwrap(), 1.0);
}

#[test]
fn calls_propagate_restrictions_even_when_they_return_plain_values() {
    let o = outcome(
        "fn test(x: float) -> bool {if x>1 {true} else {false}}\nlet x ~ uniform(0,2)\nlet yes=test(x)\nobserve yes\nreport x",
    );
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
    close(o.evidence.unwrap().to_f64(), 0.5);
    let o = outcome(
        "fn keep(x: float) -> float {observe x>1\nx+1}\nlet x ~ uniform(0,2)\nlet y=keep(x)\nreport x\nreport y",
    );
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
    stats(&o, 1, 2.5, 1.0 / 12.0, 2.0, 3.0);
}

#[test]
fn independent_calls_never_share_fresh_latent_ids() {
    let src =
        "fn draw() -> float {let x ~ uniform(0,2)\nx}\nlet x=draw()\nlet y=draw()\nobserve x>1\nreport x\nreport y";
    for memoizing in [true, false] {
        let o = exec(
            src,
            &Options {
                memoize: memoizing,
                ..Options::default()
            },
        )
        .unwrap();
        stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
        stats(&o, 1, 1.0, 1.0 / 3.0, 0.0, 2.0);
    }
    assert!(
        error("fn draw() {let x ~ uniform(0,2)\nx}\nlet x=draw()\nlet y=draw()\nreport x-y").contains("independent")
    );
}

#[test]
fn aliases_in_aggregates_closures_and_callbacks_keep_identity() {
    let o = outcome(
        "let x ~ uniform(0,2)\nlet xs=[x,x+1]\nlet r={v:x}\nlet m=[\"v\":x]\nlet read=() -> x+2\nobserve x>1\nlet ys=xs.map(v -> v*2)\nreport ys[0]\nreport ys[1]\nreport r.v\nreport m[\"v\"]\nreport read()",
    );
    stats(&o, 0, 3.0, 1.0 / 3.0, 2.0, 4.0);
    stats(&o, 1, 5.0, 1.0 / 3.0, 4.0, 6.0);
    stats(&o, 2, 1.5, 1.0 / 12.0, 1.0, 2.0);
    stats(&o, 3, 1.5, 1.0 / 12.0, 1.0, 2.0);
    stats(&o, 4, 3.5, 1.0 / 12.0, 3.0, 4.0);
}

#[test]
fn loops_and_separate_latents_can_be_conditioned() {
    let o = outcome("let x ~ uniform(0,4)\nvar k=0\nwhile x>k+1 {k+=1}\nobserve k==2\nreport x");
    stats(&o, 0, 2.5, 1.0 / 12.0, 2.0, 3.0);
    close(o.evidence.unwrap().to_f64(), 0.25);
    let o = outcome("let x ~ uniform(0,2)\nlet y ~ uniform(0,4)\nobserve x>1\nobserve y<1\nreport x\nreport y");
    close(o.evidence.unwrap().to_f64(), 0.125);
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
    stats(&o, 1, 0.5, 1.0 / 12.0, 0.0, 1.0);
}

#[test]
fn recipes_mixtures_and_grouped_reports_have_numeric_summaries() {
    let o = outcome("report uniform(1,3)\nlet x ~ one_of([uniform(0,2),4])\nreport x");
    stats(&o, 0, 2.0, 1.0 / 3.0, 1.0, 3.0);
    stats(&o, 1, 2.5, 29.0 / 12.0, 0.0, 4.0);
    close(marginal(&o, 1).quantile(0.75), 4.0);
    close(marginal(&o, 1).cdf(2.0), 0.5);
    let s = output("let x ~ uniform(0,2)\nlet group=if x>1 {\"high\"} else {\"low\"}\nreport x by group");
    assert!(s.contains("median") && s.contains("1.50") && s.contains("0.50"), "{s}");
    assert!(!s.contains("<analytic"), "{s}");
}

#[test]
fn unsupported_uses_request_sampling_instead_of_losing_correlations() {
    for tail in [
        "report x*x",
        "report sin(x)",
        "report mean(x)",
        "report P(x>1)",
        "report [x]==[x+1]",
        "report x in [x]",
        "report x by x",
        "report [x]",
        "report bag([x])",
        "report [x: 1]",
        "report str(x)",
        "report prob(x)",
        "report simulate {x}",
        "if x {report true}",
        "report (x>1) and prob(50%)",
        "report if 50% {x} else {\"unknown\"}",
    ] {
        let e = error(&format!("let x ~ uniform(0,2)\n{tail}"));
        assert!(e.contains("@mode sample"), "{tail}: {e}");
    }
    let sampled = outcome("@mode sample(runs: 100, seed: 4)\nlet x ~ uniform(0,2)\nreport x*x");
    assert!(sampled.sample.is_some());
}

#[test]
fn truncated_moments_match_independent_density_integration() {
    use probl_engine::continuous::Family;
    // Check all family-specific incomplete-moment formulas against integration,
    // including triangular modes at the endpoints and shifted/scaled PERTs.
    let cases = [
        (Family::normal(7.0, 2.0).unwrap(), 4.0, 8.0),
        (Family::lognormal(0.3, 0.7).unwrap(), 0.4, 3.0),
        (Family::uniform(-3.0, 8.0).unwrap(), -1.0, 6.0),
        (Family::beta(2.0, 5.0).unwrap(), 0.1, 0.7),
        (Family::gamma(2.0, 3.0).unwrap(), 1.0, 8.0),
        (Family::exponential(0.7).unwrap(), 0.2, 3.0),
        (Family::triangular(-2.0, 1.0, 5.0).unwrap(), -1.0, 4.0),
        (Family::triangular(0.0, 0.0, 5.0).unwrap(), 1.0, 4.0),
        (Family::triangular(0.0, 5.0, 5.0).unwrap(), 1.0, 4.0),
        (Family::pert(-2.0, 1.0, 5.0).unwrap(), -1.0, 4.0),
    ];
    for (f, lo, hi) in cases {
        let (mut mass, mut first, mut second) = (0.0, 0.0, 0.0);
        let steps = 20_000;
        let dx = (hi - lo) / steps as f64;
        for i in 0..steps {
            let x = lo + (i as f64 + 0.5) * dx;
            let p = f.pdf(x) * dx;
            mass += p;
            first += p * x;
            second += p * x * x;
        }
        let (m, v) = f.interval_moments(lo, hi);
        assert!((m - first / mass).abs() < 1e-7, "{f}: mean {m}");
        assert!(
            (v - (second / mass - (first / mass).powi(2))).abs() < 1e-7,
            "{f}: variance {v}"
        );
        let src = format!("let x ~ {f}\nobserve x > {lo}\nobserve x < {hi}\nreport x");
        let o = outcome(&src);
        let result = marginal(&o, 0);
        close(result.mean(), m);
        close(result.variance(), v);
        close(o.evidence.unwrap().to_f64(), f.cdf(hi) - f.cdf(lo));
    }
    let o = outcome("let x ~ normal(0,1)\nobserve x>0\nreport x");
    close(marginal(&o, 0).mean(), (2.0 / std::f64::consts::PI).sqrt());
    close(marginal(&o, 0).variance(), 1.0 - 2.0 / std::f64::consts::PI);
}

#[test]
fn signed_comparisons_and_boolean_equality_retain_event_identity() {
    for op in [">", ">=", "<", "<="] {
        close(chance(&format!("let x ~ uniform(0,2)\nreport -x {op} -1")), 0.5);
        close(chance(&format!("let x ~ uniform(0,2)\nreport 1 {op} x")), 0.5);
    }
    for (test, p) in [
        ("x==x", 1.0),
        ("x!=x", 0.0),
        ("x==1", 0.0),
        ("x!=1", 1.0),
        ("(x>1)==(x<1)", 0.0),
        ("(x>1)==(x>1)", 1.0),
        ("(x>1)==true", 0.5),
    ] {
        close(chance(&format!("let x ~ uniform(0,2)\nreport {test}")), p);
    }
    let o = outcome("let x ~ uniform(0,2)\nlet y ~ x\nobserve y>1\nreport x");
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
}

#[test]
fn finite_choices_and_call_captures_preserve_conditioning() {
    let o = outcome("let x ~ uniform(0,2)\nfn keep() {observe x>1\n42}\nlet unused=keep()\nreport x");
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
    let o = outcome("let x ~ uniform(0,2)\nlet k ~ d2\nlet y=x+k\nobserve y>2\nreport x\nreport y");
    close(o.evidence.unwrap().to_f64(), 0.75);
    close(marginal(&o, 0).mean(), 7.0 / 6.0);
    close(marginal(&o, 1).mean(), 17.0 / 6.0);
}

#[test]
fn unbounded_truncation_uses_the_right_tail_moments() {
    let o = outcome("let x ~ exponential(2)\nobserve x>3\nreport x");
    close(marginal(&o, 0).mean(), 3.5);
    close(marginal(&o, 0).variance(), 0.25);
    let o = outcome("let x ~ gamma(2,1)\nobserve x>1\nreport x");
    close(marginal(&o, 0).mean(), 2.5);
    close(marginal(&o, 0).variance(), 1.75);
}

#[test]
fn dead_latent_restrictions_do_not_prevent_world_merging() {
    let o = outcome("repeat 40 {let x ~ uniform(0,2)\nif x>1 {let ignored=1}}\nreport true");
    close(o.reports[0].chance().unwrap(), 1.0);
    assert!(o.stats.peak_worlds < 5, "{:?}", o.stats);
    // A function's input is still live in its caller even if dead locally.
    let o = outcome(
        "fn side(x: float) {if x>1 {42} else {0}}\nlet x ~ uniform(0,2)\nlet tag=side(x)\nobserve tag==42\nreport x",
    );
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
}

#[test]
fn conditioning_does_not_rebuild_unrelated_collections() {
    let o = outcome("var xs=[]\nrepeat 100 {xs=[xs]}\nlet x ~ uniform(0,2)\nobserve x>1\nreport len(xs)\nreport x");
    assert_eq!(o.reports[0].distribution(), vec![(Value::Int(1.into()), 1.0)]);
    stats(&o, 1, 1.5, 1.0 / 12.0, 1.0, 2.0);
}
