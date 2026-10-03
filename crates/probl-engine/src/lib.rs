//! The Probl engine: values, distributions and the world-set interpreter.
//! The rules it implements are in docs/semantics.md.

pub mod analytic;
pub mod builtins;
pub mod chain;
pub mod complex;
pub mod conjugate;
pub mod continuous;
pub mod data;
pub mod dates;
pub mod dist;
pub mod error;
pub mod interp;
mod math;
pub mod ops;
pub mod report;
mod stats;
mod text;
mod type_name;
pub mod value;
pub mod weight;
pub mod world;

pub use error::{ErrorKind, RuntimeError};
pub use interp::{Stats, Updates};
pub use weight::Weight;

use continuous::Rng;
use dist::Budget;
use error::OpError;
use probl_sema::Liveness;
use probl_sema::conjugate::{Conjugacy, Variable};
use probl_sema::ir::{Mode, Program};
use probl_syntax::Span;
use report::{Format, Sink};
use std::any::Any;
use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use value::Value;

/// Upper bounds set by whoever runs a program. A program's `@max_worlds` and
/// `@max_iterations` can lower them, never raise them.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Maximum bits per integer; cannot exceed the parser's hard ceiling.
    pub max_integer_bits: u64,
    /// Cumulative allowance for large integer results (including shared copies).
    pub max_integer_bytes: u64,
    /// Maximum UTF-8 bytes in one string.
    pub max_string_bytes: usize,
    /// Cumulative UTF-8 payload bytes produced by the runtime (not live memory).
    pub max_string_alloc_bytes: u64,
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
    /// Threads that sample batches of runs at the same time. The output
    /// doesn't depend on it.
    pub max_threads: usize,
    /// States of a loop solved as a Markov chain; a loop with more is
    /// unrolled instead.
    pub max_chain_states: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_integer_bits: probl_number::MAX_INTEGER_BITS,
            max_integer_bytes: 256 * 1024 * 1024,
            max_string_bytes: 16 * 1024 * 1024,
            max_string_alloc_bytes: 256 * 1024 * 1024,
            max_worlds: 10_000_000,
            max_outcomes: 2_000_000,
            max_collection: 10_000_000,
            max_work: 20_000_000_000,
            max_iterations: 10_000_000,
            max_call_depth: 500,
            max_cached_calls: 1_000_000,
            max_output: 64 * 1024 * 1024,
            stack_size: 64 * 1024 * 1024,
            max_threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
            max_chain_states: 50_000,
        }
    }
}

/// How the engine runs. The defaults are what `probl run` uses, except that
/// the CLI also supplies an execution-date snapshot.
#[derive(Clone, Debug)]
pub struct Options {
    /// Execution-date snapshot, as days since 1970-01-01. The host captures
    /// it once or supplies a replay date. The engine never reads a clock.
    /// Required only when the program evaluates the `today` constant.
    pub today: Option<i32>,
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
    /// The program's data, loaded with [`data::load`]. A program that reads
    /// data can't run without it.
    pub inputs: Option<Arc<data::Inputs>>,
    /// When sampling, update conjugate priors exactly (docs/semantics.md,
    /// section 14). Turning it off draws every variable from its prior, for
    /// comparing: the estimates mean the same either way.
    pub conjugate: bool,
    /// When enumerating, solve loops that cycle as Markov chains
    /// (docs/semantics.md, section 10). Turning it off unrolls them, which
    /// is only useful for checking the engine.
    pub solve: bool,
    /// When sampling, told after each batch how many runs are done, and how
    /// many there are.
    pub progress: Option<Progress>,
}

/// A function told how many runs are done, and how many there are.
#[derive(Clone)]
pub struct Progress(pub Arc<dyn Fn(u64, u64) + Send + Sync>);

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Progress")
    }
}

