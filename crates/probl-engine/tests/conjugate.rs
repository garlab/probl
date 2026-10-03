//! Exact updates for conjugate priors when sampling (docs/semantics.md,
//! section 14): which draws are delayed and what draws them, the errors,
//! the evidence, and the estimates against exact posteriors.

mod common;

use common::*;
use probl_engine::continuous::{Family, Rng};
use probl_engine::dist::Counts;
use probl_engine::{Limits, Options, Outcome, Updates};

fn without() -> Options {
    Options {
        conjugate: false,
        ..Options::default()
    }
}

/// What happened to the variable `name`, which must be one whose draws can
/// be delayed.
#[track_caller]
fn updates(out: &Outcome, name: &str) -> Updates {
    out.updates
        .iter()
        .find(|(v, _)| v.name == name)
        .map(|(_, u)| *u)
        .unwrap_or_else(|| panic!("`{name}` can't be delayed"))
}

fn counts(delayed: u64, exact: u64, drawn: u64) -> Updates {
    Updates { delayed, exact, drawn }
}

/// Each report's estimate and standard error: a probability for a fact,
/// the mean otherwise.
fn estimates(out: &Outcome) -> Vec<(f64, f64)> {
    out.reports
        .iter()
        .map(|sink| {
            let acc = sink.groups.values().next().expect("the report was reached");
            if acc.is_event() {
                (acc.chance(), acc.chance_se())
            } else {
                acc.mean_se()
            }
        })
        .collect()
}

#[test]
fn draws_observed_through_a_conjugate_form_are_delayed() {
    let out = outcome(
        "@mode sample(runs: 100, seed: 1)
let p ~ beta(2, 3)
let r ~ gamma(2, 1)
let m ~ normal(0, 1)
let q ~ beta(1, 1)
let s ~ 3 to 7
observe true from bernoulli(prob(p))
observe 3 from poisson(r)
observe 0.5 from normal(m, 1)
observe 2 from poisson(s)
report q > 0.5",
    );
    // `q` isn't observed, and `s`'s lognormal isn't a conjugate prior.
    let names: Vec<&str> = out.updates.iter().map(|(v, _)| v.name.as_str()).collect();
    assert_eq!(names, ["p", "r", "m", "s"]);
    for name in ["p", "r", "m"] {
        assert_eq!(updates(&out, name), counts(100, 100, 0), "{name}");
    }
    assert_eq!(updates(&out, "s"), counts(0, 0, 0));
    // Nothing is delayed when enumerating, or when turned off.
    assert_eq!(
        updates(
            &exec(
                "@mode sample(runs: 100, seed: 1)\nlet p ~ beta(2, 3)\nobserve true from bernoulli(prob(p))",
                &without()
            )
            .unwrap(),
            "p"
        ),
        counts(0, 0, 0)
    );
    assert!(
        outcome("let p ~ bernoulli(30%)\nobserve p\nreport p")
            .updates
            .is_empty()
    );
}

#[test]
fn every_other_use_draws_the_variable_first() {
    let uses = [
        "let c = p",
        "report p",
        "report p > 0.5",
        "if p > 0.5 { print(1) }",
        "print(p)",
        "let e = \"{p}\"",
        "let xs = [p, 0.5]",
        "let f = x -> x * p",
        "fn g() -> float { return p }\nlet y = g()",
        "let d = simulate { p * 2 }",
        "observe 2 from binomial(10, prob(p * 1))",
        "observe p > 0.2",
        "observe true from bernoulli(prob(p * 1))",
        "let c = chance { prob(p) => 1, else => 2 }",
    ];
    for u in uses {
        // Drawn once, and not updated after: it stays drawn. (No
        // observation may follow a report.)
        let after = if u.starts_with("report") {
            ""
        } else {
            "observe false from bernoulli(prob(p))"
        };
        let src = format!(
            "@mode sample(runs: 50, seed: 1)
let p ~ beta(2, 3)
observe true from bernoulli(prob(p))
{u}
{after}"
        );
        let out = exec(&src, &Options::default()).unwrap_or_else(|e| panic!("{u}:\n{e}"));
        assert_eq!(updates(&out, "p"), counts(50, 50, 50), "{u}");
    }
}

