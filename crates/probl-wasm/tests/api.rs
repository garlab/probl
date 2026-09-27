//! The playground's API, run natively (the same code runs in WebAssembly).

use probl_engine::Options;
use probl_engine::data::{self, Resolver, Snapshots};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn run(request: Value) -> (Value, Vec<String>) {
    let printed = Arc::new(Mutex::new(Vec::new()));
    let sink = printed.clone();
    let mut print = move |line: &str| sink.lock().unwrap().push(line.to_string());
    let answer = probl_wasm::run(&request.to_string(), &mut print);
    let lines = printed.lock().unwrap().clone();
    (serde_json::from_str(&answer).unwrap(), lines)
}

fn examples() -> Vec<Value> {
    serde_json::from_str::<Value>(&probl_wasm::examples())
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

/// The files of an example, for the engine's own loader.
struct Given(serde_json::Map<String, Value>);

impl Resolver for Given {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        Ok(path.to_string())
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn std::io::Read + Send>, String> {
        let text = self.0[identity].as_str().unwrap().as_bytes().to_vec();
        Ok(Box::new(std::io::Cursor::new(text)))
    }
}

#[test]
fn the_examples_print_what_the_command_line_prints() {
    let all = examples();
    assert_eq!(all.len(), 9);
    for example in all {
        let source = example["source"].as_str().unwrap();
        let (answer, _) = run(json!({ "source": source, "files": example["files"] }));
        let output = answer["output"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: {answer}", example["name"]));
        // The engine as `probl run` calls it, on every thread.
        let program = probl_sema::compile(source).0.unwrap();
        let mut options = Options::default();
        if !program.inputs.is_empty() {
            let mut files = Given(example["files"].as_object().unwrap().clone());
            let inputs = data::load(
                &program,
                &mut files,
                &mut Snapshots::default(),
                &Default::default(),
                None,
            )
            .unwrap();
            options.inputs = Some(Arc::new(inputs));
        }
        let expected = probl_engine::run(&program, &options, &mut |_| {}).unwrap().output;
        assert_eq!(output, expected, "{}", example["name"]);
    }
}

#[test]
fn examples_have_titles_and_their_data() {
    let all = examples();
    assert_eq!(all[1]["name"], "02_craps");
    assert_eq!(
        all[1]["title"],
        "Craps, pass line bet: what's the chance of winning, and how long does a bet last?"
    );
    assert_eq!(all[7]["files"].as_object().unwrap().len(), 1);
    assert!(all[0]["files"].as_object().unwrap().is_empty());
}

#[test]
fn diagnostics_say_where_in_javascript_terms() {
    // "é" is two bytes in UTF-8, one unit in UTF-16.
    let src = "let s = \"é\"\nlet x = y + 1\n";
    let answer: Value = serde_json::from_str(&probl_wasm::check(src)).unwrap();
    let d = &answer["diagnostics"][0];
    assert_eq!(d["severity"], "error");
    assert!(d["message"].as_str().unwrap().contains("`y`"), "{d}");
    assert_eq!(d["line"], 2);
    let from = d["from"].as_u64().unwrap() as usize;
    let units: Vec<u16> = src.encode_utf16().collect();
    assert_eq!(String::from_utf16(&units[from..from + 1]).unwrap(), "y");
    assert!(d["rendered"].as_str().unwrap().contains("playground.probl:2"), "{d}");
    // A program that compiles has none.
    let answer: Value = serde_json::from_str(&probl_wasm::check("report d6")).unwrap();
    assert_eq!(answer["diagnostics"], json!([]));
}

#[test]
fn what_programs_print_comes_as_it_happens() {
    let (answer, lines) = run(json!({ "source": "print(\"hello\")\nreport d6 > 4" }));
    assert_eq!(lines, ["hello"]);
    assert!(answer["output"].as_str().unwrap().contains("33.33%"));
    assert_eq!(answer["stats"]["peak_worlds"], 1);
}

#[test]
fn modes_are_chosen_as_on_the_command_line() {
    let (answer, _) = run(json!({ "source": "report d6 > 4", "runs": 1000, "seed": 3 }));
    assert!(
        answer["output"]
            .as_str()
            .unwrap()
            .starts_with("sample · 1,000 runs · seed 3"),
        "{answer}"
    );
    assert_eq!(answer["stats"]["runs"], 1000);
    let src = "@mode sample(runs: 500, seed: 2)\nreport d6 > 4";
    let (answer, _) = run(json!({ "source": src, "mode": "enumerate" }));
    assert!(answer["output"].as_str().unwrap().starts_with("enumerated"), "{answer}");
    let (answer, _) = run(json!({ "source": src, "seed": 9 }));
    assert!(
        answer["output"]
            .as_str()
            .unwrap()
            .starts_with("sample · 500 runs · seed 9"),
        "{answer}"
    );
}

#[test]
fn programs_read_only_the_files_given() {
    let src = "type Row = { n: int }\nlet rows: list[Row] = read(\"data/rows.csv\")\nreport len(rows)";
    let (answer, _) = run(json!({ "source": src, "files": { "data/rows.csv": "n\n1\n2\n" } }));
    assert!(
        answer["output"].as_str().unwrap().contains("len(rows)    2"),
        "{answer}"
    );
    let (answer, _) = run(json!({ "source": src }));
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("isn't one of the playground's files"),
        "{answer}"
    );
    let stdin = "let lines: list[str] = read(\"-\", format: \"lines\")\nreport len(lines)";
    let (answer, _) = run(json!({ "source": stdin }));
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no standard input"),
        "{answer}"
    );
}

#[test]
fn errors_say_what_kind_and_where() {
    let src = "let xs = [1, 2]\nreport xs[5]";
    let (answer, _) = run(json!({ "source": src }));
    let e = &answer["error"];
    assert_eq!(e["kind"], "language");
    assert_eq!(e["line"], 2);
    assert!(e["message"].as_str().unwrap().contains("out of range"), "{e}");
    // A compile error comes back as the error, with every diagnostic.
    let (answer, _) = run(json!({ "source": "report y" }));
    assert_eq!(answer["error"]["severity"], "error");
    assert_eq!(answer["diagnostics"].as_array().unwrap().len(), 1);
    // The playground's limits are lower than the command line's.
    let src = "var xs = []\nrepeat 10 {\n  let r ~ d6\n  xs.push(r)\n}\nreport len(xs)";
    let (answer, _) = run(json!({ "source": src }));
    assert_eq!(answer["error"]["kind"], "limit", "{answer}");
}
