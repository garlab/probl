//! Integer precision, promotion, and bounded execution across language features.
mod common;

use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};
use probl_number::Integer;
use std::collections::{BTreeSet, HashSet};

#[test]
fn bit_length_and_integer_log_are_exact() {
    for (n, bits) in [
        ("0", 0),
        ("1", 1),
        ("-1", 1),
        ("7", 3),
        ("-7", 3),
        ("8", 4),
        ("-9223372036854775808", 64),
        ("2^100 - 1", 100),
        ("2^100", 101),
        ("10^400", 1329),
        ("0b0001", 1),
        ("0x000f", 4),
    ] {
        assert_eq!(chance(&format!("report bit_length({n}) == {bits}")), 1.0, "{n}");
        if bits != 0 {
            assert_eq!(chance(&format!("report ilog2(abs({n})) == {}", bits - 1)), 1.0, "{n}");
        }
    }
    // Adjacent powers must stay distinct despite rounding in floating log2.
    for k in [1, 2, 31, 53, 63, 64, 100, 1000] {
        assert_eq!(
            chance(&format!(
                "report ilog2(2^{k} - 1) == {} and ilog2(2^{k}) == {k} and ilog2(2^{k} + 1) == {k}",
                k - 1
            )),
            1.0
        );
    }
    for mode in ["enumerate", "sample(runs: 200, seed: 8)"] {
        for expression in [
            "bit_length(one_of([-7, -4, 4, 7])) == 3",
            "ilog2(one_of([8, 9, 15])) == 3",
        ] {
            assert_eq!(chance(&format!("@mode {mode}\nreport {expression}")), 1.0);
        }
    }
    assert_eq!(
        distribution("report bit_length(d4)"),
        vec![
            (Value::Int(1.into()), 0.25),
            (Value::Int(2.into()), 0.5),
            (Value::Int(3.into()), 0.25)
        ]
    );
}

#[test]
fn bit_queries_check_domains_and_take_constant_work() {
    for name in ["bit_length", "ilog2"] {
        for input in ["1.4", "true", "50%", "complex(1)", "\"7\"", "[]"] {
            assert!(error(&format!("report {name}({input})")).contains("needs an int"));
        }
        for args in ["", "1, 2"] {
            assert!(compile_error(&format!("report {name}({args})")).contains("takes"));
        }
    }
    for n in ["0", "-1", "-10^400"] {
        assert!(error(&format!("report ilog2({n})")).contains("positive integer"));
    }
    let n = Integer::from_radix_digits(&"f".repeat(16384), 16).unwrap();
    for (f, expected) in [
        (probl_sema::Builtin::BitLength, 65536),
        (probl_sema::Builtin::ILog2, 65535),
    ] {
        let mut budget = probl_engine::dist::Budget {
            work_left: 1,
            ..probl_engine::dist::Budget::unlimited()
        };
        let result = probl_engine::builtins::call_plain(f, &[Value::Int(n.clone())], &mut budget).unwrap();
        assert_eq!(result, Value::Int(expected.into()));
    }
}

