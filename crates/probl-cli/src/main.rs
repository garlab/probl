//! The `probl` command-line tool.

use clap::{Parser, Subcommand, ValueEnum};
use probl::__internal::{EngineChecks, Symbolic, diagrams, engine_checks};
use probl::{Cancel, Date, ErrorKind, FailureMode, Files, Limits, LocalFiles, Options, Program, Snapshots};
use probl_cli::parse_size;
use probl_engine::data::{self, InputLimits};
use probl_engine::report::thousands;
use probl_sema::ir::DataFormat;
use probl_syntax::{SourceFile, render_all};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "probl",
    version,
    about = "Probl: a language where conditions are probabilities"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ModeArg {
    Enumerate,
    Sample,
}

/// When to answer a program with decision diagrams (experimental).
#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SymbolicArg {
    /// Always, failing on a program they don't support.
    Only,
    /// When they support the program, enumerating it otherwise.
    Prefer,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum OnErrorArg {
    Total,
    Partial,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum FormatArg {
    Csv,
    Json,
    Lines,
}

impl FormatArg {
    fn format(self) -> DataFormat {
        match self {
            FormatArg::Csv => DataFormat::Csv,
            FormatArg::Json => DataFormat::Json,
            FormatArg::Lines => DataFormat::Lines,
        }
    }
}

#[derive(clap::Args)]
struct RunArgs {
    file: PathBuf,
    /// Pin the execution-date snapshot (`today`); defaults to the UTC date at launch.
    #[arg(long, value_parser = parse_execution_date, value_name = "YYYY-MM-DD")]
    today: Option<Date>,
    /// Set a variable in the program's `env`, in place of the process's (repeatable).
    #[arg(long = "env", value_parser = parse_env_var, value_name = "NAME=VALUE")]
    env: Vec<(String, String)>,
    /// Also print the simplest fraction near each probability, like ≈ 244/495 (a hint, not a proof).
    #[arg(long)]
    fractions: bool,
    /// Summary columns for a zero-based report ID (repeatable): ID=forecast,
    /// ID=moments, or ID=mean,sd,median,q:0.1,q:0.9. Other reports keep defaults.
    #[arg(long = "summary", value_parser = parse_report_summary, value_name = "ID=SPEC")]
    summaries: Vec<(usize, probl::ReportSummary)>,
    /// Retain exact populations (default), moments, or approximate quantiles for a sampled report.
    #[arg(long = "collect", value_parser = parse_report_collection, value_name = "ID=exact|moments|quantiles[:CAPACITY]")]
    collections: Vec<(usize, probl::ReportCollection)>,
    /// Stop `while` and `loop` once the worlds still inside weigh less than this share of what entered.
    #[arg(long)]
    epsilon: Option<f64>,
    /// When enumerating, compute what has no formula by numerical integration, to this relative tolerance (like `@integrate`).
    #[arg(long, value_name = "TOLERANCE", num_args = 0..=1, require_equals = true, default_missing_value = "1e-9")]
    integrate: Option<f64>,
    /// Print how much work the engine did.
    #[arg(long)]
    stats: bool,
    /// Enumerate or sample, whatever the program's `@mode` says.
    #[arg(long, value_enum)]
    mode: Option<ModeArg>,
    /// When sampling: the number of runs (implies `--mode sample`).
    #[arg(long)]
    runs: Option<u64>,
    /// When sampling: the seed of the random numbers (implies `--mode sample`).
    #[arg(long)]
    seed: Option<u64>,
    /// When sampling: the most threads to use (the output doesn't depend on it).
    #[arg(long)]
    threads: Option<usize>,
    /// When sampling: draw every variable from its prior, without exact updates for conjugate priors (for comparing; the estimates mean the same).
    #[arg(long)]
    no_conjugate: bool,
    /// What a fault in one world does to the others, whatever the program's `@on_error` says: `total` stops the run (the default when enumerating), `partial` lets the others finish (the default when sampling).
    #[arg(long, value_enum, value_name = "MODE")]
    on_error: Option<OnErrorArg>,
    /// Stop the run after this many seconds.
    #[arg(long)]
    timeout: Option<f64>,
    /// The most worlds a statement may produce.
    #[arg(long)]
    max_worlds: Option<usize>,
    /// The most work (world-steps and outcomes computed) the run may do.
    #[arg(long)]
    max_work: Option<u64>,
    /// Live storage budget for moments/quantile report collectors, including keys
    /// and worker batches, like 128M (default 1G). Excludes exact reports.
    #[arg(long, value_parser = parse_size)]
    max_report_summary: Option<u64>,
    /// The most data the program may read, all files together, like 64M.
    #[arg(long, value_parser = parse_size)]
    max_input: Option<u64>,
    /// Stack size of the engine thread, in MiB (for checking the engine).
    #[arg(long, hide = true)]
    stack_mb: Option<usize>,
    /// Nested-call limit (for checking the engine).
    #[arg(long, hide = true)]
    max_depth: Option<usize>,
    /// Don't merge identical worlds (for checking the engine).
    #[arg(long, hide = true)]
    no_merge: bool,
    /// Don't reuse function results (for checking the engine).
    #[arg(long, hide = true)]
    no_memo: bool,
    /// Unroll loops that cycle instead of solving them (for checking the engine).
    #[arg(long, hide = true)]
    no_solve: bool,
    /// Experimental: answer the program with decision diagrams instead of worlds, `only` (the default) or where they're supported (`prefer`).
    #[arg(long, hide = true, value_enum, value_name = "WHEN", num_args = 0..=1, require_equals = true, default_missing_value = "only")]
    symbolic: Option<SymbolicArg>,
}

#[derive(Subcommand)]
enum Command {
    /// Run a program and print its reports.
    Run(Box<RunArgs>),
    /// Check a program for errors without running it.
    Check {
        file: PathBuf,
        /// Also read the program's data, and check it fits the declared types.
        #[arg(long)]
        data: bool,
        /// The most data the program may read, all files together, like 64M.
        #[arg(long, value_parser = parse_size)]
        max_input: Option<u64>,
    },
    /// Suggest a type to read a data file with (`-` is standard input).
    Schema {
        file: String,
        /// The format, when the file's extension doesn't say.
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// The largest file to read, like 64M.
        #[arg(long, value_parser = parse_size)]
        max_input: Option<u64>,
    },
    /// Start an interactive session.
    Repl,
    /// Print the lowered program (for debugging the compiler).
    #[command(hide = true)]
    Ir { file: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => {
            let RunArgs {
                file,
                today,
                env,
                fractions,
                summaries,
                collections,
                epsilon,
                integrate,
                stats,
                mode,
                runs,
                seed,
                threads,
                no_conjugate,
                on_error,
                timeout,
                max_worlds,
                max_work,
                max_report_summary,
                max_input,
                stack_mb,
                max_depth,
                no_merge,
                no_memo,
                no_solve,
                symbolic,
            } = *args;
            let today = match today.map(Ok).unwrap_or_else(execution_date) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mut options = environment(Options::new(), &env)
                .today(today)
                .fractions(fractions)
                .conjugate(!no_conjugate);
            let mut configured = std::collections::BTreeSet::new();
            for (id, summary) in summaries {
                if !configured.insert(id) {
                    eprintln!("error: duplicate summary override for report {id}");
                    return ExitCode::FAILURE;
                }
                options = options.report_summary(id, summary);
            }
            configured.clear();
            for (id, collection) in collections {
                if !configured.insert(id) {
                    eprintln!("error: duplicate collection override for report {id}");
                    return ExitCode::FAILURE;
                }
                options = options.report_collection(id, collection);
            }
            if let Some(e) = epsilon {
                options = options.epsilon(e);
            }
            if let Some(t) = integrate {
                options = options.integrate(t);
            }
            options = match mode {
                Some(ModeArg::Enumerate) => options.enumerate(),
                Some(ModeArg::Sample) => options.sample(),
                None => options,
            };
            if let Some(n) = runs {
                options = options.runs(n);
            }
            if let Some(s) = seed {
                options = options.seed(s);
            }
            options = match on_error {
                Some(OnErrorArg::Total) => options.on_error(FailureMode::Total),
                Some(OnErrorArg::Partial) => options.on_error(FailureMode::Partial),
                None => options,
            };
            let mut limits = Limits::default();
            if let Some(n) = max_worlds {
                limits.max_worlds = n;
            }
            if let Some(n) = max_work {
                limits.max_work = n;
            }
            if let Some(n) = max_report_summary {
                limits.max_report_summary_bytes = match usize::try_from(n) {
                    Ok(n) => n,
                    Err(_) => {
                        eprintln!("error: --max-report-summary is too large for this platform");
                        return ExitCode::FAILURE;
                    }
                };
            }
            if let Some(mb) = stack_mb {
                limits.stack_size = mb * 1024 * 1024;
            }
            if let Some(d) = max_depth {
                limits.max_call_depth = d;
            }
            if let Some(n) = threads {
                limits.max_threads = n.max(1);
            }
            if let Some(n) = max_input {
                limits.max_input_bytes = n;
            }
            options = options.limits(limits);
            let checks = EngineChecks {
                merge: !no_merge,
                memoize: !no_memo,
                solve: !no_solve,
                symbolic: match symbolic {
                    Some(SymbolicArg::Only) => Symbolic::Only,
                    Some(SymbolicArg::Prefer) => Symbolic::Prefer,
                    None => Symbolic::Off,
                },
            };
            options = engine_checks(options, checks);
            let cancel = timeout.map(|seconds| {
                let cancel = Cancel::new();
                let timer = cancel.clone();
                let duration = std::time::Duration::from_secs_f64(seconds.max(0.0));
                std::thread::spawn(move || {
                    std::thread::sleep(duration);
                    timer.cancel();
                });
                cancel
            });
            if let Some(cancel) = &cancel {
                options = options.cancel(cancel);
            }
            run_file(&file, options, cancel.as_ref(), stats)
        }
        Command::Check { file, data, max_input } => check_file(&file, data.then_some(max_input)),
        Command::Schema {
            file,
            format,
            max_input,
        } => schema(&file, format, max_input),
        Command::Repl => repl(),
        Command::Ir { file } => ir_file(&file),
    }
}

fn parse_execution_date(s: &str) -> Result<Date, String> {
    s.parse().map_err(|e: probl::ParseDateError| e.to_string())
}

fn parse_report_collection(s: &str) -> Result<(usize, probl::ReportCollection), String> {
    let (id, policy) = s
        .split_once('=')
        .ok_or("expected ID=exact, ID=moments or ID=quantiles[:CAPACITY]")?;
    let id = id
        .parse()
        .map_err(|_| "report collection ID must be a zero-based integer")?;
    Ok((
        id,
        policy.parse::<probl::ReportCollection>().map_err(|e| e.to_string())?,
    ))
}

fn parse_report_summary(s: &str) -> Result<(usize, probl::ReportSummary), String> {
    let (id, selection) = s
        .split_once('=')
        .ok_or("expected ID=SPEC, for example 0=forecast or 0=mean,sd,median,q:0.9")?;
    let id = id
        .parse()
        .map_err(|_| "report summary ID must be a zero-based integer")?;
    Ok((
        id,
        selection.parse::<probl::ReportSummary>().map_err(|e| e.to_string())?,
    ))
}

fn parse_env_var(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((name, value)) if !name.is_empty() => Ok((name.to_string(), value.to_string())),
        _ => Err("expected NAME=VALUE".to_string()),
    }
}