#[test]
fn an_observation_that_isnt_conjugate_draws_the_variable() {
    let out = outcome(
        "@mode sample(runs: 50, seed: 1)
let m ~ normal(0.5, 0.1)
observe 1.0 from normal(m, 1)
observe 3 from binomial(10, prob(m))
observe 1.0 from normal(m, 1)",
    );
    assert_eq!(updates(&out, "m"), counts(50, 50, 50));
    // The variable in another part of the observation.
    let out = outcome(
        "@mode sample(runs: 50, seed: 1)
let p ~ beta(2, 3)
observe true from bernoulli(prob(p))
observe round(p * 10) from binomial(10, prob(p))",
    );
    assert_eq!(updates(&out, "p"), counts(50, 50, 50));
}

#[test]
fn observations_in_a_loop_update_until_the_variable_is_used() {
    let out = outcome(
        "@mode sample(runs: 50, seed: 1)
let p ~ beta(2, 3)
for k in [3, 4, 5, 6] {
  observe k from binomial(10, prob(p))
}
for k in [3, 4, 5] {
  observe k from binomial(10, prob(p))
  let c = p
}",
    );
    assert_eq!(updates(&out, "p"), counts(50, 250, 50));
    // A variable drawn again in a loop is delayed again each time.
    let out = outcome(
        "@mode sample(runs: 50, seed: 1)
repeat 3 {
  let p ~ beta(2, 3)
  observe true from bernoulli(prob(p))
}",
    );
    assert_eq!(updates(&out, "p"), counts(150, 150, 0));
}

#[test]
fn type_annotations_preserve_or_convert_the_draw() {
    let out =
        outcome("@mode sample(runs: 50, seed: 1)\nlet p: float ~ beta(2, 3)\nobserve true from bernoulli(prob(p))");
    assert_eq!(updates(&out, "p"), counts(50, 50, 0));
    let out = outcome("@mode sample(runs: 50, seed: 1)\nlet r: float ~ gamma(2, 1)\nobserve 3 from poisson(r)");
    assert_eq!(updates(&out, "r"), counts(50, 50, 0));
    // A probability annotation materializes the draw and checks its value.
    // This is equivalent with exact updating enabled or disabled.
    for family in ["beta(2, 3)", "gamma(2, 0.001)"] {
        let source = format!(
            "@mode sample(runs: 50, seed: 1)\nlet p: prob ~ {family}\nobserve true from bernoulli(p)\nreport p"
        );
        let out = outcome(&source);
        assert_eq!(out.output, exec(&source, &without()).unwrap().output);
        assert!(
            out.reports[0]
                .distribution()
                .iter()
                .all(|(v, _)| matches!(v, probl_engine::value::Value::Prob(_)))
        );
        assert!(out.stats.updates.iter().all(|u| u.exact == 0));
    }
    let src = "@mode sample(runs: 50, seed: 1)\nlet r: prob ~ gamma(2, 1)\nobserve 3 from poisson(r)";
    let error = exec(src, &Options::default()).unwrap_err();
    assert!(error.contains("finite number between 0 and 1"), "{error}");
    assert_eq!(error, exec(src, &without()).unwrap_err());
}

#[test]
fn parameters_are_fixed_at_the_draw() {
    let out = outcome(
        "@mode sample(runs: 4000, seed: 1)
var alpha = 2.0
let p ~ beta(alpha, 3)
alpha = 100.0
observe true from bernoulli(prob(p))
report p",
    );
    // The posterior is beta(3, 3), whatever `alpha` became.
    let (mean, se) = estimates(&out)[0];
    assert!((mean - 0.5).abs() < 5.0 * se, "{mean} ± {se}");
}

#[test]
fn invalid_parameters_and_observations_give_the_same_errors() {
    let cases = [
        "let p ~ beta(0, 1)\nobserve true from bernoulli(prob(p))",
        "let p ~ beta(1, 1)\nobserve 3 from binomial(-1, prob(p))",
        "let p ~ beta(1, 1)\nobserve 3 from binomial(2.5, prob(p))",
        "let p ~ beta(1, 1)\nobserve 3 from binomial(\"ten\", prob(p))",
        "let m ~ normal(0, 1)\nobserve 1.0 from normal(m, 0)",
        "let m ~ normal(0, 1)\nobserve 1.0 from normal(m, -2)",
        "let m ~ normal(0, 1)\nobserve true from normal(m, 1)",
        "let m ~ normal(0, 1)\nobserve \"x\" from normal(m, 1)",
        "let p: int ~ beta(1, 1)\nobserve true from bernoulli(prob(p))",
        "let p ~ beta(1, 1)\nobserve 11 from binomial(10, prob(p))",
        "let p ~ beta(1, 1)\nobserve 2.5 from binomial(10, prob(p))",
        "let r ~ gamma(2, 1)\nobserve -1 from poisson(r)",
        "let r ~ gamma(2, 1)\nobserve true from poisson(r)",
    ];
    for c in cases {
        let src = format!("@mode sample(runs: 100, seed: 3)\n{c}\n");
        let exact = exec(&src, &Options::default()).expect_err(c);
        assert_eq!(exact, exec(&src, &without()).expect_err(c), "{c}");
    }
}

