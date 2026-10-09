//! Benchmarks: run the models in `benches/` and `examples/` and print what
//! each costs, as a Markdown table (docs/benchmarks.md explains them).
//!
//! ```sh
//! cargo run --release -p probl-bench                # every model
//! cargo run --release -p probl-bench -- tennis      # the models whose name contains "tennis"
//! cargo run --release -p probl-bench -- --quick     # one timed run each
//! cargo run --release -p probl-bench -- --threads=1 # sampling on one thread
//! cargo run --release -p probl-bench -- --no-conjugate # no exact updates for conjugate priors
//! cargo run --release -p probl-bench -- --no-solve   # unroll loops that cycle instead of solving them
//! cargo run --release -p probl-bench -- --json > after.jsonl    # one JSON record per model
//! cargo run --release -p probl-bench -- compare before.jsonl after.jsonl
//! ```
//!
//! For each model: the median time of a few runs, the engine's statistics,
//! the peak heap (from a separate run, with allocations counted), and, for
//! enumerated models, how much work the same run does without merging
//! worlds. Runs stop after a time limit, so models that explode say so
//! instead of hanging.
//!
//! `compare` checks one build against another, from `--json` records: a
//! model whose output or work changed, or that got slower or used more
//! memory than the threshold allows (`--threshold=5`, in percent). A file
//! can hold several runs of the suite; each model's fastest time and
//! smallest heap count.

use probl::__internal::{EngineChecks, engine_checks};
use probl::{Cancel, Data, Date, Limits, LocalFiles, Mode, Options, Outcome, Program};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

// ── Counting allocations ─────────────────────────────────────────────────

/// The system allocator, counting the bytes in use while `COUNTING` is set.
struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static IN_USE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(bytes: usize) {
    let now = IN_USE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() && COUNTING.load(Ordering::Relaxed) {
            grew(layout.size());
        }
        p
    }

    // `fetch_update`'s new name, `try_update`, needs a newer Rust than the
    // crates support (1.85).
    #[allow(deprecated)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        if COUNTING.load(Ordering::Relaxed) {
            // Memory allocated before counting started can be freed while
            // counting: never go below zero.
            let _ = IN_USE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(layout.size()))
            });
        }
    }

    // `fetch_update`'s new name, `try_update`, needs a newer Rust than the
    // crates support (1.85).
    #[allow(deprecated)]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() && COUNTING.load(Ordering::Relaxed) {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                let less = layout.size() - new_size;
                let _ = IN_USE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_sub(less)));
            }
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// ── Running models ───────────────────────────────────────────────────────

/// How long one run may take before it's cancelled.
const TIME_LIMIT: Duration = Duration::from_secs(60);

struct Run {
    time: Duration,
    result: Result<Outcome, String>,
}

/// How the models are run.
#[derive(Clone)]
struct Settings {
    /// One timed run each, instead of five.
    quick: bool,
    /// The most threads that sample at once (all the cores by default).
    threads: Option<usize>,
    /// Update conjugate priors exactly when sampling (the default).
    conjugate: bool,
    /// Solve loops that cycle as Markov chains (the default).
    solve: bool,
    /// The model's data, read once before the runs.
    data: Option<Data>,
}

fn run_once(program: &Program, settings: &Settings, merge: bool, limit: Duration) -> Run {
    let cancel = Cancel::new();
    let mut limits = Limits::default();
    if let Some(n) = settings.threads {
        limits.max_threads = n;
    }
    // The examples' date, so that calendar models give the same output on
    // any day.
    let today = Date::from_ymd(2026, 9, 29).expect("a valid date");
    let mut options = Options::new()
        .cancel(&cancel)
        .limits(limits)
        .conjugate(settings.conjugate)
        .today(today);
    if let Some(data) = &settings.data {
        options = options.data(data.clone());
    }
    let checks = EngineChecks {
        merge,
        solve: settings.solve,
        ..EngineChecks::default()
    };
    let options = engine_checks(options, checks);
    // A watchdog cancels the run at the time limit.
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (cancel, done) = (cancel.clone(), done.clone());
        std::thread::spawn(move || {
            let start = Instant::now();
            while !done.load(Ordering::Relaxed) && start.elapsed() < limit {
                std::thread::sleep(Duration::from_millis(20));
            }
            cancel.cancel();
        })
    };
    let mut print = |_: &str| {};
    let start = Instant::now();
    let result = program.run_with(&options, &mut print);
    let time = start.elapsed();
    done.store(true, Ordering::Relaxed);
    watchdog.join().unwrap();
    Run {
        time,
        result: result.map_err(|e| e.to_string()),
    }
}

