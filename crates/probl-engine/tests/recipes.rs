//! `simulate` with continuous draws inside: finite results of conditions on
//! them, distributions whose outcomes own their draws, local posteriors,
//! and structured reports of records of outcomes (docs/semantics.md,
//! sections 8, 9 and 13).
mod common;
use common::*;
use probl_engine::continuous::{Family, Mixture};
use probl_engine::report::analytic_mixture;
use probl_engine::value::Value;
use probl_engine::{Options, Outcome};
use probl_sema::ir::Mode;

fn marginal(out: &Outcome, i: usize) -> Mixture {
    analytic_mixture(&out.reports[i].distribution()).expect("a continuous report")
}

fn phi(z: f64) -> f64 {
    Family::normal(0.0, 1.0).unwrap().cdf(z)
}

const HALF: &str = "let d = simulate {\n let x ~ normal(0, 1)\n observe x > 0\n x\n}\n";

/// The mean and sd of a half-normal.
fn half_normal() -> (f64, f64) {
    let mean = (2.0 / std::f64::consts::PI).sqrt();
    (mean, (1.0 - mean * mean).sqrt())
}

#[test]
fn conditions_on_local_draws_are_finite_distributions() {
    let o = outcome("let d = simulate {\n let x ~ normal(0, 1)\n x > 0\n}\nreport d\nreport P(d)");
    close(o.reports[0].chance().unwrap(), 0.5);
    assert_eq!(o.reports[1].distribution(), vec![(Value::Prob(0.5), 1.0)]);
    // Conditions on the same draw are dependent; on others, independent.
    let d = distribution(
        "let d = simulate {\n let x ~ normal(0, 1)\n let y ~ normal(0, 1)\n [x > 0, x > 1, y > 0]\n}\nreport d",
    );
    let tail = 1.0 - phi(1.0);
    let expect = [
        ([false, false, false], 0.25),
        ([false, false, true], 0.25),
        ([true, false, false], (0.5 - tail) / 2.0),
        ([true, false, true], (0.5 - tail) / 2.0),
        ([true, true, false], tail / 2.0),
        ([true, true, true], tail / 2.0),
    ];
    assert_eq!(d.len(), expect.len());
    for ((v, p), (bools, q)) in d.iter().zip(expect) {
        assert_eq!(*v, Value::list(bools.iter().map(|b| Value::Bool(*b)).collect()));
        close(*p, q);
    }
    // Local evidence normalizes the result, and isn't the program's.
    let o = outcome("let d = simulate {\n let x ~ normal(0, 1)\n observe x > 1\n x > 2\n}\nreport d");
    close(o.reports[0].chance().unwrap(), (1.0 - phi(2.0)) / (1.0 - phi(1.0)));
    assert!(o.evidence.is_none_or(|e| e.to_f64() == 1.0));
    assert_eq!(o.stats.peak_worlds, 1);
}

