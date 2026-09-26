//! The `probl` command-line tool.

use clap::{Parser, Subcommand, ValueEnum};
use probl_engine::Options;
use probl_sema::ir::Mode;
use probl_syntax::{Diagnostic, SourceFile, render_all};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// What the command line asks for, over the program's `@mode`.
struct ModeChoice {
    mode: Option<ModeArg>,
    runs: Option<u64>,
    seed: Option<u64>,
}

impl ModeChoice {
    fn resolve(&self, program: &Mode) -> Option<Mode> {
        let sample =
            self.mode == Some(ModeArg::Sample) || (self.mode.is_none() && (self.runs.is_some() || self.seed.is_some()));
        if self.mode == Some(ModeArg::Enumerate) {
            return Some(Mode::Enumerate);
        }
        if !sample {
            return None;
        }
        let (runs, seed) = match program {
            Mode::Sample { runs, seed } => (*runs, *seed),
            _ => (10_000, 0),
        };
        Some(Mode::Sample {
            runs: self.runs.unwrap_or(runs),
            seed: self.seed.unwrap_or(seed),
        })
    }
}

#[derive(Subcommand)]
enum Command {
    /// Run a program and print its reports.
    Run {
        file: PathBuf,
        /// Also print the simplest fraction near each probability, like ≈ 244/495 (a hint, not a proof).
        #[arg(long)]
        fractions: bool,
        /// Stop `while` and `loop` once the worlds still inside weigh less than this share of what entered.
        #[arg(long)]
        epsilon: Option<f64>,
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
        /// Stop the run after this many seconds.
        #[arg(long)]
        timeout: Option<f64>,
        /// The most worlds a statement may produce.
        #[arg(long)]
        max_worlds: Option<usize>,
        /// The most work (world-steps and outcomes computed) the run may do.
        #[arg(long)]
        max_work: Option<u64>,
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
    },
    /// Check a program for errors without running it.
    Check { file: PathBuf },
    /// Start an interactive session.
    Repl,
    /// Print the lowered program (for debugging the compiler).
    #[command(hide = true)]
    Ir { file: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            file,
            fractions,
            epsilon,
            stats,
            mode,
            runs,
            seed,
            threads,
            timeout,
            max_worlds,
            max_work,
            stack_mb,
            max_depth,
            no_merge,
            no_memo,
        } => {
            let mut options = Options {
                merge: !no_merge,
                memoize: !no_memo,
                epsilon,
                fractions,
                ..Options::default()
            };
            if let Some(n) = max_worlds {
                options.limits.max_worlds = n;
            }
            if let Some(n) = max_work {
                options.limits.max_work = n;
            }
            if let Some(mb) = stack_mb {
                options.limits.stack_size = mb * 1024 * 1024;
            }
            if let Some(d) = max_depth {
                options.limits.max_call_depth = d;
            }
            if let Some(n) = threads {
                options.limits.max_threads = n.max(1);
            }
            if let Some(seconds) = timeout {
                let cancel = Arc::new(AtomicBool::new(false));
                options.cancel = Some(cancel.clone());
                let duration = std::time::Duration::from_secs_f64(seconds.max(0.0));
                std::thread::spawn(move || {
                    std::thread::sleep(duration);
                    cancel.store(true, Ordering::Relaxed);
                });
            }
            run_file(&file, &mut options, stats, &ModeChoice { mode, runs, seed })
        }
        Command::Check { file } => check_file(&file),
        Command::Repl => repl(),
        Command::Ir { file } => ir_file(&file),
    }
}

fn color() -> bool {
    std::io::stderr().is_terminal()
}

fn read(path: &PathBuf) -> Option<SourceFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(SourceFile::new(path.display().to_string(), text)),
        Err(e) => {
            eprintln!("error: can't read {}: {e}", path.display());
            None
        }
    }
}

fn report_diagnostics(diags: &[Diagnostic], file: &SourceFile) {
    if !diags.is_empty() {
        eprint!("{}", render_all(diags, file, color()));
    }
}

fn run_file(path: &PathBuf, options: &mut Options, stats: bool, choice: &ModeChoice) -> ExitCode {
    let Some(file) = read(path) else {
        return ExitCode::FAILURE;
    };
    let (program, diags) = probl_sema::compile(&file.text);
    report_diagnostics(&diags, &file);
    let Some(program) = program else {
        return ExitCode::FAILURE;
    };
    options.mode = choice.resolve(&program.settings.mode);
    let mut print = |line: &str| println!("{line}");
    match probl_engine::run(&program, options, &mut print) {
        Ok(outcome) => {
            println!("{}", outcome.output);
            if stats {
                let s = &outcome.stats;
                eprintln!(
                    "stats: peak {} worlds · {} world-steps · {} calls ({} reused)",
                    s.peak_worlds, s.world_steps, s.calls, s.memo_hits
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprint!("{}", e.to_diagnostic().render(&file, color()));
            if e.kind == probl_engine::ErrorKind::Internal {
                return ExitCode::from(70);
            }
            ExitCode::FAILURE
        }
    }
}

fn check_file(path: &PathBuf) -> ExitCode {
    let Some(file) = read(path) else {
        return ExitCode::FAILURE;
    };
    let (program, diags) = probl_sema::compile(&file.text);
    report_diagnostics(&diags, &file);
    if program.is_some() {
        eprintln!("{}: ok", file.name);
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn ir_file(path: &PathBuf) -> ExitCode {
    let Some(file) = read(path) else {
        return ExitCode::FAILURE;
    };
    let (program, diags) = probl_sema::compile(&file.text);
    report_diagnostics(&diags, &file);
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
    println!(
        "Probl {} — type an expression to see its distribution; Ctrl-D to quit.",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = std::io::stdin();
    let mut session = String::new();
    let mut lines = stdin.lock().lines();
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
        let input = as_report_if_expression(&input);
        let candidate = format!("{session}{input}");
        let (program, diags) = probl_sema::compile(&candidate);
        let file = SourceFile::new("<repl>", candidate.clone());
        report_diagnostics(&diags, &file);
        let Some(program) = program else {
            continue;
        };
        let before = probl_sema::compile(&session).0.map_or(0, |p| p.reports.len());
        let mut print = |line: &str| println!("{line}");
        match probl_engine::run(&program, &Options::default(), &mut print) {
            Ok(outcome) => {
                // Show only what the new input reported.
                let lines: Vec<&str> = outcome.output.lines().collect();
                let body = lines.iter().skip(2).copied().collect::<Vec<_>>();
                let new_reports = program.reports.len() - before;
                if new_reports > 0 {
                    for line in body
                        .iter()
                        .rev()
                        .take(new_reports)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                    {
                        println!("{line}");
                    }
                }
                session = candidate;
            }
            Err(e) => eprint!("{}", e.to_diagnostic().render(&file, color())),
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
    format!("report ({text}) as {text:?}\n")
}
