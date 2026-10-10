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
        error("fn draw() {let x ~ uniform(0,2)\nx}\nlet x=draw()\nlet y=draw()\nreport x-y")
            .contains("different continuous draws")
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
        "report x*x*x",
        "report sin(x)",
        "report [x]==[x+1]",
        "report x in [x]",
        "report x by x",
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

#[test]
fn piecewise_math_is_exact() {
    let o = outcome(
        "let x ~ uniform(-1,1)\nreport abs(x)\nreport max(x,0)\nreport min(x,0.5)\nreport clamp(x,-0.5,0.5)\nreport abs(x)-x\nreport max(x,0)==0",
    );
    stats(&o, 0, 0.5, 1.0 / 12.0, 0.0, 1.0);
    close(marginal(&o, 0).quantile(0.05), 0.05);
    // Half the probability is at 0.
    stats(&o, 1, 0.25, 1.0 / 6.0 - 1.0 / 16.0, 0.0, 1.0);
    close(marginal(&o, 1).cdf(0.0), 0.5);
    stats(&o, 2, -0.0625, 0.25 - 0.0625 * 0.0625, -1.0, 0.5);
    assert_eq!(marginal(&o, 2).quantile(0.5), 0.0);
    stats(&o, 3, 0.0, 1.0 / 6.0, -0.5, 0.5);
    // The same draw: 0 above zero, and -2x below.
    stats(&o, 4, 0.5, 2.0 / 3.0 - 0.25, 0.0, 2.0);
    close(o.reports[5].chance().unwrap(), 0.5);
    assert!(
        o.output
            .contains("mean 0.25 · sd 0.32 · 5% 0.00 · median 0.00 · 95% 0.90"),
        "{}",
        o.output
    );

    // A half-normal, and the positive part of a normal.
    let o = outcome("let z ~ normal(0,1)\nreport abs(z)\nreport max(z,0)\nreport -min(-z,0)");
    let pi = std::f64::consts::PI;
    close(marginal(&o, 0).mean(), (2.0 / pi).sqrt());
    close(marginal(&o, 0).variance(), 1.0 - 2.0 / pi);
    close(marginal(&o, 1).mean(), 1.0 / (2.0 * pi).sqrt());
    close(marginal(&o, 1).variance(), 0.5 - 1.0 / (2.0 * pi));
    close(marginal(&o, 2).mean(), 1.0 / (2.0 * pi).sqrt());
}

#[test]
fn piecewise_outcomes_condition_their_draw() {
    // An event about |x| is an event about x.
    let o = outcome("let x ~ uniform(-1,1)\nlet y = abs(x)\nobserve y < 0.5\nreport x\nreport y");
    close(o.evidence.unwrap().to_f64(), 0.5);
    stats(&o, 0, 0.0, 1.0 / 12.0, -0.5, 0.5);
    stats(&o, 1, 0.25, 1.0 / 48.0, 0.0, 0.5);
    // An atom is evidence with its probability.
    let o = outcome("let x ~ uniform(-1,1)\nobserve max(x,0) == 0\nreport x");
    close(o.evidence.unwrap().to_f64(), 0.5);
    stats(&o, 0, -0.5, 1.0 / 12.0, -1.0, 0.0);
    // Once x > 0, |x| is x itself.
    let o = outcome("let x ~ uniform(-1,1)\nobserve x > 0\nreport abs(x) - x\nreport min(x, -1)");
    assert_eq!(o.reports[0].distribution(), vec![(Value::Float(0.0), 1.0)]);
    assert_eq!(o.reports[1].distribution(), vec![(Value::Float(-1.0), 1.0)]);
    // Branches agree with the piecewise form.
    let a = output("let x ~ normal(1,2)\nlet y = if x < 0 { -x } else { x }\nreport y");
    let b = output("let x ~ normal(1,2)\nlet y = abs(x)\nreport y");
    assert_eq!(a, b);
}

#[test]
fn piecewise_faults_and_limits_stay_as_they_were() {
    // Bounds out of order fault whatever the value is.
    let e = exec_raw("let x ~ uniform(0,2)\nreport clamp(x, 1, 0)", &Options::default()).unwrap_err();
    assert!(e.message.contains("lower bound is above"), "{}", e.message);
    assert!(e.fault.is_some());
    let o = outcome("let x ~ uniform(0,2)\nlet y = try { clamp(x, 1, 0) } catch DomainError { -1 }\nreport y");
    assert_eq!(o.reports[0].distribution(), vec![(Value::Int((-1).into()), 1.0)]);
    for (tail, what) in [
        ("report min(x, y)", "different continuous draws"),
        ("report abs(x) * x * x", "nonlinear arithmetic"),
        ("report clamp(x, 0, y)", "continuous bounds"),
        ("report x / max(x, 1)", "nonlinear arithmetic"),
    ] {
        let e = error(&format!("let x ~ uniform(0,2)\nlet y ~ uniform(0,2)\n{tail}"));
        assert!(e.contains(what) && e.contains("@mode sample"), "{tail}: {e}");
    }
}

