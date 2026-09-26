//! No input may crash Probl (docs/semantics.md, section 11; audit finding
//! I4). Examples and generated programs are mutated at random, then parsed,
//! compiled, run under small limits, and their diagnostics and errors
//! rendered. Every problem must come back as a diagnostic or an error:
//! never as a panic, a hang, or an internal error.
//!
//! `PROBL_FUZZ_CASES` sets the number of cases (default 300) and
//! `PROBL_FUZZ_SEED` the first seed (default 1).

use probl_engine::{ErrorKind, Limits, Options};
use probl_oracle::generate::{self, Rng};
use probl_syntax::{SourceFile, render_all};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Pieces to insert: punctuation, keywords, and values at the edges.
const FRAGMENTS: &[&str] = &[
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ",",
    ";",
    "\n",
    ":",
    ".",
    "..",
    "..<",
    "=>",
    "->",
    "~",
    "=",
    "+=",
    "|",
    "#",
    "\"",
    "{x}",
    "\\",
    "if ",
    "else ",
    "while ",
    "loop ",
    "repeat ",
    "for x in ",
    "match x ",
    "chance ",
    "simulate ",
    "fn f(a) ",
    "return ",
    "break",
    "continue",
    "observe ",
    "report ",
    " by ",
    " as \"x\"",
    "let x = ",
    "var y ~ ",
    "and ",
    "or ",
    "not ",
    " in ",
    " from ",
    "take()",
    ".push(1)",
    " with { a: 1 }",
    "x -> x",
    "type T = { a: int }\n",
    "enum E { A, B }\n",
    ": int",
    ": dist[prob]",
    "d6",
    "d0",
    "0d6",
    "1000d1000",
    "d4294967295",
    "one_of([])",
    "bernoulli(2)",
    "poisson(1e300)",
    "geometric(0%)",
    "binomial(-1, 50%)",
    "roll(1, 1)",
    "P(",
    "mean(",
    "2 to 1",
    "normal(0, 1)",
    "date(\"2026-02-30\")",
    "9223372036854775807",
    "-9223372036854775808",
    "18446744073709551616",
    "1e308",
    "1e-320",
    "0.0",
    "-0",
    "0%",
    "100%",
    "1e9%",
    "2 ^ 63",
    "1 / 0",
    "5 mod 0",
    "[1, 2][5]",
    "@mode enumerate\n",
    "@epsilon 0\n",
    "@max_worlds 1\n",
    "@max_iterations 0\n",
    "@mode sample(runs: 0)\n",
    "@mode sample(runs: 50, seed: 1)\n",
    "normal(0, 1)",
    "3% to 7%",
    "0 to 5",
    "normal_range(1, 1)",
    "beta(1e-300, 1e-300)",
    "gamma(1e308, 1e-308)",
    "pert(1, 0, 2)",
    "exponential(0)",
    "uniform(1, 1)",
    "triangular(0, 5, 1)",
    " from normal(0, 1)",
    "poisson(1e18)",
    "binomial(9223372036854775807, 50%)",
    "quantile(",
    "cdf(",
    "pdf(",
    "é",
    "→",
    "🎲",
    "\u{0}",
    "\t",
    "\r\n",
    "\u{feff}",
];

/// Small: crashes happen early, and the examples would otherwise run for
/// seconds each.
fn limits() -> Limits {
    Limits {
        max_worlds: 5_000,
        max_outcomes: 20_000,
        max_collection: 20_000,
        max_work: 100_000,
        max_iterations: 2_000,
        max_call_depth: 100,
        max_cached_calls: 10_000,
        max_output: 64 * 1024,
        ..Limits::default()
    }
}

fn examples() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("the examples directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "probl"))
        .collect();
    paths.sort();
    paths.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect()
}

