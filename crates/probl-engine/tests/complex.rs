//! Complex scalars remain ordinary data in Probl's classical probability engine.

mod common;

use common::*;
use probl_engine::complex::Complex;
use probl_engine::value::Value;
use std::collections::{BTreeSet, HashSet};

fn value(expression: &str) -> Complex {
    let values = distribution(&format!("report {expression}"));
    match values.as_slice() {
        [(Value::Complex(z), p)] if *p == 1.0 => *z,
        _ => panic!("expected a complex scalar: {values:?}"),
    }
}

fn components(expression: &str, re: f64, im: f64) {
    let z = value(expression);
    close(z.re(), re);
    close(z.im(), im);
}

#[test]
fn construction_arithmetic_and_integer_powers() {
    for (expression, re, im) in [
        ("complex(3)", 3.0, 0.0),
        ("complex(complex(1, 2))", 1.0, 2.0),
        ("complex(1, 2) + complex(3, 4)", 4.0, 6.0),
        ("complex(1, 2) - complex(3, 4)", -2.0, -2.0),
        ("complex(1, 2) * complex(3, 4)", -5.0, 10.0),
        ("complex(1, 2) / complex(3, 4)", 0.44, 0.08),
        ("2 + complex(1, 2)", 3.0, 2.0),
        ("2 - complex(1, 2)", 1.0, -2.0),
        ("complex(1, 2) * 50%", 0.5, 1.0),
        ("2 / complex(1, 1)", 1.0, -1.0),
        ("-complex(1, 2)", -1.0, -2.0),
        ("complex(0, 1) ^ 2", -1.0, 0.0),
        ("complex(1, 1) ^ -2", 0.0, -0.5),
        ("complex(0) ^ 0", 1.0, 0.0),
        ("complex(0, 1) ^ 9223372036854775807", 0.0, -1.0),
        ("complex(0, 1) ^ (-9223372036854775807 - 1)", 1.0, 0.0),
    ] {
        components(expression, re, im);
    }
    components(
        "simulate { var z = complex(1, 2); z += 2; z *= complex(0, 1); z }",
        -2.0,
        3.0,
    );
}

#[test]
fn components_magnitude_conjugation_and_phase() {
    close(mean("report real(complex(3, 4))"), 3.0);
    close(mean("report imag(complex(3, 4))"), 4.0);
    close(mean("report real(3)"), 3.0);
    close(mean("report imag(3)"), 0.0);
    close(mean("report abs(complex(3, 4))"), 5.0);
    close(mean("report abs2(complex(3, 4))"), 25.0);
    close(mean("report abs2(-3)"), 9.0);
    close(mean("report arg(complex(0, 1))"), std::f64::consts::FRAC_PI_2);
    close(mean("report arg(complex(-1, -0.0))"), std::f64::consts::PI);
    close(mean("report arg(complex(0))"), 0.0);
    components("conj(complex(3, 4))", 3.0, -4.0);
    components("cis(pi / 2)", 0.0, 1.0);
    for angle in [-3.0, -1.0, 0.0, 0.5, 2.0] {
        close(mean(&format!("report abs2(cis({angle}))")), 1.0);
        components(&format!("cis({angle}) * conj(cis({angle}))"), 1.0, 0.0);
    }
}

#[test]
fn arithmetic_avoids_intermediate_overflow_and_underflow() {
    components("complex(1e308, 1e308) / complex(1e308, 1e308)", 1.0, 0.0);
    components("complex(1e-308, 1e-308) / complex(1e-308, 1e-308)", 1.0, 0.0);
    components("complex(1e308, -1e308) * complex(1e-308)", 1.0, -1.0);
    for op in ["*", "/"] {
        let z = value(&format!("complex(1e308, 1e-308) {op} complex(1)"));
        assert_eq!(z.re(), 1e308);
        assert_eq!(z.im(), 1e-308);
    }
    close(mean("report abs(complex(3e200, 4e200)) / 1e200"), 5.0);
    close(mean("report abs(complex(3e-200, 4e-200)) / 1e-200"), 5.0);
    assert_eq!(mean("report abs2(complex(1.2e-162, 1.2e-162))"), f64::from_bits(1));
    close(mean("report real(complex(1e160) ^ -2) / 1e-320"), 1.0);
}

#[test]
fn complex_equality_is_numeric_but_storage_identity_stays_typed() {
    for expression in [
        "complex(1) == 1",
        "1.0 == complex(1)",
        "complex(0.5) == 50%",
        "complex(1, 2) == complex(1, 2)",
        "complex(1, 2) != complex(1, -2)",
        "complex(1, 1) != 1",
        "complex(1) != true",
        "complex(9007199254740992) != 9007199254740993",
        "complex(-9223372036854775807 - 1) == (-9223372036854775807 - 1)",
    ] {
        close(chance(&format!("report {expression}")), 1.0);
    }
    let plus = Value::Complex(Complex::new(0.0, 0.0).unwrap());
    let minus = Value::Complex(Complex::new(-0.0, -0.0).unwrap());
    assert_eq!(plus, minus);
    assert_eq!(plus.cmp(&minus), std::cmp::Ordering::Equal);
    assert_eq!(HashSet::from([plus.clone(), minus.clone()]).len(), 1);
    assert_eq!(BTreeSet::from([plus.clone(), minus.clone()]).len(), 1);
    assert_ne!(plus, Value::Float(0.0));
    close(mean("report len(support(one_of([complex(0.0), complex(-0.0)])))"), 1.0);
}

