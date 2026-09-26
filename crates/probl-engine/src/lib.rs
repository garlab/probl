//! The Probl engine: values, distributions and the world-set interpreter.
//! The rules it implements are in docs/semantics.md.

pub mod builtins;
pub mod continuous;
pub mod dates;
pub mod dist;
pub mod error;
pub mod interp;
pub mod ops;
pub mod report;
pub mod value;
pub mod weight;
pub mod world;

pub use error::{ErrorKind, RuntimeError};
pub use interp::Stats;
pub use weight::Weight;

use dist::Budget;
use error::OpError;
use probl_sema::ir::{Mode, Program};
use report::{Format, Sink};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Upper bounds set by whoever runs a program. A program's `@max_worlds` and
/// `@max_iterations` can lower them, never raise them.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Worlds one statement may produce.
    pub max_worlds: usize,
    /// Outcomes of one distribution, or of combining distributions.
    pub max_outcomes: usize,
    /// Elements of one collection built by the program.
    pub max_collection: usize,
    /// Units of work: world-steps plus outcomes computed.
    pub max_work: u64,
    /// Iterations of one loop.
    pub max_iterations: u64,
    /// Nested calls.
    pub max_call_depth: usize,
    /// Function results kept for reuse (the cache is cleared when full).
    pub max_cached_calls: usize,
    /// Bytes of `print` output.
    pub max_output: usize,
    /// Stack size of the engine's thread; it bounds how deep calls can nest,
    /// together with `max_call_depth`.
    pub stack_size: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_worlds: 10_000_000,
            max_outcomes: 2_000_000,
            max_collection: 10_000_000,
            max_work: 20_000_000_000,
            max_iterations: 10_000_000,
            max_call_depth: 500,
            max_cached_calls: 1_000_000,
            max_output: 64 * 1024 * 1024,
            stack_size: 64 * 1024 * 1024,
        }
    }
}

/// How the engine runs. The defaults are what `probl run` uses.
#[derive(Clone, Debug)]
pub struct Options {
    /// Merge worlds that reach the same state (turning it off is only useful
    /// for testing that merging doesn't change results).
    pub merge: bool,
    /// Reuse the result of a function called again with the same inputs.
    pub memoize: bool,
    /// Overrides the program's `@epsilon`.
    pub epsilon: Option<f64>,
    /// Also print the simplest fraction near each probability.
    pub fractions: bool,
    pub limits: Limits,
    /// Set to stop a running program.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Overrides the program's `@mode`.
    pub mode: Option<Mode>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            merge: true,
            memoize: true,
            epsilon: None,
            fractions: false,
            limits: Limits::default(),
            cancel: None,
            mode: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    /// The summary line and the reports, as printed by `probl run`.
    pub output: String,
    pub stats: Stats,
    /// Weight left unresolved (upper bound on the weight of cut-off worlds).
    pub unresolved: Weight,
    /// The probability of the evidence, if the program observed anything.
    pub evidence: Option<Weight>,
    /// What each `report` collected, in the order of `Program::reports`.
    pub reports: Vec<Sink>,
    /// When sampling: how.
    pub sample: Option<Sampled>,
}

/// How a program was sampled (docs/semantics.md, section 14).
#[derive(Clone, Debug)]
pub struct Sampled {
    pub runs: u64,
    pub seed: u64,
    /// The effective sample size: (Σ w)² / Σ w².
    pub effective: f64,
    /// The runs' total weight, Σ w.
    pub weight: Weight,
    /// Σ w².
    pub squares: Weight,
}

/// Run a program. `print` receives the output of `print(…)` as it happens.
/// Every problem, including a crash of the engine itself, comes back as an
/// error.
pub fn run(
    program: &Program,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
) -> Result<Outcome, RuntimeError> {
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name("probl-engine".into())
            .stack_size(options.limits.stack_size)
            .spawn_scoped(scope, || run_here(program, options, print));
        let handle = match handle {
            Ok(h) => h,
            Err(e) => {
                return Err(internal(format!("couldn't start the engine: {e}")));
            }
        };
        handle.join().unwrap_or_else(|panic| {
            let detail = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "no details".to_string());
            Err(internal(detail))
        })
    })
}

fn internal(detail: String) -> RuntimeError {
    RuntimeError {
        message: "internal error: the engine crashed".to_string(),
        span: Default::default(),
        notes: vec![detail],
        help: Some("this is a bug in Probl; please report it with the program that caused it".to_string()),
        kind: ErrorKind::Internal,
    }
}