impl Default for Options {
    fn default() -> Options {
        Options {
            today: None,
            merge: true,
            memoize: true,
            epsilon: None,
            fractions: false,
            limits: Limits::default(),
            cancel: None,
            mode: None,
            inputs: None,
            conjugate: true,
            solve: true,
            progress: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    /// The execution-date snapshot supplied by the host, for reproducibility.
    pub today: Option<i32>,
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
    /// The data the program read, if any.
    pub data: Vec<data::SourceInfo>,
    /// The variables whose draws could be delayed for exact updates when
    /// sampling, and what happened to them.
    pub updates: Vec<(Variable, Updates)>,
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
    /// The standard error of the evidence estimate (`Outcome::evidence`),
    /// relative to it.
    pub evidence_se: f64,
    /// Whether an observation used a density, which makes the evidence a
    /// density too.
    pub densities: bool,
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
        handle
            .join()
            .unwrap_or_else(|panic| Err(internal(panic_detail(&*panic))))
    })
}

/// Run a program on the calling thread, as [`run`] does on a thread of its
/// own. The calling thread's stack must be large enough for the program's
/// calls (`Limits::stack_size` doesn't apply), and a panic isn't caught.
/// With one thread allowed (`Limits::max_threads`), sampling starts no
/// thread either, so this works where threads aren't available, such as in
/// WebAssembly.
pub fn run_on_this_thread(
    program: &Program,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
) -> Result<Outcome, RuntimeError> {
    run_here(program, options, print)
}

/// What a panic said.
fn panic_detail(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "no details".to_string())
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
    if options.today.is_some_and(|d| !dates::valid(d)) {
        return Err(RuntimeError::new(
            Default::default(),
            "execution date must be within 0001-01-01..9999-12-31",
        ));
    }
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
        today: options.today,
        epsilon: options.epsilon.unwrap_or(settings.epsilon),
        merging: options.merge,
        memoizing: options.memoize,
        max_worlds: limits.max_worlds.min(settings.max_worlds),
        max_iterations: limits.max_iterations.min(settings.max_iterations),
        max_call_depth: limits.max_call_depth,
        max_cached_calls: limits.max_cached_calls,
        max_output: limits.max_output,
        budget: Budget {
            max_integer_bits: limits.max_integer_bits,
            integer_bytes_left: Arc::new(AtomicU64::new(limits.max_integer_bytes)),
            max_string_bytes: limits.max_string_bytes,
            string_bytes_left: Arc::new(AtomicU64::new(limits.max_string_alloc_bytes)),
            cancel: options.cancel.clone(),
            max_outcomes: limits.max_outcomes,
            max_collection: limits.max_collection,
            work_left: limits.max_work,
            shared: None,
        },
        cancel: options.cancel.clone(),
        sample_seed: sample.map(|(_, seed)| seed),
        conjugate: options.conjugate,
        solving: options.solve,
        max_chain_states: limits.max_chain_states,
    };
    let epsilon = config.epsilon;
    let inputs = inputs(program, options)?;
    let live = probl_sema::analyze(program);
    let conj = probl_sema::conjugate::analyze(program);
    if let Some((runs, seed)) = sample {
        return sampled(program, &live, &conj, config, options, print, runs, seed);
    }
    let mut engine = interp::Engine::new(program, &live, &conj, config, inputs, print);
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
        weighted: program.main().effects.observes,
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
            header.push_str(&format!(" · evidence {}", scientific(z)));
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
        today: options.today,
        output,
        stats: engine.stats.clone(),
        unresolved,
        evidence,
        reports: std::mem::take(&mut engine.sinks),
        sample: None,
        data: sources(options),
        updates: Vec::new(),
    })
}

/// A weight too small for a percentage, in scientific notation: `2.9e-25`.
fn scientific(w: Weight) -> String {
    let l = w.log10();
    let exponent = l.floor();
    let mantissa = libm::pow(10.0, l - exponent);
    // Rounding the mantissa can make it 10.0.
    let (mantissa, exponent) = if format!("{mantissa:.2}") == "10.00" {
        (1.0, exponent + 1.0)
    } else {
        (mantissa, exponent)
    };
    format!("{mantissa:.2}e{exponent}")
}

