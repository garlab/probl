//! Continuous distributions and sampling (docs/semantics.md, sections 13 and
//! 14). The sampled checks use fixed seeds, and bounds of several standard
//! errors.

mod common;

use common::*;
use probl_engine::{Limits, Options};

/// The first report's probability and standard error.
fn estimate(src: &str) -> (f64, f64) {
    let out = outcome(src);
    let acc = out.reports[0].groups.values().next().expect("the report was reached");
    (acc.chance(), acc.chance_se())
}

/// The first report's mean and standard error.
fn sampled_mean(src: &str) -> (f64, f64) {
    let out = outcome(src);
    out.reports[0]
        .groups
        .values()
        .next()
        .expect("the report was reached")
        .mean_se()
}

#[track_caller]
fn within(estimate: (f64, f64), exact: f64, errors: f64) {
    let (x, se) = estimate;
    assert!(
        (x - exact).abs() <= errors * se.max(1e-12),
        "estimate {x} ± {se}, exact {exact}"
    );
}

#[test]
fn comparisons_with_numbers_are_exact_when_enumerating() {
    close(chance("report normal(0, 1) > 1.96"), 0.024997895148220435);
    close(chance("report 1.96 < normal(0, 1)"), 0.024997895148220435);
    close(chance("report uniform(0, 10) <= 2.5"), 0.25);
    close(chance("report normal(0, 1) == 0"), 0.0);
    // `a to b`: 5% and 95% quantiles at a and b.
    close(chance("report 3% to 7% < 3%"), 0.05);
    close(chance("report normal_range(-8%, 0%) > 0%"), 0.05);
    // Against a distribution of numbers: each outcome, weighted.
    close(chance("report uniform(0, 6) < d6"), 3.5 / 6.0);
}

#[test]
fn questions_use_the_formulas() {
    let value = |src: &str| distribution(src)[0].0.as_f64().unwrap();
    close(value("report mean(normal(3, 2))"), 3.0);
    close(value("report sd(uniform(0, 12))"), 12f64.sqrt());
    close(value("report median(3% to 7%)"), (0.03f64 * 0.07).sqrt());
    close(value("report cdf(exponential(2), 1)"), 1.0 - (-2f64).exp());
    close(
        value("report pdf(normal(0, 1), 0)"),
        1.0 / (2.0 * std::f64::consts::PI).sqrt(),
    );
    assert!((value("report quantile(normal_range(-8%, 0%), 0.05)") + 0.08).abs() < 1e-9);
    // A mixture of a distribution and a number.
    close(value("report mean(one_of([normal(0, 1), 10]))"), 5.0);
}

#[test]
fn continuous_draws_need_sampling() {
    let e = error("let x ~ normal(0, 1)\nreport x");
    assert!(
        e.contains("can't draw from a continuous distribution when enumerating"),
        "{e}"
    );
    assert!(e.contains("@mode sample"), "{e}");
    assert!(error("report normal(0, 1)").contains("can't report a continuous distribution"));
    assert!(error("observe 1 from normal(0, 1)\nreport true").contains("needs sample mode"));
    assert!(error("let x = if 50% { normal(0, 1) } else { 1 }\nlet y ~ x\nreport y").contains("continuous"));
    // Anything but comparing with a number needs a value.
    let e = error("report normal(0, 1) * 2 > 1");
    assert!(e.contains("needs a value, not a normal distribution"), "{e}");
    assert!(error("report round(normal(0, 1)) > 1").contains("needs a value"));
}

#[test]
fn estimates_need_positive_ends() {
    let e = error("report 0 to 5 > 1");
    assert!(e.contains("two positive numbers") && e.contains("normal_range"), "{e}");
    assert!(error("report 5 to 1 > 1").contains("below"));
    assert!(error("report normal(0, 0) > 1").contains("above 0"));
}

const CRAPS: &str = r#"
let come_out ~ 2d6
var win = false
if come_out in [7, 11] {
  win = true
} else if come_out not in [2, 3, 12] {
  loop {
    let r ~ 2d6
    if r == come_out { win = true; break }
    if r == 7 { break }
  }
}
report win
"#;

#[test]
fn sampling_estimates_the_same_model() {
    let src = format!("@mode sample(runs: 50_000, seed: 1){CRAPS}");
    within(estimate(&src), 244.0 / 495.0, 5.0);
    let text = output(&src);
    assert!(text.starts_with("sample · 50,000 runs · seed 1"), "{text}");
    assert!(text.contains("% ± 0.2%"), "{text}");
}

