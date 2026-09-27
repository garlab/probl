//! Calls that come back to themselves, solved by iteration
//! (docs/semantics.md, section 6).

mod common;

use common::*;
use probl_engine::Options;
use probl_engine::value::Value;

#[track_caller]
fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-10 * expected.abs().max(1.0),
        "expected {expected}, got {actual}"
    );
}

fn chance_of(src: &str, value: Value) -> f64 {
    distribution(src)
        .into_iter()
        .find(|(v, _)| *v == value)
        .map_or(0.0, |(_, p)| p)
}

#[test]
fn a_call_that_may_call_itself_again_returns() {
    // The audit's example: it returns 1, with certainty.
    let out = outcome("fn f() -> int {\n  if 50% { return 1 }\n  return f()\n}\nreport f()");
    assert_eq!(out.reports[0].distribution(), [(Value::Int(1), 1.0)]);
    assert!(out.unresolved.to_f64() <= 1e-12);
    assert_eq!(out.stats.solved_calls, 1);
    assert!(
        out.output.starts_with("enumerated · unresolved < 1e-12"),
        "{}",
        out.output
    );
}

#[test]
fn exploding_dice() {
    // A 6 rolls again and adds: the mean is 3.5 / (5/6) = 4.2, a 7 is a 6
    // then a 1, and a 13 two 6s then a 1.
    let src = "fn explode() -> int {
  let r ~ d6
  if r == 6 { return 6 + explode() }
  return r
}
report explode()";
    near(mean(src), 4.2);
    near(chance_of(src, Value::Int(7)), 1.0 / 36.0);
    near(chance_of(src, Value::Int(13)), 1.0 / 216.0);
    assert_eq!(chance_of(src, Value::Int(6)), 0.0);
}

#[test]
fn calls_that_come_back_through_others() {
    // even() is true with 50%, or else what odd() says, and odd() is false
    // with 50%, or else what even() says: E = 1/2 + E/4, so 2/3.
    let out = outcome(
        "fn even() -> bool {
  if 50% { return true }
  return odd()
}
fn odd() -> bool {
  if 50% { return false }
  return even()
}
report even()",
    );
    near(out.reports[0].chance().unwrap(), 2.0 / 3.0);
}

#[test]
fn a_recursive_tennis_game_matches_the_loop() {
    let src = "fn game(s: int, r: int) -> bool {
  if s >= 4 and s - r >= 2 { return true }
  if r >= 4 and r - s >= 2 { return false }
  if s == r and s > 3 { return game(3, 3) }
  if 64% { return game(s + 1, r) }
  return game(s, r + 1)
}
report game(0, 0)";
    // p⁴(1 + 4q + 10q²) + 20p³q³ · p² / (1 − 2pq), from exact fractions.
    near(chance(src), 0.812_614_662_684_866_5);
    // The same without reusing results.
    let options = Options {
        memoize: false,
        ..Options::default()
    };
    let unmemoized = exec(src, &options).unwrap();
    near(unmemoized.reports[0].chance().unwrap(), 0.812_614_662_684_866_5);
}

#[test]
fn observations_inside_count_in_every_round() {
    // Each round halves the weight, and returns with 50%: Σ (1/4)ⁿ = 1/3.
    let out = outcome(
        "fn f() -> int {
  observe 50%
  if 50% { return 1 }
  return f()
}
report f()",
    );
    near(out.evidence.unwrap().to_f64(), 1.0 / 3.0);
}

#[test]
fn loops_inside_and_around_recursion() {
    let out = outcome(
        "fn f(n: int) -> int {
  var x = 0
  while x < 3 {
    x = if 50% { 0 } else { x + 1 }
  }
  if 50% { return n }
  return f(n)
}
var total = 0
var done = false
while not done {
  total = f(2)
  done ~ bernoulli(50%)
}
report total",
    );
    assert_eq!(out.reports[0].distribution(), [(Value::Int(2), 1.0)]);
    assert!(out.stats.solved_calls >= 1);
    assert!(out.stats.solved_loops >= 1);
}

#[test]
fn a_call_that_never_returns_is_an_error() {
    let e = error("fn f() -> int { return f() }\nreport f()");
    assert!(e.contains("`f()` never returns for some of its worlds"), "{e}");
    assert!(e.contains("100% of its weight keeps coming back to it"), "{e}");
    // Only part of the weight: `h()` never returns.
    let e = error(
        "fn g() -> int {
  if 50% { return 1 }
  return h()
}
fn h() -> int { return h() }
report g()",
    );
    assert!(e.contains("`h()` never returns"), "{e}");
}

#[test]
fn a_call_that_comes_back_and_prints_is_an_error() {
    let e = error("fn f() -> int {\n  print(1)\n  if 50% { return 1 }\n  return f()\n}\nreport f()");
    assert!(e.contains("`f()` comes back to itself, and prints"), "{e}");
}

#[test]
fn simulate_coming_back_to_a_running_call_is_not_supported() {
    let e = error(
        "fn f() -> int {
  if 50% { return 1 }
  let d = simulate { f() }
  return 2
}
report f()",
    );
    assert!(
        e.contains("a `simulate` block that comes back to a call still running"),
        "{e}"
    );
}

