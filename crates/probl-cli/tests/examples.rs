//! Golden tests: every program in `examples/` must print the output in its
//! `# ── Output` block: exactly, when it enumerates; when it samples, with
//! numbers that agree within their sampling error (see `compare_sampled`).
//!
//! Examples that need features that aren't implemented yet are listed in
//! `PENDING`. They are reported as ignored, but only if they really are
//! unsupported: an example that isn't in the list and hits an unsupported
//! feature fails, and so does a listed example that starts working.

use libtest_mimic::{Arguments, Failed, Trial};
use probl_cli::LocalFiles;
use probl_engine::data::{self, InputLimits, Snapshots};
use probl_engine::{ErrorKind, Options};
use probl_syntax::{SourceFile, render_all};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Examples waiting for features that aren't implemented yet.
const PENDING: &[&str] = &[];

fn main() {
    let args = Arguments::from_args();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("examples directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "probl"))
        .collect();
    paths.sort();
    let mut trials: Vec<Trial> = paths.into_iter().map(trial).collect();
    trials.push(Trial::test(
        "sampled_outputs_are_compared_within_their_error",
        comparator,
    ));
    libtest_mimic::run(&args, trials).exit();
}

/// `compare_sampled` accepts sampling noise and rejects real differences.
fn comparator() -> Result<(), Failed> {
    let expected = "sample · 1,000 runs · seed 1\n\nwin    46.1% ± 0.3%\nsize   mean 398 · sd 75 · 5% 288\nwhen   5% 2027-01-26\n\nt\n  k    5%\n  1    100\n  3    300\n  (some rows are left out here)";
    let noisy = "sample · 1,000 runs · seed 1\n\nwin    46.9% ± 0.3%\nsize   mean 401 · sd 74 · 5% 290\nwhen   5% 2027-01-28\n\nt\n  k    5%\n  1    101\n  2    200\n  3    299";
    compare_sampled(expected, noisy)?;
    let wrong = [
        noisy.replace("46.9%", "48.1%"),
        noisy.replace("mean 401", "mean 430"),
        noisy.replace("2027-01-28", "2027-02-02"),
        noisy.replace("  3    299", "  3    350"),
        noisy.replace("win ", "won "),
        noisy.replace("  3    299", ""),
    ];
    for actual in wrong {
        if compare_sampled(expected, &actual).is_ok() {
            return Err(Failed::from(format!("accepted a wrong output:\n{actual}")));
        }
    }
    Ok(())
}

enum Run {
    Output(String),
    Error(String),
    Pending(String),
}

fn trial(path: PathBuf) -> Trial {
    let name = path.file_stem().unwrap().to_string_lossy().to_string();
    let src = std::fs::read_to_string(&path).unwrap();
    let file = SourceFile::new(path.display().to_string(), src.clone());
    let result = run(&file, &path);
    let listed = PENDING.contains(&name.as_str());
    let pending = listed && matches!(result, Run::Pending(_));
    Trial::test(name, move || {
        let Some(expected) = expected_output(&src) else {
            return Err(Failed::from("no `# ── Output` block"));
        };
        match result {
            Run::Output(_) if listed => Err(Failed::from("this example now runs: remove it from PENDING")),
            Run::Output(actual) if actual.starts_with("sample ·") => compare_sampled(&expected, &actual),
            Run::Output(actual) => compare(&expected, &actual),
            Run::Error(e) => Err(Failed::from(format!("the program failed:\n{e}"))),
            Run::Pending(reason) => Err(Failed::from(format!(
                "uses an unsupported feature but isn't listed in PENDING: {reason}"
            ))),
        }
    })
    .with_ignored_flag(pending)
}

fn run(file: &SourceFile, path: &Path) -> Run {
    let (program, diags) = probl_sema::compile(&file.text);
    let Some(program) = program else {
        return Run::Error(render_all(&diags, file, false));
    };
    // Data next to the example, as `probl run` reads it.
    let mut files = LocalFiles::next_to(path, None);
    let inputs = data::load(
        &program,
        &mut files,
        &mut Snapshots::default(),
        &InputLimits::default(),
        None,
    );
    let options = match inputs {
        Ok(inputs) => Options {
            today: probl_engine::dates::parse("2026-09-29"),
            inputs: Some(Arc::new(inputs)),
            ..Options::default()
        },
        Err(e) => return Run::Error(e.to_diagnostic().render(file, false)),
    };
    let mut print = |_: &str| {};
    match probl_engine::run(&program, &options, &mut print) {
        Ok(outcome) => Run::Output(outcome.output),
        Err(e) if e.kind == ErrorKind::Unsupported => Run::Pending(e.message),
        Err(e) => Run::Error(e.to_diagnostic().render(file, false)),
    }
}

/// The lines after `# ── Output`, without their `# ` prefix.
fn expected_output(src: &str) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    let start = lines.iter().position(|l| l.starts_with("# ── Output"))?;
    let mut out: Vec<&str> = lines[start + 1..]
        .iter()
        .map(|l| l.strip_prefix("# ").unwrap_or(l.strip_prefix('#').unwrap_or(l)))
        .collect();
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    Some(out.join("\n"))
}

