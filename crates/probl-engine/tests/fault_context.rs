//! What a fault that stops a run says about itself (stage 0 of
//! docs/design/error-handling.md): the failing operation's inputs, and, when
//! enumerating, the share of the weight reaching it for which it fails. A
//! share is given only when it can be established; checking the other
//! worlds prints nothing and runs none of the program's own code.

use probl_engine::{FailedRun, FailureMode, FaultContext, Options, RuntimeError, run};
use probl_sema::ir::Mode;

/// The error that stops `src`, and what it printed first.
fn stop(src: &str, options: &Options) -> (RuntimeError, Vec<String>) {
    let (program, diags) = probl_sema::compile(src);
    let program = program.unwrap_or_else(|| panic!("compile errors: {diags:?}"));
    let mut lines = Vec::new();
    let mut print = |line: &str| lines.push(line.to_string());
    let e = run(&program, options, &mut print).expect_err("expected the run to stop");
    (e, lines)
}

fn context(src: &str) -> FaultContext {
    let (e, _) = stop(src, &Options::default());
    assert!(e.fault.is_some(), "{e:?}");
    *e.context.unwrap_or_else(|| panic!("no context: {}", e.message))
}

fn share(src: &str) -> Option<f64> {
    context(src).share
}

fn inputs(c: &FaultContext) -> Vec<(Option<&str>, &str)> {
    c.inputs.iter().map(|i| (i.name.as_deref(), i.value.as_str())).collect()
}

fn close(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("a share");
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn the_reciprocal_fails_for_a_sixth_wherever_it_is() {
    for src in [
        "let x ~ d6 - 1\nlet y = 1 / x\nreport y",
        "let x ~ d6 - 1\nreport 1 / x",
        "let x ~ d6 - 1\nprint(1 / x)",
        "let x ~ d6 - 1\nreport x by 1 / x",
    ] {
        let c = context(src);
        close(c.share, 1.0 / 6.0);
        assert!(!c.per_pass && c.run.is_none(), "{src}");
        assert_eq!(inputs(&c), [(Some("x"), "0")], "{src}");
    }
}

#[test]
fn the_share_includes_the_evidence_so_far() {
    close(share("let x ~ d6 - 1\nobserve x != 1\nreport 1 / x"), 1.0 / 5.0);
    // Weights, not world counts: one world in two weighs a tenth.
    close(share("let x ~ one_of([0: 10%, 1: 90%])\nreport 1 / x"), 0.1);
    // Within a branch, of the weight that took it.
    close(share("let x ~ d6 - 1\nif x < 2 { report 1 / x }"), 0.5);
}

#[test]
fn other_operations_and_unnamed_inputs() {
    let c = context("let x ~ d6 - 1\nlet xs = [1, 2, 3]\nreport xs[x]");
    close(c.share, 0.5);
    assert_eq!(inputs(&c), [(Some("xs"), "[1, 2, 3]"), (Some("x"), "3")]);
    let c = context("let x ~ d6 - 1\nreport sqrt(x - 3)");
    close(c.share, 0.5);
    assert_eq!(inputs(&c), [(None, "-3")]);
    // Literals are in the source already.
    let c = context("let x ~ d6 - 1\nreport 10 / (x - 2)");
    assert_eq!(inputs(&c), [(None, "0")]);
}

#[test]
fn checking_the_other_worlds_prints_nothing() {
    // x = 0 is the first world: nothing was printed before it, and the
    // check of the other five prints nothing either.
    let (e, lines) = stop("let x ~ d6 - 1\nprint(1 / x)", &Options::default());
    close(e.context.unwrap().share, 1.0 / 6.0);
    assert!(lines.is_empty(), "{lines:?}");
    // The worlds before the failing one printed, as before.
    let (e, lines) = stop("let x ~ d6 - 1\nprint(1 / (x - 5))", &Options::default());
    close(e.context.unwrap().share, 1.0 / 6.0);
    assert_eq!(lines.len(), 5, "{lines:?}");
}

#[test]
fn a_loop_pass_is_labelled() {
    let c = context("let x ~ d6 - 1\nfor i in 0..3 {\n  let y = 1 / (x - i)\n}");
    close(c.share, 1.0 / 6.0);
    assert!(c.per_pass);
    assert_eq!(inputs(&c), [(None, "0")]);
}

#[test]
fn without_a_justified_share_only_the_inputs_are_given() {
    for src in [
        // One call's worlds aren't every call's.
        "fn f(n) { 1 / n }\nlet x ~ d6 - 1\nreport f(x)",
        // Another operation fails in other worlds.
        "let x ~ d6 - 1\nreport 1 / x + 1 / (x - 1)",
        // Only some worlds reach it.
        "let x ~ d6 - 1\nreport x > 2 or 1 / (x - 1) > 0",
        // Checking it would run a callback.
        "let x ~ d6 - 1\nreport [1].map(n -> n)[0] / x",
        // Weights in different units: some include a density.
        "let x ~ d6 - 1\nif x > 2 { observe 0.5 from normal(0, 1) }\nreport 1 / x",
        // Inside a `try`, which may take some worlds' faults.
        "let x ~ d6 - 1\nlet y = try { 1 / x } catch IndexOutOfBounds { 0 }",
        // Statements of other kinds.
        "let x ~ d6 - 1\nobserve 1 / x > 0",
        "let x ~ d6 - 1\nif 1 / x > 0 { report x }",
    ] {
        let c = context(src);
        assert_eq!(c.share, None, "{src}");
        assert!(!c.inputs.is_empty(), "{src}");
    }
}

#[test]
fn a_sampled_run_names_the_run() {
    let options = Options {
        mode: Some(Mode::Sample { runs: 1000, seed: 3 }),
        on_error: Some(FailureMode::Total),
        ..Options::default()
    };
    let (e, lines) = stop("let x ~ d6 - 1\nprint(1 / x)", &options);
    let c = e.context.unwrap();
    assert_eq!(c.share, None);
    assert_eq!(inputs(&c), [(Some("x"), "0")]);
    let FailedRun { run, runs, seed } = c.run.unwrap();
    assert_eq!((runs, seed), (1000, 3));
    // The runs before it printed, one line each.
    assert_eq!(lines.len() as u64, run);
}

#[test]
fn long_inputs_are_cut_short_and_other_errors_have_no_context() {
    let c = context("let x ~ d6 - 1\nlet xs = [...0..1000]\nreport xs[x * 1000]");
    let value = &c.inputs[0].value;
    assert!(value.ends_with('…') && value.len() <= 64, "{value}");
    let (e, _) = stop("let x ~ d6 - 1\nlet y: int = x / 2", &Options::default());
    assert!(e.fault.is_none() && e.context.is_none(), "{e:?}");
}
