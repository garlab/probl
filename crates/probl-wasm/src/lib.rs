//! Probl compiled to WebAssembly, for the playground in the browser
//! (docs/playground-plan.md).
//!
//! The functions take and give JSON text: [`check`] gives a program's
//! diagnostics, [`run`] runs it with the playground's limits, and
//! [`examples`] gives the bundled examples. Built for
//! `wasm32-unknown-unknown`, the module exports them as plain functions
//! (see `exports` below), and `web/src/probl.js` passes the text through
//! the module's memory. Natively, they're ordinary Rust functions, which
//! the tests call.

use probl_engine::data::{self, InputLimits, Resolver, Snapshots};
use probl_engine::{Limits, Options, Progress, RuntimeError};
use probl_sema::ir::Mode;
use probl_syntax::{Diagnostic, Severity, SourceFile, Span};
use serde_json::{Map, Value as Json, json};
use std::io::Read;
use std::sync::Arc;

/// The playground's limits: lower than the command line's, since a browser
/// tab has less memory and a smaller stack, and one thread.
pub fn limits() -> Limits {
    Limits {
        max_worlds: 1_000_000,
        max_outcomes: 1_000_000,
        max_collection: 1_000_000,
        max_work: 2_000_000_000,
        max_call_depth: 150,
        max_cached_calls: 200_000,
        max_output: 1024 * 1024,
        max_threads: 1,
        max_chain_states: 20_000,
        ..Limits::default()
    }
}

/// The limits on data a program reads.
fn input_limits() -> InputLimits {
    InputLimits {
        max_bytes: 8 * 1024 * 1024,
        max_values: 1_000_000,
        max_collection: 1_000_000,
        ..InputLimits::default()
    }
}

/// The name programs are shown under in diagnostics.
const FILE: &str = "playground.probl";

/// A program's diagnostics: `{"diagnostics": [...]}`, each with its
/// severity, message, notes and help, where it is (see [`diagnostic`]), and
/// the text the command line would print.
pub fn check(source: &str) -> String {
    let file = SourceFile::new(FILE, source);
    let (_, diags) = probl_sema::compile(source);
    json!({ "diagnostics": diags.iter().map(|d| diagnostic(d, &file)).collect::<Vec<_>>() }).to_string()
}

/// Run a program. The request is `{"source": …}`, and optionally:
///
/// - `"mode"`: `"enumerate"` or `"sample"`, and `"runs"` and `"seed"`,
///   which override the program's `@mode` as on the command line;
/// - `"conjugate"`: `false` to sample without exact updates;
/// - `"files"`: `{path: text}`, the files `read` may read.
///
/// `print` receives what the program prints, as it prints it, and
/// `progress`, when sampling, how many runs are done after each batch, and
/// how many there are. The answer is
/// `{"output": …, "stats": …, "diagnostics": [...]}`, or
/// `{"error": …, "diagnostics": [...]}`, with the error described as a
/// diagnostic, and its `"kind"`: `"language"`, `"unsupported"`, `"limit"`
/// or `"internal"`.
pub fn run(request: &str, print: &mut (dyn FnMut(&str) + Send), progress: Option<Progress>) -> String {
    let request: Json = match serde_json::from_str(request) {
        Ok(r) => r,
        Err(e) => {
            return json!({ "error": { "kind": "internal", "message": format!("a bad request: {e}") } }).to_string();
        }
    };
    let source = request["source"].as_str().unwrap_or_default();
    let file = SourceFile::new(FILE, source);
    let (program, diags) = probl_sema::compile(source);
    let diagnostics: Vec<Json> = diags.iter().map(|d| diagnostic(d, &file)).collect();
    let Some(program) = program else {
        let first = diags.iter().find(|d| d.is_error()).map(|d| diagnostic(d, &file));
        return json!({ "error": first, "diagnostics": diagnostics }).to_string();
    };
    let failed = |e: RuntimeError| json!({ "error": runtime_error(&e, &file), "diagnostics": diagnostics }).to_string();
    let mut options = Options {
        limits: limits(),
        conjugate: request["conjugate"].as_bool().unwrap_or(true),
        mode: mode(&request, &program.settings.mode),
        progress,
        ..Options::default()
    };
    if !program.inputs.is_empty() {
        let mut files = Files(request["files"].as_object().cloned().unwrap_or_default());
        match data::load(&program, &mut files, &mut Snapshots::default(), &input_limits(), None) {
            Ok(inputs) => options.inputs = Some(Arc::new(inputs)),
            Err(e) => return failed(e),
        }
    }
    match probl_engine::run_on_this_thread(&program, &options, print) {
        Ok(outcome) => {
            let s = &outcome.stats;
            let mut stats = json!({
                "peak_worlds": s.peak_worlds,
                "world_steps": s.world_steps,
                "calls": s.calls,
                "reused_calls": s.memo_hits,
                "solved_loops": s.solved_loops,
                "chain_states": s.chain_states,
                "solved_calls": s.solved_calls,
                "call_rounds": s.call_rounds,
            });
            if let Some(sampled) = &outcome.sample {
                stats["runs"] = json!(sampled.runs);
                stats["effective_runs"] = json!(sampled.effective);
            }
            json!({ "output": outcome.output, "stats": stats, "diagnostics": diagnostics }).to_string()
        }
        Err(e) => failed(e),
    }
}

