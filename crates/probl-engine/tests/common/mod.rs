//! Helpers shared by the engine's integration tests.
#![allow(dead_code)]

use probl_engine::value::Value;
use probl_engine::{Options, Outcome, RuntimeError, run};
use probl_syntax::{SourceFile, render_all};

/// Run a program. A run whose worlds failed in partial mode, as sampled runs
/// are by default, gives the first failure's error, as a total run would:
/// these tests are about what fails, not about what the other worlds do.
pub fn exec_raw(src: &str, options: &Options) -> Result<Outcome, RuntimeError> {
    let outcome = exec_partial(src, options)?;
    match outcome.failures.groups.first() {
        Some(failure) => Err(failure.error.clone()),
        None => Ok(outcome),
    }
}

/// Run a program, with its failures in the outcome when it's partial.
pub fn exec_partial(src: &str, options: &Options) -> Result<Outcome, RuntimeError> {
    let (program, diags) = probl_sema::compile(src);
    let file = SourceFile::new("test.probl", src);
    let Some(program) = program else {
        panic!("compile errors:\n{}", render_all(&diags, &file, false));
    };
    let mut print = |_: &str| {};
    run(&program, options, &mut print)
}

pub fn exec(src: &str, options: &Options) -> Result<Outcome, String> {
    let file = SourceFile::new("test.probl", src);
    exec_raw(src, options).map_err(|e| e.to_diagnostic().render(&file, false))
}

pub fn outcome(src: &str) -> Outcome {
    exec(src, &Options::default()).unwrap_or_else(|e| panic!("runtime error:\n{e}"))
}

/// The printed output (summary line and reports).
pub fn output(src: &str) -> String {
    outcome(src).output
}

/// The chance reported by the first `report`.
pub fn chance(src: &str) -> f64 {
    outcome(src).reports[0]
        .chance()
        .expect("the first report should be a probability")
}

pub fn mean(src: &str) -> f64 {
    let dist = outcome(src).reports[0].distribution();
    dist.iter().map(|(v, p)| v.as_f64().unwrap() * p).sum()
}

pub fn distribution(src: &str) -> Vec<(Value, f64)> {
    outcome(src).reports[0].distribution()
}

pub fn error(src: &str) -> String {
    exec(src, &Options::default()).expect_err("expected a runtime error")
}

/// The rendered compile errors of a program that must not compile.
pub fn compile_error(src: &str) -> String {
    let (program, diags) = probl_sema::compile(src);
    assert!(program.is_none(), "expected compile errors for {src:?}");
    render_all(&diags, &SourceFile::new("test.probl", src), false)
}

#[track_caller]
pub fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
}