fn mutate(src: &str, rng: &mut Rng) -> String {
    let mut chars: Vec<char> = src.chars().collect();
    for _ in 0..1 + rng.below(4) {
        let len = chars.len();
        let at = rng.below(len + 1);
        match rng.below(6) {
            0 => {
                let end = (at + 1 + rng.below(12)).min(len);
                chars.drain(at..end);
            }
            1 | 2 => {
                let piece = rng.pick(FRAGMENTS);
                chars.splice(at..at, piece.chars());
            }
            3 => {
                // Repeat a piece, which also nests brackets deeply.
                let end = (at + 1 + rng.below(40)).min(len);
                let piece: Vec<char> = chars[at..end].to_vec();
                let most = if rng.chance(10) { 300 } else { 3 };
                for _ in 0..1 + rng.below(most) {
                    chars.splice(at..at, piece.iter().copied());
                }
            }
            4 => {
                // Replace a number with a large or odd one.
                if let Some(start) = (at..len).find(|&i| chars[i].is_ascii_digit()) {
                    let end = (start..len).find(|&i| !chars[i].is_ascii_digit()).unwrap_or(len);
                    let n = *rng.pick(&[
                        "0",
                        "1",
                        "65",
                        "4294967296",
                        "9223372036854775807",
                        "99999999999999999999",
                    ]);
                    chars.splice(start..end, n.chars());
                }
            }
            _ => {
                // Swap two pieces.
                let a = rng.below(len + 1);
                let (lo, hi) = (at.min(a), at.max(a));
                let span = rng.below(20).min(hi - lo);
                if span > 0 && hi + span <= len {
                    for i in 0..span {
                        chars.swap(lo + i, hi + i);
                    }
                }
            }
        }
    }
    chars.into_iter().collect()
}

/// Text made of random fragments, with no structure at all.
fn noise(rng: &mut Rng) -> String {
    (0..rng.below(60)).map(|_| *rng.pick(FRAGMENTS)).collect()
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "(no message)".into())
}

/// Everything a user can do with a program, which must not crash.
fn exercise(src: &str, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let file = SourceFile::new("fuzz.probl", src);
    let (program, diags) =
        catch_unwind(|| probl_sema::compile(src)).map_err(|p| format!("compiling panicked: {}", panic_message(&*p)))?;
    catch_unwind(AssertUnwindSafe(|| render_all(&diags, &file, false)))
        .map_err(|p| format!("rendering diagnostics panicked: {}", panic_message(&*p)))?;
    let Some(program) = program else {
        return Ok(());
    };
    let options = Options {
        limits: limits(),
        cancel: Some(cancel.clone()),
        ..Options::default()
    };
    let mut print = |_: &str| {};
    match probl_engine::run(&program, &options, &mut print) {
        Ok(_) => Ok(()),
        Err(e) if e.kind == ErrorKind::Internal => Err(format!("the engine crashed: {}", e.notes.join("; "))),
        Err(e) => catch_unwind(AssertUnwindSafe(|| e.to_diagnostic().render(&file, false)))
            .map(|_| ())
            .map_err(|p| format!("rendering a runtime error panicked: {}", panic_message(&*p))),
    }
}

enum Event {
    Start(u64, String, Arc<AtomicBool>),
    Done(Result<(), String>),
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[test]
fn mutated_programs_never_crash() {
    let cases = env("PROBL_FUZZ_CASES", 300);
    let first = env("PROBL_FUZZ_SEED", 1);
    let examples = examples();
    let (tx, rx) = mpsc::channel();
    // The CLI compiles on the main thread, whose stack is usually 8 MiB.
    let worker = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            for seed in first..first + cases {
                let mut rng = Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
                let src = match rng.below(10) {
                    0 => noise(&mut rng),
                    1..=5 => mutate(rng.pick(&examples), &mut rng),
                    _ => mutate(&generate::program(seed), &mut rng),
                };
                let cancel = Arc::new(AtomicBool::new(false));
                tx.send(Event::Start(seed, src.clone(), cancel.clone())).unwrap();
                tx.send(Event::Done(exercise(&src, &cancel))).unwrap();
            }
        })
        .unwrap();

    let mut failures = Vec::new();
    let mut current: Option<(u64, String, Arc<AtomicBool>, Instant)> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Start(seed, src, cancel)) => current = Some((seed, src, cancel, Instant::now())),
            Ok(Event::Done(result)) => {
                let (seed, src, _, _) = current.take().unwrap();
                if let Err(why) = result {
                    failures.push(format!("seed {seed}: {why}\n---\n{src}\n---"));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let Some((seed, src, cancel, started)) = &current else {
                    continue;
                };
                // A run that takes too long is cancelled; one that doesn't
                // stop hangs somewhere that isn't checking for cancellation.
                if started.elapsed() > Duration::from_secs(10) {
                    cancel.store(true, Ordering::Relaxed);
                }
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "seed {seed} doesn't finish, even when cancelled:\n---\n{src}\n---"
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    worker.join().expect("the fuzzing thread");
    assert!(
        failures.is_empty(),
        "{} of {cases} cases failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
