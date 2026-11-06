//! The playground's API, run natively (the same code runs in WebAssembly).

use probl::{MemoryFiles, Options};
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

#[test]
fn datetime_snapshots_normalize_replay_and_survive_partial_results() {
    for source in [
        "report now; report date(now)==today",
        "@on_error partial\nlet n~d2; report now; report 1/(n-1)",
    ] {
        let (out, _) = run(json!({"source":source,"now":"2027-01-01T00:15:00.123456789+01:00"}));
        assert_eq!(out["now"], "2026-12-31T23:15:00.123456789");
        assert_eq!(out["today"], "2026-12-31");
        assert_eq!(out["reports"]["run"]["now"], out["now"]);
        assert_eq!(out["reports"]["run"]["today"], out["today"]);
        let (replay, _) = run(json!({"source":source,"now":out["now"]}));
        assert_eq!(out["output"], replay["output"]);
        assert_eq!(out["reports"], replay["reports"]);
    }
    let (out, _) = run(json!({"source":"report now","today":"2024-02-29"}));
    assert_eq!(out["now"], "2024-02-29T00:00:00");
    for request in [
        json!({"source":"report now"}),
        json!({"source":"report now","now":null}),
        json!({"source":"report now","now":"2024-02-29"}),
        json!({"source":"report now","now":"2024-03-01T00:00:00","today":"2024-02-29"}),
    ] {
        assert!(run(request).0["error"].is_object());
    }
}

#[test]
fn the_environment_is_supplied_by_the_host() {
    let source = "report env.get(\"REGION\", \"eu\") as \"region\"";
    let (out, _) = run(json!({ "source": source, "env": { "REGION": "us" } }));
    assert!(out["output"].as_str().unwrap().contains("region    us"), "{out}");
    // The module has no environment of its own.
    let (out, _) = run(json!({ "source": source }));
    assert!(out["output"].as_str().unwrap().contains("region    eu"), "{out}");
    for bad in [json!({ "REGION": 1 }), json!(["REGION"]), json!("REGION=us")] {
        let (out, _) = run(json!({ "source": source, "env": bad }));
        assert_eq!(out["error"]["kind"], "language", "{bad}");
    }
}