struct Measured {
    name: String,
    mode: String,
    time: Duration,
    outcome: Result<Outcome, String>,
    peak_heap: usize,
    unmerged: Option<Result<(Duration, u64), String>>,
}

fn measure(name: &str, program: &Program, settings: &Settings) -> Measured {
    let enumerated = !matches!(program.mode(), Mode::Sample { .. });
    // The heap is counted in a run of its own, which isn't timed: counting
    // slows allocation down, much more so when threads allocate at once.
    IN_USE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let counted = run_once(program, settings, true, TIME_LIMIT);
    COUNTING.store(false, Ordering::Relaxed);
    let peak_heap = PEAK.load(Ordering::Relaxed);
    // A failed run isn't repeated, and neither is a long one.
    let first = match counted.result {
        Ok(_) => run_once(program, settings, true, TIME_LIMIT),
        Err(_) => counted,
    };
    let mut times = vec![first.time];
    if first.result.is_ok() && first.time < Duration::from_secs(5) && !settings.quick {
        for _ in 0..4 {
            times.push(run_once(program, settings, true, TIME_LIMIT).time);
        }
    }
    times.sort();
    let time = times[times.len() / 2];
    let unmerged = (enumerated && first.result.is_ok()).then(|| {
        let limit = (first.time * 20).clamp(Duration::from_secs(1), Duration::from_secs(30));
        let run = run_once(program, settings, false, limit);
        run.result.map(|o| (run.time, o.stats().world_steps()))
    });
    Measured {
        name: name.to_string(),
        mode: if enumerated { "enumerate" } else { "sample" }.to_string(),
        time,
        outcome: first.result,
        peak_heap,
        unmerged,
    }
}

fn models(filter: &[String]) -> Vec<(String, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    for dir in ["benches", "examples"] {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .unwrap_or_else(|e| panic!("can't read {dir}/: {e}"))
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "probl"))
            .collect();
        paths.sort();
        for p in paths {
            let name = format!("{dir}/{}", p.file_stem().unwrap().to_string_lossy());
            if filter.is_empty() || filter.iter().any(|f| name.contains(f.as_str())) {
                out.push((name, p));
            }
        }
    }
    out
}

// ── Printing ─────────────────────────────────────────────────────────────

fn seconds(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 0.001 {
        format!("{:.0} µs", s * 1e6)
    } else if s < 1.0 {
        format!("{:.0} ms", s * 1e3)
    } else {
        format!("{s:.2} s")
    }
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn bytes(n: usize) -> String {
    let n = n as f64;
    if n < 1024.0 * 1024.0 {
        format!("{:.0} KB", n / 1024.0)
    } else if n < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.0} MB", n / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", n / (1024.0 * 1024.0 * 1024.0))
    }
}

