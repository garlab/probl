//! Loops solved as absorbing Markov chains (docs/semantics.md, section 10):
//! exact answers where unrolling approximates, loops that never end, and
//! agreement with unrolling where both work.

mod common;

use common::*;
use probl_engine::continuous::Rng;
use probl_engine::value::Value;
use probl_engine::{Limits, Options, Outcome};

fn unrolled() -> Options {
    Options {
        solve: false,
        epsilon: Some(1e-15),
        ..Options::default()
    }
}

fn run_with(src: &str, options: &Options) -> Outcome {
    exec(src, options).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

#[track_caller]
fn exactly(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-13 * expected.abs().max(1.0),
        "expected {expected}, got {actual}"
    );
}

#[test]
fn a_fair_gamblers_ruin_is_exact() {
    let src = "var money = 3
while money > 0 and money < 10 {
  money += if 50% { 1 } else { -1 }
}
report money == 10";
    let out = outcome(src);
    exactly(out.reports[0].chance().unwrap(), 0.3);
    assert!(out.unresolved.is_zero());
    assert_eq!(out.stats.solved_loops, 1);
    assert_eq!(out.output, "enumerated\n\nmoney == 10    30.00%\n");
    // Unrolled, it's only close.
    let plain = run_with(
        src,
        &Options {
            solve: false,
            ..Options::default()
        },
    );
    assert!(!plain.unresolved.is_zero());
    assert_eq!(plain.stats.solved_loops, 0);
}

#[test]
fn a_biased_gamblers_ruin_is_exact() {
    let out = outcome(
        "var money = 3
while money > 0 and money < 10 {
  money += if 60% { 1 } else { -1 }
}
report money == 10",
    );
    // (1 − r³) / (1 − r¹⁰), with r = 0.4 / 0.6.
    exactly(out.reports[0].chance().unwrap(), 0.716_122_361_051_271);
}

#[test]
fn a_tennis_game_with_deuce_is_exact() {
    let out = outcome(
        "fn game(p) {
  var s = 0
  var r = 0
  while max(s, r) < 4 or abs(s - r) < 2 {
    if p { s += 1 } else { r += 1 }
    if s == r and s > 3 { s = 3; r = 3 }
  }
  s > r
}
report game(64%)",
    );
    // p⁴(1 + 4q + 10q²) + 20p³q³ · p² / (1 − 2pq), from exact fractions.
    exactly(out.reports[0].chance().unwrap(), 0.812_614_662_684_866_5);
    assert!(out.unresolved.is_zero());
}

#[test]
fn loops_that_some_worlds_never_leave_are_errors() {
    let error = error(
        "var x = 0
while x < 5 {
  if x == 2 { x = 2 } else { x += if 50% { 1 } else { 2 } }
}
report x",
    );
    assert!(error.contains("some worlds can never leave this loop"), "{error}");
    assert!(error.contains("for example, the worlds where `x` is 2"), "{error}");
    // Unrolled, it only runs out of iterations.
    let options = Options {
        solve: false,
        limits: Limits {
            max_iterations: 1000,
            ..Limits::default()
        },
        ..Options::default()
    };
    let plain = exec(
        "var x = 0\nwhile x < 5 {\n  if x == 2 { x = 2 } else { x += 1 }\n}",
        &options,
    )
    .unwrap_err();
    assert!(plain.contains("this loop ran 1000 times without finishing"), "{plain}");
}

