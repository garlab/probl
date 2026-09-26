//! Programs that read data (docs/data-input.md): the loaded values are the
//! same in every world and every run, whatever the mode and the threads.

use probl_engine::data::{InputLimits, Inputs, Resolver, Snapshots, load};
use probl_engine::{Limits, Options};
use std::io::{Cursor, Read};
use std::sync::Arc;

struct Files(Vec<(&'static str, String)>);

impl Resolver for Files {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        Ok(path.to_string())
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        let (_, text) = self.0.iter().find(|(p, _)| *p == identity).ok_or("no such file")?;
        Ok(Box::new(Cursor::new(text.clone().into_bytes())))
    }
}

fn run(src: &str, files: Vec<(&'static str, String)>, threads: usize) -> String {
    let (program, diags) = probl_sema::compile(src);
    let program = program.unwrap_or_else(|| panic!("{diags:?}"));
    let inputs: Inputs = load(
        &program,
        &mut Files(files),
        &mut Snapshots::default(),
        &InputLimits::default(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let options = Options {
        inputs: Some(Arc::new(inputs)),
        limits: Limits {
            max_threads: threads,
            ..Limits::default()
        },
        ..Options::default()
    };
    let mut print = |_: &str| {};
    probl_engine::run(&program, &options, &mut print)
        .unwrap_or_else(|e| panic!("{}", e.message))
        .output
}

fn pilot() -> Vec<(&'static str, String)> {
    let mut csv = String::from("day,visitors,signups\n");
    let counts = [9, 6, 11, 7, 8, 12, 5, 9, 10, 7, 8, 11, 6, 9];
    for (i, n) in counts.iter().enumerate() {
        csv.push_str(&format!("2026-09-{:02},250,{n}\n", i + 1));
    }
    vec![("pilot.csv", csv)]
}

const PILOT: &str = r#"
type Day = { day: date, visitors: int, signups: int }
let pilot: list[Day] = read("pilot.csv")
"#;

#[test]
fn data_is_a_constant_when_enumerating() {
    let src =
        format!("{PILOT}report len(pilot) as \"days\"\nlet d ~ one_of(pilot)\nreport d.signups >= 10 as \"good day\"");
    let out = run(&src, pilot(), 1);
    assert!(out.contains("days") && out.contains("14"), "{out}");
    assert!(out.contains("28.57%"), "{out}");
}

#[test]
fn data_is_the_same_in_every_batch() {
    let src = format!(
        "@mode sample(runs: 20_000, seed: 3){PILOT}\
         let rate ~ beta(2, 40)\n\
         for row in pilot {{\n  observe row.signups from binomial(row.visitors, rate)\n}}\n\
         report rate * 100 as \"conversion rate (%)\""
    );
    let one = run(&src, pilot(), 1);
    assert!(one.contains("effective sample size"), "{one}");
    for threads in [2, 8] {
        assert_eq!(run(&src, pilot(), threads), one, "{threads} threads");
    }
}

#[test]
fn a_program_that_reads_data_needs_it_loaded() {
    let (program, _) = probl_sema::compile(PILOT);
    let mut print = |_: &str| {};
    let e = probl_engine::run(&program.unwrap(), &Options::default(), &mut print).unwrap_err();
    assert!(e.message.contains("reads data, which wasn't loaded"), "{}", e.message);
    // Inputs loaded for another program don't fit.
    let other = probl_sema::compile("let pilot: list[{ day: date }] = read(\"pilot.csv\")")
        .0
        .unwrap();
    let inputs = load(
        &other,
        &mut Files(pilot()),
        &mut Snapshots::default(),
        &InputLimits::default(),
        None,
    )
    .unwrap();
    let options = Options {
        inputs: Some(Arc::new(inputs)),
        ..Options::default()
    };
    let e = probl_engine::run(&probl_sema::compile(PILOT).0.unwrap(), &options, &mut print).unwrap_err();
    assert!(e.notes.iter().any(|n| n.contains("a different program")), "{:?}", e);
}
