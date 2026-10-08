//! `try` and `catch` (docs/semantics.md, section 11): a world whose `try`
//! body faults goes on in the first `catch` that takes the fault, with what
//! it did before the fault; the other worlds keep their results.

mod common;

use common::*;
use probl_engine::{ErrorKind, FailureMode, Limits, Options, Outcome};
use probl_sema::ir::Mode;

fn mean_of(out: &Outcome, report: usize) -> f64 {
    out.reports[report]
        .distribution()
        .iter()
        .map(|(v, p)| v.as_f64().unwrap() * p)
        .sum()
}

#[track_caller]
fn exactly(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
        "expected {expected}, got {actual}"
    );
}

fn run(src: &str, options: &Options) -> Outcome {
    exec(src, options).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

/// Every combination of the engine's checks: merging, memoization and
/// solving loops, on and off.
fn checks() -> Vec<Options> {
    let mut all = Vec::new();
    for merge in [true, false] {
        for memoize in [true, false] {
            for solve in [true, false] {
                all.push(Options {
                    merge,
                    memoize,
                    solve,
                    epsilon: Some(1e-15),
                    ..Options::default()
                });
            }
        }
    }
    all
}

#[test]
fn a_catch_gives_the_failed_worlds_a_value() {
    let src = "let x ~ d6 - 1
let y = try { 1 / x } catch DivisionByZero { 0 }
report y";
    let out = outcome(src);
    exactly(mean_of(&out, 0), 137.0 / 360.0);
    assert!(out.failures.is_empty());
    // When sampling too, and no run fails.
    let sampled = Options {
        mode: Some(Mode::Sample { runs: 20_000, seed: 1 }),
        ..Options::default()
    };
    let out = run(src, &sampled);
    assert!(out.failures.is_empty());
    assert!((mean_of(&out, 0) - 137.0 / 360.0).abs() < 0.01);
}

#[test]
fn the_first_catch_that_takes_the_fault_runs() {
    let src = "let i ~ d3 - 1
let xs = [10, 20]
let v = try { 100 / xs[i] } catch DivisionByZero { -1 } catch IndexOutOfBounds { -2 } catch { -3 }
report v
let k ~ d2
let w = try { [\"a\": 1][\"b\"] + k } catch { 0 }
report w
let e = try { 7 mod (k - 1) } catch IndexOutOfBounds { 1 } catch { 2 }
report e";
    let out = outcome(src);
    exactly(mean_of(&out, 0), (10.0 + 5.0 - 2.0) / 3.0);
    exactly(mean_of(&out, 1), 0.0);
    // `7 mod 0` isn't an index: the catch for every fault takes it.
    exactly(mean_of(&out, 2), (2.0 + 0.0) / 2.0);
}

#[test]
fn a_fault_no_catch_takes_goes_on_outward() {
    // The inner `try` doesn't catch a missing key; the outer one does.
    let src = "let k ~ d2
let v = try {
  try { [1: 10][k] } catch DivisionByZero { -1 }
} catch MissingKey { 0 }
report v";
    exactly(mean_of(&outcome(src), 0), 5.0);
    // Nothing catches it: the run stops, or the world fails.
    let src = "let k ~ d2\nlet v = try { [1: 10][k] } catch DivisionByZero { -1 }\nreport v";
    assert!(error(src).contains("isn't in the map"));
    let partial = Options {
        on_error: Some(FailureMode::Partial),
        ..Options::default()
    };
    let out = exec_partial(src, &partial).unwrap();
    exactly(out.failures.weight.to_f64(), 0.5);
}

#[test]
fn a_fault_in_a_catch_is_for_the_try_around_it() {
    // Not the same `try`'s other catches.
    let src = "let x ~ d2 - 1
let v = try { 1 / x } catch DivisionByZero { [1][5] } catch IndexOutOfBounds { -1 }
report v";
    assert!(error(src).contains("out of range"));
    let src = "let x ~ d2 - 1
let v = try {
  try { 1 / x } catch DivisionByZero { [1][5] }
} catch IndexOutOfBounds { -1 }
report v";
    exactly(mean_of(&outcome(src), 0), (1.0 - 1.0) / 2.0);
}

#[test]
fn faults_in_calls_reach_the_caller_s_try() {
    let src = "fn f(scale) {
  let y ~ d6 - 1
  return scale / y
}
let v = try { f(1) } catch DivisionByZero { 0 }
report v
fn g(z) { return try { 1 / z } catch { 5 } }
let k ~ d2 - 1
report g(k)";
    for options in checks() {
        let out = run(src, &options);
        exactly(mean_of(&out, 0), 137.0 / 360.0);
        exactly(mean_of(&out, 1), 3.0);
        assert!(out.failures.is_empty());
    }
    // A call's fault that its caller doesn't catch stops the run.
    let src = "fn f(z) { return 1 / z }\nlet k ~ d2 - 1\nreport f(k)";
    assert!(error(src).contains("division by zero"));
}

#[test]
fn a_caught_world_keeps_what_it_did_before_the_fault() {
    // The assignment happened; the draw isn't made again.
    let src = "var marker = 0
let y = try {
  marker = 1
  let x ~ d6 - 1
  1 / x
} catch DivisionByZero {
  marker * 100
}
report y == 100 as \"caught\"
report marker";
    let out = outcome(src);
    exactly(out.reports[0].chance().unwrap(), 1.0 / 6.0);
    assert_eq!(out.reports[1].distribution().len(), 1);
    // The weight it had, observations included.
    let src = "let x ~ d6 - 1
observe x < 3
let y = try { 1 / x } catch DivisionByZero { 10 }
report y";
    let out = outcome(src);
    exactly(out.evidence.unwrap().to_f64(), 0.5);
    exactly(mean_of(&out, 0), (10.0 + 1.0 + 0.5) / 3.0);
}

#[test]
fn a_catch_can_read_what_the_body_no_longer_needs() {
    // `a` isn't read in the body, but the catch reads it: worlds that
    // differ in it must not merge, nor lose it.
    let src = "let a ~ d2
let x ~ d2 - 1
let y = try {
  let b = 1 / x
  b
} catch DivisionByZero { a * 10 }
report y";
    for options in checks() {
        let out = run(src, &options);
        let distribution = out.reports[0].distribution();
        assert_eq!(distribution.len(), 3, "{distribution:?}");
        exactly(mean_of(&out, 0), 0.5 + 0.25 * 10.0 + 0.25 * 20.0);
    }
    // A draw doesn't move past a statement that can fault inside a `try`:
    // the catch sees what was drawn by then.
    let src = "var x = 0
let z ~ d2 - 1
let y = try {
  x ~ d6
  let q = 1 / z
  x
} catch DivisionByZero { x * 10 }
report y";
    exactly(mean_of(&outcome(src), 0), 0.5 * 3.5 + 0.5 * 35.0);
}

#[test]
fn only_faults_are_caught() {
    for src in [
        // A type error.
        "let x ~ d6\nlet y = try { x + \"a\" } catch { 0 }\nreport y",
        // A declared type.
        "let x ~ d6\nlet y = try { let p: prob = x / 3\n p } catch { 0 }\nreport y",
        // Impossible evidence.
        "let x ~ d6\nlet y = try { observe x > 6\n x } catch { 0 }\nreport y",
    ] {
        let e = exec_raw(src, &Options::default()).expect_err(src);
        assert!(e.fault.is_none(), "{src}: {e:?}");
    }
    let limited = Options {
        limits: Limits {
            max_worlds: 10,
            ..Limits::default()
        },
        ..Options::default()
    };
    let e = exec_raw(
        "let y = try { let a ~ d6\nlet b ~ d6\n a * 10 + b } catch { 0 }\nreport y",
        &limited,
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
}

#[test]
fn simulate_and_callbacks_are_caught_whole() {
    // A fault inside `simulate` fails the whole `simulate`, in every world
    // that runs it; a catch inside it gives a local outcome instead.
    let src = "let d = try { simulate { let y ~ d6 - 1; 1 / y } } catch DivisionByZero { d2 }
report d
let local = simulate { let y ~ d6 - 1; try { 1 / y } catch DivisionByZero { 0 } }
let v ~ local
report v";
    let out = outcome(src);
    exactly(mean_of(&out, 0), 1.5);
    exactly(mean_of(&out, 1), 137.0 / 360.0);

    let src = "let x ~ d2 - 1
let whole = try { [1, x].map(v -> 10 / v) } catch DivisionByZero { [] }
report len(whole)
let each = [1, 0].map(v -> try { 10 / v } catch DivisionByZero { 0 })
report each[1]";
    let out = outcome(src);
    exactly(mean_of(&out, 0), 1.0);
    exactly(mean_of(&out, 1), 0.0);
}

#[test]
fn loops_and_recursion_carry_faults_to_the_catch() {
    // Each round faults with 1/6 and leaves with 1/6: half the weight
    // reaches the catch, solved as a chain or unrolled.
    let src = "let v = try {
  loop {
    let r ~ d6
    if r == 1 { let z = 1 / 0 }
    if r == 6 { break }
  }
  1
} catch DivisionByZero { 0 }
report v
var caught = 0
loop {
  let r ~ d6
  let w = try { 1 / (r - 1) } catch DivisionByZero { 0 }
  if r == 6 { break }
}
report caught";
    // Unrolled without merging, the worlds would multiply each round.
    for options in checks().into_iter().filter(|o| o.merge) {
        let out = run(src, &options);
        assert!(
            (mean_of(&out, 0) - 0.5).abs() < 1e-12,
            "{:?}",
            out.reports[0].distribution()
        );
        assert_eq!(out.stats.solved_loops == 2, options.solve);
    }
    let src = "fn walk() {
  let r ~ d3
  if r == 1 { return 1 / 0 }
  if r == 2 { return 1 }
  return walk()
}
let v = try { walk() } catch { 0 }
report v";
    assert!((mean_of(&outcome(src), 0) - 0.5).abs() < 1e-9);
}

#[test]
fn caught_faults_are_not_failures_in_either_mode() {
    let src = "@on_error partial
let x ~ d6 - 1
let y = try { 1 / x } catch DivisionByZero { 0 }
report y";
    assert!(outcome(src).failures.is_empty());
    // A fault that no catch takes still fails its world in partial mode.
    let src = "@on_error partial
let x ~ d6 - 1
let y = try { 1 / x } catch IndexOutOfBounds { 0 }
report y";
    let out = exec_partial(src, &Options::default()).unwrap();
    exactly(out.failures.weight.to_f64(), 1.0 / 6.0);
}

#[test]
fn evidence_in_a_catch_follows_the_rules_for_evidence() {
    // Conditioning on success, said with evidence.
    let src = "let x ~ d6 - 1
let y = try { 1 / x } catch DivisionByZero { observe false; 0 }
report y";
    let out = outcome(src);
    exactly(out.evidence.unwrap().to_f64(), 5.0 / 6.0);
    exactly(mean_of(&out, 0), 137.0 / 300.0);
    // A catch can run after a report in its body.
    let src = "let x ~ d6 - 1
let y = try { report x; 1 / x } catch DivisionByZero { observe false; 0 }";
    assert!(compile_error(src).contains("can run after a `report`"));
}

#[test]
fn sampled_runs_catch_alike_on_any_number_of_threads() {
    let src = "@mode sample(runs: 5000, seed: 4)
let k ~ d100
let xs = [1, 2, 3]
let v = try { xs[k] } catch IndexOutOfBounds { 0 }
report v
report k";
    let on = |threads| {
        let options = Options {
            limits: Limits {
                max_threads: threads,
                ..Limits::default()
            },
            ..Options::default()
        };
        run(src, &options).output
    };
    let one = on(1);
    for threads in [2, 8] {
        assert_eq!(on(threads), one);
    }
}

#[test]
fn catches_are_checked_when_compiling() {
    assert!(compile_error("let y = try { 1 } catch DivisionByZer { 0 }").contains("did you mean `DivisionByZero`?"));
    assert!(compile_error("let y = try { 1 }\nreport y").contains("`try` needs a `catch`"));
    assert!(compile_error("let y = try { 1 } catch { 0 } catch DivisionByZero { 2 }").contains("never runs"));
    // `catch` can start the next line, as `else` can.
    let out = outcome("let x ~ d2 - 1\nlet y = try {\n  1 / x\n}\ncatch DivisionByZero {\n  0\n}\nreport y");
    exactly(mean_of(&out, 0), 0.5);
    // A `catch` that can't run is a warning.
    let (program, diagnostics) =
        probl_sema::compile("let y = try { 1 } catch DivisionByZero { 0 } catch DivisionByZero { 2 }");
    assert!(program.is_some());
    assert!(
        diagnostics.iter().any(|d| d.message.contains("never runs")),
        "{diagnostics:?}"
    );
}
