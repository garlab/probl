//! The experimental symbolic backend (docs/design/symbolic-inference.md): a
//! program answered with decision diagrams gives what enumeration gives,
//! and one it doesn't support is enumerated, or rejected when only diagrams
//! may answer. Generated programs are checked against the oracle too
//! (crates/probl-oracle/tests/differential.rs).
mod common;
use common::*;
use probl_engine::{ErrorKind, Limits, Options, Outcome, Symbolic};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn with(symbolic: Symbolic) -> Options {
    Options {
        symbolic,
        ..Options::default()
    }
}

/// Answered by diagrams: what enumeration gives, number for number.
#[track_caller]
fn answered(src: &str) -> Outcome {
    let o = exec(src, &with(Symbolic::Only)).unwrap_or_else(|e| panic!("{e}"));
    assert!(o.stats.diagrams.is_some(), "not answered by diagrams");
    same(&o, &outcome(src));
    o
}

#[track_caller]
fn same(a: &Outcome, b: &Outcome) {
    let near = |x: f64, y: f64| (x - y).abs() <= 1e-12 * x.abs().max(y.abs()).max(1e-300);
    match (a.evidence, b.evidence) {
        (Some(x), Some(y)) => assert!((x.ln() - y.ln()).abs() < 1e-12, "evidence {x:?}, {y:?}"),
        (x, y) => assert_eq!(x.is_some(), y.is_some(), "observed"),
    }
    assert_eq!(a.reports.len(), b.reports.len());
    for (s, t) in a.reports.iter().zip(&b.reports) {
        assert!(near(s.reach().ratio(a.finished), t.reach().ratio(b.finished)), "reach");
        let (d, e) = (s.distribution(), t.distribution());
        assert_eq!(d.len(), e.len(), "{d:?} {e:?}");
        for ((v, p), (w, q)) in d.iter().zip(&e) {
            assert_eq!(v, w);
            assert!(near(*p, *q), "{v:?}: {p} and {q}");
        }
    }
    assert_eq!(a.output, b.output);
}

/// `p / P(at least one of n)`, and that evidence, for `n` independent
/// faults with probability `p`, given that one happened.
fn faults(n: usize, p: f64) -> (f64, f64) {
    let evidence = -(n as f64 * (-p).ln_1p()).exp_m1();
    (p / evidence, evidence)
}

fn faults_program(n: usize, p: &str) -> String {
    let names: Vec<String> = (0..n).map(|i| format!("f{i}")).collect();
    let mut src: String = names.iter().map(|f| format!("let {f} ~ bernoulli({p})\n")).collect();
    // In parts: an expression can't nest too deeply.
    let parts: Vec<String> = names.chunks(50).map(|c| format!("({})", c.join(" or "))).collect();
    src += &format!("observe {}\n", parts.join(" or "));
    for f in &names {
        src += &format!("report {f}\n");
    }
    src
}

#[test]
fn draws_keep_their_identity() {
    let o = answered(
        "let d = bernoulli(30%)\nlet a ~ d\nlet b = a\nlet c ~ d\nreport a == b\nreport a == c\nreport a != c",
    );
    close(o.reports[0].chance().unwrap(), 1.0);
    close(o.reports[1].chance().unwrap(), 0.58);
    close(o.reports[2].chance().unwrap(), 0.42);
    // A probability condition is a fresh trial each time.
    let o = answered("let a = if 30% { true } else { false }\nlet b = if 30% { true } else { false }\nreport a and b");
    close(o.reports[0].chance().unwrap(), 0.09);
}

#[test]
fn evidence_conditions_every_report() {
    let o = answered(&faults_program(12, "1%"));
    let (posterior, evidence) = faults(12, 0.01);
    for r in &o.reports {
        assert!((r.chance().unwrap() / posterior - 1.0).abs() < 1e-13);
    }
    assert!((o.evidence.unwrap().to_f64() / evidence - 1.0).abs() < 1e-13);
}