#[test]
fn loops_that_rarely_end_finish() {
    // It leaves with a chance of 10⁻⁹ a round: unrolling would need about
    // 3 × 10¹⁰ rounds to get within 10⁻¹² of the answer.
    let src = "var done = false
while not done {
  done ~ bernoulli(0.000000001)
}
report done";
    let out = outcome(src);
    exactly(out.reports[0].chance().unwrap(), 1.0);
    let options = Options {
        solve: false,
        limits: Limits {
            max_iterations: 100_000,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert!(exec(src, &options).unwrap_err().contains("without finishing"));
}

#[test]
fn observations_inside_a_solved_loop_weigh_its_worlds() {
    // Each round halves the weight and moves on with a chance of 50%: the
    // evidence is E[0.5^T] for T rounds to move twice, (1/3)² = 1/9.
    let out = outcome(
        "var s = 0
while s < 2 {
  observe 50%
  s = if 50% { s + 1 } else { s }
}
report s",
    );
    exactly(out.evidence.unwrap().to_f64(), 1.0 / 9.0);
    assert!(out.output.starts_with("enumerated · evidence 11.11%"), "{}", out.output);
}

#[test]
fn a_loop_that_rules_out_all_its_worlds_is_impossible_evidence() {
    let error = error("var x = 0\nwhile x == 0 {\n  observe 50%\n}\nreport x");
    assert!(error.contains("the evidence is impossible"), "{error}");
}

#[test]
fn returns_breaks_and_continues_leave_or_go_round() {
    // A 6 first returns 1, a 1 first returns 2: 6/11 against 5/11.
    let out = outcome(
        "fn race() -> int {
  while true {
    if d6 == 6 { return 1 }
    if d6 == 1 { return 2 }
  }
  return 0
}
report race() == 1",
    );
    exactly(out.reports[0].chance().unwrap(), 6.0 / 11.0);
    let out = outcome(
        "var x = 0
loop {
  x = if 50% { 0 } else { x + 1 }
  if x == 0 { continue }
  if x == 3 { break }
}
report x",
    );
    assert_eq!(out.reports[0].distribution(), [(Value::Int(3.into()), 1.0)]);
    assert_eq!(out.stats.solved_loops, 1);
}

#[test]
fn loops_that_report_print_or_count_are_unrolled() {
    // A report or print happens on every visit, so those loops aren't
    // solved; a loop that counts its rounds never comes back to a state.
    for src in [
        "var x = 0\nwhile x < 3 {\n  x = if 50% { 0 } else { x + 1 }\n  report x by x\n}",
        "var x = 0\nwhile x < 3 {\n  x = if 50% { 0 } else { x + 1 }\n  print(x)\n}",
        "var x = 0\nvar n = 0\nwhile x < 3 {\n  x = if 50% { 0 } else { x + 1 }\n  n += 1\n}\nreport n",
    ] {
        assert_eq!(outcome(src).stats.solved_loops, 0, "{src}");
    }
}

#[test]
fn chains_too_large_are_unrolled() {
    // Twelve in a row: 13 states, and about 8,000 rounds on average.
    let src = "var x = 0
while x < 12 {
  x = if 50% { 0 } else { x + 1 }
}
report x == 12";
    let options = Options {
        limits: Limits {
            max_chain_states: 10,
            ..Limits::default()
        },
        ..Options::default()
    };
    let small = run_with(src, &options);
    assert_eq!(small.stats.solved_loops, 0);
    let solved = outcome(src);
    assert_eq!(solved.stats.solved_loops, 1);
    exactly(solved.reports[0].chance().unwrap(), 1.0);
}

#[test]
fn memoized_functions_with_solved_loops_agree_with_unrolling() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../benches/tennis.probl")).unwrap();
    let solved = outcome(&src);
    assert!(solved.stats.solved_loops > 0);
    assert!(solved.unresolved.is_zero());
    let plain = run_with(&src, &unrolled());
    for (a, b) in solved.reports.iter().zip(&plain.reports) {
        for ((x, p), (y, q)) in a.distribution().iter().zip(b.distribution()) {
            assert_eq!(x, &y);
            assert!((p - q).abs() < 1e-12, "{x:?}: {p} against {q}");
        }
    }
}

