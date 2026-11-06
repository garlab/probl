//! Imports (docs/design/imports-and-tests.md): definitions are static, and a
//! module's top level runs once per run, in the program's worlds, before the
//! files that import it.

use probl_engine::value::Value;
use probl_engine::{Options, run};
use probl_sema::{Loader, Source, compile_program};
use std::collections::HashMap;

/// Files in memory, by path from the program's directory.
struct Files(HashMap<&'static str, &'static str>);

impl Loader for Files {
    fn load(&mut self, importer: &str, path: &str) -> Result<Source, String> {
        let dir = importer.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
        for part in path.split('/') {
            match part {
                "." | "" => {}
                ".." => {
                    parts.pop();
                }
                part => parts.push(part),
            }
        }
        let identity = parts.join("/");
        let text = self.0.get(identity.as_str()).ok_or("there's no such file")?;
        Ok(Source {
            identity: identity.clone(),
            name: identity,
            text: text.to_string(),
        })
    }
}

/// Compile `main.probl` with the other files, or say what's wrong.
fn compile(files: &[(&'static str, &'static str)]) -> Result<probl_sema::ir::Program, Vec<String>> {
    let mut files = Files(files.iter().copied().collect());
    let entry = Source {
        identity: "main.probl".into(),
        name: "main.probl".into(),
        text: files.0["main.probl"].to_string(),
    };
    let compiled = compile_program(entry, &mut files);
    let errors: Vec<String> = compiled
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| {
            let (file, _) = compiled.sources.locate(d.span).expect("a located diagnostic");
            let help = d.help.as_deref().map(|h| format!(" (help: {h})")).unwrap_or_default();
            format!("{}: {}{help}", file.name, d.message)
        })
        .collect();
    compiled.program.ok_or(errors)
}

fn outcome(files: &[(&'static str, &'static str)]) -> (probl_engine::Outcome, Vec<String>) {
    let program = compile(files).unwrap_or_else(|e| panic!("compile errors: {e:#?}"));
    let mut lines = Vec::new();
    let mut print = |line: &str| lines.push(line.to_string());
    let out = run(&program, &Options::default(), &mut print).unwrap_or_else(|e| panic!("{}", e.message));
    (out, lines)
}