/// The sampled estimate of the evidence, for the summary line (section 14):
/// a probability, or the logarithm of a density.
fn evidence_estimate(z: Weight, relative_se: f64, densities: bool) -> String {
    let known = relative_se.is_finite();
    if densities {
        let ln = z.log10() * std::f64::consts::LN_10;
        if !known {
            return format!("log evidence {ln:.2}");
        }
        let decimals = if relative_se > 0.0 {
            (-libm::log10(relative_se).floor()).clamp(2.0, 6.0) as usize
        } else {
            2
        };
        return format!("log evidence {ln:.decimals$} ± {relative_se:.decimals$}");
    }
    let x = z.to_f64();
    if x >= 1e-4 {
        return match known {
            true => format!("evidence {}", report::estimate(x, x * relative_se)),
            false => format!("evidence {}", value::fmt_prob(x)),
        };
    }
    if !known {
        return format!("evidence {}", scientific(z));
    }
    let pct = relative_se * 100.0;
    let decimals = if pct >= 10.0 {
        0
    } else if pct >= 1.0 {
        1
    } else {
        2
    };
    format!("evidence {} (± {pct:.decimals$}%)", scientific(z))
}

/// The values of the program's inputs: loaded, and loaded for it.
fn inputs<'a>(program: &Program, options: &'a Options) -> Result<&'a [Value], RuntimeError> {
    let Some(first) = program.inputs.first() else {
        return Ok(&[]);
    };
    match &options.inputs {
        Some(inputs) if inputs.fit(program) => {
            if inputs.max_string_bytes() > options.limits.max_string_bytes {
                return Err(RuntimeError::limit(
                    first.span,
                    "an input string exceeds the runtime string size limit",
                ));
            }
            if inputs.max_integer_bits() > options.limits.max_integer_bits {
                return Err(RuntimeError::limit(
                    first.span,
                    "an input integer exceeds the runtime integer size limit",
                ));
            }
            Ok(inputs.values())
        }
        Some(_) => Err(internal("the data was loaded for a different program".to_string())),
        None => Err(
            RuntimeError::new(first.span, "the program reads data, which wasn't loaded")
                .with_help("load it with `probl_engine::data::load` before running the program"),
        ),
    }
}

fn sources(options: &Options) -> Vec<data::SourceInfo> {
    options.inputs.as_ref().map_or_else(Vec::new, |i| i.sources().to_vec())
}

/// What the batches produced so far, added up in batch order.
struct Combined {
    sinks: Vec<Sink>,
    totals: interp::SampleTotals,
    observed: bool,
    densities: bool,
    unresolved: Weight,
    last_ruling_out: Option<Span>,
    stats: Stats,
}

impl Combined {
    fn new(reports: usize) -> Combined {
        Combined {
            sinks: vec![Sink::default(); reports],
            totals: interp::SampleTotals {
                weight: Weight::ZERO,
                squares: Weight::ZERO,
            },
            observed: false,
            densities: false,
            unresolved: Weight::ZERO,
            last_ruling_out: None,
            stats: Stats::default(),
        }
    }

    fn absorb(&mut self, batch: interp::Batch) {
        for (sink, theirs) in self.sinks.iter_mut().zip(batch.sinks) {
            sink.absorb(theirs);
        }
        self.totals.weight += batch.totals.weight;
        self.totals.squares += batch.totals.squares;
        self.observed |= batch.observed;
        self.densities |= batch.densities;
        self.unresolved += batch.unresolved;
        self.last_ruling_out = batch.last_ruling_out.or(self.last_ruling_out);
        self.stats.absorb(&batch.stats);
    }
}

/// A batch's result as a worker sends it: its number, what it produced (or
/// its error), and what it printed.
type Sent = (u64, Result<interp::Batch, RuntimeError>, interp::Printed);