#[test]
fn many_live_facts_beyond_enumeration() {
    // 2^200 assignments: enumeration can't hold them, diagrams don't need to.
    let src = faults_program(200, "1%");
    let o = exec(&src, &with(Symbolic::Only)).unwrap();
    let (posterior, evidence) = faults(200, 0.01);
    for r in &o.reports {
        assert!((r.chance().unwrap() / posterior - 1.0).abs() < 1e-12);
    }
    assert!((o.evidence.unwrap().to_f64() / evidence - 1.0).abs() < 1e-12);
    assert_eq!(o.stats.diagrams.unwrap().variables, 200);
    let limited = Options {
        limits: Limits {
            max_worlds: 100_000,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(exec_raw(&src, &limited).unwrap_err().kind, ErrorKind::Limit);
}

#[test]
fn observations_apply_where_they_run() {
    let o = answered(
        "let a ~ bernoulli(30%)\nlet b ~ bernoulli(50%)\nif a { observe b }\nreport a\nreport b\nif not a { report b }",
    );
    // a and b: 15%; a without b is ruled out; not a: 70%.
    close(o.evidence.unwrap().to_f64(), 0.85);
    close(o.reports[0].chance().unwrap(), 0.15 / 0.85);
    close(o.reports[1].chance().unwrap(), 0.5 / 0.85);
    close(o.reports[2].chance().unwrap(), 0.5);
    close(o.reports[2].reach().ratio(o.finished), 0.7 / 0.85);
    // Assignments in branches, and a report no world reaches.
    answered(
        "let a ~ bernoulli(30%)\nvar x = false\nif a { x = ~bernoulli(50%) } else { x = not a }\nobserve x\nreport a\nif a and not x { report x }",
    );
}

#[test]
fn impossible_evidence_fails_at_the_same_observation() {
    let src =
        "let a ~ bernoulli(30%)\nlet b ~ bernoulli(50%)\nobserve a or b\nif a { observe false }\nobserve a\nreport b";
    let symbolic = exec(src, &with(Symbolic::Only)).unwrap_err();
    assert_eq!(symbolic, exec(src, &Options::default()).unwrap_err());
    assert!(symbolic.contains("the evidence is impossible"), "{symbolic}");
}

#[test]
fn certain_draws_are_constants() {
    let o = answered("let a ~ bernoulli(0%)\nlet b ~ bernoulli(100%)\nlet c ~ bernoulli(1)\nreport a\nreport b and c");
    assert_eq!(o.stats.diagrams.unwrap().variables, 0);
    close(o.reports[0].chance().unwrap(), 0.0);
    close(o.reports[1].chance().unwrap(), 1.0);
}

#[test]
fn tiny_evidence_keeps_its_precision() {
    // Forty facts of probability 10⁻¹⁰, all seen: evidence 10⁻⁴⁰⁰, far
    // below an f64. Seeing `g or f0` then changes nothing.
    let names: Vec<String> = (0..40).map(|i| format!("f{i}")).collect();
    let mut src: String = names
        .iter()
        .map(|f| format!("let {f} ~ bernoulli(0.0000000001)\n"))
        .collect();
    src += "let g ~ bernoulli(50%)\n";
    for f in &names {
        src += &format!("observe {f}\n");
    }
    src += "observe g or f0\nreport g\nreport f1\n";
    let o = answered(&src);
    assert!((o.evidence.unwrap().log10() + 400.0).abs() < 1e-9);
    close(o.reports[0].chance().unwrap(), 0.5);
    close(o.reports[1].chance().unwrap(), 1.0);
    // A posterior of 10⁻²⁰ keeps its digits.
    let o = answered("let a ~ bernoulli(0.0000000001)\nlet b ~ bernoulli(0.0000000001)\nreport a and b");
    assert!((o.reports[0].chance().unwrap() / 1e-20 - 1.0).abs() < 1e-12);
}

#[test]
fn laws_that_depend_on_facts() {
    // The diagnosis pattern: a symptom's law depends on the cause, and a
    // test's likelihood on the condition.
    let o = answered(
        "let sick ~ bernoulli(1%)\nscore if sick { 95% } else { 8% }\nlet burglary ~ bernoulli(1%)\nlet alarm ~ if burglary { bernoulli(95%) } else { bernoulli(1%) }\nobserve ~bernoulli(10%)\nobserve alarm\nreport sick\nreport burglary",
    );
    close(o.reports[0].chance().unwrap(), 0.0095 / (0.0095 + 0.99 * 0.08));
    close(o.reports[1].chance().unwrap(), 0.0095 / (0.0095 + 0.99 * 0.01));
    close(
        o.evidence.unwrap().to_f64(),
        (0.0095 + 0.99 * 0.08) * 0.1 * (0.0095 + 0.99 * 0.01),
    );
    // Values other than booleans: a probability chosen by a branch, a
    // constant report, and a reported law.
    answered(
        "let a ~ bernoulli(30%)\nvar p = 10%\nif a { p = 90% }\nlet b ~ bernoulli(p)\nobserve true from bernoulli(p)\nreport b\nreport p\nreport bernoulli(25%)\nreport 30%",
    );
    answered("let x: bool ~ bernoulli(50%)\nlet y = ~x\nobserve ~y\nreport x");
}

#[test]
fn unsupported_programs_are_enumerated_or_rejected() {
    for (src, what) in [
        ("let x ~ d6\nreport x > 3", "dice"),
        ("var n = 0\nwhile ~bernoulli(50%) { n += 1 }\nreport n", "a loop"),
        (
            "fn f() { ~bernoulli(50%) }\nlet x = f()\nreport x",
            "calling a function",
        ),
        ("let a ~ bernoulli(50%)\nreport a by a", "`by`"),
        ("let a ~ bernoulli(50%)\nprint(\"{a}\")\nreport a", "`print`"),
        ("let x = chance { 30% => true, else => false }\nreport x", "`chance`"),
        // A probability out of range fails where enumeration reaches it.
        (
            "let p = 1.5\nlet a ~ bernoulli(50%)\nif a { let x ~ bernoulli(p)\nreport x }",
            "an operation that fails",
        ),
    ] {
        let e = exec_raw(src, &with(Symbolic::Only)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported, "{src}");
        assert!(e.message.contains("isn't supported by the symbolic backend"), "{e:?}");
        assert!(e.message.contains(what), "{src}: {}", e.message);
        // Preferred, they're enumerated, as without diagrams.
        match (exec(src, &with(Symbolic::Prefer)), exec(src, &Options::default())) {
            (Ok(a), Ok(b)) => {
                assert!(a.stats.diagrams.is_none());
                assert_eq!(a.output, b.output);
            }
            (Err(a), Err(b)) => assert_eq!(a, b),
            (a, b) => panic!("{src}: {a:?} and {b:?}"),
        }
    }
    // An error in a branch no world reaches stays unreached.
    let o = answered("let p = 1.5\nlet a ~ bernoulli(0%)\nif a { let x ~ bernoulli(p)\nreport x }\nreport a");
    assert!(o.reports[0].groups.is_empty());
}

#[test]
fn the_node_limit_and_cancellation() {
    let src = faults_program(12, "1%");
    let limits = Limits {
        max_nodes: 30,
        ..Limits::default()
    };
    let only = Options {
        limits: limits.clone(),
        ..with(Symbolic::Only)
    };
    let e = exec_raw(&src, &only).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    assert!(e.message.contains("more than 30 nodes"), "{}", e.message);
    // Preferred, it's enumerated instead.
    let prefer = Options {
        limits,
        ..with(Symbolic::Prefer)
    };
    let o = exec(&src, &prefer).unwrap();
    assert!(o.stats.diagrams.is_none());
    assert_eq!(o.output, outcome(&src).output);
    // A cancelled run stops, rather than being enumerated.
    let cancelled = Options {
        cancel: Some(Arc::new(AtomicBool::new(true))),
        ..with(Symbolic::Prefer)
    };
    let e = exec_raw(&src, &cancelled).unwrap_err();
    assert!(e.message.contains("cancelled"), "{}", e.message);
}

#[test]
fn sampling_ignores_the_backend() {
    let src = "@mode sample(runs: 1_000, seed: 3)\nlet a ~ bernoulli(30%)\nreport a";
    let o = exec(src, &with(Symbolic::Only)).unwrap();
    assert!(o.stats.diagrams.is_none());
    assert_eq!(o.output, output(src));
}