/// The mode a request asks for, as the command line's `--mode`, `--runs`
/// and `--seed` would: `None` keeps the program's.
fn mode(request: &Json, program: &Mode) -> Option<Mode> {
    let asked = request["mode"].as_str();
    let (runs, seed) = (request["runs"].as_u64(), request["seed"].as_u64());
    if asked == Some("enumerate") {
        return Some(Mode::Enumerate);
    }
    if asked != Some("sample") && runs.is_none() && seed.is_none() {
        return None;
    }
    let (default_runs, default_seed) = match program {
        Mode::Sample { runs, seed } => (*runs, *seed),
        _ => (10_000, 0),
    };
    Some(Mode::Sample {
        runs: runs.unwrap_or(default_runs),
        seed: seed.unwrap_or(default_seed),
    })
}

/// The files a program may read: only those given, by the path it's
/// written with.
struct Files(Map<String, Json>);

impl Resolver for Files {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        if path == "-" {
            return Err("the playground has no standard input".into());
        }
        if self.0.contains_key(path) {
            Ok(path.to_string())
        } else {
            Err(format!("`{path}` isn't one of the playground's files"))
        }
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        let text = self.0.get(identity).and_then(Json::as_str).unwrap_or_default();
        Ok(Box::new(std::io::Cursor::new(text.as_bytes().to_vec())))
    }
}

/// A diagnostic as JSON. `from` and `to` count UTF-16 code units, as
/// JavaScript strings do; `line` and `column` start at 1.
fn diagnostic(d: &Diagnostic, file: &SourceFile) -> Json {
    let mut out = located(d.span, file);
    out["severity"] = json!(match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    });
    out["message"] = json!(d.message);
    out["notes"] = json!(d.notes);
    out["help"] = json!(d.help);
    out["rendered"] = json!(d.render(file, false));
    out
}

fn runtime_error(e: &RuntimeError, file: &SourceFile) -> Json {
    let mut out = diagnostic(&e.to_diagnostic(), file);
    out["kind"] = json!(match e.kind {
        probl_engine::ErrorKind::Language => "language",
        probl_engine::ErrorKind::Unsupported => "unsupported",
        probl_engine::ErrorKind::Limit => "limit",
        probl_engine::ErrorKind::Internal => "internal",
    });
    out
}

