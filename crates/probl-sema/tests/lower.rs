use probl_sema::{analyze, compile, pretty};
use probl_syntax::{SourceFile, render_all};
use std::path::Path;

fn ir(src: &str) -> String {
    let (program, diags) = compile(src);
    let file = SourceFile::new("test.probl", src);
    let Some(program) = program else {
        panic!("unexpected errors:\n{}", render_all(&diags, &file, false));
    };
    let liveness = analyze(&program);
    pretty::program(&program, Some(&liveness))
}

fn errors(src: &str) -> String {
    let (program, diags) = compile(src);
    assert!(program.is_none(), "expected errors for {src:?}");
    render_all(&diags, &SourceFile::new("test.probl", src), false)
}

#[test]
fn all_examples_lower() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "probl"))
        .collect();
    paths.sort();
    for path in paths {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        insta::assert_snapshot!(name, ir(&src));
    }
}

#[test]
fn if_expressions_and_calls_are_hoisted() {
    insta::assert_snapshot!(ir(
        "fn f(x) { if (chance { 50% => true, else => false }) { x } else { x + 1 } }\n\
         var pos = 0\n\
         pos += if (chance { 30% => true, else => false }) { 1 } else { -1 }\n\
         let y = f(pos) * 2\n\
         report y"
    ));
}

#[test]
fn and_with_a_call_on_the_right_only_runs_it_when_needed() {
    insta::assert_snapshot!(ir("fn check(x) { x > 3 }\n\
         let a ~ d6\n\
         let ok = a > 1 and check(a)\n\
         report ok"));
}

#[test]
fn functions_capture_the_globals_their_callees_need() {
    insta::assert_snapshot!(ir("let limit = 10\n\
         let bonus = 2\n\
         fn inner(x) { x + bonus }\n\
         fn outer(x) { if x > limit { 0 } else { inner(x) } }\n\
         let s = simulate { outer(d20) }\n\
         report s"));
}

#[test]
fn match_with_and_without_guards() {
    insta::assert_snapshot!(ir("enum Weather { Sun, Rain }\n\
         let w ~ one_of([Sun, Rain])\n\
         let mood = match w { Sun => 1, Rain => -1 }\n\
         let n ~ d6\n\
         let size = match n { 1 | 2 => \"small\", x if x > 4 => \"big\", _ => \"medium\" }\n\
         report mood\n\
         report size"));
}

#[test]
fn static_errors() {
    insta::assert_snapshot!(errors("let rolls = 1\nreport rols"));
    insta::assert_snapshot!(errors("let x = 1\nx = 2"));
    insta::assert_snapshot!(errors("var total = 0\nfn add(n) { total += n }"));
    insta::assert_snapshot!(errors("fn f() { report 1 }"));
    insta::assert_snapshot!(errors("for i in 1..3 { report i }"));
    insta::assert_snapshot!(errors("break\nfn g() { return }\nreturn 1"));
    insta::assert_snapshot!(errors("type P = { x: int, y: int }\nlet p = P { x: 1, z: 2 }"));
    insta::assert_snapshot!(errors("fn f(a, b) { a }\nlet v = f(1)\nlet w = max()"));
    insta::assert_snapshot!(errors("var deck = bag([1: 2])\nlet c = deck.take()"));
    insta::assert_snapshot!(errors("let d = if 30% { 1 }"));
}

#[test]
fn rules_from_the_semantics() {
    // No observation may follow a report (docs/semantics.md, section 7).
    insta::assert_snapshot!(errors("let a ~ d6\nreport a\nobserve a > 2"));
    insta::assert_snapshot!(errors(
        "fn check(x) { observe x > 1\n x }\nlet a ~ d6\nreport a\nlet b = check(a)"
    ));
    // The mode formerly called `exact`.
    insta::assert_snapshot!(errors("@mode exact\nreport 1"));
    // Declared types.
    insta::assert_snapshot!(errors("let p: prob = \"x\"\nlet q: probability = 1\nlet r: list = [1]"));
}

