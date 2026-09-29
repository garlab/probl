//! Exact integer mathematics, floating-point special functions, and their domains.

mod common;

use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};

fn integer(src: &str) -> i64 {
    let values = distribution(src);
    match values.as_slice() {
        [(Value::Int(n), p)] if *p == 1.0 => *n,
        _ => panic!("expected one exact integer from {src}: {values:?}"),
    }
}

#[test]
fn combinations_and_factorials_are_exact() {
    for (n, k, expected) in [
        (0, 0, 1),
        (0, 1, 0),
        (5, 6, 0),
        (5, 0, 1),
        (5, 5, 1),
        (52, 5, 2_598_960),
        (66, 33, 7_219_428_434_016_265_740),
        (9_007_199_254_740_993, 1, 9_007_199_254_740_993),
        (i64::MAX, 1, i64::MAX),
        (i64::MAX, i64::MAX - 1, i64::MAX),
    ] {
        assert_eq!(integer(&format!("report choose({n}, {k})")), expected);
    }
    assert_eq!(integer("report factorial(0)"), 1);
    assert_eq!(integer("report factorial(1)"), 1);
    assert_eq!(integer("report factorial(10)"), 3_628_800);
    assert_eq!(integer("report factorial(20)"), 2_432_902_008_176_640_000);

    // Compare with Pascal's triangle, independently of the multiplicative formula.
    let mut row = vec![1i64];
    for n in 0..=16 {
        for (k, expected) in row.iter().enumerate() {
            assert_eq!(integer(&format!("report choose({n}, {k})")), *expected);
        }
        let mut next = vec![1];
        next.extend(row.windows(2).map(|pair| pair[0] + pair[1]));
        next.push(1);
        row = next;
    }
}

#[test]
fn divisors_and_multiples_handle_signs_zero_and_large_integers() {
    for (source, expected) in [
        ("gcd(54, 24)", 6),
        ("gcd(-54, 24)", 6),
        ("gcd(0, -24)", 24),
        ("gcd(0, 0)", 0),
        ("gcd(9007199254740993, 3)", 3),
        ("gcd(-9223372036854775807 - 1, 2)", 2),
        ("lcm(21, 6)", 42),
        ("lcm(-21, -6)", 42),
        ("lcm(0, 0)", 0),
        ("lcm(-9223372036854775807 - 1, 0)", 0),
        (
            "lcm(6000000000000000000, 3000000000000000000)",
            6_000_000_000_000_000_000,
        ),
    ] {
        assert_eq!(integer(&format!("report {source}")), expected);
    }
    for a in 1..=12 {
        for b in 1..=12 {
            let divisor = (1..=a.min(b)).rev().find(|d| a % d == 0 && b % d == 0).unwrap();
            assert_eq!(integer(&format!("report gcd({a}, {b})")), divisor);
            assert_eq!(integer(&format!("report lcm({a}, {b})")), a * b / divisor);
        }
    }
}

#[test]
fn totients_match_counting_coprime_integers() {
    for n in 1..=100 {
        let expected = (1..=n).filter(|k| !(2..=n).any(|d| n % d == 0 && k % d == 0)).count() as i64;
        assert_eq!(integer(&format!("report euler_phi({n})")), expected);
    }
    assert_eq!(integer("report euler_phi(65537)"), 65_536);
    assert_eq!(
        integer("report euler_phi(4611686018427387904)"),
        2_305_843_009_213_693_952
    );
    assert_eq!(integer("report euler_phi(1000000000)"), 400_000_000);
}