#[test]
fn collections_carry_outcomes_of_a_draw() {
    let o = outcome(
        "let x ~ uniform(0,2)\nreport [x, 1].get(0)\nreport sum([x, x, 1])\nreport mean([x, x + 1])\nreport sum([x, -x])\nreport reverse([x, 1])[1]\nreport [\"a\": x].get(\"a\")\nreport minimum([x, 1])\nreport zip([x], [2])[0][0]",
    );
    stats(&o, 0, 1.0, 1.0 / 3.0, 0.0, 2.0);
    stats(&o, 1, 3.0, 4.0 / 3.0, 1.0, 5.0);
    stats(&o, 2, 1.5, 1.0 / 3.0, 0.5, 2.5);
    assert_eq!(o.reports[3].distribution(), vec![(Value::Float(0.0), 1.0)]);
    stats(&o, 4, 1.0, 1.0 / 3.0, 0.0, 2.0);
    stats(&o, 5, 1.0, 1.0 / 3.0, 0.0, 2.0);
    stats(&o, 6, 0.75, 2.0 / 3.0 - 0.75 * 0.75, 0.0, 1.0);
    stats(&o, 7, 1.0, 1.0 / 3.0, 0.0, 2.0);
    // A popped outcome is still the draw.
    let o = outcome("let x ~ uniform(0,2)\nvar xs = [1, x]\nlet top = xs.pop()\nobserve top > 1\nreport x");
    stats(&o, 0, 1.5, 1.0 / 12.0, 1.0, 2.0);
    for tail in [
        "report [\"a\": 1].get(x, 0)",
        "report sort([x, 1])",
        "report contains([x], 1)",
        "report sum([x, y])",
        "report mean(one_of([x, x + 1]))",
    ] {
        let e = error(&format!("let x ~ uniform(0,2)\nlet y ~ uniform(0,2)\n{tail}"));
        assert!(e.contains("@mode sample"), "{tail}: {e}");
    }
    // An index is an int, as when sampling.
    assert!(error("let x ~ uniform(0,2)\nreport [1, 2].get(x)").contains("needs an int"));
}

#[test]
fn an_event_key_splits_its_group() {
    let o = outcome("let x ~ uniform(0,2)\nreport x by x > 1\nreport x > 1.5 by x > 1");
    let group = |i: usize, key: bool| {
        let acc = &o.reports[i].groups[&Value::Bool(key)];
        (acc.total.to_f64(), analytic_mixture(&acc.distribution()))
    };
    let (w, m) = group(0, false);
    close(w, 0.5);
    let m = m.unwrap();
    close(m.mean(), 0.5);
    close(m.quantile(1.0), 1.0);
    let (w, m) = group(0, true);
    close(w, 0.5);
    close(m.unwrap().mean(), 1.5);
    close(o.reports[1].groups[&Value::Bool(true)].chance(), 0.5);
    close(o.reports[1].groups[&Value::Bool(false)].chance(), 0.0);
    // The same groups as a branch that names them.
    let rows = |src: &str| {
        let out = output(src);
        out.lines()
            .filter(|l| !l.contains("5%"))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        rows("let x ~ normal(1,2)\nreport x by x > 1.5"),
        rows("let x ~ normal(1,2)\nlet g = if x > 1.5 { true } else { false }\nreport x by g")
    );
    // Infinitely many groups still aren't a table.
    assert!(error("let x ~ uniform(0,2)\nreport true by x").contains("grouping by a continuous outcome"));
}

#[test]
fn atoms_at_an_unbounded_end() {
    use probl_engine::continuous::Family;
    // max(z, 0) is 0 with probability 1/2, then z: its quantiles above the
    // atom are z's own.
    let o = outcome("let z ~ normal(0,1)\nreport max(z,0)\nreport min(z,0)");
    let z = Family::normal(0.0, 1.0).unwrap();
    let m = marginal(&o, 0);
    assert_eq!(m.median(), 0.0);
    assert_eq!(m.quantile(0.75), z.quantile(0.75));
    close(m.mean(), 1.0 / (2.0 * std::f64::consts::PI).sqrt());
    let m = marginal(&o, 1);
    assert_eq!(m.median(), 0.0);
    assert_eq!(m.quantile(0.25), z.quantile(0.25));
}

#[test]
fn pieces_are_limited() {
    // The tent map doubles the pieces each round.
    let src = "let x ~ uniform(0,1)\nvar y = x\nrepeat 10 { y = abs(2 * y - 1) }\nreport y";
    let o = outcome(src);
    // It keeps x uniform: each fold is measure-preserving.
    let m = marginal(&o, 0);
    close(m.mean(), 0.5);
    close(m.cdf(0.3), 0.3);
    let e = exec_raw(&src.replace("10", "40"), &Options::default()).unwrap_err();
    assert!(e.message.contains("pieces is over the limit"), "{}", e.message);
}