/// A random loop over two small counters that wrap around, with chances,
/// observations, `break`, `continue` and an inner loop. Every round may
/// break first, so no world is stuck in it.
fn random_loop(rng: &mut Rng) -> String {
    let mut pick = |n: usize| (rng.uniform() * n as f64) as usize;
    let mut body = vec![format!("  if {}% {{ break }}", 1 + pick(30))];
    for _ in 0..2 + pick(5) {
        let (v, k, m) = (["a", "b"][pick(2)], 1 + pick(3), 2 + pick(3));
        let p = 10 + pick(80);
        body.push(match pick(8) {
            0 => format!("  {v} = ({v} + {k}) mod {m}"),
            1 => format!("  {v} = if {p}% {{ ({v} + {k}) mod {m} }} else {{ {v} }}"),
            2 => format!("  if a == b {{ observe {p}% }}"),
            3 => format!("  if bernoulli({p}%) and {v} == {} {{ break }}", pick(m)),
            4 => format!("  if {v} == {} {{ continue }}", pick(m)),
            5 => format!("  {v} = chance {{ {p}% => 0, else => {} }}", pick(3)),
            6 => format!(
                "  var t = 0\n  while t < 2 {{\n    t = if {p}% {{ t + 1 }} else {{ t }}\n    {v} = ({v} + t) mod 3\n  }}"
            ),
            _ => format!("  {v} = ({v} + if {p}% {{ 1 }} else {{ 0 }}) mod {m}"),
        });
    }
    format!(
        "var a = {}\nvar b = {}\nwhile a != {} or b != {} {{\n{}\n}}\nreport a\nreport b",
        pick(3),
        pick(3),
        pick(3),
        pick(3),
        body.join("\n")
    )
}

/// Solving and unrolling agree on random loops that cycle: same errors, and
/// the same measure at every report within what unrolling leaves unresolved.
/// `PROBL_CHAIN_CASES` runs more of them.
#[test]
fn random_loops_agree_with_unrolling() {
    let cases: u64 = std::env::var("PROBL_CHAIN_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300);
    let mut rng = Rng::new(9);
    let (mut solved_loops, mut compared) = (0, 0);
    for _ in 0..cases {
        let src = random_loop(&mut rng);
        let limits = Limits {
            max_iterations: 20_000,
            ..Limits::default()
        };
        let solved = exec_raw(
            &src,
            &Options {
                limits: limits.clone(),
                ..Options::default()
            },
        );
        let plain = exec_raw(&src, &Options { limits, ..unrolled() });
        match (solved, plain) {
            (Ok(s), Ok(p)) => {
                solved_loops += s.stats.solved_loops;
                assert!(s.unresolved.to_f64() <= p.unresolved.to_f64() + 1e-15, "{src}");
                let slack = p.unresolved.to_f64() + 1e-12;
                let (zs, zp) = (
                    s.evidence.map_or(1.0, |z| z.to_f64()),
                    p.evidence.map_or(1.0, |z| z.to_f64()),
                );
                assert!((zs - zp).abs() <= slack, "evidence {zs} against {zp}:\n{src}");
                for (a, b) in s.reports.iter().zip(&p.reports) {
                    let (da, db) = (a.distribution(), b.distribution());
                    for (x, pa) in &da {
                        let pb = db.iter().find(|(y, _)| y == x).map_or(0.0, |(_, q)| *q);
                        assert!(
                            (pa - pb).abs() <= slack / zp.max(1e-300) + 1e-12,
                            "{x:?}: {pa} against {pb}:\n{src}"
                        );
                        compared += 1;
                    }
                }
            }
            (Err(s), Err(p)) => {
                // A loop some worlds can't leave: solving says so, and
                // unrolling runs out of iterations. And when observations
                // rule out every world, solving finds none left, while
                // unrolling stops with some unresolved.
                let stuck = s.message.contains("never leave") && p.message.contains("without finishing");
                let ruled_out = s.message.contains("evidence is impossible") && p.message.contains("left unresolved");
                assert!(
                    stuck || ruled_out || s.message == p.message,
                    "{}\nagainst\n{}\n{src}",
                    s.message,
                    p.message
                );
            }
            // Unrolling can run out of iterations where solving finishes.
            (Ok(_), Err(p)) if p.message.contains("without finishing") => {}
            (s, p) => panic!("one failed:\n{:?}\n{:?}\n{src}", s.err(), p.err()),
        }
    }
    eprintln!("{cases} loops, {solved_loops} solved, {compared} probabilities compared");
    assert!(
        solved_loops >= cases / 3,
        "only {solved_loops} of {cases} loops were solved"
    );
}
