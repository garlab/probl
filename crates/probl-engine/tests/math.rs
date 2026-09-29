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

fn float(src: &str) -> f64 {
    let values = distribution(src);
    match values.as_slice() {
        [(Value::Float(n), p)] if *p == 1.0 => *n,
        _ => panic!("expected one float from {src}: {values:?}"),
    }
}

#[test]
fn erfc_preserves_small_tails() {
    close(float("report erfc(0)"), 1.0);
    close(float("report erfc(1)"), 0.157_299_207_050_285_13);
    close(float("report erfc(-1)"), 1.842_700_792_949_714_8);
    close(float("report erfc(8) / 1.1224297172982928e-29"), 1.0);
    // Subtracting the rounded erf value loses this tail completely.
    assert_eq!(float("report 1 - erf(8)"), 0.0);
    assert_eq!(float("report erfc(30)"), 0.0);
    assert_eq!(float("report erfc(-30)"), 2.0);
    for x in [0.125, 0.5, 2.0, 8.0] {
        close(mean(&format!("report erfc({x}) + erfc(-{x})")), 2.0);
    }
}

#[test]
fn cube_roots_and_base_two_exponentials_handle_extreme_scales() {
    for (expression, expected) in [
        ("cbrt(-8)", -2.0),
        ("cbrt(0)", 0.0),
        ("cbrt(27)", 3.0),
        ("cbrt(12.5%)", 0.5),
        ("cbrt(-1e300) / 1e100", -1.0),
        ("cbrt(1e-300) / 1e-100", 1.0),
        ("exp2(0)", 1.0),
        ("exp2(-3)", 0.125),
        ("exp2(10)", 1024.0),
        ("exp2(0.5)", std::f64::consts::SQRT_2),
    ] {
        close(float(&format!("report {expression}")), expected);
    }
    assert_eq!(float("report exp2(1023)"), f64::from_bits(2046u64 << 52));
    assert_eq!(float("report exp2(-1074)"), f64::from_bits(1));
    assert_eq!(float("report exp2(-1075)"), 0.0);
    for x in [0.125, 1.0, 3.0, 100.0] {
        close(mean(&format!("report exp2(log2({x}))")), x);
    }
}

#[test]
fn trunc_rounds_toward_zero_and_integer_rounding_uses_the_full_range() {
    for (expression, expected) in [
        ("trunc(1.9)", 1),
        ("trunc(-1.9)", -1),
        ("trunc(-1e-300)", 0),
        ("trunc(90%)", 0),
        ("trunc(9007199254740993)", 9_007_199_254_740_993),
        ("trunc(9223372036854775807)", i64::MAX),
        ("trunc(-9223372036854775807 - 1)", i64::MIN),
    ] {
        assert_eq!(integer(&format!("report {expression}")), expected);
    }
    // All four functions share the float-to-int conversion. The old decimal
    // cutoff rejected valid values near the boundary; casts must never saturate.
    for name in ["trunc", "floor", "ceil", "round"] {
        assert_eq!(
            integer(&format!("report {name}(9.223372036854775e18)")),
            9_223_372_036_854_774_784
        );
        assert_eq!(integer(&format!("report {name}(-9.223372036854776e18)")), i64::MIN);
        for input in ["9.223372036854776e18", "-9.223372036854778e18", "1e308", "0 ^ -1"] {
            assert!(error(&format!("report {name}({input})")).contains("too large to be an int"));
        }
    }
}

#[test]
fn libm_batch_checks_domains_types_and_arities() {
    for name in ["erfc", "cbrt", "exp2", "trunc"] {
        assert!(error(&format!("report {name}(true)")).contains("needs a number"));
        assert!(compile_error(&format!("report {name}()")).contains("takes"));
        assert!(compile_error(&format!("report {name}(1, 2)")).contains("takes"));
        assert!(error(&format!("report {name}(normal(0, 1))")).contains("draw a value first"));
    }
    for expression in ["erfc(0 ^ -1)", "cbrt(0 ^ -1)", "exp2(0 ^ -1)", "exp2(1024)"] {
        assert!(error(&format!("report {expression}")).contains("isn't defined for"));
    }
}