/// Where a span is, for an editor.
fn located(span: Span, file: &SourceFile) -> Json {
    let text = &file.text;
    let clamp = |i: u32| {
        let mut i = (i as usize).min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (lo, hi) = (clamp(span.lo), clamp(span.hi.max(span.lo)));
    let utf16 = |i: usize| text[..i].encode_utf16().count();
    let (line, column) = file.line_col(lo as u32);
    json!({ "from": utf16(lo), "to": utf16(hi), "line": line, "column": column })
}

/// The bundled examples: `[{"name", "title", "source", "files"}]`, the
/// title from the first comment, and `files` the data they read.
pub fn examples() -> String {
    macro_rules! example {
        ($name:literal) => {
            (
                $name,
                include_str!(concat!("../../../examples/", $name, ".probl")),
            )
        };
    }
    let all = [
        example!("01_tour"),
        example!("02_craps"),
        example!("03_rpg_duel"),
        example!("04_risk_battle"),
        example!("05_snakes_and_ladders"),
        example!("06_blackjack_dealer"),
        example!("07_launch_forecast"),
        example!("08_signup_forecast"),
        example!("09_roadmap"),
    ];
    let data = json!({ "data/pilot.csv": include_str!("../../../examples/data/pilot.csv") });
    let list: Vec<Json> = all
        .iter()
        .map(|(name, source)| {
            let title = source
                .lines()
                .find_map(|l| l.strip_prefix("# "))
                .unwrap_or(name)
                .trim_end_matches('.');
            let files = if source.contains("read(\"data/") {
                data.clone()
            } else {
                json!({})
            };
            json!({ "name": name, "title": title, "source": source, "files": files })
        })
        .collect();
    Json::Array(list).to_string()
}

/// The exported functions, for `wasm32-unknown-unknown`. Text goes in
/// through memory the module allocates (`probl_alloc`), and comes back in a
/// buffer the module keeps until the next call (`probl_result`), whose
/// length each function returns.
#[cfg(target_arch = "wasm32")]
mod exports {
    use std::cell::RefCell;
    use std::sync::Once;

    #[link(wasm_import_module = "probl")]
    unsafe extern "C" {
        /// A line the program printed.
        fn print(ptr: *const u8, len: usize);
        /// What a panic said, just before the module stops.
        fn panicked(ptr: *const u8, len: usize);
        /// Runs done, and runs there are, after each batch of sampled runs.
        fn progress(done: f64, total: f64);
    }

    thread_local! {
        static RESULT: RefCell<String> = const { RefCell::new(String::new()) };
    }

    static HOOK: Once = Once::new();

    fn setup() {
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                let message = info.to_string();
                // SAFETY: the pointer and length describe `message`.
                unsafe { panicked(message.as_ptr(), message.len()) }
            }));
        });
    }

    /// Take the text the caller wrote into memory from `probl_alloc`.
    fn take(ptr: *mut u8, len: usize) -> String {
        // SAFETY: `probl_alloc(len)` returned `ptr`, and it's taken once.
        let bytes = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn give(text: String) -> usize {
        let len = text.len();
        RESULT.with(|r| *r.borrow_mut() = text);
        len
    }

    /// Memory for `len` bytes of text, taken back by the call it's given to.
    #[unsafe(no_mangle)]
    pub extern "C" fn probl_alloc(len: usize) -> *mut u8 {
        Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
    }

    /// The text the last call gave back.
    #[unsafe(no_mangle)]
    pub extern "C" fn probl_result() -> *const u8 {
        RESULT.with(|r| r.borrow().as_ptr())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_check(ptr: *mut u8, len: usize) -> usize {
        setup();
        give(super::check(&take(ptr, len)))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_run(ptr: *mut u8, len: usize) -> usize {
        setup();
        let request = take(ptr, len);
        // SAFETY: the pointer and length describe `line`.
        let mut printed = |line: &str| unsafe { print(line.as_ptr(), line.len()) };
        // SAFETY: `progress` takes two numbers.
        let told = probl_engine::Progress(std::sync::Arc::new(|done, total| unsafe {
            progress(done as f64, total as f64)
        }));
        give(super::run(&request, &mut printed, Some(told)))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_examples() -> usize {
        setup();
        give(super::examples())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_version() -> usize {
        give(env!("CARGO_PKG_VERSION").to_string())
    }
}