/// Sampled output agrees with the expected output when every expected line
/// has a matching actual line, in order: the same words, with numbers that
/// agree within their error. An estimate printed as `p% ± e%` must be within
/// five of the larger error; other numbers within 4%; dates within three
/// days. The expected output may leave out rows of a table (and say so in a
/// line in parentheses), since the reference simulation's rows were copied
/// by hand.
fn compare_sampled(expected: &str, actual: &str) -> Result<(), Failed> {
    let is_note = |l: &&str| {
        let t = l.trim();
        t.starts_with('(') && t.ends_with(')') && t.contains("left out")
    };
    let expected: Vec<&str> = expected.lines().map(str::trim_end).filter(|l| !is_note(l)).collect();
    let actual: Vec<&str> = actual.lines().map(str::trim_end).collect();
    let row = |l: &str| l.starts_with("  ");
    let first = |l: &str| l.split_whitespace().next().map(str::to_string);
    let mut j = 0;
    for e in &expected {
        loop {
            let Some(a) = actual.get(j) else {
                return Err(Failed::from(format!(
                    "missing line: {e}\n\nactual output:\n{}",
                    actual.join("\n")
                )));
            };
            j += 1;
            match agree(e, a) {
                Ok(()) => break,
                // A table row the expected output leaves out.
                Err(_) if row(e) && row(a) && first(e) != first(a) => continue,
                Err(why) => {
                    return Err(Failed::from(format!(
                        "expected: {e}\nactual:   {a}\n{why}\n\nactual output:\n{}",
                        actual.join("\n")
                    )));
                }
            }
        }
    }
    if let Some(extra) = actual[j..].iter().find(|l| !l.trim().is_empty() && !row(l)) {
        return Err(Failed::from(format!("unexpected line: {extra}")));
    }
    Ok(())
}

fn agree(expected: &str, actual: &str) -> Result<(), String> {
    let (e, a): (Vec<&str>, Vec<&str>) = (
        expected.split_whitespace().collect(),
        actual.split_whitespace().collect(),
    );
    if e.len() != a.len() {
        return Err("different words".into());
    }
    for i in 0..e.len() {
        let (x, y) = (e[i], a[i]);
        if x == y {
            continue;
        }
        if let (Some(dx), Some(dy)) = (date(x), date(y)) {
            if (dx - dy).abs() <= 3 {
                continue;
            }
            return Err(format!("{x} and {y} are more than three days apart"));
        }
        let (Some(nx), Some(ny)) = (number(x), number(y)) else {
            return Err(format!("`{x}` differs from `{y}`"));
        };
        let error = |t: &[&str]| {
            t.get(i + 1)
                .filter(|s| **s == "±")
                .and(t.get(i + 2))
                .and_then(|s| number(s))
        };
        let tolerance = match (error(&e), error(&a)) {
            // An estimate: within five standard errors (and rounding).
            (Some(sx), Some(sy)) => 5.0 * sx.max(sy) + 0.05,
            // A standard error: a loose check.
            _ if i >= 1 && e[i - 1] == "±" => (0.5 * nx.abs().max(ny.abs())).max(0.1),
            // In scientific notation, relative to the number.
            _ if x.contains('e') || y.contains('e') => 0.04 * nx.abs().max(ny.abs()),
            _ => (0.04 * nx.abs().max(ny.abs())).max(0.05),
        };
        if (nx - ny).abs() > tolerance {
            return Err(format!("{x} and {y} differ by more than {tolerance}"));
        }
    }
    Ok(())
}

/// A number as printed: `1,234`, `-0.5`, `33.0%`, `(± 0.5%)`.
fn number(s: &str) -> Option<f64> {
    s.trim_matches(['(', ')'])
        .trim_end_matches('%')
        .replace(',', "")
        .parse()
        .ok()
}

/// A date as a day count, roughly (only differences matter).
fn date(s: &str) -> Option<i64> {
    let mut parts = s.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    // Days from a fixed epoch (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe)
}

fn compare(expected: &str, actual: &str) -> Result<(), Failed> {
    let norm = |s: &str| {
        s.lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string()
    };
    let (expected, actual) = (norm(expected), norm(actual));
    if expected == actual {
        return Ok(());
    }
    let mut msg = String::from("output differs (- expected, + actual):\n");
    let (e, a): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    for i in 0..e.len().max(a.len()) {
        match (e.get(i), a.get(i)) {
            (Some(x), Some(y)) if x == y => msg.push_str(&format!("  {x}\n")),
            (x, y) => {
                if let Some(x) = x {
                    msg.push_str(&format!("- {x}\n"));
                }
                if let Some(y) = y {
                    msg.push_str(&format!("+ {y}\n"));
                }
            }
        }
    }
    Err(Failed::from(msg))
}
