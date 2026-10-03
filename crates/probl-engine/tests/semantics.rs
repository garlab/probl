//! The engine against questions with known answers, and checks that merging
//! and memoization never change a result.

mod common;

use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Options};

// ── Known answers ────────────────────────────────────────────────────────

#[test]
fn two_dice() {
    close(chance("report 2d6 == 7"), 1.0 / 6.0);
    close(chance("let a ~ d6\nlet b ~ d6\nreport a + b == 7"), 1.0 / 6.0);
    close(chance("report 2d6 > 2d6"), 575.0 / 1296.0);
}

#[test]
fn drawn_events_control_branches() {
    close(
        chance("let x = if (chance { 30% => true, else => false }) { 1 } else { 2 }\nreport x == 1"),
        0.3,
    );
    close(
        chance(
            "var x = 0\nif (chance { 30% => true, else => false }) { x = 1 }\nif (chance { 50% => true, else => false }) { x += 1 }\nreport x == 2",
        ),
        0.15,
    );
    close(
        chance("let big ~ d6 > 4\nlet x = if big { 1 } else { 0 }\nreport x == 1"),
        1.0 / 3.0,
    );
}

#[test]
fn events_need_identities() {
    close(chance("let a ~ bernoulli(30%)\nreport a and a"), 0.3);
    close(
        chance("let a ~ bernoulli(30%)\nlet b ~ bernoulli(50%)\nreport a or b"),
        0.65,
    );
    close(chance("let a ~ bernoulli(30%)\nreport not a"), 0.7);
    close(chance("report not (d6 > 4)"), 2.0 / 3.0);
    // One settled fact and one uncertain one combine safely.
    close(chance("let r ~ d6\nreport r > 4 and d6 > 4"), 1.0 / 9.0);
    // A drawn distribution of facts gives an event with an identity.
    close(chance("let big ~ d6 > 4\nreport big and big"), 1.0 / 3.0);
    close(mean("report prob(30%) and prob(30%)"), 0.09);
    close(chance("report (d6 > 4) and (d6 > 4)"), 1.0 / 9.0);
    close(chance("let e = d6 > 4\nreport e or e"), 5.0 / 9.0);
    close(chance("let p ~ prob(30%)\nreport p"), 0.3);
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
                chance { prob(distinct / 365) => { shared = true }, else => { distinct += 1 } }
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
            money += if (chance { 40% => true, else => false }) { 1 } else { -1 }
        }
        report money == 10";
    let r: f64 = 0.6 / 0.4;
    close(chance(src), (1.0 - r.powi(3)) / (1.0 - r.powi(10)));
}

#[test]
fn loops_that_may_never_end_stop_at_epsilon() {
    let o = outcome("var n = 1\nwhile ({ let face ~ d6; face != 6 }) { n += 1 }\nreport n");
    let dist = o.reports[0].distribution();
    let mean: f64 = dist.iter().map(|(v, p)| v.as_f64().unwrap() * p).sum();
    assert!((mean - 6.0).abs() < 1e-8);
    let u = o.unresolved.to_f64();
    assert!(u > 0.0 && u < 1e-12);
}

#[test]
fn evidence() {
    close(
        chance("let a ~ d6\nlet b ~ d6\nobserve a + b == 7\nreport a == 1"),
        1.0 / 6.0,
    );
    let medical = "
        let sick ~ bernoulli(1%)
        let positive = if sick { 95% } else { 8% }
        observe true from bernoulli(prob(positive))
        report sick";
    close(chance(medical), 0.0095 / (0.0095 + 0.99 * 0.08));
    close(outcome(medical).evidence.unwrap().to_f64(), 0.0887);
    for observation in [
        "let seen ~ bernoulli(prob(heads)); observe seen",
        "observe true from bernoulli(prob(heads))",
    ] {
        let coin = format!(
            "let coin ~ one_of([\"fair\", \"biased\"])
             let heads = if coin == \"fair\" {{ 50% }} else {{ 90% }}
             repeat 5 {{ {observation} }}
             report coin == \"biased\""
        );
        let (b, f) = (0.9f64.powi(5), 0.5f64.powi(5));
        close(chance(&coin), b / (b + f));
    }
    close(
        chance("let k ~ d6\nobserve 3 from binomial(5, prob(k / 6))\nreport k > 3"),
        {
            let pmf = |p: f64| 10.0 * p.powi(3) * (1.0 - p).powi(2);
            let ks: Vec<f64> = (1..=6).map(|k| pmf(k as f64 / 6.0)).collect();
            ks[3..].iter().sum::<f64>() / ks.iter().sum::<f64>()
        },
    );
}

