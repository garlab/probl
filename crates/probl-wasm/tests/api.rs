//! The playground's API, run natively (the same code runs in WebAssembly).

use probl_engine::Options;
use probl_engine::data::{self, Resolver, Snapshots};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn run(request: Value) -> (Value, Vec<String>) {
    let printed = Arc::new(Mutex::new(Vec::new()));
    let sink = printed.clone();
    let mut print = move |line: &str| sink.lock().unwrap().push(line.to_string());
    let answer = probl_wasm::run(&request.to_string(), &mut print, None);
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

#[test]
fn execution_date_is_supplied_by_the_host_and_returned_for_replay() {
    for stamp in ["2024-02-29", "2026-09-29"] {
        let (out, _) = run(json!({ "source": "report today", "today": stamp }));
        assert_eq!(out["today"], stamp);
        assert!(out["output"].as_str().unwrap().contains(stamp));
    }
    let (missing, _) = run(json!({ "source": "report today" }));
    assert_eq!(missing["error"]["kind"], "language");
    for bad in [json!("2026-02-29"), json!(123), json!(null)] {
        let (out, _) = run(json!({ "source": "report today", "today": bad }));
        assert_eq!(out["error"]["kind"], "language");
    }
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
    assert_eq!(all.len(), 13);
    for example in all {
        let source = example["source"].as_str().unwrap();
        let (answer, _) = run(json!({ "source": source, "today": "2026-09-29", "files": example["files"] }));
        let output = answer["output"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: {answer}", example["name"]));
        // The engine as `probl run` calls it, on every thread.
        let program = probl_sema::compile(source).0.unwrap();
        let mut options = Options {
            today: probl_engine::dates::parse("2026-09-29"),
            ..Options::default()
        };
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
fn sampling_says_how_far_it_is() {
    let told = Arc::new(Mutex::new(Vec::new()));
    let sink = told.clone();
    let progress = probl_engine::Progress(Arc::new(move |done, total| sink.lock().unwrap().push((done, total))));
    let request = json!({ "source": "report d6 > 4", "runs": 2500 }).to_string();
    probl_wasm::run(&request, &mut |_| {}, Some(progress));
    assert_eq!(*told.lock().unwrap(), [(1000, 2500), (2000, 2500), (2500, 2500)]);
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

#[test]
fn check_gives_the_names_with_their_comments() {
    let src = "# The rate before the pilot.\n# Probably a few percent.\nlet rate ~ beta(2, 40)   # prior\nlet s = \"é#\"  # not in the string\nreport rate * 100\n";
    let answer: Value = serde_json::from_str(&probl_wasm::check(src)).unwrap();
    let symbols = &answer["symbols"];
    let definitions = symbols["definitions"].as_array().unwrap();
    let rate = definitions.iter().find(|d| d["name"] == "rate").unwrap();
    assert_eq!(rate["kind"], "variable");
    assert_eq!(rate["line"], 3);
    assert_eq!(rate["detail"], "let rate ~ beta(2, 40)");
    assert_eq!(
        rate["doc"],
        "The rate before the pilot.\nProbably a few percent.\nprior"
    );
    let s = definitions.iter().find(|d| d["name"] == "s").unwrap();
    assert_eq!(s["detail"], "let s = \"é#\"");
    assert_eq!(s["doc"], "not in the string");
    // The use of `rate` in the report, after a non-ASCII character: offsets
    // count UTF-16 code units.
    let rate_index = definitions.iter().position(|d| d["name"] == "rate").unwrap();
    let reference = symbols["references"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[2] == rate_index)
        .unwrap();
    let units: Vec<u16> = src.encode_utf16().collect();
    let (from, to) = (
        reference[0].as_u64().unwrap() as usize,
        reference[1].as_u64().unwrap() as usize,
    );
    assert_eq!(String::from_utf16(&units[from..to]).unwrap(), "rate");
    assert!(from > src.find("report").unwrap(), "{from}");
    // A program that doesn't parse has no symbols.
    let answer: Value = serde_json::from_str(&probl_wasm::check("let x = (")).unwrap();
    assert_eq!(answer["symbols"], Value::Null);
}

#[test]
fn functions_fields_and_variants_describe_themselves() {
    let src =
        "enum Market { Boom, Slump }\ntype Day = { visitors: int }\nfn f(d: Day) -> int {\n  return d.visitors\n}\n";
    let answer: Value = serde_json::from_str(&probl_wasm::check(src)).unwrap();
    let definitions = answer["symbols"]["definitions"].as_array().unwrap().clone();
    let find = |name: &str| definitions.iter().find(|d| d["name"] == name).unwrap().clone();
    assert_eq!(find("f")["detail"], "fn f(d: Day) -> int");
    assert_eq!(find("d")["kind"], "parameter");
    assert_eq!(find("visitors")["detail"], "Day.visitors: int");
    assert_eq!(find("Slump")["owner"], "Market");
    assert_eq!(answer["symbols"]["functions"].as_array().unwrap().len(), 1);
}

#[test]
fn the_reference_documents_every_built_in() {
    let docs: Value = serde_json::from_str(&probl_wasm::docs()).unwrap();
    let builtins = docs["builtins"].as_array().unwrap();
    let public = probl_sema::Builtin::ALL.iter().filter(|b| b.is_public()).count();
    assert_eq!(builtins.len(), public);
    let binomial = builtins.iter().find(|b| b["name"] == "binomial").unwrap();
    assert_eq!(binomial["category"], "Distributions");
    assert_eq!(binomial["signature"], "binomial(n: int, p: prob) -> dist[int]");
    for b in builtins {
        assert!(!b["summary"].as_str().unwrap().is_empty(), "{b}");
    }
    let constants = docs["constants"].as_array().unwrap();
    assert_eq!(constants.len(), probl_sema::Constant::ALL.len());
    let pi = constants.iter().find(|c| c["name"] == "pi").unwrap();
    assert_eq!(pi["signature"], "pi = 3.141592653589793");
    let keywords = docs["keywords"].as_array().unwrap();
    assert!(keywords.iter().any(|k| k["name"] == "observe"));
    assert!(docs["read"]["summary"].as_str().unwrap().contains("CSV"));
}