#[test]
fn reports_in_loops_are_classified() {
    let (program, _) = compile(
        "report 1 as \"once\"\n\
         for i in 1..3 { report i by i as \"per key\" }\n\
         for i in 1..3 { report i by 1 as \"per visit\" }\n\
         var n = 0\nwhile n < 3 { n += 1\n report n by n as \"while\" }",
    );
    let kinds: Vec<_> = program
        .unwrap()
        .reports
        .iter()
        .map(|r| (r.label.clone(), r.kind))
        .collect();
    use probl_sema::ir::ReportKind::*;
    assert_eq!(
        kinds,
        vec![
            ("once".to_string(), Once),
            ("per key".to_string(), PerKey),
            ("per visit".to_string(), PerVisit),
            ("while".to_string(), PerVisit),
        ]
    );
}

#[test]
fn effects_decide_memoization() {
    let (program, _) = compile(
        "fn noisy() { print(\"hi\")\n 1 }\n\
         fn caller() { noisy() }\n\
         fn quiet(x) { x + 1 }\n\
         fn looks(x) { observe x > 0\n x }\n\
         let a = caller() + quiet(1) + looks(1)",
    );
    let program = program.unwrap();
    let effects = |name: &str| program.functions.iter().find(|f| f.name == name).unwrap().effects;
    assert!(effects("noisy").prints && effects("caller").prints);
    assert!(!effects("quiet").prints && !effects("quiet").observes);
    assert!(effects("looks").observes && !effects("looks").prints);
}

#[test]
fn slots_die_where_they_are_last_used() {
    let src = "let a ~ d6\nlet unused = a * 2\nif a > 3 { report true }\nlet b = a + 1\nreport b";
    let (program, _) = compile(src);
    let program = program.expect("compiles");
    let liveness = analyze(&program);
    let main = program.main();
    // What dies in each top-level statement, by name.
    let dies: Vec<Vec<&str>> = main
        .body
        .stmts
        .iter()
        .map(|stmt| {
            liveness.dies[stmt.id as usize]
                .iter()
                .map(|&s| main.slots[s as usize].name.as_str())
                .collect()
        })
        .collect();
    assert_eq!(
        dies,
        [
            vec![],         // a ~ d6: read later
            vec!["unused"], // written, never read
            vec![],         // the `if` reads a, which is read later
            vec!["a"],      // b = a + 1: the last read of a
            vec!["b"],      // report b
        ]
    );
}

#[test]
fn read_is_checked_when_compiling() {
    let fails = |src: &str, expected: &str| {
        let e = errors(src);
        assert!(e.contains(expected), "{expected:?} isn't in:\n{e}");
    };
    // Where, and how.
    fails(
        "fn f() { let x: list[int] = read(\"a.txt\")\nx }",
        "must be at the top level",
    );
    fails(
        "if true { let x: list[int] = read(\"a.txt\") }",
        "must be at the top level",
    );
    fails(
        "report len(read(\"a.txt\"))",
        "must be the whole value of a `let` with a type",
    );
    fails(
        "let x = read(\"a.csv\")",
        "say what the data is: `let x: list[Row] = read(…)`",
    );
    fails("let x: list[int] ~ read(\"a.txt\")", "bound with `=`, not `~`");
    fails(
        "let m = \"a\"\nlet x: list[int] = read(\"{m}.txt\")",
        "the path must be written out",
    );
    fails(
        "let x: list[int] = read(\"a.xlsx\")",
        "can't tell the format of `a.xlsx`",
    );
    fails("let x: list[int] = read(\"a.dat\", format: \"xml\")", "the format is");
    fails(
        "let x: list[int] = read(\"a.txt\", sep: \";\")",
        "`read` has no argument `sep`",
    );
    fails(
        "let x: list[int] = read(\"-\")\nlet y: list[int] = read(\"-\")",
        "standard input can only be read once",
    );
    // What each format reads as.
    fails(
        "let x: list[int] = read(\"a.csv\")",
        "a CSV file reads as a list of records",
    );
    fails(
        "type R = { tags: list[str] }\nlet x: list[R] = read(\"a.csv\")",
        "a CSV cell holds a single value, but `tags` is a `list[str]`",
    );
    fails(
        "let x: list[list[int]] = read(\"a.txt\")",
        "lines read as a list of single values",
    );
    fails(
        "let m: map[{ a: int }, int] = read(\"a.json\")",
        "a map read from JSON has text keys",
    );
    // Types data can't have, even deep in named types: the error is at the field.
    let e =
        errors("type Inner = { d: dist[int] }\ntype Outer = { inner: list[Inner] }\nlet x: Outer = read(\"a.json\")");
    assert!(
        e.contains("data can't be a distribution") && e.contains("test.probl:1:"),
        "{e}"
    );
    // Two fields that would match the same column.
    let e = errors("type Row = { ab: int, a_b: int }\nlet rows: list[Row] = read(\"rows.csv\")");
    assert!(
        e.contains("the fields `ab` and `a_b` would match the same names in the data"),
        "{e}"
    );
}