#[test]
fn the_evidence_is_exact_when_every_observation_updates_exactly() {
    let src = "@mode sample(runs: 2000, seed: 1)
let a ~ beta(2, 50)
let b ~ beta(2, 50)
let visitors = 400
for k in [12, 9, 15, 11, 10, 13, 8, 12, 14, 10, 11, 9, 13, 12, 10,
          11, 14, 9, 10, 12, 13, 11, 10, 12, 9, 11, 13, 10, 12, 11] {
  observe k from binomial(visitors, prob(a))
}
for k in [14, 12, 16, 13, 11, 15, 12, 14, 13, 16, 12, 11, 15, 14, 13,
          12, 15, 13, 14, 12, 16, 13, 12, 14, 15, 13, 12, 14, 13, 15] {
  observe k from binomial(visitors, prob(b))
}
report b > a";
    let out = outcome(src);
    let sampled = out.sample.as_ref().unwrap();
    assert!((sampled.effective - 2000.0).abs() < 1e-6, "{}", sampled.effective);
    assert!(sampled.evidence_se < 1e-6);
    // log₁₀ of the probability of the data, from mpmath.
    let exact = -61.376_285_015_138_59;
    assert!((out.evidence.unwrap().log10() - exact).abs() < 1e-10);
    // Estimated without exact updates.
    let plain = exec(src, &without()).unwrap();
    let se = plain.sample.as_ref().unwrap().evidence_se;
    assert!(plain.sample.as_ref().unwrap().effective < 200.0);
    let ratio = plain.evidence.unwrap().ratio(out.evidence.unwrap());
    assert!((ratio - 1.0).abs() < 5.0 * se, "{ratio} ± {se}");
}

#[test]
fn long_sequences_and_densities_keep_their_evidence() {
    // 20,000 observations: e^−12734.6, far below an f64.
    let out = outcome(
        "@mode sample(runs: 20, seed: 1)
let p ~ beta(1, 1)
for i in 1..20000 {
  observe i mod 3 == 0 from bernoulli(prob(p))
}",
    );
    let ln = out.evidence.unwrap().log10() * std::f64::consts::LN_10;
    assert!((ln - -12_734.606_122_134_105).abs() < 1e-8, "{ln}");
    // Densities: the joint density of the four values, from mpmath.
    let out = outcome(
        "@mode sample(runs: 20, seed: 1)
let m ~ normal(0, 2)
for y in [1.3, 0.4, -0.2, 2.1] {
  observe y from normal(m, 1)
}",
    );
    let sampled = out.sample.as_ref().unwrap();
    assert!(sampled.densities);
    let ln = out.evidence.unwrap().log10() * std::f64::consts::LN_10;
    assert!((ln - -6.717_654_922_493_858).abs() < 1e-12, "{ln}");
    assert!(
        out.output
            .starts_with("sample · 20 runs · seed 1 · log evidence -6.72 ± 0.00"),
        "{}",
        out.output
    );
    // A density above 1, so a positive log.
    let out = outcome("@mode sample(runs: 20, seed: 1)\nlet m ~ normal(0, 0.001)\nobserve 0 from normal(m, 0.001)");
    let ln = out.evidence.unwrap().log10() * std::f64::consts::LN_10;
    assert!((ln - 5.642_243_155_497_492).abs() < 1e-12, "{ln}");
}