#[test]
fn special_functions_match_known_values_and_identities() {
    close(mean("report ln_gamma(1)"), 0.0);
    close(mean("report ln_gamma(2)"), 0.0);
    close(mean("report ln_gamma(0.5)"), 0.5 * std::f64::consts::PI.ln());
    close(mean("report ln_gamma(6)"), 120.0f64.ln());
    close(mean("report ln_gamma(1000)"), 5_905.220_423_209_181);
    for x in [0.1, 0.5, 2.5, 100.0] {
        close(
            mean(&format!("report ln_gamma({x} + 1) - ln_gamma({x}) - ln({x})")),
            0.0,
        );
    }
    close(mean("report erf(0)"), 0.0);
    close(mean("report erf(1)"), 0.842_700_792_949_714_9);
    close(mean("report erf(-1)"), -0.842_700_792_949_714_9);
    close(mean("report erf(30)"), 1.0);
    close(mean("report erf(-30)"), -1.0);
    close(mean("report erf(1e-20) / 1e-20"), 2.0 / std::f64::consts::PI.sqrt());
    close(mean("report erf(1e-300) / 1e-300"), 2.0 / std::f64::consts::PI.sqrt());
}

#[test]
fn domains_types_arity_and_overflow_are_checked() {
    for source in [
        "choose(-1, 0)",
        "choose(3, -1)",
        "factorial(-1)",
        "euler_phi(-1)",
        "euler_phi(0)",
    ] {
        assert!(error(&format!("report {source}")).contains("needs"), "{source}");
    }
    for source in [
        "choose(3.0, 1)",
        "choose(3, 1.0)",
        "factorial(true)",
        "factorial(3.5)",
        "gcd(5.0, 1)",
        "lcm(1, 50%)",
        "euler_phi(1.0)",
    ] {
        assert!(error(&format!("report {source}")).contains("needs an int"), "{source}");
    }
    for source in ["ln_gamma(0)", "ln_gamma(-0.5)", "ln_gamma(1e308)", "erf(0 ^ -1)"] {
        assert!(
            error(&format!("report {source}")).contains("isn't defined for"),
            "{source}"
        );
    }
    for source in [
        "choose(67, 33)",
        "choose(9223372036854775807, 2)",
        "factorial(21)",
        "factorial(9223372036854775807)",
        "gcd(-9223372036854775807 - 1, 0)",
        "lcm(-9223372036854775807 - 1, 1)",
        "lcm(9223372036854775807, 2)",
    ] {
        assert!(
            error(&format!("report {source}")).contains("integer overflow"),
            "{source}"
        );
    }
    for source in [
        "choose(5)",
        "factorial()",
        "gcd(1)",
        "lcm(1)",
        "euler_phi()",
        "ln_gamma()",
        "erf(1, 2)",
    ] {
        assert!(compile_error(&format!("report {source}")).contains("takes"), "{source}");
    }
}

#[test]
fn integer_work_respects_host_limits() {
    let options = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    let err = exec_raw("report euler_phi(1000000007)", &options).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Limit);
    assert!(err.message.contains("work budget"));
    // Enormous arguments must overflow promptly rather than loop up to n or k.
    for source in [
        "factorial(9223372036854775807)",
        "choose(9223372036854775807, 4611686018427387903)",
    ] {
        let err = exec_raw(&format!("report {source}"), &options).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Language);
        assert!(err.message.contains("integer overflow"));
    }
}

#[test]
fn functions_lift_over_distributions_in_both_modes() {
    for mode in ["enumerate", "sample(runs: 1000, seed: 1)"] {
        for (expression, expected) in [
            ("choose(d2 + 2, 2)", 4.5),
            ("factorial(d3)", 3.0),
            ("gcd(d2, d2)", 1.25),
            ("lcm(d2, d2)", 1.75),
            ("euler_phi(d4)", 1.5),
            ("ln_gamma(one_of([1, 6]))", 0.5 * 120.0f64.ln()),
            ("erf(one_of([-1, 1]))", 0.0),
        ] {
            close(mean(&format!("@mode {mode}\nreport {expression}")), expected);
        }
    }
    assert!(error("report factorial(normal(0, 1))").contains("draw a value first"));
    assert!(error("report erf(normal(0, 1))").contains("draw a value first"));
}