/// Run the batches of a sampled program on up to `max_threads` threads, and
/// combine them in batch order: what's printed, the first error, and every
/// sum come out the same whatever the number of threads. The threads share
/// one budget of work.
#[allow(clippy::too_many_arguments)]
fn run_batches(
    program: &Program,
    live: &Liveness,
    conj: &Conjugacy,
    config: &interp::Config,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
    runs: u64,
    seed: u64,
) -> Result<Combined, RuntimeError> {
    let batches = runs.div_ceil(interp::BATCH);
    let threads = (options.limits.max_threads.max(1) as u64).min(batches);
    let inputs = inputs(program, options)?;
    // How far past the batch being combined the threads may go: it bounds
    // the finished batches waiting in memory.
    let ahead = 2 * threads;
    let mut config = config.clone();
    config.budget.shared = Some(Arc::new(AtomicU64::new(config.budget.work_left)));
    config.budget.work_left = 0;
    let config = &config;
    if threads == 1 {
        return run_batches_here(program, live, conj, config, inputs, options, print, runs, seed);
    }

    let next = AtomicU64::new(0);
    // The lowest batch that failed: no batch after it needs to run.
    let failed = AtomicU64::new(u64::MAX);
    // Bytes printed so far, in batch order.
    let printed = AtomicUsize::new(0);
    // Batches combined so far, which the threads wait for when too far ahead.
    let combined_upto = Mutex::new(0u64);
    let caught_up = Condvar::new();
    let stop = |error: RuntimeError| {
        // With the lock held, so that no thread misses the wake-up.
        let _upto = combined_upto.lock().unwrap_or_else(PoisonError::into_inner);
        failed.store(0, Ordering::Relaxed);
        caught_up.notify_all();
        Err(error)
    };
    let (tx, rx) = mpsc::channel::<Sent>();
    std::thread::scope(|scope| {
        for t in 0..threads {
            let tx = tx.clone();
            let (next, failed, printed) = (&next, &failed, &printed);
            let (combined_upto, caught_up) = (&combined_upto, &caught_up);
            let worker = move || {
                let mut ignore = |_: &str| {};
                // The data, copied: threads sharing it would all touch its
                // reference counts, and slow each other down.
                let copied: Vec<Value> = if threads > 1 {
                    inputs.iter().map(Value::unshared).collect()
                } else {
                    inputs.to_vec()
                };
                let mut engine = interp::Engine::new(program, live, conj, config.clone(), &copied, &mut ignore);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= batches {
                        break;
                    }
                    let mut upto = combined_upto.lock().unwrap_or_else(PoisonError::into_inner);
                    while index >= *upto + ahead && index <= failed.load(Ordering::Relaxed) {
                        upto = caught_up.wait(upto).unwrap_or_else(PoisonError::into_inner);
                    }
                    drop(upto);
                    if index > failed.load(Ordering::Relaxed) {
                        break;
                    }
                    let first = index * interp::BATCH;
                    let n = (runs - first).min(interp::BATCH);
                    let before = printed.load(Ordering::Relaxed);
                    let (result, lines) = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        engine.run_batch(Rng::stream(seed, index), first, n, before)
                    }))
                    .unwrap_or_else(|panic| (Err(internal(panic_detail(&*panic))), Vec::new()));
                    let stopped = result.is_err();
                    if stopped {
                        failed.fetch_min(index, Ordering::Relaxed);
                    }
                    if tx.send((index, result, lines)).is_err() || stopped {
                        break;
                    }
                }
            };
            let spawned = std::thread::Builder::new()
                .name("probl-sampler".into())
                .stack_size(options.limits.stack_size)
                .spawn_scoped(scope, worker);
            if let Err(e) = spawned {
                // The threads already started can do the work.
                if t == 0 {
                    return Err(internal(format!("couldn't start a sampling thread: {e}")));
                }
                break;
            }
        }
        drop(tx);

        let mut combined = Combined::new(program.reports.len());
        let mut pending = BTreeMap::new();
        let mut bytes = 0;
        for index in 0..batches {
            let (result, lines) = loop {
                if let Some(sent) = pending.remove(&index) {
                    break sent;
                }
                match rx.recv() {
                    Ok((i, result, lines)) => {
                        pending.insert(i, (result, lines));
                    }
                    Err(_) => return stop(internal("a sampling thread stopped before its batch was done".into())),
                }
            };
            for (span, line) in lines {
                bytes += line.len() + 1;
                if bytes > options.limits.max_output {
                    return stop(interp::too_much_output(span));
                }
                print(&line);
            }
            printed.store(bytes, Ordering::Relaxed);
            match result {
                Ok(batch) => combined.absorb(batch),
                Err(e) => return stop(e),
            }
            if let Some(progress) = &options.progress {
                (progress.0)(((index + 1) * interp::BATCH).min(runs), runs);
            }
            *combined_upto.lock().unwrap_or_else(PoisonError::into_inner) = index + 1;
            caught_up.notify_all();
        }
        Ok(combined)
    })
}

