//! Regression tests for the findings of docs/project-audit.md. Each test is
//! named after the finding it covers and checks the behaviour now specified
//! in docs/semantics.md.

mod common;

use common::*;
use probl_engine::{ErrorKind, Limits, Options};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

// ── D1: events, probabilities and distributions of probabilities ─────────

#[test]
fn d1_logic_on_probabilities_is_rejected() {
    assert!(error("let p = 30%\nreport p and not p").contains("needs facts"));
    assert!(error("let p = 30%\nreport p or not p").contains("needs facts"));
    assert!(error("report not 30%").contains("needs facts"));
    // Drawn events have identities, so the usual logical laws hold.
    close(
        chance("let happened ~ bernoulli(30%)\nreport happened and not happened"),
        0.0,
    );
    close(
        chance("let happened ~ bernoulli(30%)\nreport happened or not happened"),
        1.0,
    );
}

#[test]
fn d1_a_shared_rate_is_not_averaged_away() {
    // Choose a 10% or 90% rate, then run two independent trials with it.
    let src = "
        let p ~ simulate { if (chance { 50% => true, else => false }) { 10% } else { 90% } }
        let a ~ bernoulli(prob(p))
        let b ~ bernoulli(prob(p))
        report a and b";
    close(chance(src), 0.5 * 0.01 + 0.5 * 0.81);
    // Floats and percentages mean the same thing.
    let floats = src.replace("10%", "0.1").replace("90%", "0.9");
    close(chance(&floats), 0.41);
}

#[test]
fn d1_match_needs_a_settled_value() {
    let src = "let x = d6\nlet v = match x { 1 => \"a\", 2 => \"b\", _ => \"other\" }\nreport v";
    assert!(error(src).contains("`match` needs a settled value"));
    let settled = "let x ~ d6\nlet v = match x { 1 | 2 => \"low\", _ => \"high\" }\nreport v == \"low\"";
    close(chance(settled), 1.0 / 3.0);
}

#[test]
fn d1_type_annotations_are_enforced() {
    assert!(compile_error("let p: prob = \"not a probability\"").contains("expected a probability"));
    assert!(error("let s = \"x\"\nlet p: prob = s").contains("`p` should be a prob"));
}

// ── D2: accuracy after conditioning ──────────────────────────────────────

#[test]
fn d2_bounded_loops_are_never_cut_short() {
    let src = "
        @epsilon 0.01
        var win = false
        if (chance { 0.5% => true, else => false }) { repeat 1 { win = true } }
        observe true from bernoulli(prob(if win { true } else { 0.00001 }))
        report win";
    let expected = 0.005 / (0.005 + 0.995 * 0.00001);
    close(chance(src), expected);
    close(
        chance("observe true from bernoulli(1e-13)\nrepeat 0 { }\nreport true"),
        1.0,
    );
}

#[test]
fn d2_unbounded_loops_are_cut_relative_to_what_entered() {
    // The loop only sees 0.5% of the weight, but is judged against that.
    let src = "
        @epsilon 0.01
        var win = false
        if (chance { 0.5% => true, else => false }) {
            var n = 0
            while n < 1 { n += 1 }
            win = true
        }
        observe true from bernoulli(prob(if win { true } else { 0.00001 }))
        report win";
    close(chance(src), 0.005 / (0.005 + 0.995 * 0.00001));
}

#[test]
fn d2_truncation_before_evidence_is_shown_as_a_range() {
    // A loop that may never end is cut at ε; then evidence favours exactly
    // the worlds that were cut. The answer is reported as a range, not as a
    // misleadingly precise number.
    // With ε = 1%, the loop is cut after about 458 steps; the evidence keeps
    // only the last few resolved steps, which weigh less than the cut tail.
    let src = "
        @epsilon 0.01
        var steps = 0
        while (chance { 99% => true, else => false }) { steps += 1 }
        observe steps > 400
        report steps > 450";
    let out = output(src);
    assert!(out.contains('–'), "expected a range in:\n{out}");
    // When nothing resolved fits the evidence, there's no answer at all.
    let hopeless = "@epsilon 0.01\nvar steps = 0\nwhile (chance { 99.9% => true, else => false }) { steps += 1 }\nobserve steps > 10000\nreport true";
    assert!(error(hopeless).contains("left unresolved"));
}

// ── D3: inference scopes and report denominators ─────────────────────────

#[test]
fn d3_reports_say_how_much_weight_reached_them() {
    let out = output("if (chance { 1% => true, else => false }) { report true as \"win\" }");
    assert!(out.contains("win    100.00% (reached in 1.00% of worlds)"), "{out}");
}

#[test]
fn d3_repeated_reports_are_labelled_per_visit() {
    let out = output("let long ~ bernoulli(50%)\nrepeat if long { 9 } else { 1 } { report long by \"all\" }");
    assert!(out.contains("long (per visit)"), "{out}");
    // Keyed by a `for` loop's variable, each world reports once per key.
    let per_key = output("for i in 1..3 { report i > 1 by i }");
    assert!(!per_key.contains("per visit"), "{per_key}");
}