/// `options`, with the environment a program's `env` stands for: the
/// process's, captured once when the command starts, then `given` (`--env`),
/// which takes precedence. A variable whose name or value isn't valid UTF-8
/// is left out. Neither the compiler nor the engine reads the environment.
fn environment(options: Options, given: &[(String, String)]) -> Options {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .chain(given.iter().cloned())
        .fold(options, |options, (name, value)| options.env(name, value))
}

/// The CLI reads the clock once. Neither the compiler nor engine reads it.
fn execution_date() -> Result<Date, String> {
    Date::today_utc().ok_or_else(|| "the system clock is outside the supported date range".to_string())
}

fn color() -> bool {
    std::io::stderr().is_terminal()
}

fn read(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("error: can't read {}: {e}", path.display());
            None
        }
    }
}

/// Compile a program, printing its diagnostics.
fn compile(name: &str, text: &str) -> Option<Program> {
    match probl::compile(name, text) {
        Ok(program) => {
            for warning in program.warnings() {
                eprint!("{}", warning.render(color()));
            }
            Some(program)
        }
        Err(e) => {
            eprint!("{}", e.render(color()));
            None
        }
    }
}

/// Read a program's data with these options, reporting any problem.
fn load(program: &Program, files: &mut dyn Files, options: Options) -> Option<Options> {
    if !program.reads_data() {
        return Some(options);
    }
    match program.load(files, &options) {
        Ok(data) => Some(options.data(data)),
        Err(e) => {
            eprint!("{}", e.render(color()));
            None
        }
    }
}

