//! The Probl engine: values, distributions and the world-set interpreter.
//! The rules it implements are in docs/semantics.md.

pub mod builtins;
pub mod continuous;
pub mod data;
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

use continuous::Rng;
use dist::Budget;
use error::OpError;
use probl_sema::Liveness;
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
            max_threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
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
    /// The program's data, loaded with [`data::load`]. A program that reads
    /// data can't run without it.
    pub inputs: Option<Arc<data::Inputs>>,
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
            inputs: None,
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
    /// The data the program read, if any.
    pub data: Vec<data::SourceInfo>,
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
        handle
            .join()
            .unwrap_or_else(|panic| Err(internal(panic_detail(&*panic))))
    })
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
            shared: None,
        },
        cancel: options.cancel.clone(),
        sample_seed: sample.map(|(_, seed)| seed),
    };
    let epsilon = config.epsilon;
    let inputs = inputs(program, options)?;
    let live = probl_sema::analyze(program);
    if let Some((runs, seed)) = sample {
        return sampled(program, &live, config, options, print, runs, seed);
    }
    let mut engine = interp::Engine::new(program, &live, config, inputs, print);
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
        data: sources(options),
    })
}

/// The values of the program's inputs: loaded, and loaded for it.
fn inputs<'a>(program: &Program, options: &'a Options) -> Result<&'a [Value], RuntimeError> {
    let Some(first) = program.inputs.first() else {
        return Ok(&[]);
    };
    match &options.inputs {
        Some(inputs) if inputs.fit(program) => Ok(inputs.values()),
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
        self.unresolved += batch.unresolved;
        self.last_ruling_out = batch.last_ruling_out.or(self.last_ruling_out);
        self.stats.peak_worlds = self.stats.peak_worlds.max(batch.stats.peak_worlds);
        self.stats.world_steps += batch.stats.world_steps;
        self.stats.calls += batch.stats.calls;
        self.stats.memo_hits += batch.stats.memo_hits;
    }
}

/// A batch's result as a worker sends it: its number, what it produced (or
/// its error), and what it printed.
type Sent = (u64, Result<interp::Batch, RuntimeError>, interp::Printed);

/// Run the batches of a sampled program on up to `max_threads` threads, and
/// combine them in batch order: what's printed, the first error, and every
/// sum come out the same whatever the number of threads. The threads share
/// one budget of work.
fn run_batches(
    program: &Program,
    live: &Liveness,
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
                let mut engine = interp::Engine::new(program, live, config.clone(), &copied, &mut ignore);
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
            *combined_upto.lock().unwrap_or_else(PoisonError::into_inner) = index + 1;
            caught_up.notify_all();
        }
        Ok(combined)
    })
}

/// Sample the program and print its estimates (docs/semantics.md, section 14).
fn sampled(
    program: &Program,
    live: &Liveness,
    config: interp::Config,
    options: &Options,
    print: &mut (dyn FnMut(&str) + Send),
    runs: u64,
    seed: u64,
) -> Result<Outcome, RuntimeError> {
    let mut engine = run_batches(program, live, &config, options, print, runs, seed)?;
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
        data: sources(options),
    })
}
