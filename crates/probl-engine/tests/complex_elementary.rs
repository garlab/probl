//! Principal complex functions, including branch sides and numerical extremes.
mod common;

use common::*;
use probl_engine::complex::Complex;
use probl_engine::value::Value;
use std::f64::consts::{FRAC_PI_2, LN_2, PI};

fn value(expression: &str) -> Complex {
    let values = distribution(&format!("report {expression}"));
    match values.as_slice() {
        [(Value::Complex(z), p)] if *p == 1.0 => *z,
        _ => panic!("expected a complex scalar: {values:?}"),
    }
}

fn relative(actual: f64, expected: f64) {
    let tolerance = 5e-14 * expected.abs() + 4.0 * f64::from_bits(1);
    assert!((actual - expected).abs() <= tolerance, "{actual:e} != {expected:e}");
}

fn components(expression: &str, re: f64, im: f64) {
    let z = value(expression);
    relative(z.re(), re);
    relative(z.im(), im);
}

#[test]
fn elementary_functions_match_independent_reference_values() {
    // CPython cmath at 1 + 2i; these check both dispatch and the scalar kernel.
    for (name, re, im) in [
        ("sqrt", 1.272019649514069, 0.7861513777574233),
        ("exp", -1.1312043837568135, 2.4717266720048188),
        ("ln", 0.8047189562170503, 1.1071487177940904),
        ("log10", 0.3494850021680094, 0.480828578784234),
        ("sin", 3.165778513216168, 1.959601041421606),
        ("cos", 2.0327230070196656, -3.0518977991517997),
        ("tan", 0.0338128260798967, 1.0147936161466333),
        ("asin", 0.4270785863924761, 1.5285709194809982),
        ("acos", 1.1437177404024204, -1.5285709194809982),
        ("atan", 1.3389725222944935, 0.40235947810852507),
        ("sinh", -0.4890562590412937, 1.4031192506220405),
        ("cosh", -0.64214812471552, 1.0686074213827783),
        ("tanh", 1.16673625724092, -0.24345820118572534),
        ("asinh", 1.4693517443681852, 1.0634400235777521),
        ("acosh", 1.5285709194809982, 1.1437177404024204),
        ("atanh", 0.17328679513998632, 1.1780972450961724),
    ] {
        components(&format!("{name}(complex(1, 2))"), re, im);
    }
    for expression in [
        "cbrt(complex(1, 2)) ^ 3 - complex(1, 2)",
        "exp2(complex(1, 2)) - exp(complex(1, 2) * ln(2))",
        "log2(complex(1, 2)) * ln(2) - ln(complex(1, 2))",
        "expm1(complex(1, 2)) + 1 - exp(complex(1, 2))",
        "log1p(complex(1, 2)) - ln(complex(2, 2))",
    ] {
        assert!(value(expression).abs() < 1e-13, "{expression}");
    }
}

#[test]
fn principal_values_on_cuts_use_canonical_positive_zero() {
    components("ln(complex(-1, -0.0))", 0.0, PI);
    components("sqrt(complex(-4, -0.0))", 0.0, 2.0);
    components("sqrt(complex(0))", 0.0, 0.0);
    components("cbrt(complex(0))", 0.0, 0.0);
    components("cbrt(complex(-8))", 1.0, 3.0_f64.sqrt());
    let a = 1.3169578969248166; // acosh(2)
    let b = 0.5493061443340549; // ln(3)/2
    for (expr, re, im) in [
        ("asin(complex(2))", FRAC_PI_2, a),
        ("asin(complex(-2))", -FRAC_PI_2, a),
        ("acos(complex(2))", 0.0, -a),
        ("acos(complex(-2))", PI, -a),
        ("atanh(complex(2))", b, FRAC_PI_2),
        ("atanh(complex(-2))", -b, FRAC_PI_2),
        ("acosh(complex(-2))", a, PI),
        ("acosh(complex(0))", 0.0, FRAC_PI_2),
        ("asinh(complex(0, 2))", a, FRAC_PI_2),
        ("asinh(complex(-0.0, -2))", a, -FRAC_PI_2),
        ("atan(complex(0, 2))", FRAC_PI_2, b),
        ("atan(complex(-0.0, -2))", FRAC_PI_2, -b),
    ] {
        components(expr, re, im);
    }
    // Both sides are available through nonzero components, not signed zeros.
    assert!(value("ln(complex(-1, -1e-20))").im() < 0.0);
    assert!(value("sqrt(complex(-1, -1e-20))").im() < 0.0);
    assert!(value("asin(complex(2, -1e-20))").im() < 0.0);
    assert!(value("atan(complex(-1e-20, 2))").re() < 0.0);
    assert!(value("asinh(complex(-1e-20, 2))").re() < 0.0);
    components("ln(exp(complex(0, 4)))", 0.0, 4.0 - 2.0 * PI);
    // Other logarithm branches are explicit ordinary arithmetic.
    assert!(value("exp(ln(complex(1, 2)) + complex(0, 4*pi)) - complex(1, 2)").abs() < 1e-13);
}