#[test]
fn the_examples_print_what_the_command_line_prints() {
    let all = examples();
    assert_eq!(all.len(), 22);
    for example in all {
        let source = example["source"].as_str().unwrap();
        let (answer, _) = run(json!({ "source": source, "today": "2026-09-29", "files": example["files"] }));
        let output = answer["output"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: {answer}", example["name"]));
        // The library as `probl run` calls it, on every thread.
        let program = probl::compile("example.probl", source).unwrap();
        let mut options = Options::new().today("2026-09-29".parse().unwrap());
        if program.reads_data() {
            let mut files: MemoryFiles = example["files"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(path, text)| (path.clone(), text.as_str().unwrap()))
                .collect();
            let data = program.load(&mut files, &options).unwrap();
            options = options.data(data);
        }
        let expected = program.run(&options).unwrap();
        assert_eq!(output, expected.text(), "{}", example["name"]);
        let document: Value =
            serde_json::from_str(&expected.reports_json(&probl::ReportJsonLimits::default()).unwrap()).unwrap();
        assert_eq!(answer["reports"], document, "{}", example["name"]);
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
    assert_eq!(d["file"], "playground.probl");
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
    let progress: probl_wasm::Progress = Arc::new(move |done, total| sink.lock().unwrap().push((done, total)));
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
        "enum Market { Boom, Slump }\ntype Day = { visitors: int }\nfn f(d: Day): int {\n  return d.visitors\n}\n";
    let answer: Value = serde_json::from_str(&probl_wasm::check(src)).unwrap();
    let definitions = answer["symbols"]["definitions"].as_array().unwrap().clone();
    let find = |name: &str| definitions.iter().find(|d| d["name"] == name).unwrap().clone();
    assert_eq!(find("f")["detail"], "fn f(d: Day): int");
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
    assert!(keywords.iter().any(|k| k["name"] == "typeof"));
    assert!(keywords.iter().any(|k| k["name"] == "catch"));
    assert!(docs["read"]["summary"].as_str().unwrap().contains("CSV"));
    // What a `catch` can name, each with what it is.
    let faults: Vec<&str> = docs["faults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(!f["summary"].as_str().unwrap().is_empty(), "{f}");
            f["name"].as_str().unwrap()
        })
        .collect();
    let names: Vec<&str> = probl_sema::ir::Fault::ALL.iter().map(|f| f.name()).collect();
    assert_eq!(faults, names);
}

#[test]
fn a_partial_result_has_the_output_and_the_failures() {
    let source = "let x ~ d6 - 1\nlet y = 1 / x\nreport y";
    // Sampling is partial by default.
    let (out, _) = run(json!({ "source": source, "mode": "sample", "runs": 1000 }));
    assert_eq!(out["partial"], true);
    assert_eq!(out["finished"], true);
    assert!(out["output"].as_str().unwrap().contains("runs failed"), "{out}");
    assert_eq!(out["error"]["message"], "division by zero");
    assert_eq!(out["failures"].as_array().unwrap().len(), 1);
    assert_eq!(out["stats"]["runs"], 1000);
    // Enumeration is total, unless the request asks otherwise. The fault
    // that stops it says how often, and for which values.
    let (out, _) = run(json!({ "source": source }));
    assert!(out["output"].is_null() && out["partial"].is_null(), "{out}");
    assert_eq!(out["error"]["message"], "division by zero");
    assert_eq!(
        out["error"]["notes"],
        json!(["fails for 16.67% of the weight reaching this operation, for example when x = 0"])
    );
    let (out, _) = run(json!({ "source": source, "on_error": "partial" }));
    assert!(
        out["output"]
            .as_str()
            .unwrap()
            .starts_with("enumerated · partial result · 16.67% failed"),
        "{out}"
    );
    let (out, _) = run(json!({ "source": source, "mode": "sample", "on_error": "total" }));
    assert!(out["output"].is_null(), "{out}");
}

#[test]
fn structured_reports_match_the_library_including_partial_results() {
    for source in [
        "for year in 0..3 { report d6 by year; report d6>3 by year }",
        "@mode sample(runs: 100, seed: 31)\nlet x~normal(0,1); report x; report x<0",
        "let x~normal(0,1); report {a:x,b:x+2}",
        "@on_error partial\nlet n~d2; report 1/(n-1)",
        "for k in [9007199254740993,date(2026,10,19),\"1\",1] {report 2 by k}",
    ] {
        let (actual, _) = run(json!({"source":source,"today":"2026-09-29"}));
        let result = probl::compile("playground.probl", source)
            .unwrap()
            .run(&Options::new().today("2026-09-29".parse().unwrap()));
        let outcome = match &result {
            Ok(out) => out,
            Err(e) => e.partial().unwrap(),
        };
        let expected: Value =
            serde_json::from_str(&outcome.reports_json(&probl::ReportJsonLimits::default()).unwrap()).unwrap();
        assert_eq!(actual["reports"], expected, "{source}");
        assert!(actual.get("reports_error").is_none());
        assert_eq!(actual["output"], outcome.text());
    }
}

#[test]
fn structured_report_limits_keep_the_model_output_available() {
    let (answer, _) = run(json!({"source":"for k in 1..10000 { report true by k }"}));
    assert!(answer["output"].as_str().unwrap().lines().count() >= 10_000);
    assert!(answer.get("error").is_none());
    assert!(answer.get("reports").is_none());
    assert_eq!(answer["reports_error"]["kind"], "limit");
}

#[test]
fn summary_requests_match_the_library_and_validate_before_printing() {
    let source =
        "print(\"ran\"); for k in 0..2 { report one_of([0:50%,10:50%]) by k as \"same\" }; report true as \"same\"";
    let (answer, _) = run(json!({"source":source,"report_summaries":{"0":"q:0.5,median,mean"}}));
    let out = probl::compile("playground.probl", source)
        .unwrap()
        .run(&Options::new().report_summary(0, "q:0.5,median,mean".parse().unwrap()))
        .unwrap();
    let expected: Value =
        serde_json::from_str(&out.reports_json(&probl::ReportJsonLimits::default()).unwrap()).unwrap();
    assert_eq!(answer["reports"], expected);
    assert_eq!(answer["output"], out.text());
    let checked: Value = serde_json::from_str(&probl_wasm::check(source)).unwrap();
    assert_eq!(
        checked["reports"],
        json!([{"id":0,"label":"same"},{"id":1,"label":"same"}])
    );
    for selection in [
        json!([]),
        json!({"0":[]}),
        json!({"2":"mean"}),
        json!({"00":"mean"}),
        json!({"0":"q:NaN"}),
        json!({"0":"mean,mean"}),
    ] {
        let (answer, printed) = run(json!({"source":source,"report_summaries":selection}));
        assert!(answer.get("error").is_some(), "{answer}");
        assert!(printed.is_empty(), "{answer}");
    }
    let (partial, _) =
        run(json!({"source":"@on_error partial\nlet n~d2; report 1/(n-1)","report_summaries":{"0":"moments"}}));
    assert_eq!(partial["partial"], true);
    assert_eq!(
        partial["reports"]["reports"][0]["summary_selection"],
        json!(["mean", "sd"])
    );
    assert!(
        partial["reports"]["reports"][0]["groups"][0]["summary"]["quantiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn collection_requests_validate_and_mark_unavailable_quantiles() {
    let source = "@mode sample(runs:100,seed:42)\nprint(\"ran\"); let x~d6; report x";
    let (answer, _) = run(json!({"source":source,"report_collections":{"0":"moments"}}));
    assert!(answer.get("error").is_none(), "{answer}");
    assert_eq!(answer["reports"]["reports"][0]["collection"], "moments");
    assert_eq!(
        answer["reports"]["reports"][0]["groups"][0]["summary"]["quantile_method"],
        "unavailable"
    );
    for policy in [
        json!([]),
        json!({"0":null}),
        json!({"0":"bad"}),
        json!({"00":"moments"}),
        json!({"1":"moments"}),
    ] {
        let (answer, printed) = run(json!({"source":source,"report_collections":policy}));
        assert!(answer.get("error").is_some(), "{answer}");
        assert!(printed.is_empty());
    }
}

#[test]
fn quantile_collection_has_explicit_approximation_metadata() {
    let source = "@mode sample(runs:2101,seed:42)\nlet x~normal(0,1); report max(0.0,x)";
    let (answer, _) = run(json!({"source":source,"report_collections":{"0":"quantiles"}}));
    assert!(answer.get("error").is_none(), "{answer}");
    let report = &answer["reports"]["reports"][0];
    assert_eq!(report["collection"], "quantiles:512");
    let summary = &report["groups"][0]["summary"];
    assert_eq!(summary["quantile_method"], "compactor_v1");
    assert_eq!(summary["approximation"]["capacity"], 512);
    assert_eq!(summary["approximation"]["samples"], "2101");
    assert_eq!(summary["quantiles"][2]["convention"], "inverse_cdf");
    for policy in ["quantiles:0", "quantiles:4097"] {
        let (answer, printed) = run(json!({"source":source,"report_collections":{"0":policy}}));
        assert!(answer.get("error").is_some());
        assert!(printed.is_empty());
    }
    let (answer, _) = run(json!({"source":"@mode sample(runs:10)\nreport 0.0","report_collections":{"0":"quantiles"}}));
    assert_eq!(
        answer["reports"]["reports"][0]["groups"][0]["summary"]["deterministic"],
        true
    );
}

fn check(request: Value) -> Value {
    serde_json::from_str(&probl_wasm::check_program(&request.to_string())).unwrap()
}

const MAIN: &str = "import { walk, step as move, Day } from \"./lib/walk.probl\"\nlet d = Day { visitors: 3 }\nreport walk(2) + move + d.visitors\n";
const WALK: &str = "\
# One step, left or right.
export let step = one_of([-1, 1])
export type Day = { visitors: int }
# A walk of `turns` steps.
export fn walk(turns: int): int {
  var position = 0
  repeat turns { position += ~step }
  position
}
";

/// The text from UTF-16 offsets `from` and `to` in `text`.
fn slice(text: &str, from: &Value, to: &Value) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    String::from_utf16(&units[from.as_u64().unwrap() as usize..to.as_u64().unwrap() as usize]).unwrap()
}

#[test]
fn a_program_of_several_files_runs() {
    let request = json!({ "source": MAIN, "name": "main.probl", "sources": { "lib/walk.probl": WALK } });
    let (answer, _) = run(request.clone());
    assert!(
        answer["output"].as_str().unwrap().contains("walk(2) + move"),
        "{answer}"
    );
    // An error in an imported file says which file, and where in it.
    let broken = WALK.replace("position\n}", "position + nothing\n}");
    let (answer, _) = run(json!({ "source": MAIN, "name": "main.probl", "sources": { "lib/walk.probl": broken } }));
    let e = &answer["error"];
    assert_eq!(e["file"], "lib/walk.probl", "{answer}");
    assert_eq!(e["line"], 8);
    assert_eq!(slice(&broken, &e["from"], &e["to"]), "nothing");
    assert!(e["rendered"].as_str().unwrap().contains("lib/walk.probl:8:"), "{e}");
    // A file that isn't given can't be imported.
    let (answer, _) = run(json!({ "source": MAIN }));
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("there's no such file"),
        "{answer}"
    );
    // Nor can sources that aren't text.
    let (answer, _) = run(json!({ "source": MAIN, "sources": { "lib/walk.probl": 1 } }));
    assert_eq!(answer["error"]["kind"], "usage", "{answer}");
}

#[test]
fn checking_a_file_gives_its_own_names_and_what_they_import() {
    let sources = json!({ "lib/walk.probl": WALK });
    let answer = check(json!({ "source": MAIN, "name": "main.probl", "sources": sources }));
    assert_eq!(answer["diagnostics"], json!([]), "{answer}");
    assert_eq!(answer["files"], json!(["main.probl", "lib/walk.probl"]));
    let symbols = &answer["symbols"];
    let definitions = symbols["definitions"].as_array().unwrap();
    let named = |name: &str, file: Value| {
        definitions
            .iter()
            .position(|d| d["name"] == name && d["file"] == file)
            .unwrap_or_else(|| panic!("{name} in {file}: {symbols}"))
    };
    // `walk` is declared in the import, and stands for the function in the
    // other file, which describes itself there.
    let walk = named("walk", Value::Null);
    let d = &definitions[walk];
    assert_eq!(d["kind"], "function");
    assert_eq!(slice(MAIN, &d["from"], &d["to"]), "walk");
    assert_eq!(d["scope"], json!([0, MAIN.encode_utf16().count()]));
    let origin = &definitions[d["imported"].as_u64().unwrap() as usize];
    assert_eq!(origin["file"], "lib/walk.probl");
    assert_eq!(origin["line"], 5);
    assert_eq!(origin["detail"], "export fn walk(turns: int): int");
    assert_eq!(origin["doc"], "A walk of `turns` steps.");
    assert_eq!(origin["scope"], json!([-1, -1]));
    assert_eq!(slice(WALK, &origin["from"], &origin["to"]), "walk");
    // `step as move`: `move` is declared here, a global, and `step` names
    // the variable there.
    let local = named("move", Value::Null);
    assert_eq!(definitions[local]["global"], true);
    let step = named("step", json!("lib/walk.probl"));
    assert_eq!(definitions[local]["imported"], step);
    let references = symbols["references"].as_array().unwrap();
    let uses = |def: usize| -> Vec<String> {
        references
            .iter()
            .filter(|r| r[2] == def)
            .map(|r| slice(MAIN, &r[0], &r[1]))
            .collect()
    };
    assert_eq!(uses(walk), ["walk"]);
    assert_eq!(uses(local), ["move"]);
    assert_eq!(uses(step), ["step"]);
    // A field of an imported record is the other file's.
    let visitors = named("visitors", json!("lib/walk.probl"));
    assert_eq!(uses(visitors), ["visitors", "visitors"]);

    // The other file, checked as it's imported: its own names, and an
    // error where it reports.
    let answer = check(json!({
        "source": MAIN, "name": "main.probl", "file": "lib/walk.probl",
        "sources": { "lib/walk.probl": format!("{WALK}report walk(1)\n") },
    }));
    assert_eq!(answer["files"], json!(["lib/walk.probl"]));
    let d = &answer["diagnostics"][0];
    assert_eq!(d["file"], "lib/walk.probl");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("only allowed in the program's entry file"),
        "{d}"
    );
    let definitions = answer["symbols"]["definitions"].as_array().unwrap();
    assert!(definitions.iter().all(|d| d["file"].is_null()), "{definitions:?}");
    assert!(
        definitions
            .iter()
            .any(|d| d["name"] == "walk" && d["imported"].is_null())
    );
    // The entry, checked by its name, is checked as the entry.
    let answer = check(json!({ "source": "report 1", "name": "main.probl", "file": "main.probl" }));
    assert_eq!(answer["diagnostics"], json!([]), "{answer}");
    // Errors in a file it imports say which file.
    let broken = WALK.replace("position\n}", "position + nothing\n}");
    let answer = check(json!({ "source": MAIN, "name": "main.probl", "sources": { "lib/walk.probl": broken } }));
    let d = &answer["diagnostics"][0];
    assert_eq!(
        (d["file"].as_str(), d["line"].as_u64()),
        (Some("lib/walk.probl"), Some(8)),
        "{answer}"
    );
    // A file that isn't there can't be checked.
    let answer = check(json!({ "source": MAIN, "file": "nowhere.probl" }));
    assert_eq!(answer["error"]["kind"], "usage", "{answer}");
}
