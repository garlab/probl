//! How a program runs: its options and limits, cancelling it, and dates.

use crate::__internal::EngineChecks;
use crate::files::Data;
use probl_engine::data::InputLimits;
use probl_sema::ir;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How to run a program. Every setting is optional:
///
/// ```
/// let options = probl::Options::new().runs(50_000).seed(7);
/// ```
#[derive(Clone)]
pub struct Options {
    choice: Option<Choice>,
    runs: Option<u64>,
    seed: Option<u64>,
    today: Option<Date>,
    epsilon: Option<f64>,
    fractions: bool,
    conjugate: bool,
    pub(crate) limits: Limits,
    pub(crate) data: Option<Data>,
    pub(crate) cancel: Option<Cancel>,
    progress: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    pub(crate) checks: EngineChecks,
}

/// What `enumerate` and `sample` asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Choice {
    Enumerate,
    Sample,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            choice: None,
            runs: None,
            seed: None,
            today: None,
            epsilon: None,
            fractions: false,
            conjugate: true,
            limits: Limits::default(),
            data: None,
            cancel: None,
            progress: None,
            checks: EngineChecks::default(),
        }
    }
}

impl Options {
    pub fn new() -> Options {
        Options::default()
    }

    /// Enumerate, whatever the program's `@mode` says (`--mode enumerate`).
    pub fn enumerate(mut self) -> Options {
        self.choice = Some(Choice::Enumerate);
        self
    }

    /// Sample (`--mode sample`): with the program's runs and seed, if its
    /// `@mode` samples, and otherwise 10,000 runs and seed 0.
    pub fn sample(mut self) -> Options {
        self.choice = Some(Choice::Sample);
        self
    }

    /// Sample this many runs (`--runs`). It implies sampling, unless
    /// `enumerate` asks otherwise.
    pub fn runs(mut self, n: u64) -> Options {
        self.runs = Some(n);
        self
    }

    /// Sample with this seed (`--seed`). It implies sampling, unless
    /// `enumerate` asks otherwise.
    pub fn seed(mut self, seed: u64) -> Options {
        self.seed = Some(seed);
        self
    }

    /// The date `today` stands for. It's needed only when the program uses
    /// `today`: the engine never reads a clock.
    pub fn today(mut self, date: Date) -> Options {
        self.today = Some(date);
        self
    }

    /// Stop `while` and `loop` once the worlds still inside weigh less than
    /// this share of what entered, instead of the program's `@epsilon`.
    pub fn epsilon(mut self, e: f64) -> Options {
        self.epsilon = Some(e);
        self
    }

    /// Also print the simplest fraction near each probability, in
    /// [`Outcome::text`](crate::Outcome::text).
    pub fn fractions(mut self, on: bool) -> Options {
        self.fractions = on;
        self
    }

    /// When sampling, update conjugate priors exactly (on by default).
    /// Turning it off draws every variable from its prior, for comparing:
    /// the estimates mean the same either way.
    pub fn conjugate(mut self, on: bool) -> Options {
        self.conjugate = on;
        self
    }

    pub fn limits(mut self, limits: Limits) -> Options {
        self.limits = limits;
        self
    }

    /// The program's data, from [`Program::load`](crate::Program::load).
    pub fn data(mut self, data: Data) -> Options {
        self.data = Some(data);
        self
    }

    /// Stop the run, or the loading of data, when `cancel` is cancelled. A
    /// cancelled run is an error of kind [`Limit`](crate::ErrorKind::Limit).
    pub fn cancel(mut self, cancel: &Cancel) -> Options {
        self.cancel = Some(cancel.clone());
        self
    }

    /// When sampling, `f` is told after each batch of runs how many are done,
    /// and how many there are.
    pub fn progress(mut self, f: impl Fn(u64, u64) + Send + Sync + 'static) -> Options {
        self.progress = Some(Arc::new(f));
        self
    }