#[test]
fn d3_observations_inside_simulate_are_local() {
    let o = outcome("let d = simulate { observe true from bernoulli(10%)\n 1 }\nreport d");
    assert!(o.evidence.is_none());
    assert!(!o.output.contains("evidence"));
}

#[test]
fn d3_impossible_evidence_is_an_error() {
    assert!(error("observe false\nreport true").contains("the evidence is impossible"));
    // Unreachable reports are ordinary control flow.
    assert!(output("if false { report 1 as \"x\" }").contains("(never reached)"));
}

#[test]
fn d3_no_observe_after_a_report() {
    let src = "let a ~ d6\nreport a\nobserve a > 2";
    assert!(compile_error(src).contains("this `observe` can run after a `report`"));
    let in_loop = "for i in 1..3 {\n  let a ~ d6\n  observe a > 1\n  report a by i\n}";
    assert!(compile_error(in_loop).contains("can run after a `report`"));
    let via_call = "fn check(x) { observe x > 1\n x }\nlet a ~ d6\nreport a\nlet b = check(a)";
    assert!(compile_error(via_call).contains("`check` makes observations"));
    // Observations inside `simulate` are local, so they're fine.
    output("let a ~ d6\nreport a\nlet s = simulate { let b ~ d6\n observe b > 1\n a + b }\nreport s");
    // …but a `simulate` whose own evidence is impossible is an error.
    assert!(error("let a ~ d6\nlet s = simulate { observe a > 1\n a }").contains("ruled out"));
}

// ── I1: numerical contract ───────────────────────────────────────────────

#[test]
fn i1_reconstructed_fractions_are_marked_approximate() {
    let options = Options {
        fractions: true,
        ..Options::default()
    };
    let out = exec("report 2d6 == 7", &options).unwrap().output;
    assert!(out.contains("16.67% (≈ 1/6)"), "{out}");
    assert!(out.starts_with("enumerated"), "{out}");
}

#[test]
fn i1_missing_mass_composes() {
    // The pool of two geometric draws misses about twice the single tail.
    let o = outcome("let p = roll(2, geometric(50%))\nreport len(support(p)) > 0");
    close(o.reports[0].chance().unwrap(), 1.0);
    // A simulated distribution keeps its truncation as missing mass instead
    // of adding it to the program's unresolved weight.
    let o = outcome(
        "let t = simulate { var n = 1\n while (chance { 50% => true, else => false }) { n += 1 }\n n }\nreport t > 3",
    );
    assert!(o.unresolved.is_zero());
    let (lo, hi) = o.reports[0].groups.values().next().unwrap().chance_bounds(o.unresolved);
    assert!(lo < hi && hi - lo < 1e-11);
}

#[test]
fn i1_many_observations_do_not_underflow() {
    let src = "
        let biased ~ bernoulli(50%)
        repeat 400 { observe true from bernoulli(prob(if biased { 1e-5 } else { 2e-5 })) }
        report biased";
    let o = outcome(src);
    assert!(!o.evidence.unwrap().is_zero());
    let expected = 1.0 / (1.0 + 2f64.powi(400));
    close(o.reports[0].chance().unwrap(), expected);
}

// ── I2: evaluation order and effects ─────────────────────────────────────

#[test]
fn i2_operands_are_evaluated_left_to_right() {
    close(
        chance("var x = 1\nreport x + if true { x = 2; 0 } else { 0 } == 1"),
        1.0,
    );
    close(
        chance("var x = 1\nfn read_x() { x }\nreport read_x() + if true { x = 2; 0 } else { 0 } == 1"),
        1.0,
    );
    close(chance("var x = 1\nreport [x, { x = 2; x }] == [1, 2]"), 1.0);
    // `+=` reads the variable before evaluating the value.
    close(chance("var x = 1\nx += { x = 10; 1 }\nreport x == 2"), 1.0);
    // Assignment evaluates the value before the indices of the place.
    close(
        chance("var xs = [0, 0]\nvar i = 0\nxs[i] = { i = 1; 5 }\nreport xs == [0, 5]"),
        1.0,
    );
}

#[test]
fn i2_functions_that_print_are_not_memoized() {
    let src = "fn f() { print(\"called\")\n 1 }\nrepeat 2 { let x = f() }";
    for memoize in [true, false] {
        let (program, _) = probl_sema::compile(src);
        let mut lines = Vec::new();
        let mut print = |l: &str| lines.push(l.to_string());
        let options = Options {
            memoize,
            ..Options::default()
        };
        probl_engine::run(&program.unwrap(), &options, &mut print).unwrap();
        assert_eq!(lines, vec!["called", "called"], "memoize = {memoize}");
    }
}

// ── I3: resource limits and error containment ────────────────────────────

#[test]
fn i3_singleton_distributions_do_not_crash() {
    close(chance("report roll(1, 1) == [1]"), 1.0);
    close(chance("report d1 == 1"), 1.0);
    close(chance("report one_of([5]) + 1 == 6"), 1.0);
}