#[test]
fn draws_from_a_recipe_are_fresh_and_its_fields_stay_together() {
    let src = "let d = simulate {\n let x ~ normal(0, 1)\n {x: x, twice: 2 * x}\n}\nlet r ~ d\nlet s ~ d\nreport r.twice - 2 * r.x\nreport r.x - s.x\nreport r.twice";
    let o = outcome(src);
    assert_eq!(o.reports[0].distribution(), vec![(Value::Float(0.0), 1.0)]);
    close(marginal(&o, 1).sd(), 2f64.sqrt());
    close(marginal(&o, 2).sd(), 2.0);
    // Sampling draws each outcome's draws once, together.
    let sampled = exec(
        &format!("@mode sample(runs: 2_000, seed: 2)\n{src}"),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(sampled.reports[0].distribution(), vec![(Value::Float(0.0), 1.0)]);
    // A draw inside a function, called twice, and in a nested `simulate`.
    let o = outcome(&format!(
        "{HALF}fn f(r) {{\n let v ~ r\n return v\n}}\nlet a = f(d)\nlet b = f(d)\nreport a > 1 and b > 1"
    ));
    let p = 2.0 * (1.0 - phi(1.0));
    close(o.reports[0].chance().unwrap(), p * p);
    let o = outcome(
        "let outer = simulate {\n let inner = simulate {\n  let x ~ normal(0, 1)\n  x\n }\n let a ~ inner\n let b ~ inner\n a + b\n}\nreport outer",
    );
    close(marginal(&o, 0).sd(), 2f64.sqrt());
}

#[test]
fn a_recipe_answers_queries_from_its_marginal() {
    let (mean, sd) = half_normal();
    let o = outcome(&format!(
        "{HALF}report mean(d)\nreport sd(d)\nreport median(d)\nreport quantile(d, 0.9)\nreport cdf(d, 1)\nreport pdf(d, 1)\nreport d > 1\nreport mean(d * 2)\nreport mean(one_of([d, -1]))\nreport typeof d"
    ));
    let number = |i: usize| o.reports[i].distribution()[0].0.as_f64().unwrap();
    close(number(0), mean);
    close(number(1), sd);
    assert!((number(2) - 0.6744897501960817).abs() < 1e-9);
    assert!((number(3) - 1.6448536269514722).abs() < 1e-9);
    close(number(4), 2.0 * phi(1.0) - 1.0);
    close(number(5), 2.0 * Family::normal(0.0, 1.0).unwrap().pdf(1.0));
    close(o.reports[6].chance().unwrap(), 2.0 * (1.0 - phi(1.0)));
    close(number(7), 2.0 * mean);
    close(number(8), (mean - 1.0) / 2.0);
    assert_eq!(
        o.reports[9].distribution(),
        vec![(Value::Str("dist[float]".into()), 1.0)]
    );
}

#[test]
fn a_simulate_can_observe_densities() {
    // A normal posterior, as a recipe: precision 1 + 4, mean 4 × 1.2 / 5.
    let o = outcome(
        "let posterior = simulate {\n let mu ~ normal(0, 1)\n observe 1.2 from normal(mu, 0.5)\n mu\n}\nlet a ~ posterior\nlet b ~ posterior\nreport a\nreport a - b",
    );
    let m = marginal(&o, 0);
    close(m.mean(), 0.96);
    close(m.sd(), 0.2f64.sqrt());
    close(marginal(&o, 1).sd(), 0.4f64.sqrt());
    // The program's own evidence isn't a density.
    assert!(!o.densities);
    assert!(o.output.starts_with("enumerated\n"), "{}", o.output);
    // A joint update inside, with the fields correlated.
    let o = outcome(
        "let both = simulate {\n let mu ~ normal(0, 1)\n let y ~ normal(mu, 1)\n observe 1.5 from normal(y, 0.5)\n {mu: mu, y: y}\n}\nlet r ~ both\nreport r.y - r.mu\nreport r.mu",
    );
    close(marginal(&o, 0).mean(), 1.5 / 2.25);
    close(marginal(&o, 1).sd(), (1.0 - 1.0 / 2.25f64).sqrt());
    // Each world's weight must have as many densities.
    let e = error(
        "let d = simulate {\n let k ~ one_of([0, 1])\n let x ~ normal(0, 1)\n if k == 1 { observe 0.5 from normal(x, 1) }\n x\n}\nreport d",
    );
    assert!(e.contains("observe different numbers of continuous values"), "{e}");
    let e = error(
        "let d = simulate {\n var n = 0\n while 50% { n += 1 }\n let mu ~ normal(n, 1)\n observe 1 from normal(mu, 1)\n mu\n}\nreport d",
    );
    assert!(e.contains("while some probability is unresolved"), "{e}");
    // A function inside it still can't.
    let e = error(
        "fn see(m) { observe 1 from normal(m, 1) }\nlet d = simulate {\n let mu ~ normal(0, 1)\n see(mu)\n mu\n}\nreport d",
    );
    assert!(e.contains("inside a function"), "{e}");
}

#[test]
fn records_and_lists_of_outcomes_report_each_field() {
    let o = outcome(
        "let x ~ normal(0, 1)\nlet k ~ one_of([\"a\", \"b\"])\nreport {x: x, twice: 2 * x, pos: x > 0, name: k}\nreport [x, x + 1] by k",
    );
    let group = &o.results[0].groups[0];
    let fields = &group.fields.as_ref().expect("fields").0;
    let paths: Vec<&str> = fields.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, [".name", ".pos", ".twice", ".x"]);
    assert!(group.numeric.is_none() && group.distribution.is_empty());
    let field = |i: usize| &fields[i].1;
    assert_eq!(field(0).distribution.len(), 2);
    close(field(1).fact.unwrap().point.unwrap(), 0.5);
    close(field(2).numeric.as_ref().unwrap().sd.point.unwrap(), 2.0);
    close(field(3).numeric.as_ref().unwrap().mean.point.unwrap(), 0.0);
    // With `by`, each key's fields.
    let keyed = &o.results[1].groups;
    assert_eq!(keyed.len(), 2);
    for g in keyed {
        let fields = &g.fields.as_ref().expect("fields").0;
        close(fields[1].1.numeric.as_ref().unwrap().mean.point.unwrap(), 1.0);
    }
    assert!(o.output.contains("(marginal of each field)"), "{}", o.output);
    // A recipe of records reports the same way.
    let o = outcome("let d = simulate {\n let z ~ normal(0, 1)\n {z: z, sq: z * z}\n}\nreport d");
    let fields = &o.results[0].groups[0].fields.as_ref().expect("fields").0;
    close(fields[0].1.numeric.as_ref().unwrap().mean.point.unwrap(), 1.0);
}