#[test]
fn rare_but_possible_data_keeps_its_runs() {
    let out = outcome(
        "@mode sample(runs: 100, seed: 1)
let p ~ beta(1000, 1000)
observe 0 from binomial(100_000, prob(p))
report p",
    );
    let ln = out.evidence.unwrap().log10() * std::f64::consts::LN_10;
    assert!((ln - -4_234.102_082_009_189).abs() < 1e-9, "{ln}");
    // The posterior is beta(1000, 101_000).
    let (mean, se) = estimates(&out)[0];
    assert!((mean - 1000.0 / 102_000.0).abs() < 5.0 * se, "{mean} ± {se}");
    // Drawing `p` from its prior, no run explains the data.
    let error = exec(
        "@mode sample(runs: 100, seed: 1)\nlet p ~ beta(1000, 1000)\nobserve 0 from binomial(100_000, prob(p))",
        &without(),
    )
    .unwrap_err();
    assert!(error.contains("every run was ruled out"), "{error}");
}

/// A model with one variable of each family, and its exact posteriors:
/// `p` is beta(7, 9), `r` gamma(10, 0.375), and `m` normal(0.7556, 2/3).
const MODEL: &str = "let p ~ beta(2, 3)
let r ~ gamma(2, 1.5)
let m ~ normal(0, 2)
observe 4 from binomial(10, prob(p))
observe true from bernoulli(prob(p))
observe 3 from poisson(r)
observe 5 from poisson(r)
observe 1.3 from normal(m, 1)
observe 0.4 from normal(m, 1)
report p > 0.5
report r > 3
report m > 1
report p
report r
report m";

fn exact_answers() -> Vec<f64> {
    let p = Family::beta(7.0, 9.0).unwrap();
    let r = Family::gamma(10.0, 0.375).unwrap();
    let m = Family::normal(1.7 / 2.25, (1.0f64 / 2.25).sqrt()).unwrap();
    vec![
        1.0 - p.cdf(0.5),
        1.0 - r.cdf(3.0),
        1.0 - m.cdf(1.0),
        p.mean(),
        r.mean(),
        m.mean(),
    ]
}

#[test]
fn the_estimates_agree_with_drawing_from_the_prior() {
    let src = format!("@mode sample(runs: 40_000, seed: 5)\n{MODEL}");
    let exact = estimates(&outcome(&src));
    let plain = estimates(&exec(&src, &without()).unwrap());
    for (i, ((x, sx), (y, sy))) in exact.iter().zip(&plain).enumerate() {
        let se = (sx * sx + sy * sy).sqrt();
        assert!((x - y).abs() < 5.0 * se, "report {i}: {x} ± {sx} against {y} ± {sy}");
    }
    for (i, ((x, se), truth)) in exact.iter().zip(exact_answers()).enumerate() {
        assert!((x - truth).abs() < 5.0 * se, "report {i}: {x} ± {se} against {truth}");
    }
}

/// With every run weighted the same, the reports still have Monte Carlo
/// error, and their standard errors must describe it: over many seeds, the
/// errors against the exact posteriors, in standard errors, have a mean
/// square near 1.
#[test]
fn the_reports_standard_errors_are_calibrated() {
    let truths = exact_answers();
    let (mut sum, mut count) = (0.0, 0);
    for seed in 0..300 {
        let out = outcome(&format!("@mode sample(runs: 2000, seed: {seed})\n{MODEL}"));
        assert!((out.sample.as_ref().unwrap().effective - 2000.0).abs() < 1e-6);
        for ((x, se), truth) in estimates(&out).into_iter().zip(&truths) {
            let z = (x - truth) / se;
            sum += z * z;
            count += 1;
        }
    }
    let mean = sum / count as f64;
    assert!((0.85..1.15).contains(&mean), "mean z² {mean} over {count} estimates");
}