#[test]
fn a_seed_repeats_its_run() {
    let with = |seed: u64| output(&format!("@mode sample(runs: 2_000, seed: {seed}){CRAPS}"));
    assert_eq!(with(5), with(5));
    assert_ne!(with(5), with(6));
    // The engine's settings don't change which paths are sampled.
    let src = format!("@mode sample(runs: 2_000, seed: 5){CRAPS}");
    let plain = exec(&src, &Options::default()).unwrap().output;
    let other = exec(
        &src,
        &Options {
            merge: false,
            memoize: false,
            ..Options::default()
        },
    )
    .unwrap()
    .output;
    assert_eq!(plain, other);
}

#[test]
fn runs_are_counted_separately() {
    // Runs that end in the same state must not merge: weights 0.9 or 0.1,
    // equally likely, give an effective sample size of n · 0.25 / 0.41.
    let src = "@mode sample(runs: 100_000, seed: 2)\nlet x ~ d6\nobserve if x > 3 { 90% } else { 10% }\nreport x > 3";
    let out = outcome(src);
    let ess = out.sample.as_ref().unwrap().effective;
    assert!(
        (ess / 100_000.0 - 0.25 / 0.41).abs() < 0.01,
        "effective sample size {ess}"
    );
    within(estimate(src), 0.9, 5.0);
}

#[test]
fn simulate_is_enumerated_when_sampling() {
    let src = "@mode sample(runs: 1_000, seed: 3)\nlet p = P(simulate { let x ~ d6\nobserve x > 3\nx == 6 })\nreport p";
    let d = distribution(src);
    assert_eq!(d.len(), 1);
    close(d[0].0.as_f64().unwrap(), 1.0 / 3.0);
    // Sampling inside it isn't supported yet.
    let e = error("@mode sample(runs: 10, seed: 1)\nlet d = simulate { let x ~ normal(0, 1)\nx > 0 }\nreport d");
    assert!(e.contains("computed by enumeration"), "{e}");
}

#[test]
fn a_sampled_call_may_return_to_itself() {
    // Enumerating solves it by iteration, and sampling follows each run.
    let f = "fn tries() { if 50% { 1 } else { 1 + tries() } }\nreport tries()";
    assert!((mean(f) - 2.0).abs() < 1e-9);
    within(
        sampled_mean(&format!("@mode sample(runs: 20_000, seed: 4)\n{f}")),
        2.0,
        5.0,
    );
}

#[test]
fn densities_are_evidence_when_sampling() {
    // Normal prior, normal likelihood: the posterior mean of mu is 0.75.
    let src = "@mode sample(runs: 50_000, seed: 5)\nlet mu ~ normal(0, 1)\nobserve 1.5 from normal(mu, 1)\nreport mu";
    within(sampled_mean(src), 0.75, 5.0);
}

#[test]
fn a_choice_with_a_continuous_option_is_a_mixture() {
    let src = "@mode sample(runs: 20_000, seed: 6)\nlet x ~ if 50% { uniform(0, 1) } else { 5 }\nreport x == 5";
    within(estimate(src), 0.5, 5.0);
    let src = "@mode sample(runs: 20_000, seed: 6)\nlet x ~ one_of([uniform(0, 1), 5])\nreport x";
    within(sampled_mean(src), 2.75, 5.0);
}

#[test]
fn counts_are_drawn_directly() {
    // Sampling draws `binomial` and `poisson` without listing their outcomes,
    // and weighs evidence with their formulas.
    let src = "@mode sample(runs: 20_000, seed: 7)\nlet k ~ poisson(100.5)\nreport k";
    within(sampled_mean(src), 100.5, 5.0);
    let src = "@mode sample(runs: 20_000, seed: 7)\nlet p ~ one_of([10%, 50%])\nobserve 3 from binomial(10, p)\nreport p == 10%";
    // P(3 | 10%) = 0.0574, P(3 | 50%) = 0.1172.
    within(estimate(src), 0.057395628 / (0.057395628 + 0.1171875), 5.0);
}

#[test]
fn a_report_that_is_rarely_reached_says_so() {
    let text = output("@mode sample(runs: 10_000, seed: 8)\nif 10% { report true as \"rare\" }");
    assert!(text.contains("(reached in ") && text.contains("% of runs)"), "{text}");
}

#[test]
fn evidence_no_run_survives_is_an_error() {
    let e = error("@mode sample(runs: 100, seed: 1)\nobserve false\nreport true");
    assert!(e.contains("every run was ruled out"), "{e}");
}

/// Run a sampled program on `threads` threads: its output (or error) and
/// what it printed.
fn on_threads(src: &str, threads: usize, max_work: u64) -> (Result<String, String>, Vec<String>) {
    let (program, _) = probl_sema::compile(src);
    let program = program.expect("the program compiles");
    let options = Options {
        limits: Limits {
            max_threads: threads,
            max_work,
            ..Limits::default()
        },
        ..Options::default()
    };
    let mut lines = Vec::new();
    let result = probl_engine::run(&program, &options, &mut |line: &str| lines.push(line.to_string()));
    (result.map(|o| o.output).map_err(|e| e.message), lines)
}