#[test]
fn inverse_identities_and_conjugation_hold_off_the_cuts() {
    for x in [-3.0, -0.25, 0.25, 3.0] {
        for y in [-2.0, -0.125, 0.125, 2.0] {
            let z = Complex::new(x, y).unwrap();
            for (forward, inverse) in [
                (Complex::sin as fn(Complex) -> _, Complex::asin as fn(Complex) -> _),
                (Complex::cos, Complex::acos),
                (Complex::tan, Complex::atan),
                (Complex::sinh, Complex::asinh),
                (Complex::cosh, Complex::acosh),
                (Complex::tanh, Complex::atanh),
                (Complex::exp, Complex::ln),
            ] {
                let recovered = forward(inverse(z).unwrap()).unwrap();
                relative(recovered.re(), x);
                relative(recovered.im(), y);
                let reflected = inverse(z.conjugate()).unwrap();
                let expected = inverse(z).unwrap().conjugate();
                relative(reflected.re(), expected.re());
                relative(reflected.im(), expected.im());
            }
        }
    }
}

#[test]
fn tiny_inputs_and_branch_point_offsets_survive_cancellation() {
    components("log1p(complex(1e-100, 1e-100))", 1e-100, 1e-100);
    components("expm1(complex(1e-100, 1e-100))", 1e-100, 1e-100);
    components("log1p(complex(0, 1e-100))", 0.5e-200, 1e-100);
    components("expm1(complex(0, 1e-100))", -0.5e-200, 1e-100);
    components("ln(complex(1, 1e-100))", 0.5e-200, 1e-100);
    components("asin(complex(1, 1e-100))", FRAC_PI_2, 1e-50);
    components("acos(complex(1, 1e-100))", 1e-50, -1e-50);
    components("acosh(complex(1, 1e-100))", 1e-50, 1e-50);
    components(
        "atanh(complex(1, 1e-300))",
        0.5 * (LN_2 + 300.0 * libm::log(10.0)),
        PI / 4.0,
    );
    for name in [
        "sin", "tan", "asin", "atan", "sinh", "tanh", "asinh", "atanh", "log1p", "expm1",
    ] {
        components(&format!("{name}(complex(1e-300, -1e-300))"), 1e-300, -1e-300);
    }
    components("sqrt(complex(5e-324))", libm::sqrt(f64::from_bits(1)), 0.0);
    components("exp2(complex(-1074))", f64::from_bits(1), 0.0);
    assert_eq!(value("exp2(complex(-1075))").re(), 0.0);
}

#[test]
fn large_inputs_do_not_overflow_finite_results() {
    let expected_log = libm::log(1.7e308) + 0.5 * LN_2;
    components("ln(complex(1.7e308, 1.7e308))", expected_log, PI / 4.0);
    components(
        "sqrt(complex(1.7e308, 1.7e308))",
        1.4325088230154573e154,
        5.933645827121221e153,
    );
    components("tan(complex(1e308, 1e308))", 0.0, 1.0);
    components("tanh(complex(-1e308, 1e308))", -1.0, 0.0);
    components("asin(complex(1e308, 1e308))", PI / 4.0, libm::log(1e308) + 1.5 * LN_2);
    components("atanh(complex(1e308, 1e308))", 5e-309, FRAC_PI_2);
    components("atan(complex(1e308, 1e308))", FRAC_PI_2, 5e-309);
    // exp's modulus overflows, but both Cartesian components still fit.
    let z = value("exp(complex(710, pi/4))");
    relative(z.re(), 1.5796728482882015e308);
    relative(z.im(), 1.5796728482882015e308);
    components("exp(complex(-1e308, 1e308))", 0.0, 0.0);
    assert!(value("cos(complex(0, 710))").re().is_finite());
    assert!(value("sinh(complex(710))").re().is_finite());
}

#[test]
fn real_domains_singularities_and_overflow_stay_explicit() {
    for expression in ["sqrt(-1)", "ln(-1)", "asin(2)", "acosh(0)", "atanh(2)"] {
        assert!(error(&format!("report {expression}")).contains("isn't defined"));
    }
    close(mean("report cbrt(-8)"), -2.0);
    for expression in [
        "ln(complex(0))",
        "log2(complex(0))",
        "log10(complex(0))",
        "log1p(complex(-1))",
        "atanh(complex(1))",
        "atanh(complex(-1))",
        "atan(complex(0, 1))",
        "atan(complex(0, -1))",
        "exp(complex(1000))",
        "exp2(complex(2000))",
        "sin(complex(0, 1000))",
        "cosh(complex(1000))",
    ] {
        let message = error(&format!("report {expression}"));
        let name = expression.split('(').next().unwrap();
        assert!(message.contains(&format!("`{name}`")), "{expression}: {message}");
    }
    for name in ["sqrt", "cos", "ln", "exp"] {
        assert!(error(&format!("report {name}(true)")).contains("number"));
        assert!(error(&format!("report {name}(normal(0, 1))")).contains("draw a value first"));
    }
}

#[test]
fn complex_functions_lift_without_creating_branches_or_changing_weights() {
    for mode in ["enumerate", "sample(runs: 100, seed: 7)"] {
        let source = format!("@mode {mode}\nreport sqrt(one_of([complex(-4), complex(9)]))");
        let d = distribution(&source);
        assert_eq!(d.len(), 2);
        assert!(
            d.iter()
                .any(|(z, _)| *z == Value::Complex(Complex::new(0.0, 2.0).unwrap()))
        );
        assert!(
            d.iter()
                .any(|(z, _)| *z == Value::Complex(Complex::new(3.0, 0.0).unwrap()))
        );
        for (_, p) in d {
            close(p, 0.5);
        }
    }
    components("ln(complex(-1))", 0.0, PI);
}