    /// The mode for a program whose `@mode` is `program`, as the command
    /// line chooses it: `None` keeps the program's.
    fn mode(&self, program: &ir::Mode) -> Option<ir::Mode> {
        if self.choice == Some(Choice::Enumerate) {
            return Some(ir::Mode::Enumerate);
        }
        let sample = self.choice == Some(Choice::Sample) || self.runs.is_some() || self.seed.is_some();
        if !sample {
            return None;
        }
        let (runs, seed) = match program {
            ir::Mode::Sample { runs, seed } => (*runs, *seed),
            _ => (10_000, 0),
        };
        Some(ir::Mode::Sample {
            runs: self.runs.unwrap_or(runs),
            seed: self.seed.unwrap_or(seed),
        })
    }

    /// The engine's options for running `program`, without its data.
    pub(crate) fn engine(&self, program: &ir::Program) -> probl_engine::Options {
        probl_engine::Options {
            today: self.today.map(|d| d.0),
            merge: self.checks.merge,
            memoize: self.checks.memoize,
            epsilon: self.epsilon,
            fractions: self.fractions,
            limits: self.limits.engine(),
            cancel: self.cancel.as_ref().map(|c| c.0.clone()),
            mode: self.mode(&program.settings.mode),
            inputs: None,
            conjugate: self.conjugate,
            solve: self.checks.solve,
            progress: self.progress.clone().map(probl_engine::Progress),
        }
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("choice", &self.choice)
            .field("runs", &self.runs)
            .field("seed", &self.seed)
            .field("today", &self.today)
            .field("epsilon", &self.epsilon)
            .field("fractions", &self.fractions)
            .field("conjugate", &self.conjugate)
            .field("limits", &self.limits)
            .field("data", &self.data.is_some())
            .field("cancel", &self.cancel)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

/// Upper bounds on what a run, and the loading of its data, may do. A
/// program's `@max_worlds` and `@max_iterations` can lower them, never raise
/// them. The default is the command line's; change a field to change one:
///
/// ```
/// let mut limits = probl::Limits::default();
/// limits.max_worlds = 10_000;
/// ```
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Bits in one integer, in the program and in its data.
    pub max_integer_bits: u64,
    /// Bytes of large integers made, in all (shared copies included), in the
    /// program and in its data.
    pub max_integer_bytes: u64,
    /// Bytes in one string.
    pub max_string_bytes: usize,
    /// Bytes of strings made by the run, in all.
    pub max_string_alloc_bytes: u64,
    /// Worlds one statement may produce.
    pub max_worlds: usize,
    /// Outcomes of one distribution, or of combining distributions.
    pub max_outcomes: usize,
    /// Elements of one collection, in the program and in its data.
    pub max_collection: usize,
    /// Units of work: world-steps plus outcomes computed.
    pub max_work: u64,
    /// Iterations of one loop.
    pub max_iterations: u64,
    /// Nested calls.
    pub max_call_depth: usize,
    /// Function results kept for reuse (the cache is cleared when full).
    pub max_cached_calls: usize,
    /// Bytes printed by `print`.
    pub max_output: usize,
    /// Stack size of the thread a run starts (not on `wasm32`, where it runs
    /// on the calling thread). With `max_call_depth`, it bounds how deep
    /// calls nest.
    pub stack_size: usize,
    /// Threads that sample batches of runs at the same time. The results
    /// don't depend on it.
    pub max_threads: usize,
    /// States of a loop solved as a Markov chain; a loop with more is
    /// unrolled instead.
    pub max_chain_states: usize,
    /// Bytes of data read, all the inputs together.
    pub max_input_bytes: u64,
    /// Values made from the data: every number, string, record and element.
    pub max_input_values: u64,
    /// How deeply JSON arrays and objects may nest.
    pub max_input_depth: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::from_engine(probl_engine::Limits::default(), InputLimits::default())
    }
}

impl Limits {
    /// The playground's limits: less memory, a smaller stack, and one
    /// thread, for a browser tab.
    pub fn browser() -> Limits {
        let engine = probl_engine::Limits {
            max_integer_bytes: 64 * 1024 * 1024,
            max_string_bytes: 4 * 1024 * 1024,
            max_string_alloc_bytes: 64 * 1024 * 1024,
            max_worlds: 1_000_000,
            max_outcomes: 1_000_000,
            max_collection: 1_000_000,
            max_work: 2_000_000_000,
            max_call_depth: 150,
            max_cached_calls: 200_000,
            max_output: 1024 * 1024,
            max_threads: 1,
            max_chain_states: 20_000,
            ..probl_engine::Limits::default()
        };
        let input = InputLimits {
            max_bytes: 8 * 1024 * 1024,
            max_values: 1_000_000,
            ..InputLimits::default()
        };
        Limits::from_engine(engine, input)
    }

