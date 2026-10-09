//! Rounding a continuous draw when enumerating: `floor`, `ceil`, `trunc`
//! and `round` of a draw with bounded values are ints with finitely many
//! values, each with its probability, tied to the draw
//! (docs/semantics.md, section 13).
mod common;
use common::*;
use probl_engine::value::Value;

fn int(n: i64) -> Value {
    Value::Int(n.into())
}

/// A report's values and probabilities.
#[track_caller]
fn is(src: &str, expected: &[(i64, f64)]) {
    let d = distribution(src);
    assert_eq!(d.len(), expected.len(), "{src}: {d:?}");
    for ((v, p), (k, q)) in d.iter().zip(expected) {
        assert_eq!(*v, int(*k), "{src}");
        close(*p, *q);
    }
}

#[test]
fn rounding_splits_the_draw_into_bins() {
    let third = 1.0 / 3.0;
    is(
        "let x ~ uniform(0, 3)\nreport floor(x)",
        &[(0, third), (1, third), (2, third)],
    );
    is(
        "let x ~ uniform(0, 3)\nreport ceil(x)",
        &[(1, third), (2, third), (3, third)],
    );
    // Halves away from zero, as `round` does; a half has no probability.
    is(
        "let x ~ uniform(0, 3)\nreport round(x)",
        &[(0, 1.0 / 6.0), (1, third), (2, third), (3, 1.0 / 6.0)],
    );
    // Toward zero, on both sides of it.
    is(
        "let x ~ uniform(-1.5, 1.5)\nreport trunc(x)",
        &[(-1, 1.0 / 6.0), (0, 2.0 * third), (1, 1.0 / 6.0)],
    );
    // A decreasing function of the draw, and a scaled one.
    is(
        "let x ~ uniform(0, 3)\nreport floor(-x)",
        &[(-3, third), (-2, third), (-1, third)],
    );
    is(
        "let x ~ uniform(0, 1)\nreport floor(4 * x)",
        &[(0, 0.25), (1, 0.25), (2, 0.25), (3, 0.25)],
    );
    // An atom rounds as a number does: max(x, 0.5) is 0.5 half the time.
    is("let x ~ uniform(0, 1)\nreport round(max(x, 0.5))", &[(1, 1.0)]);
    // A normal on part of its range.
    let phi = |z: f64| 0.5 * libm::erfc(-z / std::f64::consts::SQRT_2);
    let o = outcome("let z ~ normal(0, 1)\nobserve z > 0 and z < 2\nreport floor(z) == 0");
    close(o.reports[0].chance().unwrap(), (phi(1.0) - 0.5) / (phi(2.0) - 0.5));
}

#[test]
fn a_rounded_draw_conditions_the_draw() {
    let o = outcome("let x ~ uniform(0, 3)\nobserve floor(x) == 1\nreport x");
    close(o.evidence.unwrap().to_f64(), 1.0 / 3.0);
    let s = o.output;
    assert!(s.contains("mean 1.50") && s.contains("5% 1.05"), "{s}");
    // Named, it's a plain int in each world, with the draw restricted to its bin.
    let o = outcome(
        "let x ~ uniform(0, 3)\nlet k = floor(x)\nreport typeof k\nreport [\"a\", \"b\", \"c\"][k]\nreport k mod 2\nreport x by k",
    );
    assert_eq!(o.reports[0].distribution(), vec![(Value::str("int"), 1.0)]);
    assert_eq!(o.reports[1].distribution().len(), 3);
    close(o.reports[2].distribution()[1].1, 1.0 / 3.0);
    assert!(o.output.contains("  1   1.05   1.25     1.50"), "{}", o.output);
    // An annotation takes it, and observing the name restricts the draw.
    let o = outcome("let y ~ uniform(0, 3)\nlet j: int = floor(y)\nobserve j == 2\nreport y");
    assert!(o.output.contains("mean 2.50"), "{}", o.output);
}

#[test]
fn rounded_outcomes_are_ints_in_arithmetic() {
    let o = outcome(
        "let x ~ uniform(0, 3)\nreport floor(x) + 1\nreport floor(x) * 0.5\nreport floor(x) / 2\nlet k = floor(x) * 2\nreport typeof k",
    );
    assert_eq!(o.reports[0].distribution()[0].0, int(1));
    assert_eq!(o.reports[1].distribution()[1].0, Value::Float(0.5));
    assert_eq!(o.reports[2].distribution()[1].0, Value::Float(0.5));
    assert_eq!(o.reports[3].distribution(), vec![(Value::str("int"), 1.0)]);
}

#[test]
fn what_rounding_cant_do_yet() {
    for (tail, what) in [
        ("let z ~ normal(0, 1)\nreport floor(z)", "values have no bound"),
        ("let x ~ uniform(0, 3)\nreport round(x, 1)", "decimal places"),
        (
            "let x ~ uniform(0, 1)\nreport floor(1000000 * x)",
            "pieces is over the limit",
        ),
        (
            "let x ~ uniform(0, 3)\nlet k: int = x\nreport k",
            "converting this outcome to int",
        ),
        (
            "let x ~ uniform(0, 3)\nreport [1, 2, 3][floor(x)]",
            "int outcome of a continuous draw",
        ),
    ] {
        let e = error(tail);
        assert!(e.contains(what), "{tail}: {e}");
    }
}
