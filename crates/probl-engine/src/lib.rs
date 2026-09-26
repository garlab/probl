//! The Probl engine: values, distributions and the world-set interpreter.

pub mod builtins;
pub mod dates;
pub mod dist;
pub mod error;
pub mod interp;
pub mod ops;
pub mod report;
pub mod value;
pub mod world;

pub use error::RuntimeError;
pub use interp::Stats;

use probl_sema::ir::{Mode, Program};
use report::{Format, pct};

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
    /// Also print probabilities as fractions.
    pub fractions: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            merge: true,
            memoize: true,
            epsilon: None,
            fractions: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    /// The summary line and the reports, as printed by `probl run`.
    pub output: String,
    pub stats: Stats,
    /// Probability left unaccounted for.
    pub unresolved: f64,
    /// The probability of the evidence, if the program observed anything.
    pub evidence: Option<f64>,
    /// What each `report` collected, in the order of `Program::reports`.
    pub reports: Vec<report::Sink>,
}

/// Engine threads get a large stack so deeply recursive programs don't
/// overflow it.
const STACK_SIZE: usize = 512 * 1024 * 1024;

/// Run a program. `print` receives the output of `print(…)` as it happens.
pub fn run(
    program: &Program,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
) -> Result<Outcome, RuntimeError> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("probl-engine".into())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || run_here(program, options, print))
            .expect("failed to start the engine thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn run_here(
    program: &Program,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
) -> Result<Outcome, RuntimeError> {
    let settings = &program.settings;
    match settings.mode {
        Mode::Auto | Mode::Exact => {}
        ref other => {
            return Err(error::OpError::unsupported(format!("`{}` mode isn't implemented yet", other.name()))
                .help("sample, particles and beam modes arrive in v0.2; for now remove the `@mode` line to run exactly")
                .at(settings.mode_span.unwrap_or_default()));
        }
    }
    let live = probl_sema::analyze(program);
    let epsilon = options.epsilon.unwrap_or(settings.epsilon);
    let mut engine = interp::Engine::new(program, &live, epsilon, options.merge, options.memoize, print);
    let finished = engine.run_main()?;

    let format = Format {
        fractions: options.fractions,
    };
    let mut header = String::from("exact");
    let evidence = engine.observed.then_some(finished);
    if let Some(e) = evidence {
        header.push_str(&format!(" · evidence {}", pct(e, format)));
    }
    let unresolved = engine.unresolved;
    if unresolved > 0.0 {
        if unresolved <= epsilon {
            header.push_str(&format!(" · unresolved < {epsilon:e}"));
        } else {
            header.push_str(&format!(" · unresolved {unresolved:.1e}"));
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
    })
}
