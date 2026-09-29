//! Integer precision, promotion, and bounded execution across language features.
mod common;

use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};
use probl_number::Integer;
use std::collections::{BTreeSet, HashSet};

#[test]
fn arithmetic_and_rounding_stay_exact() {
    for expression in [
        "9223372036854775807 + 1 == 9223372036854775808",
        "-9223372036854775808 - 1 == -9223372036854775809",
        "-(-9223372036854775808) == 9223372036854775808",
        "abs(-9223372036854775808) == 9223372036854775808",
        "10^100 + 1 - 10^100 == 1",
        "(10^100 + 1) * 3 == 3 * 10^100 + 3",
        "(-10^100 - 1) div 3 == -(10^100 div 3) - 1",
        "(-10^100 - 1) mod 3 == 1",
        "(10^100 + 1) mod -3 == -1",
        "-9223372036854775808 div -1 == 9223372036854775808",
        "factorial(30) == 265252859812191058636308480000000",
        "choose(100, 50) == 100891344545564193334812497256",
        "choose(10^100, 2) == (10^100 * (10^100 - 1)) div 2",
        "choose(10^100, 10^100) == 1",
        "gcd(10^100 * 6, 10^100 * 9) == 10^100 * 3",
        "lcm(10^100 * 6, 10^100 * 9) == 10^100 * 18",
        "euler_phi(2^100) == 2^99",
        "round(10^100 + 15, -1) == 10^100 + 20",
        "round(-10^100 - 15, -1) == -10^100 - 20",
        "round(10^100, -(10^100)) == 0",
        "round(10^100, 10^100) == 10^100",
        "floor(1e100) == 1e100",
        "ceil(-1e100) == -1e100",
        "(-1)^(10^400 + 1) == -1",
        "(-1)^(-10^400 - 1) == -1.0",
        "complex(0, 1)^(10^400 + 1) == complex(0, 1)",
        "complex(0, 1)^(-10^400 - 1) == complex(0, -1)",
    ] {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
}

#[test]
fn floats_are_an_explicit_precision_boundary() {
    for expression in [
        "9007199254740992 != 9007199254740993",
        "9007199254740992 < 9007199254740993",
        "9007199254740993 > 9007199254740992.0",
        "9007199254740993 != 9007199254740993.0",
        "9223372036854775807 < 9223372036854775808.0",
        "9223372036854775808 == 9223372036854775808.0",
        "10^400 > 1e308",
        "-10^400 < -1e308",
        "-3 > -3.5",
        "3 < 3.5",
        "complex(9007199254740992) != 9007199254740993",
        "(10^400) / (10^400) == 1.0",
        "(-10^400) / (2 * 10^400) == -0.5",
        "(10^400) / (-2 * 10^400) == -0.5",
        "(10^400)^-1 == 0.0",
        "2^-3 == 0.125",
    ] {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
    for expression in ["sin(10^400)", "complex(10^400)", "10^400 / 1", "0^-1"] {
        let err = exec_raw(&format!("report {expression}"), &Options::default()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Language, "{expression}: {err:?}");
    }
}

#[test]
fn storage_identity_survives_promotion_and_numeric_order_is_total() {
    let large: Integer = "9223372036854775808".parse().unwrap();
    let demoted = large.sub(&1.into()).unwrap().sub(&i64::MAX.into()).unwrap();
    let zero: Integer = "-0000".parse().unwrap();
    assert_eq!(HashSet::from([demoted, zero, 0.into()]).len(), 1);
    let values = [
        Value::Int((-1).into()),
        Value::Float(-0.0),
        Value::Float(0.0),
        Value::Int(0.into()),
        Value::Float(f64::NAN),
        Value::Float(-f64::NAN),
        Value::Float(f64::NEG_INFINITY),
        Value::Float(f64::INFINITY),
        Value::Int(large),
        Value::Int("1".repeat(400).parse().unwrap()),
    ];
    assert_eq!(
        HashSet::from(values.clone()).len(),
        BTreeSet::from(values.clone()).len()
    );
    for a in &values {
        for b in &values {
            assert_eq!(a.cmp(b).is_eq(), a == b);
            for c in &values {
                if a <= b && b <= c {
                    assert!(a <= c);
                }
            }
        }
    }
}

#[test]
fn collections_and_worlds_keep_large_values_distinct() {
    for expression in [
        "len(0..10^100) == 10^100 + 1",
        "(10^100..10^100+4)[3] == 10^100 + 3",
        "(0..10^100)[10^99] == 10^99",
        "(10^100 + 1) in (10^100..10^100 + 2)",
        "9007199254740992.0 not in (9007199254740993..9007199254740995)",
        "sort([10^100 + 1, 10^100])[0] == 10^100",
        "[10^100: 3, 10^100 + 1: 4][10^100 + 1] == 4",
        "str(10^30 + 1) == \"1000000000000000000000000000001\"",
        "\"{10^30 + 1}\" == \"1000000000000000000000000000001\"",
    ] {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
    let src = "let n ~ one_of([10^100, 10^100 + 1])\nreport n - 10^100";
    for merge in [false, true] {
        let out = exec_raw(
            src,
            &Options {
                merge,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(
            out.reports[0].distribution(),
            vec![(Value::Int(0.into()), 0.5), (Value::Int(1.into()), 0.5)]
        );
    }
    for mode in ["enumerate", "sample(runs: 200, seed: 1)"] {
        assert_eq!(
            chance(&format!(
                "@mode {mode}\nlet n ~ one_of(10^100..10^100 + 2)\nreport n >= 10^100 and n <= 10^100 + 2"
            )),
            1.0
        );
        let text = output(&format!("@mode {mode}\nreport one_of([10^400, 10^400 + 1])"));
        assert!(!text.contains("NaN") && !text.contains("inf"), "{text}");
    }
}

#[test]
fn size_work_and_memory_limits_fail_cleanly() {
    for src in [
        "report 2^65536",
        "report 2^(10^100)",
        "report factorial(10^100)",
        "report choose(10^100, 10^99)",
    ] {
        let e = exec_raw(src, &Options::default()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Limit, "{src}: {e:?}");
    }
    assert!(compile_error(&format!("report {}", "9".repeat(20_000))).contains("integer size"));
    let mut options = Options {
        limits: Limits {
            max_integer_bits: 64,
            ..Limits::default()
        },
        ..Options::default()
    };
    for src in ["report 18446744073709551616", "report 2^64", "report floor(1e100)"] {
        assert_eq!(exec_raw(src, &options).unwrap_err().kind, ErrorKind::Limit);
    }
    options.limits = Limits {
        max_integer_bytes: 500,
        ..Limits::default()
    };
    let e = exec_raw("let x = 10^100\nreport one_of(x..x + 1000)", &options).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    assert!(e.message.contains("memory allowance"));
    options.limits = Limits {
        max_work: 100,
        ..Limits::default()
    };
    assert_eq!(
        exec_raw("report euler_phi(2^127 - 1)", &options).unwrap_err().kind,
        ErrorKind::Limit
    );
}
