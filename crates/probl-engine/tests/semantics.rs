//! The engine against questions with known answers, and checks that merging
//! and memoization never change a result.

use probl_engine::value::Value;
use probl_engine::{Options, Outcome, run};
use probl_syntax::{SourceFile, render_all};

fn exec(src: &str, options: &Options) -> Result<Outcome, String> {
    let (program, diags) = probl_sema::compile(src);
    let file = SourceFile::new("test.probl", src);
    let Some(program) = program else {
        panic!("compile errors:\n{}", render_all(&diags, &file, false));
    };
    let mut print = |_: &str| {};
    run(&program, options, &mut print).map_err(|e| e.to_diagnostic().render(&file, false))
}

fn outcome(src: &str) -> Outcome {
    exec(src, &Options::default()).unwrap_or_else(|e| panic!("runtime error:\n{e}"))
}

/// The chance reported by the first `report`.
fn chance(src: &str) -> f64 {
    outcome(src).reports[0].chance().expect("the first report should be a probability")
}

fn mean(src: &str) -> f64 {
    let dist = outcome(src).reports[0].distribution();
    dist.iter().map(|(v, p)| v.as_f64().unwrap() * p).sum()
}

fn error(src: &str) -> String {
    exec(src, &Options::default()).expect_err("expected a runtime error")
}

#[track_caller]
fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
}

// ── Known answers ────────────────────────────────────────────────────────

#[test]
fn two_dice() {
    close(chance("report 2d6 == 7"), 1.0 / 6.0);
    close(chance("let a ~ d6\nlet b ~ d6\nreport a + b == 7"), 1.0 / 6.0);
    close(chance("report 2d6 > 2d6"), 575.0 / 1296.0);
}

#[test]
fn conditions_split_worlds() {
    close(chance("let x = if 30% { 1 } else { 2 }\nreport x == 1"), 0.3);
    close(chance("var x = 0\nif 30% { x = 1 }\nif 50% { x += 1 }\nreport x == 2"), 0.15);
}

#[test]
fn chances_are_independent_and_facts_are_not() {
    close(chance("let rain = 30%\nreport rain and rain"), 0.09);
    close(chance("let raining ~ 30%\nreport raining and raining"), 0.3);
    close(chance("report 30% or 50%"), 0.65);
    close(chance("report not 30%"), 0.7);
}

#[test]
fn monty_hall() {
    let src = "
        let car ~ one_of([1, 2, 3])
        let pick = 1
        let opened ~ one_of([1, 2, 3].filter(d -> d != car and d != pick))
        let other = [1, 2, 3].filter(d -> d != pick and d != opened)[0]
        report other == car as \"switching wins\"";
    close(chance(src), 2.0 / 3.0);
}

#[test]
fn birthday_problem() {
    let src = "
        var distinct = 0
        var shared = false
        repeat 23 {
            if not shared {
                if distinct / 365 { shared = true } else { distinct += 1 }
            }
        }
        report shared";
    let mut p_distinct = 1.0;
    for i in 0..23 {
        p_distinct *= (365 - i) as f64 / 365.0;
    }
    close(chance(src), 1.0 - p_distinct);
}

#[test]
fn gamblers_ruin() {
    let src = "
        var money = 3
        while money > 0 and money < 10 {
            money += if 40% { 1 } else { -1 }
        }
        report money == 10";
    let r: f64 = 0.6 / 0.4;
    close(chance(src), (1.0 - r.powi(3)) / (1.0 - r.powi(10)));
}

#[test]
fn loops_that_may_never_end_stop_at_epsilon() {
    let o = outcome("var n = 1\nwhile d6 != 6 { n += 1 }\nreport n");
    let dist = o.reports[0].distribution();
    let mean: f64 = dist.iter().map(|(v, p)| v.as_f64().unwrap() * p).sum();
    assert!((mean - 6.0).abs() < 1e-8);
    assert!(o.unresolved > 0.0 && o.unresolved < 1e-12);
}

