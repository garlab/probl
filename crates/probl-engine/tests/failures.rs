//! Failure modes (docs/semantics.md, section 11): in total mode a fault
//! stops the run; in partial mode it ends only the world it happens in, and
//! the outcome says how much failed. Total is the default when enumerating,
//! partial when sampling.

mod common;

use common::exec_partial;
use probl_engine::{ErrorKind, FailureMode, Limits, Options, Outcome, RuntimeError};
use probl_sema::ir::Mode;

const RECIPROCAL: &str = "let x ~ d6 - 1
let y = 1 / x
report y";

fn partial() -> Options {
    Options {
        on_error: Some(FailureMode::Partial),
        ..Options::default()
    }
}

fn sampled(runs: u64, seed: u64) -> Options {
    Options {
        mode: Some(Mode::Sample { runs, seed }),
        ..Options::default()
    }
}

fn run(src: &str, options: &Options) -> Outcome {
    exec_partial(src, options).unwrap_or_else(|e| panic!("{src}\n{e:?}"))
}

fn fails(src: &str, options: &Options) -> RuntimeError {
    exec_partial(src, options).expect_err(src)
}

#[track_caller]
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
        "expected {expected}, got {actual}"
    );
}

/// The weight that failed, and the weight that finished.
fn weights(out: &Outcome) -> (f64, f64) {
    (out.failures.weight.to_f64(), out.finished.to_f64())
}

fn reach(out: &Outcome, report: usize) -> Option<f64> {
    out.results[report].reach.map(|r| r.share)
}

#[test]
fn enumeration_is_total_and_sampling_is_partial_by_default() {
    let e = fails(RECIPROCAL, &Options::default());
    assert_eq!(e.message, "division by zero");

    let out = run(RECIPROCAL, &sampled(1000, 0));
    assert_eq!(out.on_error, FailureMode::Partial);
    assert!(
        out.failures.runs > 100 && out.failures.runs < 250,
        "{}",
        out.failures.runs
    );
    assert!(out.output.contains("partial result"), "{}", out.output);

    // `--mode sample` on a program that enumerates: the mode used decides.
    let src = format!("@mode enumerate\n{RECIPROCAL}");
    assert_eq!(run(&src, &sampled(100, 1)).on_error, FailureMode::Partial);
    let src = format!("@mode sample(runs: 100)\n{RECIPROCAL}");
    let enumerated = Options {
        mode: Some(Mode::Enumerate),
        ..Options::default()
    };
    assert!(fails(&src, &enumerated).message.contains("division by zero"));
}

#[test]
fn the_pragma_and_the_options_choose_the_mode() {
    let out = run(&format!("@on_error partial\n{RECIPROCAL}"), &Options::default());
    assert_eq!(out.on_error, FailureMode::Partial);
    assert!(!out.failures.is_empty());

    let src = format!("@on_error total\n{RECIPROCAL}");
    fails(&src, &sampled(1000, 0));
    // A host's choice wins over the program's, either way.
    assert_eq!(run(&src, &partial()).on_error, FailureMode::Partial);
    let total = Options {
        on_error: Some(FailureMode::Total),
        ..sampled(1000, 0)
    };
    fails(&format!("@on_error partial\n{RECIPROCAL}"), &total);
    // No fault: the mode is reported, and nothing else changes.
    let out = run("report d6 > 4", &partial());
    assert_eq!(out.on_error, FailureMode::Partial);
    assert!(out.failures.is_empty());
    assert_eq!(out.output, run("report d6 > 4", &Options::default()).output);
}

#[test]
fn a_failed_world_ends_and_the_others_finish() {
    let out = run(RECIPROCAL, &partial());
    let (failed, finished) = weights(&out);
    close(failed, 1.0 / 6.0);
    close(finished, 5.0 / 6.0);
    // The report describes the worlds that reached it, and says how many.
    let mean: f64 = out.reports[0]
        .distribution()
        .iter()
        .map(|(v, p)| v.as_f64().unwrap() * p)
        .sum();
    close(mean, 137.0 / 300.0);
    close(reach(&out, 0).unwrap(), 5.0 / 6.0);
    assert!(
        out.output.starts_with("enumerated · partial result · 16.67% failed"),
        "{}",
        out.output
    );
    let [failure] = &out.failures.groups[..] else {
        panic!("{:?}", out.failures.groups);
    };
    assert_eq!(failure.error.message, "division by zero");
    assert_eq!(failure.error.kind, ErrorKind::Language);
}

