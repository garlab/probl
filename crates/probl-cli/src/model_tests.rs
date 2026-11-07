use probl::{LocalFiles, LocalSources, Options, Snapshots, TestStatus};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// .test.probl files or directories to search (no symlink directories).
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,
    /// Run only tests whose name contains this text.
    #[arg(long)]
    filter: Option<String>,
    /// Pin today, using midnight for now unless --now is also given.
    #[arg(long, value_parser = super::parse_execution_date)]
    today: Option<probl::Date>,
    /// Pin the suite's UTC-normalized datetime.
    #[arg(long)]
    now: Option<probl::DateTime>,
    #[arg(long = "env", value_parser = super::parse_env_var, value_name = "NAME=VALUE")]
    env: Vec<(String, String)>,
    /// Show each test's captured prints and diagnostic reports, even on success.
    #[arg(long, short)]
    verbose: bool,
    /// Maximum worlds per statement in each test.
    #[arg(long)]
    max_worlds: Option<usize>,
    /// Maximum work per test.
    #[arg(long)]
    max_work: Option<u64>,
    /// Stop the suite after this many seconds, leaving later tests unrun.
    #[arg(long)]
    timeout: Option<f64>,
}

pub fn run(args: Args) -> ExitCode {
    let mut paths = BTreeSet::new();
    for path in &args.paths {
        if let Err(error) = discover(path, &mut paths, 0, true) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    }
    let now = match args
        .now
        .or_else(|| args.today.map(|d| d.at_midnight()))
        .map(Ok)
        .unwrap_or_else(super::execution_datetime)
    {
        Ok(now) => now,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    if args.today.is_some_and(|date| date != now.date()) {
        eprintln!("error: --today must match the calendar date of --now");
        return ExitCode::FAILURE;
    }
    let mut limits = probl::Limits::default();
    if let Some(n) = args.max_worlds {
        limits.max_worlds = n;
    }
    if let Some(n) = args.max_work {
        limits.max_work = n;
    }
    let cancel = probl::Cancel::new();
    if let Some(seconds) = args.timeout {
        let duration = match std::time::Duration::try_from_secs_f64(seconds) {
            Ok(duration) => duration,
            Err(_) => {
                eprintln!("error: timeout must be a finite nonnegative number of seconds");
                return ExitCode::FAILURE;
            }
        };
        if duration.is_zero() {
            cancel.cancel();
        } else {
            let timer = cancel.clone();
            std::thread::spawn(move || {
                std::thread::sleep(duration);
                timer.cancel();
            });
        }
    }
    let options = super::environment(Options::new(), &args.env)
        .now(now)
        .limits(limits)
        .cancel(&cancel);
    // Compile every selected file before running any model code.
    let mut suites = Vec::new();
    let (mut test_count, mut source_bytes) = (0, 0);
    for path in paths {
        if cancel.is_cancelled() {
            eprintln!("error: suite cancelled before execution; tests are unrun");
            return ExitCode::FAILURE;
        }
        let suite = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                probl::compile_tests(&path.display().to_string(), &text, &mut LocalSources::new())
                    .map_err(|e| e.render(super::color()))
            });
        if let Ok(suite) = &suite {
            test_count += suite.tests().len();
            source_bytes += suite.compiled_source_bytes();
            if test_count > 1000 || source_bytes > 64 * 1024 * 1024 {
                eprintln!("error: suite exceeds 1,000 tests or 64 MiB of compiled source; tests are unrun");
                return ExitCode::FAILURE;
            }
        }
        suites.push((path, suite));
    }
    let mut counts = [0usize; 4];
    let mut selected = 0;
    let mut stopped = false;
    println!("enumerated tests · now {now}");
    for (path, suite) in suites {
        let suite = match suite {
            Ok(suite) => suite,
            Err(error) => {
                counts[2] += 1;
                eprintln!("{}: {error}", path.display());
                continue;
            }
        };
        let mut files = Snapshots::new(LocalFiles::next_to(&path));
        for test in suite.tests() {
            if args.filter.as_ref().is_some_and(|filter| !test.name().contains(filter)) {
                continue;
            }
            selected += 1;
            if stopped || cancel.is_cancelled() {
                stopped = true;
                println!("UNRUN {} :: {}", path.display(), test.name());
                continue;
            }
            let result = test.run(&options, &mut files);
            stopped |= result.error_kind == Some(probl::ErrorKind::Internal);
            let (index, label) = status(result.status);
            counts[index] += 1;
            println!("{label} {} :: {}", path.display(), result.name);
            for assertion in result.assertions.iter().filter(|a| a.status != TestStatus::Pass) {
                let (_, label) = status(assertion.status);
                if assertion.status == TestStatus::Fail {
                    println!(
                        "  {label}: {} (false weight {:.6e}; log weight {})",
                        assertion.label, assertion.false_probability, assertion.log_false_weight
                    );
                } else {
                    println!("  {label}: {}", assertion.label);
                }
            }
            for diagnostic in &result.diagnostics {
                eprint!("{}", diagnostic.render(super::color()));
            }
            if let Some(message) = result.message.filter(|_| result.diagnostics.is_empty()) {
                println!("  {message}");
            }
            if args.verbose || result.status != TestStatus::Pass {
                for line in result.printed {
                    println!("  {line}");
                }
                if !result.output.is_empty() {
                    println!("{}", result.output);
                }
            }
        }
    }
    if selected == 0 {
        eprintln!("error: no tests selected");
        return ExitCode::FAILURE;
    }
    println!(
        "{} passed · {} failed · {} errors · {} inconclusive",
        counts[0], counts[1], counts[2], counts[3]
    );
    if !stopped && counts[1..].iter().all(|n| *n == 0) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn status(status: TestStatus) -> (usize, &'static str) {
    match status {
        TestStatus::Pass => (0, "pass"),
        TestStatus::Fail => (1, "FAIL"),
        TestStatus::Error => (2, "ERROR"),
        TestStatus::Inconclusive => (3, "INCONCLUSIVE"),
    }
}

fn discover(path: &Path, files: &mut BTreeSet<PathBuf>, depth: usize, explicit: bool) -> Result<(), String> {
    if depth > 64 {
        return Err("test discovery exceeds 64 directory levels".into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(&*name, "target" | "node_modules") {
                continue;
            }
            discover(&entry.path(), files, depth + 1, false)?;
        }
    } else if meta.is_file() && path.to_string_lossy().ends_with(".test.probl") {
        files.insert(path.canonicalize().map_err(|e| e.to_string())?);
        if files.len() > 1000 {
            return Err("test discovery exceeds 1,000 files".into());
        }
    } else if explicit {
        return Err(format!("{} is not a .test.probl file or directory", path.display()));
    }
    Ok(())
}
