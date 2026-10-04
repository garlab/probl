use probl_syntax::{SourceFile, parse_program, render_all, sexpr};
use std::path::Path;

fn parse_ok(src: &str) -> String {
    let (program, diags) = parse_program(src);
    let file = SourceFile::new("test.probl", src);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics:\n{}",
        render_all(&diags, &file, false)
    );
    sexpr::program(&program)
}

fn parse_errors(src: &str) -> String {
    let (_, diags) = parse_program(src);
    assert!(!diags.is_empty(), "expected diagnostics for {src:?}");
    render_all(&diags, &SourceFile::new("test.probl", src), false)
}

#[test]
fn all_examples_parse() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "probl"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 9);
    for path in paths {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let tree = parse_ok(&src);
        insta::assert_snapshot!(name, tree);
    }
}

#[test]
fn prefix_draw_precedence_and_score() {
    assert_eq!(
        parse_ok("~d6 != 6\n~d6 + ~d6\n~(d6 > 3)\n~f()[0]\n~d6 ^ 2\nscore 30%"),
        "(!= (~ 1d6) 6)\n(+ (~ 1d6) (~ 1d6))\n(~ (> 1d6 3))\n(~ (index (call f) 0))\n(^ (~ 1d6) 2)\n(score 30%)\n"
    );
}

#[test]
fn typeof_precedence_and_parentheses() {
    assert_eq!(
        parse_ok(
            "typeof x == \"prob\"\ntypeof (d6 > 3)\ntypeof x.f()[0]\ntypeof\n  33%\ntypeof typeof x\ntypeof x ^ 2"
        ),
        "(== (typeof x) \"prob\")\n(typeof (> 1d6 3))\n(typeof (index (.f x) 0))\n(typeof 33%)\n(typeof (typeof x))\n(^ (typeof x) 2)\n"
    );
}

#[test]
fn precedence() {
    insta::assert_snapshot!(parse_ok(
        "a or b and not c == d\n\
         -x ^ 2 * 3 + 4\n\
         2 ^ 3 ^ 2\n\
         x not in [1, 2] and y in 1..6\n\
         d20 + 5 >= 15\n\
         3% to 7% \n\
         a..<n - 1\n\
         f(x).g(y)[0].h"
    ), @r"
    (or a (and b (not (== c d))))
    (+ (* (- (^ x 2)) 3) 4)
    (^ 2 (^ 3 2))
    (and (not in x [1 2]) (in y (.. 1 6)))
    (>= (+ 1d20 5) 15)
    (to 3% 7%)
    (..< a (- n 1))
    (. (index (.g (call f x) y) 0) h)
    ");
}

#[test]
fn records_blocks_and_headers() {
    insta::assert_snapshot!(parse_ok(
        "let hero = Fighter { hp: 12, dice: 1d8 }\n\
         let r = { won: h > 0, rounds }\n\
         let b = { x }\n\
         if hero == x { a } else { b }\n\
         for hp in [7, 12] { report hp by hp }\n\
         let tough = hero with { hp: 20 }"
    ), @r"
    (let hero = (record Fighter (hp 12) (dice 1d8)))
    (let r = (record (won (> h 0)) (rounds rounds)))
    (let b = (block x))
    (if (== hero x) (block a) (block b))
    (for hp [7 12] (block (report hp by hp)))
    (let tough = (with hero (hp 20)))
    ");
}

#[test]
fn chance_match_and_lambdas() {
    insta::assert_snapshot!(parse_ok(
        "chance {\n  50% => pos += 1\n  30% => pos -= 1\n  else => {}\n}\n\
         let w = chance { 60% => \"sun\", else => \"rain\" }\n\
         match weather {\n  \"sun\" => mood += 1\n  \"rain\" | \"snow\" if cold => mood -= 1\n  _ => {}\n}\n\
         let n = roll(5, d10).count(x -> x >= 8)\n\
         let add = (a, b) -> a + b"
    ), @r#"
    (chance (50% => (+= pos 1)) (30% => (-= pos 1)) (else => (block)))
    (let w = (chance (60% => "sun") (else => "rain")))
    (match weather ("sun" => (+= mood 1)) ((| "rain" "snow") if cold => (-= mood 1)) (_ => (block)))
    (let n = (.count (call roll 5 1d10) (-> (x) (>= x 8))))
    (let add = (-> (a b) (+ a b)))
    "#);
}

#[test]
fn statements() {
    insta::assert_snapshot!(parse_ok(
        "@mode sample(runs: 1_000, seed: 7)\n\
         var x ~ 2d6\n\
         x ~ one_of([Boom: 20%, Slump: 80%])\n\
         observe 3 from binomial(10, rate)\n\
         report \"x is {x + 1}\" as \"label\"\n\
         while d6 != 6 { rolls += 1 }\n\
         loop { if done { break }; continue }\n\
         repeat 3 { return }\n\
         if a {\n  b\n}\nelse {\n  c\n}"
    ), @r#"
    (@mode (call sample runs: 1000 seed: 7))
    (var x ~ 2d6)
    (~ x (call one_of [Boom: 20%, Slump: 80%]))
    (observe 3 from (call binomial 10 rate))
    (report (str "x is " (+ x 1)) as "label")
    (while (!= 1d6 6) (block (+= rolls 1)))
    (loop (block (if done (block (break))) (continue)))
    (repeat 3 (block (return)))
    (if a (block b) (block c))
    "#);
}

#[test]
fn syntax_errors() {
    insta::assert_snapshot!(parse_errors("let x = 1 +\nlet y = 2"));
    insta::assert_snapshot!(parse_errors("if a < b < c { }"));
    insta::assert_snapshot!(parse_errors("x = (1, 2"));
    insta::assert_snapshot!(parse_errors("fn f() {\n  let a = 1\n  a +* 2\n}\nreport 30 %"));
    insta::assert_snapshot!(parse_errors("3 = x\nlet y 5"));
}

#[test]
fn a_failed_arm_is_skipped_whole() {
    let (program, diags) = parse_program("match x {\n  [1, 2 3, 4, 5] => 1\n  [a, b] => a + b\n}");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(sexpr::program(&program), "(match x ([a b] => (+ a b)))\n");
}