#[test]
fn reports_before_a_failure_keep_the_worlds_that_failed() {
    let src = "let x ~ d6 - 1
report x == 0 as \"zero\"
let y = 1 / x
report y";
    let out = run(src, &partial());
    close(out.reports[0].chance().unwrap(), 1.0 / 6.0);
    close(reach(&out, 0).unwrap(), 1.0);
    close(reach(&out, 1).unwrap(), 5.0 / 6.0);

    // The same when sampling, within the sampling error.
    let out = run(src, &sampled(20_000, 4));
    let zero = out.reports[0].chance().unwrap();
    assert!((zero - 1.0 / 6.0).abs() < 0.02, "{zero}");
    let later = reach(&out, 1).unwrap();
    assert!((later - 5.0 / 6.0).abs() < 0.02, "{later}");
    // Every requested run is accounted for: none is drawn again.
    assert_eq!(out.failures.runs + out.finished.to_f64() as u64, 20_000);
}

#[test]
fn faults_inside_calls_fail_their_callers_worlds() {
    // The call branches: one of its six worlds fails, in every caller.
    let src = "fn f(scale) {
  let y ~ d6 - 1
  return scale / y
}
let a ~ bernoulli(50%)
let v = if a { f(1) } else { 0 }
report v";
    for (merge, memoize) in [(true, true), (false, true), (true, false), (false, false)] {
        let options = Options {
            merge,
            memoize,
            ..partial()
        };
        let out = run(src, &options);
        let (failed, finished) = weights(&out);
        close(failed, 1.0 / 12.0);
        close(finished, 11.0 / 12.0);
        let note = &out.failures.groups[0].error.notes;
        assert!(note.contains(&"in a call to `f`".to_string()), "{note:?}");
    }

    // When sampling, the run that made the call fails.
    let out = run(
        "fn inv(v) { return 1 / v }\nlet x ~ d2 - 1\nreport inv(x)",
        &sampled(1000, 2),
    );
    let failure = &out.failures.groups[0];
    assert_eq!(failure.runs, out.failures.runs);
    assert!(failure.first_run.is_some());
    assert_eq!(out.failures.runs + out.finished.to_f64() as u64, 1000);
}

#[test]
fn solved_loops_and_recursion_count_what_fails() {
    // Each round fails with 1/6 and leaves with 1/6: half the weight fails.
    let src = "var tries = 0
loop {
  let r ~ d6
  if r == 1 { tries = 1 / 0 }
  if r == 6 { break }
}
report tries";
    for solve in [true, false] {
        let options = Options {
            solve,
            epsilon: Some(1e-15),
            ..partial()
        };
        let out = run(src, &options);
        let (failed, finished) = weights(&out);
        assert!((failed - 0.5).abs() < 1e-12, "solve {solve}: {failed}");
        assert!((finished - 0.5).abs() < 1e-12, "solve {solve}: {finished}");
        assert_eq!(out.stats.solved_loops > 0, solve);
    }

    let src = "fn walk() {
  let r ~ d3
  if r == 1 { return 1 / 0 }
  if r == 2 { return 1 }
  return walk()
}
report walk()";
    let out = run(src, &partial());
    let (failed, finished) = weights(&out);
    assert!((failed - 0.5).abs() < 1e-9, "{failed}");
    assert!((finished - 0.5).abs() < 1e-9, "{finished}");
}

#[test]
fn simulate_and_callbacks_fail_as_a_whole() {
    // An invalid outcome inside `simulate` fails the world that runs it,
    // not a sixth of it.
    let src = "let a ~ bernoulli(50%)
let d = if a { simulate { let y ~ d6 - 1; 1 / y } } else { d2 }
report d";
    let out = run(src, &partial());
    let (failed, _) = weights(&out);
    close(failed, 0.5);
    assert_eq!(out.failures.groups[0].error.message, "division by zero");

    let src = "let x ~ d2 - 1
let ys = [1, 2, x].map(v -> 10 / v)
report ys";
    let (failed, finished) = weights(&run(src, &partial()));
    close(failed, 0.5);
    close(finished, 0.5);
}

#[test]
fn other_errors_still_stop_the_run() {
    let options = partial();
    // A type error.
    let e = fails("let x ~ d6\nlet y = x + \"a\"\nreport y", &options);
    assert!(e.fault.is_none() && e.message.contains("can't add"), "{e:?}");
    // A declared type is a contract, not a fault of the values.
    let e = fails("let x ~ d6\nlet p: prob = x / 3\nreport p", &options);
    assert!(e.fault.is_none(), "{e:?}");
    // A limit.
    let limited = Options {
        limits: Limits {
            max_worlds: 10,
            ..Limits::default()
        },
        ..options.clone()
    };
    let e = fails("let a ~ d6\nlet b ~ d6\nreport 1 / (a - b)", &limited);
    assert_eq!(e.kind, ErrorKind::Limit);
    // Impossible evidence is not a fault either.
    fails("let x ~ d6\nobserve x > 6\nreport x", &options);
}