#[test]
fn evidence() {
    close(chance("let a ~ d6\nlet b ~ d6\nobserve a + b == 7\nreport a == 1"), 1.0 / 6.0);
    let medical = "
        let sick ~ 1%
        let positive = if sick { 95% } else { 8% }
        observe positive
        report sick";
    close(chance(medical), 0.0095 / (0.0095 + 0.99 * 0.08));
    close(outcome(medical).evidence.unwrap(), 0.0887);
    let coin = "
        let coin ~ one_of([\"fair\", \"biased\"])
        let heads = if coin == \"fair\" { 50% } else { 90% }
        repeat 5 { observe true from heads }
        report coin == \"biased\"";
    let (b, f) = (0.9f64.powi(5), 0.5f64.powi(5));
    close(chance(coin), b / (b + f));
    close(chance("let k ~ d6\nobserve 3 from binomial(5, k / 6)\nreport k > 3"), {
        let pmf = |p: f64| 10.0 * p.powi(3) * (1.0 - p).powi(2);
        let ks: Vec<f64> = (1..=6).map(|k| pmf(k as f64 / 6.0)).collect();
        ks[3..].iter().sum::<f64>() / ks.iter().sum::<f64>()
    });
}

#[test]
fn cards_without_replacement() {
    let src = "
        var deck = bag([\"a\": 2, \"b\": 1])
        let first ~ deck.take()
        let second ~ deck.take()
        report first == \"a\" and second == \"a\"";
    close(chance(src), 1.0 / 3.0);
}

#[test]
fn dice_pools() {
    close(mean("report roll(4, d6).highest(3).sum()"), 15869.0 / 1296.0);
    close(mean("report roll(5, d10).count(x -> x >= 8)"), 1.5);
}

#[test]
fn counts() {
    close(mean("let k ~ binomial(10, 30%)\nreport k"), 3.0);
    close(mean("let k ~ poisson(4.5)\nreport k"), 4.5);
    close(mean("let k ~ geometric(25%)\nreport k"), 4.0);
}

#[test]
fn chance_blocks() {
    let src = "
        var pos = 0
        chance {
            50% => pos += 1
            30% => pos -= 1
        }
        report pos == 0";
    close(chance(src), 0.2);
    close(chance("let w = chance { 60% => \"sun\", 30% => \"rain\", else => \"snow\" }\nreport w == \"snow\""), 0.1);
}

#[test]
fn simulate_returns_distributions() {
    close(chance("let g = simulate { let a ~ d6\n a * 2 }\nreport g == 4"), 1.0 / 6.0);
    close(chance("report simulate { d6 > 4 }"), 1.0 / 3.0);
    // A settled draw from a simulated distribution, compared with a fresh one.
    close(chance("let g = simulate { d6 }\nlet mine ~ g\nreport g >= mine"), 7.0 / 12.0);
}

#[test]
fn functions_and_recursion() {
    close(chance("fn fact(n) { if n <= 1 { 1 } else { n * fact(n - 1) } }\nreport fact(10) == 3628800"), 1.0);
    let src = "
        fn heads(n) { if n == 0 { 0 } else { (if 50% { 1 } else { 0 }) + heads(n - 1) } }
        report heads(10) == 5";
    close(chance(src), 252.0 / 1024.0);
    close(chance("fn count(n) { if n == 0 { 0 } else { 1 + count(n - 1) } }\nreport count(900) == 900"), 1.0);
}

#[test]
fn match_statements() {
    let src = "
        enum Weather { Sun, Rain }
        let w ~ one_of([Sun, Rain, Rain])
        let mood = match w { Sun => 1, Rain => -1 }
        report mood == -1";
    close(chance(src), 2.0 / 3.0);
    let guards = "
        let n ~ d6
        let size = match n { 1 | 2 => \"small\", x if x > 4 => \"big\", _ => \"medium\" }
        report size == \"medium\"";
    close(chance(guards), 2.0 / 6.0);
}

#[test]
fn collections_and_records() {
    let src = "
        type P = { x: int, y: int }
        var p = P { x: 1, y: 2 }
        p.x += 10
        var xs = [1, 2, 3]
        xs[0] = 5
        xs.push(7)
        let top = xs.pop()
        let m = [\"a\": 1]
        let q = p with { y: 0 }
        report p.x == 11 and xs == [5, 2, 3] and top == 7 and m[\"a\"] == 1 and q.y == 0 and q.x == 11";
    close(chance(src), 1.0);
}

#[test]
fn dates() {
    close(chance("report date(\"2026-09-28\") + 7 == date(\"2026-10-05\")"), 1.0);
    close(chance("report weekday(date(\"2026-09-28\")) == \"Monday\""), 1.0);
}

// ── Errors ───────────────────────────────────────────────────────────────

#[test]
fn runtime_errors() {
    assert!(error("let x ~ d6\nreport 10 / (x - x)").contains("division by zero"));
    assert!(error("let xs = [1, 2]\nreport xs[2]").contains("out of range"));
    assert!(error("fn f(x) { if 50% { f(x) } else { x } }\nreport f(1)").contains("calls itself"));
    assert!(error("report if 3 { 1 } else { 2 }").contains("expected a probability"));
    assert!(error("for i in 1..d6 { }").contains("range"));
    assert!(error("let w = chance { 60% => 1, 30% => 2 }\nreport w").contains("no `else`"));
    assert!(error("chance { 60% => {}, 50% => {} }").contains("more than 100%"));
    assert!(error("report one_of([\"a\": 50%, \"b\": 40%])").contains("add up to 90%"));
}

#[test]
fn unimplemented_features_say_so() {
    let (program, _) = probl_sema::compile("report normal(0, 1) > 1.96");
    let mut print = |_: &str| {};
    let err = run(&program.unwrap(), &Options::default(), &mut print).unwrap_err();
    assert!(err.unsupported);
}

// ── Merging and memoization don't change results ─────────────────────────

const DIFFERENTIAL: &[&str] = &[
    "let a ~ d6\nlet b ~ d6\nvar s = a + b\nif s > 7 { s -= 7 }\nreport s",
    "var pos = 0\nrepeat 8 { pos += if 50% { 1 } else { -1 } }\nreport pos",
    "var x = 0\nvar y = 0\nrepeat 5 { chance { 30% => x += 1, 20% => y += 1, else => {} } }\nreport x - y",
    "fn hit(n) { if n > 3 { d6 } else { 0 } }\nvar total = 0\nrepeat 3 { let r ~ d6\n let h ~ hit(r)\n total += h }\nreport total",
    "var hp = 10\nvar rounds = 0\nwhile hp > 0 and rounds < 6 { rounds += 1\n let d ~ d4\n hp -= d }\nreport rounds",
    "let a ~ d6\nobserve a != 3\nlet b ~ one_of([a, 7])\nreport b",
    "var deck = bag([1: 2, 2: 2, 3: 1])\nlet x ~ deck.take()\nlet y ~ deck.take()\nreport x * 10 + y",
];

fn distributions(src: &str, options: &Options) -> Vec<Vec<(Value, f64)>> {
    let o = exec(src, options).unwrap();
    o.reports.iter().map(|s| s.distribution()).collect()
}

#[test]
fn merging_and_memoization_do_not_change_results() {
    let plain = Options {
        merge: false,
        memoize: false,
        ..Options::default()
    };
    for src in DIFFERENTIAL {
        let fast = distributions(src, &Options::default());
        let slow = distributions(src, &plain);
        assert_eq!(fast.len(), slow.len());
        for (f, s) in fast.iter().zip(&slow) {
            assert_eq!(f.len(), s.len(), "different supports for:\n{src}");
            for ((vf, pf), (vs, ps)) in f.iter().zip(s) {
                assert_eq!(vf, vs, "different values for:\n{src}");
                assert!((pf - ps).abs() < 1e-12, "{pf} vs {ps} for:\n{src}");
            }
        }
    }
}