#[test]
fn observing_a_recipe_requires_an_explicit_likelihood() {
    let prior = distribution("report 3d8");
    for mode in [
        probl_sema::ir::Mode::Enumerate,
        probl_sema::ir::Mode::Sample { runs: 100, seed: 7 },
    ] {
        let out = exec(
            "let x = 3d8\nobserve true from (x > 10)\nreport x",
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        close(out.evidence.unwrap().to_f64(), 392.0 / 512.0);
        let actual = out.reports[0].distribution();
        assert_eq!(actual.len(), prior.len());
        for ((value, probability), (expected, mass)) in actual.iter().zip(&prior) {
            assert_eq!(value, expected);
            close(*probability, *mass);
        }
    }
}

#[test]
fn observing_a_draw_conditions_its_reported_distribution() {
    let src = "let x ~ 3d8\nobserve x > 10\nreport x\nreport x > 10";
    let out = outcome(src);
    // Independent reference: enumerate the 512 ordered triples of die faces.
    let mut counts = [0_u32; 25];
    for a in 1..=8 {
        for b in 1..=8 {
            for c in 1..=8 {
                if a + b + c > 10 {
                    counts[a + b + c] += 1;
                }
            }
        }
    }
    let survivors: u32 = counts.iter().sum();
    assert_eq!(survivors, 392);
    close(out.evidence.unwrap().to_f64(), f64::from(survivors) / 512.0);
    let actual = out.reports[0].distribution();
    assert_eq!(actual.len(), 14);
    for ((value, probability), n) in actual.iter().zip(11..=24) {
        close(value.as_f64().unwrap(), n as f64);
        close(*probability, f64::from(counts[n]) / f64::from(survivors));
    }
    close(out.reports[1].chance().unwrap(), 1.0);

    let sampled = outcome(&format!("@mode sample(runs: 1000, seed: 7)\n{src}"));
    for (value, _) in sampled.reports[0].distribution() {
        assert!((11.0..=24.0).contains(&value.as_f64().unwrap()));
    }
    close(sampled.reports[1].chance().unwrap(), 1.0);
}

#[test]
fn cards_without_replacement() {
    let src = "
        var deck = bag([\"a\": 2, \"b\": 1])
        let first = deck.take()
        let second = deck.take()
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
    close(
        chance("let w = chance { 60% => \"sun\", 30% => \"rain\", else => \"snow\" }\nreport w == \"snow\""),
        0.1,
    );
}

#[test]
fn simulate_returns_distributions() {
    close(
        chance("let g = simulate { let a ~ d6\n a * 2 }\nreport g == 4"),
        1.0 / 6.0,
    );
    close(chance("report simulate { d6 > 4 }"), 1.0 / 3.0);
    // A settled draw from a simulated distribution, compared with a fresh one.
    close(
        chance("let g = simulate { d6 }\nlet mine ~ g\nreport g >= mine"),
        7.0 / 12.0,
    );
    // A distribution over probabilities stays one.
    let rates = distribution("report simulate { chance { 50% => prob(10%), else => prob(90%) } }");
    assert_eq!(rates, vec![(Value::Prob(0.1), 0.5), (Value::Prob(0.9), 0.5)]);
}

#[test]
fn functions_and_recursion() {
    close(
        chance("fn fact(n) { if n <= 1 { 1 } else { n * fact(n - 1) } }\nreport fact(10) == 3628800"),
        1.0,
    );
    let src = "
        fn heads(n) { if n == 0 { 0 } else { (if (chance { 50% => true, else => false }) { 1 } else { 0 }) + heads(n - 1) } }
        report heads(10) == 5";
    close(chance(src), 252.0 / 1024.0);
    close(
        chance("fn depth(n) { if n == 0 { 0 } else { 1 + depth(n - 1) } }\nreport depth(400) == 400"),
        1.0,
    );
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

#[test]
fn math_functions() {
    for (f, expected) in [
        ("sin", 0.5f64.sin()),
        ("cos", 0.5f64.cos()),
        ("tan", 0.5f64.tan()),
        ("asin", 0.5f64.asin()),
        ("acos", 0.5f64.acos()),
        ("atan", 0.5f64.atan()),
        ("sinh", 0.5f64.sinh()),
        ("cosh", 0.5f64.cosh()),
        ("tanh", 0.5f64.tanh()),
        ("log2", -1.0),
        ("log1p", 1.5f64.ln()),
        ("expm1", 0.5f64.exp() - 1.0),
    ] {
        close(mean(&format!("report {f}(0.5)")), expected);
    }
    close(mean("report atan2(1, -1)"), 3.0 * std::f64::consts::FRAC_PI_4);
    close(mean("report atan2(-1, 0)"), -std::f64::consts::FRAC_PI_2);
    close(mean("report atan2(-1, -1)"), -3.0 * std::f64::consts::FRAC_PI_4);
    close(mean("report atan2(0, 0)"), 0.0);
    close(mean("report asin(-1)"), -std::f64::consts::FRAC_PI_2);
    close(mean("report acos(-1)"), std::f64::consts::PI);
    close(mean("report asin(1)"), std::f64::consts::FRAC_PI_2);
    close(mean("report acos(1)"), 0.0);
    close(mean("report hypot(3, 4)"), 5.0);
    close(mean("report hypot(3e200, 4e200) / 1e200"), 5.0);
    close(mean("report hypot(3e-200, 4e-200) / 1e-200"), 5.0);
    close(mean("report hypot(1e308, 1e308) / 1e308"), std::f64::consts::SQRT_2);
    // Accurate where `ln(1 + x)` and `exp(x) - 1` lose every digit.
    assert!((mean("report log1p(1e-20)") / 1e-20 - 1.0).abs() < 1e-12);
    assert!((mean("report expm1(1e-20)") / 1e-20 - 1.0).abs() < 1e-12);
    close(mean("let x ~ d6\nreport sin(x)^2 + cos(x)^2"), 1.0);
    let cos_d4 = (1..=4).map(|n| f64::from(n).cos()).sum::<f64>() / 4.0;
    close(mean("report cos(d4)"), cos_d4);
    for src in [
        "report cosh(1000)",
        "report sinh(-1000)",
        "report expm1(1000)",
        "report asin(1.5)",
        "report acos(-2)",
        "report log1p(-1)",
        "report log2(0)",
        "report log2(-1)",
        "report asin(one_of([0, 2]))",
    ] {
        assert!(error(src).contains("isn't defined for"), "{src}");
    }
    assert!(error("report hypot(1.5e308, 1.5e308)").contains("isn't a finite number"));
    assert!(error("report atan2(0 ^ -1, 1)").contains("division by zero"));
    assert!(error("report sin(0 ^ -1)").contains("division by zero"));
    assert!(error("report sin(true)").contains("needs a number"));
    assert!(error("report atan2(1, \"x\")").contains("needs a number"));
    assert!(error("report sin(normal(0, 1))").contains("draw a value first"));
    assert!(compile_error("report sin()").contains("takes 1 argument"));
    assert!(compile_error("report atan2(1)").contains("takes 2 arguments"));
}

#[test]
fn math_functions_preserve_distribution_semantics_in_both_modes() {
    for mode in ["enumerate", "sample(runs: 1000, seed: 1)"] {
        // Both arguments lift independently, with their original weights.
        close(
            mean(&format!(
                "@mode {mode}\nreport hypot(one_of([0: 25%, 3: 75%]), one_of([0, 4]))"
            )),
            3.5,
        );
        // A drawn angle has one identity in every use, also for continuous draws.
        close(
            mean(&format!("@mode {mode}\nlet x ~ d6\nreport sin(x)^2 + cos(x)^2")),
            1.0,
        );
    }
    close(
        mean("@mode sample(runs: 1000, seed: 1)\nlet x ~ uniform(-pi, pi)\nreport sin(x)^2 + cos(x)^2"),
        1.0,
    );
}

#[test]
fn atan2_respects_equality_when_merging_and_memoizing() {
    close(mean("report atan2(-0.0, -1)"), std::f64::consts::PI);
    close(mean("report atan2(0, -0.0)"), 0.0);
    let src = "
        fn angle(y) { atan2(y, -1) }
        var y = 0.0
        if (chance { 50% => true, else => false }) { y = -0.0 }
        report angle(y) < 0
    ";
    for merge in [false, true] {
        for memoize in [false, true] {
            let out = exec(
                src,
                &Options {
                    merge,
                    memoize,
                    ..Options::default()
                },
            )
            .unwrap();
            close(out.reports[0].chance().unwrap(), 0.0);
        }
    }
    close(chance(&format!("@mode sample(runs: 1000, seed: 1)\n{src}")), 0.0);
}

#[test]
fn constants() {
    close(mean("report pi"), std::f64::consts::PI);
    close(mean("report e"), std::f64::consts::E);
    close(mean("report euler_gamma"), 0.577_215_664_901_532_9);
    close(mean("report sin(pi / 2) + ln(e)"), 2.0);
    // The harmonic numbers approach ln(n) + γ.
    close(
        mean("var h = 0.0\nfor k in 1..10000 { h += 1 / k }\nreport h - ln(10000) - euler_gamma"),
        1.0 / 20_000.0 - 1.0 / (12.0 * 1e8),
    );
    // A program's own names hide them.
    close(mean("let e = 5\nreport e"), 5.0);
    close(mean("fn f(pi) { pi * 2 }\nreport f(1)"), 2.0);
    close(mean("let e = 5\nfn f() { e + 1 }\nreport f()"), 6.0);
    close(mean("fn f() { e + 1 }\nlet e = 5\nreport f()"), 6.0);
    close(mean("fn f() { pi }\nreport f()"), std::f64::consts::PI);
    close(mean("let f = x -> x + e\nreport f(0)"), std::f64::consts::E);
    close(mean("report simulate { euler_gamma }"), 0.577_215_664_901_532_9);
    close(mean("var pi = 2\npi += 1\nreport pi"), 3.0);
    close(mean("fn pi() { 3 }\nreport pi()"), 3.0);
    close(mean("var total = 0\nfor e in [1, 2] { total += e }\nreport total"), 3.0);
    close(mean("let xs = [1, 2].map(e -> e * 10)\nreport xs[1]"), 20.0);
    close(chance("enum Letter { e, f }\nreport e == Letter.e"), 1.0);
    assert!(compile_error("fn pi() { 3 }\nreport pi").contains("the function `pi` can't be used as a value"));
    assert!(compile_error("pi = 3").contains("can't assign to `pi`: it's a constant"));
    assert!(compile_error("pi()").contains("`pi` is a constant, not a function"));
    assert!(compile_error("report pie").contains("did you mean `pi`?"));
}

#[test]
fn declared_types_are_checked() {
    close(chance("let n: int = 3\nreport n == 3"), 1.0);
    close(
        chance("fn half(x: float) -> float { x / 2 }\nreport half(3) == 1.5"),
        1.0,
    );
    assert!(error("fn f(x: int) { x }\nreport f(\"a\") == 1").contains("`x` should be an int, but it's a str"));
    assert!(error("fn f(x) -> int { x / 2 }\nreport f(3) == 1").contains("the result should be an int"));
    assert!(error("var n: int = 1\nn = n / 2").contains("`n` should be an int, but it's a float"));
    assert!(error("let d: dist[int] = one_of([\"a\"])").contains("should be a dist[int]"));
    assert!(compile_error("let p: prob = \"x\"").contains("expected a probability, found a string"));
    assert!(compile_error("let p: probability = 1").contains("unknown type `probability`"));
}

// ── Errors ───────────────────────────────────────────────────────────────

#[test]
fn runtime_errors() {
    assert!(error("let x ~ d6\nreport 10 / (x - x)").contains("division by zero"));
    assert!(error("let xs = [1, 2]\nreport xs[2]").contains("out of range"));
    assert!(error("fn f(x) { f(x) }\nreport f(1)").contains("`f(1)` never returns for some of its worlds"));
    assert!(compile_error("report if 3 { 1 } else { 2 }").contains("between 0 and 1"));
    assert!(error("for i in 1..d6 { }").contains("range"));
    assert!(error("let w = chance { 60% => 1, 30% => 2 }\nreport w").contains("no `else`"));
    assert!(error("chance { 60% => {}, 50% => {} }").contains("more than 100%"));
    assert!(error("report one_of([\"a\": prob(50%), \"b\": prob(40%)])").contains("add up to 90%"));
}

#[test]
fn unimplemented_features_say_so() {
    for src in [
        "@mode particles(runs: 10, seed: 1)\nreport 1",
        "@mode beam(worlds: 10)\nreport 1",
        "report bins(normal(0, 1), 10)",
    ] {
        let err = exec_raw(src, &Options::default()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported, "{src}");
    }
}

// ── Merging and memoization don't change results ─────────────────────────

const DIFFERENTIAL: &[&str] = &[
    "let a ~ d6\nlet b ~ d6\nvar s = a + b\nif s > 7 { s -= 7 }\nreport s",
    "var pos = 0\nrepeat 8 { pos += if (chance { 50% => true, else => false }) { 1 } else { -1 } }\nreport pos",
    "var x = 0\nvar y = 0\nrepeat 5 { chance { 30% => x += 1, 20% => y += 1, else => {} } }\nreport x - y",
    "fn hit(n) { if n > 3 { d6 } else { 0 } }\nvar total = 0\nrepeat 3 { let r ~ d6\n let h ~ hit(r)\n total += h }\nreport total",
    "var hp = 10\nvar rounds = 0\nwhile hp > 0 and rounds < 6 { rounds += 1\n let d ~ d4\n hp -= d }\nreport rounds",
    "let a ~ d6\nobserve a != 3\nlet b ~ one_of([a, 7])\nreport b",
    "var deck = bag([1: 2, 2: 2, 3: 1])\nlet x = deck.take()\nlet y = deck.take()\nreport x * 10 + y",
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
