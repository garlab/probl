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
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        insta::assert_snapshot!(name, ir(&src));
    }
}

#[test]
fn if_expressions_and_calls_are_hoisted() {
    insta::assert_snapshot!(ir("fn f(x) { if 50% { x } else { x + 1 } }\n\
         var pos = 0\n\
         pos += if 30% { 1 } else { -1 }\n\
         let y = f(pos) * 2\n\
         report y"));
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