fn run_file(path: &Path, options: Options, cancel: Option<&Cancel>, stats: bool) -> ExitCode {
    let Some(text) = read(path) else {
        return ExitCode::FAILURE;
    };
    let Some(program) = compile(&path.display().to_string(), &text) else {
        return ExitCode::FAILURE;
    };
    let mut files = LocalFiles::next_to(path).stdin(true);
    if let Some(cancel) = cancel {
        files = files.cancel(cancel);
    }
    let Some(options) = load(&program, &mut files, options) else {
        return ExitCode::FAILURE;
    };
    let mut print = |line: &str| println!("{line}");
    match program.run_with(&options, &mut print) {
        Ok(outcome) => {
            println!("{}", outcome.text());
            if stats {
                if let Some(today) = outcome.today() {
                    eprintln!("execution date: {today} (UTC default; --today overrides it)");
                }
                let s = outcome.stats();
                let mut line = format!(
                    "stats: peak {} worlds · {} world-steps · {} calls ({} reused)",
                    s.peak_worlds(),
                    s.world_steps(),
                    s.calls(),
                    s.reused_calls()
                );
                let count = |n: u64, one: &str, many: &str| {
                    format!("{} {}", thousands(n as i64), if n == 1 { one } else { many })
                };
                if s.solved_loops() > 0 {
                    line.push_str(&format!(
                        " · {} solved ({})",
                        count(s.solved_loops(), "loop", "loops"),
                        count(s.chain_states(), "state", "states")
                    ));
                }
                if s.solved_calls() > 0 {
                    line.push_str(&format!(
                        " · {} solved ({})",
                        count(s.solved_calls(), "recursive call", "recursive calls"),
                        count(s.call_rounds(), "round", "rounds")
                    ));
                }
                if let Some((variables, nodes)) = diagrams(s) {
                    line = format!(
                        "stats: symbolic · {} · {}",
                        count(variables, "variable", "variables"),
                        count(nodes, "decision node", "decision nodes")
                    );
                }
                eprintln!("{line}");
                for u in s.exact_updates() {
                    eprintln!(
                        "exact updates: `{}` (line {}) · {} draws delayed · {} observations · drawn {} times",
                        u.variable(),
                        u.line(),
                        thousands(u.delayed() as i64),
                        thousands(u.observations() as i64),
                        thousands(u.drawn() as i64)
                    );
                }
                for source in outcome.data() {
                    eprintln!(
                        "data: {} · {} bytes · sha256 {}",
                        source.identity(),
                        source.bytes(),
                        source.sha256()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            // A partial result: what the other worlds gave, then where the
            // failed ones failed.
            if let Some(outcome) = e.partial() {
                println!("{}", outcome.text());
            }
            eprint!("{}", e.render(color()));
            match e.partial() {
                Some(outcome) if outcome.finished() => ExitCode::from(PARTIAL),
                _ if e.kind() == ErrorKind::Internal => ExitCode::from(70),
                _ => ExitCode::FAILURE,
            }
        }
    }
}

/// The exit status of a partial result: some worlds failed, and the others
/// finished.
const PARTIAL: u8 = 3;

/// Compile a program; with `data`, also read its data, with at most that many
/// bytes if given.
fn check_file(path: &Path, data: Option<Option<u64>>) -> ExitCode {
    let Some(text) = read(path) else {
        return ExitCode::FAILURE;
    };
    let name = path.display().to_string();
    let Some(program) = compile(&name, &text) else {
        return ExitCode::FAILURE;
    };
    let mut sources = String::new();
    if let Some(max_input) = data.filter(|_| program.reads_data()) {
        let mut limits = Limits::default();
        if let Some(n) = max_input {
            limits.max_input_bytes = n;
        }
        let mut files = LocalFiles::next_to(path).stdin(true);
        match program.load(&mut files, &Options::new().limits(limits)) {
            Ok(data) => {
                let n = data.sources().len();
                sources = format!(", and so is its data ({n} file{})", if n == 1 { "" } else { "s" });
            }
            Err(e) => {
                eprint!("{}", e.render(color()));
                return ExitCode::FAILURE;
            }
        }
    }
    eprintln!("{name}: ok{sources}");
    ExitCode::SUCCESS
}

/// Print a suggested type for a data file.
fn schema(path: &str, format: Option<FormatArg>, max_input: Option<u64>) -> ExitCode {
    let Some(format) = format.map(FormatArg::format).or_else(|| DataFormat::from_path(path)) else {
        eprintln!("error: can't tell the format of {path} from its name: say it with --format csv, json or lines");
        return ExitCode::FAILURE;
    };
    let default = InputLimits::default();
    let limits = InputLimits {
        max_bytes: max_input.unwrap_or(default.max_bytes),
        ..default
    };
    let mut files = LocalFiles::new(".").stdin(true);
    let reader = files.resolve(path).and_then(|id| files.open(&id));
    let mut bytes = Vec::new();
    let read = reader.and_then(|r| {
        r.take(limits.max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())
    });
    if let Err(why) = read {
        eprintln!("error: can't read {path}: {why}");
        return ExitCode::FAILURE;
    }
    match data::suggest(&bytes, format, path, &limits) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("error: {path}: {why}");
            ExitCode::FAILURE
        }
    }
}

fn ir_file(path: &Path) -> ExitCode {
    let Some(text) = read(path) else {
        return ExitCode::FAILURE;
    };
    let file = SourceFile::new(path.display().to_string(), text);
    let (program, diags) = probl_sema::compile(&file.text);
    if !diags.is_empty() {
        eprint!("{}", render_all(&diags, &file, color()));
    }
    let Some(program) = program else {
        return ExitCode::FAILURE;
    };
    let liveness = probl_sema::analyze(&program);
    print!("{}", probl_sema::pretty::program(&program, Some(&liveness)));
    ExitCode::SUCCESS
}

/// A session is a growing program: each input is appended and the whole
/// program runs again, printing only the reports the new input added.
fn repl() -> ExitCode {
    let today = match execution_date() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "Probl {} — type an expression to see its distribution; Ctrl-D to quit.",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = std::io::stdin();
    let mut session = String::new();
    let mut lines = stdin.lock().lines();
    // The session's reports so far: the new input's come after them.
    let mut reported = 0;
    // Data read during the session stays the same until `:reload`, so that
    // running the session again doesn't change earlier bindings. Standard
    // input is the session, not data.
    let mut files = Snapshots::new(LocalFiles::new("."));
    // One date and one environment for the whole session.
    let options = environment(Options::new().today(today), &[]);
    loop {
        let mut input = String::new();
        let mut prompt = "probl> ";
        loop {
            print!("{prompt}");
            std::io::stdout().flush().ok();
            let Some(Ok(line)) = lines.next() else {
                println!();
                return ExitCode::SUCCESS;
            };
            input.push_str(&line);
            input.push('\n');
            if open_brackets(&input) <= 0 {
                break;
            }
            prompt = "   ... ";
        }
        if input.trim().is_empty() {
            continue;
        }
        if input.trim() == ":reload" {
            files.clear();
            println!("the data will be read again");
            continue;
        }
        let input = as_report_if_expression(&input);
        let candidate = format!("{session}{input}");
        let Some(program) = compile("<repl>", &candidate) else {
            continue;
        };
        let Some(options) = load(&program, &mut files, options.clone()) else {
            continue;
        };
        let mut print = |line: &str| println!("{line}");
        match program.run_with(&options, &mut print) {
            Ok(outcome) => {
                let reports = outcome.reports().len();
                print!("{}", outcome.render(reported..reports));
                reported = reports;
                session = candidate;
            }
            Err(e) => {
                // A partial result shows what the other worlds gave, but the
                // input isn't kept.
                if let Some(outcome) = e.partial() {
                    print!("{}", outcome.render(reported..outcome.reports().len()));
                }
                eprint!("{}", e.render(color()));
            }
        }
    }
}

/// How many brackets are still open (ignoring strings and comments).
fn open_brackets(text: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '#' if !in_string => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' | '(' | '[' if !in_string => depth += 1,
            '}' | ')' | ']' if !in_string => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Turn a bare expression like `2d6 >= 10` into a report of itself.
fn as_report_if_expression(input: &str) -> String {
    use probl_syntax::ast::{ExprKind, Item, StmtKind};
    let (program, diags) = probl_syntax::parse_program(input);
    if !diags.is_empty() || program.items.len() != 1 {
        return input.to_string();
    }
    let Item::Stmt(stmt) = &program.items[0] else {
        return input.to_string();
    };
    let StmtKind::Expr(e) = &stmt.kind else {
        return input.to_string();
    };
    let is_statement_like = matches!(
        &e.kind,
        ExprKind::If { .. } | ExprKind::Chance { .. } | ExprKind::Match { .. } | ExprKind::Block(_)
    ) || matches!(&e.kind, ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Name(n) if n == "print"));
    if is_statement_like {
        return input.to_string();
    }
    let text = input.trim();
    format!("report ({text}) as \"{}\"\n", escape(text))
}

/// `text` as the inside of a Probl string literal: its braces would
/// otherwise be interpolations, and Rust's `{:?}` writes escapes like
/// `\u{301}` that Probl doesn't have.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '"' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}