    /// The engine's limits and the data's; the data's integers and
    /// collections are limited as the program's are.
    fn from_engine(engine: probl_engine::Limits, input: InputLimits) -> Limits {
        Limits {
            max_integer_bits: engine.max_integer_bits,
            max_integer_bytes: engine.max_integer_bytes,
            max_string_bytes: engine.max_string_bytes,
            max_string_alloc_bytes: engine.max_string_alloc_bytes,
            max_worlds: engine.max_worlds,
            max_outcomes: engine.max_outcomes,
            max_collection: engine.max_collection,
            max_work: engine.max_work,
            max_iterations: engine.max_iterations,
            max_call_depth: engine.max_call_depth,
            max_cached_calls: engine.max_cached_calls,
            max_output: engine.max_output,
            stack_size: engine.stack_size,
            max_threads: engine.max_threads,
            max_chain_states: engine.max_chain_states,
            max_input_bytes: input.max_bytes,
            max_input_values: input.max_values,
            max_input_depth: input.max_depth,
        }
    }

    pub(crate) fn engine(&self) -> probl_engine::Limits {
        probl_engine::Limits {
            max_integer_bits: self.max_integer_bits,
            max_integer_bytes: self.max_integer_bytes,
            max_string_bytes: self.max_string_bytes,
            max_string_alloc_bytes: self.max_string_alloc_bytes,
            max_worlds: self.max_worlds,
            max_outcomes: self.max_outcomes,
            max_collection: self.max_collection,
            max_work: self.max_work,
            max_iterations: self.max_iterations,
            max_call_depth: self.max_call_depth,
            max_cached_calls: self.max_cached_calls,
            max_output: self.max_output,
            stack_size: self.stack_size,
            max_threads: self.max_threads.max(1),
            max_chain_states: self.max_chain_states,
        }
    }

    pub(crate) fn input(&self) -> InputLimits {
        InputLimits {
            max_integer_bits: self.max_integer_bits,
            max_integer_bytes: self.max_integer_bytes,
            max_bytes: self.max_input_bytes,
            max_values: self.max_input_values,
            max_collection: self.max_collection,
            max_depth: self.max_input_depth,
        }
    }
}

/// Stops a run, or the loading of data, from another thread. Clones share
/// the same switch.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

/// A calendar date, from 0001-01-01 to 9999-12-31. It prints, and parses,
/// as `YYYY-MM-DD`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i32);

impl Date {
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Date> {
        probl_engine::dates::from_parts(year.into(), month.into(), day.into()).map(Date)
    }

    /// Today in UTC, from the system clock; `None` if the clock is outside
    /// the dates Probl supports. Not on `wasm32`, which has no clock: the
    /// host gives the date there.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn today_utc() -> Option<Date> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i128::from(d.as_secs()),
            Err(e) => -i128::from(e.duration().as_secs()) - i128::from(e.duration().subsec_nanos() != 0),
        };
        i64::try_from(seconds)
            .ok()
            .and_then(probl_engine::dates::from_unix_seconds)
            .map(Date)
    }

    pub(crate) fn from_days(days: i32) -> Date {
        Date(days)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&probl_engine::dates::format(self.0))
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Date({self})")
    }
}

impl FromStr for Date {
    type Err = ParseDateError;

    fn from_str(s: &str) -> Result<Date, ParseDateError> {
        probl_engine::dates::parse(s).map(Date).ok_or(ParseDateError)
    }
}

/// A date that isn't `YYYY-MM-DD`, or is out of range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseDateError;

impl fmt::Display for ParseDateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected YYYY-MM-DD within 0001-01-01..9999-12-31")
    }
}

impl std::error::Error for ParseDateError {}