/// What the output's summary line says after the mode: unresolved weight,
/// the effective sample size, evidence.
fn note(outcome: &Outcome) -> String {
    let header = outcome.text().lines().next().unwrap_or("");
    let parts: Vec<&str> = header.split(" · ").skip(1).collect();
    parts
        .into_iter()
        .filter(|p| !p.ends_with("runs") && !p.starts_with("seed"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn row(m: &Measured) -> String {
    let (worlds, steps, per_step, calls, note) = match &m.outcome {
        Ok(o) => {
            let s = o.stats();
            let per_step = if s.world_steps() > 0 {
                format!("{:.0} ns", m.time.as_secs_f64() * 1e9 / s.world_steps() as f64)
            } else {
                "".into()
            };
            let calls = if s.calls() > 0 {
                format!(
                    "{} ({:.0}% reused)",
                    count(s.calls()),
                    100.0 * s.reused_calls() as f64 / s.calls() as f64
                )
            } else {
                "".into()
            };
            (
                count(s.peak_worlds() as u64),
                count(s.world_steps()),
                per_step,
                calls,
                note(o),
            )
        }
        Err(e) => ("".into(), "".into(), "".into(), "".into(), format!("**{e}**")),
    };
    let unmerged = match &m.unmerged {
        None => String::new(),
        Some(Ok((time, steps))) => {
            let merged = m.outcome.as_ref().map_or(1, |o| o.stats().world_steps().max(1));
            format!(
                "{} steps ({:.0}×), {}",
                count(*steps),
                *steps as f64 / merged as f64,
                seconds(*time)
            )
        }
        Some(Err(e)) => format!("gave up: {e}"),
    };
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        m.name,
        m.mode,
        seconds(m.time),
        worlds,
        steps,
        per_step,
        calls,
        bytes(m.peak_heap),
        unmerged,
        note
    )
}

/// One model's measurement, as `--json` writes it.
fn record(m: &Measured) -> String {
    let (output, world_steps, peak_worlds) = match &m.outcome {
        Ok(o) => (
            o.text().to_string(),
            o.stats().world_steps(),
            o.stats().peak_worlds() as u64,
        ),
        Err(e) => (format!("error: {e}"), 0, 0),
    };
    serde_json::json!({
        "name": m.name,
        "mode": m.mode,
        "seconds": m.time.as_secs_f64(),
        "peak_heap": m.peak_heap,
        "world_steps": world_steps,
        "peak_worlds": peak_worlds,
        "output": output,
    })
    .to_string()
}

/// What one build did for one model, over the runs in its file.
struct Seen {
    seconds: f64,
    peak_heap: u64,
    world_steps: Vec<u64>,
    outputs: Vec<String>,
}

fn read_records(path: &str) -> BTreeMap<String, Seen> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    for line in text.lines().filter(|l| l.starts_with('{')) {
        let r: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{path}: {e}"));
        let name = r["name"].as_str().unwrap_or_default().to_string();
        let seconds = r["seconds"].as_f64().unwrap_or(f64::NAN);
        let peak_heap = r["peak_heap"].as_u64().unwrap_or(0);
        let steps = r["world_steps"].as_u64().unwrap_or(0);
        let output = r["output"].as_str().unwrap_or_default().to_string();
        let s = seen.entry(name).or_insert(Seen {
            seconds,
            peak_heap,
            world_steps: Vec::new(),
            outputs: Vec::new(),
        });
        s.seconds = s.seconds.min(seconds);
        s.peak_heap = s.peak_heap.min(peak_heap);
        if !s.world_steps.contains(&steps) {
            s.world_steps.push(steps);
        }
        if !s.outputs.contains(&output) {
            s.outputs.push(output);
        }
    }
    seen
}

fn change(before: f64, after: f64) -> String {
    format!("{:+.1}%", (after / before - 1.0) * 100.0)
}

/// Compare two builds' records: exit with failure if a model's output or
/// work changed, or it got slower or bigger than `threshold` percent.
fn compare(before: &str, after: &str, threshold: f64) -> std::process::ExitCode {
    let (before, after) = (read_records(before), read_records(after));
    let limit = 1.0 + threshold / 100.0;
    println!("| model | time before | after | change | peak heap before | after | change | |");
    println!("|---|--:|--:|--:|--:|--:|--:|---|");
    let mut problems = 0;
    let (mut total_before, mut total_after) = (0.0, 0.0);
    for (name, b) in &before {
        let Some(a) = after.get(name) else {
            println!("| {name} | | | | | | | **missing after** |");
            problems += 1;
            continue;
        };
        let mut flags = Vec::new();
        if b.outputs.len() > 1 || a.outputs.len() > 1 {
            flags.push("output varies between runs");
        } else if b.outputs != a.outputs {
            flags.push("**output differs**");
        }
        if b.world_steps != a.world_steps {
            flags.push("**work differs**");
        }
        // Small differences are noise, whatever their share.
        if a.seconds > b.seconds * limit && a.seconds - b.seconds > 0.002 {
            flags.push("**slower**");
        }
        if a.peak_heap as f64 > b.peak_heap as f64 * limit && a.peak_heap - b.peak_heap > 64 * 1024 {
            flags.push("**more memory**");
        }
        problems += flags.iter().filter(|f| f.starts_with("**")).count();
        total_before += b.seconds;
        total_after += a.seconds;
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} |",
            seconds(Duration::from_secs_f64(b.seconds)),
            seconds(Duration::from_secs_f64(a.seconds)),
            change(b.seconds, a.seconds),
            bytes(b.peak_heap as usize),
            bytes(a.peak_heap as usize),
            change(b.peak_heap.max(1) as f64, a.peak_heap.max(1) as f64),
            flags.join(", ")
        );
    }
    for name in after.keys().filter(|n| !before.contains_key(*n)) {
        println!("| {name} | | | | | | | new |");
    }
    println!(
        "\nTotal time {} → {} ({}); {problems} problem(s) at a {threshold}% threshold.",
        seconds(Duration::from_secs_f64(total_before)),
        seconds(Duration::from_secs_f64(total_after)),
        change(total_before, total_after)
    );
    if problems == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("compare") {
        let files: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();
        let threshold = args
            .iter()
            .find_map(|a| a.strip_prefix("--threshold="))
            .map_or(5.0, |t| t.parse().expect("--threshold= needs a number"));
        let [before, after] = files[..] else {
            panic!("compare needs two files: before.jsonl after.jsonl");
        };
        return compare(before, after, threshold);
    }
    let json = args.iter().any(|a| a == "--json");
    let mut settings = Settings {
        quick: args.iter().any(|a| a == "--quick"),
        threads: args.iter().find_map(|a| a.strip_prefix("--threads=")).map(|n| {
            n.parse()
                .unwrap_or_else(|_| panic!("--threads= needs a number, not {n:?}"))
        }),
        conjugate: !args.iter().any(|a| a == "--no-conjugate"),
        solve: !args.iter().any(|a| a == "--no-solve"),
        data: None,
    };
    let filter: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();
    if !json {
        println!(
            "| model | mode | time | peak worlds | world-steps | per step | calls | peak heap | without merging | notes |"
        );
        println!("|---|---|--:|--:|--:|--:|--:|--:|---|---|");
    }
    for (name, path) in models(&filter) {
        let src = std::fs::read_to_string(&path).unwrap();
        let Ok(program) = probl::compile(&name, &src) else {
            println!("| {name} | | | | | | | | | **doesn't compile** |");
            continue;
        };
        settings.data = None;
        if program.reads_data() {
            match program.load(&mut LocalFiles::next_to(&path), &Options::new()) {
                Ok(data) => settings.data = Some(data),
                Err(e) => {
                    println!("| {name} | | | | | | | | | **can't read its data: {e}** |");
                    continue;
                }
            }
        }
        let m = measure(&name, &program, &settings);
        println!("{}", if json { record(&m) } else { row(&m) });
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    /// The benchmark models must keep compiling as the language changes.
    #[test]
    fn every_model_compiles() {
        for (name, path) in super::models(&[]) {
            let src = std::fs::read_to_string(&path).unwrap();
            if let Err(e) = probl::compile(&name, &src) {
                panic!("{name} doesn't compile:\n{}", e.render(false));
            }
        }
    }
}