/// Simulation-based calibration: for parameters drawn from the prior and
/// data drawn given them, the posterior probability that the parameter is
/// at most its true value is uniform, if the posterior is right.
#[test]
fn simulation_based_calibration() {
    let datasets = 300;
    let prior_and_data = |kind: usize, rng: &mut Rng| -> (f64, String) {
        match kind {
            0 => {
                let p = Family::beta(2.0, 3.0).unwrap().sample(rng);
                let ks: Vec<String> = (0..5)
                    .map(|_| Counts::Binomial { n: 10, p }.sample(rng).to_string())
                    .collect();
                let body = format!(
                    "let x ~ beta(2, 3)\nfor k in [{}] {{\n  observe k from binomial(10, prob(x))\n}}",
                    ks.join(", ")
                );
                (p, body)
            }
            1 => {
                let p = Family::beta(1.0, 1.0).unwrap().sample(rng);
                let facts: Vec<&str> = (0..8)
                    .map(|_| if rng.uniform() < p { "true" } else { "false" })
                    .collect();
                let body = format!(
                    "let x ~ beta(1, 1)\nfor v in [{}] {{\n  observe v from bernoulli(prob(x))\n}}",
                    facts.join(", ")
                );
                (p, body)
            }
            2 => {
                let rate = Family::gamma(2.0, 1.5).unwrap().sample(rng);
                let ks: Vec<String> = (0..4)
                    .map(|_| Counts::Poisson { rate }.sample(rng).to_string())
                    .collect();
                let body = format!(
                    "let x ~ gamma(2, 1.5)\nfor k in [{}] {{\n  observe k from poisson(x)\n}}",
                    ks.join(", ")
                );
                (rate, body)
            }
            _ => {
                let mean = Family::normal(0.0, 2.0).unwrap().sample(rng);
                let ys: Vec<String> = (0..3)
                    .map(|_| format!("{:.17}", Family::normal(mean, 1.0).unwrap().sample(rng)))
                    .collect();
                let body = format!(
                    "let x ~ normal(0, 2)\nfor y in [{}] {{\n  observe y from normal(x, 1)\n}}",
                    ys.join(", ")
                );
                (mean, body)
            }
        }
    };
    for kind in 0..4 {
        let mut rng = Rng::new(1000 + kind as u64);
        let mut ranks: Vec<f64> = (0..datasets)
            .map(|i| {
                let (truth, body) = prior_and_data(kind, &mut rng);
                let src = format!("@mode sample(runs: 1000, seed: {i})\n{body}\nreport x <= {truth:.17}");
                let out = exec(&src, &Options::default()).unwrap_or_else(|e| panic!("{src}\n{e}"));
                estimates(&out)[0].0
            })
            .collect();
        ranks.sort_by(f64::total_cmp);
        // Kolmogorov–Smirnov against the uniform distribution; the 0.001
        // critical value is 1.95 / √n.
        let n = ranks.len() as f64;
        let d = ranks
            .iter()
            .enumerate()
            .map(|(i, &u)| (u - i as f64 / n).abs().max((u - (i + 1) as f64 / n).abs()))
            .fold(0.0, f64::max);
        assert!(d < 1.95 / n.sqrt(), "family {kind}: D = {d}");
    }
}

#[test]
fn the_output_does_not_depend_on_threads() {
    let src = format!("@mode sample(runs: 20_000, seed: 9)\n{MODEL}");
    let with = |threads: usize| {
        let options = Options {
            limits: Limits {
                max_threads: threads,
                ..Limits::default()
            },
            ..Options::default()
        };
        exec(&src, &options).unwrap().output
    };
    let one = with(1);
    assert_eq!(one, with(3));
    assert_eq!(one, with(8));
}

#[test]
fn programs_without_conjugate_observations_are_unchanged() {
    for src in [
        "@mode sample(runs: 2000, seed: 4)\nlet x ~ normal(0, 1)\nobserve x > 0\nreport x",
        "@mode sample(runs: 2000, seed: 4)\nlet s ~ 3 to 7\nobserve 2 from poisson(s)\nreport s",
        "@mode sample(runs: 2000, seed: 4)\nlet p ~ beta(2, 3)\nlet q = p\nobserve true from bernoulli(prob(q))\nreport p",
    ] {
        assert_eq!(output(src), exec(src, &without()).unwrap().output, "{src}");
    }
}