fn errors(files: &[(&'static str, &'static str)]) -> String {
    compile(files).expect_err("compile errors").join("\n")
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

const PRIOR: &str = "export let rate ~ one_of([0.2, 0.5, 0.8])\n";
const SHOP_A: &str = "import { rate } from \"./prior.probl\"\nobserve 4 from binomial(5, rate)\nexport let next_week ~ binomial(10, rate)\n";
const SHOP_B: &str = "import { rate } from \"./prior.probl\"\nobserve 4 from binomial(5, rate)\nexport let next_week ~ binomial(10, rate)\n";

#[test]
fn observations_in_several_modules_update_one_shared_draw() {
    let main = "import { rate } from \"./prior.probl\"
import { next_week as a } from \"./shop_a.probl\"
import { next_week as b } from \"./shop_b.probl\"
report rate == 0.8
report a + b";
    let (out, _) = outcome(&[
        ("main.probl", main),
        ("prior.probl", PRIOR),
        ("shop_a.probl", SHOP_A),
        ("shop_b.probl", SHOP_B),
    ]);
    // As the same code written out in one file, in evaluation order.
    let inline = "let rate ~ one_of([0.2, 0.5, 0.8])
observe 4 from binomial(5, rate)
let a ~ binomial(10, rate)
observe 4 from binomial(5, rate)
let b ~ binomial(10, rate)
report rate == 0.8
report a + b";
    let (program, _) = probl_sema::compile(inline);
    let expected = run(&program.unwrap(), &Options::default(), &mut |_| {}).unwrap();
    let likelihood = |r: f64| 5.0 * r.powi(4) * (1.0 - r);
    let weights: Vec<f64> = [0.2, 0.5, 0.8].iter().map(|&r| likelihood(r).powi(2)).collect();
    close(
        out.reports[0].chance().unwrap(),
        weights[2] / weights.iter().sum::<f64>(),
    );
    assert_eq!(out.output, expected.output);
}

#[test]
fn a_module_shared_by_two_importers_runs_once() {
    let (out, lines) = outcome(&[
        (
            "main.probl",
            "import { ra } from \"./a.probl\"\nimport { rb } from \"./b.probl\"\nprint(\"main\")\nreport ra == rb",
        ),
        (
            "a.probl",
            "import { roll } from \"./common.probl\"\nprint(\"a\")\nexport let ra = roll",
        ),
        (
            "b.probl",
            "import { roll } from \"./common.probl\"\nprint(\"b\")\nexport let rb = roll",
        ),
        ("common.probl", "print(\"common\")\nexport let roll ~ d2"),
    ]);
    close(out.reports[0].chance().unwrap(), 1.0);
    // Dependencies first, each once per world: common's draw splits the
    // worlds after it prints.
    assert_eq!(
        lines,
        [
            "common",
            "[50%] a",
            "[50%] a",
            "[50%] b",
            "[50%] b",
            "[50%] main",
            "[50%] main"
        ]
    );
}

#[test]
fn definitions_are_static_and_functions_read_their_own_module() {
    let lib = "let scale = 10
var count = 1
count += 1
export var total = count * scale
export fn scaled(x) { x * scale }
export fn walk(turns) {
  var position = 0
  repeat turns { position += ~step }
  position
}
export let step = one_of([-1, 1])
export type Fighter = { hp: int, ac: int }
export enum Side { Left, Right }
export fn hit(f: Fighter): Fighter { f with { hp: f.hp - 1 } }";
    let main = "import { scaled, walk, step as move, total, Fighter, Side, hit } from \"./lib.probl\"
let scale = 3
report scaled(2)
report total
report walk(2)
report move
let f = Fighter { hp: 3, ac: 1 }
report hit(f).hp
report Side.Right
var mine = total
mine += 1
report mine";
    let (out, _) = outcome(&[("main.probl", main), ("lib.probl", lib)]);
    let values: Vec<Vec<(Value, f64)>> = out.reports.iter().map(|r| r.distribution()).collect();
    // `scaled` reads its own module's `scale`, not the importer's.
    assert_eq!(values[0], [(Value::Int(20.into()), 1.0)]);
    // An exported `var` exposes its final value.
    assert_eq!(values[1], [(Value::Int(20.into()), 1.0)]);
    // A function that draws runs in the caller's worlds.
    assert_eq!(values[2].len(), 3);
    // A recipe exported as a value stays a recipe.
    assert_eq!(values[3].len(), 2);
    assert_eq!(values[4], [(Value::Int(2.into()), 1.0)]);
    assert_eq!(values[6], [(Value::Int(21.into()), 1.0)]);
}

#[test]
fn private_names_stay_in_their_file() {
    let (out, _) = outcome(&[
        (
            "main.probl",
            "import { a } from \"./a.probl\"\nimport { b } from \"./b.probl\"\nlet helper = 3\nfn f() { 3 }\nreport a() + b() + helper + f()",
        ),
        ("a.probl", "let helper = 1\nfn f() { helper }\nexport fn a() { f() }"),
        ("b.probl", "let helper = 2\nfn f() { helper }\nexport fn b() { f() }"),
    ]);
    assert_eq!(out.reports[0].distribution(), [(Value::Int(9.into()), 1.0)]);
}

#[test]
fn import_errors_say_what_to_do() {
    let cases: &[(&[(&'static str, &'static str)], &str)] = &[
        (
            &[("main.probl", "import { x } from \"./missing.probl\"")],
            "main.probl: can't import \"./missing.probl\": there's no such file",
        ),
        (
            &[
                ("main.probl", "import { x } from \"./a.probl\""),
                ("a.probl", "import { y } from \"./b.probl\"\nexport let x = 1"),
                ("b.probl", "import { x } from \"./a.probl\"\nexport let y = 1"),
            ],
            "b.probl: this import makes a cycle: a.probl → b.probl → a.probl",
        ),
        (
            &[("main.probl", "import { x } from \"./main.probl\"")],
            "main.probl: this import makes a cycle: main.probl → main.probl",
        ),
        (
            &[
                ("main.probl", "import { hidden } from \"./a.probl\""),
                ("a.probl", "let hidden = 1"),
            ],
            "main.probl: `hidden` isn't exported by \"./a.probl\" (help: add `export` before its definition there)",
        ),
        (
            &[
                ("main.probl", "import { totl } from \"./a.probl\""),
                ("a.probl", "export let total = 1"),
            ],
            "main.probl: \"./a.probl\" doesn't define `totl` (help: did you mean `total`?)",
        ),
        (
            &[
                ("main.probl", "import { x, x } from \"./a.probl\""),
                ("a.probl", "export let x = 1"),
            ],
            "`x` is imported twice",
        ),
        (
            &[
                ("main.probl", "import { x } from \"./a.probl\"\nlet x = 2"),
                ("a.probl", "export let x = 1"),
            ],
            "main.probl: `x` is imported",
        ),
        (
            &[
                ("main.probl", "import { f } from \"./a.probl\"\nfn f() { 1 }"),
                ("a.probl", "export fn f() { 2 }"),
            ],
            "main.probl: `f` is already defined in this file",
        ),
        (
            &[
                ("main.probl", "import { x } from \"./a.probl\"\nx = 2"),
                ("a.probl", "export var x = 1"),
            ],
            "main.probl: can't assign to `x`: it's imported, and imports are read-only",
        ),
        (
            &[
                ("main.probl", "import { x } from \"./a.probl\""),
                ("a.probl", "export let x = 1\nreport x"),
            ],
            "a.probl: `report` is only allowed in the program's entry file",
        ),
        (
            &[
                ("main.probl", "import { x } from \"./a.probl\""),
                ("a.probl", "@mode enumerate\nexport let x = 1"),
            ],
            "a.probl: pragmas belong in the program's entry file",
        ),
        (
            &[
                ("main.probl", "let y = 1\nimport { x } from \"./a.probl\""),
                ("a.probl", "export let x = 1"),
            ],
            "main.probl: imports go before everything else in a file",
        ),
        (
            &[("main.probl", "import \"./a.probl\""), ("a.probl", "export let x = 1")],
            "main.probl: a bare `import` isn't supported yet",
        ),
        (
            &[
                ("main.probl", "import { x } from \"./a.probl\""),
                ("a.probl", "export let [x, y] = [1, 2]"),
            ],
            "a.probl: `export` names one value, function or type",
        ),
    ];
    for (files, expected) in cases {
        let e = errors(files);
        assert!(e.contains(expected), "expected {expected:?} in:\n{e}");
    }
}

#[test]
fn a_program_compiled_on_its_own_cant_import() {
    let (program, diags) = probl_sema::compile("import { x } from \"./a.probl\"\nreport x");
    assert!(program.is_none());
    assert!(diags[0].message.contains("can't import"), "{:?}", diags[0].message);
}

#[test]
fn a_module_reads_data_relative_to_itself() {
    let program = compile(&[
        (
            "main.probl",
            "import { x } from \"./lib/a.probl\"\nlet y: list[int] = read(\"y.json\")\nreport x",
        ),
        ("lib/a.probl", "export let x: list[int] = read(\"x.json\")"),
    ])
    .unwrap();
    let paths: Vec<&str> = program.inputs.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["lib/x.json", "y.json"]);
}
