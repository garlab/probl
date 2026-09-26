//! The `probl` command-line tool.

use clap::{Parser, Subcommand};
use probl_engine::Options;
use probl_syntax::{Diagnostic, SourceFile, render_all};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
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

#[derive(Subcommand)]
enum Command {
    /// Run a program and print its reports.
    Run {
        file: PathBuf,
        /// Also print probabilities as fractions, like 244/495.
        #[arg(long)]
        fractions: bool,
        /// Stop loops once the worlds still inside weigh less than this.
        #[arg(long)]
        epsilon: Option<f64>,
        /// Print how much work the engine did.
        #[arg(long)]
        stats: bool,
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
            no_merge,
            no_memo,
        } => {
            let options = Options {
                merge: !no_merge,
                memoize: !no_memo,
                epsilon,
                fractions,
            };
            run_file(&file, &options, stats)
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

fn run_file(path: &PathBuf, options: &Options, stats: bool) -> ExitCode {
    let Some(file) = read(path) else {
        return ExitCode::FAILURE;
    };
    let (program, diags) = probl_sema::compile(&file.text);
    report_diagnostics(&diags, &file);
    let Some(program) = program else {
        return ExitCode::FAILURE;
    };
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