/// A random program that mixes exact updates with every way of using the
/// variables: other observations, reads, branches, loops, calls, lambdas,
/// `simulate`, assignments and draws again.
fn random_program(rng: &mut Rng, seed: u64) -> String {
    let mut pick = |n: usize| (rng.uniform() * n as f64) as usize;
    let mut lines = vec![format!("@mode sample(runs: 2000, seed: {seed})")];
    lines.push(["var p ~ beta(2, 3)", "var p: float ~ beta(2, 3)"][pick(2)].to_string());
    lines.push(["var r ~ gamma(2, 1.5)", "var r: float ~ gamma(2, 1.5)"][pick(2)].to_string());
    lines.push("var m ~ normal(0, 2)".to_string());
    lines.push("fn seen(k: int) {\n  observe k from binomial(10, prob(p))\n}".to_string());
    for i in 0..4 + pick(10) {
        let fact = ["true", "false"][pick(2)];
        let (k10, k6, k5) = (pick(11), pick(7), pick(6));
        let y = pick(41) as f64 / 10.0 - 2.0;
        lines.push(match pick(22) {
            0 => format!("observe {k10} from binomial(10, prob(p))"),
            1 => format!("observe {fact} from bernoulli(prob(p))"),
            2 => "observe true from bernoulli(prob(p))".to_string(),
            3 => format!("observe {k6} from poisson(r)"),
            4 => format!("observe {y} from normal(m, {})", ["0.5", "1", "2"][pick(3)]),
            5 => format!("observe {k10} from binomial(10, prob(p * 0.9))"),
            6 => "observe p > 0.2".to_string(),
            7 => format!("let c{i} = p + r + m"),
            8 => format!(
                "if m > 0 {{\n  observe {k6} from poisson(r)\n}} else {{\n  observe {fact} from bernoulli(prob(p))\n}}"
            ),
            9 => format!("repeat 2 {{\n  observe {k5} from binomial(5, prob(p))\n}}"),
            10 => format!("for k in [{k6}, {}] {{\n  observe k from poisson(r)\n}}", pick(7)),
            11 => format!("seen({k10})"),
            12 => format!("let f{i} = x -> x * m"),
            13 => format!("let s{i} = simulate {{ r * 2 }}"),
            14 => "p ~ beta(1, 1)".to_string(),
            15 => "r = 1.5".to_string(),
            16 => "print(p)".to_string(),
            17 => format!("let xs{i} = [p, r]"),
            18 => format!("m ~ normal({y}, 1)"),
            19 => format!("if p > 0.5 {{\n  let t{i} = r\n}}"),
            20 => format!(
                "for y in [{y}, {}] {{\n  observe y from normal(m, 1)\n}}",
                pick(21) as f64 / 10.0 - 1.0
            ),
            _ => format!("let c{i} = chance {{ p => 1, else => 0 }}"),
        });
    }
    lines.extend(["report p", "report r", "report m"].map(String::from));
    lines.join("\n")
}

/// Random programs estimate the same posteriors with exact updates as
/// without, and never read a delayed variable. `PROBL_CONJUGATE_CASES` runs
/// more of them.
///
/// Estimates are compared only when both ways have enough effective runs
/// for their standard errors to be trusted. Exact updates can have fewer:
/// a variable drawn between its observations comes from its distribution
/// after the first ones, and if the later ones disagree with them, they
/// weight that draw worse than one from the prior (semantics §14).
#[test]
fn random_programs_agree_with_drawing_from_the_prior() {
    let cases: u64 = std::env::var("PROBL_CONJUGATE_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(200);
    let mut rng = Rng::new(77);
    let (mut sum, mut compared, mut worst) = (0.0, 0, (0.0f64, String::new()));
    let (mut programs, mut fewer) = (0, 0);
    for case in 0..cases {
        let src = random_program(&mut rng, case);
        let exact = exec_raw(&src, &Options::default());
        let plain = exec_raw(&src, &without());
        for result in [&exact, &plain] {
            if let Err(e) = result {
                assert_ne!(e.kind, probl_engine::ErrorKind::Internal, "{src}\n{e:?}");
            }
        }
        let (Ok(exact), Ok(plain)) = (exact, plain) else {
            continue;
        };
        let (with, without) = (
            exact.sample.as_ref().unwrap().effective,
            plain.sample.as_ref().unwrap().effective,
        );
        programs += 1;
        fewer += (with < 0.9 * without) as u64;
        if with.min(without) < 300.0 {
            continue;
        }
        for ((x, sx), (y, sy)) in estimates(&exact).into_iter().zip(estimates(&plain)) {
            let z = (x - y) / (sx * sx + sy * sy).sqrt().max(1e-9);
            sum += z.abs().min(4.0).powi(2);
            compared += 1;
            if z.abs() > worst.0 {
                worst = (z.abs(), src.clone());
            }
        }
    }
    let mean = sum / compared as f64;
    eprintln!(
        "{programs} programs ran, {fewer} with fewer effective runs with exact updates; \
         {compared} estimates compared, mean z² {mean:.2}, worst |z| {:.1}",
        worst.0
    );
    assert!(compared >= 150, "only {compared} estimates compared");
    assert!(mean < 1.5, "mean z² {mean} over {compared} estimates");
    assert!(worst.0 < 6.0, "|z| = {} for:\n{}", worst.0, worst.1);
}