#[test]
fn reports_that_mix_records_of_outcomes_with_other_values_are_unsupported() {
    for (tail, what) in [
        ("report if x > 1 { {a: x} } else { 0 }", "together with other values"),
        ("report if x > 1 { {a: x} } else { {b: x} }", "different fields"),
        ("report if x > 1 { [x] } else { [x, x] }", "different fields"),
    ] {
        let e = error(&format!("let x ~ uniform(0, 2)\n{tail}"));
        assert!(e.contains(what), "{tail}: {e}");
    }
}

#[test]
fn what_recipes_dont_cover_is_unsupported_or_invalid() {
    for (tail, what) in [
        ("report support(d)", "outcomes can be listed"),
        ("report pmf(d, 1)", "outcomes can be listed"),
        ("report d + d", "another distribution of them"),
        ("let y ~ normal(0, 1)\nreport d + y", "an outcome of a draw"),
    ] {
        let e = error(&format!("{HALF}{tail}"));
        assert!(e.contains(what), "{tail}: {e}");
    }
    // A local model of an outer draw needs a contract for conditional models.
    let e = error("let x ~ uniform(0, 2)\nlet d = simulate { x + 1 }\nreport d");
    assert!(e.contains("capturing an analytic draw"), "{e}");
}

#[test]
fn sampling_agrees() {
    let src = "let d = simulate {\n let mu ~ normal(0, 1)\n observe 0.8 from normal(mu, 0.5)\n {mu: mu, pos: mu > 0}\n}\nlet r ~ d\nlet c = one_of([d, 0])\nreport r.mu\nreport r.pos";
    let exact = outcome(src);
    let sampled = exec(
        src,
        &Options {
            mode: Some(Mode::Sample { runs: 50_000, seed: 9 }),
            ..Options::default()
        },
    )
    .unwrap();
    let mean = |o: &Outcome| -> f64 {
        o.reports[0]
            .distribution()
            .iter()
            .map(|(v, p)| v.as_f64().unwrap() * p)
            .sum()
    };
    assert!((marginal(&exact, 0).mean() - mean(&sampled)).abs() < 0.02);
    assert!((exact.reports[1].chance().unwrap() - sampled.reports[1].chance().unwrap()).abs() < 0.02);
}
