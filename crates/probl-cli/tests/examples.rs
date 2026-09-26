//! Golden tests: every program in `examples/` must print exactly the output in
//! its `# ── Output` block. Examples that need features that aren't
//! implemented yet are reported as ignored (pending) instead of failing.

use libtest_mimic::{Arguments, Failed, Trial};
use probl_engine::Options;
use probl_syntax::{SourceFile, render_all};
use std::path::{Path, PathBuf};

fn main() {
    let args = Arguments::from_args();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("examples directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "probl"))
        .collect();
    paths.sort();
    let trials = paths.into_iter().map(trial).collect();
    libtest_mimic::run(&args, trials).exit();
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
    let result = run(&file);
    let pending = matches!(result, Run::Pending(_));
    Trial::test(name, move || {
        let Some(expected) = expected_output(&src) else {
            return Err(Failed::from("no `# ── Output` block"));
        };
        match result {
            Run::Output(actual) => compare(&expected, &actual),
            Run::Error(e) => Err(Failed::from(format!("the program failed:\n{e}"))),
            Run::Pending(reason) => Err(Failed::from(format!("pending: {reason}"))),
        }
    })
    .with_ignored_flag(pending)
}

fn run(file: &SourceFile) -> Run {
    let (program, diags) = probl_sema::compile(&file.text);
    let Some(program) = program else {
        return Run::Error(render_all(&diags, file, false));
    };
    let mut print = |_: &str| {};
    match probl_engine::run(&program, &Options::default(), &mut print) {
        Ok(outcome) => Run::Output(outcome.output),
        Err(e) if e.unsupported => Run::Pending(e.message),
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