#[test]
fn round_retains_the_one_argument_form_and_supports_decimal_places() {
    assert_eq!(integer("report round(1.5)"), 2);
    assert_eq!(integer("report round(-1.5)"), -2);
    assert_eq!(integer("report round(9007199254740993)"), 9_007_199_254_740_993);
    for (expression, expected) in [
        ("round(1.234, 2)", 1.23),
        ("round(1.125, 2)", 1.13),
        ("round(-1.125, 2)", -1.13),
        ("round(1.5, 0)", 2.0),
        ("round(-1.5, 0)", -2.0),
        ("round(1250.0, -2)", 1300.0),
        ("round(-1250.0, -2)", -1300.0),
        ("round(12.5%, 2)", 0.13),
        ("round(1e308, 2)", 1e308),
        ("round(1e308, -308)", 1e308),
        ("round(1e308, -309)", 0.0),
        ("round(1.234, 323)", 1.234),
        ("round(1.234, 9223372036854775807)", 1.234),
        ("round(1.234, -9223372036854775807 - 1)", 0.0),
    ] {
        assert_eq!(float(&format!("report {expression}")), expected, "{expression}");
    }
    close(float("report round(1.234e-300, 302) / 1e-300"), 1.23);
    close(float("report round(1.234e-310, 312) / 1e-310"), 1.23);
    assert_eq!(float("report round(5e-324, 323)"), 0.0);
    assert_eq!(float("report round(5e-324, 324)"), f64::from_bits(1));
    assert!(error("report round(1.79e308, -308)").contains("isn't defined for"));
    assert!(error("report round(1e308)").contains("too large to be an int"));
}

#[test]
fn round_keeps_integer_inputs_exact_and_checks_overflow() {
    for (expression, expected) in [
        ("round(9007199254740993, 2)", 9_007_199_254_740_993),
        ("round(9007199254740993, -1)", 9_007_199_254_740_990),
        ("round(9007199254740995, -1)", 9_007_199_254_741_000),
        ("round(-1250, -2)", -1300),
        ("round(1250, -2)", 1300),
        ("round(9223372036854775807, 0)", i64::MAX),
        ("round(-9223372036854775807 - 1, 0)", i64::MIN),
        ("round(9223372036854775807, -20)", 0),
        ("round(-9223372036854775807 - 1, -20)", 0),
        ("round(100, -9223372036854775807 - 1)", 0),
        ("round(100, 9223372036854775807)", 100),
    ] {
        assert_eq!(integer(&format!("report {expression}")), expected, "{expression}");
    }
    for expression in [
        "round(9223372036854775807, -1)",
        "round(-9223372036854775807 - 1, -1)",
        "round(9223372036854775807, -19)",
    ] {
        assert!(error(&format!("report {expression}")).contains("integer overflow"));
    }
}

#[test]
fn inverse_hyperbolic_functions_match_values_and_invert_the_forward_functions() {
    close(float("report asinh(0)"), 0.0);
    close(float("report asinh(1)"), 0.881_373_587_019_543);
    close(float("report acosh(1)"), 0.0);
    close(float("report acosh(2)"), 1.316_957_896_924_816_6);
    close(float("report atanh(0.5)"), 0.549_306_144_334_054_8);
    for x in [-3.0, -1.0, 0.0, 0.5, 3.0] {
        close(mean(&format!("report asinh(sinh({x}))")), x);
        close(mean(&format!("report acosh(cosh({x}))")), x.abs());
        close(mean(&format!("report atanh(tanh({x}))")), x);
    }
    close(float("report asinh(1e-300) / 1e-300"), 1.0);
    close(float("report atanh(1e-300) / 1e-300"), 1.0);
    close(float("report asinh(1e308)"), 709.889_355_822_726);
    close(float("report acosh(1e308)"), 709.889_355_822_726);
}

#[test]
fn rounding_and_inverse_hyperbolic_domains_and_arities_are_checked() {
    for expression in [
        "round(1, 1.0)",
        "round(1, true)",
        "round(1, 50%)",
        "round(true, 2)",
        "asinh(true)",
        "acosh(\"1\")",
        "atanh([])",
    ] {
        assert!(error(&format!("report {expression}")).contains("needs"), "{expression}");
    }
    for expression in [
        "round(0 ^ -1, 2)",
        "round(0 ^ -1, 9223372036854775807)",
        "asinh(0 ^ -1)",
        "acosh(0.999)",
        "acosh(-1)",
        "acosh(0 ^ -1)",
        "atanh(-1)",
        "atanh(1)",
        "atanh(2)",
        "atanh(0 ^ -1)",
    ] {
        assert!(
            error(&format!("report {expression}")).contains("isn't defined for"),
            "{expression}"
        );
    }
    for expression in ["round()", "round(1, 2, 3)", "asinh()", "acosh(1, 2)", "atanh()"] {
        assert!(compile_error(&format!("report {expression}")).contains("takes"));
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
            ("erfc(one_of([-1, 1]))", 1.0),
            ("cbrt(one_of([-8, 27]))", 0.5),
            ("exp2(d2)", 3.0),
            ("trunc(one_of([-1.9, 2.9]))", 0.5),
            ("round(one_of([1.125, 2.125]), 2)", 1.63),
            ("round(1.25, one_of([0, 1]))", 1.15),
            ("asinh(one_of([-1, 1]))", 0.0),
            ("acosh(one_of([1, 2]))", 0.658_478_948_462_408_3),
            ("atanh(one_of([-0.5, 0.5]))", 0.0),
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