#[test]
fn the_output_does_not_depend_on_threads() {
    // 7,500 runs: 8 batches, the last one partial.
    let src = r#"
@mode sample(runs: 7_500, seed: 9)
let x ~ d6
let y ~ normal(x, 1)
observe y > 2
if x == 6 and y > 7.5 { print("high", x, round(y)) }
report x
report y > 4 as "y above 4"
report y by x mod 2
"#;
    let (one, printed) = on_threads(src, 1, u64::MAX);
    let one = one.unwrap();
    assert!(printed.len() > 20, "{printed:?}");
    assert!(one.contains("effective sample size"), "{one}");
    for threads in [2, 3, 8] {
        let (other, other_printed) = on_threads(src, threads, u64::MAX);
        assert_eq!(other.unwrap(), one, "{threads} threads");
        assert_eq!(other_printed, printed, "{threads} threads");
    }
}

#[test]
fn the_first_error_in_run_order_is_reported() {
    // About one run in a thousand fails, with its own index in the message:
    // most batches fail, and the first one's error must be reported, after
    // what the batches up to it printed.
    let src = r#"
@mode sample(runs: 20_000, seed: 3)
let k ~ d100
let m ~ d100
if m == 1 { print("run with", k) }
let xs = [0, 1, 2]
if m == 1 and k > 90 { report xs[k] }
report k
"#;
    let (one, printed) = on_threads(src, 1, u64::MAX);
    let e = one.unwrap_err();
    assert!(e.contains("is out of range"), "{e}");
    assert!(!printed.is_empty());
    for threads in [2, 8] {
        let (other, other_printed) = on_threads(src, threads, u64::MAX);
        assert_eq!(other.unwrap_err(), e, "{threads} threads");
        assert_eq!(other_printed, printed, "{threads} threads");
    }
}

#[test]
fn threads_share_the_work_budget() {
    let src = format!("@mode sample(runs: 20_000, seed: 1){CRAPS}");
    let steps = outcome(&src).stats.world_steps;
    // Half the work the runs need: each of 8 threads would have enough on
    // its own, but together they don't.
    for threads in [1, 8] {
        let (result, _) = on_threads(&src, threads, steps / 2);
        let e = result.unwrap_err();
        assert!(e.contains("used up its work budget"), "{threads} threads: {e}");
    }
    let (plenty, _) = on_threads(&src, 8, steps * 4);
    assert!(plenty.is_ok());
}

#[test]
fn the_evidence_is_estimated() {
    // Probabilities: two dice, P(x > 4 and y = 6) = 1/18.
    let src = "@mode sample(runs: 50_000, seed: 5)\nlet x ~ d6\nobserve x > 4\nlet y ~ d6\nobserve y == 6\nreport x";
    let out = outcome(src);
    let info = out.sample.as_ref().unwrap();
    let z = out.evidence.unwrap().to_f64();
    within((z, z * info.evidence_se), 1.0 / 18.0, 5.0);
    assert!(!info.densities);
    assert!(
        out.output.starts_with("sample · 50,000 runs · seed 5 · evidence 5."),
        "{}",
        out.output
    );
    assert!(out.output.contains("% ± 0.1"), "{}", out.output);
    // Densities: a normal prior and a normal observation; the observation's
    // marginal is normal(0, √2), so the log evidence is -1.5²/4 - ln(4π)/2.
    let src = "@mode sample(runs: 50_000, seed: 5)\nlet mu ~ normal(0, 1)\nobserve 1.5 from normal(mu, 1)\nreport mu";
    let out = outcome(src);
    let info = out.sample.as_ref().unwrap();
    assert!(info.densities);
    let ln_z = out.evidence.unwrap().log10() * std::f64::consts::LN_10;
    within(
        (ln_z, info.evidence_se),
        -1.5f64.powi(2) / 4.0 - (4.0 * std::f64::consts::PI).ln() / 2.0,
        5.0,
    );
    assert!(out.output.contains("· log evidence -1.8"), "{}", out.output);
    // Tiny evidence is written in scientific notation, with a relative error.
    let src = "@mode sample(runs: 20_000, seed: 5)\nlet x ~ d6\nrepeat 20 { observe 10% }\nreport x";
    let out = output(src);
    assert!(out.contains("evidence 1.00e-20 (± 0.00%)"), "{out}");
    // No evidence without observations.
    let out = outcome("@mode sample(runs: 1_000, seed: 5)\nlet x ~ d6\nreport x");
    assert!(
        out.evidence.is_none() && !out.output.contains("evidence"),
        "{}",
        out.output
    );
}