#[test]
fn sampling_follows_each_run() {
    let src = "@mode sample(runs: 20_000, seed: 3)
fn explode() -> int {
  let r ~ d6
  if r == 6 { return 6 + explode() }
  return r
}
report explode()";
    let out = outcome(src);
    let acc = out.reports[0].groups.values().next().unwrap();
    let (m, se) = acc.mean_se();
    assert!((m - 4.2).abs() < 5.0 * se, "{m} ± {se}");
    assert_eq!(out.stats.solved_calls, 0);
}

/// The same random process twice: as a function that calls itself with the
/// next state, and as a loop over the state. Updates wrap around small
/// counters, and every step may stop first. Half the time, each step also
/// adds 1 to the result: after the call returns in the function, and with a
/// counter in the loop, so there are infinitely many possible results.
fn recursion_and_loop(rng: &mut probl_engine::continuous::Rng) -> (String, String) {
    let mut pick = |n: usize| (rng.uniform() * n as f64) as usize;
    let stop = format!("{}%", 5 + pick(40));
    let mut steps = Vec::new();
    for _ in 0..1 + pick(5) {
        let (v, k, m, p) = (["x", "y"][pick(2)], 1 + pick(3), 2 + pick(3), 10 + pick(80));
        steps.push(match pick(5) {
            0 => format!("{v} = ({v} + {k}) mod {m}"),
            1 => format!("{v} = if {p}% {{ ({v} + {k}) mod {m} }} else {{ {v} }}"),
            2 => format!("if x == y {{ observe {p}% }}"),
            3 => format!("{v} = chance {{ {p}% => 0, else => {} }}", pick(3)),
            _ => format!("if {v} == {} and bernoulli({p}%) {{ {v} = {} }}", pick(3), pick(3)),
        });
    }
    let (x0, y0) = (pick(3), pick(3));
    let counting = pick(2) == 1;
    let indent = |pad: &str| steps.iter().map(|s| format!("{pad}{s}")).collect::<Vec<_>>().join("\n");
    let (plus, count, counted) = if counting {
        (" + 1", "\n  steps += 1", " + steps")
    } else {
        ("", "", "")
    };
    let recursion = format!(
        "fn f(a: int, b: int) -> int {{\n  if {stop} {{ return a * 3 + b }}\n  var x = a\n  var y = b\n{}\n  return f(x, y){plus}\n}}\nreport f({x0}, {y0})",
        indent("  ")
    );
    let looping = format!(
        "var x = {x0}\nvar y = {y0}\nvar steps = 0\nvar result = 0\nloop {{\n  if {stop} {{\n    result = x * 3 + y{counted}\n    break\n  }}\n{}{count}\n}}\nreport result",
        indent("  ")
    );
    (recursion, looping)
}

/// Iterating a recursion agrees with solving the same process as a loop.
/// `PROBL_RECURSION_CASES` runs more of them.
#[test]
fn random_recursions_agree_with_loops() {
    let cases: u64 = std::env::var("PROBL_RECURSION_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300);
    let mut rng = probl_engine::continuous::Rng::new(21);
    let mut compared = 0;
    for _ in 0..cases {
        let (recursion, looping) = recursion_and_loop(&mut rng);
        let (r, l) = (
            exec_raw(&recursion, &Options::default()),
            exec_raw(&looping, &Options::default()),
        );
        match (r, l) {
            (Ok(r), Ok(l)) => {
                assert!(r.stats.solved_calls >= 1, "{recursion}");
                // Counting, the loop never comes back to a state: it's unrolled.
                let slack = r.unresolved.to_f64() + l.unresolved.to_f64() + 1e-12;
                let (zr, zl) = (
                    r.evidence.map_or(1.0, |z| z.to_f64()),
                    l.evidence.map_or(1.0, |z| z.to_f64()),
                );
                assert!((zr - zl).abs() <= slack, "evidence {zr} against {zl}:\n{recursion}");
                let (dr, dl) = (r.reports[0].distribution(), l.reports[0].distribution());
                for (v, p) in &dl {
                    let q = dr.iter().find(|(w, _)| w == v).map_or(0.0, |(_, q)| *q);
                    assert!(
                        (p - q).abs() <= slack / zl.min(zr) + 1e-9,
                        "{v:?}: {q} against {p}:\n{recursion}\n{looping}"
                    );
                    compared += 1;
                }
            }
            (Err(r), Err(l)) => {
                // Observations that rule out every world.
                let impossible = |e: &probl_engine::RuntimeError| e.message.contains("impossible");
                assert!(
                    impossible(&r) && impossible(&l),
                    "{}\nagainst\n{}\n{recursion}",
                    r.message,
                    l.message
                );
            }
            (r, l) => panic!("one failed:\n{:?}\n{:?}\n{recursion}\n{looping}", r.err(), l.err()),
        }
    }
    eprintln!("{cases} processes, {compared} probabilities compared");
    assert!(compared as u64 >= cases);
}