#[test]
fn value_dependent_errors_are_faults() {
    for (src, failed) in [
        ("let i ~ d3 - 1\nlet v = [10][i]\nreport v", 2.0 / 3.0),
        ("let k ~ d2\nlet v = [1: \"a\"][k]\nreport v", 0.5),
        ("let x ~ d2 - 2\nlet v = sqrt(x)\nreport v", 0.5),
        ("let p ~ one_of([0%, 50%])\nlet v = logit(p)\nreport v", 0.5),
        (
            "let n ~ d2\nvar deck = bag([\"a\": 1])\nrepeat n { let c = deck.take() }\nreport n",
            0.5,
        ),
        ("let n ~ d2 - 1\nlet v = date(9999, 12, 31) + n\nreport v", 0.5),
        ("let n ~ d2 - 1\nlet v = 7 mod n\nreport v", 0.5),
        ("let s ~ d2 - 1\nlet v = normal(0, s)\nreport v", 0.5),
        ("let p ~ one_of([0.5, 1.5])\nlet v ~ bernoulli(p)\nreport v", 0.5),
    ] {
        let out = run(src, &partial());
        assert!((weights(&out).0 - failed).abs() < 1e-12, "{src}: {:?}", weights(&out));
        assert!(out.failures.groups[0].error.fault.is_some(), "{src}");
    }
}

#[test]
fn failed_weight_counts_only_when_no_evidence_could_follow() {
    // All the evidence comes first: the failed worlds' weight is final.
    let src = "let x ~ d6 - 1
observe x < 3
let y = 1 / x
report y";
    let out = run(src, &partial());
    assert!(!out.failures.before_evidence);
    close(out.evidence.unwrap().to_f64(), 0.5);
    close(out.failures.weight.ratio(out.failures.weight + out.finished), 1.0 / 3.0);
    close(reach(&out, 0).unwrap(), 2.0 / 3.0);
    assert!(out.output.contains("33.33% failed"), "{}", out.output);

    // Evidence after the fault: the failed worlds would have met it.
    let src = "let x ~ d6 - 1
let y = 1 / x
observe x > 2
report y";
    let out = run(src, &partial());
    assert!(out.failures.before_evidence);
    assert_eq!(reach(&out, 0), None);
    assert!(
        out.output
            .starts_with("enumerated · partial result · evidence of the finished worlds 50.00%"),
        "{}",
        out.output
    );

    // Evidence in a later round of a loop, or after the call that failed.
    for src in [
        "var n = 0\nwhile n < 3 { n += 1; let x ~ d2 - 1; let y = 1 / x; observe true }\nreport n",
        "fn f() { let x ~ d2 - 1; return 1 / x }\nlet v = f()\nobserve v > 0\nreport v",
    ] {
        assert!(run(src, &partial()).failures.before_evidence, "{src}");
    }
}

#[test]
fn partial_sampling_does_not_depend_on_threads() {
    let src = r#"
@mode sample(runs: 20_000, seed: 3)
let k ~ d100
let m ~ d100
if m == 1 { print("run with", k) }
let xs = [0, 1, 2]
if m == 1 and k > 90 { report xs[k] }
report k
"#;
    let on = |threads: usize| {
        let (program, _) = probl_sema::compile(src);
        let options = Options {
            limits: Limits {
                max_threads: threads,
                ..Limits::default()
            },
            ..Options::default()
        };
        let mut lines = Vec::new();
        let out = probl_engine::run(&program.unwrap(), &options, &mut |l: &str| lines.push(l.to_string())).unwrap();
        let failures: Vec<_> = out
            .failures
            .groups
            .iter()
            .map(|g| (g.error.message.clone(), g.runs, g.first_run, g.weight.to_f64()))
            .collect();
        (out.output, lines, failures)
    };
    let one = on(1);
    assert!(!one.2.is_empty() && one.1.len() > 100, "{:?}", one.2);
    for threads in [2, 8] {
        assert_eq!(on(threads), one, "{threads} threads");
    }
}

#[test]
fn every_world_failing_still_gives_the_outcome() {
    let out = run("let x ~ d6\nreport x == 1 as \"one\"\nlet y = x / 0", &partial());
    let (failed, finished) = weights(&out);
    close(failed, 1.0);
    assert_eq!(finished, 0.0);
    // What the worlds reported before failing is kept.
    close(out.reports[0].chance().unwrap(), 1.0 / 6.0);
    // Evidence that rules out every finished world isn't called impossible
    // when others failed first.
    let out = run("let x ~ d2 - 1\nlet y = 1 / x\nobserve y > 5\nreport y", &partial());
    assert_eq!(out.finished.to_f64(), 0.0);
    assert!(!out.failures.is_empty());
}