#[test]
fn complex_values_work_in_typed_collections_functions_and_reports() {
    let source = r#"
type Pair = { z: complex }
fn rotate(z: complex) -> complex { z * complex(0, 1) }
let p = Pair { z: complex(1, 2) }
let zs: list[complex] = [p.z, rotate(p.z)]
let table: map[complex, str] = [p.z: "found"]
let d: dist[complex] = one_of(zs)
report sum(zs)
report table[complex(1, 2)]
report complex(1, 2) in zs
report len(support(d))
report "z: {p.z}"
"#;
    let reports = outcome(source).reports;
    assert_eq!(
        reports[0].distribution()[0].0,
        Value::Complex(Complex::new(-1.0, 3.0).unwrap())
    );
    assert_eq!(reports[1].distribution()[0].0, Value::str("found"));
    assert_eq!(reports[2].distribution()[0].0, Value::Bool(true));
    assert_eq!(reports[3].distribution()[0].0, Value::Int(2));
    assert_eq!(reports[4].distribution()[0].0, Value::str("z: complex(1.0, 2.0)"));
    assert!(compile_error("let z: complex = 1").contains("expected a complex"));
    assert!(error("fn f(z: complex) { z }\nreport f(1)").contains("complex"));
    assert!(compile_error("let z: complex = read(\"z.json\")").contains("data can't be `complex`"));
}

#[test]
fn complex_distributions_are_classical_mixtures_in_both_modes() {
    for mode in ["enumerate", "sample(runs: 1000, seed: 1)"] {
        let source = format!("@mode {mode}\nreport one_of([complex(0, 1), complex(0, -1)])");
        let values = distribution(&source);
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|(_, p)| *p == 0.5));
        close(mean(&format!("@mode {mode}\nreport abs2(complex(d2, d2))")), 5.0);
        close(mean(&format!("@mode {mode}\nreport abs2(cis(one_of([0, pi])))")), 1.0);
        let source = format!("@mode {mode}\nreport mean(one_of([complex(1, 2), 3]))");
        assert_eq!(
            distribution(&source)[0].0,
            Value::Complex(Complex::new(2.0, 1.0).unwrap())
        );
    }
    close(
        mean("report pmf(one_of([complex(0, 1), complex(0, -1)]), complex(0, 1))"),
        0.5,
    );
}

#[test]
fn a_phase_interference_calculation_works_as_ordinary_data() {
    let source = r#"
fn h(s: list[complex]) -> list[complex] {
    [(s[0] + s[1]) / sqrt(2), (s[0] - s[1]) / sqrt(2)]
}
let first = h([complex(1), complex(0)])
let second = h([first[0], first[1] * cis(pi)])
report abs2(second[0])
report abs2(second[1])
"#;
    let reports = outcome(source).reports;
    close(reports[0].distribution()[0].0.as_f64().unwrap(), 0.0);
    close(reports[1].distribution()[0].0.as_f64().unwrap(), 1.0);
}

#[test]
fn complex_values_never_become_probabilities_or_ordered_reals() {
    // Adding an unordered numeric type must not break existing categorical queries.
    close(chance("report median(bernoulli(75%))"), 1.0);
    for expression in [
        "complex(1) < complex(2)",
        "complex(1) >= 0",
        "min(complex(1), complex(2))",
        "sort([complex(2), complex(1)])",
        "median(one_of([complex(1), complex(2)]))",
        "quantile(one_of([[complex(1)], [complex(2)]]), 50%)",
        "cdf(one_of([complex(1), complex(2)]), complex(1))",
    ] {
        assert!(
            error(&format!("report {expression}")).contains("no ordering"),
            "{expression}"
        );
    }
    for source in [
        "if complex(0.5) { report 1 }",
        "observe complex(1)",
        "report bernoulli(complex(0.5))",
        "report P(complex(0.5))",
        "report one_of([1: complex(0.5), 2: complex(0.5)])",
        "report normal(complex(0), 1)",
        "report variance(one_of([complex(1), complex(2)]))",
        "report floor(complex(1))",
        "report erf(complex(1))",
        "report complex(1) mod 2",
        "report complex(1) div 2",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
}

#[test]
fn invalid_inputs_and_overflow_are_errors() {
    for expression in [
        "complex(true)",
        "complex(1, true)",
        "complex(complex(1), 2)",
        "complex(0 ^ -1)",
        "complex(0, 0 ^ -1)",
        "cis(0 ^ -1)",
        "complex(1) + true",
        "complex(1) + (0 ^ -1)",
        "complex(1e308) + complex(1e308)",
        "complex(1e308) * complex(1e308)",
        "abs2(complex(1e308))",
        "abs(complex(1.7e308, 1.7e308))",
    ] {
        assert!(!error(&format!("report {expression}")).is_empty(), "{expression}");
    }
    for expression in ["complex(1) / complex(0)", "complex(0) ^ -1"] {
        assert!(error(&format!("report {expression}")).contains("division by zero"));
    }
    for expression in ["complex(1) ^ 0.5", "2 ^ complex(2)"] {
        assert!(error(&format!("report {expression}")).contains("int exponent"));
    }
    for expression in ["complex()", "complex(1, 2, 3)", "real()", "imag(1, 2)", "cis()"] {
        assert!(compile_error(&format!("report {expression}")).contains("takes"));
    }
    assert!(error("report complex(normal(0, 1))").contains("draw a value first"));
}