fn run_here(
    program: &Program,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
) -> Result<Outcome, RuntimeError> {
    let settings = &program.settings;
    let mode = options.mode.clone().unwrap_or_else(|| settings.mode.clone());
    let sample = match mode {
        Mode::Auto | Mode::Enumerate => None,
        Mode::Sample { runs, seed } => Some((runs, seed)),
        ref other => {
            return Err(
                OpError::unsupported(format!("`{}` mode isn't implemented yet", other.name()))
                    .help("enumeration and `@mode sample(runs: 10_000)` are; particles and beam search come later")
                    .at(settings.mode_span.unwrap_or_default()),
            );
        }
    };
    if let Some((runs, _)) = sample {
        if runs == 0 || runs > u32::MAX as u64 {
            return Err(OpError::new(format!(
                "sample mode needs between 1 and {} runs",
                report::thousands(u32::MAX as i64)
            ))
            .at(settings.mode_span.unwrap_or_default()));
        }
    }
    let limits = &options.limits;
    let config = interp::Config {
        epsilon: options.epsilon.unwrap_or(settings.epsilon),
        merging: options.merge,
        memoizing: options.memoize,
        max_worlds: limits.max_worlds.min(settings.max_worlds),
        max_iterations: limits.max_iterations.min(settings.max_iterations),
        max_call_depth: limits.max_call_depth,
        max_cached_calls: limits.max_cached_calls,
        max_output: limits.max_output,
        budget: Budget {
            max_outcomes: limits.max_outcomes,
            max_collection: limits.max_collection,
            work_left: limits.max_work,
        },
        cancel: options.cancel.clone(),
        sample_seed: sample.map(|(_, seed)| seed),
    };
    let epsilon = config.epsilon;
    let live = probl_sema::analyze(program);
    let mut engine = interp::Engine::new(program, &live, config, print);
    if let Some((runs, seed)) = sample {
        return sampled(program, engine, runs, seed);
    }
    let finished = engine.run_main()?;
    let unresolved = engine.unresolved;

    if engine.observed && finished.is_zero() {
        let span = engine.last_ruling_out.unwrap_or_default();
        return Err(if unresolved.is_zero() {
            RuntimeError::new(
                span,
                "the evidence is impossible: every world was ruled out by `observe`",
            )
            .with_help("check the observations; there's no answer to condition on")
        } else {
            RuntimeError::new(span, "every world that fits the evidence was left unresolved")
                .with_help("lower `@epsilon` so that loops run longer")
        });
    }

    // Reach describes control flow; unresolved weight is shown separately.
    let format = Format {
        fractions: options.fractions,
        unresolved,
        program_total: finished,
        run_squares: None,
    };
    let plain = Format {
        fractions: false,
        ..format
    };
    let mut header = String::from("enumerated");
    let evidence = engine.observed.then_some(finished);
    if let Some(z) = evidence {
        let (lo, hi) = (finished.to_f64(), (finished + unresolved).to_f64());
        if hi - lo >= 0.00005 {
            header.push_str(&format!(
                " · evidence {}–{}",
                report::pct(lo, plain),
                report::pct(hi, plain)
            ));
        } else if z.to_f64() < 0.0001 {
            header.push_str(&format!(" · evidence {}", interp::fmt_weight(z)));
        } else {
            header.push_str(&format!(" · evidence {}", report::pct(z.to_f64(), plain)));
        }
    }
    if !unresolved.is_zero() {
        if unresolved.to_f64() <= epsilon {
            header.push_str(&format!(" · unresolved < {epsilon:e}"));
        } else {
            header.push_str(&format!(" · unresolved {:.1e}", unresolved.to_f64()));
        }
    }
    let body = report::render(program, &engine.sinks, format);
    let output = if body.is_empty() {
        header
    } else {
        format!("{header}\n\n{body}")
    };
    Ok(Outcome {
        output,
        stats: engine.stats.clone(),
        unresolved,
        evidence,
        reports: std::mem::take(&mut engine.sinks),
        sample: None,
    })
}

/// Sample the program and print its estimates (docs/semantics.md, section 14).
fn sampled(program: &Program, mut engine: interp::Engine, runs: u64, seed: u64) -> Result<Outcome, RuntimeError> {
    let totals = engine.run_sampled(runs)?;
    if totals.weight.is_zero() {
        let span = engine.last_ruling_out.unwrap_or_default();
        return Err(RuntimeError::new(span, "every run was ruled out by `observe`")
            .with_note(format!("{} runs were tried", report::thousands(runs as i64)))
            .with_help(
                "the evidence may be impossible, or too unlikely for this many runs: enumerate the model, or add runs",
            ));
    }
    let effective = (totals.weight * totals.weight).ratio(totals.squares);
    let mut header = format!("sample · {} runs · seed {seed}", report::thousands(runs as i64));
    if engine.observed {
        header.push_str(&format!(
            " · effective sample size {}",
            report::thousands(effective.round() as i64)
        ));
    }
    let unresolved = engine.unresolved;
    if !unresolved.is_zero() {
        header.push_str(&format!(
            " · unresolved {:.1e}",
            unresolved.ratio(totals.weight + unresolved)
        ));
    }
    let format = Format {
        fractions: false,
        unresolved: Weight::ZERO,
        program_total: totals.weight,
        run_squares: Some(totals.squares),
    };
    let body = report::render(program, &engine.sinks, format);
    let output = if body.is_empty() {
        header
    } else {
        format!("{header}\n\n{body}")
    };
    Ok(Outcome {
        output,
        stats: engine.stats.clone(),
        unresolved,
        evidence: None,
        reports: std::mem::take(&mut engine.sinks),
        sample: Some(Sampled {
            runs,
            seed,
            effective,
            weight: totals.weight,
            squares: totals.squares,
        }),
    })
}