/// Run the batches one after another on the calling thread, starting no
/// thread: the same output as on several threads.
#[allow(clippy::too_many_arguments)]
fn run_batches_here(
    program: &Program,
    live: &Liveness,
    conj: &Conjugacy,
    config: &interp::Config,
    inputs: &[Value],
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
    runs: u64,
    seed: u64,
) -> Result<Combined, RuntimeError> {
    let mut ignore = |_: &str| {};
    let mut engine = interp::Engine::new(program, live, conj, config.clone(), inputs, &mut ignore);
    let mut combined = Combined::new(program.reports.len());
    let mut bytes = 0;
    for index in 0..runs.div_ceil(interp::BATCH) {
        let first = index * interp::BATCH;
        let n = (runs - first).min(interp::BATCH);
        let (result, lines) = engine.run_batch(Rng::stream(seed, index), first, n, bytes);
        for (span, line) in lines {
            bytes += line.len() + 1;
            if bytes > options.limits.max_output {
                return Err(interp::too_much_output(span));
            }
            print(&line);
        }
        combined.absorb(result?);
        if let Some(progress) = &options.progress {
            (progress.0)(first + n, runs);
        }
    }
    Ok(combined)
}

/// Sample the program and print its estimates (docs/semantics.md, section 14).
#[allow(clippy::too_many_arguments)]
fn sampled(
    program: &Program,
    live: &Liveness,
    conj: &Conjugacy,
    config: interp::Config,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
    runs: u64,
    seed: u64,
) -> Result<Outcome, RuntimeError> {
    let mut engine = run_batches(program, live, conj, &config, options, print, runs, seed)?;
    let totals = engine.totals;
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
    // The evidence: the average final weight, and its standard error
    // relative to it (section 14).
    let evidence = totals.weight.scale(1.0 / runs as f64);
    let n = runs as f64;
    let evidence_se = if runs > 1 {
        ((n / effective - 1.0).max(0.0) / (n - 1.0)).sqrt()
    } else {
        f64::NAN
    };
    if engine.observed {
        header.push_str(&format!(
            " · {}",
            evidence_estimate(evidence, evidence_se, engine.densities)
        ));
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
        weighted: program.main().effects.observes,
    };
    let body = report::render(program, &engine.sinks, format);
    let output = if body.is_empty() {
        header
    } else {
        format!("{header}\n\n{body}")
    };
    Ok(Outcome {
        today: options.today,
        output,
        stats: engine.stats.clone(),
        unresolved,
        evidence: engine.observed.then_some(evidence),
        reports: std::mem::take(&mut engine.sinks),
        sample: Some(Sampled {
            runs,
            seed,
            effective,
            weight: totals.weight,
            squares: totals.squares,
            evidence_se,
            densities: engine.densities,
        }),
        data: sources(options),
        updates: conj
            .variables
            .iter()
            .enumerate()
            .map(|(i, v)| (v.clone(), engine.stats.updates.get(i).copied().unwrap_or_default()))
            .collect(),
    })
}