#[test]
fn record_types_can_refer_to_each_other() {
    // In any order, and to themselves through a collection.
    ir("type A = { b: B }\ntype B = { n: int, more: list[A] }\nlet x: A = read(\"a.json\")\nreport x.b.n");
    ir("fn read(x) { x }\nreport read(1)");
    let e = errors("type Tree = { left: Tree, right: list[Tree] }");
    assert!(e.contains("every `Tree` would contain another `Tree`, forever"), "{e}");
    let e = errors("type A = { b: B }\ntype B = { a: A }");
    assert!(e.contains("every `A` would contain another `A`"), "{e}");
}

/// Whether `parts` appear in `text` in this order.
#[track_caller]
fn in_order(text: &str, parts: &[&str]) {
    let mut at = 0;
    for part in parts {
        match text[at..].find(part) {
            Some(i) => at += i + part.len(),
            None => panic!("{part:?} isn't where expected in:\n{text}"),
        }
    }
}

#[test]
fn draws_move_to_their_first_use() {
    // Past draws, assignments, observations, reports, loops and calls that
    // don't use the variable; with its type check.
    let text = ir("fn twice(n) { n * 2 }\n\
                   let a: int ~ d6\n\
                   let b ~ bernoulli(90%)\n\
                   let c ~ one_of([\"x\", \"y\"])\n\
                   observe b\n\
                   let d = twice(3)\n\
                   var n = 0\n\
                   repeat 2 { n += 1 }\n\
                   report c\n\
                   report a + d + n");
    in_order(
        &text,
        &[
            "b_1 ~",
            "observe b_1",
            "c_2 ~",
            "report c_2",
            "a_0 ~ 1d6",
            "check a_0",
            "report",
        ],
    );
}

#[test]
fn draws_stay_before_what_they_cant_pass() {
    // A use, even inside a branch or a lambda.
    in_order(
        &ir("let a ~ d6\nlet b = 1\nif b > 0 { report a as \"a\" }"),
        &["b_1 =", "a_0 ~", "if"],
    );
    in_order(
        &ir("let a ~ d6\nlet b = 1\nlet f = x -> x + a\nreport f(b)"),
        &["b_1 =", "a_0 ~", "f_2 ="],
    );
    // Printing, directly or in a function: output is per world.
    in_order(&ir("let a ~ d6\nprint(\"hi\")\nreport a"), &["a_0 ~", "print"]);
    in_order(
        &ir("fn f() { print(\"hi\")\n1 }\nlet a ~ d6\nlet y = f()\nreport a + y"),
        &["a_0 ~", "call"],
    );
    // Leaving the block early.
    in_order(
        &ir("var n = 0\nloop {\n  let a ~ d6\n  if n > 3 { break }\n  n += a\n}"),
        &["a_1 ~", "break"],
    );
    // Distributions that read variables, can fail, or aren't written out.
    in_order(
        &ir("let p = 50%\nlet a ~ bernoulli(p)\nlet b ~ d6\nreport b\nreport a"),
        &["a_1 ~", "b_2 ~"],
    );
    in_order(
        &ir("let a ~ bernoulli(1 / 3)\nlet b ~ d6\nreport b\nreport a"),
        &["a_0 ~", "b_1 ~"],
    );
    in_order(
        &ir("let a ~ 200d100\nlet b ~ d6\nreport b\nreport a"),
        &["a_0 ~", "b_1 ~"],
    );
}