#[test]
fn i3_boundary_integers_do_not_crash() {
    output("report len((9223372036854775806)..(9223372036854775807))");
    assert_eq!(mean("report len(1..<(-9223372036854775807 - 1))"), 0.0);
    assert_eq!(chance("report 9223372036854775807 + 1 == 9223372036854775808"), 1.0);
    assert_eq!(
        chance("report (-9223372036854775807 - 1) div -1 == 9223372036854775808"),
        1.0
    );
    assert!(error("report len(support(one_of(0..9223372036854775807)))").contains("over the limit"));
}

fn limited(limits: Limits) -> Options {
    Options {
        limits,
        ..Options::default()
    }
}

#[test]
fn i3_host_limits_hold() {
    let small = limited(Limits {
        max_outcomes: 1000,
        ..Limits::default()
    });
    let err = exec_raw("report len(support(d100000))", &small).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);
    let err = exec_raw("report roll(30, d6)", &small).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);
    let err = exec_raw("report poisson(1e15)", &small).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);

    let little_work = limited(Limits {
        max_work: 10_000,
        ..Limits::default()
    });
    let err = exec_raw("var n = 0\nwhile true { n += 1 }", &little_work).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);

    // A program can lower the host's world limit but not raise it.
    let few_worlds = limited(Limits {
        max_worlds: 10,
        ..Limits::default()
    });
    let err = exec_raw("@max_worlds 1000000\nlet a ~ d20\nreport a", &few_worlds).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);
    assert!(exec("@max_worlds 5\nlet a ~ d6\nreport a", &Options::default()).is_err());

    let shallow = limited(Limits {
        max_call_depth: 10,
        ..Limits::default()
    });
    let err = exec_raw("fn f(n) { if n == 0 { 0 } else { f(n - 1) } }\nreport f(50)", &shallow).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);
}

#[test]
fn i3_runs_can_be_cancelled() {
    let cancel = Arc::new(AtomicBool::new(true));
    let options = Options {
        cancel: Some(cancel),
        ..Options::default()
    };
    let err = exec_raw("report 1", &options).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);
    assert!(err.message.contains("cancelled"));
}

#[test]
fn i3_deep_nesting_is_a_diagnostic() {
    let src = format!("report {}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(compile_error(&src).contains("nested too deeply"));
    let chain = format!("var x = 0\n{}{{ 2 }}", "if x == 1 { 1 } else ".repeat(3000));
    assert!(compile_error(&chain).contains("nested too deeply"));
}

// ── I4: found by the oracle (crates/probl-oracle) ────────────────────────

#[test]
fn i4_a_value_drawn_from_one_of_is_settled() {
    // `one_of` mixes in options that are distributions.
    let src = "let x ~ one_of([d2, 10])\nreport x == x";
    close(chance(src), 1.0);
    let d = distribution("report one_of([d2, 10])");
    assert_eq!(d.len(), 3);
    close(d[0].1, 0.25);
    close(d[2].1, 0.5);
}

#[test]
fn i4_chance_weights_require_explicit_probabilities() {
    // A fact counts with 100% or 0%, an uncertain fact with its probability.
    close(
        chance("let x ~ d6\nlet y = chance { prob(x > 4) => 1, else => 2 }\nreport y == 1"),
        1.0 / 3.0,
    );
    close(
        chance("let y = chance { P(d6 > 4) => 1, else => 2 }\nreport y == 1"),
        1.0 / 3.0,
    );
    // The weights still can't add up to more than 100%.
    assert!(error("let x = chance { prob(true) => 1, 50% => 2 }\nreport x").contains("more than 100%"));
}

#[test]
fn i4_certain_conditions_leave_no_rounding_behind() {
    // Six sixths add up to 0.9999999999999999 in floating point; a certain
    // condition must still send every world one way.
    let out = outcome(
        "let a = P(d6 != 0)\nif ({ let rolled ~ d6 + d6; rolled > 1 }) { observe true from bernoulli(50%) }\nreport a",
    );
    assert!(out.unresolved.is_zero(), "unresolved {:e}", out.unresolved.to_f64());
    assert_eq!(out.reports[0].distribution().len(), 1);
    assert!(
        error("if ({ let event ~ d6 != 0; event }) { observe true from bernoulli(0%) }\nreport true")
            .contains("evidence is impossible")
    );
}

#[test]
fn i4_operands_run_before_later_calls() {
    // The left operand runs first, so it fails before the call rules every
    // world out.
    let err = error("fn f(x) { observe false; x }\nreport simulate { observe false; 1 } == f(1)");
    assert!(err.contains("simulate"), "{err}");
    // And its output comes first.
    let (program, _) = probl_sema::compile("fn f() { print(\"second\"); 1 }\nreport [print(\"first\"), f()]");
    let mut lines = Vec::new();
    let mut print = |line: &str| lines.push(line.to_string());
    probl_engine::run(&program.unwrap(), &Options::default(), &mut print).unwrap();
    assert_eq!(lines, ["first", "second"]);
}