#[test]
fn bit_operations_are_exact_signed_integers() {
    for expression in [
        "bit_and(0b1010, 0b1100) == 8",
        "bit_or(0b1010, 0b1100) == 14",
        "bit_xor(0b1010, 0b1100) == 6",
        "bit_and(-1, 0xff) == 255",
        "bit_or(-8, 3) == -5",
        "bit_xor(-1, 0xff) == -256",
        "bit_not(0) == -1",
        "bit_not(-1) == 0",
        "bit_not(0x000f) == -16",
        "bit_and(bit_not(0b1010), 0xff) == 245",
        "bit_count(0) == 0",
        "bit_count(-0b1011) == 3",
        "bit_count(-0x8000_0000_0000_0000) == 1",
        "bit_count(2^4096 - 1) == 4096",
        "bit_count(-2^4096) == 1",
        "bit_and(-2^100, 2^101 - 1) == 2^100",
        "bit_or(-2^100, 2^100 - 1) == -1",
        "bit_xor(2^4096 + 3, 2^4096 + 5) == 6",
        "bit_not(-2^4096) == 2^4096 - 1",
        "bit_not(bit_and(2^100 + 3, -2^200 + 7)) == bit_or(bit_not(2^100 + 3), bit_not(-2^200 + 7))",
        "[bit_xor(2^100, 2^100): 7][0] == 7",
        "0xff.bit_count() == 8",
        "0xff.bit_and(0x0f) == 15",
    ] {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
}

#[test]
fn bit_operations_lift_and_preserve_draw_correlation() {
    for (src, values) in [
        ("report bit_and(one_of([2^100, 2^100 + 1]), 1)", [0, 1]),
        ("report bit_or(one_of([2, 3]), 4)", [6, 7]),
        ("report bit_xor(one_of([0, 1]), one_of([0, 1]))", [0, 1]),
        ("report bit_not(one_of([-1, 0]))", [-1, 0]),
        ("report bit_count(one_of([-3, 4]))", [1, 2]),
    ] {
        assert_eq!(
            distribution(src),
            values
                .into_iter()
                .map(|n| (Value::Int(n.into()), 0.5))
                .collect::<Vec<_>>(),
            "{src}"
        );
    }
    for mode in ["enumerate", "sample(runs: 200, seed: 8)"] {
        assert_eq!(
            chance(&format!(
                "@mode {mode}\nlet n ~ one_of([2^100, -2^100 - 1])\nreport bit_xor(n, n) == 0 and bit_not(n) == -n - 1"
            )),
            1.0
        );
    }
}

#[test]
fn bit_operations_require_integer_arguments_and_correct_arity() {
    for (name, arity) in [
        ("bit_and", 2),
        ("bit_or", 2),
        ("bit_xor", 2),
        ("bit_not", 1),
        ("bit_count", 1),
    ] {
        for input in ["1.4", "true", "50%", "complex(1)", "\"7\"", "[]"] {
            let args = if arity == 1 {
                input.to_owned()
            } else {
                format!("{input}, 1")
            };
            assert!(error(&format!("report {name}({args})")).contains("needs an int"));
            if arity == 2 {
                assert!(error(&format!("report {name}(1, {input})")).contains("needs an int"));
            }
        }
        for args in ["", "1", "1, 2", "1, 2, 3"] {
            if args.split(',').filter(|s| !s.is_empty()).count() != arity {
                assert!(compile_error(&format!("report {name}({args})")).contains("takes"));
            }
        }
    }
}

#[test]
fn bit_operations_obey_size_work_and_memory_budgets() {
    use probl_engine::builtins::call_plain;
    use probl_engine::dist::Budget;
    use probl_sema::Builtin as B;

    let n = Integer::from_radix_digits(&"f".repeat(16384), 16).unwrap();
    for f in [B::BitAnd, B::BitOr, B::BitXor, B::BitNot, B::BitCount] {
        let mut budget = Budget {
            work_left: 1,
            ..Budget::unlimited()
        };
        let args = if matches!(f, B::BitNot | B::BitCount) {
            vec![Value::Int(n.clone())]
        } else {
            vec![Value::Int(n.clone()), Value::Int(0.into())]
        };
        assert_eq!(call_plain(f, &args, &mut budget).unwrap_err().kind, ErrorKind::Limit);
    }
    let mut budget = Budget::unlimited();
    assert_eq!(
        call_plain(B::BitCount, &[Value::Int(n)], &mut budget).unwrap(),
        Value::Int(65536.into())
    );
    let options = Options {
        limits: Limits {
            max_integer_bits: 64,
            ..Limits::default()
        },
        ..Options::default()
    };
    for expr in [
        "bit_not(0xffff_ffff_ffff_ffff)",
        "bit_xor(-1, 0xffff_ffff_ffff_ffff)",
        "bit_and(-0xffff_ffff_ffff_ffff, -2)",
    ] {
        assert_eq!(
            exec_raw(&format!("report {expr}"), &options).unwrap_err().kind,
            ErrorKind::Limit
        );
    }
    for f in [B::BitAnd, B::BitOr, B::BitXor, B::BitNot] {
        let mut budget = Budget {
            integer_bytes_left: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ..Budget::unlimited()
        };
        let n = Value::Int(Integer::from(2).pow(100).unwrap());
        let args = match f {
            B::BitNot => vec![n],
            B::BitAnd => vec![n, Value::Int((-1).into())],
            _ => vec![n, Value::Int(0.into())],
        };
        let err = call_plain(f, &args, &mut budget).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Limit);
        assert!(err.message.contains("memory allowance"));
    }
}

#[test]
fn radix_literals_are_ordinary_exact_integers() {
    for expression in [
        "0b111 == 7",
        "0xfab101 == 16429313",
        "0XFA_B101 == 0xfab101",
        "0B111_001 == 57",
        "0xffff_ffff_ffff_ffff + 1 == 2^64",
        "-0x8000000000000000 == -9223372036854775808",
        "0x1_0000_0000_0000_0000_0000_0000 == 2^96",
        "0x1e3 == 483",
        "0x2d6 == 726",
        "-0x2^2 == -4",
        "len(0b1..0b11) == 3",
        "len(0x0..<0xF) == 15",
        "0xff.bit_length() == 8",
        "[0xff: 7][255] == 7",
        "str(0b111) == \"7\"",
        "\"{0xff}\" == \"255\"",
        "(match -0xff { -0xFF => 1, _ => 0 }) == 1",
    ] {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
    for expression in ["0b1.1", "0x1.8", "0x10%", "0b10%", "0b1e3"] {
        assert!(!compile_error(&format!("report {expression}")).is_empty());
    }
    for (prefix, digits) in [("0b", "1".repeat(65536)), ("0x", "f".repeat(16384))] {
        assert_eq!(chance(&format!("report bit_length({prefix}{digits}) == 65536")), 1.0);
        assert!(compile_error(&format!("report {prefix}1{digits}")).contains("integer size"));
    }
}

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
